//! Edge translation memory on the server: the versioned snapshot served to the worker / proxy / patcher,
//! the `edge_translate` job that auto-translates discovered segments (project model, glossary, validators,
//! billing), and the review API of the dashboard (pending segments, human edits).
//!
//! `GET /edge/v1/{project}/{locale}/tm[?since=v]`: `glot_core::edge::tm::Tm` JSON, one consistent snapshot read at
//! `edge_catalog_version` `v` (ETag `"tm-{locale}-{v}"`, header `X-Glot-Version`); `since=v` / `If-None-Match` of
//! the current version → 304. Public when the project's CDN is public, else a member's token. Contains the edge
//! translations (never `pending`) and, as context `*`, the catalog translations of keys whose source is exactly
//! a segment text (keys stay the reference when they exist).
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use glot_core::{
    context::KeyContext,
    edge::tm::{ANY, Tm, normalize, text_hash},
    validate::{IssueCode, Syntax, has_errors},
};
use glot_llm::translate::{Item, Request};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use super::discover::Settings;
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    jobs::JobRecord,
    projects::{self, ProjectRow, Role},
};

pub const JOB_KIND: &str = "edge_translate";
/// Segments one job translates at most (most-seen first); the next missing report queues the rest.
const JOB_MAX: i64 = 500;
/// Segment texts carry the segmenter's `<0>…</0>` tags and `{0}` placeholders: ICU structure, whatever the
/// project's catalog syntax is (that one describes its catalogs, not its pages).
const SYNTAX: Syntax = Syntax::Icu;

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/edge/v1/{project}/{locale}/tm", get(serve))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET])
                .allow_headers([header::IF_NONE_MATCH, header::AUTHORIZATION])
                .expose_headers([header::ETAG, "x-glot-version".parse().expect("header")]),
        )
}

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/edge/segments";
    Router::new()
        .route(p, get(list))
        .route(&format!("{p}/{{hash}}/{{locale}}"), put(put_translation))
}

/// Current version of (project, locale); 0 before any write.
pub async fn version<'e>(
    db: impl sqlx::PgExecutor<'e>,
    project: Uuid,
    locale: &str,
) -> sqlx::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT version FROM edge_catalog_version WHERE project_id = $1 AND locale = $2",
    )
    .bind(project)
    .bind(locale)
    .fetch_optional(db)
    .await?
    .unwrap_or(0))
}

/// One consistent snapshot (repeatable read: the version and the rows come from the same instant).
pub async fn snapshot(db: &sqlx::PgPool, p: &ProjectRow, locale: &str) -> sqlx::Result<Tm> {
    // Isolation in the BEGIN itself: a separate SET TRANSACTION fails ("must be called before any query")
    // whenever the pooled connection already ran a statement in the transaction.
    let mut tx = db
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .await?;
    let v = version(&mut *tx, p.id, locale).await?;
    let catalog: Vec<(String, String)> = sqlx::query_as(
        "SELECT k.source, t.value FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND t.locale = $2 AND t.status <> 'stale' AND t.value <> ''
           AND strpos(k.source, '{') = 0 AND strpos(k.source, '<') = 0
         ORDER BY k.key",
    )
    .bind(p.id)
    .bind(locale)
    .fetch_all(&mut *tx)
    .await?;
    let edge: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT hash, context, value FROM edge_translations
         WHERE project_id = $1 AND locale = $2 AND status <> 'pending'",
    )
    .bind(p.id)
    .bind(locale)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let rows = catalog
        .into_iter()
        .map(|(source, value)| {
            (
                text_hash(&p.source_locale, &normalize(&source)),
                ANY.to_string(),
                value,
            )
        })
        .chain(edge);
    Ok(Tm::from_rows(locale, v, rows))
}

async fn serve(
    State(state): State<AppState>,
    AppPath((slug, locale)): AppPath<(String, String)>,
    AppQuery(q): AppQuery<HashMap<String, String>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let p = crate::cdn::readable(&state, &slug, &headers).await?;
    // `zh`, `zh_CN`, `zh-Hans` → the project's `zh-CN` (crate::locale_alias); exact match first.
    let locale = crate::locale_alias::resolve(&locale, &p.target_locales)
        .ok_or_else(|| AppError::not_found("locale not found"))?
        .to_string();
    let v = version(&state.db, p.id, &locale).await?;
    let etag = format!("\"tm-{locale}-{v}\"");
    let unchanged = q.get("since").is_some_and(|s| *s == v.to_string())
        || headers
            .get(header::IF_NONE_MATCH)
            .and_then(|h| h.to_str().ok())
            == Some(etag.as_str());
    let (mut res, etag) = if unchanged {
        (StatusCode::NOT_MODIFIED.into_response(), etag)
    } else {
        let tm = snapshot(&state.db, &p, &locale).await?;
        // The snapshot may be newer than the version read above.
        let etag = format!("\"tm-{locale}-{}\"", tm.v);
        (
            (
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                serde_json::to_string(&tm).map_err(AppError::internal)?,
            )
                .into_response(),
            etag,
        )
    };
    let h = res.headers_mut();
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("etag"));
    h.insert(
        "x-glot-version",
        HeaderValue::from_str(etag.trim_matches('"').rsplit('-').next().unwrap_or("0"))
            .expect("version"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if p.public_cdn {
            "public, max-age=60, stale-while-revalidate=600, stale-if-error=86400"
        } else {
            "private, no-cache"
        }),
    );
    Ok(res)
}

// ---------- auto-translation ----------

/// Queue one `edge_translate` job for (project, locale) when segments lack a translation and none is pending.
/// Never fails the caller on quota/budget: returns whether a job is queued or running.
pub async fn queue(
    state: &AppState,
    p: &ProjectRow,
    locale: &str,
    settings: &Settings,
) -> ApiResult<bool> {
    let mut tx = state.db.begin().await?;
    // Row lock on the project: concurrent reports (all replicas) queue a single job.
    sqlx::query("SELECT 1 FROM projects WHERE id = $1 FOR UPDATE")
        .bind(p.id)
        .execute(&mut *tx)
        .await?;
    let (pending, failed_recently, spent): (bool, bool, f64) = sqlx::query_as(
        "SELECT coalesce(bool_or(status IN ('queued', 'running')), false),
                coalesce(bool_or(status = 'failed' AND finished_at > now() - interval '10 minutes'), false),
                coalesce(sum(cost_usd) FILTER (WHERE created_at >= date_trunc('month', now())), 0)
         FROM jobs WHERE project_id = $1 AND kind = $2 AND params->>'edge_locale' = $3",
    )
    .bind(p.id)
    .bind(JOB_KIND)
    .bind(locale)
    .fetch_one(&mut *tx)
    .await?;
    if pending {
        return Ok(true);
    }
    // A failing provider must not turn every report into a new failed job.
    if failed_recently || settings.monthly_budget_usd.is_some_and(|b| spent >= b) {
        return Ok(false);
    }
    let todo: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM edge_segments s WHERE s.project_id = $1 AND NOT EXISTS (
            SELECT 1 FROM edge_translations t
            WHERE t.project_id = s.project_id AND t.hash = s.hash AND t.context = s.context AND t.locale = $2))",
    )
    .bind(p.id)
    .bind(locale)
    .fetch_one(&mut *tx)
    .await?;
    if !todo {
        return Ok(false);
    }
    if let Err(e) = crate::billing::check_llm(state, p).await {
        tracing::info!(project = %p.slug, "edge auto-translation not queued: {}", e.message);
        return Ok(false);
    }
    let remaining = settings.monthly_budget_usd.map(|b| b - spent);
    sqlx::query(
        "INSERT INTO jobs (id, project_id, kind, status, params) VALUES ($1, $2, $3, 'queued', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(JOB_KIND)
    .bind(json!({ "locales": [locale], "edge_locale": locale, "budget_usd": remaining }))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.job_wakeup.notify_one();
    Ok(true)
}

type Todo = (String, String, String, bool, Option<String>);

/// The `edge_translate` job: translate the locale's untranslated segments (most seen first) with the project's
/// model, glossary and validators. Errors are never written; segments first seen on a sensitive page are
/// written `pending` (served once reviewed).
pub(crate) async fn run_job(state: &AppState, job: &mut JobRecord) -> anyhow::Result<()> {
    let params = job.params.0.clone();
    let locale = params
        .locales
        .as_ref()
        .and_then(|l| l.first())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("edge_translate job without locale"))?;
    let (p, config, translator, glossary) =
        crate::jobs::setup(state, job.project_id, params.budget_usd).await?;
    // (hash, context, source, needs_review, other contexts' translation of the same text)
    let todo: Vec<Todo> = sqlx::query_as(
        "SELECT s.hash, s.context, s.source_text, s.needs_review,
                (SELECT string_agg(o.context || ' → ' || o.value, '; ') FROM edge_translations o
                 WHERE o.project_id = s.project_id AND o.hash = s.hash AND o.locale = $2)
         FROM edge_segments s
         WHERE s.project_id = $1 AND NOT EXISTS (
            SELECT 1 FROM edge_translations t
            WHERE t.project_id = s.project_id AND t.hash = s.hash AND t.context = s.context AND t.locale = $2)
         ORDER BY (s.context ~ '^(title|meta:|h1)') DESC, s.hits DESC, s.first_seen LIMIT $3",
    )
    .bind(p.id)
    .bind(&locale)
    .bind(JOB_MAX)
    .fetch_all(&state.db)
    .await?;
    job.progress_total = todo.len() as i32;
    crate::jobs::save(state, job).await;

    for chunk in todo.chunks(config.llm.batch_size.max(1)) {
        let mut terms = vec![];
        for (_, _, source, _, _) in chunk {
            for t in glossary.matches(source) {
                if !terms
                    .iter()
                    .any(|g: &glot_core::glossary::Term| g.term == t.term)
                {
                    terms.push(t.clone());
                }
            }
        }
        let req = Request {
            source_locale: p.source_locale.clone(),
            target_locale: locale.clone(),
            items: chunk
                .iter()
                .enumerate()
                .map(|(i, (_, ctx, source, _, others))| Item {
                    key: format!("s{i}"),
                    source: source.clone(),
                    context: segment_context(i, ctx, others.as_deref()),
                    previous: None,
                })
                .collect(),
            glossary: terms,
            syntax: SYNTAX,
            style: p.style.clone(),
        };
        let (results, usage) = translator.translate(req).await?;
        job.cost_usd += usage.cost_usd;

        let mut rows: Vec<(String, String, String, String, String, Option<i16>, String)> = vec![];
        for r in results {
            let Some((hash, ctx, source, review, _)) = r
                .key
                .strip_prefix('s')
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| chunk.get(i))
            else {
                continue;
            };
            let mut issues = crate::keys::check(source, &r.value, &locale, SYNTAX, &glossary);
            if has_errors(&r.issues) || has_errors(&issues) {
                continue;
            }
            issues.extend(
                r.issues
                    .into_iter()
                    .filter(|i| i.code == IssueCode::LowScore),
            );
            rows.push((
                hash.clone(),
                ctx.clone(),
                r.value,
                if *review { "pending" } else { "machine" }.into(),
                r.model,
                r.score.map(i16::from),
                serde_json::to_string(&issues)?,
            ));
        }
        // One statement per chunk: one version bump, the page switches to the new texts together.
        let col =
            |f: fn(&(String, String, String, String, String, Option<i16>, String)) -> String| {
                rows.iter().map(f).collect::<Vec<_>>()
            };
        sqlx::query(
            "INSERT INTO edge_translations (project_id, hash, context, locale, value, status, origin, model, score, issues)
             SELECT $1, h, c, $2, v, s, 'machine', m, sc, i::jsonb
             FROM unnest($3::text[], $4::text[], $5::text[], $6::text[], $7::text[], $8::int2[], $9::text[])
                  AS x(h, c, v, s, m, sc, i)
             ON CONFLICT DO NOTHING",
        )
        .bind(p.id)
        .bind(&locale)
        .bind(col(|r| r.0.clone()))
        .bind(col(|r| r.1.clone()))
        .bind(col(|r| r.2.clone()))
        .bind(col(|r| r.3.clone()))
        .bind(col(|r| r.4.clone()))
        .bind(rows.iter().map(|r| r.5).collect::<Vec<_>>())
        .bind(col(|r| r.6.clone()))
        .execute(&state.db)
        .await?;
        job.progress_done += chunk.len() as i32;
        crate::jobs::save(state, job).await;
        if let Some(budget) = config.llm.budget_usd
            && job.cost_usd > budget
        {
            anyhow::bail!(
                "budget exceeded: spent ${:.4} of ${budget:.2}",
                job.cost_usd
            );
        }
    }
    // Translated URL slugs wait for their texts like any segment: write the ones this job just translated.
    if let Err(e) = super::slugs::materialize(state, &p).await {
        tracing::info!(target: "glot_edge", "slugs not written: {}", e.message);
    }
    Ok(())
}

/// Prompt context of a segment: where it is rendered, and how the same text was translated elsewhere
/// (an ambiguous text, "Save" the button vs "Save 20 %" in a price, is translated for this context).
fn segment_context(i: usize, ctx: &str, others: Option<&str>) -> KeyContext {
    let (role, page) = ctx.split_once('|').unwrap_or(("text", ctx));
    let ui_role = match role {
        "button" | "input" => "button",
        "a" => "link",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "label" => "label",
        "title" | "meta:og:title" | "meta:twitter:title" => "title",
        r if r.starts_with("meta:") => "text",
        r if r.ends_with(":placeholder") => "placeholder",
        r if r.ends_with(":title") => "title",
        r if r.contains(":aria-") || r.ends_with(":alt") => "aria",
        r if r.ends_with(":value") => "button",
        _ => "text",
    };
    let mut description = format!(
        "Text of a public web page ({page}), rendered in a <{role}> element. Keep the numbered tags <0>…</0> \
         and placeholders {{0}} exactly; they stand for markup and values."
    );
    if let Some(o) = others {
        description.push_str(&format!(
            " The same source text elsewhere on the site is translated: {o}. Translate it for this context."
        ));
    }
    KeyContext {
        key: format!("s{i}"),
        description: Some(description),
        ui_role: Some(ui_role.into()),
        ..Default::default()
    }
}

// ---------- review API (dashboard) ----------

#[derive(Deserialize)]
struct ListQuery {
    locale: String,
    /// missing | pending | machine | reviewed | stale; none = all
    status: Option<String>,
    /// Only segments seen on this path.
    path: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SegmentView {
    pub hash: String,
    pub context: String,
    pub source: String,
    pub url_sample: Option<String>,
    pub hits: i64,
    pub needs_review: bool,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub value: Option<String>,
    /// missing when no translation yet
    pub status: String,
    #[schema(value_type = Object)]
    pub issues: serde_json::Value,
}

async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<ListQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    crate::validate_locale(&q.locale)?;
    let rows: Vec<SegmentView> = sqlx::query_as(
        "SELECT s.hash, s.context, s.source_text AS source, s.url_sample, s.hits, s.needs_review, s.first_seen,
                s.last_seen, t.value, coalesce(t.status, 'missing') AS status, coalesce(t.issues, '[]') AS issues
         FROM edge_segments s
         LEFT JOIN edge_translations t ON t.project_id = s.project_id AND t.hash = s.hash
              AND t.context = s.context AND t.locale = $2
         WHERE s.project_id = $1 AND ($3::text IS NULL OR coalesce(t.status, 'missing') = $3)
           AND ($4::text IS NULL OR EXISTS (SELECT 1 FROM edge_page_segments g WHERE g.project_id = s.project_id
                AND g.path = $4 AND g.hash = s.hash AND g.context = s.context))
         ORDER BY s.hits DESC, s.hash, s.context LIMIT $5",
    )
    .bind(p.id)
    .bind(&q.locale)
    .bind(&q.status)
    .bind(&q.path)
    .bind(q.limit.unwrap_or(200).clamp(1, 1000))
    .fetch_all(&state.db)
    .await?;
    let v = version(&state.db, p.id, &q.locale).await?;
    Ok(Json(json!({ "version": v, "segments": rows })))
}

#[derive(Deserialize)]
struct PutBody {
    context: String,
    value: String,
}

/// Human translation / approval of a segment in one context (`reviewed`). Validation errors are refused:
/// this text goes live on the page without another check.
async fn put_translation(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, hash, locale)): AppPath<(String, String, String)>,
    AppJson(b): AppJson<PutBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    if !p.target_locales.contains(&locale) {
        return Err(AppError::bad_request(format!(
            "'{locale}' is not a target locale of this project"
        )));
    }
    let source: String = sqlx::query_scalar(
        "SELECT source_text FROM edge_segments WHERE project_id = $1 AND hash = $2 AND context = $3",
    )
    .bind(p.id)
    .bind(&hash)
    .bind(&b.context)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("segment not found"))?;
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let issues = crate::keys::check(&source, &b.value, &locale, SYNTAX, &glossary);
    if has_errors(&issues) {
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            issues
                .iter()
                .filter(|i| i.severity == glot_core::validate::Severity::Error)
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ));
    }
    sqlx::query(
        "INSERT INTO edge_translations (project_id, hash, context, locale, value, status, origin, issues, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'reviewed', 'human', $6, now())
         ON CONFLICT (project_id, hash, context, locale) DO UPDATE SET value = $5, status = 'reviewed',
            origin = 'human', model = NULL, score = NULL, issues = $6, updated_at = now()",
    )
    .bind(p.id)
    .bind(&hash)
    .bind(&b.context)
    .bind(&locale)
    .bind(&b.value)
    .bind(sqlx::types::Json(&issues))
    .execute(&state.db)
    .await?;
    let v = version(&state.db, p.id, &locale).await?;
    Ok(Json(
        json!({ "status": "reviewed", "issues": issues, "version": v }),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_context_names_role_page_and_other_meanings() {
        let c = super::segment_context(0, "td|/pricing", Some("button|/ → Guardar"));
        assert_eq!(c.ui_role.as_deref(), Some("text"));
        let d = c.description.unwrap();
        assert!(
            d.contains("/pricing") && d.contains("<td>") && d.contains("Guardar"),
            "{d}"
        );
        let b = super::segment_context(1, "button|/", None);
        assert_eq!(b.ui_role.as_deref(), Some("button"));
    }
}
