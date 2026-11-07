//! glot Edge observability: counters of the subdomain proxy, read from what every proxied response already says
//! about itself (`x-glot-edge` state, `Server-Timing: glot;dur=…, origin;dur=…`), so the proxy code needs no
//! instrumentation. Aggregated in memory per (host, UTC day), flushed every 30 s (and before each read) into
//! `edge_metrics_daily` (one row per project and day, latencies as fixed-bucket histograms), so the numbers
//! survive restarts. `GET /api/projects/{slug}/edge/metrics?days=7` answers the dashboard's `EdgeMetrics`
//! (web/components/edge/api.ts), plus the renderer's own counters when `GLOT_RENDERER_URL` is set.
// ponytail: percentiles from 21 fixed buckets (upper bound reported); exact quantiles if anyone needs them.
use axum::{
    Json, Router,
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
    routing::get,
};
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use crate::{
    ApiResult, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

/// Histogram bucket upper bounds (ms); the last bucket is everything above.
const BOUNDS: [f64; 20] = [
    1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 15.0, 20.0, 30.0, 50.0, 75.0, 100.0, 150.0, 200.0, 300.0, 500.0,
    750.0, 1000.0, 2000.0, 5000.0,
];
const BUCKETS: usize = BOUNDS.len() + 1;
const FLUSH_EVERY: Duration = Duration::from_secs(30);

#[derive(Default, Clone)]
struct Agg {
    requests: i64,
    hits: i64,
    /// Responses that went through translation (cache hit or not): the hit ratio's denominator.
    translated: i64,
    fail_open: i64,
    glot: [i64; BUCKETS],
    origin: [i64; BUCKETS],
}

fn bucket(ms: f64) -> usize {
    BOUNDS.iter().position(|b| ms <= *b).unwrap_or(BOUNDS.len())
}

/// Upper bound of the bucket holding the `p` quantile; None without samples.
fn pct(h: &[i64], p: f64) -> Option<f64> {
    let n: i64 = h.iter().sum();
    if n == 0 {
        return None;
    }
    let want = ((p * n as f64).ceil() as i64).max(1);
    let mut acc = 0;
    for (i, c) in h.iter().enumerate() {
        acc += c;
        if acc >= want {
            return Some(
                BOUNDS
                    .get(i)
                    .copied()
                    .unwrap_or(BOUNDS[BOUNDS.len() - 1] * 2.0),
            );
        }
    }
    None
}

struct Pending {
    by: HashMap<(String, NaiveDate), Agg>,
    flushed: Instant,
}

static PENDING: LazyLock<Mutex<Pending>> = LazyLock::new(|| {
    Mutex::new(Pending {
        by: HashMap::new(),
        flushed: Instant::now(),
    })
});

/// `name;dur=12.3` in a Server-Timing value.
fn dur(timing: &str, name: &str) -> Option<f64> {
    timing.split(',').find_map(|m| {
        let mut parts = m.trim().split(';');
        (parts.next()? == name).then_some(())?;
        parts.find_map(|p| p.trim().strip_prefix("dur=")?.parse().ok())
    })
}

/// Adds one response to `a`.
fn add(a: &mut Agg, state: &str, timing: &str) {
    a.requests += 1;
    if state.contains("cache=hit") {
        a.hits += 1;
    }
    if state.starts_with("translated") || state.starts_with("source") {
        a.translated += 1;
    }
    if state.starts_with("fail-open") {
        a.fail_open += 1;
    }
    if let Some(g) = dur(timing, "glot") {
        a.glot[bucket(g)] += 1;
    }
    if let Some(o) = dur(timing, "origin") {
        a.origin[bucket(o)] += 1;
    }
}

/// Middleware on the proxy routes: one sample per response.
pub async fn record(State(app): State<AppState>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_ascii_lowercase())
        .unwrap_or_default();
    let res = next.run(req).await;
    let state = res
        .headers()
        .get("x-glot-edge")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none");
    if state == "unknown" {
        return res; // not a customer domain: nothing to attribute
    }
    let timing = res
        .headers()
        .get_all("server-timing")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    let due = {
        let Ok(mut p) = PENDING.lock() else {
            return res;
        };
        let k = (host, Utc::now().date_naive());
        if p.by.len() < 10_000 || p.by.contains_key(&k) {
            add(p.by.entry(k).or_default(), state, &timing);
        }
        let due = p.flushed.elapsed() >= FLUSH_EVERY;
        if due {
            p.flushed = Instant::now();
        }
        due
    };
    if due {
        tokio::spawn(async move { flush(&app).await });
    }
    res
}

/// Writes the pending counters (host → its project); kept for the next flush if the database is unreachable.
pub async fn flush(state: &AppState) {
    let taken = match PENDING.lock() {
        Ok(mut p) => std::mem::take(&mut p.by),
        Err(_) => return,
    };
    for ((host, day), a) in taken {
        let r = sqlx::query(
            "INSERT INTO edge_metrics_daily AS m
                (project_id, day, requests, hits, translated, fail_open, glot_hist, origin_hist)
             SELECT project_id, $2, $3, $4, $5, $6, $7, $8 FROM edge_domains WHERE host = $1
             ON CONFLICT (project_id, day) DO UPDATE SET
                requests = m.requests + EXCLUDED.requests, hits = m.hits + EXCLUDED.hits,
                translated = m.translated + EXCLUDED.translated, fail_open = m.fail_open + EXCLUDED.fail_open,
                glot_hist = (SELECT array_agg(coalesce(x, 0) + coalesce(y, 0) ORDER BY i)
                             FROM unnest(m.glot_hist, EXCLUDED.glot_hist) WITH ORDINALITY AS t(x, y, i)),
                origin_hist = (SELECT array_agg(coalesce(x, 0) + coalesce(y, 0) ORDER BY i)
                               FROM unnest(m.origin_hist, EXCLUDED.origin_hist) WITH ORDINALITY AS t(x, y, i))",
        )
        .bind(&host)
        .bind(day)
        .bind(a.requests)
        .bind(a.hits)
        .bind(a.translated)
        .bind(a.fail_open)
        .bind(a.glot.to_vec())
        .bind(a.origin.to_vec())
        .execute(&state.db)
        .await;
        if r.is_err()
            && let Ok(mut p) = PENDING.lock()
        {
            let e = p.by.entry((host, day)).or_default();
            e.requests += a.requests;
            e.hits += a.hits;
            e.translated += a.translated;
            e.fail_open += a.fail_open;
            for i in 0..BUCKETS {
                e.glot[i] += a.glot[i];
                e.origin[i] += a.origin[i];
            }
        }
    }
}

async fn renderer() -> Option<Value> {
    let base = std::env::var("GLOT_RENDERER_URL").ok()?;
    let token = std::env::var("GLOT_RENDERER_TOKEN").unwrap_or_default();
    let r = reqwest::Client::new()
        .get(format!("{}/metrics", base.trim_end_matches('/')))
        .bearer_auth(token)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    r.json().await.ok()
}

#[derive(Deserialize)]
struct Q {
    days: Option<i64>,
}

type DayRow = (NaiveDate, i64, i64, i64, i64, Vec<i64>, Vec<i64>);

fn sum_hist(rows: &[DayRow], f: impl Fn(&DayRow) -> &Vec<i64>) -> Vec<i64> {
    let mut h = vec![0; BUCKETS];
    for r in rows {
        for (i, c) in f(r).iter().enumerate().take(BUCKETS) {
            h[i] += c;
        }
    }
    h
}

/// The dashboard's `EdgeMetrics` from the daily rows of the last `days` days (today included).
fn summary(days: i64, today: NaiveDate, rows: &[DayRow]) -> Value {
    let (requests, hits, translated, fail_open) = rows.iter().fold((0, 0, 0, 0), |a, r| {
        (a.0 + r.1, a.1 + r.2, a.2 + r.3, a.3 + r.4)
    });
    let glot = sum_hist(rows, |r| &r.5);
    let origin = sum_hist(rows, |r| &r.6);
    let by_day: Vec<Value> = (0..days)
        .rev()
        .map(|back| {
            let d = today - chrono::Duration::days(back);
            match rows.iter().find(|r| r.0 == d) {
                Some(r) => json!({ "date": d, "requests": r.1, "hits": r.2, "fail_open": r.4,
                                   "glot_p95_ms": pct(&r.5, 0.95) }),
                None => json!({ "date": d, "requests": 0, "hits": 0, "fail_open": 0, "glot_p95_ms": null }),
            }
        })
        .collect();
    json!({
        "days": days,
        "requests": requests,
        // share of translated responses answered without a translate pass (page cache or same origin body)
        "cache_hit_ratio": if translated > 0 { hits as f64 / translated as f64 } else { 0.0 },
        "fail_open": fail_open,
        "glot_p50_ms": pct(&glot, 0.5),
        "glot_p95_ms": pct(&glot, 0.95),
        "origin_p50_ms": pct(&origin, 0.5),
        "by_day": by_day,
    })
}

/// `GET /api/projects/{slug}/edge/metrics?days=7` (1–90).
async fn metrics(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<Q>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days = q.days.unwrap_or(7).clamp(1, 90);
    flush(&state).await;
    let today = Utc::now().date_naive();
    let rows: Vec<DayRow> = sqlx::query_as(
        "SELECT day, requests, hits, translated, fail_open, glot_hist, origin_hist FROM edge_metrics_daily
         WHERE project_id = $1 AND day > $2 ORDER BY day",
    )
    .bind(p.id)
    .bind(today - chrono::Duration::days(days))
    .fetch_all(&state.db)
    .await?;
    let mut out = summary(days, today, &rows);
    out["renderer"] = renderer().await.unwrap_or(Value::Null);
    Ok(Json(out))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/projects/{slug}/edge/metrics", get(metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_timing() {
        let t = r#"glot;dur=3.2;desc="translated miss", origin;dur=41.0"#;
        assert_eq!(dur(t, "glot"), Some(3.2));
        assert_eq!(dur(t, "origin"), Some(41.0));
        assert_eq!(dur(t, "cdn"), None);
    }

    #[test]
    fn aggregates_and_summary() {
        let mut a = Agg::default();
        add(&mut a, "translated", "glot;dur=12.0, origin;dur=40.0");
        add(
            &mut a,
            "translated; cache=hit",
            "glot;dur=0.4, origin;dur=0.0",
        );
        add(
            &mut a,
            "translated; cache=hit-body",
            "glot;dur=1.5, origin;dur=38.0",
        );
        add(&mut a, "fail-open", "glot;dur=400.0, origin;dur=20.0");
        add(&mut a, "passthrough", "glot;dur=0.2, origin;dur=5.0");
        assert_eq!(
            (a.requests, a.hits, a.translated, a.fail_open),
            (5, 2, 3, 1)
        );
        assert_eq!(pct(&a.glot, 0.5), Some(2.0));
        assert_eq!(pct(&a.glot, 0.95), Some(500.0));
        assert_eq!(pct(&[0; BUCKETS], 0.5), None);
        let today = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let rows = vec![(
            today,
            a.requests,
            a.hits,
            a.translated,
            a.fail_open,
            a.glot.to_vec(),
            a.origin.to_vec(),
        )];
        let s = summary(3, today, &rows);
        assert_eq!(s["requests"], 5);
        assert!((s["cache_hit_ratio"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(s["fail_open"], 1);
        assert_eq!(s["origin_p50_ms"], 20.0);
        let by = s["by_day"].as_array().unwrap();
        assert_eq!(by.len(), 3);
        assert_eq!(by[0]["date"], "2026-11-05");
        assert_eq!(by[0]["glot_p95_ms"], Value::Null);
        assert_eq!(by[2]["requests"], 5);
        assert_eq!(by[2]["glot_p95_ms"], 500.0);
    }
}
