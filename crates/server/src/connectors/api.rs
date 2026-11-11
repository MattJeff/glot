//! Dashboard routes (`/api/projects/{slug}/connectors…`) and the public webhook endpoint.
//! Credentials go in, never out: views list which secret fields are set, not their values.
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::types::Json as DbJson;
use uuid::Uuid;

use super::{
    KINDS, registry,
    sync::{self, AccountRow, COLS},
};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

pub fn routes() -> Router<AppState> {
    let base = "/api/projects/{slug}/connectors";
    Router::new()
        .route(base, get(list).post(create))
        .route(
            &format!("{base}/{{id}}"),
            axum::routing::patch(update).delete(delete),
        )
        .route(&format!("{base}/{{id}}/test"), post(test))
        .route(&format!("{base}/{{id}}/sync"), post(sync_now))
        .route(&format!("{base}/{{id}}/status"), get(status))
}

/// Outside the CORS layer: called by the CMS, authenticated by signature or token.
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/api/connectors/{id}/webhook", post(webhook))
}

#[derive(Serialize)]
pub struct View {
    id: Uuid,
    kind: String,
    name: String,
    settings: Value,
    /// Names of the credential fields that are set (values never leave the server).
    secret_fields: Vec<String>,
    status: String,
    last_sync_at: Option<DateTime<Utc>>,
    last_push_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    stats: Value,
    /// Generic webhook URL (secret token included): for CMSs that cannot sign their webhooks.
    webhook_url: String,
    created_at: DateTime<Utc>,
}

fn public_url() -> String {
    std::env::var("GLOT_PUBLIC_URL")
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_else(|_| "http://localhost:8790".into())
}

/// Webhook URL without the token (the one subscribed in CMSs that sign their calls).
pub fn webhook_url(id: Uuid) -> String {
    format!("{}/api/connectors/{id}/webhook", public_url())
}

fn view(state: &AppState, a: &AccountRow) -> View {
    let secret_fields = sync::creds(state, a)
        .ok()
        .and_then(|c| c.secrets.as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default();
    View {
        id: a.id,
        kind: a.kind.clone(),
        name: a.name.clone(),
        settings: a.settings.0.clone(),
        secret_fields,
        status: a.status.clone(),
        last_sync_at: a.last_sync_at,
        last_push_at: a.last_push_at,
        last_error: a.last_error.clone(),
        stats: a.stats.0.clone(),
        webhook_url: format!("{}?token={}", webhook_url(a.id), a.webhook_secret),
        created_at: a.created_at,
    }
}

async fn load(state: &AppState, project: Uuid, id: Uuid) -> ApiResult<AccountRow> {
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM connector_accounts WHERE id = $1 AND project_id = $2"
    ))
    .bind(id)
    .bind(project)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("connector not found"))
}

fn bad(e: anyhow::Error) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "connector_error", format!("{e:#}"))
}

async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<View>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<AccountRow> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM connector_accounts WHERE project_id = $1 ORDER BY created_at"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(|a| view(&state, a)).collect()))
}

#[derive(Deserialize)]
pub struct CreateBody {
    kind: String,
    #[serde(default)]
    name: Option<String>,
    secrets: Value,
    #[serde(default)]
    settings: Option<Value>,
}

fn object(v: &Value, what: &str) -> ApiResult<()> {
    if v.is_object() {
        Ok(())
    } else {
        Err(AppError::bad_request(format!("{what} must be an object")))
    }
}

async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let c = registry(&b.kind).ok_or_else(|| {
        AppError::bad_request(format!(
            "unknown or unavailable connector '{}' (known: {})",
            b.kind,
            KINDS.join(", ")
        ))
    })?;
    object(&b.secrets, "secrets")?;
    let settings = b.settings.unwrap_or_else(|| json!({}));
    object(&settings, "settings")?;
    let creds = super::Creds {
        secrets: b.secrets.clone(),
        settings: settings.clone(),
        since: None,
    };
    let info = c.test(&creds).await.map_err(bad)?;
    let id = Uuid::new_v4();
    let secret = hex::encode(rand::random::<[u8; 24]>());
    let name = b
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| info.name.clone());
    let a: AccountRow = sqlx::query_as(&format!(
        "INSERT INTO connector_accounts (id, project_id, kind, name, creds_enc, settings, webhook_secret)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {COLS}"
    ))
    .bind(id)
    .bind(p.id)
    .bind(&b.kind)
    .bind(name.trim())
    .bind(crate::auth::encrypt(&state.secret, &b.secrets.to_string()))
    .bind(DbJson(&settings))
    .bind(&secret)
    .fetch_one(&state.db)
    .await?;
    let hook = c.register_webhooks(&creds, &webhook_url(id)).await.err();
    tracing::info!(project = %p.slug, kind = %b.kind, account = %id, "connector created");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "connector": view(&state, &a),
            "info": info,
            "source_locale_mismatch": info.source_locale.as_ref().is_some_and(|l| !l.eq_ignore_ascii_case(&p.source_locale)
                && !l.split(['-', '_']).next().unwrap_or("").eq_ignore_ascii_case(&p.source_locale)),
            "webhooks_error": hook.map(|e| format!("{e:#}")),
        })),
    ))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    #[serde(default)]
    name: Option<String>,
    /// Merged into the stored credentials; an empty string value removes that field.
    #[serde(default)]
    secrets: Option<Value>,
    #[serde(default)]
    settings: Option<Value>,
}

async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<UpdateBody>,
) -> ApiResult<Json<View>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    let mut secrets = sync::creds(&state, &a)?.secrets;
    if let Some(s) = &b.secrets {
        object(s, "secrets")?;
        let o = secrets
            .as_object_mut()
            .ok_or_else(|| AppError::internal("stored secrets not an object"))?;
        for (k, v) in s.as_object().into_iter().flatten() {
            if v.as_str() == Some("") || v.is_null() {
                o.remove(k);
            } else {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(s) = &b.settings {
        object(s, "settings")?;
    }
    let row: AccountRow = sqlx::query_as(&format!(
        "UPDATE connector_accounts SET name = coalesce($3, name), creds_enc = $4, settings = coalesce($5, settings)
         WHERE id = $1 AND project_id = $2 RETURNING {COLS}"
    ))
    .bind(id)
    .bind(p.id)
    .bind(b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
    .bind(crate::auth::encrypt(&state.secret, &secrets.to_string()))
    .bind(b.settings.map(DbJson))
    .fetch_one(&state.db)
    .await?;
    Ok(Json(view(&state, &row)))
}

async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let n = sqlx::query("DELETE FROM connector_accounts WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(p.id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("connector not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn test(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    let c = registry(&a.kind).ok_or_else(|| AppError::bad_request("connector unavailable"))?;
    let creds = sync::creds(&state, &a)?;
    let info = c.test(&creds).await.map_err(bad)?;
    let locales = c.locales(&creds).await.map_err(bad)?;
    let hook = c.register_webhooks(&creds, &webhook_url(a.id)).await.err();
    Ok(Json(json!({
        "info": info,
        "locales": locales,
        "webhooks_error": hook.map(|e| format!("{e:#}")),
    })))
}

#[derive(Deserialize, Default)]
pub struct SyncBody {
    /// Incremental: only content changed since the last sync (when the CMS can filter).
    #[serde(default)]
    incremental: bool,
}

async fn sync_now(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<View>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    let b: SyncBody = if body.is_empty() {
        SyncBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?
    };
    sync::queue(
        &state.db,
        a.id,
        if b.incremental { a.last_sync_at } else { None },
    )
    .await?;
    let a = load(&state, p.id, id).await?;
    Ok((StatusCode::ACCEPTED, Json(view(&state, &a))))
}

async fn status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let a = load(&state, p.id, id).await?;
    let job: Option<(String, i32, i32)> = match a.pending_job {
        Some(j) => {
            sqlx::query_as("SELECT status, progress_done, progress_total FROM jobs WHERE id = $1")
                .bind(j)
                .fetch_optional(&state.db)
                .await?
        }
        None => None,
    };
    let pushed: Vec<(String, i64)> = sqlx::query_as(
        "SELECT l, count(*) FROM connector_items i, jsonb_object_keys(i.pushed) AS l WHERE i.account_id = $1 GROUP BY l ORDER BY l",
    )
    .bind(a.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "connector": view(&state, &a),
        "job": job.map(|(s, d, t)| json!({"id": a.pending_job, "status": s, "done": d, "total": t})),
        "pushed_per_locale": pushed.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
    })))
}

#[derive(Deserialize)]
pub struct HookQuery {
    #[serde(default)]
    token: Option<String>,
}

async fn webhook(
    State(state): State<AppState>,
    AppPath(id): AppPath<Uuid>,
    AppQuery(q): AppQuery<HookQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let a: AccountRow = sqlx::query_as(&format!(
        "SELECT {COLS} FROM connector_accounts WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("connector not found"))?;
    let c = registry(&a.kind).ok_or_else(|| AppError::not_found("connector not found"))?;
    let creds = sync::creds(&state, &a)?;
    let secret = creds.opt("webhook_secret").unwrap_or(&a.webhook_secret);
    let ok = q
        .token
        .as_deref()
        .is_some_and(|t| super::secret_eq(t, &a.webhook_secret))
        || c.verify_webhook(&headers, &body, secret);
    if !ok {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "webhook signature or token invalid",
        ));
    }
    sync::queue(&state.db, a.id, a.last_sync_at).await?;
    Ok(StatusCode::ACCEPTED)
}
