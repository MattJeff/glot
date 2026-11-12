//! Autopilot: revenue per market, not translated strings. Two halves on top of what already exists.
//!
//! **Open a country.** `recommend.rs` already ranks missing languages by expected extra conversions (Beta
//! posteriors, 90 % interval). Autopilot turns that into money (`value`: declared by the customer, else the measured
//! average of the `value` sent with conversion events), shows the formula, and adds the one click
//! (`POST …/autopilot/open`): the locale joins the targets, a capped translation job is queued, and once the job is
//! done the worker starts the measurement (`launches.rs`: holdout, or before/after with its caveat). The prediction
//! shown at the click is frozen next to the measured result.
//!
//! **Copy that optimises itself.** `variants.rs` already serves validated variants as a bandit and declares a
//! winner with a spent error budget. Autopilot picks the texts that matter (`text_kind`: headlines, CTAs, pricing,
//! never legal copy), refuses variants that add a number, price, link or claim (`legal_safe`), withdraws an arm
//! credibly worse than the official translation (`harmful`), keeps the
//! winner (`auto`) or waits for a human (`approval`, the default), gives up after `max_days` when nothing differs,
//! and reports the lift with its credible interval (`lift_report`).
//!
//! **Privacy.** Reads the existing `insight_daily` and `variant_arms` counters. The only new collection is
//! `autopilot_revenue_daily`: per (day, event, currency), a count and a sum of the `value` already sent with
//! conversion events. No visitor, session, IP, user agent, route, language or country is attached to it.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use sqlx::types::Json as DbJson;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::{AuthUser, User, VerifiedUser},
    insights::{self, lang},
    projects::{self, ProjectRow, Role},
    recommend::{self, Interval, Rate, Recommendation},
    validate_locale,
    variants::{self, VariantExperiment, draw, next_look},
};

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/autopilot";
    Router::new()
        .route(p, get(overview))
        .route(&format!("{p}/settings"), put(put_settings))
        .route(&format!("{p}/open"), post(open))
        .route(&format!("{p}/optimizations"), post(optimize))
        .route(&format!("{p}/optimizations/{{id}}/approve"), post(approve))
        .route(&format!("{p}/optimizations/{{id}}/stop"), post(stop))
}

/// Exactly what Autopilot reads and stores (returned by the API, shown in the dashboard and the docs).
pub const COLLECTED: [&str; 4] = [
    "insight_daily (existing): day, preferred language, served locale, country, experiment arm, event, count",
    "variant_arms (existing): experiment, arm, views, conversions",
    "autopilot_revenue_daily (new): day, event, currency, count, sum of the value sent with the event",
    "autopilot_posthog_daily (PostHog import, optional): day, browser language, served locale, country, sessions, \
     pageviews, one-page sessions, distinct visitors, sessions with the conversion event",
];

// ---------- money ----------

/// Valued conversions needed before a measured average is trusted.
pub const MIN_VALUED: i64 = 10;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Value {
    pub amount: f64,
    pub currency: String,
    /// declared (settings) | measured (average `value` of the goal event)
    pub basis: String,
    /// Valued conversions behind a measured average (0 when declared).
    pub events: i64,
}

/// What one goal conversion is worth: the declared value wins; else the average over the dominant currency of
/// the valued conversions, when there are at least `MIN_VALUED`; else nothing (conversions only, no money).
pub fn value_basis(
    declared: Option<f64>,
    currency: &str,
    measured: &[(String, i64, f64)],
) -> Option<Value> {
    if let Some(amount) = declared.filter(|v| v.is_finite() && *v > 0.0) {
        return Some(Value {
            amount,
            currency: currency.into(),
            basis: "declared".into(),
            events: 0,
        });
    }
    let (cur, n, sum) = measured.iter().max_by_key(|m| m.1)?;
    (*n >= MIN_VALUED && *sum > 0.0).then(|| Value {
        amount: sum / *n as f64,
        currency: cur.clone(),
        basis: "measured".into(),
        events: *n,
    })
}

/// Called by `insights::ingest` for every conversion event carrying a `value`: one counter and one sum per
/// (day, event, currency). Out-of-range values and malformed currencies are ignored, never an error for the page.
pub(crate) async fn on_revenue(
    state: &AppState,
    project_id: Uuid,
    event: &str,
    value: Option<f64>,
    currency: Option<&str>,
) -> ApiResult<()> {
    let Some(v) = value.filter(|v| v.is_finite() && (0.0..=1e6).contains(v) && *v > 0.0) else {
        return Ok(());
    };
    let cur = currency.unwrap_or("USD").to_ascii_uppercase();
    if event.starts_with('$') || cur.len() != 3 || !cur.bytes().all(|b| b.is_ascii_uppercase()) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO autopilot_revenue_daily (project_id, day, event, currency, n, value)
         VALUES ($1, (now() AT TIME ZONE 'utc')::date, $2, $3, 1, $4)
         ON CONFLICT (project_id, day, event, currency) DO UPDATE
           SET n = autopilot_revenue_daily.n + 1, value = autopilot_revenue_daily.value + $4",
    )
    .bind(project_id)
    .bind(event)
    .bind(cur)
    .bind(v)
    .execute(&state.db)
    .await?;
    Ok(())
}

// ---------- opportunities (pure) ----------

#[derive(Serialize, Debug, Clone)]
pub struct Opportunity {
    pub rank: usize,
    /// Demand, current and native conversion rates, expected extra conversions (90 % interval), cost, confidence
    /// and reasons, straight from the recommender.
    #[serde(flatten)]
    pub reco: Recommendation,
    /// How much worse these visitors convert than visitors served their own language: 1 − fallback / native.
    pub conversion_gap: Option<f64>,
    /// `gain_monthly` × value per conversion, same 90 % interval (the value itself is taken as exact).
    pub revenue_monthly: Option<Interval>,
    /// Translation cost in the value's currency (static reference rates), when it converts.
    pub cost: Option<f64>,
    /// Days for the expected uplift to pay the translation back, when the uplift is positive.
    pub payback_days: Option<f64>,
    /// Visits still missing before the estimate means anything (0 = enough): from these visitors, and from
    /// visitors served their own language (the comparison group).
    pub visits_missing: i64,
    pub native_visits_missing: i64,
    /// Goal conversions still missing, in the thinner of the two groups, before the estimate is solid.
    pub conversions_missing: i64,
    /// Share of these visitors' sessions with a single pageview (PostHog source only, ≥ 30 sessions).
    pub bounce_rate: Option<f64>,
}

/// Ranks the recommender's output for a decision: languages with enough data first, then by expected monthly
/// gain (conversions and money rank alike: the value is one constant), then demand, then the cheaper one.
pub fn rank(recos: Vec<Recommendation>, value: Option<&Value>) -> Vec<Opportunity> {
    let mut out: Vec<Opportunity> = recos
        .into_iter()
        .map(|reco| {
            // Too little data, or a baseline without a single conversion: no gap, no money, no payback.
            let thin = reco.confidence == "insufficient" || reco.native.conversions == 0;
            let gain = reco.gain_monthly.filter(|_| !thin);
            let revenue_monthly = gain.zip(value).map(|(g, v)| Interval {
                mid: g.mid * v.amount,
                low: g.low * v.amount,
                high: g.high * v.amount,
            });
            let cost = value.and_then(|v| {
                crate::experiments::ingest::convert(reco.cost_usd, "USD", &v.currency)
            });
            Opportunity {
                rank: 0,
                visits_missing: (recommend::MIN_VISITS - reco.visits).max(0),
                bounce_rate: None,
                conversions_missing: (recommend::SOLID_CONVERSIONS
                    - reco.native.conversions.min(reco.fallback.conversions))
                .max(0),
                native_visits_missing: (recommend::MIN_NATIVE - reco.native.visits).max(0),
                conversion_gap: (gain.is_some() && reco.native.rate > 0.0)
                    .then(|| 1.0 - reco.fallback.rate / reco.native.rate),
                payback_days: revenue_monthly
                    .zip(cost)
                    .filter(|(r, _)| r.mid > 0.0)
                    .map(|(r, c)| c / (r.mid / 30.0)),
                revenue_monthly,
                cost,
                reco,
            }
        })
        .collect();
    let gain = |o: &Opportunity| o.reco.gain_monthly.map_or(0.0, |g| g.mid.max(0.0));
    out.sort_by(|a, b| {
        (a.reco.confidence == "insufficient")
            .cmp(&(b.reco.confidence == "insufficient"))
            .then(gain(b).total_cmp(&gain(a)))
            .then(b.reco.visits.cmp(&a.reco.visits))
            .then(a.reco.cost_usd.total_cmp(&b.reco.cost_usd))
    });
    for (i, o) in out.iter_mut().enumerate() {
        o.rank = i + 1;
    }
    out
}

// ---------- misrouted visitors (pure) ----------

/// A language the project already ships, whose visitors are nevertheless served another locale: the cheapest
/// market to "open" (no translation, a detection or switcher fix).
#[derive(Serialize, Debug, Clone)]
pub struct Misrouted {
    pub language: String,
    /// Visits of the visitors who prefer this language, and how many of them were served another locale.
    pub visits: i64,
    pub elsewhere: i64,
    pub share: f64,
    /// Locale most of them get instead.
    pub served_instead: Option<String>,
    /// Goal conversion when served their language vs another one (posterior means, 90 % intervals).
    pub matched: Rate,
    pub mismatched: Rate,
    /// Extra conversions per 30 days if the misrouted visitors converted like the matched ones; `None` when the
    /// goal is not tracked (or the plan withholds gains).
    pub gain_monthly: Option<Interval>,
    pub revenue_monthly: Option<Interval>,
    /// insufficient | indicative | solid
    pub confidence: &'static str,
    /// Goal conversions still missing in the thinner group before the two rates can be compared.
    pub conversions_missing: i64,
}

/// `rows`: (preferred language, served locale, is a goal conversion, count) over `days`.
// Not causal: some of these visitors picked the other language themselves (a switcher click is not recorded
// outside language experiments), and the two groups may differ in more than the language they saw.
pub fn misrouted(
    rows: &[(String, String, bool, i64)],
    offered: &[String],
    days: i64,
    value: Option<&Value>,
) -> Vec<Misrouted> {
    let tracked = rows.iter().any(|r| r.2 && r.3 > 0);
    let mut langs: Vec<String> = offered.iter().map(|l| lang(l)).collect();
    langs.sort();
    langs.dedup();
    let mut out: Vec<Misrouted> = langs
        .into_iter()
        .filter_map(|language| {
            // [matched, elsewhere] × (visits, conversions)
            let mut g = [(0i64, 0i64); 2];
            let mut instead = std::collections::BTreeMap::<&str, i64>::new();
            // Served locale unknown (PostHog page without a locale code): neither matched nor misrouted.
            for (_, served, conv, n) in rows.iter().filter(|r| r.0 == language && !r.1.is_empty()) {
                let i = (lang(served) != language) as usize;
                if *conv {
                    g[i].1 += n;
                } else {
                    g[i].0 += n;
                    if i == 1 {
                        *instead.entry(served).or_default() += n;
                    }
                }
            }
            let (visits, elsewhere) = (g[0].0 + g[1].0, g[1].0);
            let share = elsewhere as f64 / visits.max(1) as f64;
            if elsewhere < recommend::MIN_VISITS || share < 0.05 {
                return None;
            }
            let (matched, mismatched) = (Rate::new(g[0].0, g[0].1), Rate::new(g[1].0, g[1].1));
            let thin = g[0].1.min(g[1].1);
            // Fewer than 10 conversions in a group: its rate says nothing, so no gain is stated at all.
            let confidence = if !tracked
                || g[0].0 < recommend::MIN_NATIVE
                || thin < recommend::SOLID_CONVERSIONS
            {
                "insufficient"
            } else if g[0].0.min(g[1].0) >= recommend::SOLID_VISITS {
                "solid"
            } else {
                "indicative"
            };
            let gain = (confidence != "insufficient").then(|| {
                recommend::gain(
                    elsewhere as f64 * 30.0 / days.max(1) as f64,
                    matched,
                    mismatched,
                )
            });
            Some(Misrouted {
                confidence,
                conversions_missing: (recommend::SOLID_CONVERSIONS - thin).max(0),
                revenue_monthly: gain.zip(value).map(|(g, v)| Interval {
                    mid: g.mid * v.amount,
                    low: g.low * v.amount,
                    high: g.high * v.amount,
                }),
                gain_monthly: gain,
                served_instead: instead.iter().max_by_key(|x| *x.1).map(|x| x.0.to_string()),
                language,
                visits,
                elsewhere,
                share,
                matched,
                mismatched,
            })
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.elsewhere));
    out
}

// ---------- texts that matter (pure) ----------

const LEGAL: &[&str] = &[
    "legal",
    "terms",
    "tos",
    "privacy",
    "cookie",
    "cookies",
    "consent",
    "gdpr",
    "disclaimer",
    "policy",
    "license",
    "licence",
    "copyright",
    "warranty",
    "imprint",
    "compliance",
    "refund",
];
const HEADLINE: &[&str] = &[
    "hero", "headline", "heading", "title", "h1", "tagline", "slogan",
];
const CTA: &[&str] = &[
    "cta",
    "button",
    "btn",
    "signup",
    "subscribe",
    "buy",
    "checkout",
    "start",
    "started",
    "try",
    "trial",
    "join",
    "order",
    "download",
    "register",
    "upgrade",
];
const PRICING: &[&str] = &["pricing", "price", "prices", "plan", "plans", "offer"];
const TOP: &[&str] = &["hero", "landing", "home", "homepage", "header"];
/// Navigation, account and back-office copy: buttons, but not what makes a visitor buy.
const NOISE: &[&str] = &[
    "admin",
    "backoffice",
    "logout",
    "signout",
    "login",
    "signin",
    "connecter",
    "connexion",
    "quit",
    "quitter",
    "cancel",
    "annuler",
    "close",
    "fermer",
    "back",
    "retour",
    "delete",
    "supprimer",
    "settings",
    "debug",
    "internal",
    "error",
    "errors",
    "menu",
    "copy",
    "clipboard",
    "dashboard",
    "footer",
];

/// `pricing.heroTitle` → [pricing, hero, title].
fn tokens(key: &str) -> Vec<String> {
    let (mut out, mut cur, mut prev_lower) = (vec![], String::new(), false);
    for c in key.chars() {
        if (!c.is_alphanumeric() || (c.is_uppercase() && prev_lower)) && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if c.is_alphanumeric() {
            cur.extend(c.to_lowercase());
        }
        prev_lower = c.is_lowercase();
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Is this a text worth optimising, and how much: `(kind, weight)` with kind headline | cta | pricing. `None` for
/// legal copy (never optimised), long prose and everything without a conversion role.
// ponytail: key name + extracted UI role only. Per-key exposure (which strings visitors actually saw) would rank
// better; add it when `glot:seen` is aggregated per key.
pub fn text_kind(key: &str, source: &str, ui_role: Option<&str>) -> Option<(&'static str, u32)> {
    let t = tokens(key);
    let has = |l: &[&str]| t.iter().any(|x| l.contains(&x.as_str()));
    if has(LEGAL)
        || has(NOISE)
        || source.contains('©')
        || !(2..=90).contains(&source.chars().count())
    {
        return None;
    }
    let cta = has(CTA) || ui_role == Some("button");
    let headline = has(HEADLINE) || ui_role == Some("heading");
    let pricing = has(PRICING);
    let weight = 3 * cta as u32 + 3 * headline as u32 + 2 * pricing as u32 + has(TOP) as u32;
    let kind = match (pricing, cta, headline) {
        (true, ..) => "pricing",
        (_, true, _) => "cta",
        (_, _, true) => "headline",
        _ => return None,
    };
    Some((kind, weight))
}

/// Words that make a promise. A variant may keep one the official translation already makes, never add one.
const CLAIMS: &[&str] = &[
    "free",
    "gratuit",
    "gratis",
    "kostenlos",
    "guarantee",
    "garanti",
    "garantie",
    "garantiert",
    "garantizado",
    "best",
    "meilleur",
    "mejor",
    "beste",
    "cheapest",
    "unlimited",
    "illimité",
    "ilimitado",
    "unbegrenzt",
    "forever",
    "certified",
    "official",
    "無料",
    "保証",
    "مجان",
];

/// Refuses a variant that says something the official translation does not: a new number, price, percentage,
/// link, e-mail address or promise word. Returns the machine code of the first reason.
pub fn legal_safe(control: &str, variant: &str) -> Result<(), &'static str> {
    let digits = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    };
    let base = digits(control);
    if digits(variant).iter().any(|d| !base.contains(d)) {
        return Err("new_number");
    }
    if "€$£¥%‰₹"
        .chars()
        .any(|c| variant.contains(c) && !control.contains(c))
    {
        return Err("new_price");
    }
    let (c, v) = (control.to_lowercase(), variant.to_lowercase());
    if ["http://", "https://", "www.", "@"]
        .iter()
        .any(|m| v.contains(m) && !c.contains(m))
    {
        return Err("new_link");
    }
    if CLAIMS.iter().any(|w| v.contains(w) && !c.contains(w)) {
        return Err("new_claim");
    }
    Ok(())
}

// ---------- guardrails (pure) ----------

/// Bar for withdrawing an arm: P(arm converts worse than the official translation), at each harm look.
/// A fixed bar, not a spent budget: wrongly withdrawing a wording costs one variant (visitors go back to the
/// official translation), while a spent budget would leave a harmful one live for longer. Looks are few (each
/// time traffic doubles), so an arm that is in truth no worse is withdrawn in a few percent of tests at most
/// (see the simulation test).
pub const HARM_PROB: f64 = 0.995;
const DRAWS: usize = 4000;

/// Views per arm before the first harm look: half the winner's minimum sample (harm must be caught sooner).
pub fn harm_min(min_n: i64) -> i64 {
    (min_n / 2).max(25)
}

/// Arms (index ≥ 1) credibly worse than the official translation (arm 0): P(arm < official) > `HARM_PROB`,
/// both seen at least `min_views` times.
pub fn harmful(arms: &[(i64, i64)], min_views: i64, rng: &mut impl Rng) -> Vec<usize> {
    if arms.len() < 2 || arms[0].0 < min_views {
        return vec![];
    }
    (1..arms.len())
        .filter(|&i| {
            arms[i].0 >= min_views && {
                let worse = (0..DRAWS)
                    .filter(|_| draw(arms[i].0, arms[i].1, rng) < draw(arms[0].0, arms[0].1, rng))
                    .count();
                worse as f64 / DRAWS as f64 > HARM_PROB
            }
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Report {
    pub arm: i32,
    pub value: String,
    /// [views, conversions] of the official translation and of this wording.
    pub control: (i64, i64),
    pub variant: (i64, i64),
    /// Relative lift over the official translation: posterior median and 95 % credible interval.
    pub lift: f64,
    pub lift_low: f64,
    pub lift_high: f64,
    pub p_better: f64,
    /// lift × the locale's goal conversions per 30 days (and × value): what the wording is worth if the lift holds.
    pub extra_conversions_monthly: Option<Interval>,
    pub revenue_monthly: Option<Interval>,
}

/// (median, 2.5 %, 97.5 %, P(variant > control)) of the relative lift, from the two Beta posteriors.
pub fn lift_report(control: (i64, i64), variant: (i64, i64), seed: u64) -> (f64, f64, f64, f64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = 20_000;
    let mut lifts: Vec<f64> = (0..n)
        .map(|_| {
            let c = draw(control.0, control.1, &mut rng).max(1e-9);
            draw(variant.0, variant.1, &mut rng) / c - 1.0
        })
        .collect();
    lifts.sort_by(f64::total_cmp);
    let better = lifts.iter().filter(|l| **l > 0.0).count() as f64 / n as f64;
    (lifts[n / 2], lifts[n / 40], lifts[n - n / 40], better)
}

// ---------- settings ----------

#[derive(Serialize, Deserialize, sqlx::FromRow, Debug, Clone)]
pub struct Settings {
    pub goal: String,
    pub value_per_conversion: Option<f64>,
    pub currency: String,
    /// approval (a human keeps the winner) | auto
    pub mode: String,
}

async fn settings(state: &AppState, project_id: Uuid) -> ApiResult<Settings> {
    Ok(sqlx::query_as(
        "SELECT goal, value_per_conversion, currency, mode FROM autopilot_settings WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(Settings {
        goal: "signup".into(),
        value_per_conversion: None,
        currency: "USD".into(),
        mode: "approval".into(),
    }))
}

fn check_mode(mode: &str) -> ApiResult<()> {
    matches!(mode, "approval" | "auto")
        .then_some(())
        .ok_or_else(|| AppError::bad_request("mode must be approval or auto"))
}

pub async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(mut s): AppJson<Settings>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    insights::check_event(&s.goal)?;
    check_mode(&s.mode)?;
    s.currency = s.currency.to_ascii_uppercase();
    if crate::experiments::ingest::convert(1.0, &s.currency, "USD").is_none() {
        return Err(AppError::bad_request(format!(
            "unknown currency '{}': use an ISO 4217 code such as USD or EUR",
            s.currency
        )));
    }
    if s.value_per_conversion
        .is_some_and(|v| !(v.is_finite() && v > 0.0 && v <= 1e9))
    {
        return Err(AppError::bad_request(
            "value_per_conversion must be a positive amount (or null to use measured revenue)",
        ));
    }
    sqlx::query(
        "INSERT INTO autopilot_settings (project_id, goal, value_per_conversion, currency, mode)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (project_id) DO UPDATE SET goal = $2, value_per_conversion = $3, currency = $4, mode = $5,
           updated_at = now()",
    )
    .bind(p.id)
    .bind(&s.goal)
    .bind(s.value_per_conversion)
    .bind(&s.currency)
    .bind(&s.mode)
    .execute(&state.db)
    .await?;
    Ok(Json(s))
}

async fn value(state: &AppState, project_id: Uuid, s: &Settings) -> ApiResult<Option<Value>> {
    let measured: Vec<(String, i64, f64)> = sqlx::query_as(
        "SELECT currency, sum(n)::int8, sum(value)::float8 FROM autopilot_revenue_daily
         WHERE project_id = $1 AND event = $2 AND day > (now() AT TIME ZONE 'utc')::date - 90 GROUP BY 1",
    )
    .bind(project_id)
    .bind(&s.goal)
    .fetch_all(&state.db)
    .await?;
    Ok(value_basis(s.value_per_conversion, &s.currency, &measured))
}

/// The ranked opportunities of a project (gain withheld below Growth, like the recommendations endpoint).
async fn opportunities(
    state: &AppState,
    p: &ProjectRow,
    src: &Source,
    s: &Settings,
) -> ApiResult<(
    recommend::Recommendations,
    Option<Value>,
    bool,
    Vec<Opportunity>,
)> {
    let mut r = recommend::for_table(state, p, src.days, &src.event, src.table).await?;
    let locked = !crate::billing::has_growth(state, p.id).await?;
    if locked {
        r.recommendations.iter_mut().for_each(recommend::lock_gain);
    }
    // Measured revenue comes from the glot tag's events: with PostHog counters, only a declared value applies.
    let v = if src.kind == "insights" {
        value(state, p.id, s).await?
    } else {
        value_basis(s.value_per_conversion, &s.currency, &[])
    };
    let ranked = rank(std::mem::take(&mut r.recommendations), v.as_ref());
    Ok((r, v, locked, ranked))
}

/// Days actually observed in the last `days`: from the first day with data to today. Scaling a young project's
/// counters over a window it was not collecting yet would understate its traffic (5 days of data ≠ 30).
async fn observed(state: &AppState, project_id: Uuid, days: i64, table: &str) -> ApiResult<i64> {
    let first: Option<chrono::NaiveDate> = sqlx::query_scalar(&format!(
        "SELECT min(day) FROM {table}
         WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int"
    ))
    .bind(project_id)
    .bind(days as i32)
    .fetch_one(&state.db)
    .await?;
    Ok(first.map_or(days, |d| {
        ((Utc::now().date_naive() - d).num_days() + 1).clamp(1, days)
    }))
}

/// Where the counters come from: the glot tag, or the history imported from PostHog (`autopilot_posthog.rs`).
#[derive(Serialize, Clone, Debug)]
pub struct Source {
    /// insights (the glot tag) | posthog
    pub kind: &'static str,
    /// The conversion event, as named at the source.
    pub goal: String,
    /// Days observed at this source in the window (first day with data → today).
    pub days: i64,
    /// Visits (sessions) per source over the window: the busier one is used unless `?source=` says otherwise.
    pub visits: Vec<(&'static str, i64)>,
    pub synced_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    table: &'static str,
    /// Event name as stored in `table`.
    #[serde(skip)]
    event: String,
}

async fn pick(
    state: &AppState,
    p: &ProjectRow,
    days: i64,
    s: &Settings,
    wanted: Option<&str>,
) -> ApiResult<Source> {
    let ph = crate::autopilot_posthog::status(state, p.id).await?;
    let mut visits = vec![];
    for (kind, table) in [
        ("insights", "insight_daily"),
        ("posthog", crate::autopilot_posthog::TABLE),
    ] {
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT coalesce(sum(n), 0)::int8 FROM {table}
             WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int AND event = '$view'"
        ))
        .bind(p.id)
        .bind(days as i32)
        .fetch_one(&state.db)
        .await?;
        visits.push((kind, n));
    }
    let posthog = match wanted {
        None => ph.connected && visits[1].1 > visits[0].1,
        Some("insights") => false,
        Some("posthog") if ph.connected => true,
        Some("posthog") => {
            return Err(AppError::bad_request(
                "PostHog is not connected: connect it in Autopilot → Data source",
            ));
        }
        Some(_) => return Err(AppError::bad_request("source must be insights or posthog")),
    };
    let (kind, table, event, goal) = if posthog {
        let goal = ph.conversion_event.clone();
        (
            "posthog",
            crate::autopilot_posthog::TABLE,
            // No conversion event chosen: an event name nothing is stored under, so visits only.
            if goal.is_some() {
                crate::autopilot_posthog::CONVERSION
            } else {
                "$none"
            }
            .to_string(),
            goal.unwrap_or_default(),
        )
    } else {
        ("insights", "insight_daily", s.goal.clone(), s.goal.clone())
    };
    Ok(Source {
        kind,
        goal,
        days: observed(state, p.id, days, table).await?,
        visits,
        synced_at: ph.last_sync_at.filter(|_| posthog),
        table,
        event,
    })
}

// ---------- open a market ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Opening {
    pub id: Uuid,
    pub locale: String,
    pub mode: String,
    pub days: i32,
    pub goal: String,
    /// translating | measuring | failed
    pub status: String,
    pub job_id: Option<Uuid>,
    pub launch_id: Option<Uuid>,
    /// The opportunity as predicted when the market was opened.
    pub estimate: Option<DbJson<JsonValue>>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Translation progress while `translating`.
    pub job_done: Option<i32>,
    pub job_total: Option<i32>,
    /// The measured story (`launches.rs`: lift, interval, verdict, `causal`), once `measuring`.
    #[sqlx(skip)]
    pub launch: Option<JsonValue>,
}

const OPENING_COLS: &str = "o.id, o.locale, o.mode, o.days, o.goal, o.status, o.job_id, o.launch_id, o.estimate,
    o.error, o.created_at, o.updated_at, j.progress_done AS job_done, j.progress_total AS job_total";

#[derive(Deserialize)]
pub struct Open {
    locale: String,
    /// holdout (default: measures the actual effect, needs runtime catalogs) | before_after
    mode: Option<String>,
    /// Measurement length, default 14.
    days: Option<i32>,
}

/// Job cap: the dry-run estimate + 50 % (it ignores glossary and context), at least 50 cents.
pub fn budget_cap(estimate_usd: f64) -> f64 {
    ((estimate_usd * 1.5 * 100.0).ceil() / 100.0).max(0.5)
}

/// One click: the locale joins the targets, a capped translation job is queued and the measurement is scheduled
/// (the worker starts it when the job is done). Every refusal says what to do instead.
pub async fn open(
    State(state): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Open>,
) -> ApiResult<(StatusCode, Json<Opening>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    validate_locale(&b.locale)?;
    if lang(&b.locale) == lang(&p.source_locale) {
        return Err(AppError::bad_request(
            "this is your source language: it is already served",
        ));
    }
    let mode = b.mode.unwrap_or_else(|| "holdout".into());
    let days = b.days.unwrap_or(14);
    if !matches!(mode.as_str(), "holdout" | "before_after") || !(7..=90).contains(&days) {
        return Err(AppError::bad_request(
            "mode must be holdout or before_after, days 7-90",
        ));
    }
    // Fail now, with the reason, rather than after the translation was paid for.
    crate::billing::require_growth(&state, p.id, "Language launches").await?;
    crate::billing::check_llm(&state, &p).await?;
    let live: Option<String> = sqlx::query_scalar(
        "SELECT status FROM language_launches WHERE project_id = $1 AND locale = $2 AND status = 'measuring'",
    )
    .bind(p.id)
    .bind(&b.locale)
    .fetch_optional(&state.db)
    .await?;
    if live.is_some() {
        return Err(AppError::conflict(
            "this language is already being measured: see Insights → Launches",
        ));
    }
    let s = settings(&state, p.id).await?;
    let src = pick(&state, &p, 30, &s, None).await?;
    let (_, _, _, ranked) = opportunities(&state, &p, &src, &s).await?;
    let estimate = ranked
        .iter()
        .find(|o| lang(&o.reco.locale) == lang(&b.locale));
    let cost = match estimate {
        Some(o) => o.reco.cost_usd,
        None => {
            let keys: Vec<(String, String)> =
                sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1 ORDER BY key")
                    .bind(p.id)
                    .fetch_all(&state.db)
                    .await?;
            insights::estimate_cost(&p, &keys, &b.locale)
        }
    };
    let estimate = estimate
        .map(serde_json::to_value)
        .transpose()
        .map_err(AppError::internal)?;

    let mut tx = state.db.begin().await?;
    let id = Uuid::new_v4();
    // Claimed first: the unique index refuses a second click while the first one is translating.
    sqlx::query(
        "INSERT INTO autopilot_openings (id, project_id, locale, mode, days, goal, estimate, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(p.id)
    .bind(&b.locale)
    .bind(&mode)
    .bind(days)
    .bind(&s.goal)
    .bind(estimate.map(DbJson))
    .bind(user.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.code == "conflict" => {
            AppError::conflict("this market is already being opened: its translation is running")
        }
        e => e,
    })?;
    sqlx::query(
        "UPDATE projects SET target_locales = array_append(target_locales, $2)
         WHERE id = $1 AND NOT ($2 = ANY(target_locales))",
    )
    .bind(p.id)
    .bind(&b.locale)
    .execute(&mut *tx)
    .await?;
    let job = Uuid::new_v4();
    let params = crate::jobs::JobParams {
        locales: Some(vec![b.locale.clone()]),
        budget_usd: Some(budget_cap(cost)),
        ..Default::default()
    };
    sqlx::query(
        "INSERT INTO jobs (id, project_id, kind, status, params, created_by) VALUES ($1, $2, 'translate', 'queued', $3, $4)",
    )
    .bind(job)
    .bind(p.id)
    .bind(DbJson(&params))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE autopilot_openings SET job_id = $2 WHERE id = $1")
        .bind(id)
        .bind(job)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    state.job_wakeup.notify_one();
    let o = opening(&state, id).await?;
    Ok((StatusCode::CREATED, Json(o)))
}

async fn opening(state: &AppState, id: Uuid) -> ApiResult<Opening> {
    Ok(sqlx::query_as(&format!(
        "SELECT {OPENING_COLS} FROM autopilot_openings o LEFT JOIN jobs j ON j.id = o.job_id WHERE o.id = $1"
    ))
    .bind(id)
    .fetch_one(&state.db)
    .await?)
}

async fn load_user(state: &AppState, id: Option<Uuid>) -> ApiResult<Option<User>> {
    Ok(
        sqlx::query_as("SELECT id, email, name FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?,
    )
}

/// Openings whose translation ended: start the measurement, or say why not.
async fn tick_openings(state: &AppState) -> ApiResult<()> {
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        String,
        i32,
        String,
        Option<Uuid>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT o.id, o.project_id, p.slug, o.locale, o.mode, o.days, o.goal, o.created_by, j.status, j.error
         FROM autopilot_openings o JOIN projects p ON p.id = o.project_id LEFT JOIN jobs j ON j.id = o.job_id
         WHERE o.status = 'translating'",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, project_id, slug, locale, mode, days, goal, by, job, job_error) in rows {
        const KEPT: &str = "The language stays in your targets (untranslated strings fall back to the source): \
                            fix the cause, then open the market again.";
        let failed = match job.as_deref() {
            Some("queued" | "running") => continue,
            Some("done") => {
                let (keys, done): (i64, i64) = sqlx::query_as(
                    "SELECT count(*), count(*) FILTER (WHERE EXISTS (SELECT 1 FROM translations t
                       WHERE t.key_id = k.id AND t.locale = $2 AND t.value <> '' AND t.status <> 'stale'))
                     FROM keys k WHERE k.project_id = $1",
                )
                .bind(project_id)
                .bind(&locale)
                .fetch_one(&state.db)
                .await?;
                if (done as f64) < recommend::UNDER_TRANSLATED * keys as f64 {
                    Some(format!(
                        "The translation job ended with {done} of {keys} strings translated: measuring a \
                         half-translated language would understate it. {KEPT}"
                    ))
                } else {
                    match start_launch(state, &slug, &locale, &mode, days, &goal, by).await {
                        Ok(launch) => {
                            sqlx::query(
                                "UPDATE autopilot_openings SET status = 'measuring', launch_id = $2,
                                   updated_at = now() WHERE id = $1",
                            )
                            .bind(id)
                            .bind(launch)
                            .execute(&state.db)
                            .await?;
                            None
                        }
                        Err(e) => Some(format!(
                            "The language is translated and live, but its measurement could not start: {}. \
                             Start it from Insights → Launches.",
                            e.message
                        )),
                    }
                }
            }
            Some(_) => Some(format!(
                "The translation job failed: {}. {KEPT}",
                job_error.as_deref().unwrap_or("no error recorded")
            )),
            None => Some(format!("The translation job was deleted. {KEPT}")),
        };
        if let Some(error) = failed {
            sqlx::query(
                "UPDATE autopilot_openings SET status = 'failed', error = $2, updated_at = now()
                 WHERE id = $1 AND status = 'translating'",
            )
            .bind(id)
            .bind(error)
            .execute(&state.db)
            .await?;
        }
    }
    Ok(())
}

/// Same path as the dashboard's "Measure a launch" (`launches::create`): validation, plan, baseline, holdout.
async fn start_launch(
    state: &AppState,
    slug: &str,
    locale: &str,
    mode: &str,
    days: i32,
    goal: &str,
    by: Option<Uuid>,
) -> ApiResult<Uuid> {
    let user = load_user(state, by).await?.ok_or_else(|| {
        AppError::bad_request("the member who opened this market no longer exists")
    })?;
    let body = serde_json::from_value(
        json!({ "locale": locale, "mode": mode, "days": days, "goal": goal }),
    )
    .map_err(AppError::internal)?;
    let (_, Json(launch)) = crate::launches::create(
        State(state.clone()),
        VerifiedUser(user),
        AppPath(slug.to_string()),
        AppJson(body),
    )
    .await?;
    Ok(launch.id)
}

// ---------- optimizations ----------

#[derive(Serialize, Debug, Clone)]
pub struct Candidate {
    pub key: String,
    pub locale: String,
    /// headline | cta | pricing
    pub kind: &'static str,
    pub weight: u32,
    pub source: String,
    pub current: String,
    /// Visits served this locale, scaled to 30 days.
    pub monthly_visits: f64,
    /// Goal conversions of visitors served this locale, scaled to 30 days.
    pub monthly_conversions: f64,
    /// Days before a first analysis can conclude: 3 wordings × 500 visitors and ≈ 30 conversions each, whichever
    /// takes longer. `None` without visits or without a single conversion in this locale.
    pub est_days: Option<f64>,
    /// Too little traffic to conclude within `max_days`: the test would end on "no difference".
    pub low_traffic: bool,
}

const DEFAULT_MIN_N: i32 = 500;
const DEFAULT_MAX_DAYS: i32 = 30;
/// Rule of thumb for the date estimate only (the stopping rule is `variants::decide`).
const MIN_CONVERSIONS: f64 = 30.0;

/// Per served locale over the window: (visits, goal conversions).
async fn locale_traffic(
    state: &AppState,
    project_id: Uuid,
    days: i64,
    goal: &str,
) -> ApiResult<std::collections::BTreeMap<String, (i64, i64)>> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT served, coalesce(sum(n) FILTER (WHERE event = '$view'), 0)::int8,
                coalesce(sum(n) FILTER (WHERE event = $3), 0)::int8
         FROM insight_daily WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int
         GROUP BY 1",
    )
    .bind(project_id)
    .bind(days as i32)
    .bind(goal)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(|(l, v, c)| (l, (v, c))).collect())
}

/// Translated headlines, CTAs and pricing copy not under test yet, the most seen locale × heaviest text first.
// ponytail: scans every translated short key of the project (one query, filtered in Rust); fine to ~100k rows,
// move `text_kind` into a stored column beyond.
async fn candidates(
    state: &AppState,
    p: &ProjectRow,
    days: i64,
    traffic: &std::collections::BTreeMap<String, (i64, i64)>,
) -> ApiResult<Vec<Candidate>> {
    let rows: Vec<(String, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT k.key, k.source, k.ui_role, t.locale, t.value FROM keys k
         JOIN translations t ON t.key_id = k.id AND t.value <> '' AND t.status <> 'stale'
         WHERE k.project_id = $1 AND t.locale = ANY($2) AND length(k.source) <= 90
           AND NOT EXISTS (SELECT 1 FROM variant_experiments e WHERE e.project_id = k.project_id
                 AND e.key = k.key AND e.locale = t.locale AND e.status IN ('running', 'decided'))
           AND NOT EXISTS (SELECT 1 FROM xp_keys x WHERE x.project_id = k.project_id AND x.key = k.key AND x.live)",
    )
    .bind(p.id)
    .bind(&p.target_locales)
    .fetch_all(&state.db)
    .await?;
    let mut out: Vec<Candidate> = rows
        .into_iter()
        .filter(|r| !crate::connectors::is_key(&r.0))
        .filter_map(|(key, source, role, locale, current)| {
            let (kind, weight) = text_kind(&key, &source, role.as_deref())?;
            let (visits, conversions) = traffic.get(&locale).copied().unwrap_or_default();
            let per_day = |n: i64| n as f64 / days as f64;
            // A test needs visitors (3 wordings × 500) *and* conversions (≈ 30 per wording) to conclude: the
            // slower of the two sets the date. No conversion yet in this locale = it cannot conclude at all.
            let est_days = (visits > 0 && conversions > 0).then(|| {
                (3.0 * DEFAULT_MIN_N as f64 / per_day(visits))
                    .max(3.0 * MIN_CONVERSIONS / per_day(conversions))
            });
            let monthly = per_day(visits) * 30.0;
            Some(Candidate {
                low_traffic: est_days.is_none_or(|d| d > DEFAULT_MAX_DAYS as f64),
                key,
                locale,
                kind,
                weight,
                source,
                current,
                monthly_visits: monthly,
                monthly_conversions: per_day(conversions) * 30.0,
                est_days,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (b.weight as f64 * b.monthly_visits)
            .total_cmp(&(a.weight as f64 * a.monthly_visits))
            .then(b.weight.cmp(&a.weight))
            .then(a.key.cmp(&b.key))
            .then(a.locale.cmp(&b.locale))
    });
    out.truncate(20);
    Ok(out)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Optimization {
    #[sqlx(rename = "experiment_id")]
    pub id: Uuid,
    pub key: String,
    pub locale: String,
    pub goal: String,
    pub kind: String,
    pub mode: String,
    /// running | awaiting_approval | promoted | rolled_back | no_winner | stopped
    pub status: String,
    /// official_best | no_difference | all_variants_worse | source_changed | stopped_manually |
    /// promoted_manually | approver_missing
    pub reason: Option<String>,
    pub min_n: i32,
    pub max_days: i32,
    /// Arms withdrawn (credibly worse) or refused before serving (legal-safe check).
    pub dropped: DbJson<JsonValue>,
    /// Frozen when it ended; live numbers while running.
    pub result: Option<DbJson<Report>>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    /// The live arms (`variants.rs`), when the experiment is among the project's 50 latest.
    #[sqlx(skip)]
    pub experiment: Option<VariantExperiment>,
}

const OPT_COLS: &str =
    "o.experiment_id, e.key, e.locale, e.goal, o.kind, o.mode, o.status, o.reason, e.min_n,
    o.max_days, o.dropped, o.result, o.created_at, o.decided_at";

/// The report of the wording to talk about: the winner when there is one, else the best challenger so far.
fn report(
    id: Uuid,
    arms: &[(i32, String, i64, i64)],
    winner: Option<i32>,
    monthly_conversions: Option<f64>,
    value: Option<&Value>,
) -> Option<Report> {
    let control = arms.iter().find(|a| a.0 == 0)?;
    let rate = |a: &(i32, String, i64, i64)| (1.0 + a.3 as f64) / (2.0 + a.2 as f64);
    let pick = winner
        .filter(|w| *w != 0)
        .and_then(|w| arms.iter().find(|a| a.0 == w))
        .or_else(|| {
            arms.iter()
                .filter(|a| a.0 != 0)
                .max_by(|a, b| rate(a).total_cmp(&rate(b)))
        })?;
    let (c, v) = ((control.2, control.3), (pick.2, pick.3));
    let (lift, lo, hi, p_better) = lift_report(c, v, id.as_u128() as u64);
    let extra = monthly_conversions.map(|m| Interval {
        mid: m * lift,
        low: m * lo,
        high: m * hi,
    });
    Some(Report {
        arm: pick.0,
        value: pick.1.clone(),
        control: c,
        variant: v,
        lift,
        lift_low: lo,
        lift_high: hi,
        p_better,
        revenue_monthly: extra.zip(value).map(|(e, v)| Interval {
            mid: e.mid * v.amount,
            low: e.low * v.amount,
            high: e.high * v.amount,
        }),
        extra_conversions_monthly: extra,
    })
}

async fn arms(state: &AppState, id: Uuid) -> ApiResult<Vec<(i32, String, i64, i64)>> {
    Ok(sqlx::query_as(
        "SELECT arm, value, views, least(conversions, views) FROM variant_arms WHERE experiment_id = $1 ORDER BY arm",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?)
}

/// Goal conversions of a served locale per 30 days, and the project's value per conversion.
async fn worth(
    state: &AppState,
    project_id: Uuid,
    locale: &str,
    goal: &str,
) -> ApiResult<(Option<f64>, Option<Value>)> {
    let window = observed(state, project_id, 30, "insight_daily").await?;
    let t = locale_traffic(state, project_id, window, goal).await?;
    let s = settings(state, project_id).await?;
    let monthly = t
        .get(locale)
        .map(|x| x.1 as f64 * 30.0 / window as f64)
        .filter(|c| *c > 0.0);
    // The value is the value of the *settings'* goal: an experiment on another goal reports conversions only.
    let v = if s.goal == goal {
        value(state, project_id, &s).await?
    } else {
        None
    };
    Ok((monthly, v))
}

#[derive(Deserialize)]
pub struct NewOptimization {
    key: String,
    locale: String,
    /// approval | auto (default: the project's Autopilot setting).
    mode: Option<String>,
    /// Visitors per wording before a winner may be declared (default 500).
    min_n: Option<i32>,
    /// Give up ("no difference") after this many days (default 30).
    max_days: Option<i32>,
}

#[derive(Serialize)]
pub struct Started {
    #[serde(flatten)]
    pub optimization: Optimization,
    /// Wordings refused before serving (validation or legal-safe check), with the reason.
    pub refused: Vec<JsonValue>,
}

/// Starts a self-optimising copy test on one key and locale: generates validated variants (`variants::create`),
/// drops those that fail the legal-safe check, and hands the experiment to the Autopilot worker.
pub async fn optimize(
    State(state): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<NewOptimization>,
) -> ApiResult<(StatusCode, Json<Started>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let s = settings(&state, p.id).await?;
    let mode = b.mode.unwrap_or_else(|| s.mode.clone());
    check_mode(&mode)?;
    let max_days = b.max_days.unwrap_or(DEFAULT_MAX_DAYS);
    if !(1..=180).contains(&max_days) {
        return Err(AppError::bad_request("max_days must be 1-180"));
    }
    let mut conn = state.db.acquire().await?;
    let k = crate::keys::find_key(&mut conn, p.id, &b.key).await?;
    drop(conn);
    let kind = match text_kind(&k.key, &k.source, k.ui_role.as_deref()) {
        Some((kind, _)) => kind,
        None if tokens(&k.key).iter().any(|t| LEGAL.contains(&t.as_str())) => {
            return Err(AppError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "legal_text",
                "this looks like legal copy (terms, privacy, consent…): Autopilot never rewrites it",
            ));
        }
        None => "other",
    };
    let body = serde_json::from_value(json!({
        "key": b.key, "locale": b.locale, "variants": 2,
        "min_n": b.min_n.unwrap_or(DEFAULT_MIN_N), "goal": s.goal,
    }))
    .map_err(AppError::internal)?;
    let (_, Json(created)) = variants::create(
        State(state.clone()),
        VerifiedUser(user.clone()),
        AppPath(slug),
        AppJson(body),
    )
    .await?;
    let x = created.experiment;
    let control = x
        .arms
        .iter()
        .find(|a| a.is_control)
        .map(|a| a.value.clone());
    let control = control.ok_or_else(|| AppError::internal("experiment without a control arm"))?;
    let mut refused: Vec<JsonValue> = created
        .rejected
        .iter()
        .map(|r| json!({ "value": r.text, "why": "validation" }))
        .collect();
    let mut kept = 0;
    for a in x.arms.iter().filter(|a| !a.is_control) {
        match legal_safe(&control, &a.value) {
            Ok(()) => kept += 1,
            Err(why) => {
                sqlx::query("DELETE FROM variant_arms WHERE experiment_id = $1 AND arm = $2")
                    .bind(x.id)
                    .bind(a.arm)
                    .execute(&state.db)
                    .await?;
                refused.push(json!({ "arm": a.arm, "value": a.value, "why": why }));
            }
        }
    }
    if kept == 0 {
        // Nothing safe to test: no experiment is left behind.
        sqlx::query("DELETE FROM variant_experiments WHERE id = $1")
            .bind(x.id)
            .execute(&state.db)
            .await?;
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_safe_variant",
            format!(
                "every generated wording added a number, price, link or claim the official translation does \
                 not make ({} refused): nothing is served. Try again, or edit the translation by hand.",
                refused.len()
            ),
        ));
    }
    sqlx::query(
        "INSERT INTO autopilot_optimizations (experiment_id, project_id, kind, mode, max_days, dropped, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(x.id)
    .bind(p.id)
    .bind(kind)
    .bind(&mode)
    .bind(max_days)
    .bind(DbJson(&refused))
    .bind(user.id)
    .execute(&state.db)
    .await?;
    let optimization = optimizations(&state, &user, &p, Some(x.id))
        .await?
        .pop()
        .ok_or_else(|| AppError::internal("optimization vanished"))?;
    Ok((
        StatusCode::CREATED,
        Json(Started {
            optimization,
            refused,
        }),
    ))
}

async fn optimizations(
    state: &AppState,
    user: &User,
    p: &ProjectRow,
    id: Option<Uuid>,
) -> ApiResult<Vec<Optimization>> {
    let mut rows: Vec<Optimization> = sqlx::query_as(&format!(
        "SELECT {OPT_COLS} FROM autopilot_optimizations o JOIN variant_experiments e ON e.id = o.experiment_id
         WHERE o.project_id = $1 AND ($2::uuid IS NULL OR o.experiment_id = $2)
         ORDER BY (o.status IN ('running', 'awaiting_approval')) DESC, o.created_at DESC LIMIT 50"
    ))
    .bind(p.id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let Json(mut live) = variants::list(
        State(state.clone()),
        AuthUser(user.clone()),
        AppPath(p.slug.clone()),
    )
    .await?;
    for o in &mut rows {
        o.experiment = live
            .iter()
            .position(|x| x.id == o.id)
            .map(|i| live.swap_remove(i));
        if o.result.is_none() {
            let (monthly, v) = worth(state, p.id, &o.locale, &o.goal).await?;
            let a = arms(state, o.id).await?;
            o.result = report(o.id, &a, None, monthly, v.as_ref()).map(DbJson);
        }
    }
    Ok(rows)
}

async fn finish(
    state: &AppState,
    id: Uuid,
    status: &str,
    reason: Option<&str>,
    result: Option<&Report>,
) -> ApiResult<()> {
    sqlx::query(
        "UPDATE autopilot_optimizations SET status = $2, reason = $3, result = coalesce($4, result),
           decided_at = coalesce(decided_at, now()) WHERE experiment_id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(reason)
    .bind(result.map(DbJson))
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Stops serving the variants: every visitor is back on the official translation.
async fn stop_experiment(state: &AppState, id: Uuid) -> ApiResult<()> {
    sqlx::query(
        "UPDATE variant_experiments SET status = 'stopped', decided_at = coalesce(decided_at, now())
         WHERE id = $1 AND status IN ('running', 'decided')",
    )
    .bind(id)
    .execute(&state.db)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Live {
    experiment_id: Uuid,
    project_id: Uuid,
    slug: String,
    mode: String,
    harm_looks: i32,
    created_by: Option<Uuid>,
    aged: bool,
    xp_status: String,
    winner: Option<i32>,
    min_n: i32,
    locale: String,
    goal: String,
    source_changed: bool,
}

/// One pass over the running optimisations: sync with the experiment, withdraw harmful arms, keep or propose the
/// winner, give up when too old.
async fn tick_optimizations(state: &AppState) -> ApiResult<()> {
    let rows: Vec<Live> = sqlx::query_as(
        "SELECT o.experiment_id, o.project_id, p.slug, o.mode, o.harm_looks, o.created_by,
                now() > o.created_at + make_interval(days => o.max_days) AS aged,
                e.status AS xp_status, e.winner, e.min_n, e.locale, e.goal,
                coalesce(k.source <> e.source, true) AS source_changed
         FROM autopilot_optimizations o
         JOIN variant_experiments e ON e.id = o.experiment_id
         JOIN projects p ON p.id = o.project_id
         LEFT JOIN keys k ON k.project_id = e.project_id AND k.key = e.key
         WHERE o.status = 'running'",
    )
    .fetch_all(&state.db)
    .await?;
    for r in rows {
        if let Err(e) = step(state, &r).await {
            tracing::error!("autopilot optimization {}: {}", r.experiment_id, e.message);
        }
    }
    Ok(())
}

async fn step(state: &AppState, r: &Live) -> ApiResult<()> {
    let id = r.experiment_id;
    let (monthly, v) = worth(state, r.project_id, &r.locale, &r.goal).await?;
    let mut a = arms(state, id).await?;
    let frozen = |a: &[(i32, String, i64, i64)], w| report(id, a, w, monthly, v.as_ref());
    match r.xp_status.as_str() {
        // Someone acted on the Experiments page: follow.
        "stopped" => {
            return finish(
                state,
                id,
                "stopped",
                Some("stopped_manually"),
                frozen(&a, None).as_ref(),
            )
            .await;
        }
        "promoted" => {
            let rep = frozen(&a, r.winner);
            return finish(
                state,
                id,
                "promoted",
                Some("promoted_manually"),
                rep.as_ref(),
            )
            .await;
        }
        _ => {}
    }
    if r.source_changed {
        stop_experiment(state, id).await?;
        return finish(
            state,
            id,
            "stopped",
            Some("source_changed"),
            frozen(&a, None).as_ref(),
        )
        .await;
    }
    if r.xp_status == "decided" {
        let rep = frozen(&a, r.winner);
        if r.winner.is_none_or(|w| w == 0) {
            // The official translation won: nothing to write, the variants stop being served.
            stop_experiment(state, id).await?;
            return finish(state, id, "no_winner", Some("official_best"), rep.as_ref()).await;
        }
        if r.mode == "auto" {
            let Some(user) = load_user(state, r.created_by).await? else {
                return finish(
                    state,
                    id,
                    "awaiting_approval",
                    Some("approver_missing"),
                    rep.as_ref(),
                )
                .await;
            };
            return match variants::promote(
                State(state.clone()),
                AuthUser(user),
                AppPath((r.slug.clone(), id)),
            )
            .await
            {
                Ok(_) => finish(state, id, "promoted", None, rep.as_ref()).await,
                // Lost the admin role, validation now fails…: a human decides, with the reason in the logs.
                Err(e) => {
                    tracing::warn!("autopilot promote {id}: {}", e.message);
                    finish(
                        state,
                        id,
                        "awaiting_approval",
                        Some("approver_missing"),
                        rep.as_ref(),
                    )
                    .await
                }
            };
        }
        return finish(state, id, "awaiting_approval", None, rep.as_ref()).await;
    }
    // Still running: harm look when traffic reached the next checkpoint (claimed atomically, one per tick).
    let min = harm_min(r.min_n as i64);
    let total: i64 = a.iter().map(|x| x.2).sum();
    if total >= next_look(a.len(), min, r.harm_looks) {
        let claimed = sqlx::query(
            "UPDATE autopilot_optimizations SET harm_looks = harm_looks + 1
             WHERE experiment_id = $1 AND harm_looks = $2 AND status = 'running'",
        )
        .bind(id)
        .bind(r.harm_looks)
        .execute(&state.db)
        .await?
        .rows_affected()
            == 1;
        let counts: Vec<(i64, i64)> = a.iter().map(|x| (x.2, x.3)).collect();
        let bad = if claimed && a.first().is_some_and(|x| x.0 == 0) {
            harmful(&counts, min, &mut rand::rng())
        } else {
            vec![]
        };
        for &i in bad.iter().rev() {
            let (arm, value, views, conversions) = a.remove(i);
            // Archived, then gone from serving: `variants::on_hit` only draws stored arms.
            sqlx::query(
                "UPDATE autopilot_optimizations SET dropped = dropped || $2 WHERE experiment_id = $1",
            )
            .bind(id)
            .bind(DbJson(json!([{
                "arm": arm, "value": value, "views": views, "conversions": conversions, "why": "worse",
                "control": [counts[0].0, counts[0].1],
            }])))
            .execute(&state.db)
            .await?;
            sqlx::query("DELETE FROM variant_arms WHERE experiment_id = $1 AND arm = $2")
                .bind(id)
                .bind(arm)
                .execute(&state.db)
                .await?;
        }
        if !bad.is_empty() && a.len() < 2 {
            stop_experiment(state, id).await?;
            return finish(state, id, "rolled_back", Some("all_variants_worse"), None).await;
        }
    }
    if r.aged {
        stop_experiment(state, id).await?;
        return finish(
            state,
            id,
            "no_winner",
            Some("no_difference"),
            frozen(&a, None).as_ref(),
        )
        .await;
    }
    Ok(())
}

async fn owned(state: &AppState, p: &ProjectRow, id: Uuid) -> ApiResult<String> {
    sqlx::query_scalar(
        "SELECT status FROM autopilot_optimizations WHERE experiment_id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("optimization not found"))
}

/// Human approval: writes the winning wording as the official translation (`variants::promote`).
pub async fn approve(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Optimization>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if owned(&state, &p, id).await? != "awaiting_approval" {
        return Err(AppError::conflict(
            "nothing to approve: this optimisation has no winner waiting",
        ));
    }
    let _ = variants::promote(
        State(state.clone()),
        AuthUser(user.clone()),
        AppPath((slug, id)),
    )
    .await?;
    finish(&state, id, "promoted", None, None).await?;
    optimizations(&state, &user, &p, Some(id))
        .await?
        .pop()
        .map(Json)
        .ok_or_else(|| AppError::internal("optimization vanished"))
}

/// Stops the test (or declines the winner): the official translation stays.
pub async fn stop(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if !matches!(
        owned(&state, &p, id).await?.as_str(),
        "running" | "awaiting_approval"
    ) {
        return Err(AppError::conflict("this optimisation already ended"));
    }
    stop_experiment(&state, id).await?;
    finish(&state, id, "stopped", Some("stopped_manually"), None).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- dashboard ----------

#[derive(Serialize)]
pub struct Autopilot {
    pub days: i64,
    pub settings: Settings,
    /// Insights public key (empty state: the one tag to install).
    pub public_key: String,
    pub total_visits: i64,
    /// Days observed in the window (first day with data → today): every monthly figure is scaled over these.
    pub data_days: i64,
    /// At least one goal event in the window: without it, opportunities rank on demand only.
    pub conversions_tracked: bool,
    /// Below Growth (cloud billing): expected gains and money are withheld.
    pub gain_locked: bool,
    /// What one conversion is worth; `None` = estimates stay in conversions.
    pub value: Option<Value>,
    pub opportunities: Vec<Opportunity>,
    /// Languages already shipped whose visitors are served another locale.
    pub misrouted: Vec<Misrouted>,
    pub openings: Vec<Opening>,
    pub candidates: Vec<Candidate>,
    pub optimizations: Vec<Optimization>,
    pub collected: [&'static str; 4],
    /// Where every number above comes from.
    pub source: Source,
    /// Sessions with a single pageview, all visitors (PostHog source only).
    pub bounce_rate_all: Option<f64>,
    pub posthog: crate::autopilot_posthog::View,
}

#[derive(Deserialize)]
pub struct Query {
    pub days: Option<i64>,
    /// insights | posthog (default: the source with more visits in the window).
    pub source: Option<String>,
}

pub async fn overview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<Query>,
) -> ApiResult<Json<Autopilot>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let s = settings(&state, p.id).await?;
    // Every rate below is scaled over the days really observed at the source, not the requested window.
    let src = pick(&state, &p, days, &s, q.source.as_deref()).await?;
    let data_days = src.days;
    let (r, value, gain_locked, mut ranked) = opportunities(&state, &p, &src, &s).await?;
    let mut openings: Vec<Opening> = sqlx::query_as(&format!(
        "SELECT {OPENING_COLS} FROM autopilot_openings o LEFT JOIN jobs j ON j.id = o.job_id
         WHERE o.project_id = $1 ORDER BY o.created_at DESC LIMIT 20"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    if openings.iter().any(|o| o.launch_id.is_some()) {
        let Json(launches) = crate::launches::list(
            State(state.clone()),
            AuthUser(user.clone()),
            AppPath(slug.clone()),
        )
        .await?;
        for o in &mut openings {
            o.launch = launches
                .iter()
                .find(|l| Some(l.id) == o.launch_id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(AppError::internal)?;
        }
    }
    // A market being opened is no longer an opportunity: it is in `openings`.
    ranked.retain(|o| {
        !openings
            .iter()
            .any(|x| x.status == "translating" && lang(&x.locale) == o.reco.language)
    });
    for (i, o) in ranked.iter_mut().enumerate() {
        o.rank = i + 1;
    }
    // Copy tests are served by the glot tag: their candidates are timed on its counters, whatever the source.
    let tag_days = observed(&state, p.id, days, "insight_daily").await?;
    let traffic = locale_traffic(&state, p.id, tag_days, &s.goal).await?;
    let rows: Vec<(String, String, bool, i64)> = sqlx::query_as(&format!(
        "SELECT preferred, served, event <> '$view', sum(n)::int8 FROM {}
         WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int
           AND (event = '$view' OR event = $3) GROUP BY 1, 2, 3",
        src.table
    ))
    .bind(p.id)
    .bind(days as i32)
    .bind(&src.event)
    .fetch_all(&state.db)
    .await?;
    let offered: Vec<String> = std::iter::once(p.source_locale.clone())
        .chain(p.target_locales.iter().cloned())
        .collect();
    let mut misrouted = misrouted(&rows, &offered, data_days, value.as_ref());
    if gain_locked {
        for m in &mut misrouted {
            (m.gain_monthly, m.revenue_monthly) = (None, None);
        }
    }
    // Engagement proxy from PostHog: sessions that saw a single page.
    let mut bounce_rate_all = None;
    if src.kind == "posthog" {
        let b: Vec<(String, i64, i64)> = sqlx::query_as(&format!(
            "SELECT preferred, coalesce(sum(n) FILTER (WHERE event = '$view'), 0)::int8,
                    coalesce(sum(n) FILTER (WHERE event = '$bounce'), 0)::int8
             FROM {} WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int GROUP BY 1",
            src.table
        ))
        .bind(p.id)
        .bind(days as i32)
        .fetch_all(&state.db)
        .await?;
        let (all, bounced) = b.iter().fold((0, 0), |(a, c), x| (a + x.1, c + x.2));
        bounce_rate_all = (all > 0).then(|| bounced as f64 / all as f64);
        for o in &mut ranked {
            o.bounce_rate = b
                .iter()
                .find(|x| x.0 == o.reco.language && x.1 >= recommend::MIN_VISITS)
                .map(|x| x.2 as f64 / x.1 as f64);
        }
    }
    Ok(Json(Autopilot {
        // No visit yet: nothing was observed, whatever the window.
        data_days: if r.total_visits == 0 { 0 } else { data_days },
        misrouted,
        days,
        public_key: r.public_key,
        total_visits: r.total_visits,
        conversions_tracked: r.conversions_tracked,
        gain_locked,
        value,
        opportunities: ranked,
        openings,
        candidates: candidates(&state, &p, tag_days, &traffic).await?,
        optimizations: optimizations(&state, &user, &p, None).await?,
        settings: s,
        collected: COLLECTED,
        posthog: crate::autopilot_posthog::status(&state, p.id).await?,
        bounce_rate_all,
        source: src,
    }))
}

// ---------- worker ----------

/// Every `GLOT_XP_TICK_SECS` (default 300), like the launch worker.
pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    let secs = std::env::var("GLOT_XP_TICK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300u64)
        .max(1);
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(&state).await {
                tracing::error!("autopilot tick: {}", e.message);
            }
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        }
    })
}

pub async fn tick(state: &AppState) -> ApiResult<()> {
    tick_openings(state).await?;
    tick_optimizations(state).await?;
    crate::autopilot_posthog::tick(state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variants::{allocate, decide};

    #[test]
    fn value_declared_wins_then_measured_needs_enough_events() {
        let m = [("EUR".to_string(), 12, 600.0), ("USD".to_string(), 3, 90.0)];
        let v = value_basis(None, "USD", &m).unwrap();
        assert_eq!(
            (v.amount, v.currency.as_str(), v.basis.as_str(), v.events),
            (50.0, "EUR", "measured", 12)
        );
        let v = value_basis(Some(80.0), "GBP", &m).unwrap();
        assert_eq!(
            (v.amount, v.currency.as_str(), v.basis.as_str()),
            (80.0, "GBP", "declared")
        );
        assert_eq!(
            value_basis(None, "USD", &[("EUR".into(), 9, 450.0)]),
            None,
            "9 orders: not an average yet"
        );
        assert_eq!(value_basis(None, "USD", &[]), None);
    }

    fn reco(locale: &str, visits: i64, conv: i64, native: (i64, i64), cost: f64) -> Recommendation {
        let (n, f) = (Rate::new(native.0, native.1), Rate::new(visits, conv));
        Recommendation {
            locale: locale.into(),
            language: locale.into(),
            kind: "missing",
            visits,
            monthly_visits: visits as f64,
            share: 0.1,
            trend: None,
            served_now: Some("en".into()),
            served_now_share: 1.0,
            coverage: 0.0,
            fallback: f,
            native: n,
            gain_monthly: Some(recommend::gain(visits as f64, n, f)),
            cost_usd: cost,
            keys_to_translate: 100,
            cost_per_conversion: None,
            gain_locked: false,
            confidence: if visits < recommend::MIN_VISITS {
                "insufficient"
            } else {
                "solid"
            },
            reasons: vec![],
            summary: String::new(),
        }
    }

    #[test]
    fn ranking_and_money_math_against_known_truth() {
        // Truth: served their language, visitors convert at 5 %. German visitors (3000/month) convert at 2 % on
        // the English fallback, Japanese (400) at 4 %, and 20 Portuguese visits convert at 0 % (a guess, not data).
        let native = (4000, 200);
        let v = Value {
            amount: 40.0,
            currency: "USD".into(),
            basis: "declared".into(),
            events: 0,
        };
        let out = rank(
            vec![
                reco("pt", 20, 0, native, 1.0),
                reco("ja", 400, 16, native, 3.0),
                reco("de", 3000, 60, native, 2.0),
            ],
            Some(&v),
        );
        let order: Vec<&str> = out.iter().map(|o| o.reco.locale.as_str()).collect();
        assert_eq!(
            order,
            ["de", "ja", "pt"],
            "biggest proven gain first, thin data last"
        );
        assert_eq!(out[0].rank, 1);
        let de = &out[0];
        // True uplift: 3000 × (5 % − 2 %) = 90 conversions = 3600 $ per month.
        let (g, r) = (de.reco.gain_monthly.unwrap(), de.revenue_monthly.unwrap());
        assert!((g.mid - 90.0).abs() < 2.0, "{g:?}");
        assert!(
            g.low < 90.0 && 90.0 < g.high && g.low > 0.0,
            "90 % interval covers the truth: {g:?}"
        );
        assert!(
            (r.mid - g.mid * 40.0).abs() < 1e-9 && r.low < 3600.0 && 3600.0 < r.high,
            "{r:?}"
        );
        assert!(
            (de.conversion_gap.unwrap() - 0.6).abs() < 0.02,
            "2 % vs 5 % = 60 % worse"
        );
        // 2 $ of translation paid back by 3600 $/month: 2 / 120 per day.
        assert!((de.payback_days.unwrap() - 2.0 / (r.mid / 30.0)).abs() < 1e-9);
        // Japanese: 400 × 1 % = 4 conversions, the interval honestly includes zero.
        let ja = out[1].reco.gain_monthly.unwrap();
        assert!(ja.low < 0.0 && ja.high > 0.0, "{ja:?}");
        // No value known: conversions only, same order, no money invented.
        let plain = rank(
            vec![
                reco("ja", 400, 16, native, 3.0),
                reco("de", 3000, 60, native, 2.0),
            ],
            None,
        );
        assert_eq!(plain[0].reco.locale, "de");
        assert!(plain[0].revenue_monthly.is_none() && plain[0].payback_days.is_none());
    }

    #[test]
    fn misrouted_visitors_of_a_shipped_language() {
        let r = |p: &str, s: &str, c: bool, n: i64| (p.to_string(), s.to_string(), c, n);
        // English speakers: 1000 served English (50 signups), 600 served French (12 signups). Truth of the
        // fixture: 5 % matched vs 2 % misrouted, so 600 × 3 % = 18 conversions per 30 days.
        let rows = [
            r("en", "en", false, 1000),
            r("en", "en", true, 50),
            r("en", "fr", false, 600),
            r("en", "fr", true, 12),
            r("fr", "fr", false, 800),
            r("fr", "en", false, 10),
            r("de", "fr", false, 500),
        ];
        let offered = ["fr".to_string(), "en".to_string()];
        let out = misrouted(&rows, &offered, 30, None);
        assert_eq!(
            out.len(),
            1,
            "fr: 10 misrouted visits is noise; de is not shipped (an opportunity, not this)"
        );
        let m = &out[0];
        assert_eq!(
            (
                m.language.as_str(),
                m.visits,
                m.elsewhere,
                m.served_instead.as_deref()
            ),
            ("en", 1600, 600, Some("fr"))
        );
        let g = m.gain_monthly.unwrap();
        assert!(
            (g.mid - 18.0).abs() < 1.0 && g.low > 0.0 && g.low < 18.0 && 18.0 < g.high,
            "{g:?}"
        );
        assert_eq!((m.confidence, m.conversions_missing), ("solid", 0));
        // Same traffic, 4 signups in all: says how many are missing instead of concluding.
        let thin = [
            r("en", "en", false, 1000),
            r("en", "fr", false, 600),
            r("en", "en", true, 4),
        ];
        let m = &misrouted(&thin, &offered, 30, None)[0];
        assert_eq!((m.confidence, m.conversions_missing), ("insufficient", 10));
        assert!(
            m.gain_monthly.is_none(),
            "4 signups in all: no gain is stated"
        );
        // Goal never tracked: no gain is invented.
        let none = [r("en", "en", false, 1000), r("en", "fr", false, 600)];
        let m = &misrouted(&none, &offered, 30, None)[0];
        assert!(m.gain_monthly.is_none() && m.confidence == "insufficient");
    }

    #[test]
    fn texts_that_matter_and_legal_copy() {
        assert_eq!(
            text_kind("landing.heroTitle", "Ship in every language", None),
            Some(("headline", 4))
        );
        assert_eq!(
            text_kind("pricing.cta", "Start your free trial", None),
            Some(("pricing", 5))
        );
        assert_eq!(
            text_kind("nav.x", "Buy now", Some("button")),
            Some(("cta", 3))
        );
        assert_eq!(
            text_kind(
                "settings.description",
                "Changes are saved automatically",
                None
            ),
            None
        );
        assert_eq!(
            text_kind("legal.terms.title", "Terms of service", Some("heading")),
            None,
            "never legal copy"
        );
        assert_eq!(
            text_kind("checkout.refundPolicy.cta", "Read the policy", None),
            None
        );
        assert_eq!(
            text_kind("hero.title", &"long ".repeat(30), None),
            None,
            "prose is not a headline"
        );
        assert_eq!(
            tokens("pricing.heroTitle_v2"),
            ["pricing", "hero", "title", "v2"]
        );
        // Seen on a real catalog: back-office and account buttons are not conversion copy.
        assert_eq!(
            text_kind("admin.orizn_admin_title", "Orizn Admin", None),
            None
        );
        assert_eq!(text_kind("admin.quitter_button", "Afsluiten", None), None);
        assert_eq!(
            text_kind(
                "affiliate.dashboard.copy_link_button",
                "Link kopiëren",
                None
            ),
            None
        );
        assert_eq!(
            text_kind("auth.logout_button", "Log out", Some("button")),
            None
        );
    }

    #[test]
    fn legal_safe_refuses_new_facts() {
        let c = "Commencez votre essai de {days} jours";
        assert_eq!(legal_safe(c, "Lancez votre essai de {days} jours"), Ok(()));
        assert_eq!(legal_safe(c, "Essai de 30 jours"), Err("new_number"));
        assert_eq!(
            legal_safe(c, "Essayez {days} jours pour 0 €"),
            Err("new_number")
        );
        assert_eq!(
            legal_safe(c, "Essayez {days} jours, -50%"),
            Err("new_number")
        );
        assert_eq!(legal_safe("Save now", "Save 100% now"), Err("new_number"));
        assert_eq!(legal_safe("Save now", "Save € now"), Err("new_price"));
        assert_eq!(
            legal_safe(c, "Essai gratuit de {days} jours"),
            Err("new_claim")
        );
        assert_eq!(
            legal_safe("Essai gratuit", "Votre essai gratuit"),
            Ok(()),
            "a promise already made stays"
        );
        assert_eq!(legal_safe(c, "Essai sur https://x.io"), Err("new_link"));
    }

    #[test]
    fn lift_report_matches_the_counts() {
        let (lift, lo, hi, p) = lift_report((1000, 50), (1000, 80), 7);
        assert!(
            (lift - 0.6).abs() < 0.08 && lo > 0.0 && hi > lift && p > 0.99,
            "{lift} [{lo}; {hi}] {p}"
        );
        let (lift, lo, hi, p) = lift_report((1000, 50), (1000, 51), 7);
        assert!(
            lift.abs() < 0.1 && lo < 0.0 && hi > 0.0 && p < 0.7,
            "noise: {lift} [{lo}; {hi}] {p}"
        );
    }

    /// Outcome of one simulated optimisation with the production rules: allocation and winner as in
    /// `variants.rs`, harm looks as in `step`.
    #[derive(Debug, PartialEq)]
    enum End {
        Winner(usize),
        RolledBack,
        Nothing,
    }

    fn simulate(rates: &[f64], min_n: i64, horizon: usize, rng: &mut StdRng) -> (End, Vec<usize>) {
        // `live[i]` = index in `rates` of the i-th arm still served.
        let mut live: Vec<usize> = (0..rates.len()).collect();
        let mut arms = vec![(0i64, 0i64); rates.len()];
        let (mut looks, mut harm_looks, mut dropped) = (0, 0, vec![]);
        for _ in 0..horizon {
            let i = allocate(&arms, min_n, rng);
            arms[i].0 += 1;
            let total: i64 = arms.iter().map(|a| a.0).sum();
            if rng.random::<f64>() < rates[live[i]] {
                arms[i].1 += 1;
                if total >= next_look(arms.len(), min_n, looks) {
                    looks += 1;
                    if let Some(w) = decide(&arms, min_n, looks - 1, rng) {
                        return (End::Winner(live[w]), dropped);
                    }
                }
            }
            if total >= next_look(arms.len(), harm_min(min_n), harm_looks) {
                harm_looks += 1;
                for &b in harmful(&arms, harm_min(min_n), rng).iter().rev() {
                    dropped.push(live.remove(b));
                    arms.remove(b);
                }
                if arms.len() < 2 {
                    return (End::RolledBack, dropped);
                }
            }
        }
        (End::Nothing, dropped)
    }

    #[test]
    fn bandit_converges_on_the_better_wording_and_stays_silent_on_noise() {
        let mut rng = StdRng::seed_from_u64(2026);
        let runs = 100;
        // Truth: official 5 %, variant A 7.5 % (really better), variant B 5 % (same).
        let got: Vec<(End, Vec<usize>)> = (0..runs)
            .map(|_| simulate(&[0.05, 0.075, 0.05], 500, 50_000, &mut rng))
            .collect();
        let right = got.iter().filter(|g| g.0 == End::Winner(1)).count();
        let wrong = got
            .iter()
            .filter(|g| matches!(g.0, End::Winner(0 | 2) | End::RolledBack))
            .count();
        let lost = got.iter().filter(|g| g.1.contains(&1)).count();
        eprintln!("better wording: kept {right}, wrong {wrong}, withdrawn by mistake {lost}");
        assert!(right >= 90, "the truly better wording wins: {right}/{runs}");
        assert!(wrong <= 5 && lost <= 5, "wrong {wrong}, lost {lost}");
        // Noise: three identical wordings. Any winner or rollback is a false alarm.
        let got: Vec<(End, Vec<usize>)> = (0..runs)
            .map(|_| simulate(&[0.05, 0.05, 0.05], 500, 50_000, &mut rng))
            .collect();
        let winners = got.iter().filter(|g| matches!(g.0, End::Winner(_))).count();
        let withdrawn = got.iter().filter(|g| !g.1.is_empty()).count();
        eprintln!("noise: false winners {winners}, false withdrawals {withdrawn} of {runs}");
        assert!(winners <= 8, "no winner on noise: {winners}/{runs}");
        assert!(withdrawn <= 8, "no withdrawal on noise: {withdrawn}/{runs}");
    }

    #[test]
    fn harmful_wording_is_withdrawn() {
        let mut rng = StdRng::seed_from_u64(7);
        let runs = 100;
        // Truth: official 5 %, variant A 2.5 % (harmful), variant B 5 % (same).
        let got: Vec<(End, Vec<usize>)> = (0..runs)
            .map(|_| simulate(&[0.05, 0.025, 0.05], 500, 30_000, &mut rng))
            .collect();
        let caught = got.iter().filter(|g| g.1.contains(&1)).count();
        let kept = got.iter().filter(|g| g.0 == End::Winner(1)).count();
        let innocent = got.iter().filter(|g| g.1.contains(&2)).count();
        eprintln!(
            "harmful wording: withdrawn {caught}, promoted {kept}, innocent withdrawn {innocent}"
        );
        assert_eq!(kept, 0, "a harmful wording is never promoted");
        assert!(caught >= 90, "withdrawn in {caught}/{runs}");
        assert!(innocent <= 5, "{innocent}");
        // Deterministic spot checks of the rule itself.
        assert_eq!(
            harmful(&[(400, 80), (400, 8), (400, 78)], 250, &mut rng),
            vec![1]
        );
        assert_eq!(
            harmful(&[(400, 80), (200, 4)], 250, &mut rng),
            Vec::<usize>::new(),
            "under the minimum sample"
        );
        assert_eq!(
            harmful(&[(400, 80), (400, 70)], 250, &mut rng),
            Vec::<usize>::new(),
            "not credibly worse"
        );
    }
}
