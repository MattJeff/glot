//! Local SEO: for each page × market, adapt the **search-facing** content (title, meta description, H1/H2, slug
//! suggestion, FAQ structured data) to what locals actually search, while the body stays a faithful translation.
//!
//! Flow: `POST …/local-seo/scan` reads the source pages (raw HTML, anonymous, SSRF-safe), audits what each market
//! shows today, asks the demand sources ([`demand`]) what people search there, and (only when real demand terms
//! exist) has the project's LLM propose content that must pass the validators ([`validate`]). Proposals are
//! drafts: nothing is served until a human approves it, unless an admin switched the project to `auto`.
//! Served overrides are read by glot Edge ([`rewrite`]) and by the website tag (`GET /cdn/{slug}/local-seo/…`).
//! Once Search Console is connected, impressions / clicks / position are compared before and after approval.
//!
//! Sent to the LLM: public page copy (title, description, headings, a 2 000-character text excerpt), aggregated
//! search queries, glossary terms. Nothing about visitors exists in this module.
use axum::{
    Json, Router,
    extract::State,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, patch, post, put},
};
use chrono::{DateTime, Utc};
use glot_llm::provider::CompletionRequest;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
    validate_locale,
};

pub mod demand;
pub mod html;
pub mod validate;

use demand::{DemandSource, Term};
pub use validate::{Content, Faq, Issue, Market, Page};
use validate::{Ctx, Sibling, has_errors, market, norm_path, served};

const MAX_PAGES: usize = 10;
const PAGE_BYTES: usize = 2 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/local-seo";
    Router::new()
        .route(p, get(view))
        .route(&format!("{p}/scan"), post(scan))
        .route(&format!("{p}/settings"), put(put_settings))
        .route(&format!("{p}/demand"), put(put_demand))
        .route(&format!("{p}/overrides/{{id}}"), patch(edit))
        .route(&format!("{p}/overrides/{{id}}/approve"), post(approve))
        .route(&format!("{p}/overrides/{{id}}/reject"), post(reject))
        .route(&format!("{p}/gsc"), delete(gsc_disconnect))
        .route(&format!("{p}/gsc/connect"), get(gsc_connect))
        .route(&format!("{p}/metrics/refresh"), post(refresh_metrics))
        .route("/api/local-seo/gsc/callback", get(gsc_callback))
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/cdn/{slug}/local-seo/{file}", get(public_overrides))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET]),
        )
}

fn unprocessable(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
}

// ---------- the hook glot Edge and the website tag consume ----------

#[derive(sqlx::FromRow)]
struct Served {
    locale: String,
    status: String,
    proposed: DbJson<Content>,
    edited: Option<DbJson<Content>>,
}

fn effective(proposed: &Content, edited: Option<&Content>) -> Content {
    edited.unwrap_or(proposed).clone()
}

/// The approved search-facing content of `path` in `locale`, or `None`: the faithful translation stays.
/// Resolution order: [`validate::resolve`].
// ponytail: one indexed query per call. Cache next to Edge's TM snapshot (same `edge_catalog_version`, bumped
// on every decision here) if it ever shows in a profile.
pub async fn lookup(db: &sqlx::PgPool, project: Uuid, locale: &str, path: &str) -> Option<Content> {
    let rows: Vec<Served> = sqlx::query_as(
        "SELECT locale, status, proposed, edited FROM local_seo_overrides WHERE project_id = $1 AND path = $2",
    )
    .bind(project)
    .bind(norm_path(path))
    .fetch_all(db)
    .await
    .ok()?;
    let rows: Vec<(String, String, Content)> = rows
        .into_iter()
        .map(|r| {
            (
                r.locale,
                r.status,
                effective(&r.proposed, r.edited.as_deref()),
            )
        })
        .collect();
    validate::resolve(&rows, locale).cloned()
}

/// **The one line glot Edge calls** on a translated HTML page before answering:
/// `let html = crate::local_seo::rewrite(&state.db, p.id, locale, path, html).await;`
/// Without an approved override (or on any error) the page comes back untouched.
pub async fn rewrite(
    db: &sqlx::PgPool,
    project: Uuid,
    locale: &str,
    path: &str,
    page: String,
) -> String {
    match lookup(db, project, locale, path).await {
        Some(c) => html::apply(&page, &c),
        None => page,
    }
}

/// `GET /cdn/{slug}/local-seo/{locale}.json`: every approved override of a market, for the website tag.
/// Public on purpose: it is the title and description the live site shows to everyone.
async fn public_overrides(
    State(state): State<AppState>,
    AppPath((slug, file)): AppPath<(String, String)>,
) -> ApiResult<Response> {
    let locale = file
        .strip_suffix(".json")
        .ok_or_else(|| AppError::not_found("not found"))?;
    validate_locale(locale)?;
    let p = projects::load_public(&state, &slug)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))?;
    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT path FROM local_seo_overrides WHERE project_id = $1 AND status IN ('approved', 'auto') ORDER BY path",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let mut pages = BTreeMap::new();
    for path in paths {
        if let Some(mut c) = lookup(&state.db, p.id, locale, &path).await {
            c.slug = None; // a suggestion for humans, not something a page applies to itself
            pages.insert(path, c);
        }
    }
    Ok((
        [(header::CACHE_CONTROL, "public, max-age=300")],
        Json(json!({ "locale": locale, "pages": pages })),
    )
        .into_response())
}

// ---------- settings, site ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct Settings {
    mode: String,
    site_url: Option<String>,
}

async fn settings(state: &AppState, project: Uuid) -> sqlx::Result<Settings> {
    Ok(
        sqlx::query_as("SELECT mode, site_url FROM local_seo_settings WHERE project_id = $1")
            .bind(project)
            .fetch_optional(&state.db)
            .await?
            .unwrap_or(Settings {
                mode: "review".into(),
                site_url: None,
            }),
    )
}

struct SiteInfo {
    /// Where source pages are read from; `None` = glot does not know the site yet.
    origin: Option<String>,
    /// `(locale, base URL)` of each market, when glot Edge serves the site.
    bases: Vec<(String, String)>,
}

impl SiteInfo {
    fn url(&self, locale: &str, path: &str) -> Option<String> {
        let base = self
            .bases
            .iter()
            .find(|(l, _)| l == locale)
            .map(|(_, b)| b.as_str())?;
        Some(format!(
            "{}{}",
            base.trim_end_matches('/'),
            if path == "/" { "/" } else { path }
        ))
    }

    /// Without glot Edge every market lives on the same URL; Search Console's country filter tells them apart.
    fn url_or_origin(&self, locale: &str, path: &str) -> Option<String> {
        self.url(locale, path).or_else(|| {
            Some(format!(
                "{}{}",
                self.origin.as_deref()?,
                if path == "/" { "/" } else { path }
            ))
        })
    }
}

/// glot Edge's verified domains, else the Preview URL, else the URL given to Local SEO.
async fn site_info(state: &AppState, p: &ProjectRow, s: &Settings) -> ApiResult<SiteInfo> {
    let edge = crate::edge::sitemap::site(state, p, false).await?;
    let mut origin = edge.origin.clone();
    if origin.is_none() {
        origin = sqlx::query_scalar(
            "SELECT base_url FROM preview_settings WHERE project_id = $1 AND base_url IS NOT NULL",
        )
        .bind(p.id)
        .fetch_optional(&state.db)
        .await?;
    }
    let origin = origin
        .or_else(|| s.site_url.clone())
        .map(|o| o.trim_end_matches('/').to_string());
    Ok(SiteInfo {
        origin,
        bases: edge.bases,
    })
}

fn check_site_url(u: &str) -> ApiResult<String> {
    let url = reqwest::Url::parse(u.trim())
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
    let url = url.ok_or_else(|| {
        AppError::bad_request("site_url must be an http(s) URL, e.g. https://example.com")
    })?;
    Ok(url.origin().ascii_serialization())
}

#[derive(Deserialize)]
struct SettingsBody {
    mode: Option<String>,
    site_url: Option<String>,
}

/// `PUT …/settings`: `mode` = `review` (default) or `auto`. Switching to `auto` is the consent to serve proposals
/// that pass every validator without a human look; it is recorded with who did it.
async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<SettingsBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let cur = settings(&state, p.id).await?;
    let mode = b.mode.unwrap_or(cur.mode);
    if !matches!(mode.as_str(), "review" | "auto") {
        return Err(AppError::bad_request("mode must be 'review' or 'auto'"));
    }
    let site_url = match b.site_url {
        Some(u) if u.trim().is_empty() => None,
        Some(u) => Some(check_site_url(&u)?),
        None => cur.site_url,
    };
    sqlx::query(
        "INSERT INTO local_seo_settings (project_id, mode, site_url, mode_set_by) VALUES ($1, $2, $3, $4)
         ON CONFLICT (project_id) DO UPDATE SET mode = $2, site_url = $3, mode_set_by = $4, updated_at = now()",
    )
    .bind(p.id)
    .bind(&mode)
    .bind(&site_url)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "mode": mode, "site_url": site_url })))
}

// ---------- manual demand ----------

#[derive(Deserialize)]
struct DemandTerm {
    term: String,
    /// Monthly searches or impressions as the customer's own tool reports them; omitted = unknown.
    impressions: Option<i64>,
}

#[derive(Deserialize)]
struct DemandBody {
    locale: String,
    /// Omitted = the whole market.
    path: Option<String>,
    terms: Vec<DemandTerm>,
}

/// `PUT …/demand`: replaces the customer's own keyword list of a market (or of one page in it).
async fn put_demand(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<DemandBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if !p.target_locales.contains(&b.locale) {
        return Err(AppError::bad_request(format!(
            "'{}' is not a target locale of this project",
            b.locale
        )));
    }
    if b.terms.len() > 200 {
        return Err(AppError::bad_request("at most 200 terms per market"));
    }
    let path = b.path.as_deref().map(norm_path).unwrap_or_default();
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM local_seo_demand WHERE project_id = $1 AND locale = $2 AND path = $3 AND source = 'manual'")
        .bind(p.id)
        .bind(&b.locale)
        .bind(&path)
        .execute(&mut *tx)
        .await?;
    let mut n = 0;
    for t in &b.terms {
        let term = t.term.split_whitespace().collect::<Vec<_>>().join(" ");
        if term.is_empty() || term.chars().count() > 120 {
            continue;
        }
        n += sqlx::query(
            "INSERT INTO local_seo_demand (project_id, locale, path, term, source, impressions)
             VALUES ($1, $2, $3, $4, 'manual', $5)
             ON CONFLICT (project_id, locale, path, term) DO UPDATE SET source = 'manual', impressions = $5, fetched_at = now()",
        )
        .bind(p.id)
        .bind(&b.locale)
        .bind(&path)
        .bind(&term)
        .bind(t.impressions.filter(|n| *n >= 0))
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(Json(json!({ "saved": n })))
}

// ---------- scan ----------

#[derive(Deserialize)]
struct PageInput {
    path: String,
    #[serde(flatten)]
    page: Page,
}

#[derive(Deserialize, Default)]
struct ScanBody {
    /// Paths to read from the site. Omitted (and no `pages`): the site's sitemap, first pages.
    paths: Option<Vec<String>>,
    /// Page content given directly (headless CMS, pages whose raw HTML is empty before JavaScript).
    pages: Option<Vec<PageInput>>,
    locales: Option<Vec<String>>,
    /// Propose again even where a decision exists.
    #[serde(default)]
    regenerate: bool,
}

fn fingerprint(p: &Page) -> String {
    let mut h = Sha256::new();
    for part in [&p.title, &p.description, &p.h1] {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    hex::encode(&h.finalize()[..8])
}

async fn translation_of(
    state: &AppState,
    p: &ProjectRow,
    locale: &str,
    text: &str,
) -> sqlx::Result<Option<String>> {
    if text.is_empty() {
        return Ok(None);
    }
    let hash =
        glot_core::edge::tm::text_hash(&p.source_locale, &glot_core::edge::tm::normalize(text));
    let edge: Option<String> = sqlx::query_scalar(
        "SELECT value FROM edge_translations WHERE project_id = $1 AND hash = $2 AND locale = $3
         AND status IN ('machine', 'reviewed') ORDER BY (status = 'reviewed') DESC LIMIT 1",
    )
    .bind(p.id)
    .bind(hash)
    .bind(locale)
    .fetch_optional(&state.db)
    .await?;
    if edge.is_some() {
        return Ok(edge);
    }
    sqlx::query_scalar(
        "SELECT t.value FROM keys k JOIN translations t ON t.key_id = k.id
         WHERE k.project_id = $1 AND k.source = $2 AND t.locale = $3 AND t.status <> 'stale' LIMIT 1",
    )
    .bind(p.id)
    .bind(text)
    .bind(locale)
    .fetch_optional(&state.db)
    .await
}

/// What the market shows today: the live localized page when glot Edge serves it, else the stored translations.
async fn current_of(
    state: &AppState,
    p: &ProjectRow,
    site: &SiteInfo,
    locale: &str,
    path: &str,
    page: &Page,
) -> ApiResult<Content> {
    if let Some(url) = site.url(locale, path)
        && let Ok((200, _, body, _)) = crate::edge::sitemap::fetch(&url, PAGE_BYTES).await
    {
        let live = html::extract(&body);
        return Ok(Content {
            title: Some(live.title),
            description: Some(live.description),
            h1: Some(live.h1),
            h2: live.h2,
            ..Default::default()
        }
        .clean());
    }
    Ok(Content {
        title: translation_of(state, p, locale, &page.title).await?,
        description: translation_of(state, p, locale, &page.description).await?,
        h1: translation_of(state, p, locale, &page.h1).await?,
        ..Default::default()
    })
}

async fn brands(state: &AppState, project: Uuid) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT term FROM glossary_terms WHERE project_id = $1 AND do_not_translate ORDER BY term",
    )
    .bind(project)
    .fetch_all(&state.db)
    .await
}

#[derive(sqlx::FromRow)]
struct OverrideRow {
    id: Uuid,
    locale: String,
    path: String,
    status: String,
    current: DbJson<Content>,
    proposed: DbJson<Content>,
    edited: Option<DbJson<Content>>,
    terms: DbJson<Vec<Term>>,
    rationale: String,
    issues: DbJson<Vec<Issue>>,
    fingerprint: String,
    model: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    applied_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

const ROW_COLS: &str = "id, locale, path, status, current, proposed, edited, terms, rationale, issues, fingerprint, model, decided_at, applied_at, updated_at";

impl OverrideRow {
    fn effective(&self) -> Content {
        effective(&self.proposed, self.edited.as_deref())
    }
    /// What counts for uniqueness: drafts and served content, not rejected or outdated proposals.
    fn claimed(&self) -> Content {
        if matches!(self.status.as_str(), "proposed" | "approved" | "auto") {
            self.effective()
        } else {
            Content::default()
        }
    }
}

async fn rows_of(state: &AppState, project: Uuid) -> sqlx::Result<Vec<OverrideRow>> {
    sqlx::query_as(&format!(
        "SELECT {ROW_COLS} FROM local_seo_overrides WHERE project_id = $1 ORDER BY path, locale"
    ))
    .bind(project)
    .fetch_all(&state.db)
    .await
}

/// The other pages of `locale`, and the same page in the other regions of its language.
fn neighbours(
    rows: &[OverrideRow],
    locale: &str,
    path: &str,
) -> (Vec<Sibling>, Vec<(String, String, String)>) {
    let lang = market(locale).language;
    let siblings = rows
        .iter()
        .filter(|r| r.locale == locale && r.path != path)
        .map(|r| Sibling {
            path: r.path.clone(),
            current: r.current.0.clone(),
            effective: r.claimed(),
        })
        .collect();
    let regions = rows
        .iter()
        .filter(|r| r.path == path && r.locale != locale && market(&r.locale).language == lang)
        .filter_map(|r| {
            Some((
                r.locale.clone(),
                r.claimed().title?,
                r.terms.first()?.term.clone(),
            ))
        })
        .collect();
    (siblings, regions)
}

struct Proposal {
    content: Content,
    rationale: String,
    model: String,
}

const SYSTEM: &str = "You are a native search copywriter for one market. You adapt a page's SEARCH-FACING content to the queries people of that market really type, given as `demand` (real search data: never invent other keywords). The page body is translated faithfully elsewhere; you only write: title, description, h1, h2 (same count and order as the page's h2, or empty), slug, faq.\n\
Rules, all mandatory:\n\
- Write in the market's language AND regional usage (es-MX is not es-ES: vocabulary, spelling, currency words).\n\
- Use the demand terms the page genuinely answers; ignore the others. Put the best one early in the title, once. Never repeat a term, never list keywords.\n\
- Claim nothing the page does not state: no 'free', 'best', 'official', 'guaranteed', prices, figures or features absent from `page`.\n\
- Keep every `brands` term exactly as spelled. Use `glossary` translations.\n\
- title width <= limits.title, description width <= limits.description (CJK characters count 2). Description: one or two natural sentences.\n\
- Do not reuse any title in `taken_titles`.\n\
- slug: lowercase ASCII letters, digits, hyphens, built from the main term; null for non-Latin scripts.\n\
- faq: 0 to 3 questions people ask (from the demand terms), each answered ONLY with facts stated in `page`. No fact, no question.\n\
Output one JSON object: {\"title\": \"\", \"description\": \"\", \"h1\": \"\", \"h2\": [], \"slug\": null, \"faq\": [{\"question\": \"\", \"answer\": \"\"}], \"rationale\": \"one sentence, in English: which terms you used and why\"}";

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// The fake LLM (`provider = "mock"`, automated tests): a deterministic proposal built from the first demand
/// term. It does not try to be right: validators judge it like any other proposal.
fn mock_proposal(ctx: &Ctx, terms: &[Term]) -> Proposal {
    let top = &terms[0];
    let brand = ctx
        .brands
        .iter()
        .find(|b| ctx.page.title.contains(b.as_str()));
    let head = capitalize(&top.term);
    let base = ctx
        .current
        .description
        .clone()
        .unwrap_or_else(|| ctx.page.description.clone());
    Proposal {
        content: Content {
            title: Some(brand.map_or(head.clone(), |b| format!("{head} | {b}"))),
            description: (!base.is_empty()).then(|| format!("{head}. {base}")),
            h1: Some(head),
            h2: vec![],
            slug: crate::edge::slugs::slugify(&top.term),
            faq: vec![],
        },
        rationale: format!("mock: built around “{}”", top.term),
        model: "mock".into(),
    }
}

async fn llm_proposal(
    state: &AppState,
    p: &ProjectRow,
    ctx: &Ctx<'_>,
    terms: &[Term],
    feedback: &[Issue],
) -> ApiResult<Proposal> {
    if p.llm.0.provider == "mock" {
        return Ok(mock_proposal(ctx, terms));
    }
    let glossary: Vec<(String, DbJson<Value>)> = sqlx::query_as(
        "SELECT term, translations FROM glossary_terms WHERE project_id = $1 AND NOT do_not_translate",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let evidence = format!(
        "{} {} {} {}",
        ctx.page.title, ctx.page.description, ctx.page.h1, ctx.page.text
    )
    .to_lowercase();
    let payload = json!({
        "task": "local_seo",
        "market": ctx.market,
        "source_locale": ctx.source_locale,
        "limits": ctx.market.limits,
        "page": ctx.page,
        "current_translation": ctx.current,
        "demand": terms,
        "brands": ctx.brands,
        "glossary": glossary.iter()
            .filter(|(t, _)| evidence.contains(&t.to_lowercase()))
            .filter_map(|(t, tr)| Some(json!({"term": t, "translation": tr.0.get(&ctx.market.locale)?})))
            .collect::<Vec<_>>(),
        "taken_titles": ctx.siblings.iter()
            .flat_map(|s| [s.effective.title.clone(), s.current.title.clone()])
            .flatten()
            .collect::<Vec<_>>(),
        "style": p.style,
        "rejected_because": feedback.iter().map(|i| &i.message).collect::<Vec<_>>(),
    });
    let provider = crate::feedback::provider(state, p)?;
    let c = provider
        .complete(CompletionRequest {
            model: p.llm.0.model.clone(),
            system: SYSTEM.into(),
            user: format!(
                "Adapt this page for `{}`.\n\n{payload:#}",
                ctx.market.locale
            ),
            max_tokens: 2000,
            json: true,
            images: vec![],
        })
        .await
        .map_err(|e| unprocessable("llm_error", e.to_string()))?;
    let text = c
        .text
        .find('{')
        .zip(c.text.rfind('}'))
        .map(|(a, b)| &c.text[a..=b])
        .unwrap_or("{}");
    let v: Value = serde_json::from_str(text)
        .map_err(|e| unprocessable("llm_error", format!("the model's answer was not JSON: {e}")))?;
    let content: Content = serde_json::from_value(v.clone()).map_err(|e| {
        unprocessable(
            "llm_error",
            format!("the model's answer had the wrong shape: {e}"),
        )
    })?;
    Ok(Proposal {
        content: content.clean(),
        rationale: v["rationale"].as_str().unwrap_or_default().to_string(),
        model: c.model,
    })
}

#[derive(Serialize)]
struct ScanResult {
    path: String,
    locale: String,
    status: String,
    /// Why nothing was proposed, when nothing was.
    note: Option<String>,
}

/// `POST …/scan`: read pages, gather demand, propose. Idempotent: a decision already taken on an unchanged
/// page is kept (and re-checked against the page), unless `regenerate`.
async fn scan(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<ScanBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let cfg = settings(&state, p.id).await?;
    let site = site_info(&state, &p, &cfg).await?;
    let locales: Vec<String> = match b.locales {
        Some(l) => {
            if let Some(bad) = l.iter().find(|l| !p.target_locales.contains(l)) {
                return Err(AppError::bad_request(format!(
                    "'{bad}' is not a target locale of this project"
                )));
            }
            l
        }
        None => p.target_locales.clone(),
    };

    // 1. The pages.
    let mut warnings: Vec<String> = vec![];
    let mut pages: Vec<(String, Page)> = vec![];
    for i in b.pages.unwrap_or_default() {
        pages.push((norm_path(&i.path), i.page));
    }
    if pages.is_empty() {
        let Some(origin) = &site.origin else {
            return Err(unprocessable(
                "no_site",
                "glot does not know where your site lives yet. Connect a domain in glot Edge, set the Preview URL in Settings, or save the site's URL in Local SEO.",
            ));
        };
        let paths = match b.paths {
            Some(paths) => paths.iter().map(|p| norm_path(p)).collect(),
            None => {
                let edge = crate::edge::sitemap::site(&state, &p, false).await?;
                let mut all = if edge.origin.is_some() {
                    crate::edge::sitemap::source_paths(&edge).await
                } else {
                    vec!["/".into()]
                };
                all.truncate(MAX_PAGES);
                all
            }
        };
        for path in paths {
            match crate::edge::sitemap::fetch(&format!("{origin}{path}"), PAGE_BYTES).await {
                Ok((200, _, body, _)) => {
                    let page = html::extract(&body);
                    if page.title.is_empty() && page.h1.is_empty() {
                        warnings.push(format!("{path}: the page has no <title> and no <h1> before JavaScript runs, so search engines see nothing to adapt."));
                    } else {
                        pages.push((path, page));
                    }
                }
                Ok((status, ..)) => {
                    warnings.push(format!("{path}: the site answered HTTP {status}."))
                }
                Err("private") => warnings.push(format!(
                    "{path}: {origin} resolves to a private address; glot only reads public sites."
                )),
                Err(e) => warnings.push(format!("{path}: the site could not be read ({e}).")),
            }
        }
    }
    if pages.len() > MAX_PAGES {
        return Err(AppError::bad_request(format!(
            "at most {MAX_PAGES} pages per scan; scan the next ones in another call"
        )));
    }
    pages.dedup_by(|a, b| a.0 == b.0);
    for (path, page) in &pages {
        sqlx::query(
            "INSERT INTO local_seo_pages (project_id, path, content, fingerprint) VALUES ($1, $2, $3, $4)
             ON CONFLICT (project_id, path) DO UPDATE SET content = $3, fingerprint = $4, scanned_at = now()",
        )
        .bind(p.id)
        .bind(path)
        .bind(DbJson(page))
        .bind(fingerprint(page))
        .execute(&state.db)
        .await?;
    }

    // 2. Demand sources. Only real ones: the customer's keyword list, and Search Console when connected.
    let mut sources: Vec<Box<dyn DemandSource>> = vec![Box::new(demand::Manual {
        db: state.db.clone(),
        project: p.id,
    })];
    match search_console(&state, p.id).await {
        Ok(Some(gsc)) => sources.push(Box::new(gsc)),
        Ok(None) => {}
        Err(e) => warnings.push(e),
    }

    // 3. Each page × market.
    let brands = brands(&state, p.id).await?;
    let mut llm_gate: Option<Result<(), String>> = None;
    let mut results = vec![];
    for (path, page) in &pages {
        let fp = fingerprint(page);
        for locale in &locales {
            let m = market(locale);
            let rows = rows_of(&state, p.id).await?;
            let existing = rows.iter().find(|r| &r.path == path && &r.locale == locale);
            let (siblings, regions) = neighbours(&rows, locale, path);
            let mut done = |status: &str, note: Option<String>| {
                results.push(ScanResult {
                    path: path.clone(),
                    locale: locale.clone(),
                    status: status.into(),
                    note,
                })
            };

            // A decision on an unchanged page stands; a served one is re-checked against the page as it is now.
            if let Some(r) = existing
                && !b.regenerate
                && r.fingerprint == fp
                && matches!(r.status.as_str(), "approved" | "auto" | "rejected")
            {
                let terms: Vec<String> = r.terms.iter().map(|t| t.term.clone()).collect();
                let ctx = Ctx {
                    market: &m,
                    source_locale: &p.source_locale,
                    page,
                    current: &r.current,
                    terms: &terms,
                    brands: &brands,
                    siblings: &siblings,
                    regions: &regions,
                };
                let issues = validate::validate(&r.effective(), &ctx);
                if served(&r.status) && has_errors(&issues) {
                    set_status(&state, &p, r.id, locale, "stale", Some(&issues)).await?;
                    done("stale", Some("The page changed and no longer supports this content: it is no longer served.".into()));
                } else {
                    done(&r.status, None);
                }
                continue;
            }
            // The source changed under a served override: it stops being served until someone looks again.
            if let Some(r) = existing
                && served(&r.status)
            {
                set_status(&state, &p, r.id, locale, "stale", None).await?;
            }

            // Keep the pre-override "current" of a row that was served: the live page now shows our own text.
            let current = match existing {
                Some(r) if r.applied_at.is_some() && r.fingerprint == fp => r.current.0.clone(),
                _ => current_of(&state, &p, &site, locale, path, page).await?,
            };

            let page_url = site.url(locale, path);
            let q = demand::Query {
                market: &m,
                path,
                page_url: page_url.as_deref(),
            };
            let mut terms = vec![];
            for s in &sources {
                match s.terms(&q).await {
                    Ok(t) => terms.extend(t),
                    Err(e) => {
                        if !warnings.contains(&e) {
                            warnings.push(e)
                        }
                    }
                }
            }
            let terms = demand::rank(terms);
            store_demand(&state, p.id, locale, path, &terms).await?;

            let term_texts: Vec<String> = terms.iter().map(|t| t.term.clone()).collect();
            let ctx = Ctx {
                market: &m,
                source_locale: &p.source_locale,
                page,
                current: &current,
                terms: &term_texts,
                brands: &brands,
                siblings: &siblings,
                regions: &regions,
            };
            let mut row = Draft {
                status: "needs_demand",
                proposal: None,
                issues: vec![],
                note: None,
            };
            if terms.is_empty() {
                row.note = Some(format!(
                    "No search data for {locale} yet: connect Search Console or add the keywords you already know for this market."
                ));
            } else {
                let gate = match &llm_gate {
                    Some(g) => g.clone(),
                    None => {
                        let g = crate::billing::check_llm(&state, &p)
                            .await
                            .map_err(|e| e.message);
                        llm_gate = Some(g.clone());
                        g
                    }
                };
                let proposed = match gate {
                    Err(e) => Err(e),
                    Ok(()) => match llm_proposal(&state, &p, &ctx, &terms, &[]).await {
                        Ok(first) => {
                            let issues = validate::validate(&first.content, &ctx);
                            // One retry with the validators' objections; the mock would answer the same.
                            if has_errors(&issues) && first.model != "mock" {
                                match llm_proposal(&state, &p, &ctx, &terms, &issues).await {
                                    Ok(second) => Ok(second),
                                    Err(_) => Ok(first),
                                }
                            } else {
                                Ok(first)
                            }
                        }
                        Err(e) => Err(e.message),
                    },
                };
                match proposed {
                    Ok(prop) => {
                        row.issues = validate::validate(&prop.content, &ctx);
                        // Auto mode serves only what no validator has anything to say about.
                        row.status = if cfg.mode == "auto" && row.issues.is_empty() {
                            "auto"
                        } else {
                            "proposed"
                        };
                        row.proposal = Some(prop);
                    }
                    Err(e) => {
                        row.status = "needs_demand";
                        row.note = Some(format!("The model could not write a proposal: {e}"));
                    }
                }
            }
            save_draft(&state, &p, locale, path, &fp, &current, &terms, &row).await?;
            done(row.status, row.note);
        }
    }
    Ok(Json(
        json!({ "pages": pages.len(), "results": results, "warnings": warnings }),
    ))
}

struct Draft {
    status: &'static str,
    proposal: Option<Proposal>,
    issues: Vec<Issue>,
    note: Option<String>,
}

async fn store_demand(
    state: &AppState,
    project: Uuid,
    locale: &str,
    path: &str,
    terms: &[Term],
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM local_seo_demand WHERE project_id = $1 AND locale = $2 AND path = $3 AND source <> 'manual'")
        .bind(project)
        .bind(locale)
        .bind(path)
        .execute(&state.db)
        .await?;
    for t in terms.iter().filter(|t| t.source != "manual") {
        sqlx::query(
            "INSERT INTO local_seo_demand (project_id, locale, path, term, source, impressions, clicks, position)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
        )
        .bind(project)
        .bind(locale)
        .bind(path)
        .bind(&t.term)
        .bind(&t.source)
        .bind(t.impressions)
        .bind(t.clicks)
        .bind(t.position)
        .execute(&state.db)
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn save_draft(
    state: &AppState,
    p: &ProjectRow,
    locale: &str,
    path: &str,
    fp: &str,
    current: &Content,
    terms: &[Term],
    d: &Draft,
) -> ApiResult<()> {
    let (content, rationale, model) = match &d.proposal {
        Some(pr) => (
            pr.content.clone(),
            pr.rationale.clone(),
            Some(pr.model.clone()),
        ),
        None => (Content::default(), d.note.clone().unwrap_or_default(), None),
    };
    let auto = d.status == "auto";
    sqlx::query(
        "INSERT INTO local_seo_overrides (id, project_id, locale, path, status, current, proposed, terms, rationale, issues, fingerprint, model, applied_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, CASE WHEN $13 THEN now() END)
         ON CONFLICT (project_id, locale, path) DO UPDATE SET status = $5, current = $6, proposed = $7, edited = NULL,
            terms = $8, rationale = $9, issues = $10, fingerprint = $11, model = $12, decided_by = NULL, decided_at = NULL,
            applied_at = CASE WHEN $13 THEN now() END, updated_at = now()",
    )
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(locale)
    .bind(path)
    .bind(d.status)
    .bind(DbJson(current))
    .bind(DbJson(&content))
    .bind(DbJson(terms))
    .bind(rationale)
    .bind(DbJson(&d.issues))
    .bind(fp)
    .bind(model)
    .bind(auto)
    .execute(&state.db)
    .await?;
    if auto {
        bump(state, p.id, locale).await?;
    }
    Ok(())
}

/// What a market serves changed: glot Edge's caches and snapshots key on this version.
async fn bump(state: &AppState, project: Uuid, locale: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO edge_catalog_version AS v (project_id, locale) VALUES ($1, $2)
         ON CONFLICT (project_id, locale) DO UPDATE SET version = v.version + 1, updated_at = now()",
    )
    .bind(project)
    .bind(locale)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn set_status(
    state: &AppState,
    p: &ProjectRow,
    id: Uuid,
    locale: &str,
    status: &str,
    issues: Option<&[Issue]>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE local_seo_overrides SET status = $2, issues = coalesce($3, issues), updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .bind(issues.map(DbJson))
        .execute(&state.db)
        .await?;
    bump(state, p.id, locale).await
}

// ---------- decisions ----------

async fn row(state: &AppState, project: Uuid, id: Uuid) -> ApiResult<OverrideRow> {
    sqlx::query_as(&format!(
        "SELECT {ROW_COLS} FROM local_seo_overrides WHERE project_id = $1 AND id = $2"
    ))
    .bind(project)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("proposal not found"))
}

/// Issues of `content` for the row's page and market, against the site as it is now.
async fn check(
    state: &AppState,
    p: &ProjectRow,
    r: &OverrideRow,
    content: &Content,
) -> ApiResult<Vec<Issue>> {
    let page: Option<DbJson<Page>> = sqlx::query_scalar(
        "SELECT content FROM local_seo_pages WHERE project_id = $1 AND path = $2",
    )
    .bind(p.id)
    .bind(&r.path)
    .fetch_optional(&state.db)
    .await?;
    let page = page.map(|p| p.0).unwrap_or_default();
    let rows = rows_of(state, p.id).await?;
    let (siblings, regions) = neighbours(&rows, &r.locale, &r.path);
    let (m, brands) = (market(&r.locale), brands(state, p.id).await?);
    let terms: Vec<String> = r.terms.iter().map(|t| t.term.clone()).collect();
    let ctx = Ctx {
        market: &m,
        source_locale: &p.source_locale,
        page: &page,
        current: &r.current,
        terms: &terms,
        brands: &brands,
        siblings: &siblings,
        regions: &regions,
    };
    Ok(validate::validate(content, &ctx))
}

fn refuse(issues: &[Issue]) -> AppError {
    let list: Vec<&str> = issues
        .iter()
        .filter(|i| i.severity == validate::Severity::Error)
        .map(|i| i.message.as_str())
        .collect();
    unprocessable(
        "has_errors",
        format!("This content cannot go live yet: {}", list.join(" ")),
    )
}

async fn decide(
    state: &AppState,
    p: &ProjectRow,
    user: Uuid,
    r: &OverrideRow,
    status: &str,
) -> ApiResult<()> {
    sqlx::query(
        "UPDATE local_seo_overrides SET status = $2, decided_by = $3, decided_at = now(),
            applied_at = CASE WHEN $2 = 'approved' THEN coalesce(applied_at, now()) END, updated_at = now() WHERE id = $1",
    )
    .bind(r.id)
    .bind(status)
    .bind(user)
    .execute(&state.db)
    .await?;
    Ok(bump(state, p.id, &r.locale).await?)
}

/// `POST …/overrides/{id}/approve`: the consent. Refused while a validator reports an error.
async fn approve(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let r = row(&state, p.id, id).await?;
    if r.effective() == Content::default() {
        return Err(unprocessable(
            "nothing_to_approve",
            "There is no proposal for this page and market yet.",
        ));
    }
    let issues = check(&state, &p, &r, &r.effective()).await?;
    if has_errors(&issues) {
        return Err(refuse(&issues));
    }
    decide(&state, &p, user.id, &r, "approved").await?;
    Ok(Json(json!({ "status": "approved" })))
}

/// `POST …/overrides/{id}/reject`: never served; also how an approved override is taken down.
async fn reject(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let r = row(&state, p.id, id).await?;
    decide(&state, &p, user.id, &r, "rejected").await?;
    Ok(Json(json!({ "status": "rejected" })))
}

#[derive(Deserialize)]
struct EditBody {
    content: Content,
    /// Save and approve in one step.
    #[serde(default)]
    approve: bool,
}

/// `PATCH …/overrides/{id}`: a human's version replaces the proposal. It goes through the same validators, and
/// an override that was live goes back to draft until approved again.
async fn edit(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<EditBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let r = row(&state, p.id, id).await?;
    let content = b.content.clean();
    let issues = check(&state, &p, &r, &content).await?;
    if b.approve && has_errors(&issues) {
        return Err(refuse(&issues));
    }
    sqlx::query("UPDATE local_seo_overrides SET edited = $2, issues = $3, status = 'proposed', updated_at = now() WHERE id = $1")
        .bind(r.id)
        .bind(DbJson(&content))
        .bind(DbJson(&issues))
        .execute(&state.db)
        .await?;
    if b.approve {
        decide(&state, &p, user.id, &r, "approved").await?;
    } else if served(&r.status) {
        bump(&state, p.id, &r.locale).await?;
    }
    Ok(Json(
        json!({ "status": if b.approve { "approved" } else { "proposed" }, "issues": issues }),
    ))
}

// ---------- dashboard ----------

#[derive(Serialize, Default)]
struct Window {
    days: i64,
    impressions: i64,
    clicks: i64,
    /// Impression-weighted average position; `None` without impressions.
    position: Option<f64>,
}

/// `GET …/local-seo`: every page × market with what it shows, what is proposed, why, and what search says.
async fn view(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let cfg = settings(&state, p.id).await?;
    let site = site_info(&state, &p, &cfg).await?;
    let rows = rows_of(&state, p.id).await?;
    let pages: Vec<(String, DbJson<Page>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT path, content, scanned_at FROM local_seo_pages WHERE project_id = $1 ORDER BY path",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let brands = brands(&state, p.id).await?;
    // Search performance 28 days before / after each override went live. Real rows only: no row, no number.
    let metrics: Vec<(Uuid, bool, i64, i64, i64, Option<f64>)> = sqlx::query_as(
        "SELECT o.id, m.day >= o.applied_at::date, count(*), sum(m.impressions)::bigint, sum(m.clicks)::bigint,
                (sum(m.position * m.impressions) / nullif(sum(m.impressions), 0))::float8
         FROM local_seo_overrides o JOIN local_seo_metrics m
           ON m.project_id = o.project_id AND m.locale = o.locale AND m.path = o.path
         WHERE o.project_id = $1 AND o.applied_at IS NOT NULL
           AND m.day >= o.applied_at::date - 28 AND m.day < o.applied_at::date + 28
         GROUP BY 1, 2",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let connection: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT site_url, connected_at FROM local_seo_connections WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    let manual: Vec<(String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT locale, path, term, impressions FROM local_seo_demand WHERE project_id = $1 AND source = 'manual' ORDER BY locale, path, impressions DESC NULLS LAST, term",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;

    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            // The audit of what the market shows today needs no demand data at all.
            let page = pages.iter().find(|(path, ..)| path == &r.path).map(|(_, c, _)| c.0.clone()).unwrap_or_default();
            let m = market(&r.locale);
            let (mut siblings, _) = neighbours(&rows, &r.locale, &r.path);
            siblings.iter_mut().for_each(|s| s.effective = Content::default());
            let ctx = Ctx { market: &m, source_locale: &p.source_locale, page: &page, current: &r.current, terms: &[], brands: &brands, siblings: &siblings, regions: &[] };
            let audit = validate::validate(&Content { slug: None, ..r.current.0.clone() }, &ctx);
            let window = |after: bool| {
                metrics.iter().find(|(id, a, ..)| *id == r.id && *a == after).map(|(_, _, days, impressions, clicks, position)| Window {
                    days: *days,
                    impressions: *impressions,
                    clicks: *clicks,
                    position: *position,
                })
            };
            json!({
                "id": r.id, "path": r.path, "locale": r.locale, "status": r.status,
                "url": site.url(&r.locale, &r.path),
                "source": {"title": page.title, "description": page.description, "h1": page.h1},
                "current": r.current, "proposed": r.proposed, "edited": r.edited, "effective": r.effective(),
                "terms": r.terms, "rationale": r.rationale, "issues": r.issues, "audit": audit,
                "model": r.model, "decided_at": r.decided_at, "applied_at": r.applied_at, "updated_at": r.updated_at,
                "metrics": r.applied_at.map(|_| json!({"before": window(false), "after": window(true)})),
            })
        })
        .collect();
    Ok(Json(json!({
        "mode": cfg.mode,
        "site": {"origin": site.origin, "saved_url": cfg.site_url, "edge": !site.bases.is_empty()},
        "llm_provider": p.llm.0.provider,
        "gsc": {
            "configured": demand::Google::from_env().is_some(),
            "connected": connection.is_some(),
            "site_url": connection.as_ref().map(|c| &c.0),
            "connected_at": connection.as_ref().map(|c| c.1),
        },
        "markets": p.target_locales.iter().map(|l| market(l)).collect::<Vec<_>>(),
        "pages": pages.iter().map(|(path, c, at)| json!({"path": path, "title": c.title, "scanned_at": at})).collect::<Vec<_>>(),
        "manual_demand": manual.iter().map(|(locale, path, term, n)| json!({"locale": locale, "path": path, "term": term, "impressions": n})).collect::<Vec<_>>(),
        "rows": out,
    })))
}

// ---------- Google Search Console ----------

async fn search_console(
    state: &AppState,
    project: Uuid,
) -> Result<Option<demand::SearchConsole>, String> {
    let row: Option<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT site_url, refresh_token_enc FROM local_seo_connections WHERE project_id = $1",
    )
    .bind(project)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let Some((site_url, enc)) = row else {
        return Ok(None);
    };
    let google = demand::Google::from_env()
        .ok_or("Search Console is connected but this server no longer has GOOGLE_OAUTH_CLIENT_ID / GOOGLE_OAUTH_CLIENT_SECRET.")?;
    let refresh_token = crate::auth::decrypt(&state.secret, &enc)
        .ok_or("The Search Console connection cannot be decrypted (GLOT_SECRET_KEY changed?): connect again.")?;
    Ok(Some(demand::SearchConsole {
        google,
        site_url,
        refresh_token,
    }))
}

fn web_origin() -> String {
    std::env::var("GLOT_WEB_ORIGIN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "http://localhost:3100".into())
        .trim_end_matches('/')
        .to_string()
}

/// On the dashboard origin (proxied to the API): the session cookie lives there, and the callback checks that the
/// signed-in user is the one who started the connection (no login-CSRF).
fn redirect_uri() -> String {
    format!("{}/api/local-seo/gsc/callback", web_origin())
}

fn sign(secret: &[u8; 32], payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(b"local-seo-gsc\0");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// `slug.user.expiry.signature`: the callback only acts for the admin who started the flow, for ten minutes.
fn make_state(secret: &[u8; 32], slug: &str, user: Uuid) -> String {
    let payload = format!("{slug}.{user}.{}", Utc::now().timestamp() + 600);
    format!("{payload}.{}", sign(secret, &payload))
}

fn read_state(secret: &[u8; 32], state: &str, user: Uuid) -> Option<String> {
    let (payload, sig) = state.rsplit_once('.')?;
    let mut parts = payload.split('.');
    let (slug, uid, exp) = (
        parts.next()?,
        parts.next()?,
        parts.next()?.parse::<i64>().ok()?,
    );
    // ponytail: plain comparison of a hex HMAC; the signature is single-use-window and not a password.
    (sign(secret, payload) == sig && uid == user.to_string() && exp >= Utc::now().timestamp())
        .then(|| slug.to_string())
}

/// `GET …/gsc/connect` → `{url}`: where the customer gives glot read-only access to their Search Console.
async fn gsc_connect(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let Some(google) = demand::Google::from_env() else {
        return Err(unprocessable(
            "gsc_not_configured",
            "This glot server has no Google OAuth client yet. Its operator must set GOOGLE_OAUTH_CLIENT_ID and GOOGLE_OAUTH_CLIENT_SECRET (see the Local SEO documentation).",
        ));
    };
    Ok(Json(
        json!({ "url": google.consent_url(&redirect_uri(), &make_state(&state.secret, &p.slug, user.id)) }),
    ))
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Google's redirect. Every outcome lands on the dashboard page with a reason it can show.
async fn gsc_callback(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppQuery(q): AppQuery<Callback>,
) -> ApiResult<Redirect> {
    let slug = q
        .state
        .as_deref()
        .and_then(|s| read_state(&state.secret, s, user.id))
        .ok_or_else(|| AppError::bad_request("This Search Console link expired or belongs to another account: start again from Local SEO."))?;
    let page = format!("{}/app/{slug}/local-seo", web_origin());
    let fail =
        |code: &str| -> ApiResult<Redirect> { Ok(Redirect::to(&format!("{page}?gsc={code}"))) };
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let (Some(code), None) = (q.code, q.error) else {
        return fail("denied");
    };
    let Some(google) = demand::Google::from_env() else {
        return fail("not_configured");
    };
    let Ok(refresh) = google.exchange(&code, &redirect_uri()).await else {
        return fail("exchange_failed");
    };
    let Ok(access) = google.access_token(&refresh).await else {
        return fail("exchange_failed");
    };
    let Ok(properties) = google.sites(&access).await else {
        return fail("no_property");
    };
    let cfg = settings(&state, p.id).await?;
    let site = site_info(&state, &p, &cfg).await?;
    let property = match (&site.origin, properties.as_slice()) {
        (Some(o), _) => demand::pick_property(&properties, o),
        (None, [only]) => Some(only.clone()),
        _ => None,
    };
    let Some(property) = property else {
        return fail("no_property");
    };
    sqlx::query(
        "INSERT INTO local_seo_connections (project_id, site_url, refresh_token_enc, connected_by) VALUES ($1, $2, $3, $4)
         ON CONFLICT (project_id) DO UPDATE SET site_url = $2, refresh_token_enc = $3, connected_by = $4, connected_at = now()",
    )
    .bind(p.id)
    .bind(property)
    .bind(crate::auth::encrypt(&state.secret, &refresh))
    .bind(user.id)
    .execute(&state.db)
    .await?;
    fail("connected")
}

/// `DELETE …/gsc`: forgets the token. Stored metrics stay (they are the customer's history).
async fn gsc_disconnect(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    sqlx::query("DELETE FROM local_seo_connections WHERE project_id = $1")
        .bind(p.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Daily Search Console numbers of every page × market that has (or had) an override live.
async fn pull_metrics(
    state: &AppState,
    p: &ProjectRow,
    gsc: &demand::SearchConsole,
    site: &SiteInfo,
) -> ApiResult<(usize, Vec<String>)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT locale, path FROM local_seo_overrides WHERE project_id = $1 AND applied_at IS NOT NULL")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let (mut n, mut errors) = (0, vec![]);
    for (locale, path) in rows {
        let Some(url) = site.url_or_origin(&locale, &path) else {
            errors.push(format!(
                "{locale} {path}: glot does not know this page's URL in that market."
            ));
            continue;
        };
        match gsc.daily(&market(&locale), &url, 90).await {
            Ok(days) => {
                for (day, impressions, clicks, position) in days {
                    sqlx::query(
                        "INSERT INTO local_seo_metrics (project_id, locale, path, day, impressions, clicks, position) VALUES ($1, $2, $3, $4, $5, $6, $7)
                         ON CONFLICT (project_id, locale, path, day) DO UPDATE SET impressions = $5, clicks = $6, position = $7",
                    )
                    .bind(p.id)
                    .bind(&locale)
                    .bind(&path)
                    .bind(day)
                    .bind(impressions)
                    .bind(clicks)
                    .bind(position)
                    .execute(&state.db)
                    .await?;
                    n += 1;
                }
            }
            Err(e) => {
                if !errors.contains(&e) {
                    errors.push(e)
                }
            }
        }
    }
    Ok((n, errors))
}

/// `POST …/metrics/refresh`.
async fn refresh_metrics(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let gsc = search_console(&state, p.id)
        .await
        .map_err(|e| unprocessable("gsc_error", e))?
        .ok_or_else(|| {
            unprocessable(
                "gsc_not_connected",
                "Search Console is not connected: there is no search data to read yet.",
            )
        })?;
    let cfg = settings(&state, p.id).await?;
    let site = site_info(&state, &p, &cfg).await?;
    let (days, errors) = pull_metrics(&state, &p, &gsc, &site).await?;
    Ok(Json(json!({ "days": days, "errors": errors })))
}
