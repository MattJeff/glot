//! Bundle localization: the JavaScript chunks of a translated page are served with their hard-coded strings
//! translated too (`glot_extract::bundle`), so a client component renders the same text as the translated HTML
//! (no React #418, no client re-render). Only literals equal to a segment seen in a rendered page, translated
//! in an unambiguous way (`Tm::lookup_rendered`).
//!
//! - Subdomain proxy (`proxy.rs`): every `*javascript*` response of a translating domain goes through
//!   [`serve`]. Cache per (sha256 of the origin chunk, locale, TM version).
//! - Worker / middleware (one URL for every language): `POST /edge/v1/{project}/{locale}/bundle` with the
//!   chunk as body and the public key (`X-Glot-Key: pk_…`), rate-limited with the missing-segment budget;
//!   returns the localized chunk (`x-glot-bundle: <n literals>`, `x-glot-version: es@7`).
//!
//! Caching. Next/Vite serve chunks as `immutable` under content-hashed names, but a localized chunk changes with
//! the TM version under the same name. Rewriting every chunk reference (HTML, flight data, the bundler runtime's
//! chunk-URL function) is fragile, so the localized response keeps the URL and drops `immutable`:
//! `Cache-Control: public, no-cache` + a strong `ETag` of the localized body. The browser keeps the file and
//! revalidates it (`304` without a body) instead of trusting a year-long copy of an older version.
// ponytail: wholesale eviction of a bounded map, an LRU if hit ratio ever matters here.
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use glot_core::edge::tm::{Tm, text_hash};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tower_http::cors::{Any, CorsLayer};

use crate::{ApiResult, AppError, AppPath, AppState};

/// Larger chunks are served untouched.
pub const MAX_JS: usize = 4 * 1024 * 1024;
const CACHE_MAX: usize = 500;
/// Same per-minute budget as `missing.rs` (shared `edge_missing_rate` row).
const RATE_PER_MIN: i32 = 2000;

pub struct Localized {
    pub js: Bytes,
    pub etag: String,
    /// Literals translated.
    pub n: usize,
}

static CACHE: LazyLock<Mutex<HashMap<[u8; 32], Arc<Localized>>>> = LazyLock::new(Default::default);

pub fn is_js(content_type: &str) -> bool {
    content_type.contains("javascript") || content_type.contains("ecmascript")
}

/// The chunk localized with this TM snapshot (cached). CPU-bound: call it off the async runtime.
pub fn localize(body: &[u8], source_locale: &str, tm: &Tm) -> Arc<Localized> {
    let key: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(body);
        h.update([0]);
        h.update(source_locale.as_bytes());
        h.update([0]);
        h.update(tm.locale.as_bytes());
        h.update(tm.v.to_le_bytes());
        h.finalize().into()
    };
    if let Some(l) = CACHE.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return l;
    }
    let src = String::from_utf8_lossy(body);
    let lookup = |s: &str| {
        tm.lookup_rendered(&text_hash(source_locale, s))
            .map(String::from)
    };
    let (js, n) = match glot_extract::bundle::localize(&src, &lookup) {
        Some((out, n)) => (Bytes::from(out), n),
        None => (Bytes::copy_from_slice(body), 0),
    };
    let etag = format!(
        "\"glot-{}\"",
        Sha256::digest(&js)
            .iter()
            .take(12)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let l = Arc::new(Localized { js, etag, n });
    if let Ok(mut c) = CACHE.lock() {
        if c.len() >= CACHE_MAX {
            c.clear();
        }
        c.insert(key, l.clone());
    }
    l
}

/// Localized chunk with the origin's other headers; `304` when the browser already has this exact body.
pub fn respond(
    status: StatusCode,
    mut headers: HeaderMap,
    l: &Localized,
    if_none_match: Option<&HeaderValue>,
    tm: &Tm,
) -> Response {
    for h in [
        header::CONTENT_LENGTH,
        header::CONTENT_ENCODING,
        header::ETAG,
        header::LAST_MODIFIED,
        header::EXPIRES,
    ] {
        headers.remove(h);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, no-cache"),
    );
    if let Ok(v) = HeaderValue::from_str(&l.etag) {
        headers.insert(header::ETAG, v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{}@{}", tm.locale, tm.v)) {
        headers.insert("x-glot-version", v);
    }
    headers.insert("x-glot-bundle", HeaderValue::from(l.n));
    let fresh = if_none_match
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == l.etag));
    if fresh {
        let mut r = StatusCode::NOT_MODIFIED.into_response();
        *r.headers_mut() = headers;
        return r;
    }
    (status, headers, l.js.clone()).into_response()
}

// ---------------------------------------------------------------------------------------------- worker endpoint

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/edge/v1/{project}/{locale}/bundle", post(endpoint))
        .layer(DefaultBodyLimit::max(MAX_JS))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([axum::http::Method::POST])
                .allow_headers([header::CONTENT_TYPE, "x-glot-key".parse().expect("hdr")]),
        )
}

async fn endpoint(
    State(state): State<AppState>,
    AppPath((slug, locale)): AppPath<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let key = headers
        .get("x-glot-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::bad_request("missing public key (X-Glot-Key)"))?;
    crate::validate_locale(&locale)?;
    let hits: Option<i32> = sqlx::query_scalar(
        "WITH k AS (SELECT k.project_id FROM insight_keys k JOIN projects p ON p.id = k.project_id
                    WHERE k.public_key = $1 AND p.slug = $2)
         INSERT INTO edge_missing_rate AS r (project_id, window_start, hits)
         SELECT project_id, date_trunc('minute', now()), 1 FROM k
         ON CONFLICT (project_id) DO UPDATE SET
           hits = CASE WHEN excluded.window_start > r.window_start THEN excluded.hits ELSE r.hits + excluded.hits END,
           window_start = greatest(r.window_start, excluded.window_start)
         RETURNING r.hits",
    )
    .bind(key)
    .bind(&slug)
    .fetch_optional(&state.db)
    .await?;
    match hits {
        None => return Err(AppError::not_found("unknown project or public key")),
        Some(h) if h > RATE_PER_MIN => {
            return Err(AppError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many edge requests for this project this minute",
            ));
        }
        _ => {}
    }
    let p = crate::projects::load_public(&state, &slug)
        .await?
        .filter(|p| p.target_locales.contains(&locale))
        .ok_or_else(|| AppError::not_found("unknown project or locale"))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let tm = super::proxy::tm_for(&state, &p, &locale, deadline)
        .await
        .ok_or_else(|| AppError::new(StatusCode::SERVICE_UNAVAILABLE, "no_tm", "no TM yet"))?;
    let source = p.source_locale.clone();
    let t = tm.clone();
    let l = tokio::task::spawn_blocking(move || localize(&body, &source, &t))
        .await
        .map_err(AppError::internal)?;
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    Ok(respond(StatusCode::OK, h, &l, None, &tm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glot_core::edge::tm::Tm;

    #[test]
    fn localized_chunk_headers_and_revalidation() {
        let tm = Tm::from_rows(
            "es",
            7,
            [(
                text_hash("en", "Cancel anytime"),
                "p|/".to_string(),
                "Cancela cuando quieras".to_string(),
            )],
        );
        let js = br#"(0,r.jsx)("p",{children:"Cancel anytime"})"#;
        let l = localize(js, "en", &tm);
        assert_eq!(l.n, 1);
        assert!(
            std::str::from_utf8(&l.js)
                .unwrap()
                .contains("Cancela cuando quieras")
        );
        let mut h = HeaderMap::new();
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
        h.insert(header::ETAG, HeaderValue::from_static("\"origin\""));
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(js.len()));
        let r = respond(StatusCode::OK, h.clone(), &l, None, &tm);
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "public, no-cache");
        assert_eq!(r.headers()[header::ETAG], l.etag.as_str());
        assert_eq!(r.headers()["x-glot-version"], "es@7");
        assert!(r.headers().get(header::CONTENT_LENGTH).is_none());
        let inm = HeaderValue::from_str(&l.etag).unwrap();
        let r = respond(StatusCode::OK, h, &l, Some(&inm), &tm);
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        // A new version with another value: another body, another ETag (the browser's copy is replaced).
        let tm2 = Tm::from_rows(
            "es",
            8,
            [(
                text_hash("en", "Cancel anytime"),
                "p|/".to_string(),
                "Cancela cuando lo desees".to_string(),
            )],
        );
        assert_ne!(localize(js, "en", &tm2).etag, l.etag);
        // Catalog-only texts are never rewritten in bundles.
        let cat = Tm::from_rows(
            "es",
            9,
            [(
                text_hash("en", "Cancel anytime"),
                glot_core::edge::tm::ANY.to_string(),
                "X".to_string(),
            )],
        );
        assert_eq!(localize(js, "en", &cat).n, 0);
    }
}
