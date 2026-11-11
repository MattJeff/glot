//! Edge control plane read by Caddy and by every edge runtime (worker, Next/Express middleware, Rust proxy).
//! - `GET /edge/ask?domain=es.example.com`: Caddy `on_demand_tls` asks before getting a certificate. 200 only for a
//!   host verified on a project in subdomain mode; anything else 404 (no certificate, no open proxy).
//! - `GET /edge/v1/{project}/status`: kill switch + versions, `{"enabled": true, "off": ["es.x.io"],
//!   "versions": {"es": 7}, "selectors": {"include": [], "exclude": [".user-bio"]},
//!   "app": {"rules": ["/login", "proxy /api/"], "action": "redirect", "locale": "query:lang"}}` (application
//!   paths, `app_paths`), `"auth": {"pages": "server", "discover": false}` (requests with a session),
//!   `"website": {"on": true, "position": "bottom-right", …}, "hreflang": {"en": "https://x.io", "es": "https://es.x.io"}`
//!   (universal website tag, `widget/glot.js`). Cached 10 s: switching Edge
//!   off (project or domain) reaches every runtime in < 1 min (runtimes revalidate every 20 s). Nothing secret:
//!   hostnames are public DNS, versions are counters.
//! - `POST /edge/v1/{project}/usage` `{"key": "pk_…", "pages": 120}`: translated pages served by a JS runtime
//!   (worker / middleware), counted against the plan's `edge_pages`; 402 past the limit → the runtime serves the
//!   origin untranslated for a while. The Rust proxy counts in-process (`proxy::count`).
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

use super::discover::Settings;
use crate::{ApiResult, AppError, AppPath, AppQuery, AppState};

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/edge/ask", get(ask))
        .route("/edge/v1/{project}/status", get(status))
        .route("/edge/v1/{project}/usage", post(usage))
}

#[derive(serde::Deserialize)]
struct Usage {
    key: String,
    pages: i64,
}

async fn usage(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    body: axum::body::Bytes,
) -> ApiResult<StatusCode> {
    let u: Usage = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    let project: uuid::Uuid = sqlx::query_scalar(
        "SELECT k.project_id FROM insight_keys k JOIN projects p ON p.id = k.project_id WHERE k.public_key = $1 AND p.slug = $2",
    )
    .bind(&u.key)
    .bind(&slug)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("unknown project or public key"))?;
    crate::billing::consume_n(
        &state,
        project,
        crate::billing::EDGE_PAGES,
        u.pages.clamp(0, 1_000_000),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lowercase host without trailing dot or port, or None when it is not a plausible DNS name.
pub fn clean_host(raw: &str) -> Option<String> {
    let h = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let h = h.split(':').next().unwrap_or("").to_string();
    let ok = (3..=253).contains(&h.len())
        && h.contains('.')
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    ok.then_some(h)
}

async fn ask(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<HashMap<String, String>>,
) -> ApiResult<StatusCode> {
    let Some(host) = q.get("domain").and_then(|d| clean_host(d)) else {
        return Ok(StatusCode::NOT_FOUND);
    };
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM edge_domains WHERE host = $1 AND mode = 'subdomain' AND verified_at IS NOT NULL)",
    )
    .bind(&host)
    .fetch_one(&state.db)
    .await?;
    Ok(if ok {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn status(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Response> {
    let p = crate::projects::load_public(&state, &slug)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))?;
    let s = Settings::load(&state.db, p.id).await?;
    let off: Vec<String> = sqlx::query_scalar(
        "SELECT host FROM edge_domains WHERE project_id = $1 AND NOT enabled ORDER BY host",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let versions: BTreeMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT locale, version FROM edge_catalog_version WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let url_pattern: Option<String> =
        sqlx::query_scalar("SELECT url_pattern FROM preview_settings WHERE project_id = $1")
            .bind(p.id)
            .fetch_optional(&state.db)
            .await?;
    let hreflang: BTreeMap<String, String> = if s.website_snippet.on {
        super::sitemap::site(&state, &p, true)
            .await?
            .bases
            .into_iter()
            .collect()
    } else {
        BTreeMap::new()
    };
    let body = json!({
        "enabled": s.enabled,
        "off": off,
        "versions": versions,
        "selectors": {"include": s.selectors_include, "exclude": s.selectors_exclude},
        "app": {
            "rules": s.app_paths,
            "action": s.app_action,
            "locale": super::app_paths::locale_style(url_pattern.as_deref()),
        },
        // Pages behind sign-in: off | patcher | server (+ app-mode discovery of UI text, review-gated).
        "auth": {"pages": s.auth_pages, "discover": s.auth_discover},
        // Universal website tag (`widget/glot.js`) + the Edge bases it advertises as hreflang (SEO-ready
        // locales on verified domains; `{}` = no Edge domain yet). Computed only when the tag is on.
        "website": s.website_snippet.0,
        "hreflang": hreflang,
    });
    let mut res = Json(body).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=10, stale-if-error=86400"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hosts() {
        use super::clean_host;
        assert_eq!(
            clean_host("ES.Cliponit.app.").as_deref(),
            Some("es.cliponit.app")
        );
        assert_eq!(clean_host("es.x.io:443").as_deref(), Some("es.x.io"));
        for bad in [
            "",
            "localhost",
            "a..b",
            "-a.io",
            "a_b.io",
            "x.io/evil",
            "1.2.3.4 x",
        ] {
            assert_eq!(clean_host(bad), None, "{bad}");
        }
    }
}
