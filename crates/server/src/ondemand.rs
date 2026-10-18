//! On-demand locales ("translation as infrastructure"): the first `GET /cdn/{slug}/{locale}.json?ondemand=1`
//! for a locale the project doesn't ship queues the normal translate job (glossary, validation, budget) and
//! answers 202; once the job is done the CDN serves the catalog like any other locale.
//! Jobs carry `params.ondemand_locale`, so readiness, the monthly budget and the daily cap are job queries.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
    validate_locale,
};

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Settings {
    /// Whitelist; `["*"]` = any locale, `[]` = off.
    pub locales: Vec<String>,
    /// Monthly cap on what on-demand jobs may spend; `null` = no cap beyond `llm.budget_usd` per job.
    pub monthly_budget_usd: Option<f64>,
    /// On-demand jobs per rolling 24 h (the CDN URL is public: this bounds what a crawler of random locales costs).
    pub per_day: i32,
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/projects/{slug}/ondemand",
        get(get_settings).put(put_settings),
    )
}

async fn get_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let (locales, monthly_budget_usd, per_day) = sqlx::query_as(
        "SELECT ondemand_locales, ondemand_budget_usd, ondemand_per_day FROM projects WHERE id = $1",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(Settings {
        locales,
        monthly_budget_usd,
        per_day,
    }))
}

async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Settings>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    for l in &b.locales {
        if l != "*" {
            validate_locale(l)?;
        }
    }
    if b.monthly_budget_usd
        .is_some_and(|v| !v.is_finite() || v < 0.0)
    {
        return Err(AppError::bad_request("monthly_budget_usd must be >= 0"));
    }
    if !(0..=1000).contains(&b.per_day) {
        return Err(AppError::bad_request("per_day must be 0-1000"));
    }
    sqlx::query("UPDATE projects SET ondemand_locales = $2, ondemand_budget_usd = $3, ondemand_per_day = $4 WHERE id = $1")
        .bind(p.id)
        .bind(&b.locales)
        .bind(b.monthly_budget_usd)
        .bind(b.per_day)
        .execute(&state.db)
        .await?;
    Ok(Json(b))
}

/// (any done job, any queued/running job) for this on-demand locale.
async fn status<'e>(
    db: impl sqlx::PgExecutor<'e>,
    project: Uuid,
    locale: &str,
) -> sqlx::Result<(bool, bool)> {
    let (done, pending): (Option<bool>, Option<bool>) = sqlx::query_as(
        "SELECT bool_or(status = 'done'), bool_or(status IN ('queued', 'running')) FROM jobs
         WHERE project_id = $1 AND params ? 'ondemand_locale' AND params->>'ondemand_locale' = $2",
    )
    .bind(project)
    .bind(locale)
    .fetch_one(db)
    .await?;
    Ok((done == Some(true), pending == Some(true)))
}

/// CDN gate for a locale outside `target_locales`: `None` = a catalog exists, serve it;
/// `Some(202)` = being translated; errors otherwise (404 when not asked with `?ondemand=1`).
pub async fn gate(
    state: &AppState,
    p: &ProjectRow,
    locale: &str,
    trigger: bool,
) -> ApiResult<Option<Response>> {
    let not_found = || AppError::not_found("catalog not found");
    if validate_locale(locale).is_err() {
        return Err(not_found());
    }
    let (done, pending) = status(&state.db, p.id, locale).await?;
    if done {
        return Ok(None);
    }
    if pending {
        return Ok(Some(accepted(p.public_cdn)));
    }
    if !trigger {
        return Err(not_found());
    }

    // Row lock on the project: concurrent first visits (and the daily cap) are serialized across replicas.
    let mut tx = state.db.begin().await?;
    let (allowed, budget, per_day): (Vec<String>, Option<f64>, i32) = sqlx::query_as(
        "SELECT ondemand_locales, ondemand_budget_usd, ondemand_per_day FROM projects WHERE id = $1 FOR UPDATE",
    )
    .bind(p.id)
    .fetch_one(&mut *tx)
    .await?;
    if !allowed.iter().any(|l| l == "*" || l == locale) {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "ondemand_not_allowed",
            format!("on-demand translation is not enabled for '{locale}'"),
        ));
    }
    if status(&mut *tx, p.id, locale).await?.1 {
        return Ok(Some(accepted(p.public_cdn)));
    }
    let (today, spent): (i64, f64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE created_at > now() - interval '1 day'),
                coalesce(sum(cost_usd) FILTER (WHERE created_at >= date_trunc('month', now())), 0)
         FROM jobs WHERE project_id = $1 AND params ? 'ondemand_locale'",
    )
    .bind(p.id)
    .fetch_one(&mut *tx)
    .await?;
    if today >= per_day as i64 {
        return Ok(Some(
            (
                [(header::RETRY_AFTER, "3600")],
                AppError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "daily on-demand translation cap reached for this project",
                ),
            )
                .into_response(),
        ));
    }
    // ponytail: checked before the job, not during it — one job can overshoot by its own cost
    // (bounded by llm.budget_usd); pass the remainder into the job if that ever matters.
    if budget.is_some_and(|b| spent >= b) {
        return Err(AppError::new(
            StatusCode::PAYMENT_REQUIRED,
            "ondemand_budget_exhausted",
            "monthly on-demand translation budget spent",
        ));
    }
    sqlx::query("INSERT INTO jobs (id, project_id, kind, status, params) VALUES ($1, $2, 'translate', 'queued', $3)")
        .bind(Uuid::new_v4())
        .bind(p.id)
        .bind(serde_json::json!({ "locales": [locale], "ondemand_locale": locale }))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    state.job_wakeup.notify_one();
    tracing::info!(project = %p.slug, locale, "on-demand locale queued");
    Ok(Some(accepted(p.public_cdn)))
}

fn accepted(public: bool) -> Response {
    let mut res = (
        StatusCode::ACCEPTED,
        [
            (header::RETRY_AFTER, "5"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(serde_json::json!({ "status": "translating" })),
    )
        .into_response();
    if public {
        res.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
    }
    res
}
