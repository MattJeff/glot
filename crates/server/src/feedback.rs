//! Native-speaker feedback loop (see ARCHITECTURE.md « Feedback »): `@glot/feedback` reports a rendered text →
//! matched to its key (reverse catalog index) → grouped by (key, locale) → at the threshold (or on demand) the
//! guarded translation pipeline proposes a correction (validator + glossary + retries), back-translated by the
//! Review API → a human accepts (written as human/reviewed) or rejects → optional GitHub PR.
//! Privacy: no IP, user agent, cookie or visitor id; url without query/fragment; dom = tag path only.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{Method, StatusCode, header},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use glot_core::{
    config::{Config, LlmConfig},
    context::KeyContext,
    lock::{Lockfile, Origin},
    validate::{Issue, has_errors},
};
use glot_llm::{
    review::ReviewItem,
    translate::{Item, Request, Translator},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::types::Json as DbJson;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    keys::{KeyRecord, Write, find_key, write_translation},
    projects::{self, ProjectRow, Role},
};

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/feedback/{key}", post(ingest))
        .layer(DefaultBodyLimit::max(8192))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/feedback";
    Router::new()
        .route(p, get(list))
        .route(&format!("{p}/settings"), put(update_settings))
        .route(&format!("{p}/{{id}}/propose"), post(propose_now))
        .route(&format!("{p}/{{id}}/accept"), post(accept))
        .route(&format!("{p}/{{id}}/reject"), post(reject))
        .route(&format!("{p}/{{id}}/pr"), post(open_pr))
}

// ---------- reverse index: rendered text → key ----------

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Literal pieces of a catalog value: ICU `{…}` blocks split it (they render to anything), tags are dropped.
fn pieces(value: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut tag) = (vec![], String::new(), 0, false);
    for c in value.chars() {
        match c {
            '{' => {
                if depth == 0 {
                    out.push(std::mem::take(&mut cur));
                }
                depth += 1;
            }
            '}' if depth > 0 => depth -= 1,
            _ if depth > 0 => {}
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if tag => {}
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|p| norm(&p)).collect()
}

/// Does the rendered `text` come from `value`? Some(literal chars matched) when the whole text matches the
/// value with its placeholders filled in. ponytail: plural/select branches count as one wildcard.
fn full_match(value: &str, text: &str) -> Option<usize> {
    let ps = pieces(value);
    let text = norm(text);
    if ps.len() == 1 {
        return (ps[0] == text).then_some(text.len());
    }
    let (first, last) = (ps[0].trim_end(), ps[ps.len() - 1].trim_start());
    let mut rest = text.strip_prefix(first)?;
    rest = rest.strip_suffix(last)?;
    for mid in &ps[1..ps.len() - 1] {
        let mid = mid.trim();
        if mid.is_empty() {
            continue;
        }
        rest = &rest[rest.find(mid)? + mid.len()..];
    }
    Some(ps.iter().map(|p| p.trim().len()).sum())
}

/// Best key for a rendered text among (key, value): a full match with the most literal text, else the
/// shortest value containing the selection (a user may select part of a sentence).
pub fn match_key<'a>(rows: &'a [(String, String)], text: &str) -> Option<&'a str> {
    let full = rows
        .iter()
        .filter_map(|(k, v)| Some((full_match(v, text).filter(|n| *n > 0)?, k)))
        .max_by_key(|(n, _)| *n);
    if let Some((_, k)) = full {
        return Some(k);
    }
    let t = norm(text);
    if t.chars().count() < 4 {
        return None;
    }
    rows.iter()
        .filter(|(_, v)| pieces(v).iter().any(|p| p.contains(&t)))
        .min_by_key(|(_, v)| v.len())
        .map(|(k, _)| k.as_str())
}

// ---------- ingestion ----------

#[derive(Deserialize)]
struct Report {
    locale: String,
    text: String,
    #[serde(default)]
    kind: Option<String>,
    suggestion: Option<String>,
    url: Option<String>,
    dom: Option<String>,
    /// Known key (overlay / instrumented apps); otherwise matched from `text`.
    key: Option<String>,
}

static DEFAULT_RATE: std::sync::LazyLock<i32> = std::sync::LazyLock::new(|| {
    std::env::var("GLOT_FEEDBACK_RATE_PER_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(30)
});

/// Drop query and fragment (they may carry tokens or emails).
fn clean_url(u: &str) -> String {
    u.split(['?', '#'])
        .next()
        .unwrap_or("")
        .chars()
        .take(500)
        .collect()
}

fn clip(s: Option<String>, n: usize) -> Option<String> {
    s.map(|s| s.trim().chars().take(n).collect::<String>())
        .filter(|s| !s.is_empty())
}

async fn ingest(
    State(state): State<AppState>,
    AppPath(public_key): AppPath<String>,
    body: Bytes,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    // Key lookup + shared per-minute counter in one statement (same scheme as insights, see ARCHITECTURE.md).
    // ponytail: no local block cache like insights; add it if feedback floods ever show up.
    let (project_id, hits, window_end): (Uuid, i32, i64) = sqlx::query_as(
        "WITH k AS (SELECT project_id FROM insight_keys WHERE public_key = $1)
         INSERT INTO feedback_rate AS r (project_id, window_start, hits)
         SELECT project_id, date_trunc('minute', now()), 1 FROM k
         ON CONFLICT (project_id) DO UPDATE SET
           hits = CASE WHEN excluded.window_start > r.window_start THEN 1 ELSE r.hits + 1 END,
           window_start = greatest(r.window_start, excluded.window_start)
         RETURNING r.project_id, r.hits, extract(epoch FROM r.window_start + interval '1 minute')::int8",
    )
    .bind(&public_key)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("unknown public key"))?;
    if hits > *DEFAULT_RATE {
        let now = Utc::now().timestamp();
        return Ok((
            [(header::RETRY_AFTER, (window_end - now).max(1).to_string())],
            AppError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too much feedback for this key; see Retry-After",
            ),
        )
            .into_response());
    }
    // Any content type: the widget posts text/plain to avoid a CORS preflight.
    let r: Report = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    crate::validate_locale(&r.locale)?;
    let kind = r.kind.as_deref().unwrap_or("wrong");
    if !["wrong", "awkward", "unclear"].contains(&kind) {
        return Err(AppError::bad_request("kind must be wrong|awkward|unclear"));
    }
    let text = norm(&r.text);
    if text.is_empty() || text.chars().count() > 1000 {
        return Err(AppError::bad_request("text must be 1-1000 chars"));
    }
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT k.id, k.key, t.value FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND t.locale = $2 AND t.value <> ''",
    )
    .bind(project_id)
    .bind(&r.locale)
    .fetch_all(&state.db)
    .await?;
    // ponytail: linear scan of the locale's catalog per report (rate-limited); index it if catalogs reach ~100k keys.
    let pairs: Vec<(String, String)> = rows
        .iter()
        .map(|(_, k, v)| (k.clone(), v.clone()))
        .collect();
    let key = match r.key.as_deref() {
        Some(k) if pairs.iter().any(|(x, _)| x == k) => Some(k),
        _ => match_key(&pairs, &text),
    };
    let Some(key_id) = key
        .and_then(|k| rows.iter().find(|(_, x, _)| x == k))
        .map(|r| r.0)
    else {
        // Not in this locale's catalog (dynamic or untranslated content): nothing to correct.
        return Ok((StatusCode::ACCEPTED, Json(json!({ "matched": false }))).into_response());
    };

    let mut tx = state.db.begin().await?;
    let group: Uuid = sqlx::query_scalar(
        "INSERT INTO feedback_groups (id, key_id, locale) VALUES ($1, $2, $3)
         ON CONFLICT (key_id, locale) WHERE status IN ('open', 'proposed') DO UPDATE SET updated_at = now()
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(key_id)
    .bind(&r.locale)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO feedback_reports (group_id, kind, text, suggestion, url, dom) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(group)
        .bind(kind)
        .bind(&text)
        .bind(clip(r.suggestion, 1000))
        .bind(r.url.as_deref().map(clean_url))
        .bind(clip(r.dom, 300))
        .execute(&mut *tx)
        .await?;
    // Threshold reached on a group without proposal → propose in the background.
    let due: bool = sqlx::query_scalar(
        "SELECT g.status = 'open' AND count(r.id) = coalesce((SELECT threshold FROM feedback_settings WHERE project_id = $2), 3)
         FROM feedback_groups g JOIN feedback_reports r ON r.group_id = g.id WHERE g.id = $1 GROUP BY g.id",
    )
    .bind(group)
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if due {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = propose_by_id(&state, project_id, group).await {
                tracing::warn!(group = %group, "feedback proposal failed: {}", e.message);
            }
        });
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "matched": true }))).into_response())
}

// ---------- proposal ----------

pub(crate) fn config(p: &ProjectRow) -> Config {
    // ponytail: same Config as jobs::translate; share a helper if a third caller appears.
    let llm = p.llm.0.clone();
    Config {
        source_locale: p.source_locale.clone(),
        target_locales: p.target_locales.clone(),
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
        i18next: Default::default(),
    }
}

pub(crate) fn provider(
    state: &AppState,
    p: &ProjectRow,
) -> ApiResult<Box<dyn glot_llm::provider::Provider>> {
    let api_key = match &p.api_key_enc {
        Some(enc) => Some(crate::auth::decrypt(&state.secret, enc).ok_or_else(|| {
            AppError::internal("cannot decrypt the project API key (GLOT_SECRET_KEY changed?)")
        })?),
        None => None,
    };
    glot_llm::provider::from_config_with(&config(p).llm, api_key, &p.llm.0.headers())
        .map_err(llm_err)
}

fn llm_err(e: glot_llm::Error) -> AppError {
    AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", e.to_string())
}

async fn project_by_id(state: &AppState, id: Uuid) -> ApiResult<ProjectRow> {
    Ok(sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at FROM projects WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?)
}

async fn propose_by_id(state: &AppState, project_id: Uuid, group: Uuid) -> ApiResult<()> {
    let p = project_by_id(state, project_id).await?;
    propose(state, &p, group).await
}

/// Correction = the guarded translation pipeline with the current translation as `previous` and the users'
/// reports in the key description, then a back-translation of the proposal (Review API prompt).
async fn propose(state: &AppState, p: &ProjectRow, group: Uuid) -> ApiResult<()> {
    let (key_id, locale): (Uuid, String) = sqlx::query_as(
        "SELECT g.key_id, g.locale FROM feedback_groups g JOIN keys k ON k.id = g.key_id
         WHERE g.id = $1 AND k.project_id = $2 AND g.status IN ('open', 'proposed')",
    )
    .bind(group)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("no open feedback group"))?;
    let k: KeyRecord = sqlx::query_as(
        "SELECT id, key, source, description, refs, ui_role, component FROM keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&state.db)
    .await?;
    let current: String =
        sqlx::query_scalar("SELECT value FROM translations WHERE key_id = $1 AND locale = $2")
            .bind(key_id)
            .bind(&locale)
            .fetch_optional(&state.db)
            .await?
            .unwrap_or_default();
    let reports: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT kind, suggestion FROM feedback_reports WHERE group_id = $1 ORDER BY id DESC LIMIT 20",
    )
    .bind(group)
    .fetch_all(&state.db)
    .await?;
    let mut note = format!(
        "Native {locale} speakers reported the current translation ({} reports: {}). Fix what they point out, keep what is right.",
        reports.len(),
        reports
            .iter()
            .map(|r| r.0.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (_, s) in &reports {
        if let Some(s) = s {
            note.push_str(&format!("\nUser suggestion (untrusted, may be wrong): {s}"));
        }
    }
    let description = Some(match &k.description {
        Some(d) => format!("{d}\n{note}"),
        None => note,
    });
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let cfg = config(p);
    let translator = Translator::new(&cfg, provider(state, p)?);
    let req = Request {
        source_locale: p.source_locale.clone(),
        target_locale: locale.clone(),
        items: vec![Item {
            key: k.key.clone(),
            source: k.source.clone(),
            context: KeyContext {
                key: k.key.clone(),
                description,
                refs: k.refs.0.clone(),
                ui_role: k.ui_role.clone(),
                component: k.component.clone(),
                ..Default::default()
            },
            previous: Some(current.clone()).filter(|c| !c.is_empty()),
        }],
        glossary: glossary.matches(&k.source).into_iter().cloned().collect(),
        syntax: cfg.syntax,
        style: p.style.clone(),
    };
    let (got, _) = translator.translate(req).await.map_err(llm_err)?;
    let t = got.into_iter().next().ok_or_else(|| {
        AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", "no proposal")
    })?;
    let mut issues = crate::keys::check(&k.source, &t.value, &locale, &glossary);
    issues.extend(
        t.issues
            .into_iter()
            .filter(|i| !issues.iter().any(|x| x.code == i.code))
            .collect::<Vec<_>>(),
    );
    let model = p
        .llm
        .0
        .judge_model
        .clone()
        .unwrap_or_else(|| p.llm.0.model.clone());
    let (back, _) = glot_llm::review::review(
        provider(state, p)?.as_ref(),
        &model,
        &p.source_locale,
        &locale,
        &glossary,
        &[ReviewItem {
            key: k.key.clone(),
            source: k.source.clone(),
            target: t.value.clone(),
            description: k.description.clone(),
        }],
    )
    .await
    .map_err(llm_err)?;
    let back = back.into_iter().next();
    sqlx::query(
        "UPDATE feedback_groups SET status = 'proposed', proposal = $2, previous = $3, back_translation = $4,
           verdict = $5, explanation = $6, issues = $7, updated_at = now() WHERE id = $1",
    )
    .bind(group)
    .bind(&t.value)
    .bind(&current)
    .bind(back.as_ref().map(|b| &b.back_translation))
    .bind(back.as_ref().and_then(|b| serde_json::to_value(b.verdict).ok()?.as_str().map(String::from)))
    .bind(back.as_ref().map(|b| &b.explanation))
    .bind(DbJson(&issues))
    .execute(&state.db)
    .await?;
    Ok(())
}

// ---------- dashboard ----------

#[derive(Serialize)]
pub struct Settings {
    pub public_key: String,
    pub threshold: i32,
    pub github_repo: Option<String>,
    pub github_base: String,
    pub catalog_path: String,
    pub lock_path: String,
    pub has_github_token: bool,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ReportView {
    pub kind: String,
    pub text: String,
    pub suggestion: Option<String>,
    pub url: Option<String>,
    pub dom: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Group {
    pub id: Uuid,
    pub key: String,
    pub locale: String,
    pub source: String,
    /// Live translation.
    pub current: Option<String>,
    /// open | proposed | accepted | rejected
    pub status: String,
    pub count: i64,
    pub proposal: Option<String>,
    pub previous: Option<String>,
    pub back_translation: Option<String>,
    pub verdict: Option<String>,
    pub explanation: Option<String>,
    pub issues: DbJson<Vec<Issue>>,
    pub pr_url: Option<String>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(skip)]
    pub reports: Vec<ReportView>,
}

#[derive(Serialize)]
pub struct Overview {
    pub settings: Settings,
    pub groups: Vec<Group>,
}

#[derive(sqlx::FromRow)]
struct SettingsRow {
    threshold: i32,
    github_repo: Option<String>,
    github_base: String,
    catalog_path: String,
    lock_path: String,
    github_token_enc: Option<Vec<u8>>,
}

async fn settings_row(state: &AppState, project_id: Uuid) -> ApiResult<SettingsRow> {
    sqlx::query("INSERT INTO feedback_settings (project_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(project_id)
        .execute(&state.db)
        .await?;
    Ok(sqlx::query_as(
        "SELECT threshold, github_repo, github_base, catalog_path, lock_path, github_token_enc FROM feedback_settings WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_one(&state.db)
    .await?)
}

async fn settings(state: &AppState, project_id: Uuid) -> ApiResult<Settings> {
    let s = settings_row(state, project_id).await?;
    Ok(Settings {
        public_key: crate::insights::public_key(state, project_id).await?,
        threshold: s.threshold,
        github_repo: s.github_repo,
        github_base: s.github_base,
        catalog_path: s.catalog_path,
        lock_path: s.lock_path,
        has_github_token: s.github_token_enc.is_some(),
    })
}

const GROUP_SELECT: &str = "SELECT g.id, k.key, g.locale, k.source, t.value AS current, g.status,
    (SELECT count(*) FROM feedback_reports r WHERE r.group_id = g.id) AS count,
    g.proposal, g.previous, g.back_translation, g.verdict, g.explanation, g.issues, g.pr_url, g.updated_at
  FROM feedback_groups g JOIN keys k ON k.id = g.key_id
  LEFT JOIN translations t ON t.key_id = g.key_id AND t.locale = g.locale
  WHERE k.project_id = $1";

async fn groups(state: &AppState, project_id: Uuid, id: Option<Uuid>) -> ApiResult<Vec<Group>> {
    let mut gs: Vec<Group> = sqlx::query_as(&format!(
        "{GROUP_SELECT} AND ($2::uuid IS NULL OR g.id = $2) ORDER BY (g.status IN ('open', 'proposed')) DESC, count DESC, g.updated_at DESC LIMIT 200"
    ))
    .bind(project_id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    for g in &mut gs {
        g.reports = sqlx::query_as(
            "SELECT kind, text, suggestion, url, dom, created_at FROM feedback_reports WHERE group_id = $1 ORDER BY id DESC LIMIT 20",
        )
        .bind(g.id)
        .fetch_all(&state.db)
        .await?;
    }
    Ok(gs)
}

async fn group(state: &AppState, project_id: Uuid, id: Uuid) -> ApiResult<Group> {
    groups(state, project_id, Some(id))
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found("feedback group not found"))
}

async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Overview>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(Overview {
        settings: settings(&state, p.id).await?,
        groups: groups(&state, p.id, None).await?,
    }))
}

#[derive(Deserialize)]
struct SettingsBody {
    threshold: Option<i32>,
    /// "" clears.
    github_repo: Option<String>,
    github_base: Option<String>,
    catalog_path: Option<String>,
    lock_path: Option<String>,
    /// Write-only. "" clears, absent keeps.
    github_token: Option<String>,
}

async fn update_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<SettingsBody>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    settings_row(&state, p.id).await?;
    if b.threshold.is_some_and(|t| !(1..=1000).contains(&t)) {
        return Err(AppError::bad_request("threshold must be 1-1000"));
    }
    let repo = b.github_repo.map(|r| r.trim().to_string());
    if let Some(r) = repo.as_deref().filter(|r| !r.is_empty()) {
        let ok = r.split('/').count() == 2
            && r.split('/').all(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            });
        if !ok {
            return Err(AppError::bad_request("github_repo must be owner/name"));
        }
    }
    for path in [&b.catalog_path, &b.lock_path].into_iter().flatten() {
        if path.is_empty() || path.starts_with('/') || path.contains("..") {
            return Err(AppError::bad_request(
                "paths must be relative to the repository root",
            ));
        }
    }
    if b.catalog_path
        .as_deref()
        .is_some_and(|c| !c.contains("{locale}"))
    {
        return Err(AppError::bad_request("catalog_path must contain {locale}"));
    }
    let token = b.github_token.map(|t| {
        Some(t.trim().to_string())
            .filter(|t| !t.is_empty())
            .map(|t| crate::auth::encrypt(&state.secret, &t))
    });
    sqlx::query(
        "UPDATE feedback_settings SET threshold = coalesce($2, threshold),
           github_repo = CASE WHEN $3::text IS NULL THEN github_repo ELSE nullif($3, '') END,
           github_base = coalesce(nullif($4, ''), github_base), catalog_path = coalesce($5, catalog_path),
           lock_path = coalesce($6, lock_path),
           github_token_enc = CASE WHEN $7 THEN $8 ELSE github_token_enc END
         WHERE project_id = $1",
    )
    .bind(p.id)
    .bind(b.threshold)
    .bind(repo)
    .bind(b.github_base.map(|s| s.trim().to_string()))
    .bind(b.catalog_path)
    .bind(b.lock_path)
    .bind(token.is_some())
    .bind(token.flatten())
    .execute(&state.db)
    .await?;
    Ok(Json(settings(&state, p.id).await?))
}

async fn propose_now(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Group>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    propose(&state, &p, id).await?;
    Ok(Json(group(&state, p.id, id).await?))
}

#[derive(Deserialize, Default)]
struct AcceptBody {
    /// Edited proposal; absent = the AI proposal.
    value: Option<String>,
}

async fn accept(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    body: Bytes,
) -> ApiResult<Json<Group>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let b: AcceptBody = if body.is_empty() {
        AcceptBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?
    };
    let g = group(&state, p.id, id).await?;
    if g.status != "proposed" && g.status != "open" {
        return Err(AppError::conflict(format!("feedback already {}", g.status)));
    }
    let value = b
        .value
        .or(g.proposal.clone())
        .ok_or_else(|| AppError::bad_request("no proposal yet: propose first or send a value"))?;
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, p.id, &g.key).await?;
    let issues = crate::keys::check(&k.source, &value, &g.locale, &glossary);
    if has_errors(&issues) {
        let msg = issues
            .iter()
            .map(|i| i.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_translation",
            msg,
        ));
    }
    write_translation(
        &mut tx,
        &glossary,
        Write {
            key: &k,
            locale: &g.locale,
            value: &value,
            origin: "human",
            status: "reviewed",
            model: None,
            score: None,
            extra_issues: vec![],
            author: Some(&user.name),
        },
    )
    .await?;
    sqlx::query("UPDATE feedback_groups SET status = 'accepted', proposal = $2, previous = coalesce(previous, $3), updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(&value)
        .bind(&g.current)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(group(&state, p.id, id).await?))
}

async fn reject(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Group>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let g = group(&state, p.id, id).await?;
    if g.status != "proposed" && g.status != "open" {
        return Err(AppError::conflict(format!("feedback already {}", g.status)));
    }
    sqlx::query("UPDATE feedback_groups SET status = 'rejected', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(Json(group(&state, p.id, id).await?))
}

// ---------- GitHub PR ----------

/// `GLOT_GITHUB_API` (tests, GitHub Enterprise), else api.github.com.
fn github_api() -> String {
    std::env::var("GLOT_GITHUB_API")
        .unwrap_or_else(|_| "https://api.github.com".into())
        .trim_end_matches('/')
        .to_string()
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Same JSON as `Lockfile::save` (pretty, trailing newline).
fn lock_json(l: &Lockfile) -> String {
    let mut s = serde_json::to_string_pretty(l).expect("lockfile serializes");
    s.push('\n');
    s
}

/// One commit on a new branch (catalog + glot.lock) through the Git Data API, then a PR with a before/after table.
async fn open_pr(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Group>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let g = group(&state, p.id, id).await?;
    if g.status != "accepted" {
        return Err(AppError::conflict(
            "accept the correction before opening a PR",
        ));
    }
    let s = settings_row(&state, p.id).await?;
    let (Some(repo), Some(enc)) = (s.github_repo, s.github_token_enc) else {
        return Err(AppError::bad_request(
            "set the GitHub repository and token in the feedback settings",
        ));
    };
    let token = crate::auth::decrypt(&state.secret, &enc).ok_or_else(|| {
        AppError::internal("cannot decrypt the GitHub token (GLOT_SECRET_KEY changed?)")
    })?;
    let value = g.current.clone().unwrap_or_default();
    let api = format!("{}/repos/{repo}", github_api());
    let http = reqwest::Client::new();
    let gh_err = |e: String| AppError::new(StatusCode::BAD_GATEWAY, "github_error", e);
    let call = async |method: reqwest::Method,
                      url: String,
                      body: Option<Value>,
                      raw: bool|
           -> ApiResult<Option<String>> {
        let mut req = http
            .request(method, &url)
            .bearer_auth(&token)
            .header(header::USER_AGENT, "glot")
            .header(
                header::ACCEPT,
                if raw {
                    "application/vnd.github.raw+json"
                } else {
                    "application/vnd.github+json"
                },
            )
            .header("x-github-api-version", "2022-11-28");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(|e| gh_err(e.to_string()))?;
        if res.status() == StatusCode::NOT_FOUND && raw {
            return Ok(None);
        }
        let status = res.status();
        let text = res.text().await.map_err(|e| gh_err(e.to_string()))?;
        if !status.is_success() {
            return Err(gh_err(format!(
                "{status} on {url}: {}",
                text.chars().take(300).collect::<String>()
            )));
        }
        Ok(Some(text))
    };
    let json_of = |s: Option<String>| -> ApiResult<Value> {
        serde_json::from_str(&s.unwrap_or_default()).map_err(|e| gh_err(e.to_string()))
    };
    let field = |v: &Value, path: &[&str]| -> ApiResult<String> {
        path.iter()
            .fold(v, |v, k| &v[*k])
            .as_str()
            .map(String::from)
            .ok_or_else(|| {
                gh_err(format!(
                    "unexpected GitHub response (missing {})",
                    path.join(".")
                ))
            })
    };

    let base = s.github_base;
    let r = json_of(
        call(
            reqwest::Method::GET,
            format!("{api}/git/ref/heads/{base}"),
            None,
            false,
        )
        .await?,
    )?;
    let base_sha = field(&r, &["object", "sha"])?;
    let c = json_of(
        call(
            reqwest::Method::GET,
            format!("{api}/git/commits/{base_sha}"),
            None,
            false,
        )
        .await?,
    )?;
    let base_tree = field(&c, &["tree", "sha"])?;

    let cat_path = s.catalog_path.replace("{locale}", &g.locale);
    let cat_text = call(
        reqwest::Method::GET,
        format!("{api}/contents/{cat_path}?ref={base_sha}"),
        None,
        true,
    )
    .await?
    .unwrap_or_else(|| "{}".into());
    let mut cat = glot_core::catalog::Catalog::parse(&cat_text, &g.locale)
        .map_err(|e| AppError::bad_request(format!("{cat_path}: {e}")))?;
    cat.entries.insert(g.key.clone(), value.clone());
    let format = match glot_core::catalog::Catalog::detect_format(&cat_text) {
        glot_core::catalog::Format::Auto => glot_core::catalog::Format::JsonNested,
        f => f,
    };
    let lock_text = call(
        reqwest::Method::GET,
        format!("{api}/contents/{}?ref={base_sha}", s.lock_path),
        None,
        true,
    )
    .await?;
    let mut lock: Lockfile = match &lock_text {
        Some(t) => serde_json::from_str(t)
            .map_err(|e| AppError::bad_request(format!("{}: {e}", s.lock_path)))?,
        None => Lockfile::default(),
    };
    lock.record(&g.locale, &g.key, &g.source, &value, Origin::Human, None);

    let t = json_of(
        call(
            reqwest::Method::POST,
            format!("{api}/git/trees"),
            Some(json!({ "base_tree": base_tree, "tree": [
                { "path": cat_path, "mode": "100644", "type": "blob", "content": cat.to_json(format) },
                { "path": s.lock_path, "mode": "100644", "type": "blob", "content": lock_json(&lock) },
            ]})),
            false,
        )
        .await?,
    )?;
    let title = format!(
        "glot: fix {} translation of `{}` (native-speaker feedback)",
        g.locale, g.key
    );
    let commit = json_of(
        call(
            reqwest::Method::POST,
            format!("{api}/git/commits"),
            Some(json!({ "message": title, "tree": field(&t, &["sha"])?, "parents": [base_sha] })),
            false,
        )
        .await?,
    )?;
    let branch = format!(
        "glot/feedback-{}-{}",
        g.locale,
        &id.simple().to_string()[..8]
    );
    call(
        reqwest::Method::POST,
        format!("{api}/git/refs"),
        Some(json!({ "ref": format!("refs/heads/{branch}"), "sha": field(&commit, &["sha"])? })),
        false,
    )
    .await?;
    let before = g.previous.clone().unwrap_or_default();
    let body = format!(
        "Correction from {} native-speaker report(s), reviewed in glot by {}.\n\n| Key | Locale | Before | After |\n|---|---|---|---|\n| `{}` | {} | {} | {} |\n\nSource ({}): {}\n{}",
        g.count,
        user.name,
        cell(&g.key),
        g.locale,
        cell(&before),
        cell(&value),
        p.source_locale,
        cell(&g.source),
        g.back_translation
            .as_deref()
            .map(|b| format!("Back-translation of the new text: {}\n", cell(b)))
            .unwrap_or_default(),
    );
    let pr = json_of(
        call(
            reqwest::Method::POST,
            format!("{api}/pulls"),
            Some(json!({ "title": title, "head": branch, "base": base, "body": body })),
            false,
        )
        .await?,
    )?;
    sqlx::query("UPDATE feedback_groups SET pr_url = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(field(&pr, &["html_url"])?)
        .execute(&state.db)
        .await?;
    Ok(Json(group(&state, p.id, id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_index() {
        let rows: Vec<(String, String)> = [
            ("home.title", "Bon retour"),
            ("home.greet", "Bonjour {name} !"),
            (
                "cart.items",
                "{count, plural, one {# article} other {# articles}} dans le panier",
            ),
            (
                "legal",
                "En continuant, vous acceptez les <b>conditions</b> d'utilisation.",
            ),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(match_key(&rows, "Bon retour"), Some("home.title"));
        assert_eq!(match_key(&rows, "  Bonjour   Ana ! "), Some("home.greet"));
        assert_eq!(
            match_key(&rows, "3 articles dans le panier"),
            Some("cart.items")
        );
        assert_eq!(
            match_key(&rows, "vous acceptez les conditions"),
            Some("legal")
        );
        assert_eq!(match_key(&rows, "Au revoir"), None);
        assert_eq!(match_key(&rows, "Bon"), None); // too short for a partial match
    }

    #[test]
    fn url_privacy() {
        assert_eq!(clean_url("https://a.io/p?email=x@y.z#t"), "https://a.io/p");
    }
}
