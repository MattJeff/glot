//! glot Edge — per-page coverage, language readiness and the international SEO report.
//!
//! **Coverage per page, not per key**: a page's coverage is the share of the segments it renders
//! (`edge_page_segments`, reported by the worker / proxy / patcher and by this crawler) that the served TM
//! snapshot translates. Critical pages (`edge_settings.critical_patterns`: home, pricing, signup, checkout… by
//! default) must each reach `critical_threshold` (98 %) for a language to be **ready**; the global coverage
//! (catalog keys + rendered segments) must reach the caller's `min` (the switcher asks 0.95, search engines get
//! [`super::sitemap::SEO_MIN`]). Not ready = not in the switcher manifest, no hreflang, no sitemap.
//!
//! **Report** (`POST /api/projects/{slug}/edge/seo-report`): for critical pages then the busiest ones, every
//! locale's URL is fetched raw (no JavaScript, SSRF-safe) and checked: HTTP status, `<html lang dir>`,
//! `Content-Language`, one self canonical, hreflang (self, x-default, reciprocity across the cluster), title /
//! description / og:title translated, JSON-LD `inLanguage`, robots (meta, `X-Robots-Tag`, robots.txt),
//! segments still in the source language, and initial HTML vs the renderer's rendered DOM (content that only
//! exists after JavaScript). "Technically indexable" is computed; "effectively indexed" stays `null` (unknown)
//! until Search Console is connected — never guessed.
use axum::{Json, Router, extract::State, routing::get};
use glot_core::edge::{
    segment::{SegmentOpts, segment},
    seo::{PageSeo, inspect},
    tm::Tm,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use super::discover::Settings;
use super::sitemap::{fetch, site, source_paths};
use crate::{
    ApiResult, AppError, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
};

const MAX_PAGES: usize = 20;
const MAX_MISSING: usize = 100;
const PAGE_BYTES: usize = 2 * 1024 * 1024;

/// Readiness of one target locale.
#[derive(Debug, Clone, Serialize)]
pub struct Readiness {
    pub locale: String,
    pub keys: i64,
    pub keys_done: i64,
    /// Distinct segments rendered by the site's pages / those the served TM translates.
    pub segments: i64,
    pub segments_done: i64,
    /// Worst critical page's coverage; None when no critical page has been seen yet.
    pub critical_coverage: Option<f64>,
    pub critical_threshold: f64,
    /// `edge_catalog_version` of (project, locale).
    pub version: i64,
}

impl Readiness {
    /// Catalog keys and rendered segments together; 0 when there is nothing at all.
    pub fn coverage(&self) -> f64 {
        let (n, d) = (
            self.keys + self.segments,
            self.keys_done.min(self.keys) + self.segments_done,
        );
        if n == 0 { 0.0 } else { d as f64 / n as f64 }
    }

    pub fn critical_ok(&self) -> bool {
        self.critical_coverage
            .is_none_or(|c| c + 1e-9 >= self.critical_threshold)
    }

    pub fn ready(&self, min: f64) -> bool {
        self.coverage() + 1e-9 >= min && self.critical_ok()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Missing {
    pub hash: String,
    pub context: String,
    /// None when the segment was seen but never discovered (outside the discovery zones, personal data…).
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageCoverage {
    pub path: String,
    pub critical: bool,
    pub segments: usize,
    pub translated: usize,
    pub coverage: f64,
    /// Complete enough to be advertised to search engines ([`page_ready`]).
    pub ready: bool,
    pub missing: Vec<Missing>,
}

/// Segments of each page's latest render: rows seen within an hour of the page's last report (a segment removed
/// from the page stops counting once the page is reported again).
async fn page_rows(
    state: &AppState,
    p: &ProjectRow,
) -> sqlx::Result<Vec<(String, String, String, Option<String>)>> {
    sqlx::query_as(
        "SELECT ps.path, ps.hash, ps.context, s.source_text
         FROM (SELECT *, max(last_seen) OVER (PARTITION BY path) AS latest
               FROM edge_page_segments WHERE project_id = $1) ps
         LEFT JOIN edge_segments s ON s.project_id = ps.project_id AND s.hash = ps.hash AND s.context = ps.context
         WHERE ps.last_seen >= ps.latest - interval '1 hour'
         ORDER BY ps.path, ps.hash, ps.context",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await
}

fn coverage_of(
    rows: &[(String, String, String, Option<String>)],
    tm: &Tm,
    settings: &Settings,
    full: bool,
) -> Vec<PageCoverage> {
    let mut pages: BTreeMap<&str, PageCoverage> = BTreeMap::new();
    for (path, hash, ctx, text) in rows {
        let critical = settings.critical(path);
        if !full && !critical {
            continue;
        }
        let pc = pages.entry(path).or_insert_with(|| PageCoverage {
            path: path.clone(),
            critical,
            segments: 0,
            translated: 0,
            coverage: 0.0,
            ready: false,
            missing: vec![],
        });
        pc.segments += 1;
        if tm.lookup(hash, ctx).is_some() {
            pc.translated += 1;
        } else if pc.missing.len() < MAX_MISSING {
            pc.missing.push(Missing {
                hash: hash.clone(),
                context: ctx.clone(),
                text: text.clone(),
            });
        }
    }
    pages
        .into_values()
        .map(|mut p| {
            p.coverage = if p.segments == 0 {
                1.0
            } else {
                p.translated as f64 / p.segments as f64
            };
            let known: Vec<&Missing> = p.missing.iter().filter(|m| m.text.is_some()).collect();
            // `missing` is capped: a page that hit the cap is far from complete anyway.
            p.ready = p.missing.len() < MAX_MISSING
                && page_ready(
                    p.segments,
                    known.len(),
                    known.iter().any(|m| m.context.starts_with("title|")),
                );
            p
        })
        .collect()
}

/// Per locale, the pages complete enough to be advertised to search engines ([`page_ready`]).
pub type ReadyPages = BTreeMap<String, BTreeSet<String>>;

/// A page is advertised (self canonical, hreflang, sitemap) once its `<title>` is translated and at most
/// `1 - SEO_MIN` of its segments still wait for a translation. `missing` = segments glot knows and has not
/// translated yet (text it never accepted — personal data, outside the discovery zones — does not block).
pub fn page_ready(segments: usize, missing: usize, title_missing: bool) -> bool {
    !title_missing
        && segments > 0
        && (segments - missing.min(segments)) as f64 + 1e-9
            >= super::sitemap::SEO_MIN * segments as f64
}

/// Readiness of every target locale (TM snapshot per locale: exactly what the edge serves).
pub async fn readiness(state: &AppState, p: &ProjectRow) -> ApiResult<Vec<Readiness>> {
    Ok(readiness_pages(state, p).await?.0)
}

/// [`readiness`] and, from the same rows and snapshots, each locale's complete pages.
pub async fn readiness_pages(
    state: &AppState,
    p: &ProjectRow,
) -> ApiResult<(Vec<Readiness>, ReadyPages)> {
    let mut ready_pages = ReadyPages::new();
    let settings = Settings::load(&state.db, p.id).await?;
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM keys WHERE project_id = $1")
        .bind(p.id)
        .fetch_one(&state.db)
        .await?;
    let done: Vec<(String, i64)> = sqlx::query_as(
        "SELECT t.locale, count(*) FILTER (WHERE t.status <> 'stale')
         FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND t.locale = ANY($2) GROUP BY t.locale",
    )
    .bind(p.id)
    .bind(&p.target_locales)
    .fetch_all(&state.db)
    .await?;
    let rows = page_rows(state, p).await?;
    let distinct: BTreeSet<(&str, &str)> =
        rows.iter().map(|r| (r.1.as_str(), r.2.as_str())).collect();
    let mut out = Vec::new();
    for l in &p.target_locales {
        let (tm, segments_done, critical) = if rows.is_empty() {
            (Tm::default(), 0, None)
        } else {
            let tm = super::tm::snapshot(&state.db, p, l).await?;
            let segs_done = distinct
                .iter()
                .filter(|(h, c)| tm.lookup(h, c).is_some())
                .count() as i64;
            let pages = coverage_of(&rows, &tm, &settings, true);
            let critical = pages
                .iter()
                .filter(|c| c.critical)
                .map(|c| c.coverage)
                .reduce(f64::min);
            ready_pages.insert(
                l.clone(),
                pages
                    .into_iter()
                    .filter(|c| c.ready)
                    .map(|c| c.path)
                    .collect(),
            );
            (tm, segs_done, critical)
        };
        let version = if tm.v > 0 {
            tm.v
        } else {
            super::tm::version(&state.db, p.id, l).await?
        };
        out.push(Readiness {
            locale: l.clone(),
            keys,
            keys_done: done.iter().find(|d| &d.0 == l).map_or(0, |d| d.1),
            segments: distinct.len() as i64,
            segments_done,
            critical_coverage: critical.map(|c| (c * 1000.0).floor() / 1000.0),
            critical_threshold: settings.critical_threshold as f64,
            version,
        });
    }
    Ok((out, ready_pages))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/projects/{slug}/edge/coverage", get(coverage))
        .route(
            "/api/projects/{slug}/edge/seo-report",
            get(last_report).post(run_report),
        )
        .route("/api/projects/{slug}/edge/hreflang", get(hreflang_snippet))
}

#[derive(Deserialize)]
struct CoverageQuery {
    locale: Option<String>,
}

/// `GET /api/projects/{slug}/edge/coverage[?locale=es]`: per locale, readiness and every page's coverage with
/// its missing segments (critical pages first, then lowest coverage).
async fn coverage(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<CoverageQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let settings = Settings::load(&state.db, p.id).await?;
    let rows = page_rows(&state, &p).await?;
    let mut locales = Vec::new();
    for r in readiness(&state, &p).await? {
        if q.locale.as_ref().is_some_and(|l| *l != r.locale) {
            continue;
        }
        let tm = super::tm::snapshot(&state.db, &p, &r.locale).await?;
        let mut pages = coverage_of(&rows, &tm, &settings, true);
        pages.sort_by(|a, b| {
            b.critical
                .cmp(&a.critical)
                .then(a.coverage.total_cmp(&b.coverage))
        });
        locales.push(json!({
            "locale": r.locale, "coverage": round(r.coverage()), "critical_coverage": r.critical_coverage,
            "critical_threshold": r.critical_threshold, "ready": r.ready(super::sitemap::SEO_MIN),
            // Pages a search engine is told about: complete, in a language that is itself ready.
            "pages_seen": pages.len(),
            "pages_indexable": if r.ready(super::sitemap::SEO_MIN) { pages.iter().filter(|p| p.ready).count() } else { 0 },
            "version": r.version, "pages": pages,
        }));
    }
    Ok(Json(
        json!({"seo_min": super::sitemap::SEO_MIN, "locales": locales}),
    ))
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[derive(Deserialize)]
struct SnippetQuery {
    path: Option<String>,
}

/// `GET /api/projects/{slug}/edge/hreflang?path=/pricing`: the `<link>` tags the source site must carry when it is
/// not behind glot (subdomain mode) — without them the hreflang cluster is not reciprocal.
async fn hreflang_snippet(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<SnippetQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let s = site(&state, &p, true).await?;
    let path = q.path.unwrap_or_else(|| "/".into());
    let noop = |_: &str| None;
    let ctx = s.seo_ctx(&p.source_locale, &path, &noop);
    let tags: Vec<String> = ctx
        .alternates(&path)
        .into_iter()
        .map(|(l, u)| format!(r#"<link rel="alternate" hreflang="{l}" href="{u}">"#))
        .collect();
    Ok(Json(json!({"path": path, "tags": tags})))
}

async fn last_report(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let r: Option<Value> =
        sqlx::query_scalar("SELECT report FROM edge_seo_reports WHERE project_id = $1")
            .bind(p.id)
            .fetch_optional(&state.db)
            .await?;
    r.map(Json)
        .ok_or_else(|| AppError::not_found("no SEO report yet"))
}

async fn run_report(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let report = crawl(&state, &p).await?;
    sqlx::query(
        "INSERT INTO edge_seo_reports (project_id, report) VALUES ($1, $2)
         ON CONFLICT (project_id) DO UPDATE SET report = $2, created_at = now()",
    )
    .bind(p.id)
    .bind(&report)
    .execute(&state.db)
    .await?;
    Ok(Json(report))
}

/// One fetched page.
struct Fetched {
    url: String,
    status: Option<u16>,
    error: Option<&'static str>,
    final_url: Option<String>,
    content_language: Option<String>,
    x_robots: Option<String>,
    html: String,
    seo: PageSeo,
}

async fn fetch_page(url: String) -> Fetched {
    match fetch(&url, PAGE_BYTES).await {
        Ok((status, h, html, final_url)) => Fetched {
            url: url.clone(),
            status: Some(status),
            error: None,
            final_url: Some(final_url.to_string()),
            content_language: h
                .get("content-language")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            x_robots: h
                .get("x-robots-tag")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_ascii_lowercase()),
            seo: inspect(&html),
            html,
        },
        Err(e) => Fetched {
            url,
            status: None,
            error: Some(e),
            final_url: None,
            content_language: None,
            x_robots: None,
            html: String::new(),
            seo: PageSeo::default(),
        },
    }
}

/// `Disallow` rules of robots.txt for `*` / Googlebot (prefix match, `Allow` longer wins).
// ponytail: prefix rules only; `*` / `$` wildcards inside paths are treated literally.
pub(super) fn robots_blocks(robots: &str, path: &str) -> bool {
    let mut applies = false;
    let mut in_group = false;
    let (mut allow, mut disallow) = (0usize, 0usize);
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "user-agent" => {
                if !in_group {
                    applies = false;
                }
                in_group = true;
                let ua = v.to_ascii_lowercase();
                applies |= ua == "*" || ua.starts_with("googlebot");
            }
            "allow" | "disallow" => {
                in_group = false;
                if applies && !v.is_empty() && path.starts_with(v) {
                    if k == "allow" {
                        allow = allow.max(v.len());
                    } else {
                        disallow = disallow.max(v.len());
                    }
                }
            }
            _ => in_group = false,
        }
    }
    disallow > allow
}

/// Normalized absolute URL for comparisons (no fragment, default ports dropped).
fn norm(u: &str, base: &str) -> Option<String> {
    let b = Url::parse(base).ok()?;
    let mut u = b.join(u).ok()?;
    u.set_fragment(None);
    Some(u.to_string())
}

/// Calls the renderer's snapshot of the source URL (`GLOT_RENDERER_URL`): Some(html) when one is stored.
async fn rendered(url: &str) -> Result<String, &'static str> {
    let Ok(base) = std::env::var("GLOT_RENDERER_URL") else {
        return Err("renderer_not_configured");
    };
    let token = std::env::var("GLOT_RENDERER_TOKEN").unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "unreachable")?;
    let r = client
        .get(format!("{}/snapshot", base.trim_end_matches('/')))
        .query(&[("url", url)])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "unreachable")?;
    match r.status().as_u16() {
        200 => r.text().await.map_err(|_| "unreachable"),
        202 => Err("queued"),
        _ => Err("unavailable"),
    }
}

async fn crawl(state: &AppState, p: &ProjectRow) -> ApiResult<Value> {
    let settings = Settings::load(&state.db, p.id).await?;
    let all = site(state, p, false).await?;
    if all.bases.is_empty() {
        return Err(AppError::bad_request(
            "no verified Edge domain: add and verify one first",
        ));
    }
    let readiness = readiness(state, p).await?;
    let advertised = site(state, p, true).await?;
    let opts = SegmentOpts::new(&p.source_locale);

    // Critical pages first (from the origin sitemap and the pages reported by the edge), then the busiest.
    let mut paths: Vec<String> = vec!["/".into()];
    let sitemap = source_paths(&all).await;
    let reported: Vec<(String,)> = sqlx::query_as(
        "SELECT path FROM edge_page_segments WHERE project_id = $1 GROUP BY path ORDER BY count(*) DESC LIMIT 200",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let candidates: Vec<String> = sitemap
        .iter()
        .cloned()
        .chain(reported.into_iter().map(|r| r.0))
        .collect();
    for c in candidates.iter().filter(|c| settings.critical(c)) {
        if !paths.contains(c) {
            paths.push(c.clone());
        }
    }
    for c in &candidates {
        if paths.len() >= MAX_PAGES {
            break;
        }
        if !paths.contains(c) {
            paths.push(c.clone());
        }
    }
    paths.truncate(MAX_PAGES);

    let mut robots: HashMap<String, String> = HashMap::new();
    for (_, b) in &all.bases {
        let Some(origin) = Url::parse(b).ok().map(|u| u.origin().ascii_serialization()) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(e) = robots.entry(origin.clone()) {
            let r = match fetch(&format!("{origin}/robots.txt"), 256 * 1024).await {
                Ok((200, _, body, _)) => body,
                _ => String::new(),
            };
            e.insert(r);
        }
    }

    let tms: HashMap<String, Tm> = {
        let mut m = HashMap::new();
        for (l, _) in &all.bases {
            if l != &p.source_locale {
                m.insert(l.clone(), super::tm::snapshot(&state.db, p, l).await?);
            }
        }
        m
    };

    let mut pages_out = Vec::new();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    // (locale, title) → where it was seen: the same title on two pages of a language competes with itself.
    let mut titles: HashMap<(String, String), Vec<(usize, usize)>> = HashMap::new();
    for path in &paths {
        // Every locale of this page, concurrently.
        let urls: Vec<(String, String)> = all
            .bases
            .iter()
            .map(|(l, b)| (l.clone(), format!("{b}{path}")))
            .collect();
        let handles: Vec<_> = urls
            .iter()
            .map(|(_, u)| tokio::spawn(fetch_page(u.clone())))
            .collect();
        let mut fetched: Vec<Fetched> = Vec::with_capacity(handles.len());
        for h in handles {
            fetched.push(h.await.map_err(AppError::internal)?);
        }
        let source = &fetched[0];
        let src_segs = segment(&source.html, &opts);
        let src_hashes: HashMap<&str, &str> = src_segs
            .iter()
            .map(|s| (s.hash.as_str(), s.text.as_str()))
            .collect();
        // The cluster as each page declares it: locale → (its URL, its alternates normalized).
        let declared: Vec<BTreeMap<String, String>> = fetched
            .iter()
            .map(|f| {
                let base = f.final_url.clone().unwrap_or_else(|| f.url.clone());
                f.seo
                    .alternates
                    .iter()
                    .filter_map(|(l, h)| Some((l.to_ascii_lowercase(), norm(h, &base)?)))
                    .collect()
            })
            .collect();

        // Initial HTML vs rendered DOM of the source page (content that needs JavaScript).
        let render = match rendered(&urls[0].1).await {
            Ok(html) => {
                let initial: BTreeSet<&str> = src_segs.iter().map(|s| s.hash.as_str()).collect();
                let r = segment(&html, &opts);
                let only_js: Vec<&str> = r
                    .iter()
                    .filter(|s| !initial.contains(s.hash.as_str()))
                    .map(|s| s.text.as_str())
                    .collect();
                json!({"status": if only_js.is_empty() { "same" } else { "js_only_content" },
                       "initial_segments": src_segs.len(), "rendered_segments": r.len(),
                       "only_after_js": only_js.len(), "sample": only_js.iter().take(10).collect::<Vec<_>>()})
            }
            Err(e) => json!({"status": "unknown", "reason": e, "initial_segments": src_segs.len()}),
        };

        let mut locales_out = Vec::new();
        for (i, f) in fetched.iter().enumerate() {
            let locale = &urls[i].0;
            let is_source = locale == &p.source_locale;
            let own = norm(&f.url, &f.url).unwrap_or_default();
            let mut issues: Vec<&'static str> = Vec::new();
            let ok_status = f.status == Some(200);
            if f.error.is_some() {
                issues.push("fetch_failed");
            } else if !ok_status {
                issues.push("http_error");
            }
            if f.final_url
                .as_deref()
                .is_some_and(|u| norm(u, u).as_deref() != Some(own.as_str()))
            {
                issues.push("redirected");
            }
            let lang_ok = f.seo.lang.as_deref().is_some_and(|x| same_lang(x, locale));
            if ok_status && !lang_ok {
                issues.push("html_lang");
            }
            if ok_status
                && f.seo.dir.as_deref().unwrap_or("ltr") != glot_core::edge::seo::dir(locale)
            {
                issues.push("html_dir");
            }
            if !is_source
                && ok_status
                && !f
                    .content_language
                    .as_deref()
                    .is_some_and(|x| same_lang(x, locale))
            {
                issues.push("content_language");
            }
            let canonical = f.seo.canonicals.first().and_then(|c| norm(c, &f.url));
            match f.seo.canonicals.len() {
                0 if ok_status => issues.push("canonical_missing"),
                n if n > 1 => issues.push("canonical_multiple"),
                _ => {}
            }
            let canonical_self = canonical
                .as_deref()
                .is_some_and(|c| strip_query(c) == strip_query(&own));
            if canonical.is_some() && !canonical_self {
                issues.push(
                    if !is_source
                        && canonical
                            .as_deref()
                            .is_some_and(|c| strip_query(c) == strip_query(&urls[0].1))
                    {
                        "canonical_to_source"
                    } else {
                        "canonical_not_self"
                    },
                );
            }
            let d = &declared[i];
            let advertised_here = advertised.base(locale).is_some();
            if ok_status && advertised_here {
                if d.get(&locale.to_ascii_lowercase()).map(|u| strip_query(u))
                    != Some(strip_query(&own))
                {
                    issues.push("hreflang_self_missing");
                }
                if !d.contains_key("x-default") {
                    issues.push("hreflang_x_default_missing");
                }
            }
            // Reciprocity: each advertised locale this page points to must point back here.
            let mut not_reciprocal = Vec::new();
            for (j, (l2, _)) in urls.iter().enumerate() {
                if j == i || advertised.base(l2).is_none() || !advertised_here {
                    continue;
                }
                let points_there = d.contains_key(&l2.to_ascii_lowercase());
                let back = declared[j]
                    .get(&locale.to_ascii_lowercase())
                    .is_some_and(|u| strip_query(u) == strip_query(&own));
                if !points_there || !back {
                    not_reciprocal.push(l2.clone());
                }
            }
            if !not_reciprocal.is_empty() {
                issues.push("hreflang_not_reciprocal");
            }
            if !is_source
                && advertised.base(locale).is_none()
                && !d.is_empty()
                && fetched[0]
                    .seo
                    .alternates
                    .iter()
                    .any(|(l, _)| same_lang(l, locale))
            {
                issues.push("advertised_but_not_ready");
            }
            // Metadata translated (differs from the source page's).
            let differs = |a: &Option<String>, b: &Option<String>| match (a, b) {
                (Some(x), Some(y)) => x != y,
                (None, None) => true,
                _ => true,
            };
            let title_tr = is_source || differs(&f.seo.title, &source.seo.title);
            let desc_tr = is_source || differs(&f.seo.description, &source.seo.description);
            let og_tr = is_source || differs(&f.seo.og_title, &source.seo.og_title);
            if ok_status && f.seo.title.is_none() {
                issues.push("title_missing");
            } else if !title_tr {
                issues.push("title_untranslated");
            }
            if !desc_tr && f.seo.description.is_some() {
                issues.push("description_untranslated");
            }
            if ok_status && f.seo.description.is_none() {
                issues.push("description_missing");
            }
            if ok_status && let Some(t) = &f.seo.title {
                titles
                    .entry((locale.clone(), t.clone()))
                    .or_default()
                    .push((pages_out.len(), i));
            }
            if !og_tr && f.seo.og_title.is_some() {
                issues.push("og_title_untranslated");
            }
            if f.seo.jsonld > 0
                && !f.seo.jsonld_languages.is_empty()
                && !f.seo.jsonld_languages.iter().any(|x| same_lang(x, locale))
            {
                issues.push("jsonld_language");
            }
            // Segments still showing source text that the TM can translate / has no translation for.
            let (mut untranslated, mut sample) = (0usize, Vec::new());
            let segs = if is_source {
                vec![]
            } else {
                segment(&f.html, &opts)
            };
            if !is_source {
                let tm = tms.get(locale);
                for s in &segs {
                    if let Some(text) = src_hashes.get(s.hash.as_str()) {
                        let same_in_tm = tm
                            .and_then(|t| {
                                t.lookup(
                                    &s.hash,
                                    &glot_core::edge::tm::context(
                                        &s.ctx.role,
                                        path,
                                        &p.target_locales,
                                    ),
                                )
                            })
                            .is_some_and(|v| v == *text);
                        if !same_in_tm {
                            untranslated += 1;
                            if sample.len() < 20 {
                                sample.push(
                                    json!({"hash": s.hash, "text": text, "role": s.ctx.role}),
                                );
                            }
                        }
                    }
                }
                if untranslated > 0 {
                    issues.push("untranslated_segments");
                }
            }
            let noindex = f
                .seo
                .robots
                .iter()
                .any(|r| r.contains("noindex") || r.contains("none"))
                || f.x_robots
                    .as_deref()
                    .is_some_and(|r| r.contains("noindex") || r.contains("none"));
            if noindex {
                issues.push("noindex");
            }
            let robots_blocked = Url::parse(&f.url).ok().is_some_and(|u| {
                robots
                    .get(&u.origin().ascii_serialization())
                    .is_some_and(|r| robots_blocks(r, u.path()))
            });
            if robots_blocked {
                issues.push("robots_txt_blocked");
            }
            // Technically indexable: reachable, indexable directives, self canonical, and (targets) advertised in a
            // reciprocal cluster with the right language. Effectively indexed: unknown without Search Console.
            let indexable = ok_status
                && !noindex
                && !robots_blocked
                && canonical_self
                && lang_ok
                && (is_source || (advertised_here && not_reciprocal.is_empty()));
            for i in &issues {
                *totals.entry(i).or_default() += 1;
            }
            locales_out.push(json!({
                "locale": locale, "url": f.url, "status": f.status, "error": f.error,
                "final_url": f.final_url, "lang": f.seo.lang, "dir": f.seo.dir,
                "content_language": f.content_language, "canonical": canonical,
                "hreflang": d, "not_reciprocal": not_reciprocal, "advertised": is_source || advertised_here,
                "title": f.seo.title, "description": f.seo.description, "og_title": f.seo.og_title,
                "jsonld": f.seo.jsonld,
                "untranslated": {"count": untranslated, "segments": if is_source { src_segs.len() } else { segs.len() }, "sample": sample},
                "noindex": noindex, "robots_blocked": robots_blocked,
                "technically_indexable": indexable,
                "indexed": Value::Null,
                "issues": issues,
            }));
        }
        pages_out.push(json!({"path": path, "critical": settings.critical(path), "render": render, "locales": locales_out}));
    }

    for at in titles.values().filter(|v| v.len() > 1) {
        for (page, locale) in at {
            if let Some(issues) = pages_out[*page]["locales"][*locale]["issues"].as_array_mut() {
                issues.push(json!("title_duplicate"));
                *totals.entry("title_duplicate").or_default() += 1;
            }
        }
    }
    // The outcome per language: pages a search engine can index, out of the pages checked.
    let mut summary: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for l in pages_out
        .iter()
        .filter_map(|p| p["locales"].as_array())
        .flatten()
    {
        let e = summary
            .entry(l["locale"].as_str().unwrap_or_default().to_string())
            .or_default();
        e.0 += 1;
        e.1 += usize::from(l["technically_indexable"] == true);
    }

    Ok(json!({
        "summary": summary.iter().map(|(l, (pages, indexable))|
            json!({"locale": l, "pages": pages, "indexable": indexable})).collect::<Vec<_>>(),
        "generated_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "javascript": false,
        "indexed": {"status": "unknown", "reason": "search_console_not_connected"},
        "readiness": readiness.iter().map(|r| json!({
            "locale": r.locale, "coverage": round(r.coverage()), "critical_coverage": r.critical_coverage,
            "critical_threshold": r.critical_threshold, "ready": r.ready(super::sitemap::SEO_MIN), "version": r.version,
        })).collect::<Vec<_>>(),
        "advertised": advertised.bases.iter().map(|(l, b)| json!({"locale": l, "base": b})).collect::<Vec<_>>(),
        "sitemaps": advertised.bases.iter().map(|(_, b)| format!("{b}/sitemap.xml")).collect::<Vec<_>>(),
        "issues": totals,
        "pages": pages_out,
    }))
}

fn same_lang(a: &str, b: &str) -> bool {
    let a = a.replace('_', "-").to_ascii_lowercase();
    let b = b.replace('_', "-").to_ascii_lowercase();
    a == b || a.split('-').next() == b.split('-').next() && (!a.contains('-') || !b.contains('-'))
}

fn strip_query(u: &str) -> &str {
    u.split(['?', '#']).next().unwrap_or(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_rules() {
        let r = "User-agent: *\nDisallow: /admin\nAllow: /admin/public\n\nUser-agent: Bingbot\nDisallow: /\n";
        assert!(robots_blocks(r, "/admin/x"));
        assert!(!robots_blocks(r, "/admin/public/x"));
        assert!(!robots_blocks(r, "/pricing"));
        assert!(robots_blocks(
            "User-agent: Googlebot\nUser-agent: *\nDisallow: /es\n",
            "/es/pricing"
        ));
        assert!(!robots_blocks("", "/"));
    }

    #[test]
    fn readiness_rules() {
        let r = Readiness {
            locale: "es".into(),
            keys: 10,
            keys_done: 10,
            segments: 90,
            segments_done: 88,
            critical_coverage: Some(0.97),
            critical_threshold: 0.98,
            version: 3,
        };
        assert!((r.coverage() - 0.98).abs() < 1e-9);
        // 98 % overall but one critical page at 97 %: not ready.
        assert!(!r.ready(0.95));
        let ok = Readiness {
            critical_coverage: Some(0.98),
            ..r.clone()
        };
        assert!(ok.ready(0.95) && !ok.ready(0.99));
        let unknown = Readiness {
            critical_coverage: None,
            ..r
        };
        assert!(unknown.ready(0.95));
        assert!(
            same_lang("es-ES", "es")
                && same_lang("pt_BR", "pt-br")
                && !same_lang("pt-BR", "pt-PT")
                && !same_lang("fr", "es")
        );
    }
}
