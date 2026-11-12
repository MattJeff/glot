//! Read-only PostHog importer for Autopilot: daily aggregates of the customer's own PostHog history, so markets can
//! be ranked before (or without) the glot tag.
//!
//! **API** (docs: <https://posthog.com/docs/api/queries>): `POST {host}/api/projects/{id}/query/` with
//! `{"query": {"kind": "HogQLQuery", "query": "…"}}`, `Authorization: Bearer <personal API key>`, scope
//! `query:read`. Results are positional rows; an explicit `LIMIT` is required beyond 100 rows (max 50 000).
//! Properties used are set by posthog-js on every pageview: `$browser_language` (= `navigator.language`),
//! `$session_id`, `$pathname`, `$host`; `$geoip_country_code` is PostHog's GeoIP enrichment.
//!
//! **Privacy.** Only aggregates leave PostHog: per (day, browser language prefix, served locale, country) the number
//! of sessions, pageviews, one-page sessions and distinct visitors (`uniq`), and the number of sessions that fired the
//! conversion event. Person and session ids are counted inside the query and never returned; paths and hosts only
//! come back as a locale code of this project (or nothing). No URL, IP, person, or event payload is fetched or stored.
//! The key is stored encrypted (`auth::encrypt`), never logged, never returned.
//!
//! **Sync.** Backfill `backfill_days` (default 90) in 7-day windows on the first sync, then every 6 h from the last
//! synced day (re-fetched: it was partial). A window's rows replace the stored ones in one transaction.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    insights::lang,
    projects::{self, ProjectRow, Role},
    validate_locale,
};

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/autopilot/posthog";
    Router::new()
        .route(p, get(view).put(save).delete(disconnect))
        .route(&format!("{p}/test"), post(test))
        .route(&format!("{p}/sync"), post(sync_now))
}

/// Stored table the engine reads (same columns as `insight_daily`, minus the experiment arm).
pub const TABLE: &str = "autopilot_posthog_daily";
/// Rows per query: far above a week of (day × language × locale × country) groups; reaching it is an error.
const LIMIT: usize = 50_000;
const WINDOW_DAYS: i64 = 7;
const RESYNC_HOURS: i64 = 6;

// ---------- pure helpers ----------

/// `us` / `eu` / an ingestion host copied from the snippet (`us.i.posthog.com`) / `app.posthog.com` → the API host;
/// anything else must be an https URL of a self-hosted instance (http only for local tests).
pub fn api_host(input: &str) -> Result<String, String> {
    let s = input.trim().trim_end_matches('/').to_ascii_lowercase();
    let bare = s
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    match bare {
        "us" | "us.posthog.com" | "us.i.posthog.com" | "app.posthog.com" | "posthog.com" => {
            return Ok("https://us.posthog.com".into());
        }
        "eu" | "eu.posthog.com" | "eu.i.posthog.com" => return Ok("https://eu.posthog.com".into()),
        _ => {}
    }
    let u = reqwest::Url::parse(&s).map_err(|_| {
        format!("'{input}' is not a PostHog host: use US, EU, or the https URL of your instance")
    })?;
    let ok_scheme =
        u.scheme() == "https" || (u.scheme() == "http" && crate::connectors::allow_private());
    if !ok_scheme || u.host_str().is_none() || u.query().is_some() {
        return Err(format!(
            "'{input}' is not a PostHog host: use US, EU, or the https URL of your instance"
        ));
    }
    Ok(s)
}

/// HogQL string literal (backslash escapes, as ClickHouse).
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Locale served by a pageview, computed inside PostHog: the first path segment or the first host label when it is
/// one of `locales` (lowercased), else `default` ('' = unknown). Only this code ever leaves PostHog.
fn served_expr(locales: &[String], default: &str) -> String {
    let list = locales
        .iter()
        .map(|l| lit(l))
        .collect::<Vec<_>>()
        .join(", ");
    let seg = "lower(arrayElement(splitByChar('/', ifNull(properties.$pathname, '')), 2))";
    let sub = "lower(arrayElement(splitByChar('.', ifNull(properties.$host, '')), 1))";
    if list.is_empty() {
        return lit(default);
    }
    format!(
        "multiIf({seg} IN ({list}), {seg}, {sub} IN ({list}), {sub}, {})",
        lit(default)
    )
}

const LANG: &str =
    "lower(arrayElement(splitByChar('-', ifNull(properties.$browser_language, '')), 1))";
const COUNTRY: &str = "upper(ifNull(properties.$geoip_country_code, ''))";

fn window(from: NaiveDate, to: NaiveDate) -> String {
    format!(
        "timestamp >= toDateTime({}) AND timestamp < toDateTime({})",
        lit(&format!("{from} 00:00:00")),
        lit(&format!("{to} 00:00:00"))
    )
}

/// Sessions, pageviews, one-page sessions and distinct visitors per (day, language, served, country) in [from, to).
// ponytail: a session is counted on the day it starts, in the window it starts; one crossing midnight at a window
// boundary is split in two. Negligible for daily rankings.
pub fn pageviews_query(
    from: NaiveDate,
    to: NaiveDate,
    locales: &[String],
    default: &str,
) -> String {
    format!(
        "SELECT day, lang, served, country, count() AS sessions, sum(pv) AS pageviews, countIf(pv = 1) AS bounced, \
         uniq(person) AS visitors FROM (SELECT properties.$session_id AS sid, toDate(min(timestamp)) AS day, \
         any({LANG}) AS lang, any({served}) AS served, any({COUNTRY}) AS country, count() AS pv, \
         any(person_id) AS person FROM events WHERE event = '$pageview' AND {w} \
         AND ifNull(properties.$session_id, '') != '' GROUP BY sid) \
         GROUP BY day, lang, served, country ORDER BY day LIMIT {LIMIT}",
        served = served_expr(locales, default),
        w = window(from, to),
    )
}

/// Sessions that fired `event` at least once, per (day, language, served, country) in [from, to).
pub fn conversions_query(
    event: &str,
    from: NaiveDate,
    to: NaiveDate,
    locales: &[String],
    default: &str,
) -> String {
    format!(
        "SELECT day, lang, served, country, count() AS sessions FROM (SELECT properties.$session_id AS sid, \
         toDate(min(timestamp)) AS day, any({LANG}) AS lang, any({served}) AS served, any({COUNTRY}) AS country \
         FROM events WHERE event = {e} AND {w} AND ifNull(properties.$session_id, '') != '' GROUP BY sid) \
         GROUP BY day, lang, served, country ORDER BY day LIMIT {LIMIT}",
        e = lit(event),
        served = served_expr(locales, default),
        w = window(from, to),
    )
}

/// The project's 100 most frequent event names over 30 days (names and counts only), and how many pageviews carry
/// a browser language.
pub const EVENTS_QUERY: &str = "SELECT event, count() AS n FROM events WHERE timestamp >= now() - INTERVAL 30 DAY \
     GROUP BY event ORDER BY n DESC LIMIT 100";
pub const COVERAGE_QUERY: &str = "SELECT count() AS pageviews, \
     countIf(ifNull(properties.$browser_language, '') != '') AS with_language FROM events \
     WHERE event = '$pageview' AND timestamp >= now() - INTERVAL 30 DAY LIMIT 1";

/// A sensible conversion among the project's real events (signup first, then purchase), else none.
pub fn suggest_event(events: &[(String, i64)]) -> Option<String> {
    const WORDS: [&[&str]; 2] = [
        &[
            "signup",
            "sign_up",
            "sign up",
            "signed up",
            "signed_up",
            "register",
            "registration",
            "account created",
        ],
        &[
            "purchase",
            "checkout",
            "order",
            "subscribe",
            "subscription",
            "payment",
            "paid",
        ],
    ];
    WORDS.iter().find_map(|words| {
        events
            .iter()
            .filter(|(e, _)| !e.starts_with('$'))
            .find(|(e, _)| {
                let e = e.to_lowercase();
                words.iter().any(|w| e.contains(w))
            })
            .map(|(e, _)| e.clone())
    })
}

/// A PostHog refusal in plain words: what is wrong and where to fix it. Never includes the key.
pub fn explain(status: u16, body: &str, host: &str, project: &str) -> AppError {
    let detail = serde_json::from_str::<JsonValue>(body)
        .ok()
        .and_then(|v| v["detail"].as_str().map(String::from))
        .unwrap_or_default();
    let region = "(US: us.posthog.com, EU: eu.posthog.com)";
    match status {
        401 => AppError::new(
            StatusCode::BAD_REQUEST,
            "posthog_unauthorized",
            format!(
                "PostHog refused the key (401). Use a personal API key (phx_…), not the project key (phc_…), and \
                 check the region matches your account {region}."
            ),
        ),
        403 if detail.to_lowercase().contains("scope") => AppError::new(
            StatusCode::BAD_REQUEST,
            "posthog_scope",
            "The key is missing the Query Read scope (query:read): in PostHog, Settings → Personal API keys, edit the \
             key and tick Query → Read.",
        ),
        403 => AppError::new(
            StatusCode::BAD_REQUEST,
            "posthog_forbidden",
            format!(
                "PostHog refused access to project {project} (403): the key is not allowed on this project or \
                 organisation. {detail}"
            ),
        ),
        404 => AppError::new(
            StatusCode::BAD_REQUEST,
            "posthog_project",
            format!(
                "PostHog has no project {project} on {host}: copy the project ID from Project settings, and check \
                 the region {region}."
            ),
        ),
        429 => AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "posthog_rate_limited",
            "PostHog's query budget for this hour is used up: glot retries at the next sync.",
        ),
        s => AppError::new(
            StatusCode::BAD_GATEWAY,
            "posthog_error",
            format!(
                "PostHog answered {s}: {}",
                detail.chars().take(200).collect::<String>()
            ),
        ),
    }
}

// ---------- HTTP ----------

pub struct Conn {
    pub host: String,
    pub project: String,
    pub key: String,
}

/// Runs one HogQL query; rows as positional JSON values.
async fn query(c: &Conn, hogql: &str) -> ApiResult<Vec<Vec<JsonValue>>> {
    let unreachable = |e: String| {
        AppError::new(
            StatusCode::BAD_GATEWAY,
            "posthog_host",
            format!("Cannot reach PostHog at {}: {e}", c.host),
        )
    };
    let client = crate::connectors::client_for(&c.host)
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    let res = client
        .post(format!("{}/api/projects/{}/query/", c.host, c.project))
        .bearer_auth(&c.key)
        .json(
            &json!({ "query": { "kind": "HogQLQuery", "query": hogql }, "name": "glot-autopilot" }),
        )
        .send()
        .await
        // The error names the URL (host + project id), never the request headers.
        .map_err(|e| unreachable(e.without_url().to_string()))?;
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    if status != 200 {
        return Err(explain(status, &body, &c.host, &c.project));
    }
    let v: JsonValue = serde_json::from_str(&body).map_err(|_| {
        AppError::new(
            StatusCode::BAD_GATEWAY,
            "posthog_error",
            "PostHog answered with something that is not a query result",
        )
    })?;
    let rows = v["results"].as_array().cloned().unwrap_or_default();
    Ok(rows
        .into_iter()
        .map(|r| r.as_array().cloned().unwrap_or_default())
        .collect())
}

fn int(v: &JsonValue) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

fn text(v: &JsonValue) -> String {
    v.as_str().unwrap_or_default().to_string()
}

// ---------- storage ----------

#[derive(sqlx::FromRow)]
struct Row {
    host: String,
    posthog_project: String,
    api_key_enc: Vec<u8>,
    conversion_event: Option<String>,
    default_locale: Option<String>,
    backfill_days: i32,
    synced_from: Option<NaiveDate>,
    synced_to: Option<NaiveDate>,
    last_sync_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

const COLS: &str = "host, posthog_project, api_key_enc, conversion_event, default_locale, backfill_days, synced_from, \
                    synced_to, last_sync_at, last_error";

async fn load(state: &AppState, project_id: Uuid) -> ApiResult<Option<Row>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM autopilot_posthog WHERE project_id = $1"
    ))
    .bind(project_id)
    .fetch_optional(&state.db)
    .await?)
}

fn conn(state: &AppState, r: &Row) -> ApiResult<Conn> {
    Ok(Conn {
        host: r.host.clone(),
        project: r.posthog_project.clone(),
        key: crate::auth::decrypt(&state.secret, &r.api_key_enc).ok_or_else(|| {
            AppError::internal("cannot decrypt the PostHog key (GLOT_SECRET_KEY changed?)")
        })?,
    })
}

/// Shown in the dashboard. Never carries the key.
#[derive(Serialize, Default)]
pub struct View {
    pub connected: bool,
    pub host: Option<String>,
    pub project_id: Option<String>,
    pub conversion_event: Option<String>,
    pub default_locale: Option<String>,
    pub backfill_days: i32,
    /// Days stored, first and last.
    pub synced_from: Option<NaiveDate>,
    pub synced_to: Option<NaiveDate>,
    pub last_sync_at: Option<DateTime<Utc>>,
    /// Why the last sync failed, in plain words.
    pub last_error: Option<String>,
}

pub async fn status(state: &AppState, project_id: Uuid) -> ApiResult<View> {
    Ok(match load(state, project_id).await? {
        None => View {
            backfill_days: 90,
            ..Default::default()
        },
        Some(r) => View {
            connected: true,
            host: Some(r.host),
            project_id: Some(r.posthog_project),
            conversion_event: r.conversion_event,
            default_locale: r.default_locale,
            backfill_days: r.backfill_days,
            synced_from: r.synced_from,
            synced_to: r.synced_to,
            last_sync_at: r.last_sync_at,
            last_error: r.last_error,
        },
    })
}

// ---------- sync ----------

#[derive(Serialize, Debug, PartialEq)]
pub struct Synced {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub queries: usize,
    pub rows: usize,
}

/// One aggregate to store: (day, preferred, served, country, event, n).
type Agg = (NaiveDate, String, String, String, &'static str, i64);

/// Project locale for a lowercased code returned by PostHog (`zh-cn` → `zh-CN`), '' when unknown.
fn served_locale(code: &str, locales: &[String]) -> String {
    locales
        .iter()
        .find(|l| l.to_ascii_lowercase() == code)
        .cloned()
        .unwrap_or_default()
}

/// Fetches [from, today] and replaces the stored rows of those days.
pub async fn sync(state: &AppState, project_id: Uuid) -> ApiResult<Synced> {
    let r = load(state, project_id)
        .await?
        .ok_or_else(|| AppError::not_found("PostHog is not connected"))?;
    let slug: String = sqlx::query_scalar("SELECT slug FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_one(&state.db)
        .await?;
    let p: ProjectRow = projects::load_public(state, &slug)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))?;
    let result = fetch(state, &r, &p).await;
    match &result {
        Ok(s) => {
            sqlx::query(
                "UPDATE autopilot_posthog SET synced_from = least(coalesce(synced_from, $2), $2), synced_to = $3,
                   last_sync_at = now(), last_error = NULL WHERE project_id = $1",
            )
            .bind(project_id)
            .bind(s.from)
            .bind(s.to)
            .execute(&state.db)
            .await?;
        }
        Err(e) => {
            sqlx::query(
                "UPDATE autopilot_posthog SET last_sync_at = now(), last_error = $2 WHERE project_id = $1",
            )
            .bind(project_id)
            .bind(&e.message)
            .execute(&state.db)
            .await?;
        }
    }
    result
}

async fn fetch(state: &AppState, r: &Row, p: &ProjectRow) -> ApiResult<Synced> {
    let c = conn(state, r)?;
    let today = Utc::now().date_naive();
    let from = r
        .synced_to
        .unwrap_or(today - Duration::days(r.backfill_days as i64 - 1));
    let locales: Vec<String> = std::iter::once(p.source_locale.clone())
        .chain(p.target_locales.iter().cloned())
        .collect();
    let lower: Vec<String> = locales.iter().map(|l| l.to_ascii_lowercase()).collect();
    let default = r
        .default_locale
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let (mut aggs, mut queries): (Vec<Agg>, usize) = (vec![], 0);
    let mut a = from;
    while a <= today {
        let b = (a + Duration::days(WINDOW_DAYS)).min(today + Duration::days(1));
        let rows = query(&c, &pageviews_query(a, b, &lower, &default)).await?;
        queries += 1;
        if rows.len() >= LIMIT {
            return Err(too_many(a, b));
        }
        for x in &rows {
            let Some(day) = x.first().and_then(|d| d.as_str()?.get(..10)?.parse().ok()) else {
                continue;
            };
            let key = |i: usize| x.get(i).map(text).unwrap_or_default();
            let (pref, served, country) = (key(1), served_locale(&key(2), &locales), key(3));
            for (i, event) in [
                (4, "$view"),
                (5, "$pageview"),
                (6, "$bounce"),
                (7, "$visitor"),
            ] {
                let n = x.get(i).map(int).unwrap_or(0);
                if n > 0 {
                    aggs.push((day, pref.clone(), served.clone(), country.clone(), event, n));
                }
            }
        }
        if let Some(event) = &r.conversion_event {
            let rows = query(&c, &conversions_query(event, a, b, &lower, &default)).await?;
            queries += 1;
            if rows.len() >= LIMIT {
                return Err(too_many(a, b));
            }
            for x in &rows {
                let Some(day) = x.first().and_then(|d| d.as_str()?.get(..10)?.parse().ok()) else {
                    continue;
                };
                let key = |i: usize| x.get(i).map(text).unwrap_or_default();
                let n = x.get(4).map(int).unwrap_or(0);
                if n > 0 {
                    aggs.push((
                        day,
                        key(1),
                        served_locale(&key(2), &locales),
                        key(3),
                        CONVERSION,
                        n,
                    ));
                }
            }
        }
        a = b;
    }
    // Languages are stored like the glot tag stores them ('' → 'und'); one transaction per sync.
    let mut tx = state.db.begin().await?;
    sqlx::query(&format!(
        "DELETE FROM {TABLE} WHERE project_id = $1 AND day >= $2"
    ))
    .bind(p.id)
    .bind(from)
    .execute(&mut *tx)
    .await?;
    let n = aggs.len();
    let mut cols: (
        Vec<NaiveDate>,
        Vec<String>,
        Vec<String>,
        Vec<String>,
        Vec<String>,
        Vec<i64>,
    ) = Default::default();
    for (day, pref, served, country, event, n) in aggs {
        let pref = if pref.is_empty() || validate_locale(&pref).is_err() {
            "und".to_string()
        } else {
            lang(&pref)
        };
        let country = if country.len() == 2 && country.bytes().all(|b| b.is_ascii_alphabetic()) {
            country
        } else {
            String::new()
        };
        cols.0.push(day);
        cols.1.push(pref);
        cols.2.push(served);
        cols.3.push(country);
        cols.4.push(event.to_string());
        cols.5.push(n);
    }
    // Several PostHog rows can collapse onto one key (an invalid language becomes 'und'): sum them.
    sqlx::query(&format!(
        "INSERT INTO {TABLE} (project_id, day, preferred, served, country, event, n)
         SELECT $1, d, p, s, c, e, sum(n) FROM UNNEST($2::date[], $3::text[], $4::text[], $5::text[], $6::text[],
           $7::int8[]) AS t(d, p, s, c, e, n) GROUP BY d, p, s, c, e"
    ))
    .bind(p.id)
    .bind(&cols.0)
    .bind(&cols.1)
    .bind(&cols.2)
    .bind(&cols.3)
    .bind(&cols.4)
    .bind(&cols.5)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Synced {
        from,
        to: today,
        queries,
        rows: n,
    })
}

/// Event name the conversions are stored under: the engine reads it as the goal when the source is PostHog.
pub const CONVERSION: &str = "$conversion";

fn too_many(a: NaiveDate, b: NaiveDate) -> AppError {
    AppError::new(
        StatusCode::BAD_GATEWAY,
        "posthog_too_many_groups",
        format!(
            "PostHog returned {LIMIT} groups or more for {a} → {b}: the import stopped rather than store a partial \
             week. This should not happen with browser languages and countries; contact us."
        ),
    )
}

/// Sources whose last sync is older than `RESYNC_HOURS` (or never ran).
pub async fn tick(state: &AppState) -> ApiResult<()> {
    let due: Vec<Uuid> = sqlx::query_scalar(
        "SELECT project_id FROM autopilot_posthog
         WHERE last_sync_at IS NULL OR last_sync_at < now() - make_interval(hours => $1)",
    )
    .bind(RESYNC_HOURS as i32)
    .fetch_all(&state.db)
    .await?;
    for id in due {
        // The error is stored on the source (and shown in the dashboard); logged without any request detail.
        if let Err(e) = sync(state, id).await {
            tracing::warn!("autopilot posthog sync {id}: {}", e.code);
        }
    }
    Ok(())
}

// ---------- handlers ----------

pub async fn view(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<View>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(status(&state, p.id).await?))
}

#[derive(Deserialize)]
pub struct Connect {
    /// us | eu | https URL of a self-hosted instance (default: the stored one).
    host: Option<String>,
    project_id: Option<String>,
    /// Personal API key (phx_…); omitted = keep the stored one.
    api_key: Option<String>,
    #[serde(default)]
    conversion_event: Option<String>,
    #[serde(default)]
    default_locale: Option<String>,
    backfill_days: Option<i32>,
}

/// The connection to use: the body's fields, else the stored ones. Refuses obvious mistakes before any request.
async fn resolve(state: &AppState, project_id: Uuid, b: &Connect) -> ApiResult<Conn> {
    let stored = load(state, project_id).await?;
    let host = match (&b.host, &stored) {
        (Some(h), _) => api_host(h).map_err(AppError::bad_request)?,
        (None, Some(r)) => r.host.clone(),
        (None, None) => {
            return Err(AppError::bad_request(
                "choose your PostHog region (US or EU) or instance URL",
            ));
        }
    };
    let project = match (&b.project_id, &stored) {
        (Some(p), _) => p.trim().to_string(),
        (None, Some(r)) => r.posthog_project.clone(),
        (None, None) => {
            return Err(AppError::bad_request(
                "paste your PostHog project ID (Project settings → Project ID)",
            ));
        }
    };
    if !(1..=64).contains(&project.len())
        || !project
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(AppError::bad_request(
            "the PostHog project ID is the number shown in Project settings (e.g. 12345)",
        ));
    }
    let key = match (&b.api_key, &stored) {
        (Some(k), _) => k.trim().to_string(),
        (None, Some(r)) => conn(state, r)?.key,
        (None, None) => {
            return Err(AppError::bad_request(
                "paste a PostHog personal API key (phx_…)",
            ));
        }
    };
    if key.starts_with("phc_") {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "posthog_project_key",
            "This is the project API key (phc_…): it can only send events. Create a personal API key (phx_…) with \
             the Query Read scope in PostHog, Settings → Personal API keys.",
        ));
    }
    if key.len() < 8 || key.len() > 200 || key.chars().any(char::is_whitespace) {
        return Err(AppError::bad_request(
            "this does not look like a PostHog personal API key (phx_…)",
        ));
    }
    Ok(Conn { host, project, key })
}

#[derive(Serialize)]
pub struct Tested {
    /// The project's most frequent event names over 30 days, with their counts.
    pub events: Vec<(String, i64)>,
    pub suggested_event: Option<String>,
    pub pageviews_30d: i64,
    /// Pageviews carrying a browser language (posthog-js sets it on every web event).
    pub with_language_30d: i64,
}

async fn run_test(c: &Conn) -> ApiResult<Tested> {
    let events: Vec<(String, i64)> = query(c, EVENTS_QUERY)
        .await?
        .iter()
        .map(|r| {
            (
                r.first().map(text).unwrap_or_default(),
                r.get(1).map(int).unwrap_or(0),
            )
        })
        .filter(|(e, _)| !e.is_empty())
        .collect();
    let cov = query(c, COVERAGE_QUERY).await?;
    let first = cov.first();
    Ok(Tested {
        suggested_event: suggest_event(&events),
        pageviews_30d: first.and_then(|r| r.first()).map(int).unwrap_or(0),
        with_language_30d: first.and_then(|r| r.get(1)).map(int).unwrap_or(0),
        events,
    })
}

/// "Test connection": the body's fields (or the stored ones), nothing saved.
pub async fn test(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Connect>,
) -> ApiResult<Json<Tested>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let c = resolve(&state, p.id, &b).await?;
    Ok(Json(run_test(&c).await?))
}

/// Connects (or updates) the source after a successful test. The first sync is started by `POST …/sync` (the
/// dashboard does it right away) or by the worker.
pub async fn save(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Connect>,
) -> ApiResult<Json<View>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let c = resolve(&state, p.id, &b).await?;
    let backfill = b.backfill_days.unwrap_or(90);
    if !(7..=365).contains(&backfill) {
        return Err(AppError::bad_request("backfill_days must be 7-365"));
    }
    let event = b
        .conversion_event
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(String::from);
    if event
        .as_deref()
        .is_some_and(|e| e.len() > 200 || e.chars().any(char::is_control))
    {
        return Err(AppError::bad_request("invalid conversion event name"));
    }
    let default_locale = b
        .default_locale
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from);
    if let Some(l) = &default_locale {
        validate_locale(l)?;
    }
    // Nothing is stored until PostHog accepted the key on this project.
    run_test(&c).await?;
    let changed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM autopilot_posthog WHERE project_id = $1 AND (host <> $2 OR posthog_project <> $3
           OR conversion_event IS DISTINCT FROM $4 OR default_locale IS DISTINCT FROM $5 OR backfill_days <> $6))",
    )
    .bind(p.id)
    .bind(&c.host)
    .bind(&c.project)
    .bind(&event)
    .bind(&default_locale)
    .bind(backfill)
    .fetch_one(&state.db)
    .await?;
    let mut tx = state.db.begin().await?;
    if changed {
        // Other project, event or locale mapping: the stored history no longer matches it, start over.
        sqlx::query(&format!("DELETE FROM {TABLE} WHERE project_id = $1"))
            .bind(p.id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO autopilot_posthog (project_id, host, posthog_project, api_key_enc, conversion_event,
           default_locale, backfill_days)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (project_id) DO UPDATE SET host = $2, posthog_project = $3, api_key_enc = $4,
           conversion_event = $5, default_locale = $6, backfill_days = $7, last_error = NULL, updated_at = now(),
           synced_from = CASE WHEN $8 THEN NULL ELSE autopilot_posthog.synced_from END,
           synced_to = CASE WHEN $8 THEN NULL ELSE autopilot_posthog.synced_to END",
    )
    .bind(p.id)
    .bind(&c.host)
    .bind(&c.project)
    .bind(crate::auth::encrypt(&state.secret, &c.key))
    .bind(&event)
    .bind(&default_locale)
    .bind(backfill)
    .bind(changed)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(status(&state, p.id).await?))
}

pub async fn sync_now(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Synced>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    Ok(Json(sync(&state, p.id).await?))
}

/// Forgets the key and every imported counter.
pub async fn disconnect(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query(&format!("DELETE FROM {TABLE} WHERE project_id = $1"))
        .bind(p.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM autopilot_posthog WHERE project_id = $1")
        .bind(p.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(api_host("US").unwrap(), "https://us.posthog.com");
        assert_eq!(
            api_host("https://eu.i.posthog.com/").unwrap(),
            "https://eu.posthog.com"
        );
        assert_eq!(
            api_host("app.posthog.com").unwrap(),
            "https://us.posthog.com"
        );
        assert_eq!(
            api_host("https://ph.example.com").unwrap(),
            "https://ph.example.com"
        );
        assert!(api_host("ftp://ph.example.com").is_err());
        assert!(api_host("https://ph.example.com/?x=1").is_err());
    }

    #[test]
    fn queries_only_ask_for_aggregates() {
        let d = |s: &str| s.parse::<NaiveDate>().unwrap();
        let locales = ["fr".to_string(), "zh-cn".to_string()];
        for q in [
            pageviews_query(d("2026-11-01"), d("2026-11-08"), &locales, "fr"),
            conversions_query(
                "Signed up's",
                d("2026-11-01"),
                d("2026-11-08"),
                &locales,
                "",
            ),
        ] {
            // What comes back: grouping keys and counts, nothing else.
            assert!(
                q.starts_with("SELECT day, lang, served, country, count() AS sessions"),
                "{q}"
            );
            let outer = &q[q.rfind(')').unwrap()..];
            assert!(
                outer.contains("GROUP BY day, lang, served, country")
                    && outer.contains("LIMIT 50000")
            );
            for banned in [
                "$ip",
                "distinct_id",
                "$current_url",
                "$referrer",
                "elements",
                "person.properties",
                "SELECT *",
            ] {
                assert!(!q.contains(banned), "{banned} in {q}");
            }
            assert!(q.contains("timestamp >= toDateTime('2026-11-01 00:00:00') AND timestamp < toDateTime('2026-11-08 00:00:00')"));
        }
        let c = conversions_query(
            "Signed up's",
            d("2026-11-01"),
            d("2026-11-08"),
            &locales,
            "",
        );
        assert!(c.contains("event = 'Signed up\\'s'"), "escaped: {c}");
        assert!(served_expr(&locales, "fr").contains("IN ('fr', 'zh-cn')"));
        assert_eq!(served_expr(&[], ""), "''");
    }

    #[test]
    fn suggestions_and_errors() {
        let ev = |v: &[&str]| v.iter().map(|e| (e.to_string(), 1)).collect::<Vec<_>>();
        assert_eq!(
            suggest_event(&ev(&["$pageview", "checkout started", "user signed up"])).as_deref(),
            Some("user signed up")
        );
        assert_eq!(
            suggest_event(&ev(&["$pageview", "Order completed"])).as_deref(),
            Some("Order completed")
        );
        assert_eq!(suggest_event(&ev(&["$pageview", "$autocapture"])), None);
        assert_eq!(explain(401, "{}", "h", "1").code, "posthog_unauthorized");
        let scope = r#"{"type":"authentication_error","code":"permission_denied","detail":"API key missing required scope 'query:read'"}"#;
        assert_eq!(explain(403, scope, "h", "1").code, "posthog_scope");
        assert_eq!(
            explain(403, r#"{"detail":"no access"}"#, "h", "1").code,
            "posthog_forbidden"
        );
        assert_eq!(explain(404, "", "h", "77").code, "posthog_project");
        assert_eq!(explain(429, "", "h", "1").code, "posthog_rate_limited");
        assert_eq!(
            served_locale("zh-cn", &["fr".into(), "zh-CN".into()]),
            "zh-CN"
        );
        assert_eq!(served_locale("xx", &["fr".into()]), "");
    }
}
