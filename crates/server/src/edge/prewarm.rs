//! Pre-warm: glot reads the source site before visitors and crawlers do, so a page is already translated the
//! first time it is asked for — and is only advertised (self canonical, hreflang, sitemap) once it is complete
//! (`seo_report::page_ready`; this module never weakens that gate, it fills what the gate looks at).
//!
//! One run per project at a time (`edge_prewarm` row, claimed atomically, so several replicas start one):
//! the origin's sitemap first, then links found on the fetched pages, at most `edge_settings.prewarm_pages`
//! pages, one request per [`delay`] (default 1 s), sequential — bounded CPU and memory on a small server, a
//! polite crawl on the customer's origin. Skipped: paths robots.txt disallows, application paths, paths outside
//! the discovery zones, URLs with a query string, non-HTML, `noindex` pages, redirects off the site.
//! Each page goes through `missing::record` exactly like a page a visitor rendered: same personal-data filter,
//! same daily cap on new segments, same monthly budget, same review rules. Nothing is translated here.
//!
//! Runs: on demand (`POST /api/projects/{slug}/edge/prewarm`), when a domain is verified, and at most once a day
//! for sites with traffic ([`kick`], from the proxy) when `edge_settings.prewarm` is on.
use axum::{Json, Router, extract::State, routing::get};
use glot_core::edge::{
    hydration,
    segment::{SegmentOpts, segment},
    seo,
};
use reqwest::Url;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::discover::Settings;
use super::{app_paths, missing, sitemap};
use crate::{
    ApiResult, AppError, AppPath, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
};

const PAGE_BYTES: usize = 2 * 1024 * 1024;
/// Automatic runs: at most one per project per day.
const EVERY: &str = "24 hours";

/// Pause between two origin requests (`GLOT_EDGE_PREWARM_DELAY_MS`, default 1000).
fn delay() -> Duration {
    Duration::from_millis(
        std::env::var("GLOT_EDGE_PREWARM_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000),
    )
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/projects/{slug}/edge/prewarm",
        get(status).post(run_now),
    )
}

async fn status_of(state: &AppState, project: Uuid) -> ApiResult<Value> {
    type Row = (
        String,
        i32,
        i32,
        i32,
        i32,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT status, pages_total, pages_done, skipped, segments, error, started_at, finished_at
         FROM edge_prewarm WHERE project_id = $1",
    )
    .bind(project)
    .fetch_optional(&state.db)
    .await?;
    Ok(match row {
        Some((status, total, done, skipped, segments, error, started, finished)) => json!({
            "status": status, "pages_total": total, "pages_done": done, "skipped": skipped,
            "segments": segments, "error": error, "started_at": started, "finished_at": finished,
        }),
        None => json!({"status": "never"}),
    })
}

async fn status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(status_of(&state, p.id).await?))
}

async fn run_now(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if !Settings::load(&state.db, p.id).await?.enabled {
        return Err(AppError::bad_request(
            "glot Edge is switched off for this project",
        ));
    }
    start(&state, &p, true).await?;
    Ok(Json(status_of(&state, p.id).await?))
}

/// Claims the project's run and starts it in the background; `false` = one is running (or, unless `force`,
/// one finished less than a day ago).
pub async fn start(state: &AppState, p: &ProjectRow, force: bool) -> ApiResult<bool> {
    let claimed: Option<i32> = sqlx::query_scalar(&format!(
        "INSERT INTO edge_prewarm (project_id) VALUES ($1)
         ON CONFLICT (project_id) DO UPDATE SET status = 'running', pages_total = 0, pages_done = 0, skipped = 0,
            segments = 0, error = NULL, started_at = now(), finished_at = NULL
         WHERE (edge_prewarm.status <> 'running' OR edge_prewarm.started_at < now() - interval '2 hours')
           AND ($2 OR edge_prewarm.started_at < now() - interval '{EVERY}')
         RETURNING 1"
    ))
    .bind(p.id)
    .bind(force)
    .fetch_optional(&state.db)
    .await?;
    if claimed.is_none() {
        return Ok(false);
    }
    let (state, p) = (state.clone(), p.clone());
    tokio::spawn(async move {
        let res = run(&state, &p).await;
        let error = res.err().map(|e| e.message);
        if let Some(e) = &error {
            tracing::info!(target: "glot_edge", project = %p.slug, "pre-warm failed: {e}");
        }
        let _ = sqlx::query(
            "UPDATE edge_prewarm SET status = CASE WHEN $2::text IS NULL THEN 'done' ELSE 'failed' END,
                error = $2, finished_at = now() WHERE project_id = $1",
        )
        .bind(p.id)
        .bind(error)
        .execute(&state.db)
        .await;
    });
    Ok(true)
}

static CHECKED: Mutex<Option<HashMap<Uuid, Instant>>> = Mutex::new(None);

/// From the proxy, on traffic: starts the daily run when it is due. In-process throttle (one database look per
/// project per hour), never in the request's way.
pub fn kick(state: &AppState, project: Uuid, slug: &str) {
    {
        let Ok(mut g) = CHECKED.lock() else { return };
        let m = g.get_or_insert_with(HashMap::new);
        if m.get(&project)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(3600))
        {
            return;
        }
        if m.len() > 10_000 {
            m.clear();
        }
        m.insert(project, Instant::now());
    }
    let (state, slug) = (state.clone(), slug.to_string());
    tokio::spawn(async move {
        let Ok(Some(p)) = projects::load_public(&state, &slug).await else {
            return;
        };
        if let Ok(s) = Settings::load(&state.db, p.id).await
            && s.enabled
            && s.prewarm
        {
            let _ = start(&state, &p, false).await;
        }
    });
}

/// `/a/b?x#y` → `/a/b` for pages worth fetching; None for URLs with a query string and static files.
fn page_path(path: &str) -> Option<String> {
    let p = path.split('#').next().unwrap_or("");
    if p.contains('?') || !p.starts_with('/') || p.len() > 2048 {
        return None;
    }
    Some(p.to_string())
}

async fn run(state: &AppState, p: &ProjectRow) -> ApiResult<()> {
    let settings = Settings::load(&state.db, p.id).await?;
    let site = sitemap::site(state, p, false).await?;
    let Some(origin) = site.origin.clone() else {
        return Err(AppError::bad_request("no verified Edge domain"));
    };
    let Some(locale) = p.target_locales.first().cloned() else {
        return Ok(());
    };
    let budget = settings.prewarm_pages.clamp(1, 2000) as usize;
    let robots = match sitemap::fetch(&format!("{origin}/robots.txt"), 256 * 1024).await {
        Ok((200, _, body, _)) => body,
        _ => String::new(),
    };
    let default = if settings.app_action == "proxy" {
        app_paths::Action::Proxy
    } else {
        app_paths::Action::Redirect
    };
    let dnt: Vec<String> = sqlx::query_scalar(
        "SELECT term FROM glossary_terms WHERE project_id = $1 AND do_not_translate",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let skip: Vec<&str> = settings
        .selectors_exclude
        .iter()
        .map(String::as_str)
        .collect();
    let mut opts = SegmentOpts::new(&p.source_locale).skip(&skip);
    opts.dnt = dnt;

    // The home page, then the sitemap; links found on the way fill what is left of the budget.
    let mut queue: VecDeque<String> = std::iter::once("/".to_string())
        .chain(sitemap::source_paths(&site).await)
        .filter_map(|u| page_path(&u))
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let (mut done, mut skipped, mut segments) = (0usize, 0usize, 0usize);
    let mut first = true;
    while let Some(path) = queue.pop_front() {
        if done >= budget {
            break;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let allowed = !super::seo_report::robots_blocks(&robots, &path)
            && settings.discoverable(&path)
            && app_paths::classify(&settings.app_paths, default, &path).is_none();
        if !allowed {
            skipped += 1;
            continue;
        }
        if !std::mem::take(&mut first) {
            tokio::time::sleep(delay()).await;
        }
        let Some(html) = page(state, p, &settings, &origin, &path).await else {
            skipped += 1;
            continue;
        };
        if seo::inspect(&html)
            .robots
            .iter()
            .any(|r| r.contains("noindex") || r.contains("none"))
        {
            skipped += 1;
            continue;
        }
        let segs = segment(&html, &opts);
        let report = missing::Report {
            key: None,
            locale: locale.clone(),
            auth: false,
            segments: segs
                .iter()
                .filter(|s| s.discoverable)
                .map(|s| missing::Item {
                    hash: s.hash.clone(),
                    text: s.text.clone(),
                    url: path.clone(),
                    role: s.ctx.role.clone(),
                })
                .chain(
                    seo::jsonld_texts(&html)
                        .into_iter()
                        .map(|text| missing::Item {
                            hash: glot_core::edge::tm::text_hash(&p.source_locale, &text),
                            text,
                            url: path.clone(),
                            role: "text".into(),
                        }),
                )
                .collect(),
            pages: vec![missing::Page {
                url: path.clone(),
                seen: segs
                    .iter()
                    .map(|s| missing::Seen {
                        hash: s.hash.clone(),
                        role: s.ctx.role.clone(),
                    })
                    .collect(),
            }],
            mode: None,
        };
        let out = missing::record(state, p, &report).await?;
        if out.skipped.is_some() {
            return Ok(()); // switched off meanwhile
        }
        segments += out.accepted;
        if settings.auto_translate {
            for l in p.target_locales.iter().skip(1) {
                super::tm::queue(state, p, l, &settings).await?;
            }
        }
        done += 1;
        let found: Vec<String> = {
            let noop = |_: &str| None;
            let ctx = site.seo_ctx(&p.source_locale, "/", &noop);
            seo::links(&html)
                .iter()
                .filter_map(|href| page_path(&ctx.site_path(href)?))
                .collect()
        };
        for next in found {
            if seen.len() + queue.len() >= budget * 3 {
                break;
            }
            if !seen.contains(&next) && !queue.contains(&next) {
                queue.push_back(next);
            }
        }
        sqlx::query(
            "UPDATE edge_prewarm SET pages_total = $2, pages_done = $3, skipped = $4, segments = $5
             WHERE project_id = $1",
        )
        .bind(p.id)
        .bind((done + queue.len()).min(budget) as i32)
        .bind(done as i32)
        .bind(skipped as i32)
        .bind(segments as i32)
        .execute(&state.db)
        .await?;
    }
    sqlx::query(
        "UPDATE edge_prewarm SET pages_total = $2, pages_done = $2, skipped = $3, segments = $4 WHERE project_id = $1",
    )
    .bind(p.id)
    .bind(done as i32)
    .bind(skipped as i32)
    .bind(segments as i32)
    .execute(&state.db)
    .await?;
    // Translations settle before the run is "done": the jobs take 500 segments at a time and nothing else
    // would queue the rest (or the slug texts `materialize` asks for) until the next visitor.
    settle(state, p, &settings).await?;
    if settings.translated_slugs {
        // First pass: slugs the TM already gives, and the texts it lacks are reported; second: those, translated.
        super::slugs::materialize(state, p).await?;
        settle(state, p, &settings).await?;
        super::slugs::materialize(state, p).await?;
    }
    Ok(())
}

/// Waits (bounded) until no auto-translation job is queued, running or needed for any target locale.
async fn settle(state: &AppState, p: &ProjectRow, settings: &Settings) -> ApiResult<()> {
    if !settings.auto_translate {
        return Ok(());
    }
    let poll = (delay() * 5).max(Duration::from_millis(200));
    for _ in 0..360 {
        let mut busy = false;
        for l in &p.target_locales {
            busy |= super::tm::queue(state, p, l, settings).await?;
        }
        if !busy {
            break;
        }
        tokio::time::sleep(poll).await;
    }
    Ok(())
}

/// The source HTML of `path` as the proxy would translate it: the origin's page, or for an SPA shell the stored
/// snapshot when there is one.
async fn page(
    state: &AppState,
    p: &ProjectRow,
    settings: &Settings,
    origin: &str,
    path: &str,
) -> Option<String> {
    let url = format!("{origin}{path}");
    let (status, headers, html, last) = sitemap::fetch(&url, PAGE_BYTES).await.ok()?;
    let is_html = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("text/html"));
    // A redirect elsewhere is another page (queued on its own if it is linked).
    if status != 200 || !is_html || last.path() != path {
        return None;
    }
    let shell = match settings.rendering.as_str() {
        "prerender" => true,
        "auto" => hydration::is_skeleton(&html),
        _ => false,
    };
    if !shell {
        return Some(html);
    }
    // No snapshot yet (the renderer queues it): the origin page, exactly what the proxy serves meanwhile.
    let snap = super::proxy::snapshot(
        state,
        p.id,
        &Url::parse(&url).ok()?,
        Instant::now() + Duration::from_secs(5),
    )
    .await;
    Some(snap.map_or(html, |b| String::from_utf8_lossy(&b).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_pages_are_fetched() {
        assert_eq!(page_path("/pricing#plans").as_deref(), Some("/pricing"));
        assert_eq!(page_path("/search?q=x"), None);
        assert_eq!(page_path("mailto:a@b.c"), None);
    }
}
