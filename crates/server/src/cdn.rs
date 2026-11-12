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
    // Not exactly a project locale: an on-demand catalog under that very code wins (exact match), else the
    // code's alias among the project's locales (`zh`, `zh_CN`, `zh-Hans` → `zh-CN`; crate::locale_alias).
    // A code with an alias never starts an on-demand translation.
    let alias;
    let mut locale = locale;
    if locale != p.source_locale && !p.target_locales.iter().any(|l| l == locale) {
        alias = crate::locale_alias::resolve_project(locale, &p.source_locale, &p.target_locales);
        let trigger = alias.is_none() && q.get("ondemand").is_some_and(|v| v == "1");
        match crate::ondemand::gate(&state, &p, locale, trigger).await {
            Ok(Some(res)) => return Ok(res),
            Ok(None) => {}
            Err(e) => locale = alias.as_deref().ok_or(e)?,
        }
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
pub(crate) async fn readable(
    state: &AppState,
    slug: &str,
    headers: &HeaderMap,
) -> ApiResult<ProjectRow> {
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

/// `GET /cdn/{slug}/locales.json[?min=0.95][&all=1]`: what a language switcher needs — the source and the target
/// locales whose catalog coverage is ≥ `min`, with endonym, direction, coverage and version, and `ready` (SEO
/// readiness, used for hreflang and sitemaps only). Ready = global coverage (catalog keys
/// and segments rendered by glot Edge pages, non-stale) ≥ `min` (default 0) **and** every critical page (home,
/// pricing, signup, checkout… `edge_settings.critical_patterns`) ≥ its threshold (98 %): `critical_coverage` is the
/// worst critical page, `null` while no page has been seen (key-only projects). `all=1` also lists the locales
/// that are not ready (`ready: false`), for dashboards. Adding a target in glot makes it appear here once ready
/// (≤ 60 s cache), no redeploy of the site.
pub async fn locales(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let min: f64 = q
        .get("min")
        .and_then(|v| v.parse().ok())
        .filter(|m: &f64| m.is_finite())
        .unwrap_or(0.0);
    let all = q.get("all").is_some_and(|v| v == "1");
    let p = readable(&state, &slug, &headers).await?;
    let ready = crate::edge::seo_report::readiness(&state, &p).await?;
    let updated_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT max(t.updated_at) FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND t.locale = ANY($2)",
    )
    .bind(p.id)
    .bind(&p.target_locales)
    .fetch_one(&state.db)
    .await?;
    let edge_updated: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT max(updated_at) FROM edge_catalog_version WHERE project_id = $1 AND locale = ANY($2)",
    )
    .bind(p.id)
    .bind(&p.target_locales)
    .fetch_one(&state.db)
    .await?;
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    let entry = |code: &str, coverage: f64, critical: Option<f64>, ready: bool, version: i64| {
        json!({"code": code, "name": endonym(code), "dir": if is_rtl(code) { "rtl" } else { "ltr" },
               "coverage": round(coverage), "critical_coverage": critical, "ready": ready, "version": version})
    };
    let source_version = crate::edge::tm::version(&state.db, p.id, &p.source_locale).await?;
    let mut list = vec![entry(
        &p.source_locale,
        1.0,
        Some(1.0),
        true,
        source_version,
    )];
    // Listed (language switcher, runtime catalogs) on catalog coverage only, as before glot Edge: segments Edge
    // discovers on rendered pages never hide a language whose catalog is translated. `ready` stays the SEO gate
    // (hreflang/sitemaps): global coverage incl. Edge segments + critical pages.
    for r in &ready {
        // Edge-only projects (no catalog keys) are listed on their Edge coverage.
        let keys = if r.keys == 0 {
            r.coverage()
        } else {
            r.keys_done as f64 / r.keys as f64
        };
        if keys + 1e-9 >= min || all {
            list.push(entry(
                &r.locale,
                keys,
                r.critical_coverage,
                r.ready(min),
                r.version,
            ));
        }
    }
    // `site_code`: the code the site itself uses for the locale, when it differs (Settings → Languages;
    // crate::live_check). The switcher writes it in the URL / cookie / event instead of `code`.
    let site_codes = crate::live_check::site_codes(&state.db, p.id).await?;
    for l in &mut list {
        let site_code = l["code"]
            .as_str()
            .and_then(|c| site_codes.get(c).filter(|s| *s != c));
        if let Some(s) = site_code {
            l["site_code"] = json!(s);
        }
    }
    let updated_at = updated_at
        .into_iter()
        .chain(edge_updated)
        .max()
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let body =
        json!({"source": p.source_locale, "locales": list, "updated_at": updated_at}).to_string();
    let etag = format!("\"loc-{}\"", glot_core::hash::hash(&body));
    Ok(respond(&headers, body, etag, p.public_cdn))
}

fn is_rtl(code: &str) -> bool {
    let base = code.split(['-', '_']).next().unwrap_or(code);
    matches!(
        base,
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ckb"
    )
}

/// Endonym of the base language; the code itself when unknown (the switcher falls back to `Intl.DisplayNames`).
// ponytail: fixed table of common languages, regional variants share the base name.
fn endonym(code: &str) -> String {
    let base = code
        .split(['-', '_'])
        .next()
        .unwrap_or(code)
        .to_ascii_lowercase();
    let name = match base.as_str() {
        "en" => "English",
        "fr" => "Français",
        "es" => "Español",
        "de" => "Deutsch",
        "it" => "Italiano",
        "pt" => "Português",
        "nl" => "Nederlands",
        "ja" => "日本語",
        "ko" => "한국어",
        "zh" => "中文",
        "ru" => "Русский",
        "ar" => "العربية",
        "hi" => "हिन्दी",
        "th" => "ไทย",
        "vi" => "Tiếng Việt",
        "tr" => "Türkçe",
        "pl" => "Polski",
        "sv" => "Svenska",
        "da" => "Dansk",
        "nb" | "no" => "Norsk",
        "fi" => "Suomi",
        "cs" => "Čeština",
        "el" => "Ελληνικά",
        "he" | "iw" => "עברית",
        "id" => "Bahasa Indonesia",
        "ms" => "Bahasa Melayu",
        "uk" => "Українська",
        "ro" => "Română",
        "hu" => "Magyar",
        "bg" => "Български",
        "hr" => "Hrvatski",
        "sk" => "Slovenčina",
        "sl" => "Slovenščina",
        "sr" => "Српски",
        "ca" => "Català",
        "fa" => "فارسی",
        "ur" => "اردو",
        "bn" => "বাংলা",
        "ta" => "தமிழ்",
        "tl" | "fil" => "Filipino",
        "lt" => "Lietuvių",
        "lv" => "Latviešu",
        "et" => "Eesti",
        "sw" => "Kiswahili",
        _ => return code.to_string(),
    };
    name.to_string()
}
