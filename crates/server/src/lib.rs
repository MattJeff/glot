#![allow(clippy::type_complexity)] // ponytail: local tuple types in queries; name them if they spread
//! glot-server: HTTP API (axum + Postgres), translation jobs, CDN/OTA, SSE.
use axum::{
    Json, Router,
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::{Notify, broadcast};
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};

pub mod auth;
pub mod cdn;
pub mod glossary;
pub mod insights;
pub mod jobs;
pub mod keys;
pub mod openapi;
pub mod projects;
pub mod review;
pub mod sync;
pub mod visual;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// AES-256 key derived from GLOT_SECRET_KEY.
    pub secret: [u8; 32],
    pub secure_cookies: bool,
    pub job_events: broadcast::Sender<jobs::Job>,
    pub job_wakeup: Arc<Notify>,
}

impl AppState {
    pub fn new(db: PgPool, secret_key: &str, secure_cookies: bool) -> Self {
        use sha2::Digest;
        let (job_events, _) = broadcast::channel(256);
        Self {
            db,
            secret: sha2::Sha256::digest(secret_key.as_bytes()).into(),
            secure_cookies,
            job_events,
            job_wakeup: Arc::new(Notify::new()),
        }
    }
}

pub fn router(state: AppState, web_origin: &str) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(
            web_origin
                .parse::<HeaderValue>()
                .expect("invalid GLOT_WEB_ORIGIN"),
        )
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let p = "/api/projects/{slug}";
    let api = Router::new()
        .route("/api/auth/signup", post(auth::signup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route(
            "/api/tokens",
            get(auth::list_tokens).post(auth::create_token),
        )
        .route(
            "/api/tokens/{id}",
            axum::routing::delete(auth::delete_token),
        )
        .route("/api/projects", get(projects::list).post(projects::create))
        .route(
            p,
            get(projects::get)
                .patch(projects::update)
                .delete(projects::delete),
        )
        .route(&format!("{p}/keys"), get(keys::list).post(keys::create))
        .route(
            &format!("{p}/keys/{{key}}"),
            axum::routing::patch(keys::update).delete(keys::delete),
        )
        .route(
            &format!("{p}/keys/{{key}}/translations/{{locale}}"),
            put(keys::put_translation),
        )
        .route(
            &format!("{p}/keys/{{key}}/translations/{{locale}}/review"),
            post(keys::review),
        )
        .route(&format!("{p}/keys/{{key}}/history"), get(keys::history))
        .route(&format!("{p}/issues"), get(keys::issues))
        .route(
            &format!("{p}/glossary"),
            get(glossary::list).post(glossary::create),
        )
        .route(
            &format!("{p}/glossary/{{id}}"),
            axum::routing::patch(glossary::update).delete(glossary::delete),
        )
        .route(&format!("{p}/jobs"), get(jobs::list).post(jobs::create))
        .route("/api/jobs/{id}", get(jobs::get))
        .route("/api/jobs/{id}/events", get(jobs::events))
        .route(&format!("{p}/sync/push"), post(sync::push))
        .route(&format!("{p}/sync/pull"), get(sync::pull))
        .route(&format!("{p}/review"), post(review::review_many))
        .route(&format!("{p}/keys/{{key}}/review"), get(review::review_one))
        .route("/api/openapi.json", get(openapi::spec))
        .merge(visual::routes())
        .merge(insights::routes())
        .layer(cors);

    Router::new()
        .merge(api)
        .merge(insights::public_routes())
        .route("/cdn/{slug}/{file}", get(cdn::serve))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(|| async { AppError::not_found("route not found") })
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ---------- errors ----------

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

pub type ApiResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "validation_error", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication required",
        )
    }
    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "insufficient permissions",
        )
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!("internal error: {e}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal server error",
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": { "code": self.code, "message": self.message } });
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => Self::not_found("not found"),
            sqlx::Error::Database(d) if d.is_unique_violation() => Self::conflict("already exists"),
            _ => Self::internal(e),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(e)
    }
}

/// Rejections of malformed JSON bodies use the same error envelope.
pub struct AppJson<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequest<S> for AppJson<T> {
    type Rejection = AppError;
    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(v)| AppJson(v))
            .map_err(|e| AppError::bad_request(e.body_text()))
    }
}

/// Path/query rejections (e.g. a malformed uuid) also use the error envelope.
pub struct AppPath<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned + Send> axum::extract::FromRequestParts<S>
    for AppPath<T>
{
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(v)| AppPath(v))
            .map_err(|e| AppError::bad_request(e.body_text()))
    }
}

pub struct AppQuery<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequestParts<S>
    for AppQuery<T>
{
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(v)| AppQuery(v))
            .map_err(|e| AppError::bad_request(e.body_text()))
    }
}

// ---------- validation helpers ----------

pub fn validate_slug(s: &str) -> ApiResult<()> {
    let ok = (2..=63).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "slug must be 2-63 chars of a-z, 0-9 and '-'",
        ))
    }
}

/// BCP 47-ish: `en`, `pt-BR`, `zh-Hant-TW`.
pub fn validate_locale(s: &str) -> ApiResult<()> {
    let mut parts = s.split('-');
    let lang = parts.next().unwrap_or("");
    let ok = (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_alphabetic())
        && parts
            .all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()));
    if ok {
        Ok(())
    } else {
        Err(AppError::bad_request(format!("invalid locale '{s}'")))
    }
}

pub fn validate_key(k: &str) -> ApiResult<()> {
    let ok = !k.is_empty()
        && k.len() <= 255
        && !k.starts_with('.')
        && !k.ends_with('.')
        && !k.contains("..")
        && !k.chars().any(|c| c.is_whitespace() || c == '/');
    if ok {
        Ok(())
    } else {
        Err(AppError::bad_request(format!("invalid key '{k}'")))
    }
}
