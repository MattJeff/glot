//! `GET /api/projects/{slug}/markets`: which language to add next (Labs). Aggregates the existing
//! insights counters (no new data collection) with glot_core's public market table and the dry-run cost.
use axum::{Json, Router, extract::State, routing::get};
use glot_core::markets::{self, Demand, Opportunity};
use serde::{Deserialize, Serialize};

use crate::{
    ApiResult, AppPath, AppQuery, AppState,
    auth::AuthUser,
    insights,
    projects::{self, Role},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/projects/{slug}/markets", get(markets))
}

#[derive(Deserialize)]
pub struct MarketsQuery {
    days: Option<i64>,
}

#[derive(Serialize)]
pub struct Markets {
    pub days: i64,
    pub total_visits: i64,
    pub keys: usize,
    pub words: usize,
    pub model: String,
    /// Date of the embedded public market data.
    pub data_as_of: &'static str,
    pub markets: Vec<Opportunity>,
}

async fn markets(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<MarketsQuery>,
) -> ApiResult<Json<Markets>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days = q.days.unwrap_or(30).clamp(1, 365);
    // Reuse the insights aggregation as is (same auth, same counters).
    let query = serde_json::from_value(serde_json::json!({ "days": days }))
        .map_err(|e| crate::AppError::bad_request(e.to_string()))?;
    let Json(o) = insights::overview(
        State(state.clone()),
        AuthUser(user),
        AppPath(slug),
        AppQuery(query),
    )
    .await?;
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1 ORDER BY key")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let demand: Vec<Demand> = o
        .demand
        .iter()
        .map(|d| Demand {
            language: d.language.clone(),
            visits: d.visits,
            conversion_rate: d.conversion_rate,
            est_extra_conversions: d.est_extra_conversions,
        })
        .collect();
    let words = markets::words(keys.iter().map(|(_, s)| s));
    let ranked = markets::rank(&o.offered, o.total_visits, &demand, words, |l| {
        insights::estimate_cost(&p, &keys, l)
    });
    Ok(Json(Markets {
        days,
        total_visits: o.total_visits,
        keys: keys.len(),
        words,
        model: p.llm.0.model.clone(),
        data_as_of: markets::DATA_AS_OF,
        markets: ranked,
    }))
}
