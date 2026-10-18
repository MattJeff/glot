//! Locale insights: anonymous demand-gap + funnel ingestion from `@glot/insights`, and language experiments.
//! Privacy: one row per (day, preferred language, served locale, country, arm, event) with a counter.
//! No IP, user agent, cookie, visitor id or route is ever stored.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, header},
    routing::{patch, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use glot_core::{
    config::{Config, LlmConfig},
    context::KeyContext,
};
use glot_llm::translate::{Item, Request};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
    validate_locale,
};

const VIEW: &str = "$view";

/// Public ingestion endpoint, callable from any origin (the customer's app), no credentials.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/insights/{key}", post(ingest))
        .layer(DefaultBodyLimit::max(4096))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

/// Dashboard routes (session/token auth), merged under the API CORS layer.
pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/insights";
    Router::new()
        .route(p, axum::routing::get(overview))
        .route(&format!("{p}/rotate-key"), post(rotate_key))
        .route(&format!("{p}/experiments"), post(create_experiment))
        .route(&format!("{p}/experiments/{{id}}"), patch(decide_experiment))
}

// ---------- ingestion ----------

#[derive(Deserialize)]
pub struct Hit {
    /// `navigator.languages` (or the parsed Accept-Language when sent from a middleware).
    #[serde(default)]
    langs: Vec<String>,
    /// Locale the app actually rendered.
    served: String,
    /// Absent = a visit; else a conversion event name (`signup`, `purchase`, custom).
    event: Option<String>,
    /// ISO 3166 country from a CDN header, when the caller is a server middleware.
    country: Option<String>,
    /// Experiment arm previously returned to this session.
    arm: Option<String>,
    // ponytail: `route` is accepted for forward compat but never stored.
    #[allow(dead_code)]
    route: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct HitResponse {
    /// Locale to render for this session when it is in the test arm of a running experiment.
    pub locale: Option<String>,
    pub arm: Option<String>,
}

/// Primary language subtag, lowercased: `de-AT` → `de`.
pub fn lang(s: &str) -> String {
    s.split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Preferred language: first valid entry of `langs`, else the first Accept-Language tag.
pub fn preferred(langs: &[String], accept_language: Option<&str>) -> Option<String> {
    let from_header = accept_language
        .into_iter()
        .flat_map(|h| h.split(','))
        .map(|t| t.split(';').next().unwrap_or("").trim().to_string());
    langs
        .iter()
        .cloned()
        .chain(from_header)
        .find(|l| l != "*" && validate_locale(l).is_ok())
        .map(|l| lang(&l))
}

/// Two-letter country from the body or a CDN header (Cloudflare, Vercel, CloudFront); '' when unknown.
pub fn country(body: Option<&str>, headers: &HeaderMap) -> String {
    let h = [
        "cf-ipcountry",
        "x-vercel-ip-country",
        "cloudfront-viewer-country",
    ]
    .iter()
    .find_map(|n| headers.get(*n)?.to_str().ok());
    match body.or(h) {
        Some(c) if c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic()) => {
            let c = c.to_ascii_uppercase();
            if c == "XX" || c == "T1" {
                String::new()
            } else {
                c
            }
        }
        _ => String::new(),
    }
}

fn valid_event(e: &str) -> bool {
    (1..=40).contains(&e.len())
        && e.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}

/// Hits accepted per public key per minute. The key ships in page source, so anyone can post with it.
const HITS_PER_MINUTE: u32 = 1200;

/// Fixed-window counter: returns false once `key` used its budget for `minute`.
/// ponytail: per-process memory; behind several replicas the budget multiplies — move to Postgres/Redis then.
fn allow_hit(
    window: &mut (u64, std::collections::HashMap<String, u32>),
    key: &str,
    minute: u64,
) -> bool {
    if window.0 != minute {
        *window = (minute, Default::default());
    }
    let n = window.1.entry(key.to_string()).or_default();
    *n += 1;
    *n <= HITS_PER_MINUTE
}

static HIT_WINDOW: std::sync::LazyLock<
    std::sync::Mutex<(u64, std::collections::HashMap<String, u32>)>,
> = std::sync::LazyLock::new(Default::default);

pub async fn ingest(
    State(state): State<AppState>,
    AppPath(key): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<HitResponse>> {
    let minute = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 60);
    if !allow_hit(
        &mut HIT_WINDOW.lock().unwrap_or_else(|e| e.into_inner()),
        &key,
        minute,
    ) {
        return Err(AppError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many insight hits for this key; retry next minute",
        ));
    }
    // Any content type: the snippet posts text/plain to avoid a CORS preflight.
    let hit: Hit = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    validate_locale(&hit.served)?;
    let event = match hit.event.as_deref() {
        None => VIEW,
        Some(e) if valid_event(e) => e,
        Some(_) => {
            return Err(AppError::bad_request(
                "event must be 1-40 chars of a-z0-9_-.:",
            ));
        }
    };
    let project_id: Uuid =
        sqlx::query_scalar("SELECT project_id FROM insight_keys WHERE public_key = $1")
            .bind(&key)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::not_found("unknown insights key"))?;
    let accept = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    let preferred = preferred(&hit.langs, accept).unwrap_or_else(|| "und".into());
    let country = country(hit.country.as_deref(), &headers);

    // Running experiment for this visitor's language: keep the session's arm, else draw one.
    let exp: Option<(String, i32)> = sqlx::query_as(
        "SELECT locale, traffic_pct FROM insight_experiments
         WHERE project_id = $1 AND status = 'running' AND now() < ends_at AND split_part(lower(locale), '-', 1) = $2",
    )
    .bind(project_id)
    .bind(&preferred)
    .fetch_optional(&state.db)
    .await?;
    let (arm, locale) = match exp {
        None => (None, None),
        Some((locale, pct)) => {
            let arm = match hit.arm.as_deref() {
                Some(a @ ("test" | "control")) => a,
                _ if event != VIEW => "", // events never draw an arm
                _ if (rand::random::<u32>() % 100) < pct as u32 => "test",
                _ => "control",
            };
            match arm {
                "" => (None, None),
                "test" => (Some("test"), Some(locale)),
                a => (Some(a), None),
            }
        }
    };

    sqlx::query(
        "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
         VALUES ($1, (now() AT TIME ZONE 'utc')::date, $2, $3, $4, $5, $6, 1)
         ON CONFLICT (project_id, day, preferred, served, country, arm, event) DO UPDATE SET n = insight_daily.n + 1",
    )
    .bind(project_id)
    .bind(&preferred)
    .bind(&hit.served)
    .bind(&country)
    .bind(arm.unwrap_or(""))
    .bind(event)
    .execute(&state.db)
    .await?;
    Ok(Json(HitResponse {
        locale,
        arm: arm.map(String::from),
    }))
}

// ---------- stats ----------

/// Wilson score interval (95 %) for k successes out of n.
pub fn wilson(k: i64, n: i64) -> (f64, f64) {
    if n <= 0 {
        return (0.0, 0.0);
    }
    let (n, z) = (n as f64, 1.96f64);
    let p = k as f64 / n;
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

/// Visitors per arm needed to detect a +20 % relative lift on `baseline` (α = 0.05 two-sided, power 0.8).
pub fn required_per_arm(baseline: f64) -> Option<i64> {
    if baseline <= 0.0 || baseline >= 1.0 / 1.2 {
        return None;
    }
    let q = baseline * 1.2;
    let z = 1.96 + 0.8416;
    Some(
        (z * z * (baseline * (1.0 - baseline) + q * (1.0 - q)) / (q - baseline).powi(2)).ceil()
            as i64,
    )
}

#[derive(Serialize, Default, Clone, Copy, Debug, utoipa::ToSchema)]
pub struct Arm {
    pub visits: i64,
    pub conversions: i64,
    pub rate: f64,
}

impl Arm {
    fn new(visits: i64, conversions: i64) -> Self {
        let rate = if visits > 0 {
            conversions as f64 / visits as f64
        } else {
            0.0
        };
        Self {
            visits,
            conversions,
            rate,
        }
    }
}

#[derive(Serialize, Debug, PartialEq, utoipa::ToSchema)]
pub struct Verdict {
    /// test rate − control rate, with its 95 % confidence interval (two-proportion, unpooled).
    pub diff: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    pub required_per_arm: Option<i64>,
    /// collecting | keep | remove | inconclusive
    pub verdict: &'static str,
}

/// No conclusion before the required sample (or the end date) — avoids stopping on a lucky peek.
pub fn verdict(test: Arm, control: Arm, ended: bool) -> Verdict {
    let diff = test.rate - control.rate;
    let se = if test.visits > 0 && control.visits > 0 {
        (test.rate * (1.0 - test.rate) / test.visits as f64
            + control.rate * (1.0 - control.rate) / control.visits as f64)
            .sqrt()
    } else {
        f64::INFINITY
    };
    let (ci_low, ci_high) = if se.is_finite() {
        (diff - 1.96 * se, diff + 1.96 * se)
    } else {
        (-1.0, 1.0)
    };
    let required = required_per_arm(control.rate);
    let enough = test.visits.min(control.visits) >= required.unwrap_or(1000);
    let verdict = if !(enough || ended) || test.visits == 0 || control.visits == 0 {
        "collecting"
    } else if ci_low > 0.0 {
        "keep"
    } else if ci_high < 0.0 {
        "remove"
    } else {
        "inconclusive"
    };
    Verdict {
        diff,
        ci_low,
        ci_high,
        required_per_arm: required,
        verdict,
    }
}

// ---------- dashboard ----------

#[derive(Serialize, utoipa::ToSchema)]
pub struct Demand {
    /// Preferred language (primary subtag).
    pub language: String,
    pub visits: i64,
    pub share: f64,
    /// The project already ships this language.
    pub offered: bool,
    pub conversions: i64,
    pub conversion_rate: Option<f64>,
    /// Extra goal conversions per 30 days if these visitors converted like visitors served their own language.
    pub est_extra_conversions: Option<f64>,
    /// Dry-run estimate of translating every key into this language (glot pricing table).
    pub est_cost_usd: Option<f64>,
    pub top_countries: Vec<(String, i64)>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LocaleFunnel {
    pub locale: String,
    pub is_source: bool,
    pub visits: i64,
    pub conversions: i64,
    pub rate: f64,
    pub ci_low: f64,
    pub ci_high: f64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct DailyPoint {
    pub day: NaiveDate,
    pub visits: i64,
    /// Visits whose preferred language the project does not offer.
    pub unserved: i64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Experiment {
    pub id: Uuid,
    pub locale: String,
    pub traffic_pct: i32,
    pub goal: String,
    pub started_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub status: String,
    pub decided_at: Option<DateTime<Utc>>,
    pub test: Arm,
    pub control: Arm,
    #[serde(flatten)]
    pub stats: Verdict,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Overview {
    pub public_key: String,
    pub days: i64,
    pub goal: String,
    pub source_locale: String,
    pub offered: Vec<String>,
    pub keys: i64,
    pub events: Vec<String>,
    pub total_visits: i64,
    pub demand: Vec<Demand>,
    pub funnel: Vec<LocaleFunnel>,
    pub daily: Vec<DailyPoint>,
    pub experiments: Vec<Experiment>,
}

#[derive(Deserialize)]
pub struct OverviewQuery {
    days: Option<i64>,
    goal: Option<String>,
}

fn new_key() -> String {
    format!("pk_{}", hex::encode(rand::random::<[u8; 12]>()))
}

async fn public_key(state: &AppState, project_id: Uuid) -> ApiResult<String> {
    sqlx::query(
        "INSERT INTO insight_keys (project_id, public_key) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(new_key())
    .execute(&state.db)
    .await?;
    Ok(
        sqlx::query_scalar("SELECT public_key FROM insight_keys WHERE project_id = $1")
            .bind(project_id)
            .fetch_one(&state.db)
            .await?,
    )
}

/// Same estimate as `glot translate --dry-run` for every key into `locale` (no glossary, no context).
fn estimate_cost(p: &ProjectRow, keys: &[(String, String)], locale: &str) -> f64 {
    let llm = p.llm.0.clone();
    let config = Config {
        source_locale: p.source_locale.clone(),
        target_locales: vec![locale.into()],
        files: vec![],
        format: Default::default(),
        syntax: Default::default(),
        llm: LlmConfig {
            provider: llm.provider,
            model: llm.model,
            fallback_model: llm.fallback_model,
            judge_model: llm.judge_model,
            api_key_env: None,
            base_url: llm.base_url,
            budget_usd: llm.budget_usd,
            style: p.style.clone(),
            ..LlmConfig::default()
        },
        qa: Default::default(),
        extract: Default::default(),
        server: None,
    };
    let req = Request {
        source_locale: p.source_locale.clone(),
        target_locale: locale.into(),
        items: keys
            .iter()
            .map(|(k, s)| Item {
                key: k.clone(),
                source: s.clone(),
                context: KeyContext {
                    key: k.clone(),
                    ..Default::default()
                },
                previous: None,
            })
            .collect(),
        glossary: vec![],
        syntax: config.syntax,
        style: p.style.clone(),
    };
    glot_llm::translate::estimate(&config, &req).cost_usd
}

async fn experiments(state: &AppState, project_id: Uuid) -> ApiResult<Vec<Experiment>> {
    type Row = (
        Uuid,
        String,
        i32,
        String,
        DateTime<Utc>,
        DateTime<Utc>,
        String,
        Option<DateTime<Utc>>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, locale, traffic_pct, goal, started_at, ends_at, status, decided_at
         FROM insight_experiments WHERE project_id = $1 ORDER BY started_at DESC LIMIT 20",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    let mut out = vec![];
    for (id, locale, traffic_pct, goal, started_at, ends_at, status, decided_at) in rows {
        let arms: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT arm, coalesce(sum(n) FILTER (WHERE event = '$view'), 0)::bigint,
                    coalesce(sum(n) FILTER (WHERE event = $5), 0)::bigint
             FROM insight_daily WHERE project_id = $1 AND preferred = $2 AND arm <> ''
               AND day BETWEEN $3::date AND $4::date GROUP BY arm",
        )
        .bind(project_id)
        .bind(lang(&locale))
        .bind(started_at)
        .bind(decided_at.unwrap_or(ends_at).min(ends_at))
        .bind(&goal)
        .fetch_all(&state.db)
        .await?;
        let arm = |name: &str| {
            arms.iter()
                .find(|a| a.0 == name)
                .map_or(Arm::default(), |a| Arm::new(a.1, a.2.min(a.1)))
        };
        let (test, control) = (arm("test"), arm("control"));
        out.push(Experiment {
            id,
            locale,
            traffic_pct,
            goal,
            started_at,
            ends_at,
            stats: verdict(test, control, Utc::now() >= ends_at),
            status,
            decided_at,
            test,
            control,
        });
    }
    Ok(out)
}

pub async fn overview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<OverviewQuery>,
) -> ApiResult<Json<Overview>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let goal = q
        .goal
        .filter(|g| valid_event(g))
        .unwrap_or_else(|| "signup".into());
    let public_key = public_key(&state, p.id).await?;

    let rows: Vec<(NaiveDate, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT day, preferred, served, country, event, sum(n)::bigint FROM insight_daily
         WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int
         GROUP BY 1, 2, 3, 4, 5",
    )
    .bind(p.id)
    .bind(days as i32)
    .fetch_all(&state.db)
    .await?;

    let offered: Vec<String> = std::iter::once(p.source_locale.clone())
        .chain(p.target_locales.iter().cloned())
        .collect();
    let offered_langs: BTreeSet<String> = offered.iter().map(|l| lang(l)).collect();

    let mut events = BTreeSet::new();
    let mut total_visits = 0;
    let mut by_lang: BTreeMap<String, (i64, i64, BTreeMap<String, i64>)> = BTreeMap::new();
    let mut by_served: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut daily: BTreeMap<NaiveDate, (i64, i64)> = BTreeMap::new();
    // Visitors served in their own language vs not: the observed "localization lift".
    let (mut matched, mut mismatched) = ((0i64, 0i64), (0i64, 0i64));
    for (day, pref, served, country, event, n) in &rows {
        let is_view = event == VIEW;
        if !is_view {
            events.insert(event.clone());
        }
        if !is_view && *event != goal {
            continue;
        }
        let (v, c) = if is_view { (*n, 0) } else { (0, *n) };
        let l = by_lang.entry(pref.clone()).or_default();
        l.0 += v;
        l.1 += c;
        let s = by_served.entry(served.clone()).or_default();
        s.0 += v;
        s.1 += c;
        let m = if lang(served) == *pref {
            &mut matched
        } else {
            &mut mismatched
        };
        m.0 += v;
        m.1 += c;
        if is_view {
            total_visits += n;
            *l.2.entry(country.clone()).or_default() += n;
            let d = daily.entry(*day).or_default();
            d.0 += n;
            if !offered_langs.contains(pref) {
                d.1 += n;
            }
        }
    }

    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1 ORDER BY key")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let lift_rate =
        (matched.0 > 0 && mismatched.0 > 0).then(|| matched.1 as f64 / matched.0 as f64);
    let per_month = 30.0 / days as f64;

    let mut demand: Vec<Demand> = by_lang
        .into_iter()
        .filter(|(l, v)| l != "und" && v.0 > 0)
        .map(|(language, (visits, conversions, countries))| {
            let offered = offered_langs.contains(&language);
            let rate = conversions as f64 / visits as f64;
            let mut top: Vec<(String, i64)> = countries
                .into_iter()
                .filter(|(c, _)| !c.is_empty())
                .collect();
            top.sort_by_key(|c| std::cmp::Reverse(c.1));
            top.truncate(3);
            Demand {
                share: visits as f64 / total_visits.max(1) as f64,
                est_extra_conversions: (!offered)
                    .then_some(lift_rate)
                    .flatten()
                    .map(|r| (r - rate).max(0.0) * visits as f64 * per_month),
                est_cost_usd: None,
                conversion_rate: Some(rate),
                offered,
                visits,
                conversions,
                top_countries: top,
                language,
            }
        })
        .collect();
    demand.sort_by(|a, b| a.offered.cmp(&b.offered).then(b.visits.cmp(&a.visits)));
    // ponytail: cost estimated for the 8 biggest gaps only (pure CPU, scales with the key count).
    for d in demand.iter_mut().filter(|d| !d.offered).take(8) {
        d.est_cost_usd = Some(estimate_cost(&p, &keys, &d.language));
    }

    let funnel = by_served
        .into_iter()
        .filter(|(_, (v, _))| *v > 0)
        .map(|(locale, (visits, conversions))| {
            let conversions = conversions.min(visits);
            let (ci_low, ci_high) = wilson(conversions, visits);
            LocaleFunnel {
                is_source: locale == p.source_locale,
                rate: conversions as f64 / visits as f64,
                locale,
                visits,
                conversions,
                ci_low,
                ci_high,
            }
        })
        .collect();

    Ok(Json(Overview {
        public_key,
        days,
        goal,
        source_locale: p.source_locale.clone(),
        offered,
        keys: keys.len() as i64,
        events: events.into_iter().collect(),
        total_visits,
        demand,
        funnel,
        daily: daily
            .into_iter()
            .map(|(day, (visits, unserved))| DailyPoint {
                day,
                visits,
                unserved,
            })
            .collect(),
        experiments: experiments(&state, p.id).await?,
    }))
}

pub async fn rotate_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let key = new_key();
    sqlx::query(
        "INSERT INTO insight_keys (project_id, public_key) VALUES ($1, $2)
         ON CONFLICT (project_id) DO UPDATE SET public_key = $2, rotated_at = now()",
    )
    .bind(p.id)
    .bind(&key)
    .execute(&state.db)
    .await?;
    Ok(Json(serde_json::json!({ "public_key": key })))
}

#[derive(Deserialize)]
pub struct NewExperiment {
    locale: String,
    traffic_pct: i32,
    days: i64,
    goal: Option<String>,
}

pub async fn create_experiment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<NewExperiment>,
) -> ApiResult<(StatusCode, Json<Experiment>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    validate_locale(&b.locale)?;
    if lang(&b.locale) == lang(&p.source_locale) {
        return Err(AppError::bad_request(
            "cannot experiment on the source language",
        ));
    }
    if !(1..=100).contains(&b.traffic_pct) || !(1..=90).contains(&b.days) {
        return Err(AppError::bad_request(
            "traffic_pct must be 1-100 and days 1-90",
        ));
    }
    let goal = b.goal.unwrap_or_else(|| "signup".into());
    if !valid_event(&goal) {
        return Err(AppError::bad_request("invalid goal event name"));
    }
    let mut tx = state.db.begin().await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO insight_experiments (id, project_id, locale, traffic_pct, goal, ends_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(days => $6))",
    )
    .bind(id)
    .bind(p.id)
    .bind(&b.locale)
    .bind(b.traffic_pct)
    .bind(&goal)
    .bind(b.days as i32)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.code == "conflict" => {
            AppError::conflict("an experiment is already running for this language")
        }
        e => e,
    })?;
    // The test arm must be servable: the locale joins the targets (translate it before traffic arrives).
    sqlx::query(
        "UPDATE projects SET target_locales = array_append(target_locales, $2)
         WHERE id = $1 AND NOT ($2 = ANY(target_locales))",
    )
    .bind(p.id)
    .bind(&b.locale)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let exp = experiments(&state, p.id)
        .await?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::internal("experiment vanished"))?;
    Ok((StatusCode::CREATED, Json(exp)))
}

#[derive(Deserialize)]
pub struct Decision {
    /// kept | removed
    status: String,
}

/// Close an experiment. `removed` also drops the locale from the project targets (translations are kept).
pub async fn decide_experiment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<Decision>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if b.status != "kept" && b.status != "removed" {
        return Err(AppError::bad_request("status must be kept or removed"));
    }
    let mut tx = state.db.begin().await?;
    let locale: String = sqlx::query_scalar(
        "UPDATE insight_experiments SET status = $3, decided_at = now()
         WHERE id = $1 AND project_id = $2 AND status = 'running' RETURNING locale",
    )
    .bind(id)
    .bind(p.id)
    .bind(&b.status)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("running experiment not found"))?;
    if b.status == "removed" {
        sqlx::query(
            "UPDATE projects SET target_locales = array_remove(target_locales, $2) WHERE id = $1",
        )
        .bind(p.id)
        .bind(&locale)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_language() {
        assert_eq!(
            preferred(&["de-AT".into(), "en".into()], None).as_deref(),
            Some("de")
        );
        assert_eq!(
            preferred(&["<script>".into()], Some("pt-BR,pt;q=0.9")).as_deref(),
            Some("pt")
        );
        assert_eq!(preferred(&[], Some("*")), None);
    }

    #[test]
    fn country_header() {
        let mut h = HeaderMap::new();
        h.insert("cf-ipcountry", "de".parse().unwrap());
        assert_eq!(country(None, &h), "DE");
        assert_eq!(country(Some("CH"), &h), "CH");
        assert_eq!(country(Some("XX"), &HeaderMap::new()), "");
        assert_eq!(country(Some("../"), &HeaderMap::new()), "");
    }

    #[test]
    fn stats() {
        let (lo, hi) = wilson(10, 100);
        assert!((lo - 0.0552).abs() < 1e-3 && (hi - 0.1744).abs() < 1e-3);
        // 5 % baseline, +20 % relative: ≈ 8,150 per arm.
        let n = required_per_arm(0.05).unwrap();
        assert!((8100..8200).contains(&n), "{n}");
        assert_eq!(required_per_arm(0.0), None);

        let c = Arm::new(10_000, 500);
        assert_eq!(verdict(Arm::new(10_000, 650), c, false).verdict, "keep");
        assert_eq!(verdict(Arm::new(10_000, 380), c, false).verdict, "remove");
        assert_eq!(
            verdict(Arm::new(10_000, 505), c, false).verdict,
            "inconclusive"
        );
        // Clear signal but under the required sample and not ended: keep collecting.
        assert_eq!(
            verdict(Arm::new(500, 50), Arm::new(500, 25), false).verdict,
            "collecting"
        );
        assert_eq!(
            verdict(Arm::new(500, 50), Arm::new(500, 25), true).verdict,
            "keep"
        );
        let v = verdict(Arm::new(10_000, 650), c, false);
        assert!(v.ci_low > 0.0 && v.ci_low < v.diff && v.diff < v.ci_high);
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;

    #[test]
    fn budget_per_key_and_minute() {
        let mut w = Default::default();
        for _ in 0..HITS_PER_MINUTE {
            assert!(allow_hit(&mut w, "k", 1));
        }
        assert!(!allow_hit(&mut w, "k", 1));
        assert!(allow_hit(&mut w, "other", 1));
        assert!(allow_hit(&mut w, "k", 2));
    }
}
