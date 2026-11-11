//! SPA snapshots rendered in the customer's CI, for projects that run no browser on the glot server.
//!
//! `POST /edge/v1/{project}/snapshots` (`Authorization: Bearer <API token>` of a project admin, what
//! `glot prerender --upload` sends):
//! `{"snapshots": [{"path": "/pricing", "html": "<!DOCTYPE html>…", "rendered_at": 1731300000000}]}`
//! → `{"imported": 1, "rejected": [{"path": "…", "error": "…"}]}`. A path is the page's path on the origin
//! (query and fragment ignored); a re-import replaces it. The proxy serves these before asking the renderer
//! service (`rendering: prerender`, or `auto` on an empty shell), translated at serve time like any snapshot.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

/// Per request / per page / per project.
const MAX_ITEMS: usize = 100;
pub const MAX_HTML: usize = 5 * 1024 * 1024;
const MAX_PATHS: i64 = 2000;
const MAX_PATH_LEN: usize = 2048;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/edge/v1/{project}/snapshots", post(import))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
}

#[derive(Deserialize)]
struct Body {
    snapshots: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    path: String,
    html: String,
    /// Unix ms (the renderer's manifest); default now.
    #[serde(default)]
    rendered_at: Option<i64>,
}

/// `/pricing?x#y` → `/pricing`; None if it is not an origin path.
pub fn clean_path(raw: &str) -> Option<&str> {
    let p = raw.split(['?', '#']).next()?;
    (p.starts_with('/') && !p.starts_with("//") && p.len() <= MAX_PATH_LEN && !p.contains('\\'))
        .then_some(p)
}

async fn import(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(body): AppJson<Body>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if body.snapshots.len() > MAX_ITEMS {
        return Err(AppError::bad_request(format!(
            "at most {MAX_ITEMS} snapshots per request"
        )));
    }
    let mut rejected = Vec::new();
    let mut imported = 0;
    let mut tx = state.db.begin().await?;
    for s in &body.snapshots {
        let error = match clean_path(&s.path) {
            None => Some("path must start with / (the page's path on the origin)"),
            Some(_) if s.html.len() > MAX_HTML => Some("html larger than 5 MiB"),
            Some(_) if s.html.trim().is_empty() => Some("empty html"),
            Some(_) => None,
        };
        if let Some(e) = error {
            rejected.push(json!({"path": s.path, "error": e}));
            continue;
        }
        let at = s
            .rendered_at
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .unwrap_or_else(Utc::now);
        sqlx::query(
            "INSERT INTO edge_snapshots (project_id, path, html, rendered_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (project_id, path) DO UPDATE
             SET html = EXCLUDED.html, rendered_at = EXCLUDED.rendered_at, imported_at = now()",
        )
        .bind(p.id)
        .bind(clean_path(&s.path))
        .bind(&s.html)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        imported += 1;
    }
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM edge_snapshots WHERE project_id = $1")
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await?;
    if n > MAX_PATHS {
        return Err(AppError::bad_request(format!(
            "at most {MAX_PATHS} snapshot paths per project"
        )));
    }
    tx.commit().await?;
    Ok(Json(json!({"imported": imported, "rejected": rejected})))
}

/// The imported snapshot of `path` on the project's origin (query ignored).
pub async fn imported(db: &sqlx::PgPool, project: Uuid, path: &str) -> Option<Vec<u8>> {
    let path = clean_path(path)?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT html FROM edge_snapshots WHERE project_id = $1 AND path = $2")
            .bind(project)
            .bind(path)
            .fetch_optional(db)
            .await
            .ok()?;
    row.map(|(h,)| h.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::clean_path;

    #[test]
    fn paths() {
        assert_eq!(clean_path("/pricing?x=1#a"), Some("/pricing"));
        assert_eq!(clean_path("/"), Some("/"));
        assert_eq!(clean_path("pricing"), None);
        assert_eq!(clean_path("//evil.com/x"), None);
        assert_eq!(clean_path("https://x/y"), None);
    }
}
