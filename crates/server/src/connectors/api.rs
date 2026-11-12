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
    AccountInfo, Check, KINDS, PushItem, fix_code, oauth, registry,
    sync::{self, AccountRow, COLS},
};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
};

pub fn routes() -> Router<AppState> {
    let base = "/api/projects/{slug}/connectors";
    let id = format!("{base}/{{id}}");
    Router::new()
        .route(base, get(list).post(create))
        .route(&id, axum::routing::patch(update).delete(delete))
        .route(&format!("{id}/test"), post(test))
        .route(&format!("{id}/doctor"), post(doctor))
        .route(&format!("{id}/sync"), post(sync_now))
        .route(&format!("{id}/hold"), post(hold))
        .route(&format!("{id}/publish"), post(publish))
        .route(&format!("{id}/diff"), get(diff))
        .route(&format!("{id}/runs"), get(runs))
        .route(&format!("{id}/runs/{{run}}/rollback"), post(rollback))
        .route(&format!("{id}/status"), get(status))
        .merge(oauth::routes())
}

/// Outside the CORS layer: called by the CMS, authenticated by signature or token.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/connectors/{id}/webhook", post(webhook))
        .merge(oauth::public_routes())
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
    /// `translate` = preview / paused (nothing translated, nothing written); `push` = translated, not published.
    hold: Option<String>,
    last_sync_at: Option<DateTime<Utc>>,
    last_push_at: Option<DateTime<Utc>>,
    /// Last change the platform notified: webhooks work.
    last_webhook_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    /// What to do about `last_error` (`connectors.fix.<code>`).
    fix: Option<&'static str>,
    /// The platform no longer accepts the credentials: the card shows Reconnect.
    reconnect: bool,
    /// Access token expiry, for credentials that renew themselves.
    token_expires_at: Option<DateTime<Utc>>,
    stats: Value,
    /// Items the last push did not write, each with its fix.
    issues: Vec<Value>,
    /// Translation job of the sync in progress.
    job: Option<Value>,
    /// Generic webhook URL (secret token included): for CMSs that cannot sign their webhooks.
    webhook_url: String,
    created_at: DateTime<Utc>,
}

pub(super) fn public_url() -> String {
    std::env::var("GLOT_PUBLIC_URL")
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_else(|_| "http://localhost:8790".into())
}

/// Webhook URL without the token (the one subscribed in CMSs that sign their calls).
pub fn webhook_url(id: Uuid) -> String {
    format!("{}/api/connectors/{id}/webhook", public_url())
}

fn view(state: &AppState, a: &AccountRow) -> View {
    let secrets = sync::creds(state, a)
        .map(|c| c.secrets)
        .unwrap_or(Value::Null);
    let fix = a.last_error.as_deref().and_then(fix_code);
    let issues = a.stats.0["last_push"]["failed"]
        .as_array()
        .into_iter()
        .flatten()
        .take(20)
        .map(|f| {
            let error = f[1].as_str().unwrap_or_default();
            json!({"item": f[0], "error": error, "fix": fix_code(error)})
        })
        .collect();
    View {
        id: a.id,
        kind: a.kind.clone(),
        name: a.name.clone(),
        settings: a.settings.0.clone(),
        // Internal token bookkeeping is not a field the user filled.
        secret_fields: secrets
            .as_object()
            .map(|o| {
                o.keys()
                    .filter(|k| {
                        !["expires_at", "refresh_expires_at", "scope", "oauth"]
                            .contains(&k.as_str())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        status: a.status.clone(),
        hold: a.hold.clone(),
        last_sync_at: a.last_sync_at,
        last_push_at: a.last_push_at,
        last_webhook_at: a.last_webhook_at,
        last_error: a.last_error.clone(),
        fix,
        reconnect: fix == Some("reconnect"),
        token_expires_at: secrets["expires_at"]
            .as_i64()
            .and_then(|t| DateTime::from_timestamp(t, 0)),
        stats: a.stats.0.clone(),
        issues,
        job: None,
        webhook_url: format!("{}?token={}", webhook_url(a.id), a.webhook_secret),
        created_at: a.created_at,
    }
}

async fn job_of(state: &AppState, a: &AccountRow) -> ApiResult<Option<Value>> {
    let Some(j) = a.pending_job else {
        return Ok(None);
    };
    let job: Option<(String, i32, i32)> =
        sqlx::query_as("SELECT status, progress_done, progress_total FROM jobs WHERE id = $1")
            .bind(j)
            .fetch_optional(&state.db)
            .await?;
    Ok(job.map(|(s, d, t)| json!({"id": j, "status": s, "done": d, "total": t})))
}

async fn full_view(state: &AppState, a: &AccountRow) -> ApiResult<View> {
    Ok(View {
        job: job_of(state, a).await?,
        ..view(state, a)
    })
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
    let mut out = vec![];
    for a in &rows {
        out.push(full_view(&state, a).await?);
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct CreateBody {
    kind: String,
    #[serde(default)]
    name: Option<String>,
    secrets: Value,
    #[serde(default)]
    settings: Option<Value>,
    /// The first sync stops after the pull and waits for an approval (the dashboard's preview).
    #[serde(default)]
    preview: bool,
}

fn object(v: &Value, what: &str) -> ApiResult<()> {
    if v.is_object() {
        Ok(())
    } else {
        Err(AppError::bad_request(format!("{what} must be an object")))
    }
}

/// Tests the credentials, stores them encrypted (a new account, or `replace`: the one reconnected),
/// subscribes the webhooks and queues the first sync.
#[allow(clippy::too_many_arguments)]
pub(super) async fn connect(
    state: &AppState,
    p: &ProjectRow,
    kind: &str,
    name: Option<String>,
    secrets: Value,
    settings: Value,
    hold: Option<&str>,
    replace: Option<Uuid>,
) -> ApiResult<(AccountRow, AccountInfo, Option<String>)> {
    let c = registry(kind).ok_or_else(|| {
        AppError::bad_request(format!(
            "unknown or unavailable connector '{kind}' (known: {})",
            KINDS.join(", ")
        ))
    })?;
    object(&secrets, "secrets")?;
    object(&settings, "settings")?;
    let mut secrets = oauth::prepare(kind, secrets).await.map_err(bad)?;
    // Platforms where glot chooses the webhook secret itself: no reason to ask the user for one.
    if matches!(kind, "woocommerce" | "bigcommerce" | "ghost")
        && secrets["webhook_secret"].as_str().is_none_or(str::is_empty)
    {
        secrets["webhook_secret"] = json!(hex::encode(rand::random::<[u8; 24]>()));
    }
    let creds = super::Creds {
        secrets: secrets.clone(),
        settings: settings.clone(),
        since: None,
    };
    let info = c.test(&creds).await.map_err(bad)?;
    let enc = crate::auth::encrypt(&state.secret, &secrets.to_string());
    let id = match replace {
        Some(id) => sqlx::query_scalar(
            "UPDATE connector_accounts SET creds_enc = $3, settings = settings || $4, last_error = NULL,
               status = CASE WHEN status = 'error' THEN 'idle' ELSE status END
             WHERE id = $1 AND project_id = $2 AND kind = $5 RETURNING id",
        )
        .bind(id)
        .bind(p.id)
        .bind(&enc)
        .bind(DbJson(&settings))
        .bind(kind)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("connector not found"))?,
        None => {
            let name = name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| info.name.clone());
            sqlx::query_scalar(
                "INSERT INTO connector_accounts (id, project_id, kind, name, creds_enc, settings, webhook_secret, hold)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(p.id)
            .bind(kind)
            .bind(name.trim())
            .bind(&enc)
            .bind(DbJson(&settings))
            .bind(hex::encode(rand::random::<[u8; 24]>()))
            .bind(hold)
            .fetch_one(&state.db)
            .await?
        }
    };
    let hook = c.register_webhooks(&creds, &webhook_url(id)).await.err();
    sync::queue(&state.db, id, None).await?;
    tracing::info!(project = %p.slug, kind = %kind, account = %id, reconnect = replace.is_some(), "connector connected");
    Ok((
        load(state, p.id, id).await?,
        info,
        hook.map(|e| format!("{e:#}")),
    ))
}

fn locale_mismatch(info: &AccountInfo, p: &ProjectRow) -> bool {
    info.source_locale.as_ref().is_some_and(|l| {
        !l.eq_ignore_ascii_case(&p.source_locale)
            && !l
                .split(['-', '_'])
                .next()
                .unwrap_or("")
                .eq_ignore_ascii_case(&p.source_locale)
    })
}

async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let (a, info, hook) = connect(
        &state,
        &p,
        &b.kind,
        b.name,
        b.secrets,
        b.settings.unwrap_or_else(|| json!({})),
        b.preview.then_some("translate"),
        None,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "connector": view(&state, &a),
            "info": info,
            "source_locale_mismatch": locale_mismatch(&info, &p),
            "webhooks_error": hook,
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
    // Another selection of content types: the next sync must pull everything again to apply it.
    let reselect = b
        .settings
        .as_ref()
        .is_some_and(|s| s.get("exclude") != a.settings.0.get("exclude"));
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
    if reselect {
        sync::queue(&state.db, id, None).await?;
    }
    Ok(Json(
        full_view(&state, &load(&state, p.id, row.id).await?).await?,
    ))
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
    let creds = oauth::fresh(&state, &a).await.map_err(bad)?;
    let info = c.test(&creds).await.map_err(bad)?;
    let locales = c.locales(&creds).await.map_err(bad)?;
    let hook = c.register_webhooks(&creds, &webhook_url(a.id)).await.err();
    Ok(Json(json!({
        "info": info,
        "locales": locales,
        "webhooks_error": hook.map(|e| format!("{e:#}")),
    })))
}

/// Whether the platform's language list covers `locale` (`fr` matches `fr`, `fr-FR`, `fr_FR`).
fn has_locale(list: &[String], locale: &str) -> bool {
    let norm = |s: &str| s.replace('_', "-").to_lowercase();
    let want = norm(locale);
    list.iter().map(|l| norm(l)).any(|l| {
        l == want
            || l.split('-').next() == Some(want.as_str())
            || want.split('-').next() == Some(l.as_str())
    })
}

/// "Test connection": every capability a sync needs, checked one by one against the platform, so a first
/// customer gets a diagnosis ("the token lacks write_translations") instead of a failed sync.
/// Read-only: nothing is written to the platform except the webhook subscription.
async fn doctor(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    let c = registry(&a.kind).ok_or_else(|| AppError::bad_request("connector unavailable"))?;
    let mut checks: Vec<Check> = vec![];
    let done = |checks: Vec<Check>| {
        let ok = checks.iter().all(|c| c.status != "fail");
        Ok(Json(json!({"ok": ok, "checks": checks})))
    };
    let creds = match oauth::fresh(&state, &a).await {
        Ok(c) => c,
        Err(e) => return done(vec![Check::fail("auth", format!("{e:#}"))]),
    };
    match c.test(&creds).await {
        Ok(info) => {
            checks.push(Check::ok("auth", info.name.clone()));
            if locale_mismatch(&info, &p) {
                checks.push(Check::warn(
                    "source_locale",
                    format!(
                        "{} ≠ {}",
                        info.source_locale.clone().unwrap_or_default(),
                        p.source_locale
                    ),
                    None,
                ));
            }
            checks.extend(
                info.notes
                    .iter()
                    .map(|n| Check::warn("note", n.clone(), None)),
            );
        }
        Err(e) => return done(vec![Check::fail("auth", format!("{e:#}"))]),
    }
    let map = &a.settings.0["locale_map"];
    let cms = |l: &String| map.get(l).and_then(Value::as_str).unwrap_or(l).to_string();
    match c.locales(&creds).await {
        Ok(list) => {
            let missing: Vec<String> = p
                .target_locales
                .iter()
                .map(cms)
                .filter(|l| !has_locale(&list, l))
                .collect();
            checks.push(if missing.is_empty() {
                Check::ok("languages", list.join(", "))
            } else {
                Check::warn("languages", missing.join(", "), Some("enable_locale"))
            });
        }
        Err(e) => checks.push(Check::fail("languages", format!("{e:#}"))),
    }
    match c.pull(&creds, None).await {
        Ok(page) if page.items.is_empty() => {
            checks.push(Check::warn("read", "0", Some("no_content")))
        }
        Ok(page) => {
            checks.push(Check::ok("read", page.items.len().to_string()));
            let probe: Vec<PushItem> = page
                .items
                .iter()
                .take(5)
                .map(|i| PushItem {
                    external_id: i.external_id.clone(),
                    field: i.field.clone(),
                    value: i.text.clone(),
                    format: i.format.clone(),
                    meta: i.meta.clone(),
                })
                .collect();
            if let Some(locale) = p.target_locales.first() {
                checks.push(match c.current(&creds, &cms(locale), &probe).await {
                    Ok(Some(_)) => Check::ok("translations", ""),
                    Ok(None) => Check::warn("translations", "", Some("no_readback")),
                    Err(e) => Check::fail("translations", format!("{e:#}")),
                });
            }
        }
        Err(e) => checks.push(Check::fail("read", format!("{e:#}"))),
    }
    checks.push(
        match c.register_webhooks(&creds, &webhook_url(a.id)).await {
            Ok(()) => Check::ok("webhooks", ""),
            Err(e) => Check::warn("webhooks", format!("{e:#}"), Some("webhook_manual")),
        },
    );
    checks.extend(c.doctor(&creds).await);
    done(checks)
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

#[derive(Deserialize)]
pub struct HoldBody {
    /// `null` = automatic (approve the preview, resume), `translate` = pause, `push` = review before publishing.
    hold: Option<String>,
}

/// Approve the preview / pause / resume / review-before-publish. Resuming queues a sync.
async fn hold(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<HoldBody>,
) -> ApiResult<Json<View>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    if !matches!(b.hold.as_deref(), None | Some("translate" | "push")) {
        return Err(AppError::bad_request(
            "hold must be null, 'translate' or 'push'",
        ));
    }
    sqlx::query("UPDATE connector_accounts SET hold = $2 WHERE id = $1")
        .bind(a.id)
        .bind(&b.hold)
        .execute(&state.db)
        .await?;
    if b.hold.as_deref() != Some("translate") && a.hold.as_deref() == Some("translate") {
        sync::queue(&state.db, a.id, None).await?;
    }
    Ok(Json(
        full_view(&state, &load(&state, p.id, id).await?).await?,
    ))
}

/// Writes now what is ready (the explicit "Publish" of an account held at `push`).
async fn publish(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    let res = sync::push_ready(&state, &a).await;
    sqlx::query(
        "UPDATE connector_accounts SET last_push_at = now(), last_error = $2 WHERE id = $1",
    )
    .bind(a.id)
    .bind(res.as_ref().err().map(|e| format!("push: {e:#}")))
    .execute(&state.db)
    .await?;
    let a = load(&state, p.id, id).await?;
    Ok(Json(
        json!({"connector": full_view(&state, &a).await?, "error": res.err().map(|e| format!("{e:#}"))}),
    ))
}

#[derive(Deserialize)]
pub struct DiffQuery {
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Dry run: what the next push would write (source, translation, new or update), without calling the platform.
async fn diff(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppQuery(q): AppQuery<DiffQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let a = load(&state, p.id, id).await?;
    let (all, waiting, unchanged) = sync::pending(&state, &a).await.map_err(bad)?;
    let all: Vec<&sync::Pending> = all
        .iter()
        .filter(|w| q.locale.as_ref().is_none_or(|l| *l == w.locale))
        .collect();
    let rows: Vec<Value> = all
        .iter()
        .take(q.limit.unwrap_or(50).min(500))
        .map(|w| {
            json!({"locale": w.locale, "id": w.item.external_id, "field": w.item.field, "source": w.source,
                "value": w.item.value, "update": w.update, "blocked": w.blocked,
                "fix": w.blocked.as_deref().and_then(fix_code)})
        })
        .collect();
    Ok(Json(
        json!({"total": all.len(), "waiting": waiting, "unchanged": unchanged, "rows": rows}),
    ))
}

async fn runs(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Vec<Value>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let a = load(&state, p.id, id).await?;
    let rows: Vec<(Uuid, DateTime<Utc>, Value, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, started_at, stats, rolled_back_at FROM connector_runs WHERE account_id = $1 ORDER BY started_at DESC LIMIT 20",
    )
    .bind(a.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, at, stats, back)| json!({"id": id, "started_at": at, "stats": stats, "rolled_back_at": back}))
            .collect(),
    ))
}

async fn rollback(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id, run)): AppPath<(String, Uuid, Uuid)>,
) -> ApiResult<Json<sync::Rollback>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let a = load(&state, p.id, id).await?;
    Ok(Json(sync::rollback(&state, &a, run).await.map_err(bad)?))
}

async fn status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let a = load(&state, p.id, id).await?;
    let pushed: Vec<(String, i64)> = sqlx::query_as(
        "SELECT l, count(*) FROM connector_items i, jsonb_object_keys(i.pushed) AS l WHERE i.account_id = $1 GROUP BY l ORDER BY l",
    )
    .bind(a.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "connector": view(&state, &a),
        "job": job_of(&state, &a).await?,
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
    let secret = oauth::signing_secret(&a.kind, &creds).unwrap_or_else(|| a.webhook_secret.clone());
    let ok = q
        .token
        .as_deref()
        .is_some_and(|t| super::secret_eq(t, &a.webhook_secret))
        || c.verify_webhook(&headers, &body, &secret);
    if !ok {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "webhook signature or token invalid",
        ));
    }
    sqlx::query("UPDATE connector_accounts SET last_webhook_at = now() WHERE id = $1")
        .bind(a.id)
        .execute(&state.db)
        .await?;
    sync::queue(&state.db, a.id, a.last_sync_at).await?;
    Ok(StatusCode::ACCEPTED)
}
