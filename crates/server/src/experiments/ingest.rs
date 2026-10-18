//! Exposure / conversion ingestion for segmented experiments (contract §6).
//! Two doors, one counter path (`record`):
//! - `POST /api/insights/{pk}` — a regular `Hit` carrying `exposures` (client runtime, also counts the `$view`);
//! - `POST /api/insights/{pk}/batch` — a JSON array of such hits (edge `waitUntil`), experiments only.
//!
//! Privacy: only `xp_counts` (experiment, cell, arm, day, event) aggregates. No IP, UA, route or visitor id is
//! stored; `dedup` is a random per-beacon id kept one hour (UNLOGGED) so retries are not counted twice.
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use super::model::{ANY, Exposure};
use crate::{ApiResult, AppError, AppPath, AppState};

pub const EXPOSURE: &str = "$exposure";
/// Block actually seen (client, ≥ 50 % visible ≥ 1 s); `value_sum` = Σ bucketed visible ms.
pub const SEEN: &str = "$seen";
/// Cap of a `visible_ms` bucket.
pub const MAX_VISIBLE_MS: f64 = 60_000.0;
/// Hits per batch (each counts as one hit for the rate limit and the `insights_events` quota).
pub const MAX_BATCH: usize = 100;
/// Exposures per hit (contract §6).
pub const MAX_EXPOSURES: usize = 20;

/// One beacon: `Hit` without the language fields (contract §6).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct XpHit {
    /// Locale the document was rendered in; an experiment that does not cover it is not counted.
    #[serde(default)]
    pub served: String,
    /// Absent = exposure; else a conversion event.
    pub event: Option<String>,
    #[serde(default)]
    pub exposures: Vec<Exposure>,
    /// Revenue of a conversion, in `currency` (converted to the experiment's currency).
    pub value: Option<f64>,
    pub currency: Option<String>,
    /// Random id of this beacon (8-64 chars `[A-Za-z0-9_-]`), reused on retry: counted once.
    pub dedup: Option<String>,
}

/// Crawlers and link unfurlers: served like anyone (no cloaking), never counted.
pub fn is_bot(headers: &HeaderMap) -> bool {
    const BOTS: &[&str] = &[
        "bot",
        "crawl",
        "spider",
        "slurp",
        "headlesschrome",
        "lighthouse",
        "pagespeed",
        "prerender",
        "phantomjs",
        "facebookexternalhit",
        "mediapartners",
        "adsbot",
        "bingpreview",
        "embedly",
        "whatsapp",
        "skypeuripreview",
        "quora link preview",
        "google-inspectiontool",
        "apis-google",
        "feedfetcher",
        "pingdom",
        "uptimerobot",
        "gtmetrix",
        "chrome-lighthouse",
    ];
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    BOTS.iter().any(|b| ua.contains(b))
}

/// `value` in `from` expressed in `to` (ISO 4217); `None` for an unknown currency.
// ponytail: static reference rates (units per EUR, Oct 2026). Every arm of an experiment converts with the same
// table, so drift biases the absolute revenue, not the comparison; swap for a daily ECB feed if absolute
// revenue is ever reported.
pub fn convert(value: f64, from: &str, to: &str) -> Option<f64> {
    const PER_EUR: &[(&str, f64)] = &[
        ("EUR", 1.0),
        ("USD", 1.08),
        ("GBP", 0.85),
        ("CHF", 0.95),
        ("JPY", 160.0),
        ("CAD", 1.48),
        ("AUD", 1.65),
        ("NZD", 1.80),
        ("SEK", 11.3),
        ("NOK", 11.6),
        ("DKK", 7.46),
        ("PLN", 4.3),
        ("CZK", 25.0),
        ("HUF", 395.0),
        ("RON", 4.97),
        ("BRL", 6.0),
        ("MXN", 20.0),
        ("INR", 90.0),
        ("CNY", 7.8),
        ("HKD", 8.4),
        ("SGD", 1.45),
        ("KRW", 1480.0),
        ("TRY", 37.0),
        ("ZAR", 19.8),
    ];
    let rate = |c: &str| {
        PER_EUR
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(c))
            .map(|r| r.1)
    };
    Some(value / rate(from)? * rate(to)?)
}

fn valid_dedup(d: &str) -> bool {
    (8..=64).contains(&d.len())
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Count `hits` for `project_id` (already rate-limited and billed). Returns (counted beacons, duplicates).
/// Three statements whatever the batch size: dedup, experiment lookup, one aggregated upsert.
pub async fn record(
    state: &AppState,
    project_id: Uuid,
    hits: Vec<XpHit>,
) -> ApiResult<(usize, usize)> {
    let mut hits: Vec<XpHit> = hits
        .into_iter()
        .filter(|h| !h.exposures.is_empty())
        .collect();
    if hits.is_empty() {
        return Ok((0, 0));
    }
    for h in &hits {
        if h.dedup.as_deref().is_some_and(|d| !valid_dedup(d)) {
            return Err(AppError::bad_request(
                "dedup must be 8-64 chars of A-Za-z0-9_-",
            ));
        }
        if h.exposures.len() > MAX_EXPOSURES || h.exposures.iter().any(|e| e.cell.len() > 64) {
            return Err(AppError::bad_request(
                "at most 20 exposures per hit, cell ≤ 64 chars",
            ));
        }
        if h.value.is_some_and(|v| !(0.0..=1e6).contains(&v)) {
            return Err(AppError::bad_request("value must be within [0, 1e6]"));
        }
    }

    // Dedup: first occurrence in the batch, then the shared table (other replicas, earlier retries).
    let total = hits.len();
    let mut seen = BTreeSet::new();
    hits.retain(|h| h.dedup.as_ref().is_none_or(|d| seen.insert(d.clone())));
    if !seen.is_empty() {
        let fresh: Vec<String> = sqlx::query_scalar(
            "INSERT INTO xp_dedup (project_id, nonce) SELECT $1, unnest($2::text[])
             ON CONFLICT DO NOTHING RETURNING nonce",
        )
        .bind(project_id)
        .bind(seen.iter().collect::<Vec<_>>())
        .fetch_all(&state.db)
        .await?;
        let fresh: BTreeSet<String> = fresh.into_iter().collect();
        hits.retain(|h| h.dedup.as_ref().is_none_or(|d| fresh.contains(d)));
        // ponytail: purge on ~1 % of batches (indexed); a cron is not worth it for a 1 h cache.
        if rand::random::<u8>() < 3 {
            sqlx::query("DELETE FROM xp_dedup WHERE seen_at < now() - interval '1 hour'")
                .execute(&state.db)
                .await?;
        }
    }
    let counted = hits.len();

    let ids: Vec<Uuid> = hits
        .iter()
        .flat_map(|h| h.exposures.iter().map(|e| e.id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    // (kind, goal, guardrails, currency, locales) of running experiments of this project.
    type Meta = (String, String, Vec<String>, Option<String>, Vec<String>);
    let meta: BTreeMap<Uuid, Meta> = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Vec<String>,
            Option<String>,
            Vec<String>,
        ),
    >(
        "SELECT id, kind, goal, guardrails, currency, locales FROM xp_experiments
         WHERE project_id = $1 AND id = ANY($2) AND status = 'running'",
    )
    .bind(project_id)
    .bind(&ids)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(id, k, g, gr, c, l)| (id, (k, g, gr, c, l)))
    .collect();

    // (experiment, cell, arm, event) -> (n, Σvalue, Σvalue²)
    let mut agg: BTreeMap<(Uuid, String, i32, String), (i64, f64, f64)> = BTreeMap::new();
    for h in &hits {
        let mut once = BTreeSet::new(); // one exposure per experiment per beacon
        for e in &h.exposures {
            let Some((kind, goal, guardrails, xp_currency, locales)) = meta.get(&e.id) else {
                continue;
            };
            if !once.insert(e.id)
                || (kind == "copy" && !locales.is_empty() && !locales.contains(&h.served))
            {
                continue;
            }
            let (event, v) = match h.event.as_deref() {
                None if e.seen => (SEEN, e.visible_ms.unwrap_or(0.0).clamp(0.0, MAX_VISIBLE_MS)),
                None => (EXPOSURE, 0.0),
                Some(ev) if ev == goal || guardrails.iter().any(|g| g == ev) => {
                    let v = match (h.value, h.currency.as_deref(), xp_currency.as_deref()) {
                        (Some(v), Some(from), Some(to)) => convert(v, from, to).unwrap_or(0.0),
                        _ => 0.0,
                    };
                    (ev, v)
                }
                Some(_) => continue,
            };
            let slot = agg
                .entry((e.id, e.cell.clone(), e.arm, event.to_string()))
                .or_default();
            *slot = (slot.0 + 1, slot.1 + v, slot.2 + v * v);
        }
    }
    if agg.is_empty() {
        return Ok((counted, total - counted));
    }
    let (mut id, mut cell, mut arm, mut event, mut n, mut s, mut q) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    for ((i, c, a, e), (cnt, sum, sq)) in agg {
        id.push(i);
        cell.push(c);
        arm.push(a);
        event.push(e);
        n.push(cnt);
        s.push(sum);
        q.push(sq);
    }
    // Only cells still counted (not personalized) and approved arms; the holdout has no arms/cells rows.
    sqlx::query(
        "INSERT INTO xp_counts (experiment_id, cell, arm, day, event, n, value_sum, value_sq)
         SELECT i.id, i.cell, i.arm, (now() AT TIME ZONE 'utc')::date, i.event, i.n, i.s, i.q
         FROM unnest($1::uuid[], $2::text[], $3::int4[], $4::text[], $5::int8[], $6::float8[], $7::float8[])
              AS i(id, cell, arm, event, n, s, q)
         JOIN xp_experiments e ON e.id = i.id
         WHERE (e.kind = 'holdout' AND i.cell = $8 AND i.arm IN (0, 1))
            OR (e.kind = 'copy'
                AND EXISTS (SELECT 1 FROM xp_cells c WHERE c.experiment_id = i.id AND c.cell = i.cell AND NOT c.personalized)
                AND EXISTS (SELECT 1 FROM xp_arms a WHERE a.experiment_id = i.id AND a.arm = i.arm AND a.approved))
         ON CONFLICT (experiment_id, cell, arm, day, event) DO UPDATE SET
           n = xp_counts.n + excluded.n,
           value_sum = xp_counts.value_sum + excluded.value_sum,
           value_sq = xp_counts.value_sq + excluded.value_sq",
    )
    .bind(&id)
    .bind(&cell)
    .bind(&arm)
    .bind(&event)
    .bind(&n)
    .bind(&s)
    .bind(&q)
    .bind(ANY)
    .execute(&state.db)
    .await?;
    Ok((counted, total - counted))
}

/// `POST /api/insights/{pk}/batch`: `[XpHit, …]` (≤ 100). Same public key, shared rate limit (one hit per element)
/// and `insights_events` quota as single hits. Bots get the same 200 and are not counted.
pub async fn batch(
    State(state): State<AppState>,
    AppPath(key): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let hits: Vec<XpHit> = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    if hits.len() > MAX_BATCH {
        return Err(AppError::bad_request("at most 100 hits per batch"));
    }
    for h in &hits {
        if let Some(e) = &h.event {
            crate::insights::check_event(e)?;
        }
    }
    let project_id = match crate::insights::admit(&state, &key, hits.len().max(1) as i32).await? {
        Ok(p) => p,
        Err(limited) => return Ok(limited),
    };
    let (accepted, duplicates) = if is_bot(&headers) || hits.is_empty() {
        (0, 0)
    } else {
        crate::billing::consume_n(
            &state,
            project_id,
            crate::billing::INSIGHTS,
            hits.len() as i64,
        )
        .await?;
        record(&state, project_id, hits).await?
    };
    Ok(Json(serde_json::json!({"accepted": accepted, "duplicates": duplicates})).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bots_and_currency() {
        let ua = |s: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::USER_AGENT, s.parse().unwrap());
            is_bot(&h)
        };
        assert!(ua(
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"
        ));
        assert!(ua("facebookexternalhit/1.1"));
        assert!(!ua(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 Safari/605.1.15"
        ));
        assert!(!is_bot(&HeaderMap::new()));
        assert_eq!(convert(10.0, "EUR", "EUR"), Some(10.0));
        assert!((convert(108.0, "usd", "EUR").unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(convert(1.0, "XYZ", "EUR"), None);
        assert!(valid_dedup("a1B2-c3_d") && !valid_dedup("short") && !valid_dedup("bad char!!"));
    }
}
