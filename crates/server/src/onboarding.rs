//! Onboarding checklist + milestones. Every step is derived from real project state (keys, translations,
//! glossary, members, CDN), except the chosen path, the onboarding token and the README badge being fetched,
//! which are recorded in `onboarding`. Milestones reached are persisted in `achievements` and never revoked.
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::{AuthUser, sha256_hex},
    projects::{self, Role},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/projects/{slug}/onboarding",
            get(get_onboarding).patch(patch_onboarding),
        )
        .route("/api/projects/{slug}/onboarding/token", post(create_token))
}

/// Shields.io endpoint for the README badge (public projects only).
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/badge/{file}", get(badge))
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LocaleProgress {
    pub locale: String,
    pub translated: i64,
    pub reviewed: i64,
    pub missing: i64,
    pub errors: i64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Step {
    pub id: &'static str,
    pub done: bool,
}

#[derive(Serialize)]
pub struct Achievement {
    pub id: String,
    pub unlocked_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct Onboarding {
    /// Public API base URL to put in commands (GLOT_PUBLIC_URL).
    pub server_url: String,
    pub slug: String,
    pub source_locale: String,
    pub keys: i64,
    pub first_push_at: Option<DateTime<Utc>>,
    pub locales: Vec<LocaleProgress>,
    pub path: Option<String>,
    pub hidden: bool,
    pub steps: Vec<Step>,
    pub achievements: Vec<Achievement>,
}

/// What the checklist is computed from.
#[derive(Default)]
pub struct Facts {
    pub keys: i64,
    pub locales: Vec<LocaleProgress>,
    pub members: i64,
    pub glossary: i64,
    pub public_cdn: bool,
    pub path: Option<String>,
    pub token: bool,
    pub badge: bool,
}

pub fn steps(f: &Facts) -> Vec<Step> {
    let pushed = f.keys > 0;
    let translated: i64 = f.locales.iter().map(|l| l.translated).sum();
    let errors: i64 = f.locales.iter().map(|l| l.errors).sum();
    vec![
        // A push implies a path and a token were used, even if not chosen here.
        Step {
            id: "path",
            done: f.path.is_some() || pushed,
        },
        Step {
            id: "token",
            done: f.token || pushed,
        },
        Step {
            id: "first_push",
            done: pushed,
        },
        Step {
            id: "first_translation",
            done: translated > 0,
        },
        Step {
            id: "zero_errors",
            done: translated > 0 && errors == 0,
        },
        Step {
            id: "invite",
            done: f.members > 1,
        },
        Step {
            id: "glossary",
            done: f.glossary > 0,
        },
        Step {
            id: "cdn",
            done: f.public_cdn,
        },
        Step {
            id: "badge",
            done: f.badge,
        },
    ]
}

pub fn milestones(f: &Facts) -> Vec<&'static str> {
    let translated: i64 = f.locales.iter().map(|l| l.translated).sum();
    let errors: i64 = f.locales.iter().map(|l| l.errors).sum();
    let mut out = vec![];
    if f.keys > 0 {
        out.push("first_push");
    }
    if f.keys > 0 && f.locales.iter().any(|l| l.missing == 0) {
        out.push("first_locale_100");
    }
    if translated > 0 && errors == 0 {
        out.push("zero_errors");
    }
    if f.locales.iter().filter(|l| l.translated > 0).count() >= 5 {
        out.push("five_languages");
    }
    if f.glossary > 0 {
        out.push("glossary_set");
    }
    out
}

fn server_url() -> String {
    std::env::var("GLOT_PUBLIC_URL")
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_else(|_| "http://localhost:8790".into())
}

async fn locale_progress(
    state: &AppState,
    p: &projects::ProjectRow,
    keys: i64,
) -> ApiResult<Vec<LocaleProgress>> {
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT t.locale,
                count(*) FILTER (WHERE t.status <> 'stale'),
                count(*) FILTER (WHERE t.status = 'reviewed'),
                count(*) FILTER (WHERE EXISTS (SELECT 1 FROM jsonb_array_elements(t.issues) i WHERE i->>'severity' = 'error'))
         FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 GROUP BY t.locale",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(p.target_locales
        .iter()
        .map(|l| {
            let (translated, reviewed, errors) = rows
                .iter()
                .find(|r| &r.0 == l)
                .map_or((0, 0, 0), |r| (r.1, r.2, r.3));
            LocaleProgress {
                locale: l.clone(),
                translated,
                reviewed,
                missing: (keys - translated).max(0),
                errors,
            }
        })
        .collect())
}

async fn get_onboarding(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Onboarding>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let (keys, first_push_at, members, glossary): (i64, Option<DateTime<Utc>>, i64, i64) =
        sqlx::query_as(
            "SELECT (SELECT count(*) FROM keys WHERE project_id = $1),
                (SELECT min(created_at) FROM keys WHERE project_id = $1),
                (SELECT count(*) FROM project_members WHERE project_id = $1),
                (SELECT count(*) FROM glossary_terms WHERE project_id = $1)",
        )
        .bind(p.id)
        .fetch_one(&state.db)
        .await?;
    let row: Option<(Option<String>, bool, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT path, hidden, token_created_at, badge_seen_at FROM onboarding WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    let (path, hidden, token, badge) = row.unwrap_or_default();
    let facts = Facts {
        keys,
        locales: locale_progress(&state, &p, keys).await?,
        members,
        glossary,
        public_cdn: p.public_cdn,
        path,
        token: token.is_some(),
        badge: badge.is_some(),
    };
    let reached: Vec<String> = milestones(&facts).into_iter().map(String::from).collect();
    sqlx::query(
        "INSERT INTO achievements (project_id, id) SELECT $1, unnest($2::text[]) ON CONFLICT DO NOTHING",
    )
    .bind(p.id)
    .bind(&reached)
    .execute(&state.db)
    .await?;
    let achievements = sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "SELECT id, unlocked_at FROM achievements WHERE project_id = $1 ORDER BY unlocked_at, id",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(id, unlocked_at)| Achievement { id, unlocked_at })
    .collect();
    Ok(Json(Onboarding {
        server_url: server_url(),
        slug: p.slug,
        source_locale: p.source_locale,
        keys,
        first_push_at,
        steps: steps(&facts),
        locales: facts.locales,
        path: facts.path,
        hidden,
        achievements,
    }))
}

#[derive(Deserialize)]
struct PatchBody {
    /// "agent" | "terminal" | "github"
    path: Option<String>,
    hidden: Option<bool>,
}

async fn patch_onboarding(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PatchBody>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    if b.path
        .as_deref()
        .is_some_and(|x| !["agent", "terminal", "github"].contains(&x))
    {
        return Err(AppError::bad_request(
            "path must be agent, terminal or github",
        ));
    }
    sqlx::query(
        "INSERT INTO onboarding (project_id, path, hidden) VALUES ($1, $2, coalesce($3, false))
         ON CONFLICT (project_id) DO UPDATE SET path = coalesce($2, onboarding.path), hidden = coalesce($3, onboarding.hidden)",
    )
    .bind(p.id)
    .bind(b.path)
    .bind(b.hidden)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CreatedToken {
    token: String,
}

/// A personal API token named after the project, shown once and pasted into the onboarding commands.
async fn create_token(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<(StatusCode, Json<CreatedToken>)> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let token = format!("glot_{}", hex::encode(rand::random::<[u8; 32]>()));
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO api_tokens (id, user_id, name, token_hash) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(user.id)
        .bind(format!("{} (onboarding)", p.slug))
        .bind(sha256_hex(&token))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO onboarding (project_id, token_created_at) VALUES ($1, now())
         ON CONFLICT (project_id) DO UPDATE SET token_created_at = now()",
    )
    .bind(p.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(CreatedToken { token })))
}

/// `GET /badge/{slug}.json`: average coverage of target locales, in the shields.io endpoint schema.
async fn badge(
    State(state): State<AppState>,
    AppPath(file): AppPath<String>,
) -> ApiResult<Response> {
    let slug = file.strip_suffix(".json").unwrap_or(&file);
    let p = projects::load_public(&state, slug)
        .await?
        .filter(|p| p.public_cdn)
        .ok_or_else(|| AppError::not_found("project not found"))?;
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM keys WHERE project_id = $1")
        .bind(p.id)
        .fetch_one(&state.db)
        .await?;
    let locales = locale_progress(&state, &p, keys).await?;
    let pct = if keys == 0 || locales.is_empty() {
        0
    } else {
        let done: i64 = locales.iter().map(|l| l.translated.min(keys)).sum();
        done * 100 / (keys * locales.len() as i64)
    };
    sqlx::query(
        "INSERT INTO onboarding (project_id, badge_seen_at) VALUES ($1, now())
         ON CONFLICT (project_id) DO UPDATE SET badge_seen_at = coalesce(onboarding.badge_seen_at, now())",
    )
    .bind(p.id)
    .execute(&state.db)
    .await?;
    let color = match pct {
        100 => "brightgreen",
        80.. => "green",
        50.. => "yellow",
        _ => "orange",
    };
    let body = serde_json::json!({
        "schemaVersion": 1,
        "label": format!("i18n · {} languages", locales.len()),
        "message": format!("{pct}%"),
        "color": color,
    });
    Ok(([(header::CACHE_CONTROL, "public, max-age=300")], Json(body)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(l: &str, translated: i64, missing: i64, errors: i64) -> LocaleProgress {
        LocaleProgress {
            locale: l.into(),
            translated,
            reviewed: 0,
            missing,
            errors,
        }
    }

    fn done(f: &Facts) -> Vec<&'static str> {
        steps(f)
            .into_iter()
            .filter(|s| s.done)
            .map(|s| s.id)
            .collect()
    }

    #[test]
    fn empty_project_has_nothing_done() {
        let f = Facts {
            members: 1,
            locales: vec![loc("fr", 0, 0, 0)],
            ..Default::default()
        };
        assert!(done(&f).is_empty());
        assert!(milestones(&f).is_empty());
    }

    #[test]
    fn push_with_targets_checks_path_token_and_translation() {
        let f = Facts {
            keys: 3,
            members: 1,
            locales: vec![loc("fr", 3, 0, 0), loc("ja", 1, 2, 0)],
            ..Default::default()
        };
        assert_eq!(
            done(&f),
            [
                "path",
                "token",
                "first_push",
                "first_translation",
                "zero_errors"
            ]
        );
        assert_eq!(
            milestones(&f),
            ["first_push", "first_locale_100", "zero_errors"]
        );
    }

    #[test]
    fn errors_block_zero_errors_and_extras_are_detected() {
        let f = Facts {
            keys: 2,
            members: 2,
            glossary: 1,
            public_cdn: true,
            badge: true,
            locales: ["fr", "de", "es", "it", "ja"]
                .map(|l| loc(l, 1, 1, 1))
                .to_vec(),
            ..Default::default()
        };
        let d = done(&f);
        assert!(!d.contains(&"zero_errors"));
        for s in ["invite", "glossary", "cdn", "badge"] {
            assert!(d.contains(&s), "{s}");
        }
        assert_eq!(
            milestones(&f),
            ["first_push", "five_languages", "glossary_set"]
        );
    }
}
