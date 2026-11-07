//! Language launch measurement: did adding a language move the goal conversion of the visitors who prefer it?
//!
//! **Holdout** (the locale is served at runtime): a language experiment (`insight_experiments`, same per-session
//! draw as `insights::ingest`) where the test arm gets the new locale and the control arm is *held* on the
//! fallback (`hold_locale`) for N days, then everyone gets it (`rolled_out`). Analysed with the Bayesian engine of
//! segmented experiments (`experiments::stats`: one cell, 2 arms, alpha-spending looks): relative lift with a 95 %
//! credible interval, SRM χ² on the assignment counts (p < 0.001 ⇒ `invalid`), no conclusion before 7 days and
//! `min_n` visitors per arm.
//!
//! **Rollout to 100 %** (stopping the holdout) needs every condition (`blockers`): ≥ 7 days, `min_n` visitors and
//! `min_conversions` (default 30) goal conversions per arm, no SRM, a credible lift (`probably_positive`), and no
//! guardrail event degraded: for each guardrail (e.g. `purchase`, or `error` with `higher_is_worse`),
//! P(new locale worse by more than 5 % relative) ≤ 0.2 with at least `min_conversions` events across both arms.
//! It is automatic only when the launch opted in (`auto_rollout`, default off); otherwise the launch waits in
//! `ready` for a confirmation (`POST …/rollout`). A launch that ends without meeting them (`done`, `invalid`)
//! keeps its holdout until the owner decides: the experiment no longer lapses at the end date.
//!
//! **Audience and assignment.** A session whose visitor picked a language (switcher, `explicit`) is never assigned
//! (or leaves its arm: `$explicit`), so the effect measured is the effect of *automatic* localisation on visitors
//! without an explicit preference. The arm is drawn on the session's first visit, before anything is rendered in
//! the new locale, and kept for the session (`@glot/insights` returns it from sessionStorage on every hit). There is
//! deliberately no visitor id (no cookie, no localStorage id): a returning visitor in a new session is drawn again.
//!
//! **Before/after** (compiled-in catalogs, no holdout possible): the same audience N days before vs since the
//! launch. Never causal (seasonality, campaigns, trends): verdicts say "higher/lower after", not "positive".
//!
//! **Share**: a read-only public story (aggregates only) behind `{id}.{HMAC(secret, id, nonce)}`; revoking
//! clears the nonce, re-sharing draws a new one (old links stay dead).
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use hmac::{Hmac, Mac};
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    experiments::stats::{self, Allocation, Metric},
    insights::{self, EXPLICIT, lang},
    projects::{self, Role},
    validate_locale,
    variants::{WIN_PROB, draw},
};

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/launches";
    Router::new()
        .route(p, get(list).post(create))
        .route(&format!("{p}/{{id}}"), get(detail))
        .route(&format!("{p}/{{id}}/share"), post(share).delete(revoke))
        .route(&format!("{p}/{{id}}/rollout"), post(rollout))
}

/// Public, unauthenticated: the shared case study.
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/api/public/launches/{token}", get(public))
}

// ---------- stats (pure) ----------

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Counts {
    pub visits: i64,
    pub conversions: i64,
}

pub struct Eval {
    pub holdout: bool,
    /// [fallback / before, new locale / after], explicit choices already removed.
    pub arms: [Counts; 2],
    /// Sessions assigned per arm (holdout SRM), explicit choices included.
    pub assigned: [i64; 2],
    pub holdout_pct: i32,
    pub min_n: i64,
    pub days: f64,
    pub look: i32,
    pub ended: bool,
    pub seed: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Relative lift of arm 1 over arm 0: posterior mean and 95 % credible interval.
    pub lift: f64,
    pub lift_lo: f64,
    pub lift_hi: f64,
    /// P(arm 1 converts better than arm 0).
    pub p_better: f64,
    pub srm_p: Option<f64>,
    /// Minimum time (7 days) and sample (`min_n` per arm) reached.
    pub enough: bool,
    #[serde(skip)]
    pub decision_look: bool,
    #[serde(skip)]
    pub winner: Option<usize>,
    /// holdout: collecting | probably_positive | probably_negative | inconclusive | invalid;
    /// before_after: collecting | higher_after | lower_after | no_clear_change.
    pub verdict: &'static str,
}

const DRAWS: usize = 4000;
const VIEW: &str = "$view";

pub fn evaluate(e: &Eval) -> Outcome {
    let cells = [stats::Cell {
        name: "*".into(),
        arms: e
            .arms
            .iter()
            .map(|c| stats::Arm {
                views: c.visits.max(0),
                conversions: c.conversions.clamp(0, c.visits.max(0)),
                ..Default::default()
            })
            .collect(),
        expected: None, // SRM below, on assignments (explicit opt-outs are not a randomization failure)
    }];
    let a = stats::analyze(
        &stats::Input {
            metric: Metric::Conversion,
            allocation: Allocation::Fixed,
            fixed: None,
            min_n: e.min_n,
            days: e.days,
            look: e.look,
            cells: &cells,
            holdout: None,
            draws: DRAWS,
        },
        &mut StdRng::seed_from_u64(e.seed),
    )
    .expect("one cell, two arms");
    let pct = e.holdout_pct as f64 / 100.0;
    let srm_p = e
        .holdout
        .then(|| stats::srm_p(&e.assigned, &[pct, 1.0 - pct]));
    let invalid = srm_p.is_some_and(|p| p < stats::SRM_P);
    let r = &a.global.arms[1];
    let enough = e.days >= stats::MIN_DAYS && e.arms.iter().all(|c| c.visits >= e.min_n);
    let winner = a.global.winner.filter(|_| e.holdout && !invalid);
    let p = r.p_beats_control;
    let verdict = if invalid {
        "invalid"
    } else if !enough && !e.ended {
        "collecting"
    } else if !e.holdout {
        match p {
            p if p >= WIN_PROB => "higher_after",
            p if p <= 1.0 - WIN_PROB => "lower_after",
            _ => "no_clear_change",
        }
    } else {
        match winner {
            Some(1) => "probably_positive",
            Some(_) => "probably_negative",
            // ponytail: one final look at 95 % on top of the alpha-spent interim looks (slightly above 5 %).
            None if e.ended && p >= WIN_PROB => "probably_positive",
            None if e.ended && p <= 1.0 - WIN_PROB => "probably_negative",
            None => "inconclusive",
        }
    };
    Outcome {
        lift: r.lift,
        lift_lo: r.lift_lo,
        lift_hi: r.lift_hi,
        p_better: p,
        srm_p,
        enough,
        decision_look: a.decision_look && !invalid && e.holdout,
        winner,
        verdict,
    }
}

/// A guardrail event: rollout is blocked if the new locale makes it worse.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Guardrail {
    pub event: String,
    /// `error`-like events: a rise is the degradation (default: a drop, e.g. `purchase`).
    #[serde(default)]
    pub higher_is_worse: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GuardrailResult {
    pub event: String,
    pub higher_is_worse: bool,
    /// Events per arm [fallback / before, new locale / after].
    pub counts: [i64; 2],
    /// P(new locale worse than the fallback by more than `GUARDRAIL_TOLERANCE`, relative).
    pub p_worse: f64,
    /// ok | degraded | insufficient
    pub status: &'static str,
}

/// Relative degradation a guardrail tolerates, and the probability of exceeding it that blocks a rollout.
pub const GUARDRAIL_TOLERANCE: f64 = 0.05;
pub const GUARDRAIL_MAX_P: f64 = 0.2;

/// Beta posteriors per arm (uniform prior), seeded: `insufficient` under `min_events` events across both arms.
pub fn guardrail(
    g: &Guardrail,
    visits: [i64; 2],
    counts: [i64; 2],
    min_events: i64,
    seed: u64,
) -> GuardrailResult {
    let mut rng = StdRng::seed_from_u64(seed);
    let worse = (0..DRAWS)
        .filter(|_| {
            let c = draw(visits[0], counts[0], &mut rng);
            let t = draw(visits[1], counts[1], &mut rng);
            if g.higher_is_worse {
                t > c * (1.0 + GUARDRAIL_TOLERANCE)
            } else {
                t < c * (1.0 - GUARDRAIL_TOLERANCE)
            }
        })
        .count() as f64
        / DRAWS as f64;
    GuardrailResult {
        event: g.event.clone(),
        higher_is_worse: g.higher_is_worse,
        counts,
        p_worse: worse,
        status: if counts[0] + counts[1] < min_events {
            "insufficient"
        } else if worse > GUARDRAIL_MAX_P {
            "degraded"
        } else {
            "ok"
        },
    }
}

/// What still prevents a rollout to 100 % (empty = ready). Codes, translated by the dashboard.
pub fn blockers(
    holdout: bool,
    o: &Outcome,
    arms: &[Counts; 2],
    days: f64,
    min_n: i64,
    min_conversions: i64,
    guards: &[GuardrailResult],
) -> Vec<&'static str> {
    if !holdout {
        return vec!["no_holdout"];
    }
    let mut b = vec![];
    if days < stats::MIN_DAYS {
        b.push("min_days");
    }
    if arms.iter().any(|a| a.visits < min_n) {
        b.push("min_visitors");
    }
    if arms.iter().any(|a| a.conversions < min_conversions) {
        b.push("min_conversions");
    }
    if o.srm_p.is_some_and(|p| p < stats::SRM_P) {
        b.push("srm");
    }
    if o.verdict != "probably_positive" {
        b.push("no_credible_lift");
    }
    if guards.iter().any(|g| g.status != "ok") {
        b.push("guardrail");
    }
    b
}

// ---------- storage ----------

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    project_id: Uuid,
    locale: String,
    mode: String,
    experiment_id: Option<Uuid>,
    fallback: String,
    goal: String,
    holdout_pct: i32,
    days: i32,
    min_n: i32,
    launched_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    status: String,
    verdict: Option<String>,
    looks: i32,
    decided_at: Option<DateTime<Utc>>,
    baseline: DbJson<Counts>,
    cost_usd: f64,
    cost_estimated: bool,
    recommendation: Option<DbJson<Value>>,
    share_nonce: Option<String>,
    min_conversions: i32,
    guardrails: DbJson<Vec<Guardrail>>,
    auto_rollout: bool,
}

const COLS: &str = "id, project_id, locale, mode, experiment_id, fallback, goal, holdout_pct, days, min_n, launched_at,
    ends_at, status, verdict, looks, decided_at, baseline, cost_usd, cost_estimated, recommendation, share_nonce,
    min_conversions, guardrails, auto_rollout";

#[derive(Serialize, Clone, Debug)]
pub struct Point {
    pub day: NaiveDate,
    /// [fallback / before, new locale / after]
    pub visits: [i64; 2],
    pub conversions: [i64; 2],
}

struct Measure {
    arms: [Counts; 2],
    assigned: [i64; 2],
    explicit: [i64; 2],
    /// Guardrail event counts per arm, in `Row::guardrails` order.
    guards: Vec<[i64; 2]>,
    daily: Vec<Point>,
}

/// Counters of the audience (visitors preferring the launched language) over the measured window.
async fn measure(state: &AppState, r: &Row, end: DateTime<Utc>) -> ApiResult<Measure> {
    let holdout = r.mode == "holdout";
    let from = if holdout {
        r.launched_at
    } else {
        r.launched_at - chrono::Duration::days(r.days as i64)
    };
    let mut events = vec![VIEW.to_string(), EXPLICIT.to_string(), r.goal.clone()];
    events.extend(r.guardrails.0.iter().map(|g| g.event.clone()));
    let rows: Vec<(NaiveDate, String, String, i64)> = sqlx::query_as(
        "SELECT day, arm, event, sum(n)::int8 FROM insight_daily WHERE project_id = $1 AND preferred = $2
           AND day BETWEEN ($3 AT TIME ZONE 'utc')::date AND ($4 AT TIME ZONE 'utc')::date AND event = ANY($5)
         GROUP BY day, arm, event ORDER BY day",
    )
    .bind(r.project_id)
    .bind(lang(&r.locale))
    .bind(from)
    .bind(end)
    .bind(&events)
    .fetch_all(&state.db)
    .await?;
    let launch_day = r.launched_at.date_naive();
    let mut m = Measure {
        arms: Default::default(),
        assigned: [0; 2],
        explicit: [0; 2],
        guards: vec![[0; 2]; r.guardrails.0.len()],
        daily: vec![],
    };
    let mut days: BTreeMap<NaiveDate, Point> = BTreeMap::new();
    for (day, arm, event, n) in rows {
        let i = match (holdout, arm.as_str()) {
            (true, "control") => 0,
            (true, "test") => 1,
            (true, _) => continue, // explicit choices and visitors outside the holdout
            (false, _) => (day >= launch_day) as usize,
        };
        let p = days.entry(day).or_insert(Point {
            day,
            visits: [0; 2],
            conversions: [0; 2],
        });
        // An explicit choice withdraws a session already counted as a visit.
        if event == VIEW {
            m.assigned[i] += n;
            m.arms[i].visits += n;
            p.visits[i] += n;
        } else if event == EXPLICIT {
            m.explicit[i] += n;
            m.arms[i].visits -= n;
            p.visits[i] -= n;
        } else if event == r.goal {
            m.arms[i].conversions += n;
            p.conversions[i] += n;
        } else if let Some(j) = r.guardrails.0.iter().position(|g| g.event == event) {
            m.guards[j][i] += n;
        }
    }
    for a in &mut m.arms {
        a.visits = a.visits.max(0);
    }
    m.daily = days
        .into_values()
        .map(|mut p| {
            p.visits = p.visits.map(|v| v.max(0));
            p
        })
        .collect();
    Ok(m)
}

/// Translation spend for `locale`: jobs naming it, a multi-locale job prorated per locale.
// ponytail: jobs without an explicit locale list (all targets) are not attributed; the creation estimate covers it.
async fn jobs_cost(state: &AppState, project_id: Uuid, locale: &str) -> ApiResult<f64> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(sum(cost_usd / greatest(jsonb_array_length(params->'locales'), 1)), 0)::float8
         FROM jobs WHERE project_id = $1 AND jsonb_typeof(params->'locales') = 'array' AND params->'locales' ? $2",
    )
    .bind(project_id)
    .bind(locale)
    .fetch_one(&state.db)
    .await?)
}

fn seed(id: Uuid) -> u64 {
    id.as_u64_pair().0
}

struct Checked {
    m: Measure,
    o: Outcome,
    guards: Vec<GuardrailResult>,
    blockers: Vec<&'static str>,
    ended: bool,
}

/// End of the measured window, the evaluation of it and the rollout conditions.
async fn outcome(state: &AppState, r: &Row) -> ApiResult<Checked> {
    let now = Utc::now();
    let end = r.decided_at.unwrap_or(now).min(r.ends_at);
    let ended = r.decided_at.is_some() || now >= r.ends_at;
    let m = measure(state, r, end).await?;
    let holdout = r.mode == "holdout";
    let days = (end - r.launched_at).num_seconds() as f64 / 86_400.0;
    let o = evaluate(&Eval {
        holdout,
        arms: m.arms,
        assigned: m.assigned,
        holdout_pct: r.holdout_pct,
        min_n: r.min_n as i64,
        days,
        look: r.looks,
        ended,
        seed: seed(r.id),
    });
    let visits = [m.arms[0].visits, m.arms[1].visits];
    let guards: Vec<GuardrailResult> = r
        .guardrails
        .0
        .iter()
        .zip(&m.guards)
        .map(|(g, c)| guardrail(g, visits, *c, r.min_conversions as i64, seed(r.id)))
        .collect();
    let blockers = blockers(
        holdout,
        &o,
        &m.arms,
        days,
        r.min_n as i64,
        r.min_conversions as i64,
        &guards,
    );
    Ok(Checked {
        m,
        o,
        guards,
        blockers,
        ended,
    })
}

// ---------- story ----------

/// The case study: everything here is an aggregate (no visitor, country or route), safe to publish.
#[derive(Serialize)]
pub struct Story {
    pub project: String,
    pub locale: String,
    pub fallback: String,
    pub mode: String,
    pub goal: String,
    pub status: String,
    pub verdict: String,
    /// false for before/after: a correlation, not a measured effect.
    pub causal: bool,
    pub launched_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub days: i32,
    pub holdout_pct: i32,
    pub min_n: i32,
    /// Goal conversions needed in each arm before a rollout.
    pub min_conversions: i32,
    /// The holdout goes to 100 % on its own when every condition holds (else `ready` waits for a confirmation).
    pub auto_rollout: bool,
    pub guardrails: Vec<GuardrailResult>,
    /// Conditions a rollout still misses (empty = all met): min_days, min_visitors, min_conversions, srm,
    /// no_credible_lift, guardrail, no_holdout.
    pub blockers: Vec<&'static str>,
    pub baseline: Counts,
    pub cost_usd: f64,
    pub cost_estimated: bool,
    pub recommendation: Option<Value>,
    /// [fallback / before, new locale / after]
    pub arms: [Counts; 2],
    /// Sessions withdrawn by an explicit language choice, per arm.
    pub explicit: [i64; 2],
    #[serde(flatten)]
    pub result: Outcome,
    pub daily: Vec<Point>,
}

#[derive(Serialize)]
pub struct Launch {
    pub id: Uuid,
    /// Present while the public link is active.
    pub share_token: Option<String>,
    #[serde(flatten)]
    pub story: Story,
}

async fn story(state: &AppState, r: &Row, project: String) -> ApiResult<Story> {
    let Checked {
        m,
        mut o,
        guards,
        blockers,
        ..
    } = outcome(state, r).await?;
    if let Some(v) = r.verdict.as_deref() {
        // Frozen at the decision: a later look must not rewrite a published story.
        o.verdict = VERDICTS
            .iter()
            .find(|x| **x == v)
            .copied()
            .unwrap_or(o.verdict);
    }
    let spent = jobs_cost(state, r.project_id, &r.locale).await?;
    let (cost_usd, cost_estimated) = if spent > 0.0 {
        (spent, false)
    } else {
        (r.cost_usd, r.cost_estimated)
    };
    Ok(Story {
        project,
        locale: r.locale.clone(),
        fallback: r.fallback.clone(),
        mode: r.mode.clone(),
        goal: r.goal.clone(),
        status: r.status.clone(),
        verdict: o.verdict.to_string(),
        causal: r.mode == "holdout",
        launched_at: r.launched_at,
        ends_at: r.ends_at,
        decided_at: r.decided_at,
        days: r.days,
        holdout_pct: r.holdout_pct,
        min_n: r.min_n,
        min_conversions: r.min_conversions,
        auto_rollout: r.auto_rollout,
        guardrails: guards,
        blockers,
        baseline: r.baseline.0,
        cost_usd,
        cost_estimated,
        recommendation: r.recommendation.as_ref().map(|j| j.0.clone()),
        arms: m.arms,
        explicit: m.explicit,
        result: o,
        daily: m.daily,
    })
}

const VERDICTS: [&str; 8] = [
    "collecting",
    "probably_positive",
    "probably_negative",
    "inconclusive",
    "invalid",
    "higher_after",
    "lower_after",
    "no_clear_change",
];

fn token(secret: &[u8; 32], id: Uuid, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(b"launch-share:");
    mac.update(id.as_bytes());
    mac.update(nonce.as_bytes());
    format!(
        "{}.{}",
        id.simple(),
        hex::encode(&mac.finalize().into_bytes()[..16])
    )
}

async fn load(state: &AppState, project_id: Uuid, id: Uuid) -> ApiResult<Row> {
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM language_launches WHERE id = $1 AND project_id = $2"
    ))
    .bind(id)
    .bind(project_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("launch not found"))
}

async fn view(state: &AppState, r: &Row, project: &str) -> ApiResult<Launch> {
    Ok(Launch {
        id: r.id,
        share_token: r
            .share_nonce
            .as_deref()
            .map(|n| token(&state.secret, r.id, n)),
        story: story(state, r, project.to_string()).await?,
    })
}

// ---------- dashboard ----------

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<Launch>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM language_launches WHERE project_id = $1 ORDER BY launched_at DESC LIMIT 50"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let mut out = vec![];
    for r in &rows {
        out.push(view(&state, r, &p.name).await?);
    }
    Ok(Json(out))
}

pub async fn detail(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Launch>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let r = load(&state, p.id, id).await?;
    Ok(Json(view(&state, &r, &p.name).await?))
}

#[derive(Deserialize)]
pub struct NewLaunch {
    locale: String,
    /// holdout (default) | before_after
    mode: Option<String>,
    /// Share of the audience held on the fallback (holdout), default 20.
    holdout_pct: Option<i32>,
    /// N, default 14.
    days: Option<i32>,
    goal: Option<String>,
    /// Default: the source locale.
    fallback: Option<String>,
    min_n: Option<i32>,
    /// Goal conversions per arm before a rollout, default 30.
    min_conversions: Option<i32>,
    #[serde(default)]
    guardrails: Vec<Guardrail>,
    /// Go to 100 % without confirmation once every condition holds (default off).
    #[serde(default)]
    auto_rollout: bool,
}

/// glot's recommendation for `locale` (`recommend.rs`) from the data available at launch, computed as if it were
/// not offered yet (the launch is what makes it live); `None` when the recommender has nothing on it.
async fn recommendation(
    state: &AppState,
    p: &projects::ProjectRow,
    locale: &str,
    goal: &str,
) -> ApiResult<Option<Value>> {
    let mut before = p.clone();
    before.target_locales.retain(|l| lang(l) != lang(locale));
    let r = crate::recommend::for_project(state, &before, 30, goal).await?;
    let Some(i) = r
        .recommendations
        .iter()
        .position(|x| lang(&x.locale) == lang(locale))
    else {
        return Ok(None);
    };
    let mut v = serde_json::to_value(&r.recommendations[i]).map_err(AppError::internal)?;
    v["rank"] = (i + 1).into();
    v["of"] = r.recommendations.len().into();
    v["at"] = serde_json::to_value(Utc::now()).map_err(AppError::internal)?;
    Ok(Some(v))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<NewLaunch>,
) -> ApiResult<(StatusCode, Json<Launch>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    crate::billing::require_growth(&state, p.id, "Language launches").await?;
    validate_locale(&b.locale)?;
    let fallback = b.fallback.unwrap_or_else(|| p.source_locale.clone());
    validate_locale(&fallback)?;
    if lang(&b.locale) == lang(&p.source_locale) || lang(&fallback) == lang(&b.locale) {
        return Err(AppError::bad_request(
            "the launched locale must differ from the source and the fallback",
        ));
    }
    let mode = b.mode.unwrap_or_else(|| "holdout".into());
    let holdout = match mode.as_str() {
        "holdout" => true,
        "before_after" => false,
        _ => {
            return Err(AppError::bad_request(
                "mode must be holdout or before_after",
            ));
        }
    };
    let holdout_pct = if holdout {
        b.holdout_pct.unwrap_or(20)
    } else {
        0
    };
    let days = b.days.unwrap_or(14);
    let min_n = b.min_n.unwrap_or(200);
    if (holdout && !(5..=50).contains(&holdout_pct)) || !(7..=90).contains(&days) || min_n < 50 {
        return Err(AppError::bad_request(
            "holdout_pct must be 5-50, days 7-90 and min_n ≥ 50",
        ));
    }
    let goal = b.goal.unwrap_or_else(|| "signup".into());
    insights::check_event(&goal)?;
    let min_conversions = b.min_conversions.unwrap_or(30);
    if min_conversions < 1 || b.guardrails.len() > 5 {
        return Err(AppError::bad_request(
            "min_conversions must be ≥ 1 and guardrails at most 5",
        ));
    }
    for (i, g) in b.guardrails.iter().enumerate() {
        insights::check_event(&g.event)?;
        if g.event == goal || b.guardrails[..i].iter().any(|x| x.event == g.event) {
            return Err(AppError::bad_request(
                "a guardrail must differ from the goal and the other guardrails",
            ));
        }
    }

    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1 ORDER BY key")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let rec = recommendation(&state, &p, &b.locale, &goal).await?;
    let spent = jobs_cost(&state, p.id, &b.locale).await?;
    let (cost, estimated) = if spent > 0.0 {
        (spent, false)
    } else {
        (insights::estimate_cost(&p, &keys, &b.locale), true)
    };
    let baseline: (i64, i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(n) FILTER (WHERE event = '$view'), 0)::int8,
                coalesce(sum(n) FILTER (WHERE event = $3), 0)::int8,
                coalesce(sum(n) FILTER (WHERE event = $4), 0)::int8
         FROM insight_daily WHERE project_id = $1 AND preferred = $2
           AND day >= (now() AT TIME ZONE 'utc')::date - $5::int AND day < (now() AT TIME ZONE 'utc')::date",
    )
    .bind(p.id)
    .bind(lang(&b.locale))
    .bind(EXPLICIT)
    .bind(&goal)
    .bind(days)
    .fetch_one(&state.db)
    .await?;
    let baseline = Counts {
        visits: (baseline.0 - baseline.1).max(0),
        conversions: baseline.2,
    };

    let mut tx = state.db.begin().await?;
    let id = Uuid::new_v4();
    let experiment_id = if holdout {
        let xid = Uuid::new_v4();
        sqlx::query(
            // The holdout lasts until the launch stops it (rollout), not until the measurement end date.
            "INSERT INTO insight_experiments (id, project_id, locale, traffic_pct, goal, ends_at, hold_locale)
             VALUES ($1, $2, $3, $4, $5, 'infinity', $6)",
        )
        .bind(xid)
        .bind(p.id)
        .bind(&b.locale)
        .bind(100 - holdout_pct)
        .bind(&goal)
        .bind(&fallback)
        .execute(&mut *tx)
        .await
        .map_err(|e| match AppError::from(e) {
            e if e.code == "conflict" => {
                AppError::conflict("an experiment is already running for this language")
            }
            e => e,
        })?;
        Some(xid)
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO language_launches (id, project_id, locale, mode, experiment_id, fallback, goal, holdout_pct,
           days, min_n, ends_at, baseline, cost_usd, cost_estimated, recommendation, min_conversions, guardrails,
           auto_rollout)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now() + make_interval(days => $9), $11, $12, $13, $14,
           $15, $16, $17)",
    )
    .bind(id)
    .bind(p.id)
    .bind(&b.locale)
    .bind(&mode)
    .bind(experiment_id)
    .bind(&fallback)
    .bind(&goal)
    .bind(holdout_pct)
    .bind(days)
    .bind(min_n)
    .bind(DbJson(baseline))
    .bind(cost)
    .bind(estimated)
    .bind(rec.map(DbJson))
    .bind(min_conversions)
    .bind(DbJson(&b.guardrails))
    .bind(b.auto_rollout && holdout)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.code == "conflict" => AppError::conflict("this language is already being measured"),
        e => e,
    })?;
    // The new locale must be servable: it joins the targets (as a language experiment does).
    sqlx::query(
        "UPDATE projects SET target_locales = array_append(target_locales, $2)
         WHERE id = $1 AND NOT ($2 = ANY(target_locales))",
    )
    .bind(p.id)
    .bind(&b.locale)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let r = load(&state, p.id, id).await?;
    Ok((StatusCode::CREATED, Json(view(&state, &r, &p.name).await?)))
}

pub async fn share(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    crate::billing::require_growth(&state, p.id, "Shared case studies").await?;
    // Idempotent: sharing twice keeps the same link.
    let nonce: String = sqlx::query_scalar(
        "UPDATE language_launches SET share_nonce = coalesce(share_nonce, $3)
         WHERE id = $1 AND project_id = $2 RETURNING share_nonce",
    )
    .bind(id)
    .bind(p.id)
    .bind(hex::encode(rand::random::<[u8; 16]>()))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("launch not found"))?;
    Ok(Json(
        serde_json::json!({ "token": token(&state.secret, id, &nonce) }),
    ))
}

/// The owner's confirmation: stop the holdout (everyone who prefers the language gets it). Allowed once the
/// measurement stopped (`ready`, or `done` / `invalid` without meeting the conditions: the owner's call).
pub async fn rollout(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Launch>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let r = load(&state, p.id, id).await?;
    if r.mode != "holdout" || !matches!(r.status.as_str(), "ready" | "done" | "invalid") {
        return Err(AppError::conflict(
            "only a holdout whose measurement has stopped can go to 100 %",
        ));
    }
    let mut tx = state.db.begin().await?;
    let n = sqlx::query(
        "UPDATE language_launches SET status = 'rolled_out' WHERE id = $1 AND status = $2",
    )
    .bind(r.id)
    .bind(&r.status)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::conflict("the launch changed meanwhile"));
    }
    stop_holdout(&mut tx, r.experiment_id).await?;
    tx.commit().await?;
    let r = load(&state, p.id, id).await?;
    Ok(Json(view(&state, &r, &p.name).await?))
}

/// Stopping the holdout = everyone who prefers the language gets it (no arm drawn any more).
async fn stop_holdout(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    experiment: Option<Uuid>,
) -> ApiResult<()> {
    if let Some(xid) = experiment {
        sqlx::query(
            "UPDATE insight_experiments SET status = 'kept', decided_at = now() WHERE id = $1 AND status = 'running'",
        )
        .bind(xid)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn revoke(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let n = sqlx::query(
        "UPDATE language_launches SET share_nonce = NULL WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(p.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("launch not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn public(
    State(state): State<AppState>,
    AppPath(tok): AppPath<String>,
) -> ApiResult<axum::response::Response> {
    // One error for every failure: no oracle on ids or revocation.
    let gone = || AppError::not_found("case study not found");
    let (id, sig) = tok.split_once('.').ok_or_else(gone)?;
    let id = Uuid::try_parse(id).map_err(|_| gone())?;
    let sig = hex::decode(sig).map_err(|_| gone())?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT project_id, share_nonce FROM language_launches WHERE id = $1 AND share_nonce IS NOT NULL",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (project_id, nonce) = row.ok_or_else(gone)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&state.secret).expect("hmac key");
    mac.update(b"launch-share:");
    mac.update(id.as_bytes());
    mac.update(nonce.as_bytes());
    if sig.len() != 16 || mac.verify_truncated_left(&sig).is_err() {
        return Err(gone());
    }
    let r = load(&state, project_id, id).await?;
    let name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_one(&state.db)
        .await?;
    let s = story(&state, &r, name).await?;
    // Revocation takes effect at once: nothing may cache a shared story.
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(s)).into_response())
}

// ---------- worker ----------

/// Every `GLOT_XP_TICK_SECS` (default 300): spend decision looks; once every rollout condition holds, roll out
/// (`auto_rollout`) or wait for a confirmation (`ready`); at the end date close as `done` (holdout kept until the
/// owner decides), invalidate on SRM, close before/after measurements.
pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    let secs = std::env::var("GLOT_XP_TICK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300u64)
        .max(1);
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(&state).await {
                tracing::error!("launches tick: {}", e.message);
            }
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        }
    })
}

pub async fn tick(state: &AppState) -> ApiResult<()> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM language_launches WHERE status = 'measuring'"
    ))
    .fetch_all(&state.db)
    .await?;
    for r in rows {
        let Checked {
            o, blockers, ended, ..
        } = outcome(state, &r).await?;
        let ready = blockers.is_empty();
        let status = match o.verdict {
            // ponytail: SRM acts at decision looks and at the end only (a χ² re-tested every tick false-alarms).
            "invalid" if o.decision_look || ended => "invalid",
            _ if ready && r.auto_rollout => "rolled_out",
            _ if ready => "ready",
            _ if ended => "done",
            _ => {
                if o.decision_look {
                    sqlx::query("UPDATE language_launches SET looks = looks + 1 WHERE id = $1 AND looks = $2")
                        .bind(r.id)
                        .bind(r.looks)
                        .execute(&state.db)
                        .await?;
                }
                continue;
            }
        };
        let verdict = o.verdict; // `ended` never yields "collecting"
        let spent = jobs_cost(state, r.project_id, &r.locale).await?;
        let mut tx = state.db.begin().await?;
        let done = sqlx::query(
            "UPDATE language_launches SET status = $2, verdict = $3, decided_at = least(now(), ends_at),
               cost_usd = CASE WHEN $4 > 0 THEN $4 ELSE cost_usd END,
               cost_estimated = cost_estimated AND $4 <= 0
             WHERE id = $1 AND status = 'measuring'",
        )
        .bind(r.id)
        .bind(status)
        .bind(verdict)
        .bind(spent)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if done == 1 && status == "rolled_out" {
            stop_holdout(&mut tx, r.experiment_id).await?;
        }
        tx.commit().await?;
        tracing::info!(launch = %r.id, status, verdict, "language launch measured");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(arms: [(i64, i64); 2], days: f64, ended: bool) -> Eval {
        Eval {
            holdout: true,
            arms: arms.map(|(visits, conversions)| Counts {
                visits,
                conversions,
            }),
            assigned: [arms[0].0, arms[1].0],
            holdout_pct: 20,
            min_n: 200,
            days,
            look: 0,
            ended,
            seed: 7,
        }
    }

    #[test]
    fn verdicts_on_fixtures() {
        // 20 % held out: control 2 000 at 3 %, new locale 8 000 at 4.5 % (+50 %).
        let win = evaluate(&e([(2000, 60), (8000, 360)], 14.0, false));
        assert_eq!(win.verdict, "probably_positive", "{win:?}");
        assert!(win.lift_lo > 0.0 && win.lift > 0.3 && win.lift < 0.7);
        assert_eq!(win.winner, Some(1));
        // Same rates, too early: the time guard holds the verdict.
        assert_eq!(
            evaluate(&e([(2000, 60), (8000, 360)], 3.0, false)).verdict,
            "collecting"
        );
        // Under the minimum sample.
        assert_eq!(
            evaluate(&e([(100, 3), (400, 18)], 14.0, false)).verdict,
            "collecting"
        );
        // No difference: never "positive", interval straddles 0.
        let flat = evaluate(&e([(2000, 60), (8000, 242)], 14.0, true));
        assert_eq!(flat.verdict, "inconclusive");
        assert!(flat.lift_lo < 0.0 && flat.lift_hi > 0.0);
        // Worse.
        assert_eq!(
            evaluate(&e([(2000, 80), (8000, 160)], 14.0, true)).verdict,
            "probably_negative"
        );
        // Deterministic for a launch (seeded): the story does not flicker between reads.
        assert_eq!(
            evaluate(&e([(2000, 60), (8000, 300)], 14.0, false)),
            evaluate(&e([(2000, 60), (8000, 300)], 14.0, false))
        );
    }

    #[test]
    fn srm_invalidates() {
        // 50/50 split where 20/80 was configured: the assignment is broken, whatever the rates say.
        let mut x = e([(5000, 150), (5000, 300)], 14.0, false);
        x.assigned = [5000, 5000];
        let o = evaluate(&x);
        assert_eq!(o.verdict, "invalid");
        assert_eq!(o.winner, None);
        assert!(o.srm_p.unwrap() < stats::SRM_P);
        // Opt-outs (explicit choices) are removed from the analysed visits but not from the SRM check.
        let mut y = e([(1900, 57), (7990, 359)], 14.0, false);
        y.assigned = [2000, 8000];
        assert!(evaluate(&y).srm_p.unwrap() > 0.01);
    }

    #[test]
    fn before_after_never_causal() {
        let mut x = e([(3000, 90), (3000, 150)], 14.0, true);
        x.holdout = false;
        let o = evaluate(&x);
        assert_eq!(o.verdict, "higher_after");
        assert_eq!(o.srm_p, None);
        assert_eq!(o.winner, None, "before/after never rolls anything out");
        x.arms[1].conversions = 92;
        assert_eq!(evaluate(&x).verdict, "no_clear_change");
    }

    #[test]
    fn guardrails_block_on_degradation_or_thin_data() {
        let buy = Guardrail {
            event: "purchase".into(),
            higher_is_worse: false,
        };
        let err = Guardrail {
            event: "error".into(),
            higher_is_worse: true,
        };
        // Purchases 4 % → 3 % (−25 %): degraded. 10 % → 10.5 %: ok. 10 events: insufficient.
        assert_eq!(
            guardrail(&buy, [2000, 8000], [80, 240], 30, 1).status,
            "degraded"
        );
        let ok = guardrail(&buy, [2000, 8000], [200, 840], 30, 1);
        assert_eq!(ok.status, "ok", "{ok:?}");
        assert!(ok.p_worse < GUARDRAIL_MAX_P);
        assert_eq!(
            guardrail(&buy, [2000, 8000], [2, 8], 30, 1).status,
            "insufficient"
        );
        // Errors 1 % → 2 %: a rise is the degradation.
        assert_eq!(
            guardrail(&err, [2000, 8000], [20, 160], 30, 1).status,
            "degraded"
        );
        assert_eq!(guardrail(&err, [2000, 8000], [40, 120], 30, 1).status, "ok");
    }

    #[test]
    fn rollout_needs_every_condition() {
        let arms = [(2000, 60), (8000, 360)];
        let counts = arms.map(|(visits, conversions)| Counts {
            visits,
            conversions,
        });
        let win = evaluate(&e(arms, 14.0, false));
        let ok = guardrail(
            &Guardrail {
                event: "purchase".into(),
                higher_is_worse: false,
            },
            [2000, 8000],
            [200, 840],
            30,
            1,
        );
        let b = |o: &Outcome, counts: &[Counts; 2], days: f64, g: &[GuardrailResult]| {
            blockers(true, o, counts, days, 200, 30, g)
        };
        assert!(b(&win, &counts, 14.0, std::slice::from_ref(&ok)).is_empty());
        // Too few real conversions in the control arm (min 30), whatever the lift.
        let few = [
            Counts {
                visits: 2000,
                conversions: 29,
            },
            counts[1],
        ];
        assert_eq!(b(&win, &few, 14.0, &[]), ["min_conversions"]);
        assert_eq!(b(&win, &counts, 6.0, &[]), ["min_days"]);
        let degraded = GuardrailResult {
            status: "degraded",
            ..ok.clone()
        };
        assert_eq!(b(&win, &counts, 14.0, &[degraded]), ["guardrail"]);
        let thin = GuardrailResult {
            status: "insufficient",
            ..ok
        };
        assert_eq!(b(&win, &counts, 14.0, &[thin]), ["guardrail"]);
        let flat = evaluate(&e([(2000, 60), (8000, 242)], 14.0, true));
        assert_eq!(b(&flat, &counts, 14.0, &[]), ["no_credible_lift"]);
        let mut srm = e(arms, 14.0, false);
        srm.assigned = [5000, 5000];
        assert!(b(&evaluate(&srm), &counts, 14.0, &[]).contains(&"srm"));
        assert_eq!(
            blockers(false, &win, &counts, 14.0, 200, 30, &[]),
            ["no_holdout"]
        );
    }

    #[test]
    fn share_token_is_bound_to_id_and_nonce() {
        let s = [1u8; 32];
        let id = Uuid::new_v4();
        let t = token(&s, id, "a");
        assert!(t.starts_with(&id.simple().to_string()));
        assert_ne!(t, token(&s, id, "b"), "new nonce = new link");
        assert_ne!(t, token(&s, Uuid::new_v4(), "a"));
        assert_ne!(t, token(&[2u8; 32], id, "a"));
    }
}
