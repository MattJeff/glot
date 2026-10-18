//! Layout RUM: translated strings that overflow their box for real visitors, then repaired.
//! `@glot/insights/layout` (1 % of page views, GPC/DNT/bots excluded) posts per page view which keys rendered and
//! which overflowed, with the box / needed width and the font → counters per (day, key, locale, viewport band).
//! The repair loop (hourly worker + `POST …/insights/layout/repair`) takes keys whose overflow rate in one band is
//! above `GLOT_LAYOUT_MIN_RATE` (10 %) over ≥ `GLOT_LAYOUT_MIN_VIEWS` (30) views, asks the shorten pipeline of
//! `glot fit` for a variant (validator + pixel budget measured from the visitors' boxes), and opens one PR through
//! the GitHub App link — or, with no linked repo, writes it as a proposed correction in the feedback inbox.
//! Privacy: no IP, user agent, URL, text, cookie or visitor id is stored.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use chrono::{DateTime, Utc};
use glot_core::{
    lock::{Lockfile, Origin},
    measure::{Font, MARGIN, measure_text},
    validate::has_errors,
};
use glot_llm::visual::{Shorten, parse_shortened};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppPath, AppQuery, AppState,
    auth::AuthUser,
    github_app as app,
    projects::{self, ProjectRow, Role},
    validate_key, validate_locale,
};

/// Viewport band lower bounds (px): phones 360/390/430, tablets, laptops, desktops.
pub const BANDS: [i32; 10] = [0, 360, 390, 430, 600, 768, 1024, 1280, 1440, 1920];

pub fn band(vw: i32) -> (i32, Option<i32>) {
    let i = BANDS.iter().rposition(|b| *b <= vw).unwrap_or(0);
    (BANDS[i], BANDS.get(i + 1).copied())
}

fn env_num(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn min_rate() -> f64 {
    env_num("GLOT_LAYOUT_MIN_RATE", 0.10)
}
fn min_views() -> i64 {
    env_num("GLOT_LAYOUT_MIN_VIEWS", 30.0) as i64
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/insights/{key}/layout", post(ingest))
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/insights/layout";
    Router::new()
        .route(p, get(overview))
        .route(&format!("{p}/repair"), post(repair_now))
}

// ---------- ingestion ----------

#[derive(Deserialize)]
struct Beacon {
    locale: String,
    vw: i32,
    items: Vec<BeaconItem>,
}

#[derive(Deserialize)]
struct BeaconItem {
    k: String,
    o: u8,
    #[serde(default)]
    b: i64,
    #[serde(default)]
    n: i64,
    #[serde(default)]
    f: String,
    #[serde(default)]
    fs: f32,
    #[serde(default = "one")]
    fl: u8,
}
fn one() -> u8 {
    1
}

async fn ingest(
    State(state): State<AppState>,
    AppPath(key): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let b: Beacon = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    validate_locale(&b.locale)?;
    if b.items.len() > 200 || !(0..=20_000).contains(&b.vw) {
        return Err(AppError::bad_request("at most 200 items, vw 0-20000"));
    }
    if crate::experiments::ingest::is_bot(&headers) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    // One row per key (a key may be listed twice: overflow wins), sizes only for overflows.
    let mut rows: BTreeMap<String, (i64, i64, i64, i64, String)> = BTreeMap::new();
    for i in b.items {
        validate_key(&i.k)?;
        let over = i.o == 1 && (1..=20_000).contains(&i.b) && i.n > i.b && i.n <= 100_000;
        let row = if over {
            let font = format!(
                "{} {}",
                i.f.chars().take(40).collect::<String>(),
                i.fs.clamp(1.0, 200.0)
            );
            (1, i64::from(i.fl == 0), i.b, i.n, font)
        } else {
            (0, 0, 0, 0, String::new())
        };
        let e = rows.entry(i.k).or_insert(row.clone());
        if row.0 > e.0 {
            *e = row;
        }
    }
    let project_id = match crate::insights::admit(&state, &key, 1).await? {
        Ok(p) => p,
        Err(limited) => return Ok(limited),
    };
    crate::billing::consume(&state, project_id, crate::billing::INSIGHTS).await?;
    let (lo, _) = band(b.vw);
    let mut cols: (
        Vec<String>,
        Vec<i64>,
        Vec<i64>,
        Vec<i64>,
        Vec<i64>,
        Vec<String>,
    ) = Default::default();
    for (k, (o, fb, bx, n, f)) in rows {
        cols.0.push(k);
        cols.1.push(o);
        cols.2.push(fb);
        cols.3.push(bx);
        cols.4.push(n);
        cols.5.push(f);
    }
    sqlx::query(
        "INSERT INTO layout_rum_daily AS d (project_id, day, key, locale, vw_min, views, overflows, fallback, box_px, need_px, font)
         SELECT $1, (now() AT TIME ZONE 'utc')::date, k, $2, $3, 1, o, fb, b, n, f
         FROM UNNEST($4::text[], $5::int8[], $6::int8[], $7::int8[], $8::int8[], $9::text[]) AS t(k, o, fb, b, n, f)
         ON CONFLICT (project_id, day, key, locale, vw_min) DO UPDATE SET
           views = d.views + 1, overflows = d.overflows + excluded.overflows, fallback = d.fallback + excluded.fallback,
           box_px = d.box_px + excluded.box_px, need_px = d.need_px + excluded.need_px,
           font = CASE WHEN excluded.font <> '' THEN excluded.font ELSE d.font END",
    )
    .bind(project_id)
    .bind(&b.locale)
    .bind(lo)
    .bind(&cols.0)
    .bind(&cols.1)
    .bind(&cols.2)
    .bind(&cols.3)
    .bind(&cols.4)
    .bind(&cols.5)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------- read ----------

#[derive(Serialize, sqlx::FromRow, Debug, Clone)]
pub struct Row {
    pub key: String,
    pub locale: String,
    pub vw_min: i32,
    #[sqlx(skip)]
    pub vw_max: Option<i32>,
    pub views: i64,
    pub overflows: i64,
    /// Overflows while the webfont had not loaded (font swap, not the copy).
    pub fallback: i64,
    /// (overflows − fallback) / views.
    pub rate: f64,
    /// Average box / needed width over the overflows.
    pub box_px: i64,
    pub need_px: i64,
    pub font: String,
    /// Live translation (None: key unknown to the project).
    pub current: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Fix {
    pub id: Uuid,
    pub key: String,
    pub locale: String,
    pub previous: String,
    pub proposal: Option<String>,
    pub evidence: String,
    /// pending | pr | suggested | failed
    pub status: String,
    pub pr_url: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct Q {
    days: Option<i32>,
}

/// Rows of the last `days`, worst first; `min` = only bands above the repair thresholds.
async fn rows(
    state: &AppState,
    p: &ProjectRow,
    days: i32,
    min: Option<(f64, i64)>,
) -> ApiResult<Vec<Row>> {
    let (rate, views) = min.unwrap_or((-1.0, 1));
    let mut rows: Vec<Row> = sqlx::query_as(
        "SELECT r.*, t.value AS current FROM (
           SELECT key, locale, vw_min, sum(views)::int8 AS views, sum(overflows)::int8 AS overflows,
             sum(fallback)::int8 AS fallback, (sum(overflows) - sum(fallback))::float8 / sum(views) AS rate,
             (sum(box_px) / greatest(sum(overflows), 1))::int8 AS box_px,
             (sum(need_px) / greatest(sum(overflows), 1))::int8 AS need_px, max(font) AS font
           FROM layout_rum_daily WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2
           GROUP BY key, locale, vw_min HAVING sum(views) >= $4) r
         LEFT JOIN keys k ON k.project_id = $1 AND k.key = r.key
         LEFT JOIN translations t ON t.key_id = k.id AND t.locale = r.locale
         WHERE r.rate > $3 ORDER BY r.rate DESC, r.views DESC LIMIT 200",
    )
    .bind(p.id)
    .bind(days.clamp(1, 90))
    .bind(rate)
    .bind(views)
    .fetch_all(&state.db)
    .await?;
    for r in &mut rows {
        r.vw_max = band(r.vw_min).1;
    }
    Ok(rows)
}

async fn fixes(state: &AppState, project: Uuid) -> ApiResult<Vec<Fix>> {
    Ok(sqlx::query_as(
        "SELECT id, key, locale, previous, proposal, evidence, status, pr_url, error, created_at
         FROM layout_rum_fixes WHERE project_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(project)
    .fetch_all(&state.db)
    .await?)
}

async fn overview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<Q>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days = q.days.unwrap_or(30);
    Ok(Json(json!({
        "days": days,
        "min_rate": min_rate(),
        "min_views": min_views(),
        "rows": rows(&state, &p, days, None).await?,
        "fixes": fixes(&state, p.id).await?,
    })))
}

async fn repair_now(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let created = repair(&state, &p).await?;
    Ok(Json(
        json!({ "created": created, "fixes": fixes(&state, p.id).await? }),
    ))
}

// ---------- repair loop ----------

/// "'Zahlungserinnerung senden' is truncated for 31 % of mobile DE visitors (360–390 px, 412 views in 7 days)".
pub fn evidence(r: &Row, days: i32) -> String {
    let device = match r.vw_max {
        Some(m) if m <= 600 => "mobile ",
        Some(m) if m <= 1024 => "tablet ",
        _ => "",
    };
    let band = match r.vw_max {
        Some(m) => format!("{}–{m} px", r.vw_min),
        None => format!("≥ {} px", r.vw_min),
    };
    format!(
        "'{}' is truncated for {:.0} % of {device}{} visitors ({band}, {} views in {days} days)",
        r.current.as_deref().unwrap_or(&r.key),
        r.rate * 100.0,
        r.locale.to_uppercase(),
        r.views
    )
}

/// Does `candidate` fit where `current` needed `need` px for a `boxw` px box? Measured with the same font,
/// so the ratio holds even when our metrics differ from the browser's.
pub fn fits(
    candidate: &str,
    current: &str,
    font: &Font,
    boxw: i64,
    need: i64,
) -> Result<(), String> {
    let (cur, cand) = (measure_text(current, font), measure_text(candidate, font));
    let px = |w: f32| w * need as f32 / cur.max(1.0);
    if cur <= 0.0 || px(cand) <= boxw as f32 * (1.0 - MARGIN) {
        Ok(())
    } else {
        Err(format!(
            "still too wide: ~{:.0}px for a {boxw}px box",
            px(cand)
        ))
    }
}

fn font_of(s: &str) -> Font {
    let (family, size) = s.rsplit_once(' ').unwrap_or((s, "16"));
    Font {
        family: family.to_string(),
        postscript: None,
        size_px: size.parse().unwrap_or(16.0),
        weight: 400,
        letter_spacing_px: 0.0,
        text_transform: None,
    }
}

pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    let secs = env_num("GLOT_LAYOUT_TICK_SECS", 3600.0).max(1.0) as u64;
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT DISTINCT project_id FROM layout_rum_daily WHERE day > (now() AT TIME ZONE 'utc')::date - 7",
            )
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
            for id in ids {
                let r = async {
                    let p: ProjectRow = sqlx::query_as(
                        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at FROM projects WHERE id = $1",
                    )
                    .bind(id)
                    .fetch_one(&state.db)
                    .await?;
                    repair(&state, &p).await
                };
                if let Err(e) = r.await {
                    tracing::error!(project = %id, "layout repair failed: {}", e.message);
                }
            }
        }
    })
}

struct Proposal {
    fix: Uuid,
    key_id: Uuid,
    key: String,
    source: String,
    locale: String,
    previous: String,
    value: String,
    evidence: String,
}

/// One pass over the project: claim each new (key, locale, text) above the thresholds, shorten it, then one PR
/// (GitHub App link) or proposed corrections in the feedback inbox. Returns the number of fixes created.
pub async fn repair(state: &AppState, p: &ProjectRow) -> ApiResult<usize> {
    const DAYS: i32 = 7;
    let all = rows(state, p, DAYS, Some((min_rate(), min_views()))).await?;
    // Worst band per (key, locale); only target-locale translations can be shortened.
    let mut worst: BTreeMap<(String, String), Row> = BTreeMap::new();
    for r in all {
        if r.current.is_some() && r.locale != p.source_locale && r.need_px > r.box_px {
            worst.entry((r.key.clone(), r.locale.clone())).or_insert(r);
        }
    }
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let mut out = vec![];
    // ponytail: 10 keys per pass bounds LLM cost; the next tick takes the rest.
    for r in worst.into_values().take(10) {
        let current = r.current.clone().unwrap_or_default();
        let evidence = evidence(&r, DAYS);
        let fix = Uuid::new_v4();
        let claimed = sqlx::query(
            "INSERT INTO layout_rum_fixes (id, project_id, key, locale, previous, evidence, status)
             VALUES ($1, $2, $3, $4, $5, $6, 'pending') ON CONFLICT DO NOTHING",
        )
        .bind(fix)
        .bind(p.id)
        .bind(&r.key)
        .bind(&r.locale)
        .bind(&current)
        .bind(&evidence)
        .execute(&state.db)
        .await?
        .rows_affected();
        if claimed == 0 {
            continue;
        }
        let (key_id, source): (Uuid, String) =
            sqlx::query_as("SELECT id, source FROM keys WHERE project_id = $1 AND key = $2")
                .bind(p.id)
                .bind(&r.key)
                .fetch_one(&state.db)
                .await?;
        match shorten(state, p, &r, &source, &glossary).await {
            Ok(value) => out.push(Proposal {
                fix,
                key_id,
                key: r.key.clone(),
                source,
                locale: r.locale.clone(),
                previous: current,
                value,
                evidence,
            }),
            Err(e) => {
                sqlx::query(
                    "UPDATE layout_rum_fixes SET status = 'failed', error = $2 WHERE id = $1",
                )
                .bind(fix)
                .bind(&e)
                .execute(&state.db)
                .await?;
            }
        }
    }
    if out.is_empty() {
        return Ok(0);
    }
    for f in &out {
        sqlx::query("UPDATE layout_rum_fixes SET proposal = $2 WHERE id = $1")
            .bind(f.fix)
            .bind(&f.value)
            .execute(&state.db)
            .await?;
    }
    let pr = match app::load_link(state, p.id).await? {
        Some(link) => Some(open_pr(state, p, &link.repo, link.installation_id, &out).await),
        None => None,
    };
    if let Some(Ok(url)) = &pr {
        sqlx::query("UPDATE layout_rum_fixes SET status = 'pr', pr_url = $2 WHERE id = ANY($1)")
            .bind(out.iter().map(|f| f.fix).collect::<Vec<_>>())
            .bind(url)
            .execute(&state.db)
            .await?;
        return Ok(out.len());
    }
    let error = pr.and_then(|r| r.err());
    for f in &out {
        // The feedback inbox already has accept / reject / PR for a proposed correction.
        sqlx::query(
            "INSERT INTO feedback_groups (id, key_id, locale, status, proposal, previous, explanation)
             VALUES ($1, $2, $3, 'proposed', $4, $5, $6)
             ON CONFLICT (key_id, locale) WHERE status IN ('open', 'proposed') DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(f.key_id)
        .bind(&f.locale)
        .bind(&f.value)
        .bind(&f.previous)
        .bind(format!("Layout RUM: {}", f.evidence))
        .execute(&state.db)
        .await?;
        sqlx::query("UPDATE layout_rum_fixes SET status = 'suggested', error = $2 WHERE id = $1")
            .bind(f.fix)
            .bind(&error)
            .execute(&state.db)
            .await?;
    }
    Ok(out.len())
}

/// The `glot fit` shorten request, 3 attempts; a variant must pass the validator and the measured pixel budget.
async fn shorten(
    state: &AppState,
    p: &ProjectRow,
    r: &Row,
    source: &str,
    glossary: &glot_core::glossary::Glossary,
) -> Result<String, String> {
    let provider = crate::feedback::provider(state, p).map_err(|e| e.message)?;
    let current = r.current.clone().unwrap_or_default();
    let font = font_of(&r.font);
    let mut req = Shorten {
        locale: r.locale.clone(),
        key: r.key.clone(),
        source: source.to_string(),
        current: current.clone(),
        width_px: r.box_px as u32,
        need_width_px: r.need_px as u32,
        height_px: 0,
        need_height_px: 0,
        font: r.font.clone(),
        glossary: glossary.matches(source).into_iter().cloned().collect(),
        rejected: vec![],
        images: vec![],
    };
    for _ in 0..3 {
        let c = provider
            .complete(req.request(&p.llm.0.model))
            .await
            .map_err(|e| e.to_string())?;
        let why = match parse_shortened(&c.text) {
            None => "unparseable answer".to_string(),
            Some(v) if v == current => "same text as before".to_string(),
            Some(v) => {
                let issues = crate::keys::check(source, &v, &r.locale, glossary);
                if has_errors(&issues) {
                    issues
                        .iter()
                        .map(|i| i.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                } else {
                    match fits(&v, &current, &font, r.box_px, r.need_px) {
                        Ok(()) => return Ok(v),
                        Err(why) => why,
                    }
                }
            }
        };
        let v = parse_shortened(&c.text).unwrap_or_default();
        req.rejected.push((v, why));
    }
    Err(format!(
        "no variant fit after 3 attempts: {}",
        req.rejected
            .last()
            .map(|r| r.1.as_str())
            .unwrap_or_default()
    ))
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// One commit (catalogs + glot.lock when present) on `glot/layout-…` through the Git Data API, then the PR.
async fn open_pr(
    state: &AppState,
    p: &ProjectRow,
    repo: &str,
    installation: i64,
    out: &[Proposal],
) -> Result<String, String> {
    let gh_app = app::require_app(state).await.map_err(|e| e.message)?;
    let s = |e: anyhow::Error| e.to_string();
    let name = repo.split('/').nth(1).unwrap_or(repo);
    let jwt = app::app_jwt(&gh_app).map_err(s)?;
    let api = app::api_base();
    let tok = app::gh(
        &jwt,
        reqwest::Method::POST,
        &format!("{api}/app/installations/{installation}/access_tokens"),
        Some(json!({ "repositories": [name], "permissions": { "contents": "write", "pull_requests": "write", "metadata": "read" } })),
    )
    .await
    .map_err(s)?;
    let token = tok["token"]
        .as_str()
        .ok_or("GitHub returned no installation token")?;
    let get = |url: String| async move { app::gh(token, reqwest::Method::GET, &url, None).await };
    let post = |url: String, b: Value| async move {
        app::gh(token, reqwest::Method::POST, &url, Some(b)).await
    };
    let api = format!("{api}/repos/{repo}");
    let (base, cat_pattern, lock_path): (String, String, String) = sqlx::query_as(
        "SELECT coalesce(l.branch, 'main'), coalesce(f.catalog_path, 'messages/{locale}.json'), coalesce(f.lock_path, 'glot.lock')
         FROM github_links l LEFT JOIN feedback_settings f ON f.project_id = l.project_id WHERE l.project_id = $1",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let base_sha = get(format!("{api}/git/ref/heads/{base}"))
        .await
        .map_err(s)?["object"]["sha"]
        .as_str()
        .ok_or("no base sha")?
        .to_string();
    let base_tree = get(format!("{api}/git/commits/{base_sha}"))
        .await
        .map_err(s)?["tree"]["sha"]
        .as_str()
        .ok_or("no base tree")?
        .to_string();
    let text = |v: &Value| -> Option<String> {
        let b = base64::engine::general_purpose::STANDARD
            .decode(v["content"].as_str()?.replace('\n', ""))
            .ok()?;
        String::from_utf8(b).ok()
    };
    use glot_core::catalog::{Catalog, Format};
    let mut tree = vec![];
    let mut by_locale: BTreeMap<&str, Vec<&Proposal>> = BTreeMap::new();
    for f in out {
        by_locale.entry(&f.locale).or_default().push(f);
    }
    for (locale, fs) in &by_locale {
        let path = cat_pattern.replace("{locale}", locale);
        let v = get(format!("{api}/contents/{path}?ref={base_sha}"))
            .await
            .map_err(s)?;
        let t = text(&v).ok_or_else(|| format!("{path}: unreadable"))?;
        let mut cat = Catalog::parse(&t, locale).map_err(|e| format!("{path}: {e}"))?;
        for f in fs {
            cat.entries.insert(f.key.clone(), f.value.clone());
        }
        let format = match Catalog::detect_format(&t) {
            Format::Auto => Format::JsonNested,
            f => f,
        };
        tree.push(json!({ "path": path, "mode": "100644", "type": "blob", "content": cat.to_json(format) }));
    }
    // ponytail: a missing (or unreadable) lock is left alone; `glot check` then sees the edit as human.
    if let Some(mut lock) = get(format!("{api}/contents/{lock_path}?ref={base_sha}"))
        .await
        .ok()
        .and_then(|v| text(&v))
        .and_then(|t| serde_json::from_str::<Lockfile>(&t).ok())
    {
        for f in out {
            let origin = Origin::Machine {
                model: p.llm.0.model.clone(),
            };
            lock.record(&f.locale, &f.key, &f.source, &f.value, origin, None);
        }
        let mut l = serde_json::to_string_pretty(&lock).map_err(|e| e.to_string())?;
        l.push('\n');
        tree.push(json!({ "path": lock_path, "mode": "100644", "type": "blob", "content": l }));
    }
    let t = post(
        format!("{api}/git/trees"),
        json!({ "base_tree": base_tree, "tree": tree }),
    )
    .await
    .map_err(s)?;
    let title = format!(
        "glot: shorten {} string(s) that overflow for real visitors",
        out.len()
    );
    let commit = post(
        format!("{api}/git/commits"),
        json!({ "message": title, "tree": t["sha"], "parents": [base_sha] }),
    )
    .await
    .map_err(s)?;
    let branch = format!("glot/layout-{}", &Uuid::new_v4().simple().to_string()[..8]);
    post(
        format!("{api}/git/refs"),
        json!({ "ref": format!("refs/heads/{branch}"), "sha": commit["sha"] }),
    )
    .await
    .map_err(s)?;
    let rows: String = out
        .iter()
        .map(|f| {
            format!(
                "| `{}` | {} | {} | {} | {} |\n",
                cell(&f.key),
                f.locale,
                cell(&f.previous),
                cell(&f.value),
                cell(&f.evidence)
            )
        })
        .collect();
    let body = format!(
        "These translations overflow their box for real visitors (glot Layout RUM, sampled, anonymous). \
         Shorter variants passed glot's validator (placeholders, tags, glossary) and the pixel budget measured \
         from the visitors' boxes.\n\n| Key | Locale | Before | After | Evidence |\n|---|---|---|---|---|\n{rows}"
    );
    let pr = post(
        format!("{api}/pulls"),
        json!({ "title": title, "head": branch, "base": base, "body": body }),
    )
    .await
    .map_err(s)?;
    pr["html_url"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "GitHub returned no PR url".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_and_evidence() {
        assert_eq!(band(374), (360, Some(390)));
        assert_eq!(band(10), (0, Some(360)));
        assert_eq!(band(2560), (1920, None));
        let r = Row {
            key: "invoice.remind".into(),
            locale: "de".into(),
            vw_min: 360,
            vw_max: Some(390),
            views: 412,
            overflows: 130,
            fallback: 2,
            rate: 0.31,
            box_px: 120,
            need_px: 180,
            font: "Inter 14".into(),
            current: Some("Zahlungserinnerung senden".into()),
        };
        assert_eq!(
            evidence(&r, 7),
            "'Zahlungserinnerung senden' is truncated for 31 % of mobile DE visitors (360–390 px, 412 views in 7 days)"
        );
    }

    #[test]
    fn pixel_budget_is_relative_to_the_measured_text() {
        let f = font_of("NoSuchFont 14");
        assert_eq!(f.size_px, 14.0);
        // Needed 180px for a 120px box: "Mahnung senden" fits, the same length does not.
        assert!(fits("Mahnung senden", "Zahlungserinnerung senden", &f, 120, 180).is_ok());
        assert!(
            fits(
                "Zahlungserinnerung senden!",
                "Zahlungserinnerung senden",
                &f,
                120,
                180
            )
            .is_err()
        );
    }
}
