//! No-code setup of the universal website tag (`widget/glot.js`), driven from the dashboard with nothing installed:
//! - `POST /api/projects/{slug}/edge/website/scan {"url"}`: one fetch of the public page → platform (the matching
//!   `/integrations/{id}` guide), language, whether the tag is installed (right project, key, in `<head>`), whether
//!   the site's CSP blocks it, other translation scripts, private-looking paths to exclude (`/account`, `/cart`…:
//!   HttpOnly session cookies are invisible to the tag, so paths are the safe default).
//! - `POST …/website/crawl {"url", "pages"?}` (admin): pre-translation before go-live. Homepage links + sitemap,
//!   at most `MAX_PAGES` pages, fetched anonymously; their segments go through `missing::record_trusted` for every
//!   offered locale (same checks, daily cap and budget as visitor discovery), so the first visitor in a new language
//!   already has a TM. Remembers the site (`website_snippet.site`).
//! - `GET …/website/page?url=&locale=`: the source HTML of a page of that site + the locale's TM, for the
//!   dashboard's static preview / click-to-edit (rendered in a script-less sandbox, `glot-editor.js`).
//!
//! Every fetch goes through `sitemap::fetch`: public addresses only (pinned), redirects re-checked hop by hop,
//! timeouts, capped bodies. Separate from the Edge pre-warm crawler on purpose (this one runs once, at setup).
// ponytail: the crawl runs inside the request (≤ 25 s, 4 fetches at a time); a job with progress when sites
// need more than MAX_PAGES pages.
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use glot_core::edge::{
    hydration::is_skeleton,
    segment::{SegmentOpts, segment},
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};

use super::{discover::Settings, missing, sitemap};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

const MAX_PAGES: usize = 50;
const DEFAULT_PAGES: usize = 20;
const MAX_HTML: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(25);

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/edge/website";
    Router::new()
        .route(&format!("{p}/scan"), post(scan))
        .route(&format!("{p}/crawl"), post(crawl))
        .route(&format!("{p}/page"), get(page))
}

/// Paths the tag never learns text from unless the project lists them in `discover_include`: areas that are
/// per-visitor on every site builder. A visitor's session cookie is often HttpOnly (invisible to the tag), so
/// the path is the guard. The setup crawl (anonymous, no cookie) may still read their public shell.
// ponytail: first path segment only (`/fr/account` passes); localized prefixes when a builder needs them.
pub const PRIVATE_PATHS: &[&str] = &[
    "account",
    "my-account",
    "members",
    "member",
    "members-area",
    "profile",
    "checkout",
    "checkouts",
    "cart",
    "basket",
    "orders",
    "order",
    "dashboard",
    "admin",
    "wp-admin",
    "billing",
    "settings",
    "user",
    "users",
    "customer",
    "customers",
];

/// `path` is under one of [`PRIVATE_PATHS`].
pub fn private_path(path: &str) -> bool {
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    PRIVATE_PATHS.iter().any(|p| first.eq_ignore_ascii_case(p))
}

/// Platform signatures, most specific first: `(id in content/integrations/platforms.json, needles)`; a needle
/// is searched in the lowercased HTML and response header names/values.
const PLATFORMS: &[(&str, &[&str])] = &[
    (
        "wix",
        &[
            "static.wixstatic.com",
            "x-wix-request-id",
            "wix.com website builder",
        ],
    ),
    (
        "squarespace",
        &[
            "static1.squarespace.com",
            "this is squarespace",
            "squarespace-cdn.com",
        ],
    ),
    ("webflow", &["data-wf-site", "content=\"webflow\""]),
    ("shopify", &["cdn.shopify.com", "x-shopid", "shopify.theme"]),
    ("framer", &["framerusercontent.com", "content=\"framer"]),
    ("bubble", &["bubble_page_load_data", "cdn.bubble.io"]),
    ("woocommerce", &["woocommerce", "wc-ajax"]),
    (
        "wordpress",
        &["/wp-content/", "/wp-includes/", "content=\"wordpress"],
    ),
    ("duda", &["cdn-website.com", "dudaone"]),
    ("jimdo", &["jimstatic.com", "jimdo"]),
    ("weebly-square", &["editmysite.com", "weebly.com"]),
    ("ghost", &["content=\"ghost"]),
    ("hubspot-cms", &["content=\"hubspot", "hs-scripts.com"]),
    ("drupal", &["content=\"drupal", "data-drupal-"]),
    ("lovable", &["lovable.dev", "lovable-uploads"]),
    ("bigcommerce", &["bigcommerce.com"]),
    ("prestashop", &["content=\"prestashop", "var prestashop"]),
    ("magento", &["mage/cookies", "magento_"]),
    ("shopware", &["shopware"]),
    ("typo3", &["content=\"typo3"]),
    ("softr", &["softr.io"]),
    ("kajabi", &["kajabi"]),
    ("thinkific", &["thinkific"]),
    ("teachable", &["teachable"]),
    ("carrd", &["carrd.co"]),
];

/// Other translation scripts on the page: two tools rewriting the same text fight each other.
const OTHER_TOOLS: &[(&str, &str)] = &[
    ("weglot", "cdn.weglot.com"),
    ("conveythis", "conveythis.com/javascript"),
    ("gtranslate", "gtranslate.net"),
    ("bablic", "bablic.com/js"),
    ("localize", "global.localizecdn.com"),
    ("linguise", "linguise.com"),
    (
        "google-translate",
        "translate.google.com/translate_a/element.js",
    ),
];

#[derive(Serialize, Debug, Default)]
pub struct Scan {
    /// Final URL (after redirects).
    pub url: String,
    pub status: Option<u16>,
    /// `invalid` | `private` | `unreachable` | `timeout` | `not_html`
    pub error: Option<&'static str>,
    pub platform: Option<&'static str>,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub tag: Tag,
    /// CSP directives of the site that block the tag (`script-src`, `connect-src`); empty = not blocked.
    pub csp_blocked: Vec<&'static str>,
    pub other_tools: Vec<&'static str>,
    /// Private-looking paths linked from the page, as discovery exclusion globs (`/account*`).
    pub suggested_exclude: Vec<String>,
    /// The HTML has almost no text before JavaScript runs: the static preview will be empty; the tag itself
    /// (in the visitor's browser) is not affected.
    pub js_only: bool,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Tag {
    pub found: bool,
    /// `data-project` of the tag found.
    pub project: Option<String>,
    pub right_project: bool,
    /// `data-key` present (without it the tag translates but never reports new text).
    pub key: bool,
    pub in_head: bool,
}

/// An attribute's value in one start tag (quoted or bare).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pat = format!("{name}=");
    let (i, _) = lower
        .match_indices(&pat)
        .find(|(i, _)| *i > 0 && lower.as_bytes()[i - 1].is_ascii_whitespace())?;
    let v = &tag[i + pat.len()..];
    let q = v.chars().next()?;
    let v = if q == '"' || q == '\'' {
        v[1..].split(q).next()?
    } else {
        v.split(|c: char| c.is_whitespace() || c == '>').next()?
    };
    Some(v.replace("&amp;", "&"))
}

/// Every start tag `<name …>` of `html` (lowercased copy in `lower`), with its byte offset.
fn tags<'a>(html: &'a str, lower: &str, name: &str) -> Vec<(usize, &'a str)> {
    let open = format!("<{name}");
    lower
        .match_indices(&open)
        .filter(|(i, _)| {
            lower
                .as_bytes()
                .get(i + open.len())
                .is_some_and(|c| c.is_ascii_whitespace() || *c == b'>')
        })
        .map(|(i, _)| {
            let end = lower[i..].find('>').map_or(html.len(), |e| i + e);
            (i, &html[i..end])
        })
        .collect()
}

fn find_tag(html: &str, lower: &str, slug: &str) -> Tag {
    let head_end = lower.find("</head>").unwrap_or(lower.len());
    for (at, t) in tags(html, lower, "script") {
        if attr(t, "src").is_some_and(|s| s.contains("/widget/glot.js")) {
            let project = attr(t, "data-project");
            return Tag {
                found: true,
                right_project: project.as_deref() == Some(slug),
                project,
                key: attr(t, "data-key").is_some_and(|k| !k.is_empty()),
                in_head: at < head_end,
            };
        }
    }
    Tag::default()
}

/// Does one CSP source list let `host` through? (`*`, a scheme, the host, a `*.domain` wildcard.)
fn source_list_allows(list: &str, host: &str) -> bool {
    list.split_ascii_whitespace().any(|s| {
        let s = s.trim_end_matches('/').to_ascii_lowercase();
        let h = s
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        s == "*"
            || s == "https:"
            || h == host
            || h.strip_prefix("*.")
                .is_some_and(|d| host.ends_with(&format!(".{d}")))
    })
}

/// Directives of `policies` (header and `<meta>` CSPs; each must allow) that block the tag: its script from
/// `app_host`, its requests to `api_host`.
pub fn csp_blocked(policies: &[String], app_host: &str, api_host: &str) -> Vec<&'static str> {
    let mut out = vec![];
    for (name, fallbacks, host) in [
        (
            "script-src",
            ["script-src-elem", "script-src", "default-src"],
            app_host,
        ),
        (
            "connect-src",
            ["connect-src", "connect-src", "default-src"],
            api_host,
        ),
    ] {
        let blocked = policies.iter().any(|p| {
            let dirs: HashMap<String, &str> = p
                .split(';')
                .filter_map(|d| {
                    let d = d.trim();
                    let (k, v) = d.split_once(char::is_whitespace).unwrap_or((d, ""));
                    (!k.is_empty()).then(|| (k.to_ascii_lowercase(), v))
                })
                .collect();
            fallbacks
                .iter()
                .find_map(|f| dirs.get(*f))
                .is_some_and(|list| !source_list_allows(list, host))
        });
        if blocked {
            out.push(name);
        }
    }
    out
}

/// Same-origin page paths linked from `html` (no query, no assets), in document order.
fn links(html: &str, lower: &str, page: &Url) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for (_, t) in tags(html, lower, "a") {
        let Some(u) = attr(t, "href").and_then(|h| page.join(h.trim()).ok()) else {
            continue;
        };
        let path = u.path().to_string();
        let asset = path
            .rsplit('/')
            .next()
            .and_then(|f| f.rsplit_once('.'))
            .is_some_and(|(_, ext)| !matches!(ext, "html" | "htm" | "php" | "aspx"));
        if u.origin() == page.origin() && !asset && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// What one fetched page tells (pure: no I/O).
pub fn inspect(
    html: &str,
    headers: &[(String, String)],
    page: &Url,
    slug: &str,
    app_host: &str,
    api_host: &str,
) -> Scan {
    let lower = html.to_ascii_lowercase();
    let head: String = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\n"))
        .collect::<String>()
        .to_ascii_lowercase();
    let has = |n: &str| lower.contains(n) || head.contains(n);
    let mut policies: Vec<String> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("content-security-policy"))
        .map(|(_, v)| v.clone())
        .collect();
    policies.extend(tags(html, &lower, "meta").into_iter().filter_map(|(_, t)| {
        attr(t, "http-equiv")
            .filter(|h| h.eq_ignore_ascii_case("content-security-policy"))
            .and_then(|_| attr(t, "content"))
    }));
    let mut suggested: Vec<String> = links(html, &lower, page)
        .iter()
        .filter(|p| private_path(p))
        .map(|p| {
            let first = p.trim_start_matches('/').split('/').next().unwrap_or("");
            format!("/{}*", first.to_ascii_lowercase())
        })
        .collect();
    suggested.sort();
    suggested.dedup();
    Scan {
        url: page.to_string(),
        platform: PLATFORMS
            .iter()
            .find(|(_, needles)| needles.iter().any(|n| has(n)))
            .map(|(id, _)| *id),
        lang: tags(html, &lower, "html")
            .first()
            .and_then(|(_, t)| attr(t, "lang"))
            .filter(|l| !l.is_empty()),
        title: lower.find("<title").and_then(|i| {
            let rest = &html[i..];
            let s = rest.find('>')? + 1;
            let e = rest.to_ascii_lowercase().find("</title")?;
            (s <= e).then(|| rest[s..e].trim().chars().take(200).collect())
        }),
        tag: find_tag(html, &lower, slug),
        csp_blocked: csp_blocked(&policies, app_host, api_host),
        other_tools: OTHER_TOOLS
            .iter()
            .filter(|(_, n)| lower.contains(n))
            .map(|(id, _)| *id)
            .collect(),
        suggested_exclude: suggested,
        js_only: is_skeleton(html),
        ..Default::default()
    }
}

/// A public http(s) URL typed by a person (`example.com` → `https://example.com/`).
fn clean_url(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 500 {
        return None;
    }
    let u = if raw.contains("://") {
        Url::parse(raw)
    } else {
        Url::parse(&format!("https://{raw}"))
    }
    .ok()?;
    (matches!(u.scheme(), "http" | "https") && u.host_str().is_some()).then_some(u)
}

struct Fetched {
    url: Url,
    status: u16,
    headers: Vec<(String, String)>,
    html: String,
}

async fn fetch_html(url: &Url) -> Result<Fetched, &'static str> {
    let (status, headers, html, url) = sitemap::fetch(url.as_str(), MAX_HTML).await?;
    let headers: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let is_html = headers
        .iter()
        .any(|(k, v)| k == "content-type" && v.contains("html"));
    if status < 400 && !is_html {
        return Err("not_html");
    }
    Ok(Fetched {
        url,
        status,
        headers,
        html,
    })
}

/// (script host, API host) of this glot: the dashboard origin, and `api.` for `app.` (the tag's own default).
fn glot_hosts() -> (String, String) {
    let app = Url::parse(crate::review_live::web_origin())
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let api = match app.strip_prefix("app.") {
        Some(rest) => format!("api.{rest}"),
        None => app.clone(),
    };
    (app, api)
}

#[derive(Deserialize)]
struct UrlBody {
    url: String,
    #[serde(default)]
    pages: Option<usize>,
}

async fn scan(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<UrlBody>,
) -> ApiResult<Json<Scan>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let Some(url) = clean_url(&b.url) else {
        return Ok(Json(Scan {
            url: b.url,
            error: Some("invalid"),
            ..Default::default()
        }));
    };
    Ok(Json(match fetch_html(&url).await {
        Ok(f) => {
            let (app, api) = glot_hosts();
            Scan {
                status: Some(f.status),
                ..inspect(&f.html, &f.headers, &f.url, &p.slug, &app, &api)
            }
        }
        Err(e) => Scan {
            url: url.to_string(),
            error: Some(e),
            ..Default::default()
        },
    }))
}

#[derive(Serialize, Debug, Default)]
struct Crawl {
    /// Pages listed (homepage links + sitemap) and pages actually fetched and read.
    found: usize,
    pages: usize,
    /// Distinct translatable texts read.
    segments: usize,
    /// New texts stored, per locale summed; `rejected` = personal data, excluded zones, over the daily cap.
    accepted: usize,
    known: usize,
    rejected: usize,
    /// Locales with a translation job queued or running.
    queued: Vec<String>,
    locales: Vec<String>,
    error: Option<&'static str>,
}

async fn crawl(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<UrlBody>,
) -> ApiResult<Json<Crawl>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let url = clean_url(&b.url).ok_or_else(|| AppError::bad_request("invalid site URL"))?;
    let max = b.pages.unwrap_or(DEFAULT_PAGES).clamp(1, MAX_PAGES);
    let home = match fetch_html(&url).await {
        Ok(f) if f.status < 400 => f,
        Ok(_) => {
            return Ok(Json(Crawl {
                error: Some("unreachable"),
                ..Default::default()
            }));
        }
        Err(e) => {
            return Ok(Json(Crawl {
                error: Some(e),
                ..Default::default()
            }));
        }
    };
    let origin = home.url.origin().ascii_serialization();
    // Remember the site: the preview only ever fetches pages of this origin.
    sqlx::query(
        "INSERT INTO edge_settings (project_id, website_snippet)
         VALUES ($1, '{\"on\": false, \"auto_translate\": true, \"position\": \"bottom-right\", \"locales\": []}'::jsonb
                     || jsonb_build_object('site', $2::text))
         ON CONFLICT (project_id) DO UPDATE
           SET website_snippet = edge_settings.website_snippet || jsonb_build_object('site', $2::text)",
    )
    .bind(p.id)
    .bind(&origin)
    .execute(&state.db)
    .await?;
    let settings = Settings::load(&state.db, p.id).await?;

    // Pages: the one given, what it links to (the navigation: the pages that matter), then the sitemap.
    let lower = home.html.to_ascii_lowercase();
    let mut paths = vec![home.url.path().to_string()];
    paths.extend(links(&home.html, &lower, &home.url));
    if paths.len() < max
        && let Ok((200, _, xml, _)) =
            sitemap::fetch(&format!("{origin}/sitemap.xml"), MAX_HTML).await
    {
        let (mut locs, index) = sitemap::locs(&xml);
        if index && let Some(first) = locs.first().cloned() {
            // One level of index, first child only (usually the pages sitemap).
            locs = match sitemap::fetch(&first, MAX_HTML).await {
                Ok((200, _, xml, u)) if u.origin().ascii_serialization() == origin => {
                    sitemap::locs(&xml).0
                }
                _ => vec![],
            };
        }
        paths.extend(
            locs.iter()
                .filter_map(|l| Url::parse(l).ok())
                .filter(|u| u.origin().ascii_serialization() == origin)
                .map(|u| u.path().to_string()),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    paths.retain(|p| settings.discoverable(p) && seen.insert(p.clone()));
    let found = paths.len();
    paths.truncate(max);

    let skip: Vec<&str> = settings
        .selectors_exclude
        .iter()
        .map(String::as_str)
        .collect();
    // ponytail: no do-not-translate glossary here; a segment made only of such terms gets translated to itself.
    let opts = Arc::new(SegmentOpts::new(&p.source_locale).skip(&skip));
    let read =
        |path: &str, html: &str, opts: &SegmentOpts| -> (Vec<missing::Item>, missing::Page) {
            let segs = segment(html, opts);
            let page = missing::Page {
                url: path.to_string(),
                seen: segs
                    .iter()
                    .map(|s| missing::Seen {
                        hash: s.hash.clone(),
                        role: s.ctx.role.clone(),
                    })
                    .collect(),
            };
            let items = segs
                .into_iter()
                .filter(|s| s.discoverable && s.key.is_none())
                .map(|s| missing::Item {
                    hash: s.hash,
                    text: s.text,
                    url: path.to_string(),
                    role: s.ctx.role,
                })
                .collect();
            (items, page)
        };
    let mut pages = vec![read(&paths[0], &home.html, &opts)];
    let gate = Arc::new(tokio::sync::Semaphore::new(4));
    let mut set = tokio::task::JoinSet::new();
    for path in paths.into_iter().skip(1) {
        let (gate, opts, origin) = (gate.clone(), opts.clone(), origin.clone());
        set.spawn(async move {
            let _permit = gate.acquire().await.ok()?;
            let u = Url::parse(&format!("{origin}{path}")).ok()?;
            let f = fetch_html(&u).await.ok()?;
            // A redirect out of the site (or to a sign-in page elsewhere) is not this site's text.
            (f.status == 200 && f.url.origin().ascii_serialization() == origin)
                .then(|| read(&path, &f.html, &opts))
        });
    }
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while let Ok(Some(r)) = tokio::time::timeout_at(deadline, set.join_next()).await {
        if let Ok(Some(x)) = r {
            pages.push(x);
        }
    }
    set.abort_all();

    let offered = &settings.website_snippet.locales;
    let locales: Vec<String> = p
        .target_locales
        .iter()
        .filter(|l| offered.is_empty() || offered.contains(l))
        .cloned()
        .collect();
    let mut out = Crawl {
        found,
        pages: pages.len(),
        segments: pages
            .iter()
            .flat_map(|(items, _)| items.iter().map(|i| i.hash.as_str()))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        locales: locales.clone(),
        ..Default::default()
    };
    let (segments, inventories): (Vec<_>, Vec<_>) = pages.into_iter().unzip();
    let segments: Vec<missing::Item> = segments.into_iter().flatten().collect();
    let mut inventories = Some(inventories);
    for locale in locales {
        let r = missing::Report {
            key: None,
            locale: locale.clone(),
            auth: false,
            segments: segments.clone(),
            // Page inventories are per project, not per locale: once.
            pages: inventories.take().unwrap_or_default(),
            mode: Some("website".into()),
        };
        let o = missing::record_trusted(&state, &p, &r).await?;
        // Segments are shared by every locale: only the first report stores them.
        out.accepted += o.accepted;
        out.known = out.known.max(o.known);
        out.rejected = out.rejected.max(o.rejected);
        if o.queued {
            out.queued.push(locale);
        }
    }
    Ok(Json(out))
}

async fn page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<HashMap<String, String>>,
) -> ApiResult<Json<serde_json::Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let settings = Settings::load(&state.db, p.id).await?;
    let locale = q.get("locale").cloned().unwrap_or_default();
    if !p.target_locales.contains(&locale) {
        return Err(AppError::bad_request(
            "locale is not a target locale of this project",
        ));
    }
    // Only pages of the site an admin set up: this endpoint is not a fetch-anything proxy.
    let site = Url::parse(&settings.website_snippet.site)
        .map_err(|_| AppError::bad_request("no site set up yet (run the setup first)"))?;
    let url = q
        .get("url")
        .and_then(|u| site.join(u).ok())
        .filter(|u| u.origin() == site.origin())
        .ok_or_else(|| AppError::bad_request("url must be a page of the project's site"))?;
    let f = fetch_html(&url)
        .await
        .map_err(|e| AppError::new(axum::http::StatusCode::BAD_GATEWAY, "site_unreachable", e))?;
    if f.url.origin() != site.origin() {
        return Err(AppError::bad_request(
            "the page redirects outside the project's site",
        ));
    }
    let tm = super::tm::snapshot(&state.db, &p, &locale).await?;
    Ok(Json(json!({
        "url": f.url.as_str(),
        "path": f.url.path(),
        "status": f.status,
        "html": f.html,
        "source": p.source_locale,
        "skip": settings.selectors_exclude,
        "tm": tm,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_a_builder_page() {
        let html = r#"<!doctype html><html lang="en-US"><head><title> Atelier </title>
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' https://app.glot.test">
<link href="https://static.wixstatic.com/x.css" rel="stylesheet">
<script async data-key="pk_1" data-project="shop" src="https://app.glot.test/widget/glot.js"></script>
<script src="https://cdn.weglot.com/weglot.min.js"></script></head>
<body><a href="/about">About</a> <a href="/account/orders">Orders</a> <a href='/Cart'>Cart</a>
<a href="https://other.test/account">x</a> <a href="/menu.pdf">Menu</a></body></html>"#;
        let page = Url::parse("https://shop.test/").unwrap();
        let s = inspect(html, &[], &page, "shop", "app.glot.test", "api.glot.test");
        assert_eq!(s.platform, Some("wix"));
        assert_eq!(s.lang.as_deref(), Some("en-US"));
        assert_eq!(s.title.as_deref(), Some("Atelier"));
        assert_eq!(
            s.tag,
            Tag {
                found: true,
                project: Some("shop".into()),
                right_project: true,
                key: true,
                in_head: true
            }
        );
        // script-src lets the script in; connect-src falls back to default-src 'self': the API is blocked.
        assert_eq!(s.csp_blocked, vec!["connect-src"]);
        assert_eq!(s.other_tools, vec!["weglot"]);
        assert_eq!(s.suggested_exclude, vec!["/account*", "/cart*"]);
        assert!(s.js_only);
        assert_eq!(
            links(html, &html.to_ascii_lowercase(), &page),
            vec!["/about", "/account/orders", "/Cart"]
        );

        let other = inspect(
            html,
            &[],
            &page,
            "another",
            "app.glot.test",
            "api.glot.test",
        );
        assert!(other.tag.found && !other.tag.right_project);
        let none = inspect(
            "<html><body><p>Hi</p></body></html>",
            &[("X-ShopId".into(), "1".into())],
            &page,
            "shop",
            "a",
            "b",
        );
        assert_eq!(
            (none.tag.found, none.platform, none.csp_blocked.len()),
            (false, Some("shopify"), 0)
        );
    }

    #[test]
    fn csp_and_private_paths() {
        let p = |s: &str| vec![s.to_string()];
        assert!(csp_blocked(&p("img-src *"), "app.x.io", "api.x.io").is_empty());
        assert!(csp_blocked(&p("default-src https:"), "app.x.io", "api.x.io").is_empty());
        assert!(
            csp_blocked(
                &p("script-src *.x.io; connect-src https://api.x.io"),
                "app.x.io",
                "api.x.io"
            )
            .is_empty()
        );
        assert_eq!(
            csp_blocked(
                &p("script-src 'self' 'unsafe-inline'"),
                "app.x.io",
                "api.x.io"
            ),
            vec!["script-src"]
        );
        assert_eq!(
            csp_blocked(&p("default-src 'self'"), "app.x.io", "api.x.io"),
            vec!["script-src", "connect-src"]
        );
        for (path, private) in [
            ("/account", true),
            ("/Account/orders", true),
            ("/cart", true),
            ("/", false),
            ("/accounting", false),
            ("/blog/account", false),
            ("/login", false),
        ] {
            assert_eq!(private_path(path), private, "{path}");
        }
        assert_eq!(
            clean_url("shop.test/a").unwrap().as_str(),
            "https://shop.test/a"
        );
        assert!(clean_url("ftp://x.io").is_none() && clean_url("").is_none());
    }
}
