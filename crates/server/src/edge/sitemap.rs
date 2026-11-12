//! glot Edge sitemaps, robots.txt and IndexNow.
//!
//! Every ready locale gets `{base(locale)}/sitemap.xml`: the origin sitemap's pages (fetched SSRF-safe from the
//! domain's fixed origin, cached 1 h), each with its localized URL and `xhtml:link` alternates for every ready
//! locale + x-default, so each file alone carries the whole hreflang cluster. `robots.txt` on a translated host
//! keeps the origin rules and announces that host's sitemap: this is how Google finds them (Google does not use
//! IndexNow). IndexNow (`POST /api/projects/{slug}/edge/indexnow`) is only for Bing / Yandex.
//!
//! The proxy (subdomain mode) answers `/sitemap.xml`, `/robots.txt` and `/{indexnow key}.txt` with [`special`];
//! the worker / middleware (subdirectory mode) forwards `/{locale}/sitemap.xml` to
//! `GET /edge/v1/{project}/{locale}/sitemap.xml` (same body). A locale that is not ready has no sitemap (404).
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hmac::{Hmac, Mac};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppPath, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
};

/// Global coverage a locale needs before glot advertises it to search engines (hreflang, sitemaps, IndexNow).
/// The language switcher passes its own `min` to the manifest; critical pages always need their threshold.
// ponytail: constant; a per-project setting when someone needs a different bar for SEO than for the switcher.
pub const SEO_MIN: f64 = 0.95;
const MAX_URLS: usize = 50_000;
const MAX_SITEMAP_BYTES: usize = 10 * 1024 * 1024;
const MAX_SUB_SITEMAPS: usize = 20;
const ORIGIN_TTL: Duration = Duration::from_secs(3600);

/// Where a project's pages live per locale.
#[derive(Debug, Clone, Default)]
pub struct Site {
    pub source_locale: String,
    /// `(locale, base URL)`, source first: `https://cliponit.app/es` (subdirectory) or `https://es.cliponit.app`.
    pub bases: Vec<(String, String)>,
    /// Fixed origin to fetch source pages / sitemap / robots.txt from (never taken from a request).
    pub origin: Option<String>,
    /// Per advertised locale, the pages complete enough to be advertised (`seo_report::page_ready`).
    /// `None` = not computed (`ready_only: false`): every page of every listed locale.
    pub ready_pages: Option<super::seo_report::ReadyPages>,
    /// Translated URL paths (empty unless `edge_settings.translated_slugs`).
    pub slugs: glot_core::edge::seo::Slugs,
}

impl Site {
    pub fn base(&self, locale: &str) -> Option<&str> {
        self.bases
            .iter()
            .find(|(l, _)| l == locale)
            .map(|(_, b)| b.as_str())
    }

    /// The SEO rewrite context of one page of this site.
    pub fn seo_ctx<'a>(
        &'a self,
        locale: &str,
        path: &str,
        tr: &'a dyn Fn(&str) -> Option<String>,
    ) -> glot_core::edge::seo::SeoCtx<'a> {
        glot_core::edge::seo::SeoCtx {
            source_locale: self.source_locale.clone(),
            locale: locale.into(),
            bases: self.bases.clone(),
            path: path.into(),
            tr,
            page_locales: self.page_locales(path),
            slugs: Some(&self.slugs),
        }
    }

    /// The locales whose version of `path` is complete (`None` = not computed: all of them). A URL with a
    /// query string is never measured (no discovery, no pre-warm on those), so it is never advertised.
    pub fn page_locales(&self, path: &str) -> Option<Vec<String>> {
        let page = glot_core::edge::tm::path_only(path);
        self.ready_pages.as_ref().map(|r| {
            r.iter()
                .filter(|(_, pages)| !path.contains('?') && pages.contains(&page))
                .map(|(l, _)| l.clone())
                .collect()
        })
    }
}

/// `http` for local hosts (dev / tests), `https` otherwise.
fn scheme(host: &str) -> &'static str {
    let h = host.split(':').next().unwrap_or(host);
    if h == "localhost" || h.parse::<std::net::IpAddr>().is_ok() || host.contains(':') {
        "http"
    } else {
        "https"
    }
}

fn origin_of(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
        .filter(|o| o != "null")
}

/// The project's verified, enabled Edge domains as per-locale bases. `ready_only`: only locales that pass
/// [`SEO_MIN`] and the critical-page threshold (what is advertised to search engines).
pub async fn site(state: &AppState, p: &ProjectRow, ready_only: bool) -> ApiResult<Site> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT host, mode, locale, origin_url FROM edge_domains
         WHERE project_id = $1 AND verified_at IS NOT NULL AND enabled ORDER BY created_at, host",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let (ready, ready_pages): (Vec<String>, _) = if ready_only {
        let (r, pages) = super::seo_report::readiness_pages(state, p).await?;
        (
            r.into_iter()
                .filter(|r| r.ready(SEO_MIN))
                .map(|r| r.locale)
                .collect(),
            Some(pages),
        )
    } else {
        (p.target_locales.clone(), None)
    };
    let slugs_on: bool = sqlx::query_scalar(
        "SELECT coalesce((SELECT translated_slugs AND enabled FROM edge_settings WHERE project_id = $1), false)",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    let mut site = Site {
        source_locale: p.source_locale.clone(),
        ready_pages,
        slugs: if slugs_on {
            super::slugs::load(&state.db, p.id).await?.current
        } else {
            Default::default()
        },
        ..Default::default()
    };
    let mut source_base: Option<String> = None;
    let mut bases = Vec::new();
    for (host, mode, locale, origin_url) in &rows {
        let root = format!("{}://{host}", scheme(host));
        if mode == "subdomain" {
            // The source site is the origin itself (`cliponit.app` behind `es.cliponit.app`).
            source_base = source_base.or_else(|| origin_of(origin_url));
            if locale != &p.source_locale {
                bases.push((locale.clone(), root));
            }
        } else {
            // Subdirectory / middleware: one host serves every locale, the source unprefixed.
            source_base.get_or_insert(root.clone());
            if locale != &p.source_locale {
                bases.push((locale.clone(), format!("{root}/{locale}")));
            }
        }
        site.origin
            .get_or_insert(origin_url.trim_end_matches('/').to_string());
    }
    let Some(source_base) = source_base else {
        return Ok(site);
    };
    site.bases.push((p.source_locale.clone(), source_base));
    // Target order of the project, each locale once.
    for l in &p.target_locales {
        if ready.contains(l)
            && let Some(b) = bases.iter().find(|(x, _)| x == l)
        {
            site.bases.push(b.clone());
        }
    }
    Ok(site)
}

/// GET with redirects (≤ 3 hops), SSRF-safe, body capped.
pub(crate) async fn fetch(
    url: &str,
    max: usize,
) -> Result<(u16, reqwest::header::HeaderMap, String, Url), &'static str> {
    let mut url = Url::parse(url).map_err(|_| "unreachable")?;
    for _ in 0..=3 {
        let mut resp =
            crate::review_live::probe::fetch_with(&url, allow_private(), "glot-edge-seo/1").await?;
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
        let headers = resp.headers().clone();
        let mut buf = Vec::new();
        while let Ok(Ok(Some(c))) =
            tokio::time::timeout(Duration::from_secs(10), resp.chunk()).await
        {
            buf.extend_from_slice(&c);
            if buf.len() >= max {
                buf.truncate(max);
                break;
            }
        }
        return Ok((
            status,
            headers,
            String::from_utf8_lossy(&buf).into_owned(),
            url,
        ));
    }
    Err("unreachable")
}

/// Origins on private addresses are refused unless `GLOT_EDGE_ALLOW_PRIVATE=1` (local development and tests only).
pub(crate) fn allow_private() -> bool {
    std::env::var("GLOT_EDGE_ALLOW_PRIVATE").is_ok_and(|v| v == "1")
}

/// `<loc>` values of a sitemap, and whether it is a sitemap index.
pub fn locs(xml: &str) -> (Vec<String>, bool) {
    let index = xml.contains("<sitemapindex");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<loc>") {
        rest = &rest[i + 5..];
        let Some(j) = rest.find("</loc>") else { break };
        let v = rest[..j].trim();
        let v = v
            .strip_prefix("<![CDATA[")
            .and_then(|v| v.strip_suffix("]]>"))
            .unwrap_or(v);
        out.push(unescape(v.trim()));
        rest = &rest[j..];
        if out.len() >= MAX_URLS {
            break;
        }
    }
    (out, index)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

static ORIGIN_PATHS: Mutex<Option<HashMap<String, (Instant, Vec<String>)>>> = Mutex::new(None);

/// Source paths of the site: the origin's sitemap(s) (robots.txt `Sitemap:` lines, else `/sitemap.xml`;
/// one level of index), keeping only URLs of the source site. Cached 1 h per origin. `/` when none.
pub async fn source_paths(site: &Site) -> Vec<String> {
    let Some(origin) = site.origin.clone() else {
        return vec!["/".into()];
    };
    if let Some((at, v)) = ORIGIN_PATHS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(&origin)
        && at.elapsed() < ORIGIN_TTL
    {
        return v.clone();
    }
    let mut maps: Vec<String> = match fetch(&format!("{origin}/robots.txt"), 256 * 1024).await {
        Ok((200, _, body, _)) => body
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case("sitemap")
                    .then(|| v.trim().to_string())
            })
            .collect(),
        _ => vec![],
    };
    if maps.is_empty() {
        maps.push(format!("{origin}/sitemap.xml"));
    }
    let source = site
        .base(&site.source_locale)
        .unwrap_or(&origin)
        .to_string();
    let hosts = [origin_of(&source), origin_of(&origin)];
    let mut pages: Vec<String> = Vec::new();
    let mut fetched = 0;
    while let Some(m) = maps.pop() {
        fetched += 1;
        if fetched > MAX_SUB_SITEMAPS {
            break;
        }
        // Sitemaps listed on other hosts are not followed (they could point anywhere).
        if !hosts.contains(&origin_of(&m)) {
            continue;
        }
        let Ok((200, _, xml, _)) = fetch(&m, MAX_SITEMAP_BYTES).await else {
            continue;
        };
        let (l, index) = locs(&xml);
        if index {
            maps.extend(l);
            continue;
        }
        for u in l {
            if let Ok(url) = Url::parse(&u)
                && hosts.contains(&Some(url.origin().ascii_serialization()))
            {
                let path = match url.query() {
                    Some(q) => format!("{}?{q}", url.path()),
                    None => url.path().to_string(),
                };
                if !pages.contains(&path) {
                    pages.push(path);
                }
            }
        }
        if pages.len() >= MAX_URLS {
            break;
        }
    }
    if pages.is_empty() {
        pages.push("/".into());
    }
    ORIGIN_PATHS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(origin, (Instant::now(), pages.clone()));
    pages
}

/// `urlset` of one locale: each page's localized URL + every alternate (reciprocal by construction).
pub fn urlset(site: &Site, locale: &str, paths: &[String]) -> String {
    let noop = |_: &str| None;
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
"#,
    );
    for path in paths {
        let ctx = site.seo_ctx(locale, path, &noop);
        // Only pages complete in this language: an incomplete one is a duplicate of the source for now.
        if !ctx.advertised(locale) {
            continue;
        }
        let Some(loc) = ctx.url_for(locale, path) else {
            continue;
        };
        s.push_str(&format!("<url><loc>{}</loc>", esc(&loc)));
        for (l, u) in ctx.alternates(path) {
            s.push_str(&format!(
                r#"<xhtml:link rel="alternate" hreflang="{}" href="{}"/>"#,
                esc(&l),
                esc(&u)
            ));
        }
        s.push_str("</url>\n");
    }
    s.push_str("</urlset>\n");
    s
}

/// `sitemapindex` of every advertised locale's sitemap (source included).
pub fn index(site: &Site) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
"#,
    );
    for (_, b) in &site.bases {
        s.push_str(&format!(
            "<sitemap><loc>{}/sitemap.xml</loc></sitemap>\n",
            esc(b)
        ));
    }
    s.push_str("</sitemapindex>\n");
    s
}

/// robots.txt of a translated host: the origin's rules, its `Sitemap:` lines replaced by this locale's sitemap.
pub fn robots(origin_robots: &str, sitemap_url: &str) -> String {
    let mut out: String = origin_robots
        .lines()
        .filter(|l| {
            !l.split_once(':')
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("sitemap"))
        })
        .map(|l| format!("{l}\n"))
        .collect();
    if out.trim().is_empty() {
        out = "User-agent: *\nAllow: /\n".into();
    }
    out.push_str(&format!("\nSitemap: {sitemap_url}\n"));
    out
}

/// IndexNow key of a project (stable, derived from the server secret; served at `/{key}.txt` on each host).
pub fn indexnow_key(state: &AppState, project: Uuid) -> String {
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(&state.secret).expect("hmac key");
    m.update(b"indexnow:");
    m.update(project.as_bytes());
    hex::encode(&m.finalize().into_bytes()[..16])
}

fn xml(body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (
                header::CACHE_CONTROL,
                "public, max-age=3600, stale-while-revalidate=86400",
            ),
        ],
        body,
    )
        .into_response()
}

fn text(body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (
                header::CACHE_CONTROL,
                "public, max-age=3600, stale-while-revalidate=86400",
            ),
        ],
        body,
    )
        .into_response()
}

/// Answers the SEO files of a translated host for the proxy / worker: `/sitemap.xml`, `/robots.txt`,
/// `/{indexnow key}.txt`. `path` is relative to the locale base (`/sitemap.xml` for `cliponit.app/es/sitemap.xml`).
/// `None` = not one of them (proxy as usual).
pub async fn special(
    state: &AppState,
    p: &ProjectRow,
    locale: &str,
    path: &str,
) -> Option<Response> {
    let key = indexnow_key(state, p.id);
    if path == format!("/{key}.txt") {
        return Some(text(key));
    }
    if path != "/sitemap.xml" && path != "/robots.txt" {
        return None;
    }
    let site = site(state, p, true).await.ok()?;
    let not_found = || Some(StatusCode::NOT_FOUND.into_response());
    let Some(base) = site.base(locale).map(str::to_string) else {
        return not_found();
    };
    if path == "/robots.txt" {
        let origin = match &site.origin {
            Some(o) => match fetch(&format!("{o}/robots.txt"), 256 * 1024).await {
                Ok((200, _, b, _)) => b,
                _ => String::new(),
            },
            None => String::new(),
        };
        return Some(text(robots(&origin, &format!("{base}/sitemap.xml"))));
    }
    let paths = source_paths(&site).await;
    Some(xml(urlset(&site, locale, &paths)))
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/edge/v1/{project}/sitemap.xml", get(serve_index))
        .route("/edge/v1/{project}/{locale}/sitemap.xml", get(serve_locale))
        .route("/edge/v1/{project}/{locale}/robots.txt", get(serve_robots))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/projects/{slug}/edge/indexnow", post(indexnow))
}

/// Sitemaps describe public pages of a site that has a verified Edge domain; anything else is a 404.
async fn edge_project(state: &AppState, slug: &str) -> ApiResult<(ProjectRow, Site)> {
    let p = projects::load_public(state, slug)
        .await?
        .ok_or_else(|| AppError::not_found("not found"))?;
    let site = site(state, &p, true).await?;
    if site.bases.is_empty() {
        return Err(AppError::not_found("no verified Edge domain"));
    }
    Ok((p, site))
}

async fn serve_index(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Response> {
    let (_, site) = edge_project(&state, &slug).await?;
    Ok(xml(index(&site)))
}

async fn serve_locale(
    State(state): State<AppState>,
    AppPath((slug, locale)): AppPath<(String, String)>,
) -> ApiResult<Response> {
    let (p, _) = edge_project(&state, &slug).await?;
    special(&state, &p, &locale, "/sitemap.xml")
        .await
        .ok_or_else(|| AppError::not_found("not found"))
}

async fn serve_robots(
    State(state): State<AppState>,
    AppPath((slug, locale)): AppPath<(String, String)>,
) -> ApiResult<Response> {
    let (p, _) = edge_project(&state, &slug).await?;
    special(&state, &p, &locale, "/robots.txt")
        .await
        .ok_or_else(|| AppError::not_found("not found"))
}

#[derive(Deserialize, Default)]
struct IndexNowReq {
    #[serde(default)]
    locale: Option<String>,
}

/// IndexNow endpoint (Bing, Yandex, Seznam, Naver share submissions). `GLOT_INDEXNOW_URL` overrides it (tests).
fn indexnow_url() -> String {
    std::env::var("GLOT_INDEXNOW_URL")
        .unwrap_or_else(|_| "https://api.indexnow.org/indexnow".into())
}

/// `POST /api/projects/{slug}/edge/indexnow {"locale"?}`: submits every sitemap URL of the ready locales (or one)
/// to IndexNow, one request per host. Bing / Yandex only: Google ignores IndexNow and reads the sitemaps.
async fn indexnow(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let req: IndexNowReq = if body.iter().all(u8::is_ascii_whitespace) {
        IndexNowReq::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| AppError::bad_request(e.to_string()))?
    };
    let site = site(&state, &p, true).await?;
    let paths = source_paths(&site).await;
    let key = indexnow_key(&state, p.id);
    let mut by_host: Vec<(String, Vec<String>)> = Vec::new();
    for (l, b) in &site.bases {
        if l == &site.source_locale || req.locale.as_ref().is_some_and(|x| x != l) {
            continue;
        }
        let Ok(u) = Url::parse(b) else { continue };
        let host = u.host_str().unwrap_or_default().to_string();
        // What the sitemap lists: complete pages, at their translated address.
        let urls: Vec<String> = paths
            .iter()
            .filter_map(|path| {
                let noop = |_: &str| None;
                let ctx = site.seo_ctx(l, path, &noop);
                ctx.advertised(l).then(|| ctx.url_for(l, path)).flatten()
            })
            .collect();
        match by_host.iter_mut().find(|(h, _)| *h == host) {
            Some((_, v)) => v.extend(urls),
            None => by_host.push((host, urls)),
        }
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let mut results = Vec::new();
    for (host, urls) in by_host {
        let n = urls.len().min(10_000);
        let scheme = scheme(&host);
        let payload = json!({"host": host, "key": key, "keyLocation": format!("{scheme}://{host}/{key}.txt"),
                             "urlList": &urls[..n]});
        let status = client
            .post(indexnow_url())
            .json(&payload)
            .send()
            .await
            .map(|r| r.status().as_u16())
            .ok();
        results.push(json!({"host": host, "urls": n, "status": status}));
    }
    Ok(Json(
        json!({"engines": ["bing", "yandex"], "google": "uses the sitemaps announced in robots.txt",
                   "key": key, "hosts": results}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site() -> Site {
        Site {
            source_locale: "fr".into(),
            bases: vec![
                ("fr".into(), "https://cliponit.app".into()),
                ("es".into(), "https://es.cliponit.app".into()),
            ],
            origin: Some("https://cliponit.app".into()),
            ..Default::default()
        }
    }

    #[test]
    fn sitemap_locale_and_index() {
        let x = urlset(&site(), "es", &["/".into(), "/pricing?a=1&b=2".into()]);
        assert!(
            x.contains("<url><loc>https://es.cliponit.app/</loc>"),
            "{x}"
        );
        assert!(x.contains(r#"<xhtml:link rel="alternate" hreflang="fr" href="https://cliponit.app/pricing?a=1&amp;b=2"/>"#), "{x}");
        assert!(x.contains(r#"hreflang="x-default" href="https://cliponit.app/""#));
        assert_eq!(x.matches("<url>").count(), 2);
        let i = index(&site());
        assert!(i.contains("<loc>https://es.cliponit.app/sitemap.xml</loc>"));
        assert!(i.contains("<loc>https://cliponit.app/sitemap.xml</loc>"));
    }

    #[test]
    fn parse_and_robots() {
        let (l, idx) = locs(
            "<urlset><url><loc> https://x.io/a?b=1&amp;c=2 </loc></url><url><loc><![CDATA[https://x.io/b]]></loc></url></urlset>",
        );
        assert_eq!(l, ["https://x.io/a?b=1&c=2", "https://x.io/b"]);
        assert!(!idx);
        assert!(
            locs("<sitemapindex><sitemap><loc>https://x.io/s1.xml</loc></sitemap></sitemapindex>")
                .1
        );
        let r = robots(
            "User-agent: *\nDisallow: /admin\nSitemap: https://cliponit.app/sitemap.xml\n",
            "https://es.cliponit.app/sitemap.xml",
        );
        assert!(r.contains("Disallow: /admin"));
        assert!(!r.contains("https://cliponit.app/sitemap.xml"));
        assert!(r.ends_with("Sitemap: https://es.cliponit.app/sitemap.xml\n"));
        assert!(robots("", "https://e/s.xml").starts_with("User-agent: *\nAllow: /\n"));
    }

    #[test]
    fn schemes() {
        assert_eq!(scheme("es.cliponit.app"), "https");
        assert_eq!(scheme("127.0.0.1:8080"), "http");
        assert_eq!(scheme("localhost"), "http");
    }
}
