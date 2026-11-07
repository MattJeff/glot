//! glot Edge, subdomain mode: `es.example.com CNAME edge.<glot domain>`. Caddy gets the certificate on demand
//! (`ask` = `/edge/ask`: verified hosts only) and forwards every request to `/edge/proxy{uri}`, Host kept. This
//! reverse-proxies to the domain's fixed origin and translates the HTML from the project's TM.
//!
//! - Never an open proxy: the Host must be a verified subdomain-mode `edge_domains` row; the origin is that row's
//!   `origin_url` (never taken from the request); origin addresses must be public (resolver below, pinned per
//!   connection), redirects are not followed (passed to the browser, origin host rewritten to the edge host);
//!   request bodies ≤ 10 MB, origin ≤ 20 s.
//! - Fail-open: glot's own part (domain/TM lookup, translation, SEO) has a `GLOT_BUDGET`; past it, on a DB error
//!   or a translation panic, the origin HTML is served untranslated (`x-glot-edge: fail-open`), never a 5xx due to
//!   glot. Last known domains and TMs are kept in memory and on disk (`GLOT_EDGE_CACHE_DIR`), so a Postgres outage
//!   keeps serving the last state. The glot server process itself down: Caddy serves the origin from the map
//!   `domains::caddy_sync` writes (deploy/Caddyfile `handle_errors`).
//! - One version per page: one `Arc<Tm>` snapshot per request; the page cache key contains the version, so a
//!   version bump is the purge. Origins that forbid caching (`no-cache`) are fetched every time, but an
//!   unchanged body reuses its translation (`TrCache`, keyed by the body's sha256 + domain + path + version).
//! - hreflang / localized links list only the locales ready for search engines (`sitemap::site(…, true)`);
//!   `/sitemap.xml`, `/robots.txt` and the IndexNow key file come from `sitemap::special`. Kill switch: project `edge_settings.enabled` / domain `enabled`, re-read every
//!   `DOMAIN_TTL` → passthrough in < 1 min.
//! - Origin headers are kept (CSP, HSTS, cookies…) except hop-by-hop and the ones the body change invalidates.
//!   `Server-Timing: glot;dur=…, origin;dur=…` exposes glot's cost.
//! - Application paths (`app_paths`: login, account, OAuth, API…): never translated, cached or discovered;
//!   `redirect` → 302/307 to the domain's own origin (`x-glot-edge: app-redirect`), `proxy` → the origin
//!   response untouched (`x-glot-edge: app`). Checked after glot's own routes (sitemap, robots, IndexNow key).
//! - Discovery: missing segments go to `missing::record` in the background, only for anonymous requests, paths
//!   without query string, deduplicated per process.
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use glot_core::edge::{
    hydration,
    segment::{self, SegmentOpts},
    seo::{self, SeoCtx},
    tm::{self as ctm, Tm},
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex, RwLock},
    time::{Duration, Instant},
};
use uuid::Uuid;

use super::app_paths;
use super::ask::clean_host;
use super::missing;
use crate::{AppState, projects::ProjectRow, review_live::probe::is_public};

/// glot's own budget per request (lookups + translation), past it: the origin page untranslated.
const GLOT_BUDGET: Duration = Duration::from_millis(400);
/// Domain rows (kill switch, origin) are re-read this often.
const DOMAIN_TTL: Duration = Duration::from_secs(15);
/// TM version re-checked this often (a translation edited in glot is live within it).
const VERSION_TTL: Duration = Duration::from_secs(3);
const ORIGIN_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REQUEST_BODY: usize = 10 * 1024 * 1024;
/// Larger HTML is streamed through untranslated.
const MAX_HTML: usize = 5 * 1024 * 1024;
const PAGE_CACHE_MAX: usize = 1000;

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/edge/proxy", any(proxy))
        .route("/edge/proxy/", any(proxy))
        .route("/edge/proxy/{*rest}", any(proxy))
        .route("/edge/patcher.js", get(patcher))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Domain {
    project_id: Uuid,
    slug: String,
    host: String,
    locale: String,
    origin_url: String,
    /// Domain switch AND project switch.
    enabled: bool,
    source_locale: String,
    locales: Vec<String>,
    /// `(locale, base URL)`: the source site first, then every verified, enabled subdomain of the project.
    bases: Vec<(String, String)>,
    skip_selectors: Vec<String>,
    dnt: Vec<String>,
    /// origin | prerender | auto (edge_settings.rendering): when to serve the renderer's snapshot.
    #[serde(default)]
    rendering: String,
    /// Application paths (`app_paths` rules), their default action, how the origin carries the locale.
    #[serde(default)]
    app_rules: Vec<String>,
    #[serde(default)]
    app_action: String,
    #[serde(default)]
    locale_style: String,
}

#[derive(Clone)]
struct CachedPage {
    status: StatusCode,
    headers: HeaderMap,
    body: axum::body::Bytes,
    state: &'static str,
}

struct Edge {
    client: reqwest::Client,
    /// Internal services (renderer, glot's own patcher bundle): no public-address restriction.
    internal: reqwest::Client,
    patcher_js: RwLock<Option<(Instant, Option<String>)>>,
    allow_private: bool,
    dir: PathBuf,
    domains: RwLock<HashMap<String, (Instant, Option<Arc<Domain>>)>>,
    projects: RwLock<HashMap<Uuid, ProjectRow>>,
    versions: RwLock<HashMap<(Uuid, String), (Instant, i64)>>,
    tms: RwLock<HashMap<(Uuid, String), Arc<Tm>>>,
    loading: Mutex<HashSet<(Uuid, String)>>,
    pages: Mutex<HashMap<String, (Instant, CachedPage)>>,
    reported: Mutex<HashSet<String>>,
    /// Per project: pages not yet counted, last flush, over the plan's `edge_pages` until.
    quota: Mutex<HashMap<Uuid, Quota>>,
    shells: Mutex<HashMap<String, (Instant, bool)>>,
    translations: Mutex<TrCache>,
}

/// One translation pass's result, reusable for an identical origin body.
struct Translated {
    html: axum::body::Bytes,
    total: usize,
    translated: usize,
}

/// Translated pages by sha256(origin body, domain row, path, TM version): origins that say `no-cache` (every
/// request revalidated, so no page cache) still skip the translate pass when their HTML did not change.
/// LRU bounded by bytes and entries.
// ponytail: O(n) eviction scan over ≤ TR_CACHE_MAX entries; a linked LRU if that ever shows in a profile.
#[derive(Default)]
struct TrCache {
    map: HashMap<[u8; 32], (u64, Arc<Translated>)>,
    tick: u64,
    bytes: usize,
}

const TR_CACHE_MAX: usize = 2000;
const TR_CACHE_BYTES: usize = 64 * 1024 * 1024;

impl TrCache {
    fn get(&mut self, k: &[u8; 32]) -> Option<Arc<Translated>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.0 = t;
            e.1.clone()
        })
    }

    fn put(&mut self, k: [u8; 32], v: Arc<Translated>) {
        let len = v.html.len();
        if len > TR_CACHE_BYTES / 8 {
            return;
        }
        while !self.map.is_empty()
            && (self.map.len() >= TR_CACHE_MAX || self.bytes + len > TR_CACHE_BYTES)
        {
            let Some(old) = self.map.iter().min_by_key(|e| e.1.0).map(|e| *e.0) else {
                break;
            };
            if let Some((_, o)) = self.map.remove(&old) {
                self.bytes -= o.html.len();
            }
        }
        self.tick += 1;
        self.bytes += len;
        if let Some((_, o)) = self.map.insert(k, (self.tick, v)) {
            self.bytes -= o.html.len();
        }
    }
}

/// Everything the translation output depends on: origin body, domain row (locale, bases, selectors, DNT…),
/// path, TM version.
fn tr_key(html: &[u8], d: &Domain, path: &str, v: i64) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(html);
    h.update([0]);
    h.update(serde_json::to_vec(d).unwrap_or_default());
    h.update([0]);
    h.update(path.as_bytes());
    h.update(v.to_le_bytes());
    h.finalize().into()
}

#[derive(Clone, Copy)]
struct Quota {
    pending: i64,
    flushed: Instant,
    blocked_until: Option<Instant>,
}

static EDGE: LazyLock<Edge> = LazyLock::new(|| {
    // Local development / tests only: lets the origin be 127.0.0.1. Never set in production.
    let allow_private = std::env::var("GLOT_EDGE_ALLOW_PRIVATE").is_ok_and(|v| v == "1");
    Edge {
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(ORIGIN_TIMEOUT)
            .connect_timeout(Duration::from_secs(5))
            .dns_resolver(Arc::new(PublicResolver { allow_private }))
            .user_agent("glot-edge/1")
            .build()
            .expect("edge http client"),
        internal: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("edge internal client"),
        patcher_js: Default::default(),
        allow_private,
        dir: std::env::var_os("GLOT_EDGE_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("glot-edge")),
        domains: Default::default(),
        projects: Default::default(),
        versions: Default::default(),
        tms: Default::default(),
        loading: Default::default(),
        pages: Default::default(),
        reported: Default::default(),
        quota: Default::default(),
        shells: Default::default(),
        translations: Default::default(),
    }
});

/// DNS resolution that refuses any name resolving to a private / loopback / link-local address (SSRF), used by
/// every origin connection, so the checked address is the one connected to (no rebinding window).
struct PublicResolver {
    allow_private: bool,
}

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow = self.allow_private;
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addrs.is_empty() || (!allow && addrs.iter().any(|a| !is_public(a.ip()))) {
                return Err("origin resolves to a non-public address".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

// ---------------------------------------------------------------------------------------------- state

impl Edge {
    fn disk(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }

    fn save<T: Serialize>(&self, name: &str, v: &T) {
        let (path, Ok(json)) = (self.disk(name), serde_json::to_vec(v)) else {
            return;
        };
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || {
            let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
            let _ = std::fs::create_dir_all(dir)
                .and_then(|_| std::fs::write(&tmp, json))
                .and_then(|_| std::fs::rename(&tmp, &path));
        });
    }

    fn load<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        serde_json::from_slice(&std::fs::read(self.disk(name)).ok()?).ok()
    }

    /// The verified domain for `host` (None = unknown: 404). DB errors serve the last known row (memory, disk).
    async fn domain(&self, state: &AppState, host: &str) -> Option<Arc<Domain>> {
        let cached = self.domains.read().ok()?.get(host).cloned();
        if let Some((at, d)) = &cached
            && at.elapsed() < DOMAIN_TTL
        {
            return d.clone();
        }
        // A cold lookup may take longer than the glot budget: without the domain there is no origin to serve.
        match tokio::time::timeout(Duration::from_secs(3), load_domain(state, host)).await {
            Ok(Ok(d)) => {
                let d = d.map(Arc::new);
                if let Some(d) = &d {
                    self.save(&format!("domain-{host}"), &**d);
                }
                if let Ok(mut m) = self.domains.write() {
                    if m.len() > 10_000 {
                        m.clear();
                    }
                    m.insert(host.to_string(), (Instant::now(), d.clone()));
                }
                d
            }
            _ => cached
                .and_then(|c| c.1)
                .or_else(|| self.load::<Domain>(&format!("domain-{host}")).map(Arc::new)),
        }
    }

    async fn project(&self, state: &AppState, d: &Domain) -> Option<ProjectRow> {
        if let Some(p) = self.projects.read().ok()?.get(&d.project_id) {
            return Some(p.clone());
        }
        let p = crate::projects::load_public(state, &d.slug).await.ok()??;
        self.projects.write().ok()?.insert(d.project_id, p.clone());
        Some(p)
    }

    /// The TM snapshot to translate this request with, within `deadline`. Newer versions load in the background
    /// when they miss it (the old snapshot is served whole meanwhile).
    async fn tm(&self, state: &AppState, d: &Domain, deadline: Instant) -> Option<Arc<Tm>> {
        let key = (d.project_id, d.locale.clone());
        let disk_name = format!("tm-{}-{}", d.project_id, d.locale);
        let mut cur = self.tms.read().ok()?.get(&key).cloned();
        if cur.is_none()
            && let Some(t) = self.load::<Tm>(&disk_name)
        {
            let t = Arc::new(t);
            self.tms.write().ok()?.insert(key.clone(), t.clone());
            cur = Some(t);
        }
        let known = self.versions.read().ok()?.get(&key).copied();
        let want = match known {
            Some((at, v)) if at.elapsed() < VERSION_TTL => v,
            _ => {
                let left = deadline.saturating_duration_since(Instant::now());
                match tokio::time::timeout(
                    left,
                    super::tm::version(&state.db, d.project_id, &d.locale),
                )
                .await
                {
                    Ok(Ok(v)) => {
                        self.versions
                            .write()
                            .ok()?
                            .insert(key.clone(), (Instant::now(), v));
                        v
                    }
                    _ => return cur, // stale-if-error
                }
            }
        };
        if cur.as_ref().is_some_and(|t| t.v >= want) {
            return cur;
        }
        if !self.loading.lock().ok()?.insert(key.clone()) {
            return cur; // another request is loading it
        }
        let p = self.project(state, d).await?;
        let st = state.clone();
        let locale = d.locale.clone();
        let load = tokio::spawn(async move {
            let r = super::tm::snapshot(&st.db, &p, &locale).await;
            let e = &*EDGE;
            if let Ok(t) = &r {
                let t = Arc::new(t.clone());
                if let Ok(mut m) = e.tms.write() {
                    m.insert((p.id, locale.clone()), t.clone());
                }
                e.save(&format!("tm-{}-{locale}", p.id), &*t);
            }
            if let Ok(mut l) = e.loading.lock() {
                l.remove(&(p.id, locale));
            }
            r.ok().map(Arc::new)
        });
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, load).await {
            Ok(Ok(Some(t))) => Some(t),
            _ => cur,
        }
    }
}

async fn load_domain(state: &AppState, host: &str) -> sqlx::Result<Option<Domain>> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        bool,
        String,
        Vec<String>,
        Vec<String>,
        String,
        Option<Vec<String>>,
        String,
        Option<String>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT d.project_id, p.slug, d.locale, d.origin_url, d.enabled AND coalesce(s.enabled, true),
                p.source_locale, p.target_locales, coalesce(s.selectors_exclude, '{}'),
                coalesce(s.rendering, 'auto'), s.app_paths, coalesce(s.app_action, 'redirect'), v.url_pattern
         FROM edge_domains d JOIN projects p ON p.id = d.project_id
         LEFT JOIN edge_settings s ON s.project_id = d.project_id
         LEFT JOIN preview_settings v ON v.project_id = d.project_id
         WHERE d.host = $1 AND d.mode = 'subdomain' AND d.verified_at IS NOT NULL",
    )
    .bind(host)
    .fetch_optional(&state.db)
    .await?;
    let Some((
        project_id,
        slug,
        locale,
        origin_url,
        enabled,
        source_locale,
        targets,
        skip,
        rendering,
        app_rules,
        app_action,
        url_pattern,
    )) = row
    else {
        return Ok(None);
    };
    let dnt: Vec<String> = sqlx::query_scalar(
        "SELECT term FROM glossary_terms WHERE project_id = $1 AND do_not_translate",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    // hreflang / localized links: only the locales ready for search engines (`sitemap::site(…, true)`); a
    // locale that is not ready yet (this one included) is served translated but not advertised.
    let site = match crate::projects::load_public(state, &slug).await {
        Ok(Some(p)) => super::sitemap::site(state, &p, true).await.ok(),
        _ => None,
    };
    let mut bases = site.map(|s| s.bases).unwrap_or_default();
    if bases.is_empty() {
        let source_base = Url::parse(&origin_url)
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default();
        bases.push((source_locale.clone(), source_base));
    }
    let mut locales = targets;
    locales.push(source_locale.clone());
    Ok(Some(Domain {
        project_id,
        slug,
        host: host.to_string(),
        locale,
        origin_url,
        enabled,
        source_locale,
        locales,
        bases,
        skip_selectors: skip,
        dnt,
        rendering,
        // No settings row: the suggested rules (`Settings::default`).
        app_rules: app_rules.unwrap_or_else(|| {
            super::app_paths::SUGGESTED
                .iter()
                .map(|s| s.to_string())
                .collect()
        }),
        app_action,
        locale_style: super::app_paths::locale_style(url_pattern.as_deref()),
    }))
}

// ---------------------------------------------------------------------------------------------- request

const HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// A session (logged-in visitor): never discovered, never shared through the page cache.
fn has_session(h: &HeaderMap) -> bool {
    h.contains_key(header::AUTHORIZATION)
        || h.get_all(header::COOKIE).iter().any(|c| {
            c.to_str().unwrap_or("").split(';').any(|kv| {
                let name = kv
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                [
                    "sess", "auth", "token", "sid", "jwt", "login", "logged", "user", "account",
                ]
                .iter()
                .any(|w| name.contains(w))
            })
        })
}

/// Seconds a shared cache may keep the origin's page (0 = not shareable).
fn cache_ttl(h: &HeaderMap) -> u64 {
    if h.contains_key(header::SET_COOKIE) {
        return 0;
    }
    let cc = h
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ["private", "no-store", "no-cache"]
        .iter()
        .any(|w| cc.contains(w))
    {
        return 0;
    }
    let num = |k: &str| {
        cc.split(',')
            .find_map(|d| d.trim().strip_prefix(k)?.parse::<u64>().ok())
    };
    num("s-maxage=").or_else(|| num("max-age=")).unwrap_or(0)
}

fn plain(status: StatusCode, msg: &'static str, state: &'static str) -> Response {
    let mut r = (status, msg).into_response();
    r.headers_mut()
        .insert("x-glot-edge", HeaderValue::from_static(state));
    r
}

async fn proxy(State(state): State<AppState>, req: Request) -> Response {
    let t0 = Instant::now();
    let deadline = t0 + GLOT_BUDGET;
    let e = &*EDGE;
    let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(clean_host)
    else {
        return plain(StatusCode::NOT_FOUND, "unknown edge domain", "unknown");
    };
    let Some(d) = e.domain(&state, &host).await else {
        return plain(StatusCode::NOT_FOUND, "unknown edge domain", "unknown");
    };
    let (parts, body) = req.into_parts();
    let path_q = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/edge/proxy");
    let path_q = path_q.strip_prefix("/edge/proxy").unwrap_or(path_q);
    let path_q = if path_q.starts_with('/') {
        path_q.to_string()
    } else {
        format!("/{path_q}")
    };
    if let Some(res) = own_route(&state, &d, &path_q).await {
        return res;
    }
    if parts.method == Method::POST && path_q.split('?').next() == Some(MISSING_PATH) {
        return client_missing(&state, &d, &parts.headers, body).await;
    }
    let Ok(mut origin) = Url::parse(&d.origin_url) else {
        return plain(StatusCode::BAD_GATEWAY, "bad origin", "origin-error");
    };
    let (path, query) = path_q.split_once('?').unwrap_or((&path_q, ""));
    // Application paths (login, account, API…): back to the origin's own domain, or passed through untouched.
    let cookies = parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join("; ");
    let app = app_paths::decide(&app_paths::Input {
        rules: &d.app_rules,
        action: if d.app_action == "proxy" {
            app_paths::Action::Proxy
        } else {
            app_paths::Action::Redirect
        },
        locale_style: &d.locale_style,
        locale: &d.locale,
        origin: &d.origin_url,
        host: &host,
        req_path: path,
        path,
        query,
        method: parts.method.as_str(),
        cookie: &cookies,
    });
    if let Some(app_paths::Decision::Redirect {
        status,
        location,
        cookie,
    }) = &app
        && let (Ok(loc), Ok(c)) = (
            HeaderValue::from_str(location),
            HeaderValue::from_str(cookie),
        )
    {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::from_u16(*status).unwrap_or(StatusCode::FOUND);
        let h = r.headers_mut();
        h.insert(header::LOCATION, loc);
        h.insert(header::SET_COOKIE, c);
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        h.insert("x-glot-edge", HeaderValue::from_static("app-redirect"));
        return r;
    }
    let app_proxy = app.is_some();
    origin.set_path(path);
    origin.set_query((!query.is_empty()).then_some(query));
    if let Some(ip) = origin
        .host_str()
        .and_then(|h| h.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        && !e.allow_private
        && !is_public(ip)
    {
        return plain(
            StatusCode::BAD_GATEWAY,
            "origin not allowed",
            "origin-error",
        );
    }
    let get = parts.method == Method::GET || parts.method == Method::HEAD;
    let anon = !has_session(&parts.headers);
    let over_quota = blocked(d.project_id);
    let translatable = d.enabled && !over_quota && !app_proxy && parts.method == Method::GET;

    // TM first (cheap when warm): its version keys the page cache.
    let tm = if translatable {
        e.tm(&state, &d, deadline).await
    } else {
        None
    };
    let t_lookup = t0.elapsed();
    let cache_key = tm
        .as_ref()
        .filter(|_| anon && get)
        .map(|t| format!("{host}{path_q}@{}", t.v));
    if let Some(k) = &cache_key
        && let Some((exp, page)) = e.pages.lock().ok().and_then(|m| m.get(k).cloned())
        && exp > Instant::now()
    {
        let mut res = Response::builder().status(page.status);
        *res.headers_mut().expect("builder") = page.headers;
        let mut res = res.body(Body::from(page.body)).expect("response");
        finish(&mut res, page.state, t_lookup, Duration::ZERO, Some("hit"));
        count(&state, d.project_id);
        return res;
    }

    // Origin request: fixed origin, request headers minus hop-by-hop / Host, uncompressed (we may read it).
    let body = match axum::body::to_bytes(body, MAX_REQUEST_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return plain(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request too large",
                "passthrough",
            );
        }
    };
    let mut up = reqwest::header::HeaderMap::new();
    for (k, v) in &parts.headers {
        let n = k.as_str();
        if HOP.contains(&n) || n == "host" || n == "accept-encoding" || n == "content-length" {
            continue;
        }
        up.append(k.clone(), v.clone());
    }
    up.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    if let Ok(v) = HeaderValue::from_str(&host) {
        up.insert("x-forwarded-host", v);
    }
    up.insert("x-forwarded-proto", HeaderValue::from_static("https"));
    if let Ok(v) = HeaderValue::from_str(&d.locale) {
        up.insert("x-glot-locale", v);
    }
    let t_origin = Instant::now();
    let resp = match e
        .client
        .request(parts.method.clone(), origin.clone())
        .headers(up)
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            tracing::info!(target: "glot_edge", %host, "origin error: {err}");
            return plain(
                StatusCode::BAD_GATEWAY,
                "origin unreachable",
                "origin-error",
            );
        }
    };
    let status = resp.status();
    let mut headers = HeaderMap::new();
    for (k, v) in resp.headers() {
        if !HOP.contains(&k.as_str()) {
            headers.append(k.clone(), v.clone());
        }
    }
    rewrite_location(&mut headers, &origin, &host);
    let is_html = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("text/html"));
    let len = resp.content_length().unwrap_or(0) as usize;
    let mode = if app_proxy {
        "app"
    } else if !d.enabled {
        "off"
    } else if over_quota {
        "quota"
    } else if translatable && status == StatusCode::OK && is_html && len <= MAX_HTML {
        "translate"
    } else {
        "passthrough"
    };
    if mode != "translate" {
        let origin_ms = t_origin.elapsed();
        let mut res = stream(status, headers, resp);
        finish(&mut res, mode, t_lookup, origin_ms, None);
        return res;
    }
    let html = match read_capped(resp, MAX_HTML).await {
        Ok(Some(b)) => b,
        Ok(None) => return plain(StatusCode::BAD_GATEWAY, "page too large", "origin-error"),
        Err(_) => return plain(StatusCode::BAD_GATEWAY, "origin read error", "origin-error"),
    };
    let origin_ms = t_origin.elapsed();
    // SPA shells: the renderer's snapshot of this source page (never rendered in the request path; fail-open).
    // The "empty shell" verdict is kept per path for 10 min (it scans the whole page).
    let t_shell = Instant::now();
    let html = match d.rendering.as_str() {
        "prerender" => snapshot(&origin, deadline).await.unwrap_or(html),
        "auto" if is_shell(&host, path, &html) => snapshot(&origin, deadline).await.unwrap_or(html),
        _ => html,
    };
    let t_lookup = t_lookup + t_shell.elapsed();
    let ttl = cache_ttl(&headers);
    for h in [
        "content-length",
        "content-encoding",
        "etag",
        "content-language",
    ] {
        headers.remove(h);
    }
    let Some(tm) = tm else {
        // glot had no TM within its budget: the origin page, untranslated and never cached under this URL.
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        let mut res = (status, headers, html).into_response();
        finish(&mut res, "fail-open", t_lookup, origin_ms, None);
        return res;
    };

    // Same origin body, same domain row and version: the previous translation (origins that revalidate every
    // request). Otherwise translation off the async runtime, bounded by what is left of the budget (≥ 150 ms CPU).
    let t_tr = Instant::now();
    let key = anon.then(|| tr_key(&html, &d, path, tm.v));
    let reused = key
        .as_ref()
        .and_then(|k| e.translations.lock().ok()?.get(k));
    let (out, reused) = match reused {
        Some(t) => (t, true),
        None => {
            let src = String::from_utf8_lossy(&html).into_owned();
            let (dd, tt, pq) = (d.clone(), tm.clone(), path.to_string());
            let job = tokio::task::spawn_blocking(move || translate(&src, &dd, &tt, &pq));
            let left = deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(150));
            let out = match tokio::time::timeout(left, job).await {
                Ok(Ok(o)) => o,
                _ => {
                    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                    let mut res = (status, headers, html).into_response();
                    finish(
                        &mut res,
                        "fail-open",
                        t_lookup + t_tr.elapsed(),
                        origin_ms,
                        None,
                    );
                    return res;
                }
            };
            if anon {
                report(&state, &d, path, out.missing, out.seen);
            }
            let t = Arc::new(Translated {
                html: out.html.into(),
                total: out.total,
                translated: out.translated,
            });
            if let Some(k) = key
                && let Ok(mut c) = e.translations.lock()
            {
                c.put(k, t.clone());
            }
            (t, false)
        }
    };
    let glot_ms = t_lookup + t_tr.elapsed();
    for (k, v) in seo::headers(&seo_ctx(&d, path, &|_| None)) {
        if let Ok(v) = HeaderValue::from_str(&v) {
            headers.insert(HeaderName::from_static(k), v);
        }
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{}@{}", d.locale, tm.v)) {
        headers.insert("x-glot-version", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{}/{}", out.translated, out.total)) {
        headers.insert("x-glot-coverage", v);
    }
    let st = if out.translated > 0 {
        "translated"
    } else {
        "source"
    };
    let body = out.html.clone();
    if let Some(k) = cache_key
        && ttl > 0
        && let Ok(mut m) = e.pages.lock()
    {
        if m.len() >= PAGE_CACHE_MAX {
            m.clear(); // ponytail: wholesale eviction; an LRU if hit ratio suffers
        }
        let page = CachedPage {
            status,
            headers: headers.clone(),
            body: body.clone(),
            state: st,
        };
        m.insert(k, (Instant::now() + Duration::from_secs(ttl), page));
    }
    count(&state, d.project_id);
    let mut res = (status, headers, body).into_response();
    let cache = if reused { "hit-body" } else { "miss" };
    finish(&mut res, st, glot_ms, origin_ms, Some(cache));
    res
}

/// Over the plan's `edge_pages` (the origin is served untranslated meanwhile).
fn blocked(project: Uuid) -> bool {
    EDGE.quota
        .lock()
        .ok()
        .and_then(|m| m.get(&project).and_then(|q| q.blocked_until))
        .is_some_and(|t| t > Instant::now())
}

/// Count one translated page; flushed to billing every 50 pages or 10 s, off the request path.
fn count(state: &AppState, project: Uuid) {
    if state.billing.is_none() {
        return;
    }
    let Ok(mut m) = EDGE.quota.lock() else { return };
    let q = m.entry(project).or_insert(Quota {
        pending: 0,
        flushed: Instant::now(),
        blocked_until: None,
    });
    q.pending += 1;
    if q.pending < 50 && q.flushed.elapsed() < Duration::from_secs(10) {
        return;
    }
    let n = std::mem::take(&mut q.pending);
    q.flushed = Instant::now();
    drop(m);
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) =
            crate::billing::consume_n(&state, project, crate::billing::EDGE_PAGES, n).await
            && e.status == StatusCode::PAYMENT_REQUIRED
            && let Ok(mut m) = EDGE.quota.lock()
            && let Some(q) = m.get_mut(&project)
        {
            q.blocked_until = Some(Instant::now() + Duration::from_secs(300));
        }
    });
}

fn finish(res: &mut Response, state: &str, glot: Duration, origin: Duration, cache: Option<&str>) {
    let desc = match cache {
        Some(c) => format!("{state} {c}"),
        None => state.to_string(),
    };
    let h = res.headers_mut();
    // `cache=hit`: whole response from the page cache; `cache=hit-body`: origin fetched (revalidated), its
    // unchanged body's translation reused.
    let x = match cache {
        Some(c @ ("hit" | "hit-body")) => format!("{state}; cache={c}"),
        _ => state.to_string(),
    };
    if let Ok(v) = HeaderValue::from_str(&x) {
        h.insert("x-glot-edge", v);
    }
    let timing = format!(
        "glot;dur={:.1};desc=\"{desc}\", origin;dur={:.1}",
        glot.as_secs_f64() * 1000.0,
        origin.as_secs_f64() * 1000.0
    );
    if let Ok(v) = HeaderValue::from_str(&timing) {
        h.append("server-timing", v);
    }
    tracing::debug!(target: "glot_edge", state, glot_ms = glot.as_millis() as u64, origin_ms = origin.as_millis() as u64, cache);
}

/// `Location: https://origin.example/x` → `https://es.example.com/x` (relative locations are already right).
fn rewrite_location(h: &mut HeaderMap, origin: &Url, host: &str) {
    let Some(loc) = h.get(header::LOCATION).and_then(|v| v.to_str().ok()) else {
        return;
    };
    let Ok(mut u) = origin.join(loc) else { return };
    if u.host_str() == origin.host_str() && u.port() == origin.port() && loc.contains("://") {
        let _ = u.set_scheme("https");
        let _ = u.set_host(Some(host));
        let _ = u.set_port(None);
        if let Ok(v) = HeaderValue::from_str(u.as_str()) {
            h.insert(header::LOCATION, v);
        }
    }
}

/// Whole body, or None past `max`.
async fn read_capped(mut resp: reqwest::Response, max: usize) -> reqwest::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    while let Some(c) = resp.chunk().await? {
        buf.extend_from_slice(&c);
        if buf.len() > max {
            return Ok(None);
        }
    }
    Ok(Some(buf))
}

/// The origin response streamed through as is.
fn stream(status: StatusCode, headers: HeaderMap, mut resp: reqwest::Response) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        loop {
            match resp.chunk().await {
                Ok(Some(c)) => {
                    if tx.send(Ok(c)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Err(std::io::Error::other(e))).await;
                    break;
                }
            }
        }
    });
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    let mut res = Response::new(body);
    *res.status_mut() = status;
    *res.headers_mut() = headers;
    res
}

// ---------------------------------------------------------------------------------------------- own routes

const TM_PATH: &str = "/__glot/tm.json";
const PATCHER_PATH: &str = "/__glot/patcher.js";
const MISSING_PATH: &str = "/__glot/missing";

/// `/__glot/tm.json` (this domain's TM, current version) and `/__glot/patcher.js` (glot's client patcher bundle,
/// `GLOT_EDGE_PATCHER_SRC`, default `{web origin}/widget/edge-patcher.global.js`, cached 10 min) on the edge host;
/// `/sitemap.xml`, `/robots.txt` and the IndexNow key file (`sitemap::special`) while Edge is on.
async fn own_route(state: &AppState, d: &Domain, path_q: &str) -> Option<Response> {
    let path = path_q.split('?').next().unwrap_or("");
    let e = &*EDGE;
    if d.enabled
        && (path == "/sitemap.xml"
            || path == "/robots.txt"
            || (path.ends_with(".txt") && path.rfind('/') == Some(0)))
        && let Some(p) = e.project(state, d).await
        && let Some(r) = super::sitemap::special(state, &p, &d.locale, path).await
    {
        return Some(r);
    }
    let cache = |r: &mut Response, v: &'static str| {
        r.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(v));
    };
    match path {
        TM_PATH => {
            let tm = e
                .tm(state, d, Instant::now() + Duration::from_secs(2))
                .await;
            let mut r = match tm.filter(|_| d.enabled) {
                Some(t) => (
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                    serde_json::to_string(&*t).unwrap_or_default(),
                )
                    .into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            };
            cache(&mut r, "public, max-age=30, stale-if-error=86400");
            Some(r)
        }
        PATCHER_PATH => Some(patcher().await),
        _ => None,
    }
}

/// `POST /__glot/missing` from the client patcher: text the app rendered on the client that the TM lacks. The
/// visitor's session is decided here from the request (HttpOnly cookies included), the locale is the domain's,
/// and `missing::record` re-checks hashes, personal data, discovery zones and caps. Always 204.
async fn client_missing(state: &AppState, d: &Domain, h: &HeaderMap, body: Body) -> Response {
    let done = || StatusCode::NO_CONTENT.into_response();
    if !d.enabled || d.locale == d.source_locale {
        return done();
    }
    let Ok(raw) = axum::body::to_bytes(body, 64 * 1024).await else {
        return done();
    };
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        segments: Vec<serde_json::Value>,
    }
    let Ok(b) = serde_json::from_slice::<In>(&raw) else {
        return done();
    };
    let s = |v: &serde_json::Value, k: &str, max: usize| {
        v[k].as_str().filter(|x| x.len() <= max).map(str::to_string)
    };
    let segments: Vec<missing::Item> = b
        .segments
        .iter()
        .take(200)
        .filter_map(|x| {
            let url = s(x, "url", 2000)?.split(['?', '#']).next()?.to_string();
            url.starts_with('/').then_some(())?;
            Some(missing::Item {
                hash: s(x, "hash", 64)?,
                text: s(x, "text", 2000)?,
                url,
                role: s(x, "role", 64).unwrap_or_default(),
            })
        })
        .collect();
    if segments.is_empty() {
        return done();
    }
    let Some(p) = EDGE.project(state, d).await else {
        return done();
    };
    let r = missing::Report {
        key: None,
        locale: d.locale.clone(),
        auth: has_session(h),
        segments,
        pages: vec![],
    };
    if let Err(e) = missing::record(state, &p, &r).await {
        tracing::info!(target: "glot_edge", "client discovery report failed: {}", e.message);
    }
    done()
}

/// The client patcher bundle (`GLOT_EDGE_PATCHER_SRC`, default `{web origin}/widget/edge-patcher.global.js`),
/// cached 10 min, stale-if-error; also `GET /edge/patcher.js` for the JS runtimes' `/__glot/patcher.js`.
async fn patcher() -> Response {
    let e = &*EDGE;
    let cached = e.patcher_js.read().ok().and_then(|c| c.clone());
    let js = match cached {
        Some((at, js)) if at.elapsed() < Duration::from_secs(600) => js,
        _ => {
            let src = std::env::var("GLOT_EDGE_PATCHER_SRC").unwrap_or_else(|_| {
                format!(
                    "{}/widget/edge-patcher.global.js",
                    crate::review_live::web_origin()
                )
            });
            let got = match e.internal.get(&src).send().await {
                Ok(r) if r.status().is_success() => r.text().await.ok(),
                _ => None,
            };
            let js = got.or_else(|| cached.and_then(|c| c.1)); // stale-if-error
            if let Ok(mut w) = e.patcher_js.write() {
                *w = Some((Instant::now(), js.clone()));
            }
            js
        }
    };
    // No bundle: an empty script, the page keeps the Edge-translated HTML.
    let mut r = (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        js.unwrap_or_default(),
    )
        .into_response();
    let h = r.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=600"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    r
}

fn is_shell(host: &str, path: &str, html: &[u8]) -> bool {
    let key = format!("{host}{path}");
    if let Some((at, v)) = EDGE.shells.lock().ok().and_then(|m| m.get(&key).copied())
        && at.elapsed() < Duration::from_secs(600)
    {
        return v;
    }
    let v = hydration::is_skeleton(&String::from_utf8_lossy(html));
    if let Ok(mut m) = EDGE.shells.lock() {
        if m.len() > 10_000 {
            m.clear();
        }
        m.insert(key, (Instant::now(), v));
    }
    v
}

/// The renderer's stored snapshot of `url` (`GLOT_RENDERER_URL`), within the glot budget; None = use the origin.
async fn snapshot(url: &Url, deadline: Instant) -> Option<Vec<u8>> {
    let base = std::env::var("GLOT_RENDERER_URL").ok()?;
    let token = std::env::var("GLOT_RENDERER_TOKEN").unwrap_or_default();
    let left = deadline.saturating_duration_since(Instant::now());
    let req = EDGE
        .internal
        .get(format!("{}/snapshot", base.trim_end_matches('/')))
        .query(&[("url", url.as_str())])
        .bearer_auth(token)
        .send();
    let r = tokio::time::timeout(left, req).await.ok()?.ok()?;
    if r.status() != StatusCode::OK {
        return None; // 202 = queued: the next requests get it
    }
    tokio::time::timeout(
        deadline.saturating_duration_since(Instant::now()),
        r.bytes(),
    )
    .await
    .ok()?
    .ok()
    .map(|b| b.to_vec())
}

// ---------------------------------------------------------------------------------------------- translation

struct Out {
    html: String,
    total: usize,
    translated: usize,
    missing: Vec<missing::Item>,
    seen: Vec<missing::Seen>,
}

fn seo_ctx<'a>(d: &Domain, path: &str, tr: &'a dyn Fn(&str) -> Option<String>) -> SeoCtx<'a> {
    SeoCtx {
        source_locale: d.source_locale.clone(),
        locale: d.locale.clone(),
        bases: d.bases.clone(),
        path: path.to_string(),
        tr,
    }
}

/// Segments + SEO of one page with one TM snapshot. Pure: no I/O.
fn translate(html: &str, d: &Domain, tm: &Tm, path: &str) -> Out {
    let skip: Vec<&str> = d.skip_selectors.iter().map(String::as_str).collect();
    let mut opts = SegmentOpts::new(&d.source_locale).skip(&skip);
    opts.dnt = d.dnt.clone();
    let template = ctm::template(path, &d.locales);
    let (mut total, mut translated) = (0, 0);
    let mut missing = vec![];
    let mut seen = vec![];
    // Plain-text segments (no inline tags / values): framework payload strings equal to them are translated too.
    let mut payload: HashMap<String, String> = HashMap::new();
    let out = segment::translate_html(html, &opts, |s| {
        total += 1;
        seen.push(missing::Seen {
            hash: s.hash.clone(),
            role: s.ctx.role.clone(),
        });
        let v = tm.lookup(&s.hash, &format!("{}|{template}", s.ctx.role));
        match v {
            Some(v) => {
                translated += 1;
                if s.tags.is_empty() && s.values.is_empty() && s.attr.is_none() {
                    payload.insert(s.text.clone(), v.to_string());
                }
                Some(v.to_string())
            }
            None => {
                if s.discoverable {
                    missing.push(missing::Item {
                        hash: s.hash.clone(),
                        text: s.text.clone(),
                        url: path.to_string(),
                        role: s.ctx.role.clone(),
                    });
                }
                None
            }
        }
    });
    let tr = |text: &str| {
        let n = ctm::normalize(text);
        tm.lookup(
            &ctm::text_hash(&d.source_locale, &n),
            &format!("text|{template}"),
        )
        .map(String::from)
    };
    let out = hydration::translate_payloads(&out, &|src: &str| payload.get(src).cloned()).0;
    let html = seo::rewrite(&out, &seo_ctx(d, path, &tr));
    // Page version for the client patcher (it ignores an older TM), then the patcher itself: same-origin URLs
    // (served by `own_route`), so the origin's CSP `script-src 'self'` allows it.
    let html = hydration::inject_head(
        &html,
        &format!(r#"<meta name="glot-tm" content="{}@{}">"#, d.locale, tm.v),
    );
    let html = hydration::inject_head(
        &html,
        &hydration::patcher_tag(PATCHER_PATH, TM_PATH, &d.source_locale, Some(MISSING_PATH)),
    );
    Out {
        html,
        total,
        translated,
        missing,
        seen,
    }
}

/// Background discovery report: new (hash, role) once per process, page coverage once per path per process.
fn report(
    state: &AppState,
    d: &Domain,
    path: &str,
    items: Vec<missing::Item>,
    mut seen: Vec<missing::Seen>,
) {
    let e = &*EDGE;
    let Ok(mut done) = e.reported.lock() else {
        return;
    };
    if done.len() > 50_000 {
        done.clear();
    }
    let segments: Vec<missing::Item> = items
        .into_iter()
        .filter(|i| {
            done.insert(format!(
                "{}|{}|{}|{}",
                d.project_id, d.locale, i.hash, i.role
            ))
        })
        .take(200)
        .collect();
    let pages = if done.insert(format!("page|{}|{}|{path}", d.project_id, d.locale)) {
        seen.truncate(500);
        vec![missing::Page {
            url: path.to_string(),
            seen,
        }]
    } else {
        vec![]
    };
    drop(done);
    if segments.is_empty() && pages.is_empty() {
        return;
    }
    let (state, d) = (state.clone(), d.clone());
    tokio::spawn(async move {
        let Some(p) = EDGE.project(&state, &d).await else {
            return;
        };
        let r = missing::Report {
            key: None,
            locale: d.locale.clone(),
            auth: false,
            segments,
            pages,
        };
        if let Err(e) = missing::record(&state, &p, &r).await {
            tracing::info!(target: "glot_edge", "discovery report failed: {}", e.message);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_cache_ttl() {
        let h = |pairs: &[(&str, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.append(
                    HeaderName::from_bytes(k.as_bytes()).unwrap(),
                    HeaderValue::from_str(v).unwrap(),
                );
            }
            m
        };
        assert!(has_session(&h(&[(
            "cookie",
            "_ga=1; next-auth.session-token=x"
        )])));
        assert!(has_session(&h(&[("authorization", "Bearer x")])));
        assert!(!has_session(&h(&[("cookie", "_ga=1; glot_locale=es")])));
        assert_eq!(
            cache_ttl(&h(&[("cache-control", "public, s-maxage=30, max-age=5")])),
            30
        );
        assert_eq!(cache_ttl(&h(&[("cache-control", "max-age=60")])), 60);
        assert_eq!(
            cache_ttl(&h(&[("cache-control", "private, max-age=60")])),
            0
        );
        assert_eq!(
            cache_ttl(&h(&[
                ("cache-control", "max-age=60"),
                ("set-cookie", "a=b")
            ])),
            0
        );
    }

    #[test]
    fn location_rewritten_to_the_edge_host() {
        let origin = Url::parse("https://cliponit.app/pricing").unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            header::LOCATION,
            HeaderValue::from_static("https://cliponit.app/pricing/"),
        );
        rewrite_location(&mut h, &origin, "es.cliponit.app");
        assert_eq!(h[header::LOCATION], "https://es.cliponit.app/pricing/");
        h.insert(
            header::LOCATION,
            HeaderValue::from_static("https://accounts.google.com/x"),
        );
        rewrite_location(&mut h, &origin, "es.cliponit.app");
        assert_eq!(h[header::LOCATION], "https://accounts.google.com/x");
        h.insert(header::LOCATION, HeaderValue::from_static("/login"));
        rewrite_location(&mut h, &origin, "es.cliponit.app");
        assert_eq!(h[header::LOCATION], "/login");
    }
}
