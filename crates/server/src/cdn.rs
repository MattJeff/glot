use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use glot_core::catalog::{Catalog, Format};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppPath, AppQuery, AppState, auth::authenticate, projects::ProjectRow,
};

/// `GET /cdn/{slug}/{locale}.json`: nested JSON catalog for OTA updates.
/// Public projects (public_cdn) are readable by anyone; private ones only by members.
/// `?ondemand=1` on an unshipped locale: see `ondemand::gate` (202 while translating).
pub async fn serve(
    State(state): State<AppState>,
    AppPath((slug, file)): AppPath<(String, String)>,
    AppQuery(q): AppQuery<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let not_found = || AppError::not_found("catalog not found");
    let locale = file.strip_suffix(".json").ok_or_else(not_found)?;
    let p = readable(&state, &slug, &headers).await?;
    if locale != p.source_locale
        && !p.target_locales.iter().any(|l| l == locale)
        && let Some(res) = crate::ondemand::gate(
            &state,
            &p,
            locale,
            q.get("ondemand").is_some_and(|v| v == "1"),
        )
        .await?
    {
        return Ok(res);
    }

    let rows: Vec<(String, String)> = if locale == p.source_locale {
        sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as("SELECT k.key, t.value FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.project_id = $1 AND t.locale = $2")
            .bind(p.id)
            .bind(locale)
            .fetch_all(&state.db)
            .await?
    };
    let mut catalog = Catalog::new(locale);
    catalog.entries.extend(rows);
    let body = catalog.to_json(Format::JsonNested);
    let etag = format!("\"{}\"", glot_core::hash::hash(&body));
    Ok(respond(&headers, body, etag, p.public_cdn))
}

/// Public projects (public_cdn) are readable by anyone; private ones only by members. Counts one CDN request.
async fn readable(state: &AppState, slug: &str, headers: &HeaderMap) -> ApiResult<ProjectRow> {
    let not_found = || AppError::not_found("catalog not found");
    let p = crate::projects::load_public(state, slug)
        .await?
        .ok_or_else(not_found)?;
    if !p.public_cdn {
        let user = authenticate(headers, state).await?.ok_or_else(not_found)?;
        crate::projects::load(state, user.id, slug, crate::projects::Role::Translator).await?;
    }
    crate::billing::consume(state, p.id, crate::billing::CDN).await?;
    Ok(p)
}

/// JSON body with ETag / 304 and a short shared cache (stale-while-revalidate) when public.
fn respond(headers: &HeaderMap, body: String, etag: String, public: bool) -> Response {
    let cache = if public {
        "public, max-age=60, stale-while-revalidate=600"
    } else {
        "private, no-cache"
    };
    let mut res = if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response()
    };
    let h = res.headers_mut();
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("etag"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if public {
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
    }
    res
}

/// `GET /cdn/{slug}/experiments.json` (contract §5): what the edge/runtime needs to decide locally — running and
/// personalized experiments whose keys' source is unchanged, their segments, current per-cell weights, approved
/// arms and their validated texts. Nothing else (no names, intents, facts, counts). Never varies by user agent.
/// The ETag ignores `generated_at`: an unchanged config revalidates (304) and the edge restarts its `max_stale`
/// clock on any 200/304.
pub async fn experiments(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let p = readable(&state, &slug, &headers).await?;
    let (holdout_pct, persistence): (i32, String) =
        sqlx::query_as("SELECT xp_holdout_pct, xp_persistence FROM projects WHERE id = $1")
            .bind(p.id)
            .fetch_one(&state.db)
            .await?;
    let holdout: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM xp_experiments WHERE project_id = $1 AND kind = 'holdout' AND status = 'running'",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    // A key whose source changed (or vanished) stops the whole experiment: arms were validated against it.
    type Row = (
        Uuid,
        String,
        i32,
        String,
        Vec<String>,
        String,
        Option<String>,
        Vec<String>,
        Vec<Uuid>,
        Vec<String>,
    );
    let xps: Vec<Row> = sqlx::query_as(
        "SELECT e.id, e.status, e.traffic_pct, e.goal, e.guardrails, e.metric, e.currency, e.locales, e.cells,
                array(SELECT key FROM xp_keys WHERE experiment_id = e.id ORDER BY key)
         FROM xp_experiments e
         WHERE e.project_id = $1 AND e.kind = 'copy' AND e.status IN ('running', 'personalized')
           AND NOT EXISTS (SELECT 1 FROM xp_keys x LEFT JOIN keys k ON k.project_id = e.project_id AND k.key = x.key
                           WHERE x.experiment_id = e.id AND k.source IS DISTINCT FROM x.source)
         ORDER BY e.created_at, e.id",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let ids: Vec<Uuid> = xps.iter().map(|x| x.0).collect();
    let cells: Vec<(Uuid, String, Vec<f64>, bool)> = sqlx::query_as(
        "SELECT experiment_id, cell, weights, personalized FROM xp_cells WHERE experiment_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let arms: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT experiment_id, arm FROM xp_arms WHERE experiment_id = ANY($1) AND approved ORDER BY arm",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let values: Vec<(Uuid, i32, String, String, String)> = sqlx::query_as(
        "SELECT v.experiment_id, v.arm, v.locale, v.key, v.value FROM xp_values v
         JOIN xp_arms a ON a.experiment_id = v.experiment_id AND a.arm = v.arm
         WHERE v.experiment_id = ANY($1) AND a.approved",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let seg_ids: Vec<Uuid> = xps.iter().flat_map(|x| x.8.iter().copied()).collect();
    let segments: Vec<(Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT id, rule FROM xp_segments WHERE id = ANY($1) ORDER BY id")
            .bind(&seg_ids)
            .fetch_all(&state.db)
            .await?;

    let experiments: Vec<Value> = xps
        .into_iter()
        .map(
            |(
                id,
                status,
                traffic_pct,
                goal,
                guardrails,
                metric,
                currency,
                locales,
                order,
                keys,
            )| {
                let approved: Vec<i32> = arms.iter().filter(|a| a.0 == id).map(|a| a.1).collect();
                // Priority order of the experiment's segments, then '*'.
                let rank = |c: &str| {
                    order
                        .iter()
                        .position(|s| s.to_string() == c)
                        .unwrap_or(usize::MAX)
                };
                let mut mine: Vec<&(Uuid, String, Vec<f64>, bool)> =
                    cells.iter().filter(|c| c.0 == id).collect();
                mine.sort_by_key(|c| rank(&c.1));
                let cells: Vec<Value> = mine
                .into_iter()
                .map(|(_, cell, w, personalized)| {
                    // Stored weights are indexed by arm number; publish them indexed like `arms`.
                    let mut w: Vec<f64> = approved
                        .iter()
                        .map(|a| w.get(*a as usize).copied().unwrap_or(0.0).max(0.0))
                        .collect();
                    let sum: f64 = w.iter().sum();
                    if sum > 0.0 {
                        w.iter_mut().for_each(|x| *x /= sum);
                    }
                    json!({"id": cell, "weights": w, "count": status == "running" && !personalized})
                })
                .collect();
                let arms: Vec<Value> = approved
                    .iter()
                    .map(|a| {
                        let mut by_locale: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
                        for (_, _, l, k, v) in values.iter().filter(|v| v.0 == id && v.1 == *a) {
                            by_locale.entry(l).or_default().insert(k, v);
                        }
                        if *a == 0 {
                            json!({"arm": 0})
                        } else {
                            json!({"arm": a, "values": by_locale})
                        }
                    })
                    .collect();
                json!({"id": id, "status": status, "traffic_pct": traffic_pct, "goal": goal,
                   "guardrails": guardrails, "metric": metric, "currency": currency,
                   "keys": keys, "locales": locales, "cells": cells, "arms": arms})
            },
        )
        .collect();
    let mut config = json!({
        "v": 1,
        "project": p.slug,
        "max_stale": 86400,
        "persistence": persistence,
        "holdout": holdout.filter(|_| holdout_pct > 0).map(|id| json!({"id": id, "pct": holdout_pct})),
        "segments": segments.into_iter().map(|(id, rule)| json!({"id": id, "rule": rule})).collect::<Vec<_>>(),
        "experiments": experiments,
    });
    let etag = format!("\"xp-{}\"", glot_core::hash::hash(&config.to_string()));
    config["generated_at"] =
        json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    Ok(respond(&headers, config.to_string(), etag, p.public_cdn))
}
