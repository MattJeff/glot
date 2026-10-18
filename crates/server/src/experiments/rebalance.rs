//! Batch recalculation of running segmented experiments (DB side of `stats`): every `TICK`, each running
//! experiment is analysed from `xp_counts`; new weights are published to `xp_cells` once per UTC day (so the
//! weights in force on a day are known exactly, which the SRM check replays), decisions are checked every tick
//! (alpha spending only counts real decision looks), an SRM pauses the experiment. Snapshots go to `xp_analyses`.
//! Also serves `GET …/xp/{id}/results` and `POST …/xp/estimate`.
use axum::{Json, extract::State};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json as DbJson;
use std::{collections::BTreeMap, time::Duration};
use uuid::Uuid;

use super::{
    ingest::{EXPOSURE, SEEN},
    model::SegmentRule,
    stats::{self, Allocation, Analysis, Arm, Cell, Input, Metric},
};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

const DRAWS: usize = 4000;
/// A segment needing more than this to detect +20 % should be merged into `*`.
const MAX_DAYS: f64 = 90.0;

/// Analyses every `GLOT_XP_TICK_SECS` seconds (default 300; the e2e demo uses 2), first tick at boot.
pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    let secs = std::env::var("GLOT_XP_TICK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300u64)
        .max(1);
    tokio::spawn(async move {
        loop {
            tick(&state).await;
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    })
}

pub async fn tick(state: &AppState) {
    let ids: Vec<Uuid> = match sqlx::query_scalar(
        "SELECT id FROM xp_experiments WHERE status = 'running' AND started_at IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(ids) => ids,
        Err(e) => return tracing::error!("xp rebalance: {e}"),
    };
    for id in ids {
        if let Err(e) = rebalance(state, id).await {
            tracing::error!(experiment = %id, "xp rebalance failed: {}", e.message);
        }
    }
}

type Weights = BTreeMap<String, Vec<f64>>;

/// Counters → analysis, no write. `live`: any started experiment, no lock, no SRM (results page); otherwise
/// running only, under a transaction advisory lock (another server holds it ⇒ `None`).
struct Prepared {
    a: Analysis,
    cells: Vec<(String, i32, Option<i32>)>,
    publish: bool,
    holdout: bool,
    width: usize,
    look: i32,
}

async fn prepare(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    on_demand: bool,
) -> ApiResult<Option<Prepared>> {
    #[allow(clippy::type_complexity)]
    let exp: Option<(String, String, Option<Vec<f64>>, i32, String, String, DateTime<Utc>, i32, String)> =
        sqlx::query_as(
            "SELECT e.kind, e.allocation, e.weights, e.min_n, e.goal, e.metric, e.started_at, p.xp_holdout_pct,
                    e.exposure
             FROM xp_experiments e JOIN projects p ON p.id = e.project_id
             WHERE e.id = $1 AND e.started_at IS NOT NULL
               AND ($2 OR (e.status = 'running' AND pg_try_advisory_xact_lock(hashtext(e.id::text))))",
        )
        .bind(id)
        .bind(on_demand)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((kind, allocation, fixed, min_n, goal, metric, started_at, holdout_pct, exposure)) =
        exp
    else {
        return Ok(None);
    };
    let holdout = kind == "holdout";
    let arms: Vec<(i32, bool)> =
        sqlx::query_as("SELECT arm, approved FROM xp_arms WHERE experiment_id = $1 ORDER BY arm")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    // Holdout: arm 0 = held out, arm 1 = everyone else, split fixed by the project setting.
    let (allocation, fixed) = if holdout {
        let p = holdout_pct as f64 / 100.0;
        (Allocation::Fixed, Some(vec![p, 1.0 - p]))
    } else {
        (
            Allocation::parse(&allocation).unwrap_or(Allocation::Bandit),
            fixed,
        )
    };
    // Served arms = approved ones. Every weights vector is indexed by arm number (contract v1.1), gaps = 0.
    let width = arms.iter().map(|a| a.0 as usize + 1).max().unwrap_or(0);
    let live: Vec<usize> = arms
        .iter()
        .filter(|a| a.1 && !(holdout && fixed.as_ref().is_some_and(|w| w[0] <= 0.0)))
        .map(|a| a.0 as usize)
        .collect();
    if live.len() < 2 {
        return Ok(None);
    }
    let initial: Vec<f64> = match (&fixed, allocation) {
        (Some(w), Allocation::Fixed) if w.len() == width => w.clone(),
        _ => (0..width).map(|i| live.contains(&i) as u8 as f64).collect(),
    };
    let cells: Vec<(String, i32, Option<i32>)> = sqlx::query_as(
        "SELECT cell, looks, winner FROM xp_cells WHERE experiment_id = $1 AND NOT personalized
         ORDER BY cell = '*', cell",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    if cells.is_empty() {
        return Ok(None);
    }
    let counts: Vec<(String, i32, NaiveDate, String, i64, f64, f64)> = sqlx::query_as(
        "SELECT cell, arm, day, event, n, value_sum, value_sq FROM xp_counts
         WHERE experiment_id = $1 AND event IN ('$exposure', '$seen', $2)",
    )
    .bind(id)
    .bind(&goal)
    .fetch_all(&mut *tx)
    .await?;
    let history: Vec<(DateTime<Utc>, DbJson<Weights>)> = sqlx::query_as(
        "SELECT at, weights FROM xp_analyses WHERE experiment_id = $1 AND weights IS NOT NULL ORDER BY at",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let today = Utc::now().date_naive();
    let publish = !on_demand && history.last().is_none_or(|h| h.0.date_naive() < today);

    let mut input_cells: Vec<Cell> = cells
        .iter()
        .map(|(c, ..)| Cell {
            name: c.clone(),
            arms: vec![Arm::default(); live.len()],
            // SRM tested once a day (first tick), see `stats` module doc.
            expected: publish.then(|| vec![0.0; live.len()]),
        })
        .collect();
    // Denominator: documents served (`$exposure`) or, with `exposure = 'seen'`, blocks actually seen (`$seen`).
    // The SRM uses the same series (the client cannot see an arm it was not served).
    let views_event = if exposure == "seen" && !holdout {
        SEEN
    } else {
        EXPOSURE
    };
    // (cell, arm number) -> (seen, Σ visible ms), reported in the results whatever the denominator.
    let mut seen: BTreeMap<(usize, i32), (i64, f64)> = BTreeMap::new();
    // Exposures per (cell, day), to replay the weights in force that day.
    let mut daily: BTreeMap<(usize, NaiveDate), i64> = BTreeMap::new();
    for (cell, arm, day, event, n, sum, sq) in &counts {
        let (Some(ci), Some(j)) = (
            cells.iter().position(|c| &c.0 == cell),
            live.iter().position(|l| *l as i32 == *arm),
        ) else {
            continue;
        };
        if event == SEEN {
            let s = seen.entry((ci, *arm)).or_default();
            *s = (s.0 + n, s.1 + sum);
        }
        let a = &mut input_cells[ci].arms[j];
        if event == views_event {
            a.views += n;
            *daily.entry((ci, *day)).or_default() += n;
        } else if event == EXPOSURE || event == SEEN {
            continue;
        } else {
            a.conversions += n;
            a.revenue += sum;
            a.revenue_sq += sq;
        }
    }
    for ((ci, day), n) in daily {
        let w = history
            .iter()
            .rev()
            .find(|h| h.0.date_naive() <= day)
            .and_then(|h| h.1.get(&cells[ci].0).cloned())
            .filter(|w| w.len() == width)
            .unwrap_or_else(|| initial.clone());
        let s: f64 = live.iter().map(|i| w[*i]).sum();
        if let Some(e) = input_cells[ci].expected.as_mut().filter(|_| s > 0.0) {
            for (j, i) in live.iter().enumerate() {
                e[j] += n as f64 * w[*i] / s;
            }
        }
    }
    let look = cells.iter().map(|c| c.1).max().unwrap_or(0);
    let fixed_live: Option<Vec<f64>> = fixed
        .as_ref()
        .filter(|w| w.len() == width)
        .map(|w| live.iter().map(|i| w[*i]).collect());
    let mut a = stats::analyze(
        &Input {
            metric: Metric::parse(&metric).unwrap_or(Metric::Conversion),
            allocation,
            fixed: fixed_live.as_deref(),
            min_n: min_n as i64,
            days: (Utc::now() - started_at).num_seconds() as f64 / 86_400.0,
            look,
            cells: &input_cells,
            holdout: None,
            draws: DRAWS,
        },
        &mut rand::rng(),
    )
    .map_err(|e| AppError::bad_request(format!("experiment not analysable: {e:?}")))?;
    a.arm_ids = live.iter().map(|i| *i as i32).collect();
    let mut a = a.by_arm_number();
    let mut total: BTreeMap<i32, (i64, f64)> = BTreeMap::new();
    for ((ci, arm), (n, ms)) in &seen {
        let t = total.entry(*arm).or_default();
        *t = (t.0 + n, t.1 + ms);
        if let Some(r) = a
            .cells
            .get_mut(*ci)
            .and_then(|c| c.arms.get_mut(*arm as usize))
        {
            (r.seen, r.visible_ms) = (Some(*n), (*n > 0).then(|| ms / *n as f64));
        }
    }
    for (arm, (n, ms)) in total {
        if let Some(r) = a.global.arms.get_mut(arm as usize) {
            (r.seen, r.visible_ms) = (Some(n), (n > 0).then(|| ms / n as f64));
        }
    }
    Ok(Some(Prepared {
        a,
        cells,
        publish,
        holdout,
        width,
        look,
    }))
}

/// One analysis point for one experiment. `None` when it is not running (or another server holds it).
pub async fn rebalance(state: &AppState, id: Uuid) -> ApiResult<Option<Analysis>> {
    let mut tx = state.db.begin().await?;
    let Some(Prepared {
        a,
        cells,
        publish,
        holdout,
        width,
        look,
    }) = prepare(&mut tx, id, false).await?
    else {
        return Ok(None);
    };
    let new_look = look + a.decision_look as i32;
    let mut weights = Weights::new();
    let mut decided = false;
    for (r, (cell, _, winner)) in a.cells.iter().zip(&cells) {
        let mut w: Vec<f64> = r.arms.iter().map(|x| x.weight).collect();
        w.resize(width, 0.0);
        let won = (!holdout && winner.is_none())
            .then(|| r.winner.map(|n| n as i32))
            .flatten();
        decided |= won.is_some();
        sqlx::query(
            "UPDATE xp_cells SET weights = CASE WHEN $3 THEN $4 ELSE weights END, looks = $5,
                    winner = coalesce(winner, $6),
                    decided_at = CASE WHEN winner IS NULL AND $6 IS NOT NULL THEN now() ELSE decided_at END
             WHERE experiment_id = $1 AND cell = $2",
        )
        .bind(id)
        .bind(cell)
        .bind(publish)
        .bind(&w)
        .bind(new_look)
        .bind(won)
        .execute(&mut *tx)
        .await?;
        weights.insert(cell.clone(), w);
    }
    if a.paused {
        sqlx::query(
            "UPDATE xp_experiments SET status = 'paused' WHERE id = $1 AND status = 'running'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        // ponytail: the alert is this log line + the `paused` snapshot (dashboard banner); email when asked.
        tracing::warn!(experiment = %id, "sample ratio mismatch: experiment paused");
    }
    if publish || a.decision_look || a.paused || decided {
        sqlx::query(
            "INSERT INTO xp_analyses (experiment_id, look, decision, paused, weights, analysis)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(look)
        .bind(a.decision_look)
        .bind(a.paused)
        .bind(publish.then_some(DbJson(&weights)))
        .bind(DbJson(&a))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Some(a))
}

// ---------- REST ----------

#[derive(Serialize, utoipa::ToSchema)]
pub struct Snapshot {
    pub at: DateTime<Utc>,
    pub look: i32,
    pub decision: bool,
    pub paused: bool,
    #[schema(value_type = Object)]
    pub analysis: serde_json::Value,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Results {
    #[schema(value_type = Option<Object>)]
    pub latest: Option<serde_json::Value>,
    pub history: Vec<Snapshot>,
}

/// `GET /api/projects/{slug}/xp/{id}/results`: analysis snapshots, latest first (≤ 50).
pub async fn results(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Results>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<(DateTime<Utc>, i32, bool, bool, serde_json::Value)> = sqlx::query_as(
        "SELECT a.at, a.look, a.decision, a.paused, a.analysis FROM xp_analyses a
         JOIN xp_experiments e ON e.id = a.experiment_id
         WHERE a.experiment_id = $1 AND e.project_id = $2 ORDER BY a.at DESC LIMIT 50",
    )
    .bind(id)
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM xp_experiments WHERE id = $1 AND project_id = $2",
        )
        .bind(id)
        .bind(p.id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("experiment not found"))?;
    }
    let history: Vec<Snapshot> = rows
        .into_iter()
        .map(|(at, look, decision, paused, analysis)| Snapshot {
            at,
            look,
            decision,
            paused,
            analysis,
        })
        .collect();
    let latest = match live(&state, id, history.first().map(|s| &s.analysis)).await? {
        Some(a) => Some(a),
        None => history.first().map(|s| s.analysis.clone()),
    };
    Ok(Json(Results { latest, history }))
}

// ponytail: per-process cache, entries dropped when stale; fine for a handful of experiments per server.
static LIVE: std::sync::LazyLock<
    std::sync::Mutex<BTreeMap<Uuid, (std::time::Instant, serde_json::Value)>>,
> = std::sync::LazyLock::new(Default::default);
const LIVE_TTL: Duration = Duration::from_secs(30);

/// Analysis computed now from `xp_counts` (cached `LIVE_TTL`), `live: true`. Decisions are the worker's: cell
/// winners come from `xp_cells`, recommendations from the last snapshot; no look spent, nothing written.
async fn live(
    state: &AppState,
    id: Uuid,
    last: Option<&serde_json::Value>,
) -> ApiResult<Option<serde_json::Value>> {
    if let Some((at, v)) = LIVE.lock().unwrap().get(&id)
        && at.elapsed() < LIVE_TTL
    {
        return Ok(Some(v.clone()));
    }
    let mut conn = state.db.acquire().await?;
    let Some(Prepared { mut a, cells, .. }) = prepare(&mut conn, id, true).await? else {
        return Ok(None);
    };
    a.live = true;
    a.decision_look = false;
    a.global.winner = None;
    for (r, (_, _, winner)) in a.cells.iter_mut().zip(&cells) {
        r.winner = winner.map(|w| w as usize);
    }
    let rec = |k: &str| last.map(|l| l[k].clone()).unwrap_or_default();
    a.everywhere = serde_json::from_value(rec("everywhere")).unwrap_or_default();
    a.segments = serde_json::from_value(rec("segments")).unwrap_or_default();
    let v = serde_json::to_value(&a).map_err(AppError::internal)?;
    let mut cache = LIVE.lock().unwrap();
    cache.retain(|_, (at, _)| at.elapsed() < LIVE_TTL);
    cache.insert(id, (std::time::Instant::now(), v.clone()));
    Ok(Some(v))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct EstimateRequest {
    /// Segment to estimate (`null` = all traffic).
    #[serde(default)]
    pub rule: Option<SegmentRule>,
    /// Arms including the official one (default 2).
    #[serde(default)]
    pub arms: Option<usize>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub min_n: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Estimate {
    /// Visitors per day matching the segment (last 30 days of `insight_daily`).
    pub daily_visitors: f64,
    /// Goal rate of that traffic.
    pub baseline: Option<f64>,
    /// Visitors per arm to detect +20 % (80 % power), at least `min_n`.
    pub per_arm: Option<i64>,
    /// Estimated duration, never under 7 days.
    pub days: Option<f64>,
    /// > 90 days: merge the segment into `*` (blocking warning, can be overridden).
    pub too_long: bool,
    /// False when the rule uses attributes `insight_daily` does not record (device, source…): an upper bound.
    pub exact: bool,
}

/// `POST /api/projects/{slug}/xp/estimate`: traffic and duration of a segment before creating the experiment.
pub async fn estimate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(req): AppJson<EstimateRequest>,
) -> ApiResult<Json<Estimate>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    if let Some(r) = &req.rule {
        r.validate().map_err(AppError::bad_request)?;
    }
    let goal = req.goal.unwrap_or_else(|| "signup".into());
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT served, country, event, sum(n)::bigint FROM insight_daily
         WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - 30 AND event IN ('$view', $2)
         GROUP BY 1, 2, 3",
    )
    .bind(p.id)
    .bind(&goal)
    .fetch_all(&state.db)
    .await?;
    // Only country and locale are known per row; other predicates are left out (upper bound).
    let known = |a: &str| a == "country" || a == "locale";
    let exact = req
        .rule
        .as_ref()
        .is_none_or(|r| r.all.iter().all(|p| known(&p.attr)));
    let rule = req.rule.map(|r| SegmentRule {
        all: r.all.into_iter().filter(|p| known(&p.attr)).collect(),
    });
    let (mut views, mut conv) = (0i64, 0i64);
    for (served, country, event, n) in rows {
        let ctx = BTreeMap::from([
            ("country".to_string(), country),
            ("locale".to_string(), served),
        ]);
        if rule
            .as_ref()
            .is_some_and(|r| !r.all.is_empty() && !r.matches(&ctx))
        {
            continue;
        }
        if event == "$view" {
            views += n;
        } else {
            conv += n;
        }
    }
    let daily_visitors = views as f64 / 30.0;
    let baseline = (views > 0).then(|| conv.min(views) as f64 / views as f64);
    let per_arm = baseline
        .and_then(crate::insights::required_per_arm)
        .map(|n| n.max(req.min_n.unwrap_or(500)));
    let days = per_arm.filter(|_| daily_visitors > 0.0).map(|n| {
        (n as f64 * req.arms.unwrap_or(2).clamp(2, 8) as f64 / daily_visitors).max(stats::MIN_DAYS)
    });
    Ok(Json(Estimate {
        daily_visitors,
        baseline,
        per_arm,
        days,
        too_long: days.is_none_or(|d| d > MAX_DAYS),
        exact,
    }))
}
