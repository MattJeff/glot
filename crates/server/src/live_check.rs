//! "Is this language really live on my site?" A language can be 100 % translated in glot and still never show:
//! the site's own code ignores the locale code glot gives it (`?lang=zh-CN` on a site that only accepts `zh`).
//! Two signals, one verdict per target locale:
//! - **Visitors** (`POST /cdn/{slug}/live-check`): after `<glot-locale-switcher>` sends a visitor to a language,
//!   the next page load reports whether the page changed. Counters per (day, locale); no IP, user agent, URL
//!   path, cookie or visitor id. Capped per day, origin checked against the project's site when one is set.
//! - **Probe** (`POST /api/projects/{slug}/live-check`, and after each translation job): the site's home page is
//!   fetched for the locale and for a code no site knows (`xx`: what the site shows when it ignores the code),
//!   with the SSRF guards of the Review probe. Different page → honored. Same page → the usual spellings
//!   (`zh`, `zh-cn`, `zh_CN`, `zh-Hans`) are tried; one that changes the page is the site's code for the locale.
//!   Same page for all of them proves nothing (client-rendered sites answer the same HTML): `unknown`, and the
//!   visitors decide.
//!
//! The fix is the per-project site code map (`projects.site_codes`, `{"zh-CN": "zh"}`): published in
//! `locales.json` as `site_code`, written by the switcher instead of glot's code. Applied automatically when the
//! probe finds a spelling whose page declares the language in `<html lang>`, proposed otherwise.
// ponytail: the headless renderer (GLOT_RENDERER_URL) is not used: optional in production, and the visitor
// signal already covers client-rendered sites. Wire it into `fetch_page` if probes must settle those alone.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    locale_alias,
    projects::{self, ProjectRow, Role},
    review_live::probe::{attr, body, fetch_with},
};

/// A code no site translates: its page is what the site shows when it ignores the locale.
const CONTROL: &str = "xx";
/// Share of visible text that must differ from the control page to call a page "another language".
const MIN_CHANGED: f64 = 0.1;
/// Visitor reports counted per (project, locale, day).
const DAILY_CAP: i64 = 1000;
const DAYS: i32 = 7;

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}";
    Router::new()
        .route(&format!("{p}/live-check"), get(state_of).post(run))
        .route(&format!("{p}/site-codes"), put(put_site_codes))
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/cdn/{slug}/live-check", post(beacon))
        .layer(DefaultBodyLimit::max(2048))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

// ---------- site codes ----------

/// glot locale → the code the site uses for it.
pub type SiteCodes = BTreeMap<String, String>;

pub async fn site_codes(db: &sqlx::PgPool, project: Uuid) -> sqlx::Result<SiteCodes> {
    let v: sqlx::types::Json<SiteCodes> =
        sqlx::query_scalar("SELECT site_codes FROM projects WHERE id = $1")
            .bind(project)
            .fetch_one(db)
            .await?;
    Ok(v.0)
}

async fn save_site_codes(db: &sqlx::PgPool, project: Uuid, codes: &SiteCodes) -> sqlx::Result<()> {
    sqlx::query("UPDATE projects SET site_codes = $2 WHERE id = $1")
        .bind(project)
        .bind(sqlx::types::Json(codes))
        .execute(db)
        .await?;
    Ok(())
}

/// What was learned about a locale under its previous code no longer applies.
async fn forget(db: &sqlx::PgPool, project: Uuid, locales: &[String]) -> sqlx::Result<()> {
    for table in ["live_check_beacons", "live_check_probes"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE project_id = $1 AND locale = ANY($2)"
        ))
        .bind(project)
        .bind(locales)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Spellings of `locale` the site may accept instead of `current`, without the codes another locale uses.
fn candidates(p: &ProjectRow, codes: &SiteCodes, locale: &str, current: &str) -> Vec<String> {
    let taken: Vec<String> = std::iter::once(&p.source_locale)
        .chain(&p.target_locales)
        .filter(|l| *l != locale)
        .map(|l| codes.get(l).unwrap_or(l).to_ascii_lowercase())
        .collect();
    let mut out = locale_alias::variants(locale);
    out.insert(0, locale.to_string());
    out.retain(|v| v != current && !taken.contains(&v.to_ascii_lowercase()));
    out
}

/// `PUT …/site-codes` `{"zh-CN": "zh"}`: replaces the map. Each locale keeps a distinct code.
async fn put_site_codes(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(new): AppJson<SiteCodes>,
) -> ApiResult<Json<SiteCodes>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let locales: Vec<&String> = std::iter::once(&p.source_locale)
        .chain(&p.target_locales)
        .collect();
    let mut used = HashSet::new();
    for &l in &locales {
        let code = new.get(l).unwrap_or(l);
        let ok = (1..=35).contains(&code.len())
            && code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(AppError::bad_request(format!(
                "site code for '{l}' must be 1-35 letters, digits, '-' or '_'"
            )));
        }
        if !used.insert(code.to_ascii_lowercase()) {
            return Err(AppError::bad_request(format!(
                "site code '{code}' is used by two locales"
            )));
        }
    }
    if let Some(k) = new.keys().find(|k| !locales.contains(k)) {
        return Err(AppError::bad_request(format!(
            "'{k}' is not a locale of this project"
        )));
    }
    let old = site_codes(&state.db, p.id).await?;
    let changed: Vec<String> = locales
        .iter()
        .copied()
        .filter(|&l| old.get(l).unwrap_or(l) != new.get(l).unwrap_or(l))
        .map(|l| l.to_string())
        .collect();
    save_site_codes(&state.db, p.id, &new).await?;
    if !changed.is_empty() {
        forget(&state.db, p.id, &changed).await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = check(&state, &p, Some(changed)).await {
                tracing::warn!("live check failed: {}", e.message);
            }
        });
    }
    Ok(Json(new))
}

// ---------- visitors ----------

#[derive(Deserialize)]
struct Beacon {
    /// The glot locale the visitor picked.
    locale: String,
    /// The code the switcher wrote (site code); a report made with an outdated one is dropped.
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    html_lang: String,
    applied: bool,
    #[serde(default)]
    origin: String,
}

/// `localhost`, IPs and single-label hosts are development URLs, not the public site.
fn public_host(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let h = u
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    (h.contains('.') && h.parse::<std::net::IpAddr>().is_err() && !h.starts_with('[')).then_some(h)
}

async fn beacon(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    // Sent with `navigator.sendBeacon` as text/plain (no CORS preflight): parsed whatever the content type.
    let b: Beacon = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    let p = projects::load_public(&state, &slug)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))?;
    let done = || Ok(StatusCode::NO_CONTENT.into_response());
    if crate::experiments::ingest::is_bot(&headers) {
        return done();
    }
    let Some(locale) = locale_alias::resolve(&b.locale, &p.target_locales).map(str::to_string)
    else {
        return done();
    };
    let codes = site_codes(&state.db, p.id).await?;
    if b.code
        .as_ref()
        .is_some_and(|c| c != codes.get(&locale).unwrap_or(&locale))
    {
        return done();
    }
    // The browser's own Origin header when there is one; the page-reported origin otherwise.
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(&b.origin);
    let origin = Url::parse(origin)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    let known: Vec<String> = sqlx::query_scalar(
        "SELECT base_url FROM preview_settings WHERE project_id = $1 AND base_url IS NOT NULL
         UNION SELECT 'https://' || host FROM edge_domains WHERE project_id = $1
         UNION SELECT origin_url FROM edge_domains WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let known: Vec<String> = known.iter().filter_map(|u| public_host(u)).collect();
    if !known.is_empty() && !public_host(&origin).is_some_and(|h| known.contains(&h)) {
        return done();
    }
    let lang: String = b.html_lang.chars().take(35).collect();
    let lang = if lang
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        lang
    } else {
        String::new()
    };
    // ponytail: a daily cap per locale is the rate limit (one cheap upsert per report, a no-op past the cap);
    // add a per-minute window like edge_missing_rate if this endpoint is ever hammered.
    sqlx::query(
        "INSERT INTO live_check_beacons AS b (project_id, locale, day, applied, ignored, html_lang, origin)
         VALUES ($1, $2, (now() AT TIME ZONE 'utc')::date, $3, $4, $5, $6)
         ON CONFLICT (project_id, locale, day) DO UPDATE SET
           applied = b.applied + excluded.applied, ignored = b.ignored + excluded.ignored,
           html_lang = CASE WHEN excluded.ignored > 0 THEN excluded.html_lang ELSE b.html_lang END,
           origin = excluded.origin
         WHERE b.applied + b.ignored < $7",
    )
    .bind(p.id)
    .bind(&locale)
    .bind(i64::from(b.applied))
    .bind(i64::from(!b.applied))
    .bind(if b.applied { "" } else { lang.as_str() })
    .bind(&origin)
    .bind(DAILY_CAP)
    .execute(&state.db)
    .await?;
    done()
}

// ---------- probe ----------

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Probe {
    /// `honored` (the page changes with this code), `ignored` (it does not, and `works_with` does),
    /// `unknown` (same HTML whatever the code, or an HTTP error: cannot tell), `unreachable`.
    pub verdict: String,
    /// The code that was sent to the site, and the URL fetched for it.
    pub code: String,
    pub url: String,
    pub status: Option<u16>,
    pub html_lang: Option<String>,
    pub title: Option<String>,
    /// Share of the page's visible text that differs from what the site shows for an unknown code.
    pub changed: Option<f64>,
    /// A spelling the site honors while it ignores `code`.
    pub works_with: Option<String>,
    /// `private`, `timeout`, `unreachable` (fetch refused or failed).
    pub error: Option<String>,
    /// `works_with` was saved as the locale's site code by glot (the page declared the language).
    #[serde(default)]
    pub auto_applied: bool,
    /// `works_with`'s page declares the locale in `<html lang>`.
    #[serde(skip)]
    strong: bool,
}

#[derive(Debug, Default, Clone)]
struct Page {
    status: u16,
    lang: Option<String>,
    title: Option<String>,
    texts: HashSet<String>,
}

/// `<html lang>`, `<title>` and the visible text nodes (scripts, styles, comments skipped).
fn parse_page(html: &str) -> Page {
    let lower = html.to_ascii_lowercase(); // same byte offsets as `html`
    let mut page = Page::default();
    let mut title_next = false;
    let mut i = 0;
    while i < html.len() {
        let lt = lower[i..].find('<').map_or(html.len(), |j| i + j);
        let text = html[i..lt].split_whitespace().collect::<Vec<_>>().join(" ");
        if text.chars().any(char::is_alphabetic) {
            if title_next {
                page.title = Some(text.clone());
            }
            page.texts.insert(text);
        }
        if lt == html.len() {
            break;
        }
        let rest = &lower[lt..];
        let raw = ["script", "style", "noscript", "template"]
            .iter()
            .find(|t| rest[1..].starts_with(**t));
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|e| e + 3)
        } else if let Some(tag) = raw {
            rest.find(&format!("</{tag}"))
                .and_then(|c| rest[c..].find('>').map(|e| c + e + 1))
        } else {
            rest.find('>').map(|e| e + 1)
        };
        let Some(end) = end else { break };
        if rest.starts_with("<html") && page.lang.is_none() {
            page.lang = attr(&html[lt..lt + end], "lang").filter(|l| !l.is_empty());
        }
        title_next = rest.starts_with("<title");
        i = lt + end;
    }
    page
}

fn changed(page: &Page, reference: &Page) -> f64 {
    if page.texts.is_empty() {
        return 0.0;
    }
    let new = page.texts.difference(&reference.texts).count();
    new as f64 / page.texts.len() as f64
}

/// The page is not the one the site serves when it ignores the code.
/// The page shows something else than every reference page: what the site serves for a code it cannot know,
/// and its source-language page. Both are needed: cliponit.app keeps French for a code its guard rejects
/// (`zh-CN`) but falls back to English for an unknown short code (`xx`).
fn shows_language(page: &Page, refs: &[Page]) -> bool {
    refs.iter().all(|r| differs(page, r))
}

fn differs(page: &Page, control: &Page) -> bool {
    let lang = |p: &Page| p.lang.as_ref().map(|l| l.to_ascii_lowercase());
    page.status < 400
        && (control.status >= 400
            || (lang(page).is_some() && lang(page) != lang(control))
            || changed(page, control) >= MIN_CHANGED)
}

/// Home page URL for `code`, like the dashboard's `buildUrl` (web/lib/visual.ts).
fn build_url(base: &str, pattern: &str, code: &str) -> Option<Url> {
    let mut path = pattern.replace("{locale}", code).replace("{route}", "/");
    while path.contains("//") {
        path = path.replace("//", "/");
    }
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    if path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    Url::parse(&format!("{}{path}", base.trim_end_matches('/'))).ok()
}

/// One page, redirects followed by hand (each hop re-checked: public addresses only, pinned), capped body.
async fn fetch_page(mut url: Url) -> Result<Page, &'static str> {
    for _ in 0..=3 {
        let resp = fetch_with(
            &url,
            crate::edge::sitemap::allow_private(),
            "glot-live-check/1",
        )
        .await?;
        if resp.status().is_redirection()
            && let Some(next) = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .and_then(|l| url.join(l).ok())
        {
            url = next;
            continue;
        }
        let status = resp.status().as_u16();
        let mut page = parse_page(&body(resp).await);
        page.status = status;
        return Ok(page);
    }
    Err("unreachable")
}

async fn probe_locale(
    base: &str,
    pattern: &str,
    locale: &str,
    code: &str,
    candidates: &[String],
    refs: &[Page],
) -> Probe {
    let mut out = Probe {
        verdict: "unknown".into(),
        code: code.to_string(),
        ..Default::default()
    };
    let fail = |mut out: Probe, e: &str| {
        out.verdict = "unreachable".into();
        out.error = Some(e.to_string());
        out
    };
    let Some(url) = build_url(base, pattern, code) else {
        return fail(out, "unreachable");
    };
    out.url = url.to_string();
    let page = match fetch_page(url).await {
        Ok(p) => p,
        Err(e) => return fail(out, e),
    };
    out.status = Some(page.status);
    out.changed = Some((changed(&page, &refs[0]) * 1000.0).round() / 1000.0);
    let honored = shows_language(&page, refs);
    out.html_lang = page.lang;
    out.title = page.title;
    if honored {
        out.verdict = "honored".into();
        return out;
    }
    let only = [locale.to_string()];
    for v in candidates.iter().take(5) {
        let Some(url) = build_url(base, pattern, v) else {
            continue;
        };
        if let Ok(p) = fetch_page(url).await
            && shows_language(&p, refs)
        {
            out.verdict = "ignored".into();
            out.works_with = Some(v.clone());
            out.strong = p
                .lang
                .is_some_and(|l| locale_alias::resolve(&l, &only).is_some());
            break;
        }
    }
    out
}

/// Probes the site for `only` (default: every target), stores the results, and saves a site code when the
/// evidence is unambiguous. Nothing to do without a site URL (Settings → Your site).
pub async fn check(state: &AppState, p: &ProjectRow, only: Option<Vec<String>>) -> ApiResult<()> {
    let site: Option<(String, String)> = sqlx::query_as(
        "SELECT base_url, url_pattern FROM preview_settings WHERE project_id = $1 AND base_url IS NOT NULL",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    let Some((base, pattern)) = site else {
        return Ok(());
    };
    let targets: Vec<String> = p
        .target_locales
        .iter()
        .filter(|l| only.as_ref().is_none_or(|o| o.contains(l)))
        .cloned()
        .collect();
    if targets.is_empty() {
        return Ok(());
    }
    let mut codes = site_codes(&state.db, p.id).await?;
    let kept_auto: Vec<String> = sqlx::query_scalar(
        "SELECT locale FROM live_check_probes WHERE project_id = $1 AND (result->>'auto_applied')::bool",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let control = match build_url(&base, &pattern, CONTROL) {
        Some(u) => fetch_page(u).await,
        None => Err("unreachable"),
    };
    let mut results: Vec<(String, Probe)> = Vec::new();
    match control {
        Err(e) => {
            for l in &targets {
                let probe = Probe {
                    verdict: "unreachable".into(),
                    code: codes.get(l).unwrap_or(l).clone(),
                    url: base.clone(),
                    error: Some(e.to_string()),
                    ..Default::default()
                };
                results.push((l.clone(), probe));
            }
        }
        Ok(control) => {
            let source = codes.get(&p.source_locale).unwrap_or(&p.source_locale);
            let mut refs = vec![control];
            if let Some(u) = build_url(&base, &pattern, source)
                && let Ok(page) = fetch_page(u).await
            {
                refs.push(page);
            }
            let control: Arc<Vec<Page>> = Arc::new(refs);
            let mut set = tokio::task::JoinSet::new();
            for l in targets {
                let code = codes.get(&l).unwrap_or(&l).clone();
                let cands = candidates(p, &codes, &l, &code);
                let (base, pattern, control) = (base.clone(), pattern.clone(), control.clone());
                set.spawn(async move {
                    let probe = probe_locale(&base, &pattern, &l, &code, &cands, &control).await;
                    (l, probe)
                });
            }
            while let Some(r) = set.join_next().await {
                results.push(r.map_err(AppError::internal)?);
            }
            results.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic when two locales want the same code
        }
    }
    for (locale, mut probe) in results {
        let free = |v: &str| !codes.values().any(|c| c.eq_ignore_ascii_case(v));
        match &probe.works_with {
            // The site declares the language under another spelling and nobody chose a code: use it.
            Some(v) if probe.strong && !codes.contains_key(&locale) && free(v) => {
                codes.insert(locale.clone(), v.clone());
                save_site_codes(&state.db, p.id, &codes).await?;
                forget(&state.db, p.id, std::slice::from_ref(&locale)).await?;
                probe.url = build_url(&base, &pattern, v).map_or(probe.url, |u| u.to_string());
                probe.code = v.clone();
                probe.verdict = "honored".into();
                probe.works_with = None;
                probe.auto_applied = true;
                tracing::info!(project = %p.slug, %locale, code = %probe.code, "site code applied automatically");
            }
            _ => {
                probe.auto_applied = kept_auto.contains(&locale)
                    && probe.verdict == "honored"
                    && codes.get(&locale) == Some(&probe.code);
            }
        }
        sqlx::query(
            "INSERT INTO live_check_probes (project_id, locale, result) VALUES ($1, $2, $3)
             ON CONFLICT (project_id, locale) DO UPDATE SET result = $3, checked_at = now()",
        )
        .bind(p.id)
        .bind(&locale)
        .bind(sqlx::types::Json(&probe))
        .execute(&state.db)
        .await?;
    }
    Ok(())
}

/// After a translation job: probes in the background the targets that are not known to be live (or whose
/// last probe is older than a day), so a language is checked as soon as it has something to show.
pub fn after_job(state: &AppState, project: Uuid) {
    let state = state.clone();
    tokio::spawn(async move {
        let run = async {
            let slug: String = sqlx::query_scalar("SELECT slug FROM projects WHERE id = $1")
                .bind(project)
                .fetch_one(&state.db)
                .await?;
            let Some(p) = projects::load_public(&state, &slug).await? else {
                return Ok(());
            };
            let fresh: Vec<String> = sqlx::query_scalar(
                "SELECT locale FROM live_check_probes WHERE project_id = $1
                 AND result->>'verdict' = 'honored' AND checked_at > now() - interval '1 day'",
            )
            .bind(project)
            .fetch_all(&state.db)
            .await?;
            let todo: Vec<String> = p
                .target_locales
                .iter()
                .filter(|l| !fresh.contains(l))
                .cloned()
                .collect();
            check(&state, &p, Some(todo)).await
        };
        let r: ApiResult<()> = run.await;
        if let Err(e) = r {
            tracing::warn!(%project, "live check failed: {}", e.message);
        }
    });
}

// ---------- state ----------

#[derive(Serialize, Default)]
struct Visitors {
    applied: i64,
    ignored: i64,
    /// `<html lang>` of the last page that ignored the language.
    html_lang: Option<String>,
    origin: Option<String>,
}

/// A spelling the probe saw the site honor. Never guessed: on cliponit.app `?lang=zh` falls back to English
/// like any unknown code, so "use zh" would not have shown Chinese.
#[derive(Serialize)]
struct Suggestion {
    code: String,
}

#[derive(Serialize)]
struct LocaleState {
    locale: String,
    /// The code the switcher writes for this locale (glot's own unless mapped).
    site_code: String,
    mapped: bool,
    auto_applied: bool,
    /// `live`, `not_displayed`, `unknown` (probed, cannot tell from the HTML: waiting for visitors),
    /// `unchecked` (not probed yet), `no_site` (no site URL and no visitor yet).
    state: &'static str,
    /// `visitors` or `probe`: where the state comes from.
    evidence: Option<&'static str>,
    suggestion: Option<Suggestion>,
    probe: Option<Probe>,
    checked_at: Option<DateTime<Utc>>,
    visitors: Visitors,
}

#[derive(Serialize)]
struct View {
    site_url: Option<String>,
    source: String,
    site_codes: SiteCodes,
    locales: Vec<LocaleState>,
}

async fn view(state: &AppState, p: &ProjectRow) -> ApiResult<View> {
    let site_url: Option<String> = sqlx::query_scalar(
        "SELECT base_url FROM preview_settings WHERE project_id = $1 AND base_url IS NOT NULL",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    let codes = site_codes(&state.db, p.id).await?;
    let probes: Vec<(String, sqlx::types::Json<Probe>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT locale, result, checked_at FROM live_check_probes WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    // Latest lang / origin first: `max` over (day, value) pairs keeps the most recent non-empty one.
    let seen: Vec<(String, i64, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT locale, sum(applied)::int8, sum(ignored)::int8,
                (array_agg(html_lang ORDER BY day DESC) FILTER (WHERE html_lang <> ''))[1],
                (array_agg(origin ORDER BY day DESC) FILTER (WHERE origin <> ''))[1]
         FROM live_check_beacons WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2
         GROUP BY locale",
    )
    .bind(p.id)
    .bind(DAYS)
    .fetch_all(&state.db)
    .await?;
    let locales = p
        .target_locales
        .iter()
        .map(|l| {
            let site_code = codes.get(l).unwrap_or(l).clone();
            let probe = probes.iter().find(|r| r.0 == *l);
            let visitors = seen
                .iter()
                .find(|r| r.0 == *l)
                .map(|(_, applied, ignored, html_lang, origin)| Visitors {
                    applied: *applied,
                    ignored: *ignored,
                    html_lang: html_lang.clone(),
                    origin: origin.clone(),
                })
                .unwrap_or_default();
            let verdict = probe.map(|r| r.1.0.verdict.as_str());
            // Real page loads outrank the HTML probe.
            let (state, evidence) = if visitors.applied + visitors.ignored > 0 {
                let live = visitors.applied >= visitors.ignored;
                (
                    if live { "live" } else { "not_displayed" },
                    Some("visitors"),
                )
            } else {
                match verdict {
                    Some("honored") => ("live", Some("probe")),
                    Some("ignored") => ("not_displayed", Some("probe")),
                    Some(_) => ("unknown", Some("probe")),
                    None if site_url.is_none() => ("no_site", None),
                    None => ("unchecked", None),
                }
            };
            // An alias is proposed only for a locale that still uses glot's own code.
            let suggestion = probe
                .and_then(|r| r.1.0.works_with.clone())
                .filter(|w| {
                    state == "not_displayed"
                        && site_code == *l
                        && candidates(p, &codes, l, &site_code).contains(w)
                })
                .map(|code| Suggestion { code });
            LocaleState {
                locale: l.clone(),
                mapped: site_code != *l,
                auto_applied: probe.is_some_and(|r| r.1.0.auto_applied) && site_code != *l,
                site_code,
                state,
                evidence,
                suggestion,
                probe: probe.map(|r| r.1.0.clone()),
                checked_at: probe.map(|r| r.2),
                visitors,
            }
        })
        .collect();
    Ok(View {
        site_url,
        source: p.source_locale.clone(),
        site_codes: codes,
        locales,
    })
}

/// `GET …/live-check`: per target locale, whether the site really shows it, why not, and the code to use.
async fn state_of(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<View>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(view(&state, &p).await?))
}

#[derive(Deserialize, Default)]
struct RunBody {
    locale: Option<String>,
}

/// `POST …/live-check` `{locale?}`: probes the site now (one locale or all) and returns the new state.
async fn run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    body: Bytes,
) -> ApiResult<Json<View>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let b: RunBody = if body.is_empty() {
        RunBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?
    };
    if let Some(l) = &b.locale
        && !p.target_locales.contains(l)
    {
        return Err(AppError::bad_request(
            "locale is not a target locale of this project",
        ));
    }
    check(&state, &p, b.locale.map(|l| vec![l])).await?;
    Ok(Json(view(&state, &p).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(status: u16, html: &str) -> Page {
        Page {
            status,
            ..parse_page(html)
        }
    }

    #[test]
    fn reads_lang_title_and_visible_text() {
        let p = parse_page(
            r#"<!doctype html><html data-x="1" lang="fr"><head><title>Bonjour le monde</title>
               <style>.a{color:red}</style><script>var lang = "fr"; if (a < b) {}</script></head>
               <body><!-- note --><h1>Crée tes vidéos</h1><p>Rapide   et
               simple</p><noscript>Activez JS</noscript><span>42</span></body></html>"#,
        );
        assert_eq!(p.lang.as_deref(), Some("fr"));
        assert_eq!(p.title.as_deref(), Some("Bonjour le monde"));
        let mut texts: Vec<&str> = p.texts.iter().map(String::as_str).collect();
        texts.sort();
        assert_eq!(
            texts,
            ["Bonjour le monde", "Crée tes vidéos", "Rapide et simple"]
        );
    }

    #[test]
    fn honored_only_when_the_page_differs_from_the_fallback() {
        let fr = r#"<html lang="fr"><title>Accueil</title><h1>Bonjour</h1><p>Tarifs</p>"#;
        let control = page(200, fr);
        // Same HTML (code ignored, or a client-rendered site): never "honored".
        assert!(!differs(&page(200, fr), &control));
        // Another language attribute, or enough of the text.
        assert!(differs(
            &page(200, &fr.replace("\"fr\"", "\"zh\"")),
            &control
        ));
        let zh = r#"<html><title>首页</title><h1>你好</h1><p>价格</p>"#;
        assert!(differs(&page(200, zh), &control));
        assert_eq!(changed(&page(200, zh), &control), 1.0);
        // An error page is not the language; a real page where the unknown code is a 404 is.
        assert!(!differs(&page(404, zh), &control));
        assert!(differs(&page(200, fr), &page(404, "<h1>Not found</h1>")));
        // cliponit.app: zh-CN rejected → French (the source); unknown `xx` → English. Not "honored".
        let en = page(
            200,
            r#"<html lang="en"><title>Home</title><h1>Hello</h1><p>Pricing</p>"#,
        );
        assert!(differs(&page(200, fr), &en));
        assert!(!shows_language(
            &page(200, fr),
            &[en.clone(), page(200, fr)]
        ));
        assert!(shows_language(&page(200, zh), &[en, page(200, fr)]));
    }

    #[test]
    fn urls_follow_the_preview_pattern() {
        let u = |pattern, code| {
            build_url("https://x.io/", pattern, code)
                .unwrap()
                .to_string()
        };
        assert_eq!(
            u("{route}?lang={locale}", "zh-CN"),
            "https://x.io/?lang=zh-CN"
        );
        assert_eq!(u("/{locale}{route}", "zh"), "https://x.io/zh");
        assert_eq!(u("/app/{locale}{route}", "zh"), "https://x.io/app/zh");
    }

    #[test]
    fn development_hosts_are_not_site_origins() {
        assert_eq!(
            public_host("https://www.cliponit.app/x").as_deref(),
            Some("cliponit.app")
        );
        for u in [
            "http://localhost:3000",
            "http://127.0.0.1:8080",
            "http://[::1]:1",
            "ftp://x.io",
            "",
        ] {
            assert_eq!(public_host(u), None, "{u}");
        }
    }
}
