//! `POST /edge/v1/{project}/missing`: segments a page rendered that the TM could not translate, reported by the
//! worker, the proxy or the client patcher. Authenticated by the project's public key (`pk_…`, same key as
//! Insights), rate-limited per minute, aggregated (one row per segment and context, hit counter), bounded by
//! `edge::discover` (zones, no session, daily cap, no personal data). Accepted segments queue one
//! `edge_translate` job per (project, locale); meanwhile the source text is served (never a hole).
//!
//! Body (any content type, so browsers post `text/plain` without a CORS preflight):
//! `{"key": "pk_…", "locale": "es", "auth": false,
//!   "segments": [{"hash": "…", "text": "Save <0>20 %</0>", "url": "/pricing", "role": "button"}],
//!   "pages": [{"url": "/pricing", "seen": [{"hash": "…", "role": "h1"}]}]}`
//! `auth: true` = the visitor has a session: nothing is discovered. `pages` (optional) = every segment the
//! page rendered, for per-URL coverage (`edge_page_segments`).
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use glot_core::edge::tm::{context, normalize, path_only, text_hash};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use super::discover::{Settings, looks_personal};
use crate::{ApiResult, AppError, AppPath, AppState, projects::ProjectRow};

/// Segments per minute and project (each reported segment counts).
const RATE_PER_MIN: i32 = 2000;
const MAX_ITEMS: usize = 200;
const MAX_TEXT: usize = 2000;
const MAX_SEEN: usize = 500;

#[derive(Debug, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub key: Option<String>,
    pub locale: String,
    #[serde(default)]
    pub auth: bool,
    #[serde(default)]
    pub segments: Vec<Item>,
    #[serde(default)]
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub hash: String,
    pub text: String,
    pub url: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    pub url: String,
    pub seen: Vec<Seen>,
}

#[derive(Debug, Deserialize)]
pub struct Seen {
    pub hash: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Outcome {
    /// New segments stored.
    pub accepted: usize,
    /// Already known (hit counter bumped).
    pub known: usize,
    /// Refused: hash mismatch, too long, personal data, outside the discovery zones, over the daily cap.
    pub rejected: usize,
    /// An `edge_translate` job is queued or running for this locale.
    pub queued: bool,
    /// Why nothing was discovered, when nothing was (`disabled`, `auth`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<&'static str>,
}

pub fn public_routes() -> Router<crate::AppState> {
    Router::new()
        .route("/edge/v1/{project}/missing", post(ingest))
        .layer(DefaultBodyLimit::max(512 * 1024))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

async fn ingest(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let r: Report = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    let key = r
        .key
        .clone()
        .or_else(|| {
            headers
                .get("x-glot-key")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        })
        .ok_or_else(|| AppError::bad_request("missing public key (`key` or X-Glot-Key)"))?;
    if r.segments.len() > MAX_ITEMS || r.pages.len() > 20 {
        return Err(AppError::bad_request(format!(
            "at most {MAX_ITEMS} segments and 20 pages per report"
        )));
    }
    crate::validate_locale(&r.locale)?;
    let weight = (r.segments.len() + r.pages.iter().map(|p| p.seen.len()).sum::<usize>()).max(1);
    let (project_id, hits, window_end): (Uuid, i32, i64) = sqlx::query_as(
        "WITH k AS (SELECT k.project_id FROM insight_keys k JOIN projects p ON p.id = k.project_id
                    WHERE k.public_key = $1 AND p.slug = $2)
         INSERT INTO edge_missing_rate AS r (project_id, window_start, hits)
         SELECT project_id, date_trunc('minute', now()), $3 FROM k
         ON CONFLICT (project_id) DO UPDATE SET
           hits = CASE WHEN excluded.window_start > r.window_start THEN excluded.hits ELSE r.hits + excluded.hits END,
           window_start = greatest(r.window_start, excluded.window_start)
         RETURNING r.project_id, r.hits, extract(epoch FROM r.window_start + interval '1 minute')::int8",
    )
    .bind(&key)
    .bind(&slug)
    .bind(weight.min(i32::MAX as usize) as i32)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("unknown project or public key"))?;
    if hits > RATE_PER_MIN {
        let now = chrono::Utc::now().timestamp();
        return Ok((
            [(header::RETRY_AFTER, (window_end - now).max(1).to_string())],
            AppError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many missing segments for this project; see Retry-After",
            ),
        )
            .into_response());
    }
    let p = crate::projects::load_public(&state, &slug)
        .await?
        .filter(|p| p.id == project_id)
        .ok_or_else(|| AppError::not_found("unknown project or public key"))?;
    let out = record(&state, &p, &r).await?;
    Ok((StatusCode::ACCEPTED, Json(json!(out))).into_response())
}

/// Store a report (also called in-process by the Rust proxy) and queue the auto-translation.
pub async fn record(state: &AppState, p: &ProjectRow, r: &Report) -> ApiResult<Outcome> {
    if !p.target_locales.contains(&r.locale) {
        return Err(AppError::bad_request(format!(
            "'{}' is not a target locale of this project",
            r.locale
        )));
    }
    let settings = Settings::load(&state.db, p.id).await?;
    let mut out = Outcome::default();
    if !settings.enabled {
        out.skipped = Some("disabled");
        return Ok(out);
    }
    if r.auth {
        out.skipped = Some("auth");
        return Ok(out);
    }
    let mut locales = p.target_locales.clone();
    locales.push(p.source_locale.clone());

    // Validated, deduplicated (hash, context, text, needs_review, path).
    let mut seen = BTreeSet::new();
    let mut items: Vec<(String, String, String, bool, String)> = vec![];
    for it in &r.segments {
        let text = normalize(&it.text);
        let path = path_only(&it.url);
        let ok = !text.is_empty()
            && text.chars().count() <= MAX_TEXT
            && text_hash(&p.source_locale, &text) == it.hash
            && !looks_personal(&text)
            && settings.discoverable(&path);
        if !ok {
            out.rejected += 1;
            continue;
        }
        let ctx = context(&it.role, &path, &locales);
        if seen.insert((it.hash.clone(), ctx.clone())) {
            items.push((it.hash.clone(), ctx, text, settings.sensitive(&path), path));
        }
    }

    let mut tx = state.db.begin().await?;
    // Known ones: bump the counter.
    let (hashes, ctxs): (Vec<String>, Vec<String>) =
        items.iter().map(|i| (i.0.clone(), i.1.clone())).unzip();
    let known: Vec<(String, String)> = sqlx::query_as(
        "UPDATE edge_segments s SET last_seen = now(), hits = s.hits + 1
         FROM unnest($2::text[], $3::text[]) AS x(h, c)
         WHERE s.project_id = $1 AND s.hash = x.h AND s.context = x.c
         RETURNING s.hash, s.context",
    )
    .bind(p.id)
    .bind(&hashes)
    .bind(&ctxs)
    .fetch_all(&mut *tx)
    .await?;
    out.known = known.len();
    let fresh: Vec<_> = items
        .into_iter()
        .filter(|i| !known.iter().any(|k| k.0 == i.0 && k.1 == i.1))
        .collect();
    // Daily cap on new segments (rolling 24 h).
    let today: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM edge_segments WHERE project_id = $1 AND first_seen > now() - interval '1 day'",
    )
    .bind(p.id)
    .fetch_one(&mut *tx)
    .await?;
    let room = (settings.daily_cap as i64 - today).max(0) as usize;
    out.rejected += fresh.len().saturating_sub(room);
    let fresh = &fresh[..fresh.len().min(room)];
    let col = |f: fn(&(String, String, String, bool, String)) -> String| {
        fresh.iter().map(f).collect::<Vec<_>>()
    };
    let res = sqlx::query(
        "INSERT INTO edge_segments (project_id, hash, context, source_text, needs_review, url_sample)
         SELECT $1, h, c, t, r, u FROM unnest($2::text[], $3::text[], $4::text[], $5::bool[], $6::text[]) AS x(h, c, t, r, u)
         ON CONFLICT DO NOTHING",
    )
    .bind(p.id)
    .bind(col(|i| i.0.clone()))
    .bind(col(|i| i.1.clone()))
    .bind(col(|i| i.2.clone()))
    .bind(fresh.iter().map(|i| i.3).collect::<Vec<_>>())
    .bind(col(|i| i.4.clone()))
    .execute(&mut *tx)
    .await?;
    out.accepted = res.rows_affected() as usize;

    // Page inventories (coverage per URL), discoverable paths only.
    for page in &r.pages {
        let path = path_only(&page.url);
        if !settings.discoverable(&path) {
            continue;
        }
        let mut pairs = BTreeSet::new();
        for s in page.seen.iter().take(MAX_SEEN) {
            if s.hash.len() == 32 && s.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                pairs.insert((s.hash.clone(), context(&s.role, &path, &locales)));
            }
        }
        let (h, c): (Vec<String>, Vec<String>) = pairs.into_iter().unzip();
        sqlx::query(
            "INSERT INTO edge_page_segments (project_id, path, hash, context)
             SELECT $1, $2, h, c FROM unnest($3::text[], $4::text[]) AS x(h, c)
             ON CONFLICT (project_id, path, hash, context) DO UPDATE SET last_seen = now()",
        )
        .bind(p.id)
        .bind(&path)
        .bind(&h)
        .bind(&c)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    if settings.auto_translate {
        out.queued = super::tm::queue(state, p, &r.locale, &settings).await?;
    }
    Ok(out)
}
