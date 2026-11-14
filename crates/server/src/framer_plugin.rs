//! Routes for the glot Framer plugin (integrations/framer-plugin): the plugin runs in the Framer editor, on a
//! Framer origin, so it needs CORS. Same handlers as the dashboard API, under `/api/plugin`, callable from any
//! origin but with an API token only (`Authorization: Bearer glot_…`): no cookie session is ever honoured here.
use axum::{
    Router,
    extract::Request,
    http::{Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::cors::{Any, CorsLayer};

use crate::{AppError, AppState, jobs, sync};

async fn bearer_only(req: Request, next: Next) -> Response {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer "));
    if bearer || req.method() == Method::OPTIONS {
        next.run(req).await
    } else {
        AppError::unauthorized().into_response()
    }
}

pub fn public_routes() -> Router<AppState> {
    let p = "/api/plugin/projects/{slug}";
    Router::new()
        .route(
            &format!("{p}/sync/push"),
            post(sync::push).layer(axum::extract::DefaultBodyLimit::max(sync::PUSH_LIMIT)),
        )
        .route(&format!("{p}/sync/pull"), get(sync::pull))
        .route(&format!("{p}/jobs"), post(jobs::create))
        .route("/api/plugin/jobs/{id}", get(jobs::get))
        .layer(middleware::from_fn(bearer_only))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]),
        )
}
