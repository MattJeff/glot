//! REST: `/api/projects/{slug}/xp…` (contract §4). Translator reads and edits draft texts, admin runs
//! experiments and segments, owner sets the holdout / persistence (privacy-relevant).
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, types::Json as DbJson};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use super::{load, model::*, readiness};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    keys,
    projects::{self, ProjectRow, Role},
    validate_key,
};

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/xp";
    Router::new()
        .route(p, get(list).post(create))
        .route(&format!("{p}/estimate"), post(super::rebalance::estimate))
        .route(
            &format!("{p}/{{id}}/results"),
            get(super::rebalance::results),
        )
        .route(
            &format!("{p}/segments"),
            get(list_segments).post(create_segment),
        )
        .route(
            &format!("{p}/segments/{{id}}"),
            axum::routing::patch(update_segment).delete(delete_segment),
        )
        .route(
            &format!("{p}/settings"),
            get(get_settings).put(put_settings),
        )
        .route(
            &format!("{p}/{{id}}"),
            get(get_one).patch(update).delete(delete),
        )
        .route(
            &format!("{p}/{{id}}/arms/{{arm}}"),
            put(put_arm).delete(delete_arm),
        )
        .route(&format!("{p}/{{id}}/check"), get(check))
        .route(&format!("{p}/{{id}}/start"), post(start))
        .route(&format!("{p}/{{id}}/resume"), post(start))
        .route(&format!("{p}/{{id}}/pause"), post(pause))
        .route(&format!("{p}/{{id}}/stop"), post(stop))
        .route(&format!("{p}/{{id}}/promote"), post(promote))
}

const LIVE: &str = "('running', 'paused', 'personalized')";

// ponytail: same rule as insights::valid_event (private there).
fn valid_event(e: &str) -> bool {
    (1..=40).contains(&e.len())
        && e.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}

fn bad(m: impl Into<String>) -> AppError {
    AppError::bad_request(m)
}

async fn status_of(conn: &mut PgConnection, project: Uuid, id: Uuid) -> ApiResult<Status> {
    let s: String = sqlx::query_scalar(
        "SELECT status FROM xp_experiments WHERE id = $1 AND project_id = $2 AND kind = 'copy' FOR UPDATE",
    )
    .bind(id)
    .bind(project)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found("experiment not found"))?;
    Status::parse(&s).ok_or_else(|| AppError::internal("bad status"))
}

// ---------- experiments ----------

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<Experiment>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT id FROM xp_experiments WHERE project_id = $1 AND kind = 'copy'
         ORDER BY (status IN {LIVE}) DESC, created_at DESC LIMIT 100"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let mut out = vec![];
    for id in ids {
        out.push(load(&state, p.id, id).await?);
    }
    Ok(Json(out))
}

pub async fn get_one(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Experiment>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(load(&state, p.id, id).await?))
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum CellInput {
    Id(Uuid),
    New { name: String, rule: SegmentRule },
}

#[derive(Deserialize)]
pub struct ArmInput {
    pub intent: String,
    #[serde(default)]
    pub values: BTreeMap<String, BTreeMap<String, String>>,
}

/// Body of POST (create) and PATCH (partial update, draft only except `name`).
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NewExperiment {
    pub name: Option<String>,
    pub keys: Option<Vec<String>>,
    pub prefix: Option<String>,
    pub locales: Option<Vec<String>>,
    pub cells: Option<Vec<CellInput>>,
    pub allocation: Option<Allocation>,
    pub weights: Option<Vec<f64>>,
    pub traffic_pct: Option<i32>,
    pub min_n: Option<i32>,
    pub goal: Option<String>,
    pub metric: Option<Metric>,
    /// `render` (default) or `seen`.
    pub exposure: Option<String>,
    pub currency: Option<String>,
    pub guardrails: Option<Vec<String>>,
    pub facts: Option<Vec<String>>,
    pub fits: Option<BTreeMap<String, glot_core::measure::Fit>>,
    pub review: Option<bool>,
    pub arms: Option<Vec<ArmInput>>,
}

fn valid_name(n: &str) -> ApiResult<String> {
    let n = n.trim();
    if (1..=100).contains(&n.chars().count()) {
        Ok(n.to_string())
    } else {
        Err(bad("name must be 1-100 characters"))
    }
}

async fn insert_segment(
    conn: &mut PgConnection,
    project: Uuid,
    name: &str,
    rule: &SegmentRule,
) -> ApiResult<Segment> {
    let name = valid_name(name)?;
    rule.validate().map_err(bad)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO xp_segments (id, project_id, name, rule) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(project)
        .bind(&name)
        .bind(DbJson(rule))
        .execute(&mut *conn)
        .await
        .map_err(|e| match AppError::from(e) {
            e if e.code == "conflict" => {
                AppError::conflict(format!("a segment named '{name}' already exists"))
            }
            e => e,
        })?;
    Ok(Segment {
        id,
        name,
        rule: rule.clone(),
    })
}

/// Writes the fields present in `b`. `draft` = the experiment may still change shape.
async fn apply(
    conn: &mut PgConnection,
    p: &ProjectRow,
    id: Uuid,
    b: NewExperiment,
    draft: bool,
) -> ApiResult<()> {
    if let Some(n) = &b.name {
        sqlx::query("UPDATE xp_experiments SET name = $2 WHERE id = $1")
            .bind(id)
            .bind(valid_name(n)?)
            .execute(&mut *conn)
            .await?;
    }
    let shape_change = b.keys.is_some()
        || b.prefix.is_some()
        || b.locales.is_some()
        || b.cells.is_some()
        || b.allocation.is_some()
        || b.weights.is_some()
        || b.traffic_pct.is_some()
        || b.min_n.is_some()
        || b.goal.is_some()
        || b.metric.is_some()
        || b.exposure.is_some()
        || b.currency.is_some()
        || b.guardrails.is_some()
        || b.facts.is_some()
        || b.fits.is_some()
        || b.review.is_some()
        || b.arms.is_some();
    if !shape_change {
        return Ok(());
    }
    if !draft {
        return Err(AppError::conflict(
            "only the name can change once an experiment has started",
        ));
    }

    // Key group: explicit keys or every key under `prefix.`.
    if b.keys.is_some() || b.prefix.is_some() {
        let mut keys: BTreeSet<String> = b.keys.clone().unwrap_or_default().into_iter().collect();
        if let Some(prefix) = b.prefix.as_deref().map(|s| s.trim_end_matches('.')) {
            validate_key(prefix)?;
            let found: Vec<String> = sqlx::query_scalar(
                "SELECT key FROM keys WHERE project_id = $1 AND left(key, length($2) + 1) = $2 || '.' ORDER BY key LIMIT 21",
            )
            .bind(p.id)
            .bind(prefix)
            .fetch_all(&mut *conn)
            .await?;
            if found.is_empty() {
                return Err(bad(format!("no key under '{prefix}.'")));
            }
            keys.extend(found);
        }
        if !(1..=MAX_KEYS).contains(&keys.len()) {
            return Err(bad(format!("an experiment covers 1-{MAX_KEYS} keys")));
        }
        for k in &keys {
            validate_key(k)?;
            keys::find_key(&mut *conn, p.id, k).await?;
        }
        let keys: Vec<String> = keys.into_iter().collect();
        sqlx::query("DELETE FROM xp_keys WHERE experiment_id = $1 AND NOT key = ANY($2)")
            .bind(id)
            .bind(&keys)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO xp_keys (experiment_id, project_id, key) SELECT $1, $2, unnest($3::text[])
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(p.id)
        .bind(&keys)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM xp_values WHERE experiment_id = $1 AND NOT key = ANY($2)")
            .bind(id)
            .bind(&keys)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(locales) = &b.locales {
        let mut seen = BTreeSet::new();
        for l in locales {
            if *l != p.source_locale && !p.target_locales.contains(l) {
                return Err(bad(format!("'{l}' is not a locale of this project")));
            }
            seen.insert(l.clone());
        }
        if seen.is_empty() {
            return Err(bad("at least one locale"));
        }
        let locales: Vec<String> = seen.into_iter().collect();
        sqlx::query("UPDATE xp_experiments SET locales = $2 WHERE id = $1")
            .bind(id)
            .bind(&locales)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM xp_values WHERE experiment_id = $1 AND NOT locale = ANY($2)")
            .bind(id)
            .bind(&locales)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(cells) = b.cells {
        if cells.len() > MAX_CELLS {
            return Err(bad(format!("at most {MAX_CELLS} segments per experiment")));
        }
        let mut ids: Vec<Uuid> = vec![];
        for c in cells {
            let sid = match c {
                CellInput::Id(sid) => sqlx::query_scalar::<_, Uuid>(
                    "SELECT id FROM xp_segments WHERE id = $1 AND project_id = $2",
                )
                .bind(sid)
                .bind(p.id)
                .fetch_optional(&mut *conn)
                .await?
                .ok_or_else(|| bad(format!("unknown segment {sid}")))?,
                CellInput::New { name, rule } => insert_segment(conn, p.id, &name, &rule).await?.id,
            };
            if ids.contains(&sid) {
                return Err(bad("a segment appears twice"));
            }
            ids.push(sid);
        }
        sqlx::query("UPDATE xp_experiments SET cells = $2 WHERE id = $1")
            .bind(id)
            .bind(&ids)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(w) = &b.weights
        && (w.is_empty()
            || w.len() > MAX_ARMS as usize
            || w.iter().any(|x| !x.is_finite() || *x < 0.0)
            || w.iter().sum::<f64>() <= 0.0)
    {
        return Err(bad("weights: 1-8 non-negative numbers, not all zero"));
    }
    if b.traffic_pct.is_some_and(|t| !(1..=100).contains(&t))
        || b.min_n.is_some_and(|n| !(50..=1_000_000).contains(&n))
    {
        return Err(bad("traffic_pct must be 1-100, min_n 50-1000000"));
    }
    if b.exposure
        .as_deref()
        .is_some_and(|e| e != "render" && e != "seen")
    {
        return Err(bad("exposure must be \"render\" or \"seen\""));
    }
    if b.goal.as_deref().is_some_and(|g| !valid_event(g)) {
        return Err(bad("invalid goal event name"));
    }
    if let Some(g) = &b.guardrails
        && (g.len() > 5 || g.iter().any(|e| !valid_event(e)))
    {
        return Err(bad("guardrails: at most 5 valid event names"));
    }
    if b.currency
        .as_deref()
        .is_some_and(|c| c.len() != 3 || !c.bytes().all(|b| b.is_ascii_uppercase()))
    {
        return Err(bad("currency must be an ISO 4217 code (EUR, USD…)"));
    }
    if let Some(f) = &b.facts
        && (f.len() > 20
            || f.iter()
                .any(|x| x.trim().is_empty() || x.chars().count() > 200))
    {
        return Err(bad(
            "facts: at most 20 non-empty strings of ≤ 200 characters",
        ));
    }
    sqlx::query(
        "UPDATE xp_experiments SET
           allocation = coalesce($2, allocation), weights = CASE WHEN $3 THEN $4 ELSE weights END,
           traffic_pct = coalesce($5, traffic_pct), min_n = coalesce($6, min_n), goal = coalesce($7, goal),
           metric = coalesce($8, metric), currency = coalesce($9, currency), guardrails = coalesce($10, guardrails),
           facts = coalesce($11, facts), fits = coalesce($12, fits), review = coalesce($13, review),
           exposure = coalesce($14, exposure)
         WHERE id = $1",
    )
    .bind(id)
    .bind(b.allocation.map(Allocation::as_str))
    .bind(b.weights.is_some())
    .bind(&b.weights)
    .bind(b.traffic_pct)
    .bind(b.min_n)
    .bind(&b.goal)
    .bind(b.metric.map(Metric::as_str))
    .bind(&b.currency)
    .bind(&b.guardrails)
    .bind(b.facts.as_ref().map(DbJson))
    .bind(b.fits.as_ref().map(DbJson))
    .bind(b.review)
    .bind(&b.exposure)
    .execute(&mut *conn)
    .await?;
    if let Some(arms) = b.arms {
        if arms.len() >= MAX_ARMS as usize {
            return Err(bad(format!("at most {} variants", MAX_ARMS - 1)));
        }
        let review: bool = sqlx::query_scalar("SELECT review FROM xp_experiments WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM xp_arms WHERE experiment_id = $1 AND arm > 0")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        for (i, a) in arms.into_iter().enumerate() {
            let arm = i as i32 + 1;
            upsert_arm(conn, id, arm, Some(&a.intent), Some(!review), &a.values).await?;
        }
    }
    Ok(())
}

/// Insert or update arm `arm` (≥ 1) and merge its values (`""` deletes a cell).
async fn upsert_arm(
    conn: &mut PgConnection,
    id: Uuid,
    arm: i32,
    intent: Option<&str>,
    approved: Option<bool>,
    values: &BTreeMap<String, BTreeMap<String, String>>,
) -> ApiResult<()> {
    if !(1..MAX_ARMS).contains(&arm) {
        return Err(bad(format!("arm must be 1-{}", MAX_ARMS - 1)));
    }
    let intent = intent.map(valid_name).transpose()?;
    let n = sqlx::query(
        "INSERT INTO xp_arms (experiment_id, arm, intent, approved) SELECT $1, $2, $3, coalesce($4, true)
         WHERE $3 IS NOT NULL
         ON CONFLICT (experiment_id, arm) DO UPDATE SET intent = coalesce($3, xp_arms.intent),
           approved = coalesce($4, xp_arms.approved)",
    )
    .bind(id)
    .bind(arm)
    .bind(&intent)
    .bind(approved)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if n == 0 {
        // No intent given: the arm must exist already.
        let r = sqlx::query("UPDATE xp_arms SET approved = coalesce($3, approved) WHERE experiment_id = $1 AND arm = $2")
            .bind(id)
            .bind(arm)
            .bind(approved)
            .execute(&mut *conn)
            .await?;
        if r.rows_affected() == 0 {
            return Err(bad("a new variant needs an intent"));
        }
    }
    let (locales, keys): (Vec<String>, Vec<String>) = sqlx::query_as(
        "SELECT e.locales, coalesce(array_agg(x.key) FILTER (WHERE x.key IS NOT NULL), '{}')
         FROM xp_experiments e LEFT JOIN xp_keys x ON x.experiment_id = e.id WHERE e.id = $1 GROUP BY e.locales",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    for (locale, m) in values {
        if !locales.contains(locale) {
            return Err(bad(format!(
                "'{locale}' is not a locale of this experiment"
            )));
        }
        for (key, value) in m {
            if !keys.contains(key) {
                return Err(bad(format!("'{key}' is not a key of this experiment")));
            }
            if value.chars().count() > 2000 {
                return Err(bad("a variant text is at most 2000 characters"));
            }
            if value.is_empty() {
                sqlx::query("DELETE FROM xp_values WHERE experiment_id = $1 AND arm = $2 AND key = $3 AND locale = $4")
                    .bind(id)
                    .bind(arm)
                    .bind(key)
                    .bind(locale)
                    .execute(&mut *conn)
                    .await?;
            } else {
                sqlx::query(
                    "INSERT INTO xp_values (experiment_id, arm, key, locale, value) VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (experiment_id, arm, key, locale) DO UPDATE SET value = excluded.value",
                )
                .bind(id)
                .bind(arm)
                .bind(key)
                .bind(locale)
                .bind(value)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    Ok(())
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(mut b): AppJson<NewExperiment>,
) -> ApiResult<(StatusCode, Json<Experiment>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let name = valid_name(b.name.as_deref().unwrap_or(""))?;
    if b.keys.is_none() && b.prefix.is_none() {
        return Err(bad("keys or prefix is required"));
    }
    if b.locales.is_none() {
        b.locales = Some(
            std::iter::once(p.source_locale.clone())
                .chain(p.target_locales.clone())
                .collect(),
        );
    }
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO xp_experiments (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(p.id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO xp_arms (experiment_id, arm, intent) VALUES ($1, 0, 'official')")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // `review` first so arms inserted below are drafts when it is set.
    let review = b.review.take();
    if review.is_some() {
        apply(
            &mut tx,
            &p,
            id,
            NewExperiment {
                review,
                ..Default::default()
            },
            true,
        )
        .await?;
    }
    apply(&mut tx, &p, id, b, true).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load(&state, p.id, id).await?)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<NewExperiment>,
) -> ApiResult<Json<Experiment>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    let s = status_of(&mut tx, p.id, id).await?;
    apply(&mut tx, &p, id, b, s == Status::Draft).await?;
    tx.commit().await?;
    Ok(Json(load(&state, p.id, id).await?))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    if !matches!(
        status_of(&mut tx, p.id, id).await?,
        Status::Draft | Status::Stopped
    ) {
        return Err(AppError::conflict("stop the experiment before deleting it"));
    }
    sqlx::query("DELETE FROM xp_experiments WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmPatch {
    pub intent: Option<String>,
    pub approved: Option<bool>,
    #[serde(default)]
    pub values: BTreeMap<String, BTreeMap<String, String>>,
}

pub async fn put_arm(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id, arm)): AppPath<(String, Uuid, i32)>,
    AppJson(b): AppJson<ArmPatch>,
) -> ApiResult<Json<Variant>> {
    // Translators edit texts; approving a variant (it will be served) is an admin decision.
    let min = if b.approved.is_some() {
        Role::Admin
    } else {
        Role::Translator
    };
    let p = projects::load(&state, user.id, &slug, min).await?;
    let mut tx = state.db.begin().await?;
    if status_of(&mut tx, p.id, id).await? != Status::Draft {
        return Err(AppError::conflict(
            "variants can only change while the experiment is a draft",
        ));
    }
    upsert_arm(&mut tx, id, arm, b.intent.as_deref(), b.approved, &b.values).await?;
    tx.commit().await?;
    load(&state, p.id, id)
        .await?
        .arms
        .into_iter()
        .find(|a| a.arm == arm)
        .map(Json)
        .ok_or_else(|| AppError::internal("arm vanished"))
}

pub async fn delete_arm(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id, arm)): AppPath<(String, Uuid, i32)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    if status_of(&mut tx, p.id, id).await? != Status::Draft {
        return Err(AppError::conflict(
            "variants can only change while the experiment is a draft",
        ));
    }
    let n = sqlx::query("DELETE FROM xp_arms WHERE experiment_id = $1 AND arm = $2 AND arm > 0")
        .bind(id)
        .bind(arm)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    if n == 0 {
        return Err(AppError::not_found("variant not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn check(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Readiness>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(readiness(&state, &p, id).await?))
}

/// Initial per-cell weights, indexed by arm number: `weights` (fixed) or uniform, unapproved arms at 0.
fn initial_weights(x: &Experiment) -> Vec<f64> {
    let max = x.arms.iter().map(|a| a.arm).max().unwrap_or(0) as usize;
    let mut w = vec![0.0; max + 1];
    for a in x.arms.iter().filter(|a| a.approved) {
        let i = a.arm as usize;
        w[i] = match (&x.weights, x.allocation) {
            (Some(fixed), Allocation::Fixed) => fixed.get(i).copied().unwrap_or(0.0),
            _ => 1.0,
        };
    }
    let sum: f64 = w.iter().sum();
    w.iter()
        .map(|v| if sum > 0.0 { v / sum } else { 0.0 })
        .collect()
}

/// draft → running (start) and paused → running (resume): re-validated each time.
pub async fn start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Response> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    let s = status_of(&mut tx, p.id, id).await?;
    if !matches!(s, Status::Draft | Status::Paused) {
        return Err(AppError::conflict(format!(
            "cannot start a {} experiment",
            s.as_str()
        )));
    }
    let r = readiness(&state, &p, id).await?;
    if !r.ready {
        let msg = format!("not ready: {}", r.problems.join("; "));
        let body =
            serde_json::json!({"error": {"code": "not_ready", "message": msg}, "readiness": r});
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
    }
    let x = load(&state, p.id, id).await?;
    let live: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM xp_experiments WHERE project_id = $1 AND kind = 'copy' AND status IN {LIVE} AND id <> $2"
    ))
    .bind(p.id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    crate::billing::check_experiment(&state, p.id, live + 1, x.cells.len() as i64).await?;
    sqlx::query(
        "UPDATE xp_keys x SET live = true, source = CASE WHEN $2 THEN k.source ELSE x.source END
         FROM keys k WHERE k.project_id = x.project_id AND k.key = x.key AND x.experiment_id = $1",
    )
    .bind(id)
    .bind(s == Status::Draft)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.code == "conflict" => {
            AppError::conflict("a key of this experiment is already live elsewhere")
        }
        e => e,
    })?;
    let w = initial_weights(&x);
    for cell in x
        .cells
        .iter()
        .map(|c| c.id.to_string())
        .chain([ANY.to_string()])
    {
        sqlx::query("INSERT INTO xp_cells (experiment_id, cell, weights) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(cell)
            .bind(&w)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE xp_experiments SET status = 'running', started_at = coalesce(started_at, now()) WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(load(&state, p.id, id).await?).into_response())
}

async fn transition(
    state: &AppState,
    user: Uuid,
    slug: &str,
    id: Uuid,
    from: &[Status],
    to: Status,
) -> ApiResult<Json<Experiment>> {
    let p = projects::load(state, user, slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    let s = status_of(&mut tx, p.id, id).await?;
    if !from.contains(&s) {
        return Err(AppError::conflict(format!(
            "cannot go from {} to {}",
            s.as_str(),
            to.as_str()
        )));
    }
    let end = !to.live();
    sqlx::query("UPDATE xp_experiments SET status = $2, ended_at = CASE WHEN $3 THEN now() END WHERE id = $1")
        .bind(id)
        .bind(to.as_str())
        .bind(end)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE xp_keys SET live = $2 WHERE experiment_id = $1")
        .bind(id)
        .bind(!end)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(load(state, p.id, id).await?))
}

pub async fn pause(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Experiment>> {
    transition(
        &state,
        user.id,
        &slug,
        id,
        &[Status::Running],
        Status::Paused,
    )
    .await
}

pub async fn stop(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Experiment>> {
    transition(
        &state,
        user.id,
        &slug,
        id,
        &[Status::Running, Status::Paused, Status::Personalized],
        Status::Stopped,
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    All,
    Segment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteBody {
    pub scope: Scope,
    pub arm: i32,
    pub cell: Option<String>,
}

/// `all`: the arm becomes the official translation of every (key, target locale) — same write path as an edit
/// (validated, history, then pull / CDN / PR). `segment`: the cell serves the arm at 100 %, uncounted.
pub async fn promote(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<PromoteBody>,
) -> ApiResult<Json<Experiment>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let x = load(&state, p.id, id).await?;
    let Some(arm) = x.arms.iter().find(|a| a.arm == b.arm && a.approved) else {
        return Err(bad("unknown or unapproved arm"));
    };
    let (everywhere, b_arm, cell) = (matches!(b.scope, Scope::All), b.arm, b.cell.clone());
    let mut tx = state.db.begin().await?;
    let s = status_of(&mut tx, p.id, id).await?;
    if !s.live() {
        return Err(AppError::conflict(format!(
            "cannot promote a {} experiment",
            s.as_str()
        )));
    }
    match b.scope {
        Scope::Segment => {
            let cell = b
                .cell
                .ok_or_else(|| bad("cell is required for scope 'segment'"))?;
            let n = sqlx::query(
                "UPDATE xp_cells SET winner = $3, personalized = true, decided_at = coalesce(decided_at, now()),
                   weights = array(SELECT CASE WHEN i - 1 = $3 THEN 1.0 ELSE 0.0 END::float8
                                    FROM generate_subscripts(weights, 1) i)
                 WHERE experiment_id = $1 AND cell = $2",
            )
            .bind(id)
            .bind(&cell)
            .bind(b.arm)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if n == 0 {
                return Err(bad(format!("unknown cell '{cell}'")));
            }
            sqlx::query("UPDATE xp_experiments SET status = 'personalized' WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        Scope::All => {
            if arm.arm > 0 {
                let glossary = crate::glossary::load(&state.db, p.id).await?;
                for k in &x.keys {
                    let rec = keys::find_key(&mut tx, p.id, &k.key).await?;
                    if rec.source != k.source {
                        return Err(AppError::conflict(format!(
                            "the source of '{}' changed since the start",
                            k.key
                        )));
                    }
                    for locale in x.locales.iter().filter(|l| p.target_locales.contains(l)) {
                        let Some(value) = arm.values.get(locale).and_then(|m| m.get(&k.key)) else {
                            continue;
                        };
                        keys::write_translation(
                            &mut tx,
                            &glossary,
                            keys::Write {
                                key: &rec,
                                locale,
                                value,
                                origin: "human",
                                status: "reviewed",
                                model: Some("experiment"),
                                score: None,
                                extra_issues: vec![],
                                author: Some(&user.name),
                            },
                        )
                        .await?;
                    }
                }
            }
            sqlx::query(
                "UPDATE xp_experiments SET status = 'promoted', ended_at = now() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE xp_keys SET live = false WHERE experiment_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    let x = load(&state, p.id, id).await?;
    // Git stays the source of truth: PR in the linked repo (XP9, promote.rs). Best effort: the promotion stands.
    if let Err(e) =
        super::promote::open_pr(&state, &p, &x, everywhere, b_arm, cell.as_deref()).await
    {
        tracing::warn!(experiment = %id, "promotion pull request failed: {e:#}");
    }
    Ok(Json(x))
}

// ---------- segments ----------

pub async fn list_segments(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<Segment>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<(Uuid, String, DbJson<SegmentRule>)> = sqlx::query_as(
        "SELECT id, name, rule FROM xp_segments WHERE project_id = $1 ORDER BY name",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, rule)| Segment {
                id,
                name,
                rule: rule.0,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentBody {
    pub name: Option<String>,
    pub rule: Option<SegmentRule>,
}

pub async fn create_segment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<SegmentBody>,
) -> ApiResult<(StatusCode, Json<Segment>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let rule = b.rule.ok_or_else(|| bad("rule is required"))?;
    let mut conn = state.db.acquire().await?;
    let s = insert_segment(&mut conn, p.id, b.name.as_deref().unwrap_or(""), &rule).await?;
    Ok((StatusCode::CREATED, Json(s)))
}

pub async fn update_segment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<SegmentBody>,
) -> ApiResult<Json<Segment>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let name = b.name.as_deref().map(valid_name).transpose()?;
    if let Some(r) = &b.rule {
        r.validate().map_err(bad)?;
        // A live experiment's cells must not change under it (counts would mix two populations).
        let used: bool = sqlx::query_scalar(&format!(
            "SELECT exists(SELECT 1 FROM xp_experiments WHERE project_id = $1 AND $2 = ANY(cells) AND status IN {LIVE})"
        ))
        .bind(p.id)
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if used {
            return Err(AppError::conflict(
                "this segment is used by a live experiment",
            ));
        }
    }
    let row: Option<(Uuid, String, DbJson<SegmentRule>)> = sqlx::query_as(
        "UPDATE xp_segments SET name = coalesce($3, name), rule = coalesce($4, rule)
         WHERE id = $1 AND project_id = $2 RETURNING id, name, rule",
    )
    .bind(id)
    .bind(p.id)
    .bind(&name)
    .bind(b.rule.as_ref().map(DbJson))
    .fetch_optional(&state.db)
    .await?;
    let (id, name, rule) = row.ok_or_else(|| AppError::not_found("segment not found"))?;
    Ok(Json(Segment {
        id,
        name,
        rule: rule.0,
    }))
}

pub async fn delete_segment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    let used: bool = sqlx::query_scalar(
        "SELECT exists(SELECT 1 FROM xp_experiments WHERE project_id = $1 AND $2 = ANY(cells) AND status <> 'draft')",
    )
    .bind(p.id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if used {
        return Err(AppError::conflict(
            "this segment is used by an experiment that has started",
        ));
    }
    sqlx::query("UPDATE xp_experiments SET cells = array_remove(cells, $2) WHERE project_id = $1 AND $2 = ANY(cells)")
        .bind(p.id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let n = sqlx::query("DELETE FROM xp_segments WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(p.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    if n == 0 {
        return Err(AppError::not_found("segment not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- project settings (holdout, persistence) ----------

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct Settings {
    pub holdout_pct: i32,
    pub persistence: String,
    pub holdout_id: Option<Uuid>,
}

async fn settings(state: &AppState, project: Uuid) -> ApiResult<Settings> {
    let (holdout_pct, persistence, holdout_id): (i32, String, Option<Uuid>) = sqlx::query_as(
        "SELECT p.xp_holdout_pct, p.xp_persistence, h.id FROM projects p
         LEFT JOIN xp_experiments h ON h.project_id = p.id AND h.kind = 'holdout' WHERE p.id = $1",
    )
    .bind(project)
    .fetch_one(&state.db)
    .await?;
    Ok(Settings {
        holdout_pct,
        persistence,
        holdout_id,
    })
}

pub async fn get_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(settings(&state, p.id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBody {
    pub holdout_pct: Option<i32>,
    pub persistence: Option<String>,
}

pub async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<SettingsBody>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Owner).await?;
    if b.holdout_pct.is_some_and(|h| !(0..=20).contains(&h)) {
        return Err(bad("holdout_pct must be 0-20"));
    }
    if b.persistence
        .as_deref()
        .is_some_and(|s| !["none", "session"].contains(&s))
    {
        return Err(bad("persistence must be 'none' or 'session'"));
    }
    let mut tx = state.db.begin().await?;
    let pct: i32 = sqlx::query_scalar(
        "UPDATE projects SET xp_holdout_pct = coalesce($2, xp_holdout_pct), xp_persistence = coalesce($3, xp_persistence)
         WHERE id = $1 RETURNING xp_holdout_pct",
    )
    .bind(p.id)
    .bind(b.holdout_pct)
    .bind(&b.persistence)
    .fetch_one(&mut *tx)
    .await?;
    // Holdout = pseudo-experiment: arm 0 = held out (no variant at all), arm 1 = everyone else.
    if pct > 0 || b.holdout_pct.is_some() {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO xp_experiments (id, project_id, kind, name, status, started_at)
             VALUES ($1, $2, 'holdout', 'holdout', 'running', now())
             ON CONFLICT (project_id) WHERE kind = 'holdout' DO UPDATE SET status = excluded.status
             RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO xp_arms (experiment_id, arm, intent) VALUES ($1, 0, 'holdout'), ($1, 1, 'exposed')
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        let h = pct as f64 / 100.0;
        sqlx::query(
            "INSERT INTO xp_cells (experiment_id, cell, weights) VALUES ($1, '*', $2)
             ON CONFLICT (experiment_id, cell) DO UPDATE SET weights = excluded.weights",
        )
        .bind(id)
        .bind(vec![h, 1.0 - h])
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE xp_experiments SET status = $2 WHERE id = $1")
            .bind(id)
            .bind(if pct > 0 { "running" } else { "paused" })
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(settings(&state, p.id).await?))
}
