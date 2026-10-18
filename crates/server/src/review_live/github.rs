//! Live review → GitHub, when the project's GitHub App link is set (`github_app.rs`): mirror an annotation as an
//! issue in the linked repo or as a comment on the open glot PR (`POST M/annotations/{id}/github`), and resolve it
//! when a PR that is that glot PR, or that closes the issue (`Fixes #12`), is merged (`pull_request` webhook).
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::model::{self, Access, Actor, Ctx};
use crate::{ApiResult, AppError, AppJson, AppPath, AppState, github_app as app};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/projects/{slug}/review/annotations/{id}/github",
        post(mirror),
    )
}

#[derive(Deserialize)]
pub struct MirrorBody {
    /// `issue` (default) or `pr_comment`.
    #[serde(default)]
    target: Option<String>,
}

fn gh_err(e: anyhow::Error) -> AppError {
    AppError::new(StatusCode::BAD_GATEWAY, "github_error", e.to_string())
}

/// GitHub renders markdown and notifies @mentions: reviewer text must not ping anyone.
fn quote(s: &str) -> String {
    s.replace('@', "@\u{200b}")
        .lines()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(a: &model::Annotation, slug: &str) -> (String, String) {
    let what = a.key.clone().unwrap_or_else(|| {
        a.anchor["text"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect()
    });
    let title = format!("[glot review] {what} ({}, {})", a.locale, a.route);
    let mut body = format!(
        "**{}** on [{}]({}) ({}):\n\n{}\n\n",
        a.author.name.replace('@', "@\u{200b}"),
        a.route,
        a.url,
        a.locale,
        quote(&a.body)
    );
    if let Some(k) = &a.key {
        body += &format!("Key: `{k}`\n");
    }
    if let Some(t) = &a.translation {
        body += &format!("Current: {}\n", quote(&t.value));
    }
    if let Some(s) = &a.suggestion {
        body += &format!("Suggestion: {}\n", quote(s));
    }
    body += &format!(
        "\n[Open in glot]({}/app/{slug}/review?annotation={})\n<!-- glot-annotation:{} -->",
        app::web_origin(),
        a.id,
        a.id
    );
    (title, body)
}

/// Installation token narrowed to the repo and to what this needs (`issues` or `pull_requests`).
async fn token(state: &AppState, installation: i64, repo: &str, perm: &str) -> ApiResult<String> {
    let gh_app = app::require_app(state).await?;
    let name = repo.split('/').nth(1).unwrap_or(repo);
    let v = app::gh(
        &app::app_jwt(&gh_app).map_err(gh_err)?,
        reqwest::Method::POST,
        &format!(
            "{}/app/installations/{installation}/access_tokens",
            app::api_base()
        ),
        Some(json!({ "repositories": [name], "permissions": { perm: "write", "metadata": "read" } })),
    )
    .await
    .map_err(|e| {
        gh_err(e.context(format!(
            "the GitHub App installation must grant `{perm}: write` (accept the new permission on GitHub)"
        )))
    })?;
    v["token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| gh_err(anyhow::anyhow!("GitHub returned no installation token")))
}

async fn mirror(
    State(state): State<AppState>,
    ctx: Ctx,
    AppPath((_slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<MirrorBody>,
) -> ApiResult<Json<Value>> {
    ctx.need(Access::Member)?;
    let p = &ctx.project;
    let existing: Option<(String, i32, String)> = sqlx::query_as(
        "SELECT kind, number, url FROM review_github WHERE annotation_id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    if let Some((kind, number, url)) = existing {
        return Ok(Json(
            json!({ "kind": kind, "number": number, "url": url, "created": false }),
        ));
    }
    let a = model::one(&state, p.id, id, &ctx.actor).await?;
    let link = app::load_link(&state, p.id).await?.ok_or_else(|| {
        AppError::new(
            StatusCode::CONFLICT,
            "github_not_linked",
            "connect a GitHub repository to this project first (Settings → GitHub)",
        )
    })?;
    let (title, body) = render(&a, &p.slug);
    let api = app::api_base();
    let (kind, number, url) = match b.target.as_deref().unwrap_or("issue") {
        "issue" => {
            let t = token(&state, link.installation_id, &link.repo, "issues").await?;
            let v = app::gh(
                &t,
                reqwest::Method::POST,
                &format!("{api}/repos/{}/issues", link.repo),
                Some(json!({ "title": title, "body": format!("{body}\n\nMerge a PR with `Fixes #<this issue>` to resolve it in glot.") })),
            )
            .await
            .map_err(gh_err)?;
            (
                "issue",
                v["number"].as_i64().unwrap_or(0) as i32,
                v["html_url"].as_str().unwrap_or_default().to_string(),
            )
        }
        "pr_comment" => {
            let n = link.pr_number.ok_or_else(|| {
                AppError::new(
                    StatusCode::CONFLICT,
                    "no_glot_pr",
                    "no glot pull request is open for this project",
                )
            })?;
            let t = token(&state, link.installation_id, &link.repo, "pull_requests").await?;
            let v = app::gh(
                &t,
                reqwest::Method::POST,
                &format!("{api}/repos/{}/issues/{n}/comments", link.repo),
                Some(json!({ "body": format!("### {title}\n\n{body}") })),
            )
            .await
            .map_err(gh_err)?;
            (
                "pr_comment",
                n,
                v["html_url"].as_str().unwrap_or_default().to_string(),
            )
        }
        _ => return Err(AppError::bad_request("target must be issue or pr_comment")),
    };
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO review_github (annotation_id, project_id, repo, kind, number, url) VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (annotation_id) DO NOTHING",
    )
    .bind(id)
    .bind(p.id)
    .bind(&link.repo)
    .bind(kind)
    .bind(number)
    .bind(&url)
    .execute(&mut *tx)
    .await?;
    let data = json!({ "kind": kind, "number": number, "url": url });
    model::emit(
        &mut tx,
        p.id,
        Some(id),
        None,
        "github.mirrored",
        &ctx.actor,
        data,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "kind": kind, "number": number, "url": url, "created": true }),
    ))
}

/// Issue numbers a PR closes: `fixes #12`, `Closes: #3`, `resolved #7` (GitHub's closing keywords).
/// ponytail: one number per keyword, same-repo refs only (`owner/repo#12` and `#1, #2` lists are not parsed).
fn closing_refs(text: &str) -> Vec<i32> {
    const KW: &[&str] = &[
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ];
    let words: Vec<String> = text.split_whitespace().map(|w| w.to_lowercase()).collect();
    words
        .windows(2)
        .filter(|w| KW.contains(&w[0].trim_end_matches(':')))
        .filter_map(|w| {
            let n = w[1].strip_prefix('#')?;
            n.trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()
        })
        .collect()
}

/// `pull_request` closed + merged on a linked repo: resolve the open annotations mirrored on that PR or on an
/// issue it closes, with a comment pointing at the PR.
pub async fn on_merged(
    state: &AppState,
    repo: &str,
    installation: i64,
    v: &Value,
) -> ApiResult<()> {
    let pr = &v["pull_request"];
    let number = v["number"].as_i64().unwrap_or(-1) as i32;
    let closes = closing_refs(&format!(
        "{}\n{}",
        pr["title"].as_str().unwrap_or_default(),
        pr["body"].as_str().unwrap_or_default()
    ));
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT g.annotation_id, g.project_id FROM review_github g
         JOIN github_links l ON l.project_id = g.project_id AND l.installation_id = $2
         JOIN review_annotations a ON a.id = g.annotation_id AND a.status IN ('open', 'in_progress')
         WHERE lower(g.repo) = lower($1)
           AND ((g.kind = 'pr_comment' AND g.number = $3) OR (g.kind = 'issue' AND g.number = ANY($4)))",
    )
    .bind(repo)
    .bind(installation)
    .bind(number)
    .bind(&closes)
    .fetch_all(&state.db)
    .await?;
    let actor = Actor {
        user_id: None,
        link_id: None,
        name: "GitHub".into(),
    };
    let url = pr["html_url"].as_str().unwrap_or_default();
    for (annotation, project) in rows {
        let mut tx = state.db.begin().await?;
        let cid = Uuid::new_v4();
        let body = format!("Fixed by [#{number}]({url}) (merged).");
        sqlx::query(
            "INSERT INTO review_comments (id, annotation_id, author_name, body) VALUES ($1, $2, $3, $4)",
        )
        .bind(cid)
        .bind(annotation)
        .bind(&actor.name)
        .bind(&body)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE review_annotations SET status = 'resolved', resolved_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(annotation)
        .execute(&mut *tx)
        .await?;
        model::emit(
            &mut tx,
            project,
            Some(annotation),
            Some(cid),
            "comment.created",
            &actor,
            json!({}),
        )
        .await?;
        model::emit(
            &mut tx,
            project,
            Some(annotation),
            None,
            "annotation.updated",
            &actor,
            json!({ "status": "resolved", "pr": url }),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn closing_refs() {
        assert_eq!(
            super::closing_refs("Shorter CTA\n\nFixes #12, closes: #3. Refs #9 resolved #7)"),
            vec![12, 3, 7]
        );
        assert!(super::closing_refs("fix #abc and #4").is_empty());
    }
}
