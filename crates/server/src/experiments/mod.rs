//! Segmented copy experiments: a key group × locales × segment cells, arms = intentions, decided at the edge from
//! `GET /cdn/{slug}/experiments.json`. Contract: docs/design/segmented-copy-experiments-contract.md.
//! The legacy one-key/one-locale experiments (`variants.rs`) keep running unchanged.
pub mod api;
pub mod generate;
pub mod ingest;
pub mod model;
pub mod promote;
pub mod rebalance;
pub mod stats;

use glot_core::validate::{Syntax, has_errors};
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;
use uuid::Uuid;

/// XP1 REST + cross-language promotion (`promote.rs`, XP9).
pub fn routes() -> axum::Router<AppState> {
    api::routes().merge(promote::routes())
}
pub use model::*;

use crate::{ApiResult, AppError, AppState, keys, projects::ProjectRow};

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    name: String,
    kind: String,
    status: String,
    locales: Vec<String>,
    cells: Vec<Uuid>,
    allocation: String,
    weights: Option<Vec<f64>>,
    traffic_pct: i32,
    min_n: i32,
    goal: String,
    metric: String,
    exposure: String,
    currency: Option<String>,
    guardrails: Vec<String>,
    facts: DbJson<Vec<String>>,
    fits: serde_json::Value,
    review: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn load(state: &AppState, project_id: Uuid, id: Uuid) -> ApiResult<Experiment> {
    let r: Row = sqlx::query_as(
        "SELECT id, name, kind, status, locales, cells, allocation, weights, traffic_pct, min_n, goal, metric,
                exposure, currency, guardrails, facts, fits, review, created_at, started_at, ended_at
         FROM xp_experiments WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("experiment not found"))?;
    let status = Status::parse(&r.status).ok_or_else(|| AppError::internal("bad status"))?;
    let keys: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT x.key, x.source, k.source FROM xp_keys x
         LEFT JOIN keys k ON k.project_id = x.project_id AND k.key = x.key
         WHERE x.experiment_id = $1 ORDER BY x.key",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let keys = keys
        .into_iter()
        .map(|(key, frozen, now)| {
            let draft = status == Status::Draft;
            ExperimentKey {
                source_changed: !draft && now.as_deref() != Some(frozen.as_str()),
                source: if draft {
                    now.unwrap_or_default()
                } else {
                    frozen
                },
                key,
            }
        })
        .collect();
    let segs: Vec<(Uuid, String, DbJson<SegmentRule>)> =
        sqlx::query_as("SELECT id, name, rule FROM xp_segments WHERE id = ANY($1)")
            .bind(&r.cells)
            .fetch_all(&state.db)
            .await?;
    let cells = r
        .cells
        .iter()
        .filter_map(|c| segs.iter().find(|s| s.0 == *c))
        .map(|(id, name, rule)| Segment {
            id: *id,
            name: name.clone(),
            rule: rule.0.clone(),
        })
        .collect();
    let arms: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT arm, intent, approved FROM xp_arms WHERE experiment_id = $1 ORDER BY arm",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let values: Vec<(i32, String, String, String)> =
        sqlx::query_as("SELECT arm, locale, key, value FROM xp_values WHERE experiment_id = $1")
            .bind(id)
            .fetch_all(&state.db)
            .await?;
    let arms = arms
        .into_iter()
        .map(|(arm, intent, approved)| {
            let mut v: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
            for (_, locale, key, value) in values.iter().filter(|x| x.0 == arm) {
                v.entry(locale.clone())
                    .or_default()
                    .insert(key.clone(), value.clone());
            }
            Variant {
                arm,
                intent,
                approved,
                values: v,
            }
        })
        .collect();
    let mut state_rows: Vec<Allocations> = sqlx::query_as(
        "SELECT cell, weights, looks, winner, personalized FROM xp_cells WHERE experiment_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let rank = |c: &str| {
        r.cells
            .iter()
            .position(|s| s.to_string() == c)
            .unwrap_or(usize::MAX)
    };
    state_rows.sort_by_key(|a| rank(&a.cell));
    Ok(Experiment {
        id: r.id,
        name: r.name,
        kind: r.kind,
        status,
        keys,
        locales: r.locales,
        cells,
        allocation: Allocation::parse(&r.allocation).unwrap_or(Allocation::Bandit),
        weights: r.weights,
        traffic_pct: r.traffic_pct,
        min_n: r.min_n,
        goal: r.goal,
        metric: Metric::parse(&r.metric).unwrap_or(Metric::Conversion),
        exposure: r.exposure,
        currency: r.currency,
        guardrails: r.guardrails,
        facts: r.facts.0,
        fits: r.fits,
        review: r.review,
        arms,
        state: state_rows,
        created_at: r.created_at,
        started_at: r.started_at,
        ended_at: r.ended_at,
    })
}

/// Experiment id owning `key` (live in `xp_keys`), if any. Used by the legacy `variants::create`.
pub async fn live_key_owner(
    db: &sqlx::PgPool,
    project_id: Uuid,
    key: &str,
) -> ApiResult<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT experiment_id FROM xp_keys WHERE project_id = $1 AND key = $2 AND live",
    )
    .bind(project_id)
    .bind(key)
    .fetch_optional(db)
    .await?)
}

/// Can this experiment start? Every (approved arm ≥ 1, key, locale) cell must pass `keys::check` (placeholders,
/// ICU, glossary) and `measure::check` when the key has a layout budget. Unapproved arms are reported, not blocking.
pub async fn readiness(state: &AppState, p: &ProjectRow, id: Uuid) -> ApiResult<Readiness> {
    let x = load(state, p.id, id).await?;
    let mut problems = vec![];
    let sources: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT x.key, k.source FROM xp_keys x LEFT JOIN keys k ON k.project_id = x.project_id AND k.key = x.key
         WHERE x.experiment_id = $1 ORDER BY x.key",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    if sources.is_empty() {
        problems.push("no key selected".to_string());
    }
    for (k, s) in &sources {
        if s.is_none() {
            problems.push(format!("key '{k}' does not exist"));
        }
    }
    let taken: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, 'experiment ' || experiment_id FROM xp_keys
         WHERE project_id = $1 AND live AND experiment_id <> $2 AND key = ANY($3)
         UNION ALL
         SELECT key, 'legacy experiment ' || id || ' (' || locale || ')' FROM variant_experiments
         WHERE project_id = $1 AND status IN ('running', 'decided') AND key = ANY($3)",
    )
    .bind(p.id)
    .bind(id)
    .bind(sources.iter().map(|s| s.0.clone()).collect::<Vec<_>>())
    .fetch_all(&state.db)
    .await?;
    for (k, by) in taken {
        problems.push(format!("key '{k}' is already live in {by}"));
    }
    if x.locales.is_empty() {
        problems.push("no locale selected".into());
    }
    if x.cells.len() > MAX_CELLS {
        problems.push(format!("at most {MAX_CELLS} segments"));
    }
    if x.metric == Metric::Revenue && x.currency.is_none() {
        problems.push("a revenue goal needs a currency".into());
    }
    let max_arm = x.arms.iter().map(|a| a.arm).max().unwrap_or(0);
    if let Some(w) = &x.weights
        && (w.len() != max_arm as usize + 1 || x.allocation != Allocation::Fixed)
    {
        problems.push(format!(
            "weights need allocation 'fixed' and one entry per arm 0..={max_arm}"
        ));
    }
    if !x.arms.iter().any(|a| a.arm > 0 && a.approved) {
        problems.push("at least one approved variant is needed".into());
    }
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let mut cells = vec![];
    for a in x.arms.iter().filter(|a| a.arm > 0) {
        for (key, source) in &sources {
            let Some(source) = source else { continue };
            let fit = x
                .fits
                .get(key)
                .and_then(|f| serde_json::from_value::<glot_core::measure::Fit>(f.clone()).ok());
            for locale in &x.locales {
                let value = a.values.get(locale).and_then(|m| m.get(key)).cloned();
                let mut issues = vec![];
                if let Some(v) = value.as_deref().filter(|v| !v.trim().is_empty()) {
                    issues = keys::check(source, v, locale, &glossary);
                    issues.extend(
                        fit.as_ref()
                            .and_then(|f| glot_core::measure::check(v, Syntax::Icu, f)),
                    );
                }
                let ok =
                    value.as_deref().is_some_and(|v| !v.trim().is_empty()) && !has_errors(&issues);
                if !ok && a.approved {
                    problems.push(format!(
                        "arm {} ({}) — {key} [{locale}] is missing or invalid",
                        a.arm, a.intent
                    ));
                }
                cells.push(CellCheck {
                    arm: a.arm,
                    key: key.clone(),
                    locale: locale.clone(),
                    value,
                    ok,
                    issues,
                });
            }
        }
    }
    Ok(Readiness {
        ready: problems.is_empty(),
        problems,
        cells,
    })
}
