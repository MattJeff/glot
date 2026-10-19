//! Live review: annotations on the customer's running site (widget) and in the SaaS Review page, threads,
//! reactions, captures, shareable review links, realtime (SSE), manual text editing, annotation → translation.
//! Contract shared with the widget / Review page / CLI agents: docs/design/live-review-contract.md.
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, header},
    routing::{delete, get, patch, post},
};
use std::{sync::OnceLock, time::Duration};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::AppState;

pub mod api;
pub mod github;
pub mod model;
pub mod realtime;
pub mod rewrite;

static WEB_ORIGIN: OnceLock<String> = OnceLock::new();

/// The dashboard origin (always allowed for widget tokens: the Review page iframe).
pub fn web_origin() -> &'static str {
    WEB_ORIGIN.get().map(String::as_str).unwrap_or("")
}

/// Routes shared by both prefixes; `Ctx` tells member from widget token.
fn mount(prefix: &str) -> Router<AppState> {
    use api::*;
    let a = format!("{prefix}/annotations");
    let t = format!("{prefix}/text");
    Router::new()
        .route(&format!("{prefix}/members"), get(members))
        .route(&format!("{prefix}/events"), get(realtime::events))
        .route(&a, get(list).post(create))
        .route(
            &format!("{a}/{{id}}"),
            get(get_one).patch(update).delete(remove),
        )
        .route(&format!("{a}/{{id}}/history"), get(history))
        .route(&format!("{a}/{{id}}/comments"), post(comment))
        .route(
            &format!("{a}/{{id}}/comments/{{cid}}"),
            patch(edit_comment).delete(delete_comment),
        )
        .route(&format!("{a}/{{id}}/reactions"), post(react))
        .route(&format!("{a}/{{id}}/attachments"), post(attach))
        .route(
            &format!("{a}/{{id}}/attachments/{{aid}}"),
            get(attachment).delete(delete_attachment),
        )
        .route(&format!("{a}/{{id}}/apply"), post(apply))
        .route(
            &format!("{a}/{{id}}/rephrase"),
            post(rewrite::rephrase_handler),
        )
        .route(
            &format!("{a}/{{id}}/rephrase/apply"),
            post(rewrite::apply_handler),
        )
        .route(&t, post(create_text))
        .route(&format!("{t}/{{key}}"), get(get_text).put(put_text))
        .route(&format!("{t}/{{key}}/history"), get(text_history))
        .route(&format!("{t}/{{key}}/revert"), post(revert_text))
        .route(
            &format!("{t}/{{key}}/translations/{{locale}}"),
            delete(delete_translation),
        )
        .layer(DefaultBodyLimit::max(api::MAX_CAPTURE + 64 * 1024))
}

/// Member routes (`/api/projects/{slug}/review/…`), merged under the dashboard CORS.
pub fn routes() -> Router<AppState> {
    use api::*;
    let p = "/api/projects/{slug}/review";
    mount(p)
        .route(
            &format!("{p}/settings"),
            get(get_settings).put(put_settings),
        )
        .route(&format!("{p}/links"), get(list_links).post(create_link))
        .route(&format!("{p}/links/{{id}}"), delete(revoke_link))
        .route(&format!("{p}/session"), post(session))
        .route(
            &format!("{p}/from-feedback/{{group_id}}"),
            post(from_feedback),
        )
        .merge(github::routes())
}

/// Widget routes (`/api/review/w/…`, `glr_` tokens). Strict CORS: only origins declared by some project (or the
/// dashboard); `Ctx` then binds the origin to the token's own project. No credentials (Bearer only).
pub fn widget_routes(state: &AppState, web_origin: &str) -> Router<AppState> {
    let web = web_origin.trim_end_matches('/').to_string();
    let _ = WEB_ORIGIN.set(web.clone());
    if tokio::runtime::Handle::try_current().is_ok() {
        realtime::hub(&state.db);
    }
    let db = state.db.clone();
    // ponytail: one indexed lookup per cross-origin request; cache it if widget traffic grows.
    let allow = AllowOrigin::async_predicate(move |origin: HeaderValue, _| {
        let (db, web) = (db.clone(), web.clone());
        async move {
            let Ok(o) = origin.to_str() else { return false };
            o == web
                || sqlx::query_scalar::<_, bool>(
                    // Declared review origins, or the project's site URL (Settings → Your site).
                    "SELECT EXISTS (SELECT 1 FROM review_settings WHERE $1 = ANY(origins))
                         OR EXISTS (SELECT 1 FROM preview_settings
                                    WHERE base_url = $1 OR starts_with(base_url, $1 || '/'))",
                )
                .bind(o)
                .fetch_one(&db)
                .await
                .unwrap_or(false)
        }
    });
    let cors = CorsLayer::new()
        .allow_origin(allow)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("last-event-id"),
        ])
        .max_age(Duration::from_secs(600));
    mount("/api/review/w")
        .route("/api/review/w/context", get(api::context))
        .layer(cors)
}
