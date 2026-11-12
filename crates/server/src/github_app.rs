//! « Connect GitHub » (see ARCHITECTURE.md « GitHub App »): one GitHub App per glot deployment, created through the
//! manifest flow by an admin (`GLOT_ADMIN_EMAILS`), installed by users on their repos. A project is linked to one
//! repo/branch; a `github_localize` job clones it (installation token, temp dir, size/time/memory limits), runs
//! `glot localize --yes --no-install` (never the repo's own code: no package manager, no scripts), syncs the catalogs
//! into the project (same logic as `sync push`), pushes `glot/localize` and opens a PR. Pushes on the linked branch
//! (webhook) queue a `github_update` job that refreshes the single open glot PR (or opens `glot/update`).
//! A refresh starts from the PR branch's previous output (`glot localize --carry-from`): unchanged strings cost
//! nothing and produce no diff, an identical tree is not pushed. The branch is rebuilt on the latest base (so it
//! never conflicts with it) and commits people pushed to it are replayed on top; one that no longer applies stops
//! the run before any push (`pr_conflict`). Failures are classified ([`diagnose`]) for the dashboard.
//! Secrets (private key, webhook secret, client secret) are encrypted with GLOT_SECRET_KEY; installation tokens live
//! in memory for one job and only reach git through an env-provided header (never argv, never logs).
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
};
use base64::Engine;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppQuery, AppState,
    auth::{AuthUser, User},
    jobs::{self, Job, JobRecord, Progress},
    projects::{self, ProjectRow, Role},
};

/// Job steps, in order (`progress.done` = steps finished, `progress.total` = STEPS).
/// 1 token · 2 clone · 3 glot localize · 4 sync · 5 commit & push · 6 pull request.
const STEPS: i32 = 6;

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/github";
    Router::new()
        .route("/api/github/app", get(app_status))
        .route("/api/github/app/manifest", post(manifest))
        .route(p, get(status))
        .route(&format!("{p}/connect"), post(connect))
        .route(&format!("{p}/repos"), get(repos))
        .route(&format!("{p}/link"), put(link).delete(unlink))
        .route(&format!("{p}/run"), post(run_now))
}

/// Browser redirects from github.com and the webhook (authenticated by state / signature, not by session).
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/github/app/callback", get(app_callback))
        .route("/api/github/callback", get(install_callback))
        .route(
            "/api/github/webhook",
            post(webhook).layer(DefaultBodyLimit::max(25 << 20)),
        )
}

// ---------- configuration ----------

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.into())
        .trim_end_matches('/')
        .to_string()
}
/// REST API (`GLOT_GITHUB_API`, shared with feedback PRs / GitHub Enterprise / tests).
pub(crate) fn api_base() -> String {
    env_or("GLOT_GITHUB_API", "https://api.github.com")
}
/// github.com pages: manifest form, app install, OAuth.
fn web_base() -> String {
    env_or("GLOT_GITHUB_WEB", "https://github.com")
}
/// Clone/push base: `{base}/{owner}/{repo}.git` (tests: `file:///…`).
fn git_base() -> String {
    env_or("GLOT_GITHUB_GIT", &web_base())
}
pub(crate) fn web_origin() -> String {
    env_or("GLOT_WEB_ORIGIN", "http://localhost:3100")
}
/// Where GitHub reaches the API (webhook); the web origin proxies /api too.
fn public_url() -> String {
    env_or("GLOT_PUBLIC_URL", &web_origin())
}
fn num_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn is_admin(user: &User) -> bool {
    crate::auth::is_admin_email(&user.email)
}

fn gh_error(msg: impl Into<String>) -> AppError {
    AppError::new(StatusCode::BAD_GATEWAY, "github_error", msg)
}

// ---------- app record ----------

pub(crate) struct App {
    app_id: i64,
    slug: String,
    client_id: String,
    client_secret: String,
    private_key: String,
    webhook_secret: String,
    html_url: String,
}

async fn load_app(state: &AppState) -> ApiResult<Option<App>> {
    let row: Option<(i64, String, String, Vec<u8>, Vec<u8>, Vec<u8>, String)> = sqlx::query_as(
        "SELECT app_id, slug, client_id, client_secret_enc, private_key_enc, webhook_secret_enc, html_url FROM github_app",
    )
    .fetch_optional(&state.db)
    .await?;
    let Some((app_id, slug, client_id, cs, pk, ws, html_url)) = row else {
        return Ok(None);
    };
    let dec = |b: &[u8]| {
        crate::auth::decrypt(&state.secret, b).ok_or_else(|| {
            AppError::internal("cannot decrypt the GitHub App secrets (GLOT_SECRET_KEY changed?)")
        })
    };
    Ok(Some(App {
        app_id,
        slug,
        client_id,
        client_secret: dec(&cs)?,
        private_key: dec(&pk)?,
        webhook_secret: dec(&ws)?,
        html_url,
    }))
}

pub(crate) async fn require_app(state: &AppState) -> ApiResult<App> {
    load_app(state).await?.ok_or_else(|| {
        AppError::new(
            StatusCode::CONFLICT,
            "github_app_missing",
            "the GitHub App is not set up on this glot server yet (admin: /admin/github-app)",
        )
    })
}

// ---------- GitHub auth: app JWT, installation tokens ----------

/// RS256 JWT for the app (iat 60 s in the past for clock drift, exp 9 min, iss = client id).
pub(crate) fn app_jwt(app: &App) -> anyhow::Result<String> {
    use ring::{rand::SystemRandom, signature};
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let der = base64::engine::general_purpose::STANDARD.decode(
        app.private_key
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect::<String>(),
    )?;
    let key = if app.private_key.contains("BEGIN RSA PRIVATE KEY") {
        signature::RsaKeyPair::from_der(&der)
    } else {
        signature::RsaKeyPair::from_pkcs8(&der)
    }
    .map_err(|e| anyhow::anyhow!("invalid GitHub App private key: {e}"))?;
    let now = Utc::now().timestamp();
    let msg = format!(
        "{}.{}",
        b64.encode(r#"{"alg":"RS256","typ":"JWT"}"#),
        b64.encode(json!({ "iat": now - 60, "exp": now + 540, "iss": app.client_id }).to_string())
    );
    let mut sig = vec![0; key.public().modulus_len()];
    key.sign(
        &signature::RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        msg.as_bytes(),
        &mut sig,
    )
    .map_err(|_| anyhow::anyhow!("signing the GitHub App JWT failed"))?;
    Ok(format!("{msg}.{}", b64.encode(sig)))
}

/// One-hour installation token, narrowed to `repo` (name without owner) when given. Never logged.
async fn installation_token(
    app: &App,
    installation: i64,
    repo: Option<&str>,
) -> anyhow::Result<String> {
    let mut body = json!({ "permissions": { "contents": "write", "pull_requests": "write", "metadata": "read" } });
    if let Some(r) = repo {
        body["repositories"] = json!([r]);
    }
    let v = gh(
        &app_jwt(app)?,
        reqwest::Method::POST,
        &format!(
            "{}/app/installations/{installation}/access_tokens",
            api_base()
        ),
        Some(body),
    )
    .await?;
    v["token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("GitHub returned no installation token"))
}

/// GitHub REST call; non-2xx = error with the status and the start of the body (bodies never echo our token).
pub(crate) async fn gh(
    token: &str,
    method: reqwest::Method,
    url: &str,
    body: Option<Value>,
) -> anyhow::Result<Value> {
    let mut req = reqwest::Client::new()
        .request(method, url)
        .bearer_auth(token)
        .header("user-agent", "glot")
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .timeout(Duration::from_secs(30));
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("GitHub: {e}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        let path = url.split_once("://").map_or(url, |(_, r)| r);
        anyhow::bail!(
            "GitHub {status} on {path}: {}",
            text.chars().take(300).collect::<String>()
        );
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

// ---------- states (CSRF) ----------

async fn new_state(state: &AppState, user: Uuid, project: Option<Uuid>) -> ApiResult<String> {
    let s = hex::encode(rand::random::<[u8; 24]>());
    sqlx::query("DELETE FROM github_states WHERE created_at < now() - interval '1 hour'")
        .execute(&state.db)
        .await?;
    sqlx::query("INSERT INTO github_states (state, user_id, project_id) VALUES ($1, $2, $3)")
        .bind(&s)
        .bind(user)
        .bind(project)
        .execute(&state.db)
        .await?;
    Ok(s)
}

/// Single use, 1 h: (user, project).
async fn take_state(state: &AppState, s: &str) -> ApiResult<(Uuid, Option<Uuid>)> {
    sqlx::query_as(
        "DELETE FROM github_states WHERE state = $1 AND created_at > now() - interval '1 hour' RETURNING user_id, project_id",
    )
    .bind(s)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::bad_request("unknown or expired state: start again from glot"))
}

// ---------- admin: create the app (manifest flow) ----------

#[derive(Serialize)]
struct AppStatus {
    configured: bool,
    can_admin: bool,
    slug: Option<String>,
    app_id: Option<i64>,
    html_url: Option<String>,
    webhook_url: String,
}

async fn app_status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> ApiResult<Json<AppStatus>> {
    let app = load_app(&state).await?;
    Ok(Json(AppStatus {
        configured: app.is_some(),
        can_admin: is_admin(&user),
        slug: app.as_ref().map(|a| a.slug.clone()),
        app_id: app.as_ref().map(|a| a.app_id),
        html_url: app.map(|a| a.html_url),
        webhook_url: format!("{}/api/github/webhook", public_url()),
    }))
}

#[derive(Deserialize)]
struct ManifestBody {
    #[serde(default)]
    name: Option<String>,
    /// Organization that will own the app (else the admin's personal account).
    #[serde(default)]
    org: Option<String>,
}

/// The page auto-submits `<form method=post action={action}><input name=manifest value={manifest}>`.
async fn manifest(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<ManifestBody>,
) -> ApiResult<Json<Value>> {
    if !is_admin(&user) {
        return Err(AppError::forbidden());
    }
    let name = b
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "glot".into());
    if name.len() > 34 {
        return Err(AppError::bad_request(
            "GitHub App names are at most 34 characters",
        ));
    }
    let s = new_state(&state, user.id, None).await?;
    let web = web_origin();
    let manifest = json!({
        "name": name.trim(),
        "url": web,
        "description": "Localize your app: glot opens pull requests with translated catalogs.",
        "hook_attributes": { "url": format!("{}/api/github/webhook", public_url()), "active": true },
        "redirect_url": format!("{web}/api/github/app/callback"),
        "callback_urls": [format!("{web}/api/github/callback")],
        "request_oauth_on_install": true,
        "setup_on_update": true,
        "public": true,
        "default_permissions": { "contents": "write", "pull_requests": "write", "issues": "write", "metadata": "read" },
        "default_events": ["push", "pull_request"],
    });
    let action = match b.org.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        Some(org) if org.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') => {
            format!(
                "{}/organizations/{org}/settings/apps/new?state={s}",
                web_base()
            )
        }
        Some(_) => return Err(AppError::bad_request("invalid organization name")),
        None => format!("{}/settings/apps/new?state={s}", web_base()),
    };
    Ok(Json(
        json!({ "action": action, "manifest": manifest.to_string() }),
    ))
}

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// GitHub → `redirect_url?code&state`: exchange the code (1 h, single use) for the app credentials.
async fn app_callback(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<CallbackQuery>,
) -> ApiResult<Redirect> {
    let page = format!("{}/admin/github-app", web_origin());
    let (Some(code), Some(s)) = (q.code, q.state) else {
        return Ok(Redirect::to(&format!("{page}?error=missing_code")));
    };
    let (user_id, _) = take_state(&state, &s).await?;
    let user: User = sqlx::query_as("SELECT id, email, name FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    if !is_admin(&user) {
        return Err(AppError::forbidden());
    }
    if !code.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::bad_request("invalid code"));
    }
    let res = reqwest::Client::new()
        .post(format!("{}/app-manifests/{code}/conversions", api_base()))
        .header("user-agent", "glot")
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|e| gh_error(e.to_string()))?;
    if !res.status().is_success() {
        tracing::warn!(status = %res.status(), "GitHub App manifest conversion failed");
        return Ok(Redirect::to(&format!("{page}?error=conversion_failed")));
    }
    let v: Value = res.json().await.map_err(|e| gh_error(e.to_string()))?;
    let str_of = |k: &str| {
        v[k].as_str()
            .map(String::from)
            .ok_or_else(|| gh_error(format!("missing `{k}`")))
    };
    let enc = |k: &str| str_of(k).map(|x| crate::auth::encrypt(&state.secret, &x));
    sqlx::query(
        "INSERT INTO github_app (id, app_id, slug, client_id, client_secret_enc, private_key_enc, webhook_secret_enc, html_url, created_by)
         VALUES (1, $1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET app_id = $1, slug = $2, client_id = $3, client_secret_enc = $4, private_key_enc = $5,
           webhook_secret_enc = $6, html_url = $7, created_by = $8, created_at = now()",
    )
    .bind(v["id"].as_i64().ok_or_else(|| gh_error("missing `id`"))?)
    .bind(str_of("slug")?)
    .bind(str_of("client_id")?)
    .bind(enc("client_secret")?)
    .bind(enc("pem")?)
    .bind(enc("webhook_secret")?)
    .bind(str_of("html_url")?)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    tracing::info!(app = %str_of("slug")?, "GitHub App created");
    Ok(Redirect::to(&format!("{page}?created=1")))
}

// ---------- user: install / authorize, pick a repo, link ----------

#[derive(Serialize, sqlx::FromRow)]
struct Installation {
    id: i64,
    account: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct LinkRow {
    pub(crate) installation_id: i64,
    pub(crate) repo: String,
    pub(crate) branch: String,
    pub(crate) pr_number: Option<i32>,
    pub(crate) pr_url: Option<String>,
    pr_branch: Option<String>,
    last_sha: Option<String>,
    updated_at: DateTime<Utc>,
}

const LINK_COLS: &str =
    "installation_id, repo, branch, pr_number, pr_url, pr_branch, last_sha, updated_at";

pub(crate) async fn load_link(state: &AppState, project: Uuid) -> ApiResult<Option<LinkRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {LINK_COLS} FROM github_links WHERE project_id = $1"
    ))
    .bind(project)
    .fetch_optional(&state.db)
    .await?)
}

async fn user_installations(state: &AppState, user: Uuid) -> ApiResult<Vec<Installation>> {
    Ok(sqlx::query_as(
        "SELECT installation_id AS id, account FROM github_user_installations WHERE user_id = $1 ORDER BY account",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await?)
}

async fn latest_job(state: &AppState, project: Uuid) -> ApiResult<Option<Job>> {
    let row: Option<(Uuid, String, String, i32, i32, f64, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)> =
        sqlx::query_as(
            "SELECT id, kind, status, progress_done, progress_total, cost_usd, error, created_at, finished_at
             FROM jobs WHERE project_id = $1 AND kind LIKE 'github\\_%' ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project)
        .fetch_optional(&state.db)
        .await?;
    Ok(row.map(
        |(id, kind, status, done, total, cost_usd, error, created_at, finished_at)| Job {
            id,
            kind,
            status,
            progress: Progress { done, total },
            cost_usd,
            error,
            created_at,
            finished_at,
        },
    ))
}

#[derive(Serialize)]
struct Status {
    /// None = no GitHub App on this server (admin must create it).
    app: Option<Value>,
    installations: Vec<Installation>,
    link: Option<LinkRow>,
    job: Option<Job>,
    /// Class of the last failure ([`diagnose`]): the dashboard shows a cause and a fix per class.
    failure: Option<&'static str>,
}

/// Why a run failed, from its error text: one class per thing the user can act on. The dashboard
/// (`github.failure.<class>` messages) explains the cause and links to the fix; the raw text stays
/// available, folded.
pub(crate) fn diagnose(error: &str) -> &'static str {
    let e = error.to_lowercase();
    let has = |xs: &[&str]| xs.iter().any(|x| e.contains(x));
    if has(&["human commit", "pr_conflict"]) {
        "pr_conflict"
    } else if has(&["source_mismatch"]) {
        "source_mismatch"
    } else if has(&["mock_output"]) {
        "mock_output"
    } else if has(&[
        "add your llm api key",
        "cannot decrypt the project api key",
        "missing_key",
        "api key is not set",
        "no api key",
    ]) {
        "ai_key_missing"
    } else if has(&["workspace"])
        && has(&[
            "sk-ant",
            "anthropic",
            "wrkspc",
            "workspace id",
            "workspace_id",
        ])
    {
        "ai_workspace"
    } else if has(&[
        "http 401",
        "http 403",
        "invalid x-api-key",
        "invalid_api_key",
        "invalid api key",
        "incorrect api key",
        "authentication_error",
        "permission_error",
    ]) {
        "ai_key_invalid"
    } else if has(&[
        "http 429",
        "rate_limit",
        "rate limit",
        "credit balance",
        "insufficient_quota",
        "billing",
        "budget of",
        "over budget",
        "budget exceeded",
    ]) {
        "ai_quota"
    } else if has(&["http 5", "overloaded"]) {
        "ai_unavailable"
    } else if has(&["repository larger than"]) {
        "repo_too_large"
    } else if has(&["timed out after"]) {
        "timeout"
    } else if has(&[
        "remote branch",
        "couldn't find remote ref",
        "invalid repository or branch",
    ]) {
        "branch_missing"
    } else if has(&["no longer linked"]) {
        "unlinked"
    } else if has(&[
        "glot.toml",
        "points outside the repository",
        "toml parse",
        "invalid type",
        "unknown variant",
    ]) {
        "config_invalid"
    } else if has(&[
        "cannot detect the framework",
        "own i18n runtime",
        "no .rs files",
        "no source files",
        "nothing to localize",
        "not supported",
        "unsupported",
    ]) {
        "no_framework"
    } else if has(&[
        "git push failed",
        "protected branch",
        "github 422",
        "resource not accessible",
    ]) {
        "push_rejected"
    } else if has(&[
        "git clone failed",
        "git fetch failed",
        "repository not found",
        "github 404",
        "github 403",
        "github 401",
        "could not read from remote",
        "installation",
    ]) {
        "repo_access"
    } else {
        "unknown"
    }
}

async fn status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    crate::AppPath(slug): crate::AppPath<String>,
) -> ApiResult<Json<Status>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let app = load_app(&state).await?;
    let job = latest_job(&state, p.id).await?;
    Ok(Json(Status {
        app: app.map(|a| json!({ "slug": a.slug, "html_url": a.html_url })),
        installations: user_installations(&state, user.id).await?,
        link: load_link(&state, p.id).await?,
        failure: job
            .as_ref()
            .filter(|j| j.status == "failed")
            .map(|j| diagnose(j.error.as_deref().unwrap_or_default())),
        job,
    }))
}

/// URLs carrying a fresh state: install the app (new account/repos) or authorize (already installed).
async fn connect(
    State(state): State<AppState>,
    crate::auth::VerifiedUser(user): crate::auth::VerifiedUser,
    crate::AppPath(slug): crate::AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let app = require_app(&state).await?;
    let s = new_state(&state, user.id, Some(p.id)).await?;
    let web = web_base();
    Ok(Json(json!({
        "install_url": format!("{web}/apps/{}/installations/new?state={s}", app.slug),
        "authorize_url": format!(
            "{web}/login/oauth/authorize?client_id={}&state={s}&redirect_uri={}",
            app.client_id,
            urlencode(&format!("{}/api/github/callback", web_origin()))
        ),
    })))
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct InstallQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    setup_action: Option<String>,
}

/// After install/authorize: the user code proves which installations this user can access
/// (`GET /user/installations`), so nobody can claim someone else's installation id.
async fn install_callback(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<InstallQuery>,
) -> ApiResult<Redirect> {
    let s = q
        .state
        .ok_or_else(|| AppError::bad_request("missing state: start again from glot"))?;
    let (user_id, project) = take_state(&state, &s).await?;
    let slug: Option<String> = match project {
        Some(id) => {
            sqlx::query_scalar("SELECT slug FROM projects WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await?
        }
        None => None,
    };
    let page = match &slug {
        Some(s) => format!("{}/app/{s}/github", web_origin()),
        None => format!("{}/app", web_origin()),
    };
    if q.setup_action.as_deref() == Some("request") {
        return Ok(Redirect::to(&format!("{page}?pending=1")));
    }
    let Some(code) = q.code else {
        return Ok(Redirect::to(&format!("{page}?error=no_code")));
    };
    let app = require_app(&state).await?;
    let res = reqwest::Client::new()
        .post(format!("{}/login/oauth/access_token", web_base()))
        .header("accept", "application/json")
        .header("user-agent", "glot")
        .form(&[
            ("client_id", app.client_id.as_str()),
            ("client_secret", app.client_secret.as_str()),
            ("code", code.as_str()),
        ])
        .send()
        .await
        .map_err(|e| gh_error(e.to_string()))?;
    let v: Value = res.json().await.map_err(|e| gh_error(e.to_string()))?;
    let Some(user_token) = v["access_token"].as_str() else {
        tracing::warn!(error = %v["error"], "GitHub OAuth code exchange failed");
        return Ok(Redirect::to(&format!("{page}?error=oauth")));
    };
    let list = gh(
        user_token,
        reqwest::Method::GET,
        &format!("{}/user/installations?per_page=100", api_base()),
        None,
    )
    .await
    .map_err(|e| gh_error(e.to_string()))?;
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM github_user_installations WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for i in list["installations"].as_array().into_iter().flatten() {
        let (Some(id), Some(account)) = (i["id"].as_i64(), i["account"]["login"].as_str()) else {
            continue;
        };
        sqlx::query(
            "INSERT INTO github_user_installations (user_id, installation_id, account) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(id)
        .bind(account)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Redirect::to(&format!("{page}?installed=1")))
}

async fn owned_installation(state: &AppState, user: Uuid, id: i64) -> ApiResult<()> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM github_user_installations WHERE user_id = $1 AND installation_id = $2)",
    )
    .bind(user)
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::new(
            StatusCode::FORBIDDEN,
            "github_installation_unknown",
            "this GitHub installation is not linked to your account: connect GitHub again",
        ))
    }
}

#[derive(Serialize)]
struct Repo {
    full_name: String,
    default_branch: String,
    private: bool,
}

async fn list_repos(app: &App, installation: i64) -> ApiResult<Vec<Repo>> {
    let token = installation_token(app, installation, None)
        .await
        .map_err(|e| gh_error(e.to_string()))?;
    let mut out = vec![];
    // ponytail: first 300 repos (3 pages); a search box when someone has more.
    for page in 1..=3 {
        let v = gh(
            &token,
            reqwest::Method::GET,
            &format!(
                "{}/installation/repositories?per_page=100&page={page}",
                api_base()
            ),
            None,
        )
        .await
        .map_err(|e| gh_error(e.to_string()))?;
        let items = v["repositories"].as_array().cloned().unwrap_or_default();
        for r in &items {
            if let Some(full_name) = r["full_name"].as_str() {
                out.push(Repo {
                    full_name: full_name.into(),
                    default_branch: r["default_branch"].as_str().unwrap_or("main").into(),
                    private: r["private"].as_bool().unwrap_or(false),
                });
            }
        }
        if items.len() < 100 {
            break;
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct ReposQuery {
    installation_id: i64,
}

async fn repos(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    crate::AppPath(slug): crate::AppPath<String>,
    AppQuery(q): AppQuery<ReposQuery>,
) -> ApiResult<Json<Vec<Repo>>> {
    projects::load(&state, user.id, &slug, Role::Admin).await?;
    owned_installation(&state, user.id, q.installation_id).await?;
    let app = require_app(&state).await?;
    Ok(Json(list_repos(&app, q.installation_id).await?))
}

fn valid_repo(r: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && !s.starts_with('.')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    matches!(r.split_once('/'), Some((o, n)) if ok(o) && ok(n))
}

/// Git ref names we accept: no option-like start, no `..`, safe characters only.
fn valid_branch(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 200
        && !b.starts_with(['-', '/', '.'])
        && !b.ends_with(['/', '.'])
        && !b.contains("..")
        && !b.contains("//")
        && b.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_./".contains(&c))
}

#[derive(Deserialize)]
struct LinkBody {
    installation_id: i64,
    repo: String,
    #[serde(default)]
    branch: Option<String>,
}

async fn link(
    State(state): State<AppState>,
    crate::auth::VerifiedUser(user): crate::auth::VerifiedUser,
    crate::AppPath(slug): crate::AppPath<String>,
    AppJson(b): AppJson<LinkBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    owned_installation(&state, user.id, b.installation_id).await?;
    let app = require_app(&state).await?;
    let repo = list_repos(&app, b.installation_id)
        .await?
        .into_iter()
        .find(|r| r.full_name.eq_ignore_ascii_case(&b.repo))
        .ok_or_else(|| {
            AppError::bad_request("this repository is not part of the GitHub installation")
        })?;
    let branch = b
        .branch
        .filter(|s| !s.is_empty())
        .unwrap_or(repo.default_branch);
    if !valid_repo(&repo.full_name) || !valid_branch(&branch) {
        return Err(AppError::bad_request("invalid repository or branch name"));
    }
    crate::billing::check_llm(&state, &p).await?;
    sqlx::query(
        "INSERT INTO github_links (project_id, installation_id, repo, branch) VALUES ($1, $2, $3, $4)
         ON CONFLICT (project_id) DO UPDATE SET installation_id = $2, repo = $3, branch = $4,
           pr_number = CASE WHEN github_links.repo = $3 THEN github_links.pr_number END,
           pr_url = CASE WHEN github_links.repo = $3 THEN github_links.pr_url END,
           pr_branch = CASE WHEN github_links.repo = $3 THEN github_links.pr_branch END,
           updated_at = now()",
    )
    .bind(p.id)
    .bind(b.installation_id)
    .bind(&repo.full_name)
    .bind(&branch)
    .execute(&state.db)
    .await?;
    let job = enqueue(
        &state,
        p.id,
        b.installation_id,
        "github_localize",
        Some(user.id),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "link": load_link(&state, p.id).await?, "job": job })),
    ))
}

async fn unlink(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    crate::AppPath(slug): crate::AppPath<String>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    sqlx::query("DELETE FROM github_links WHERE project_id = $1")
        .bind(p.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_now(
    State(state): State<AppState>,
    crate::auth::VerifiedUser(user): crate::auth::VerifiedUser,
    crate::AppPath(slug): crate::AppPath<String>,
) -> ApiResult<(StatusCode, Json<Job>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let l = load_link(&state, p.id)
        .await?
        .ok_or_else(|| AppError::bad_request("connect a GitHub repository first"))?;
    crate::billing::check_llm(&state, &p).await?;
    let job = enqueue(
        &state,
        p.id,
        l.installation_id,
        "github_localize",
        Some(user.id),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(job)))
}

/// Queue a GitHub job: one pending job per project (coalesced), `GLOT_GITHUB_JOBS_PER_HOUR` (20) per installation.
async fn enqueue(
    state: &AppState,
    project: Uuid,
    installation: i64,
    kind: &str,
    user: Option<Uuid>,
) -> ApiResult<Job> {
    let mut tx = state.db.begin().await?;
    // Serializes enqueues of one installation (webhook bursts) so the count below is exact.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(installation)
        .execute(&mut *tx)
        .await?;
    let pending: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE project_id = $1 AND kind LIKE 'github\\_%' AND status = 'queued' LIMIT 1",
    )
    .bind(project)
    .fetch_optional(&mut *tx)
    .await?;
    if pending.is_none() {
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs j JOIN github_links l ON l.project_id = j.project_id
             WHERE l.installation_id = $1 AND j.kind LIKE 'github\\_%' AND j.created_at > now() - interval '1 hour'",
        )
        .bind(installation)
        .fetch_one(&mut *tx)
        .await?;
        if recent >= num_env("GLOT_GITHUB_JOBS_PER_HOUR", 20) as i64 {
            return Err(AppError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many GitHub runs for this installation in the last hour, try again later",
            ));
        }
        sqlx::query(
            "INSERT INTO jobs (id, project_id, kind, status, params, created_by, progress_total) VALUES ($1, $2, $3, 'queued', '{}', $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(project)
        .bind(kind)
        .bind(user)
        .bind(STEPS)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    state.job_wakeup.notify_one();
    Ok(latest_job(state, project).await?.expect("job just queued"))
}

// ---------- webhook ----------

fn verify_signature(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(sig) = header
        .and_then(|h| h.strip_prefix("sha256="))
        .and_then(|h| hex::decode(h).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

async fn webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let app = match load_app(&state).await {
        Ok(Some(a)) => a,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return e.into_response(),
    };
    let header = |k: &str| headers.get(k).and_then(|v| v.to_str().ok());
    if !verify_signature(&app.webhook_secret, &body, header("x-hub-signature-256")) {
        return AppError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "invalid webhook signature",
        )
        .into_response();
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return AppError::bad_request("invalid JSON").into_response();
    };
    match handle_event(&state, header("x-github-event").unwrap_or(""), &v).await {
        Ok(n) => (StatusCode::ACCEPTED, Json(json!({ "queued": n }))).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Returns the number of jobs queued.
async fn handle_event(state: &AppState, event: &str, v: &Value) -> ApiResult<usize> {
    let repo = v["repository"]["full_name"].as_str().unwrap_or("");
    let installation = v["installation"]["id"].as_i64().unwrap_or(0);
    match event {
        "push" => {
            if v["deleted"].as_bool() == Some(true) {
                return Ok(0);
            }
            let r#ref = v["ref"].as_str().unwrap_or("");
            let links: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT project_id, branch FROM github_links WHERE lower(repo) = lower($1) AND installation_id = $2",
            )
            .bind(repo)
            .bind(installation)
            .fetch_all(&state.db)
            .await?;
            let mut n = 0;
            for (project, branch) in links {
                if r#ref != format!("refs/heads/{branch}") {
                    continue;
                }
                let p = project_row(state, project).await.map_err(AppError::from)?;
                if let Err(e) = crate::billing::check_llm(state, &p).await {
                    tracing::info!(project = %p.slug, "push ignored: {}", e.message);
                    continue;
                }
                match enqueue(state, project, installation, "github_update", None).await {
                    Ok(_) => n += 1,
                    Err(e) => tracing::info!(project = %p.slug, "push ignored: {}", e.message),
                }
            }
            Ok(n)
        }
        "pull_request" if v["action"] == "closed" => {
            if v["pull_request"]["merged"] == true {
                crate::review_live::github::on_merged(state, repo, installation, v).await?;
            }
            // The glot PR was merged or closed: the next run opens a new one.
            sqlx::query(
                "UPDATE github_links SET pr_number = NULL, pr_url = NULL, pr_branch = NULL, updated_at = now()
                 WHERE lower(repo) = lower($1) AND installation_id = $2 AND pr_number = $3",
            )
            .bind(repo)
            .bind(installation)
            .bind(v["number"].as_i64().unwrap_or(-1) as i32)
            .execute(&state.db)
            .await?;
            Ok(0)
        }
        "installation" if v["action"] == "deleted" => {
            sqlx::query("DELETE FROM github_links WHERE installation_id = $1")
                .bind(installation)
                .execute(&state.db)
                .await?;
            sqlx::query("DELETE FROM github_user_installations WHERE installation_id = $1")
                .bind(installation)
                .execute(&state.db)
                .await?;
            Ok(0)
        }
        _ => Ok(0),
    }
}

async fn project_row(state: &AppState, id: Uuid) -> sqlx::Result<ProjectRow> {
    sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at, syntax FROM projects WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
}

// ---------- the job ----------

/// Temp dir removed on drop, whatever happens (errors, panics, timeouts).
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs a command with a clean env, a timeout (whole process group killed) and the token scrubbed from errors.
/// `secret` reaches git only as an `http.extraHeader` from env vars.
async fn exec(
    program: &str,
    args: &[&str],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
    secrets: &[&str],
) -> anyhow::Result<(i32, String)> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Own process group so a timeout kills glot and every child it spawned (Unix; Windows relies on kill_on_drop).
    #[cfg(unix)]
    cmd.process_group(0);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("cannot run {program}: {e}"))?;
    let pid = child.id();
    let scrub = |s: String| {
        secrets
            .iter()
            .filter(|x| !x.is_empty())
            .fold(s, |s, x| s.replace(x, "***"))
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(out) => {
            let out = out?;
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            Ok((out.status.code().unwrap_or(-1), scrub(text)))
        }
        Err(_) => {
            // glot spawns child glots: kill the whole group, not just the direct child.
            if let Some(pid) = pid {
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .status();
            }
            anyhow::bail!(
                "{program} {} timed out after {}s",
                args.first().unwrap_or(&""),
                timeout.as_secs()
            )
        }
    }
}

/// `git` with no system/global config, no prompts, the token as an extra header (never in argv or .git/config).
async fn git(
    dir: &Path,
    home: &Path,
    token: &str,
    args: &[&str],
    timeout: Duration,
) -> anyhow::Result<String> {
    let header = format!(
        "Authorization: Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"))
    );
    let env = [
        ("HOME", home.display().to_string()),
        ("GIT_TERMINAL_PROMPT", "0".into()),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
        ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
        ("GIT_LFS_SKIP_SMUDGE", "1".into()),
        ("GIT_CONFIG_COUNT", "2".into()),
        ("GIT_CONFIG_KEY_0", "http.extraHeader".into()),
        ("GIT_CONFIG_VALUE_0", header.clone()),
        ("GIT_CONFIG_KEY_1", "core.hooksPath".into()),
        ("GIT_CONFIG_VALUE_1", "/dev/null".into()),
    ];
    let env = env.map(|(k, v)| (k.to_string(), v));
    let (code, out) = exec("git", args, dir, &env, timeout, &[token, &header]).await?;
    if code != 0 {
        anyhow::bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            out.trim().chars().take(500).collect::<String>()
        );
    }
    Ok(out)
}

/// Bytes on disk, never following symlinks; Err if one points outside or the tree is too large.
/// Never checked out in a localization run: media, fonts, archives, models, compiled artifacts.
const SKIP_EXT: &[&str] = &[
    "mp4",
    "mov",
    "webm",
    "mkv",
    "avi",
    "m4v",
    "mp3",
    "wav",
    "ogg",
    "flac",
    "m4a",
    "aac",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "avif",
    "heic",
    "ico",
    "bmp",
    "tif",
    "tiff",
    "psd",
    "ai",
    "sketch",
    "fig",
    "blend",
    "blend1",
    "glb",
    "gltf",
    "fbx",
    "obj",
    "usdz",
    "woff",
    "woff2",
    "ttf",
    "otf",
    "eot",
    "zip",
    "gz",
    "tgz",
    "bz2",
    "xz",
    "7z",
    "rar",
    "jar",
    "pdf",
    "wasm",
    "so",
    "dylib",
    "dll",
    "exe",
    "bin",
    "onnx",
    "pt",
    "safetensors",
    "gguf",
    "npy",
    "parquet",
    "sqlite",
    "db",
];

/// Paths of `git ls-tree -r -l` entries larger than `limit` bytes.
fn large_paths(ls_tree: &str, limit: u64) -> impl Iterator<Item = &str> {
    ls_tree.lines().filter_map(move |l| {
        let (meta, path) = l.split_once('\t')?;
        let size: u64 = meta.split_whitespace().nth(3)?.parse().ok()?;
        (size > limit).then_some(path)
    })
}

/// Escapes a path for a gitignore-style sparse-checkout pattern.
fn sparse_escape(p: &str) -> String {
    p.chars()
        .flat_map(|c| {
            let esc = matches!(c, '*' | '?' | '[' | ']' | '\\' | '!' | '#' | ' ');
            esc.then_some('\\').into_iter().chain(std::iter::once(c))
        })
        .collect()
}

fn tree_size(dir: &Path, max: u64) -> anyhow::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let m = e.path().symlink_metadata()?;
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
            if total > max {
                anyhow::bail!("repository larger than {} MB", max >> 20);
            }
        }
    }
    Ok(total)
}

/// `root` when it holds a glot.toml, else the shallowest subdirectory that does (depth <= 3,
/// skipping vendored, test and example trees), else `root` (`glot init` runs there).
fn app_dir(root: &Path) -> PathBuf {
    const SKIP: &[&str] = &[
        "node_modules",
        "target",
        "vendor",
        "examples",
        "example",
        "tests",
        "test",
        "fixtures",
    ];
    let mut level = vec![root.to_path_buf()];
    for _ in 0..=3 {
        let mut found: Vec<PathBuf> = level
            .iter()
            .filter(|d| d.join(glot_core::config::CONFIG_FILE).is_file())
            .cloned()
            .collect();
        if !found.is_empty() {
            found.sort();
            return found.swap_remove(0);
        }
        level = level
            .iter()
            .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                !n.starts_with('.') && !SKIP.contains(&n.as_str())
            })
            .map(|e| e.path())
            .collect();
    }
    root.to_path_buf()
}

/// A repo-provided glot.toml must not make glot read outside the clone (catalog paths, extract globs).
fn check_config(dir: &Path) -> anyhow::Result<()> {
    let path = dir.join(glot_core::config::CONFIG_FILE);
    if !path.exists() {
        return Ok(());
    }
    if path.symlink_metadata()?.file_type().is_symlink() {
        anyhow::bail!("glot.toml is a symlink");
    }
    let c = glot_core::config::Config::load(dir)?;
    let escapes = |p: &str| {
        p.starts_with('/')
            || p.starts_with('\\')
            || p.contains("..")
            || p.contains(':')
            || p.starts_with('~')
    };
    if let Some(bad) = c
        .files
        .iter()
        .chain(&c.extract.include)
        .find(|p| escapes(p))
    {
        anyhow::bail!("glot.toml: path `{bad}` points outside the repository");
    }
    Ok(())
}

struct Done {
    install_cmd: Option<String>,
    check_ok: bool,
    warnings: Vec<String>,
    log: String,
    /// What `glot translate` planned and spent (its `Plan:` and `Cost:` lines), for the PR.
    plan: Option<String>,
    cost: Option<String>,
    /// `glot i18nize` report (`GLOT_REPORT`): strings left with a suggestion, files restored after
    /// a failed verification. `Null` when the step did not run.
    report: Value,
}

/// `glot init` (new repos) + `glot localize --yes --no-install` with the project's LLM key in env, nothing else.
async fn localize(
    dir: &Path,
    home: &Path,
    p: &ProjectRow,
    api_key: Option<String>,
    carry: Option<&Path>,
) -> anyhow::Result<Done> {
    let bin = env_or("GLOT_BIN", "glot");
    let mem_kb = num_env("GLOT_GITHUB_MEM_MB", 1024) * 1024;
    let timeout = Duration::from_secs(num_env("GLOT_GITHUB_TIMEOUT_SECS", 900));
    let mut env: Vec<(String, String)> = vec![
        ("HOME".into(), home.display().to_string()),
        ("NO_COLOR".into(), "1".into()),
        ("GLOT_BIN_PATH".into(), bin.clone()),
        // Outside the clone: never committed.
        (
            "GLOT_REPORT".into(),
            home.join("report.json").display().to_string(),
        ),
    ];
    let key = api_key
        .map(|k| glot_llm::provider::clean_key(&k))
        .unwrap_or_default();
    // Org user keys (sk-ant-usr…) must name the workspace: the CLI reads ANTHROPIC_WORKSPACE_ID.
    if let Some(ws) = p
        .llm
        .0
        .anthropic_workspace_id
        .as_ref()
        .filter(|w| !w.is_empty())
    {
        env.push(("ANTHROPIC_WORKSPACE_ID".into(), ws.clone()));
    }
    if !key.is_empty() {
        let var = glot_core::config::Config::load(dir)
            .ok()
            .and_then(|c| c.llm.api_key_env);
        for v in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY"]
            .into_iter()
            .map(String::from)
            .chain(var.filter(|v| v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')))
        {
            env.push((v, key.clone()));
        }
    }
    let provider = p.llm.0.provider.clone();
    // The project's LLM settings win over the repository's `[llm]` in every glot of this run
    // (`forced_llm` in the CLI): a committed `provider = "mock"` never ships fake translations.
    env.push(("GLOT_LLM_PROVIDER".into(), provider.clone()));
    for (var, v) in [
        ("GLOT_LLM_MODEL", Some(&p.llm.0.model)),
        ("GLOT_LLM_BASE_URL", p.llm.0.base_url.as_ref()),
        ("GLOT_LLM_FALLBACK_MODEL", p.llm.0.fallback_model.as_ref()),
        ("GLOT_LLM_JUDGE_MODEL", p.llm.0.judge_model.as_ref()),
    ] {
        if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
            env.push((var.into(), v.clone()));
        }
    }
    // ulimit caps address space and CPU time of glot and its children; tolerated where unsupported (macOS dev).
    let sh = format!(
        "ulimit -v {mem_kb} 2>/dev/null; ulimit -t {} 2>/dev/null; exec \"$GLOT_BIN_PATH\" \"$@\" 2>&1",
        timeout.as_secs()
    );
    let mut log = String::new();
    let fresh = !dir.join(glot_core::config::CONFIG_FILE).exists();
    if fresh {
        let targets = p.target_locales.join(",");
        let (code, out) = exec(
            "sh",
            &[
                "-c",
                &sh,
                "glot",
                "init",
                "--yes",
                "--source",
                &p.source_locale,
                "--targets",
                &targets,
                "--provider",
                &provider,
            ],
            dir,
            &env,
            timeout,
            &[&key],
        )
        .await?;
        log.push_str(&out);
        if code != 0 {
            anyhow::bail!("glot init failed: {}", tail(&out, 15));
        }
        check_config(dir)?;
    }
    let mut args = vec!["-c", &sh, "glot", "localize", "--yes", "--no-install"];
    args.extend(["--provider", &provider]);
    let carry = carry.map(|c| c.display().to_string());
    if let Some(c) = &carry {
        args.extend(["--carry-from", c]);
    }
    let (code, out) = exec("sh", &args, dir, &env, timeout, &[&key]).await?;
    log.push_str(&out);
    // localize: 0 ok, 1 = translated but `glot check` failed (reported in the PR), else a real failure.
    if code != 0 && code != 1 {
        anyhow::bail!("glot localize failed: {}", tail(&out, 15));
    }
    Ok(Done {
        install_cmd: out
            .lines()
            .find_map(|l| l.trim().strip_prefix("install: run "))
            .map(|s| s.trim().to_string()),
        check_ok: code == 0,
        warnings: out
            .lines()
            .filter_map(|l| l.trim().strip_prefix("warning: "))
            .map(String::from)
            .collect(),
        plan: out
            .lines()
            .find_map(|l| l.trim().strip_prefix("Plan: "))
            .map(|s| s.trim().to_string()),
        cost: out
            .lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix("Cost: "))
            .map(|s| s.trim().to_string()),
        log,
        report: std::fs::read_to_string(home.join("report.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null),
    })
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

async fn step(state: &AppState, job: &mut JobRecord, n: i32) {
    job.progress_done = n;
    job.progress_total = STEPS;
    jobs::save(state, job).await;
}

pub(crate) async fn run_job(state: &AppState, job: &mut JobRecord) -> anyhow::Result<()> {
    step(state, job, 0).await;
    let link = load_link(state, job.project_id)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?
        .ok_or_else(|| anyhow::anyhow!("the project is no longer linked to a GitHub repository"))?;
    let app = require_app(state)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let p = project_row(state, job.project_id).await?;
    anyhow::ensure!(
        valid_repo(&link.repo) && valid_branch(&link.branch),
        "invalid repository or branch"
    );
    let api_key = match &p.api_key_enc {
        Some(enc) => Some(crate::auth::decrypt(&state.secret, enc).ok_or_else(|| {
            anyhow::anyhow!("cannot decrypt the project API key (GLOT_SECRET_KEY changed?)")
        })?),
        None => None,
    };
    if api_key.is_none() && !matches!(p.llm.0.provider.as_str(), "mock" | "local") {
        anyhow::bail!("add your LLM API key in the project settings: glot translates with it");
    }
    let repo_name = link
        .repo
        .split_once('/')
        .map(|(_, n)| n)
        .unwrap_or(&link.repo);
    let token = installation_token(&app, link.installation_id, Some(repo_name)).await?;
    step(state, job, 1).await;

    // Clone: dedicated dir, shallow, symlinks checked out as plain files, size capped.
    let root = Scratch(std::env::temp_dir().join(format!("glot-gh-{}", job.id)));
    let _ = std::fs::remove_dir_all(&root.0);
    let (home, dir) = (root.0.join("home"), root.0.join("repo"));
    std::fs::create_dir_all(&home)?;
    let url = format!("{}/{}.git", git_base(), link.repo);
    let t = Duration::from_secs(num_env("GLOT_GITHUB_GIT_TIMEOUT_SECS", 180));
    git(
        &root.0,
        &home,
        &token,
        &[
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--no-tags",
            // Big repos: blobs over 2 MB are never downloaded; media, binaries and those files are never checked out.
            "--filter=blob:limit=2m",
            "--no-checkout",
            "--branch",
            &link.branch,
            "-c",
            "core.symlinks=false",
            "--",
            &url,
            "repo",
        ],
        t,
    )
    .await?;
    let tree = git(&dir, &home, &token, &["ls-tree", "-r", "-l", "HEAD"], t).await?;
    let mut patterns = vec!["/*".to_string()];
    patterns.extend(SKIP_EXT.iter().map(|e| format!("!*.{e}")));
    patterns.extend(large_paths(&tree, 2 << 20).map(|p| format!("!/{}", sparse_escape(p))));
    let pattern_file = root.0.join("sparse");
    std::fs::write(&pattern_file, patterns.join("\n") + "\n")?;
    std::fs::create_dir_all(dir.join(".git/info"))?;
    std::fs::copy(&pattern_file, dir.join(".git/info/sparse-checkout"))?;
    git(
        &dir,
        &home,
        &token,
        &["config", "core.sparseCheckout", "true"],
        t,
    )
    .await?;
    git(&dir, &home, &token, &["checkout", "-q", "HEAD"], t).await?;
    let max = num_env("GLOT_GITHUB_MAX_REPO_MB", 4096) << 20;
    let size_dir = dir.clone();
    tokio::task::spawn_blocking(move || tree_size(&size_dir, max)).await??;
    let base_sha = git(&dir, &home, &token, &["rev-parse", "HEAD"], t)
        .await?
        .trim()
        .to_string();
    step(state, job, 2).await;

    // The open glot PR, if any: its branch is the previous output this run starts from.
    let api = format!("{}/repos/{}", api_base(), link.repo);
    let open_pr = match (link.pr_number, &link.pr_branch) {
        (Some(n), Some(b)) => {
            let v = gh(
                &token,
                reqwest::Method::GET,
                &format!("{api}/pulls/{n}"),
                None,
            )
            .await;
            match v {
                Ok(v) => (v["state"] == "open").then(|| (n, b.clone())),
                // The recorded PR is gone (repo recreated or transferred): open a new one.
                Err(e) if e.to_string().starts_with("GitHub 404") => None,
                Err(e) => return Err(e),
            }
        }
        _ => None,
    };
    let branch = match &open_pr {
        Some((_, b)) => b.clone(),
        None if job.kind == "github_update" => "glot/update".into(),
        None => "glot/localize".into(),
    };
    anyhow::ensure!(valid_branch(&branch), "invalid pull request branch");

    // glot localize (glot.toml checked before and after: it decides which files glot reads).
    // Monorepo without a root glot.toml: the shallowest configured app (e.g. `web/`).
    let app_root = app_dir(&dir);
    check_config(&app_root)?;
    // The repository's glot.toml says which language the code is written in: a project rooted on
    // another one would store translations as the source (and translate from them).
    if let Ok(c) = glot_core::config::Config::load(&app_root)
        && c.source_locale != p.source_locale
    {
        anyhow::bail!(
            "source_mismatch: repo={} project={}: the repository's glot.toml declares `source_locale = \"{}\"` but this glot project's source is `{}`",
            c.source_locale,
            p.source_locale,
            c.source_locale,
            p.source_locale
        );
    }
    let app_rel = app_root
        .strip_prefix(&dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    // Previous output (PR branch): unchanged strings then cost nothing and produce no diff.
    let prev = match &open_pr {
        Some((_, b)) => previous_output(&dir, &home, &token, b, &app_rel, &root.0, t).await,
        None => None,
    };
    let keys_before = glot_llm::engine::Workspace::open(&app_root)
        .map(|ws| ws.source().entries.len())
        .unwrap_or(0);
    let done = localize(
        &app_root,
        &home,
        &p,
        api_key,
        prev.as_ref().map(|(dir, _)| dir.as_path()),
    )
    .await?;
    check_config(&app_root)?;
    // `Cost: $0.0123 …`: shown on the run in the dashboard.
    if let Some(usd) = done
        .cost
        .as_deref()
        .and_then(|c| c.split('$').nth(1))
        .and_then(|c| {
            c.split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .next()?
                .parse::<f64>()
                .ok()
        })
    {
        job.cost_usd = usd;
    }
    step(state, job, 3).await;

    // Sync into the project: same rules as `glot push` (reviewed/human translations on the server win).
    let ws = glot_llm::engine::Workspace::open(&app_root).map_err(|e| anyhow::anyhow!("{e}"))?;
    // Last gate before anything leaves this machine (server sync, branch, PR): no fake value.
    if p.llm.0.provider != "mock"
        && let Some((loc, key, n)) = mock_values(&ws)
    {
        anyhow::bail!(
            "mock_output: {n} value(s) are placeholder translations (`⟦{loc}⟧ …`, e.g. `{loc}` `{key}`): nothing was synced or pushed"
        );
    }
    let locales: Vec<&String> = std::iter::once(&p.source_locale)
        .chain(&p.target_locales)
        .collect();
    let body = crate::sync::PushBody {
        catalogs: ws
            .catalogs
            .iter()
            .filter(|(l, _)| locales.contains(l))
            .map(|(l, c)| (l.clone(), c.entries.clone()))
            .collect(),
        lock: ws.lock.clone(),
        glossary: ws.glossary.terms.clone(),
        context: Default::default(),
        source_locale: None,
        // The repo's glot.toml is the truth for this run's catalogs (dashboard choice overridden).
        syntax: serde_json::to_value(ws.config.syntax)?
            .as_str()
            .map(str::to_string),
        set_syntax: true,
    };
    let pushed = crate::sync::apply_push(state, &p, body, "glot[bot]")
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let status = ws.status();
    step(state, job, 4).await;

    // Commit on the glot branch (workflow files excluded: the app has no `workflows` permission).
    git(&dir, &home, &token, &["checkout", "-q", "-B", &branch], t).await?;
    git(
        &dir,
        &home,
        &token,
        &["add", "-A", "--", ".", ":(exclude).github/workflows"],
        t,
    )
    .await?;
    let stat = git(&dir, &home, &token, &["diff", "--cached", "--stat=100"], t).await?;
    let seen = || {
        sqlx::query(
            "UPDATE github_links SET last_sha = $2, updated_at = now() WHERE project_id = $1",
        )
        .bind(p.id)
        .bind(&base_sha)
        .execute(&state.db)
    };
    if stat.trim().is_empty() && open_pr.is_none() {
        seen().await?;
        step(state, job, STEPS).await;
        return Ok(()); // already localized: nothing to propose.
    }
    let bot = format!("{}[bot]", app.slug);
    let email = format!("{}+{bot}@users.noreply.github.com", app.app_id);
    let (name_cfg, email_cfg) = (format!("user.name={bot}"), format!("user.email={email}"));
    let title = if job.kind == "github_update" {
        format!(
            "glot: update translations ({})",
            p.target_locales.join(", ")
        )
    } else {
        format!("glot: localize this app ({})", p.target_locales.join(", "))
    };
    if !stat.trim().is_empty() {
        git(
            &dir,
            &home,
            &token,
            &[
                "-c",
                &name_cfg,
                "-c",
                &email_cfg,
                "commit",
                "-q",
                "--no-verify",
                "-m",
                &title,
            ],
            t,
        )
        .await?;
    }
    // Commits people pushed to the glot branch are never dropped: the branch is rebuilt on the
    // latest base (so it never conflicts with it), then their commits are replayed on top, with
    // their authorship. One that no longer applies stops the run before anything is pushed.
    if let Some((n, _)) = &open_pr {
        let commits = gh(
            &token,
            reqwest::Method::GET,
            &format!("{api}/pulls/{n}/commits?per_page=100"),
            None,
        )
        .await?;
        for c in human_commits(&commits, &bot, &email) {
            let picked = async {
                git(
                    &dir,
                    &home,
                    &token,
                    &["fetch", "-q", "--depth", "2", "origin", &c],
                    t,
                )
                .await?;
                git(
                    &dir,
                    &home,
                    &token,
                    &[
                        "-c",
                        &name_cfg,
                        "-c",
                        &email_cfg,
                        "cherry-pick",
                        "--allow-empty",
                        "--keep-redundant-commits",
                        &c,
                    ],
                    t,
                )
                .await
            }
            .await;
            if let Err(e) = picked {
                let short = &c[..c.len().min(7)];
                let note = format!(
                    "{STICKY}\n**glot** did not update this pull request for `{}`: commit {short}, pushed to `{branch}` by a person, no longer applies on top of the latest `{}`.\n\nNothing was overwritten. Either merge `{}` into `{branch}` and fix the conflict, or close this pull request: glot opens a fresh one on the next push.",
                    &base_sha[..base_sha.len().min(7)],
                    link.branch,
                    link.branch
                );
                let _ = sticky_comment(&token, &api, *n, &note).await;
                anyhow::bail!(
                    "pr_conflict: human commit {short} on {branch} does not apply on the latest {} ({})",
                    link.branch,
                    tail(&e.to_string(), 2)
                );
            }
        }
    }
    // Same tree as the branch already holds: the PR is up to date, no push, no notification.
    let unchanged = match &prev {
        Some((_, head)) => {
            let old = git(
                &dir,
                &home,
                &token,
                &["rev-parse", &format!("{head}^{{tree}}")],
                t,
            )
            .await
            .ok();
            let new = git(&dir, &home, &token, &["rev-parse", "HEAD^{tree}"], t)
                .await
                .ok();
            old.is_some() && old == new
        }
        None => false,
    };
    let head_is_base = git(&dir, &home, &token, &["rev-parse", "HEAD"], t)
        .await?
        .trim()
        == base_sha;
    if unchanged {
        seen().await?;
        step(state, job, STEPS).await;
        return Ok(());
    }
    if head_is_base {
        // Everything the PR proposed is now on the base branch: nothing left to propose.
        if let Some((n, _)) = &open_pr {
            let note = format!(
                "{STICKY}\n**glot**: `{}` now contains everything this pull request proposed. It can be closed.",
                link.branch
            );
            let _ = sticky_comment(&token, &api, *n, &note).await;
        }
        seen().await?;
        step(state, job, STEPS).await;
        return Ok(());
    }
    // Never overwrite a push that landed while this run was working.
    let lease = match &prev {
        Some((_, head)) => format!("--force-with-lease=refs/heads/{branch}:{head}"),
        None => "--force".into(),
    };
    git(
        &dir,
        &home,
        &token,
        &[
            "push",
            "-q",
            &lease,
            "origin",
            &format!("HEAD:refs/heads/{branch}"),
        ],
        t,
    )
    .await?;
    step(state, job, 5).await;

    // One open glot PR at a time: update it, else find one GitHub already has for the branch, else open it.
    let stat = git(
        &dir,
        &home,
        &token,
        &["diff", "--stat=100", &base_sha, "HEAD"],
        t,
    )
    .await
    .unwrap_or(stat);
    let blob = format!(
        "{}/{}/blob/{branch}/{}",
        web_base(),
        link.repo,
        if app_rel.is_empty() {
            String::new()
        } else {
            format!("{app_rel}/")
        }
    );
    let body = pr_body(
        &p,
        &link,
        &base_sha,
        keys_before,
        &status,
        &pushed,
        &done,
        &stat,
        &blob,
    );
    let owner = link.repo.split('/').next().unwrap_or("");
    let existing = match open_pr {
        Some((n, _)) => Some(n),
        None => gh(
            &token,
            reqwest::Method::GET,
            &format!("{api}/pulls?state=open&head={owner}:{branch}"),
            None,
        )
        .await?
        .as_array()
        .and_then(|a| a.first())
        .and_then(|pr| pr["number"].as_i64())
        .map(|n| n as i32),
    };
    let pr =
        match existing {
            Some(n) => {
                let pr = gh(
                    &token,
                    reqwest::Method::PATCH,
                    &format!("{api}/pulls/{n}"),
                    Some(json!({ "title": title, "body": body })),
                )
                .await?;
                // One status comment, edited in place: a push does not add a notification each time.
                sticky_comment(
                    &token,
                    &api,
                    n,
                    &status_comment(&base_sha, &status, &pushed, &done),
                )
                .await?;
                pr
            }
            None => gh(
                &token,
                reqwest::Method::POST,
                &format!("{api}/pulls"),
                Some(json!({ "title": title, "head": branch, "base": link.branch, "body": body })),
            )
            .await?,
        };
    sqlx::query(
        "UPDATE github_links SET last_sha = $2, pr_number = $3, pr_url = $4, pr_branch = $5, updated_at = now() WHERE project_id = $1",
    )
    .bind(p.id)
    .bind(&base_sha)
    .bind(pr["number"].as_i64().map(|n| n as i32))
    .bind(pr["html_url"].as_str())
    .bind(&branch)
    .execute(&state.db)
    .await?;
    step(state, job, STEPS).await;
    Ok(())
}

/// Values written by the offline mock provider (`⟦fr⟧ Text`, `mock::fake_translate`), in any
/// catalog, the source included: (locale, a key, count).
fn mock_values(ws: &glot_llm::engine::Workspace) -> Option<(String, String, usize)> {
    let mut first = None;
    let mut n = 0;
    for (loc, cat) in &ws.catalogs {
        for (key, v) in &cat.entries {
            if has_mock_marker(v) {
                n += 1;
                first.get_or_insert_with(|| (loc.clone(), key.clone()));
            }
        }
    }
    first.map(|(l, k)| (l, k, n))
}

/// `⟦xx⟧` / `⟦pt-BR⟧`: the mock provider's marker.
pub(crate) fn has_mock_marker(v: &str) -> bool {
    v.match_indices('⟦').any(|(i, _)| {
        let rest = &v[i + '⟦'.len_utf8()..];
        rest.find('⟧').is_some_and(|e| {
            let l = &rest[..e];
            (2..=12).contains(&l.len())
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    })
}

/// Marker of the single status comment glot keeps on its pull request.
const STICKY: &str = "<!-- glot:status -->";

/// Create the status comment, or edit the one already there.
async fn sticky_comment(token: &str, api: &str, pr: i32, body: &str) -> anyhow::Result<()> {
    let list = gh(
        token,
        reqwest::Method::GET,
        &format!("{api}/issues/{pr}/comments?per_page=100"),
        None,
    )
    .await?;
    let mine = list.as_array().and_then(|a| {
        a.iter()
            .find(|c| c["body"].as_str().is_some_and(|b| b.contains(STICKY)))
            .and_then(|c| c["id"].as_i64())
    });
    let (method, url) = match mine {
        Some(id) => (
            reqwest::Method::PATCH,
            format!("{api}/issues/comments/{id}"),
        ),
        None => (reqwest::Method::POST, format!("{api}/issues/{pr}/comments")),
    };
    gh(token, method, &url, Some(json!({ "body": body }))).await?;
    Ok(())
}

/// SHAs (oldest first) of the PR commits a person made: not authored by the app, not a merge
/// (a merge of the base brings nothing once the branch is rebuilt on the latest base).
fn human_commits(commits: &Value, bot: &str, bot_email: &str) -> Vec<String> {
    commits
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            c["author"]["login"].as_str() != Some(bot)
                && c["commit"]["author"]["email"].as_str() != Some(bot_email)
                && c["commit"]["author"]["name"].as_str() != Some(bot)
                && c["parents"].as_array().is_none_or(|p| p.len() < 2)
        })
        .filter_map(|c| c["sha"].as_str())
        .filter(|s| s.len() >= 7 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(String::from)
        .collect()
}

/// Directory prefix (or exact path) a catalog pattern can match: `messages/{locale}.json` =>
/// `messages/`, `web/src/i18n.rs` => itself.
fn pattern_prefix(pattern: &str) -> &str {
    match pattern.find(['{', '*', '?', '[']) {
        Some(i) => pattern[..i].rfind('/').map_or("", |k| &pattern[..=k]),
        None => pattern,
    }
}

/// The glot files of the PR branch (config, lock, glossary, catalogs) written to a scratch
/// directory, with the branch head. `None`: no such branch or nothing readable (a full run then).
async fn previous_output(
    dir: &Path,
    home: &Path,
    token: &str,
    branch: &str,
    app_rel: &str,
    scratch: &Path,
    t: Duration,
) -> Option<(PathBuf, String)> {
    let remote = format!("refs/remotes/origin/{branch}");
    git(
        dir,
        home,
        token,
        &[
            "fetch",
            "-q",
            "--depth",
            "1",
            "origin",
            &format!("+refs/heads/{branch}:{remote}"),
        ],
        t,
    )
    .await
    .ok()?;
    let head = git(dir, home, token, &["rev-parse", &remote], t)
        .await
        .ok()?
        .trim()
        .to_string();
    let rel = |p: &str| {
        if app_rel.is_empty() {
            p.to_string()
        } else {
            format!("{app_rel}/{p}")
        }
    };
    let out = scratch.join("prev");
    let show = |path: String| {
        let (out, head) = (out.clone(), head.clone());
        async move {
            let text = git(dir, home, token, &["show", &format!("{head}:{path}")], t)
                .await
                .ok()?;
            let dest = out.join(&path);
            std::fs::create_dir_all(dest.parent()?).ok()?;
            std::fs::write(dest, text).ok()
        }
    };
    show(rel(glot_core::config::CONFIG_FILE)).await?;
    let prev_app = out.join(app_rel);
    check_config(&prev_app).ok()?;
    let config = glot_core::config::Config::load(&prev_app).ok()?;
    let prefixes: Vec<String> = config
        .files
        .iter()
        .map(|f| rel(pattern_prefix(f)))
        .collect();
    let tree = git(dir, home, token, &["ls-tree", "-r", "-l", &head], t)
        .await
        .ok()?;
    let wanted: Vec<String> = tree
        .lines()
        .filter_map(|l| {
            let (meta, path) = l.split_once('\t')?;
            let size: u64 = meta.split_whitespace().nth(3)?.parse().ok()?;
            let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            let ours = path == rel("glot.lock")
                || path == rel("glossary.toml")
                || path == rel(".glot/tm.json")
                || prefixes.iter().any(|p| {
                    if p.ends_with('/') || p.is_empty() {
                        path.starts_with(p.as_str())
                    } else {
                        path == p
                    }
                });
            (ours && size <= 2 << 20 && !SKIP_EXT.contains(&ext.as_str()) && !path.contains(".."))
                .then(|| path.to_string())
        })
        .take(2000)
        .collect();
    for path in wanted {
        show(path).await;
    }
    Some((prev_app, head))
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn bar(pct: usize) -> String {
    let full = pct / 10;
    format!("{}{}", "█".repeat(full), "░".repeat(10 - full))
}

fn coverage_table(status: &glot_llm::engine::Status) -> String {
    let mut s = String::from(
        "| Language | Coverage | Strings | Missing or rejected | Validation errors |\n|---|---|---:|---:|---:|\n",
    );
    for (loc, l) in &status.locales {
        let pct = (l.translated * 100).checked_div(l.total).unwrap_or(100);
        s.push_str(&format!(
            "| `{loc}` | {} {pct} % | {}/{} | {} | {} |\n",
            bar(pct),
            l.translated,
            l.total,
            l.missing,
            l.errors
        ));
    }
    s
}

/// Strings glot did not move, from the i18nize report: where (linked), why, and the code to write.
fn left_section(report: &Value, blob: &str) -> String {
    let left = report["left"].as_array().cloned().unwrap_or_default();
    let reverted = report["reverted"].as_array().cloned().unwrap_or_default();
    let mut s = String::new();
    if !reverted.is_empty() {
        s.push_str(&format!(
            "\n### Files left untouched\nglot re-reads every file it rewrites with the language's own parser before committing. {} did not pass, so the original was kept:\n",
            if reverted.len() == 1 { "One file" } else { "These files" }
        ));
        for r in reverted.iter().take(20) {
            let (path, why) = (r[0].as_str().unwrap_or(""), r[1].as_str().unwrap_or(""));
            s.push_str(&format!("- [`{path}`]({blob}{path}): {}\n", cell(why)));
        }
    }
    if left.is_empty() {
        return s;
    }
    s.push_str(&format!(
        "\n### {} string{} left for you\nglot only rewrites what cannot change a type or a behavior. Each line below gives the place, the reason and the code to paste:\n\n",
        left.len(),
        if left.len() == 1 { "" } else { "s" }
    ));
    let row = |l: &Value| {
        let (path, line) = (
            l["path"].as_str().unwrap_or(""),
            l["line"].as_u64().unwrap_or(0),
        );
        format!(
            "- [`{path}:{line}`]({blob}{path}#L{line}) {}\n  ```\n  {}\n  ```\n  {}\n",
            cell(&format!("“{}”", l["text"].as_str().unwrap_or(""))),
            l["suggestion"].as_str().unwrap_or("").replace("```", "'''"),
            cell(l["reason"].as_str().unwrap_or(""))
        )
    };
    const SHOWN: usize = 15;
    for l in left.iter().take(SHOWN) {
        s.push_str(&row(l));
    }
    if left.len() > SHOWN {
        s.push_str(&format!(
            "\n<details><summary>{} more</summary>\n\n",
            left.len() - SHOWN
        ));
        // GitHub caps a PR body at 65 536 characters.
        for l in left.iter().skip(SHOWN).take(150) {
            s.push_str(&row(l));
        }
        s.push_str("\n</details>\n");
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn pr_body(
    p: &ProjectRow,
    link: &LinkRow,
    sha: &str,
    keys_before: usize,
    status: &glot_llm::engine::Status,
    pushed: &crate::sync::PushResult,
    done: &Done,
    stat: &str,
    blob: &str,
) -> String {
    let web = web_origin();
    let converted = status.keys.saturating_sub(keys_before);
    let short = &sha[..sha.len().min(7)];
    let mut s = format!(
        "## Localized with glot\n\nglot scanned `{}` at `{short}`, moved **{converted} hardcoded string{}** into catalogs and translated them into {}.\n\n",
        link.branch,
        if converted == 1 { "" } else { "s" },
        p.target_locales
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let r = &done.report;
    if let (Some(files), Some(kept)) = (
        r["files_rewritten"].as_u64(),
        r["imported_translations"].as_u64(),
    ) {
        s.push_str(&format!(
            "**What changed**: {files} source file{} now call{} the translation function instead of holding text{}. Every rewritten file was parsed again before this commit, and every key it uses exists in the source catalog.\n\n",
            if files == 1 { "" } else { "s" },
            if files == 1 { "s" } else { "" },
            if kept > 0 {
                format!(
                    "; {kept} translation{} already written in the code {} kept as is, not re-translated",
                    if kept == 1 { "" } else { "s" },
                    if kept == 1 { "was" } else { "were" }
                )
            } else {
                String::new()
            }
        ));
    }
    s.push_str(&coverage_table(status));
    s.push_str(&format!(
        "\nSource `{}`: **{} keys**. Synced with the glot project [`{}`]({web}/app/{}): {} strings added, {} updated, {} reviewed translations kept.\n",
        status.source_locale, status.keys, p.slug, p.slug, pushed.created, pushed.updated, pushed.conflicts.len()
    ));
    s.push_str(if done.check_ok {
        "\n`glot check` passes: placeholders, ICU plurals and glossary are valid in every language.\n"
    } else {
        "\n`glot check` reports issues: see the table above, fix them in the dashboard or with `glot translate --force`.\n"
    });
    if let Some(cost) = &done.cost {
        s.push_str(&format!(
            "\n**Cost of this run**: {}{}, billed by your AI provider on your own key. Strings already translated are never sent again.\n",
            cell(cost),
            done.plan
                .as_ref()
                .map(|p| format!(" for {}", cell(p)))
                .unwrap_or_default()
        ));
    }
    s.push_str(&left_section(r, blob));
    s.push_str("\n### How to test\n");
    if let Some(cmd) = &done.install_cmd {
        s.push_str(&format!(
            "- [ ] Run `{cmd}` and commit the lockfile: glot never runs package managers or your code on its servers.\n"
        ));
    }
    s.push_str(&format!(
        "- [ ] Check out this branch, build the app as usual and switch language: every screen reads the same in `{}` and is translated in the other languages.\n- [ ] Skim the diff of the files below: each change replaces a text by a key, nothing else.\n",
        status.source_locale
    ));
    if !done.warnings.is_empty() {
        s.push_str(&format!(
            "- [ ] {} string{} left as is (review them):\n",
            done.warnings.len(),
            if done.warnings.len() == 1 {
                " was"
            } else {
                "s were"
            }
        ));
        for w in done.warnings.iter().take(10) {
            s.push_str(&format!("  - {}\n", cell(w)));
        }
    }
    let files = stat.lines().count().saturating_sub(1);
    s.push_str(&format!(
        "\n<details><summary>{files} file{} changed</summary>\n\n```\n{}\n```\n</details>\n",
        if files == 1 { "" } else { "s" },
        tail(stat, 60).replace("```", "'''")
    ));
    s.push_str(&format!(
        "\n<details><summary>glot output</summary>\n\n```\n{}\n```\n</details>\n",
        tail(&done.log, 60).replace("```", "'''")
    ));
    s.push_str(&format!(
        "\n### Next steps\n- Merge this PR. glot keeps translations up to date: each push to `{}` processes only the strings that changed, in this same pull request (or a single `glot/update` one after the merge). Strings you delete leave the catalogs; keys you rename keep their translations.\n- Commits you push to this branch are kept: glot rebuilds the branch on the latest `{}` and replays them on top.\n- Review or edit translations in the [dashboard]({web}/app/{}): reviewed values are never overwritten.\n- Optional: add the glot GitHub Action (`.github/workflows/glot.yml`, created by `glot init` locally) to check every PR; glot does not push workflow files.\n",
        link.branch, link.branch, p.slug
    ));
    // GitHub refuses a body over 65 536 characters.
    if s.len() > 60_000 {
        let mut cut = 60_000;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n\n…");
    }
    s
}

fn status_comment(
    sha: &str,
    status: &glot_llm::engine::Status,
    pushed: &crate::sync::PushResult,
    done: &Done,
) -> String {
    format!(
        "{STICKY}\n**glot** updated this PR for `{}`: {} keys, {} translations written, `glot check` {}.\n\n{}",
        &sha[..sha.len().min(7)],
        status.keys,
        pushed.created + pushed.updated,
        if done.check_ok {
            "passes"
        } else {
            "reports issues"
        },
        coverage_table(status)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"s3cret").unwrap();
        mac.update(b"{}");
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature("s3cret", b"{}", Some(&sig)));
        assert!(!verify_signature("other", b"{}", Some(&sig)));
        assert!(!verify_signature("s3cret", b"{ }", Some(&sig)));
        assert!(!verify_signature("s3cret", b"{}", None));
    }

    #[test]
    fn names() {
        assert!(valid_repo("acme/web.app"));
        assert!(!valid_repo("acme/../x") && !valid_repo("acme") && !valid_repo("-x/y z"));
        assert!(valid_branch("main") && valid_branch("release/1.2"));
        assert!(!valid_branch("--upload-pack=x") && !valid_branch("a..b") && !valid_branch("a b"));
    }

    #[test]
    fn app_dir_prefers_root_then_shallowest() {
        let d = std::env::temp_dir().join(format!("glot-app-{}", Uuid::new_v4()));
        for f in [
            "web/glot.toml",
            "crates/x/emails/glot.toml",
            "examples/ios/glot.toml",
        ] {
            std::fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
            std::fs::write(d.join(f), "").unwrap();
        }
        assert_eq!(app_dir(&d), d.join("web"));
        std::fs::write(d.join("glot.toml"), "").unwrap();
        assert_eq!(app_dir(&d), d);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn big_and_media_files_are_skipped() {
        let tree = "100644 blob aaa 1048576\tsrc/i18n.rs\n100644 blob bbb 5242880\tassets/demo data.json\n";
        assert_eq!(
            large_paths(tree, 2 << 20).collect::<Vec<_>>(),
            ["assets/demo data.json"]
        );
        assert_eq!(
            sparse_escape("assets/demo data[1].json"),
            "assets/demo\\ data\\[1\\].json"
        );
        assert!(SKIP_EXT.contains(&"mp4"));
    }

    #[test]
    fn failures_are_classified() {
        for (error, class) in [
            (
                "add your LLM API key in the project settings: glot translates with it",
                "ai_key_missing",
            ),
            (
                "glot localize failed: anthropic: HTTP 401: {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}",
                "ai_key_invalid",
            ),
            (
                "glot localize failed: organization key (sk-ant-usr…) needs a workspace ID: set it in Project → Settings → LLM",
                "ai_workspace",
            ),
            (
                "glot localize failed: openai: HTTP 429: You exceeded your current quota",
                "ai_quota",
            ),
            (
                "glot localize failed: anthropic: HTTP 529: overloaded",
                "ai_unavailable",
            ),
            ("repository larger than 4096 MB", "repo_too_large"),
            ("sh -c timed out after 900s", "timeout"),
            (
                "git clone failed: fatal: Remote branch release not found in upstream origin",
                "branch_missing",
            ),
            (
                "git clone failed: remote: Repository not found.",
                "repo_access",
            ),
            (
                "glot.toml: path `../secrets` points outside the repository",
                "config_invalid",
            ),
            (
                "glot init failed: cannot detect the framework from package.json; pass --lib",
                "no_framework",
            ),
            (
                "git push failed: remote: error: GH006: Protected branch update failed",
                "push_rejected",
            ),
            (
                "pr_conflict: human commit abc1234 on glot/localize does not apply on the latest main",
                "pr_conflict",
            ),
            (
                "the project is no longer linked to a GitHub repository",
                "unlinked",
            ),
            (
                "source_mismatch: repo=en project=fr: the repository's glot.toml declares `source_locale = \"en\"`",
                "source_mismatch",
            ),
            (
                "mock_output: 3763 value(s) are placeholder translations (`⟦fr⟧ …`)",
                "mock_output",
            ),
            ("something nobody planned", "unknown"),
        ] {
            assert_eq!(diagnose(error), class, "{error}");
        }
    }

    #[test]
    fn mock_markers() {
        for v in ["⟦fr⟧ Web App", "  ⟦pt-BR⟧ Olá", "Hi ⟦zh_Hant⟧"] {
            assert!(has_mock_marker(v), "{v}");
        }
        for v in ["Web App", "⟦⟧", "⟦not a locale⟧", "« fr » text", "⟦fr"] {
            assert!(!has_mock_marker(v), "{v}");
        }
    }

    #[test]
    fn human_commits_are_found() {
        let commits = json!([
            { "sha": "1111111aaaa", "author": { "login": "glot-test[bot]" }, "commit": { "author": { "name": "glot-test[bot]", "email": "42+glot-test[bot]@users.noreply.github.com" } }, "parents": [{}] },
            { "sha": "2222222bbbb", "author": { "login": "ana" }, "commit": { "author": { "name": "Ana", "email": "ana@example.com" } }, "parents": [{}] },
            // "Update branch" on GitHub: a merge of the base, nothing to replay.
            { "sha": "3333333cccc", "author": { "login": "ana" }, "commit": { "author": { "name": "Ana", "email": "ana@example.com" } }, "parents": [{}, {}] },
            { "sha": "--upload-pack=x", "author": { "login": "eve" }, "commit": { "author": { "name": "Eve", "email": "e@example.com" } }, "parents": [{}] },
        ]);
        assert_eq!(
            human_commits(
                &commits,
                "glot-test[bot]",
                "42+glot-test[bot]@users.noreply.github.com"
            ),
            ["2222222bbbb"]
        );
    }

    #[test]
    fn catalog_prefixes() {
        assert_eq!(pattern_prefix("messages/{locale}.json"), "messages/");
        assert_eq!(pattern_prefix("web/src/i18n.rs"), "web/src/i18n.rs");
        assert_eq!(pattern_prefix("{locale}.json"), "");
        assert_eq!(
            pattern_prefix("public/locales/{locale}/*.json"),
            "public/locales/"
        );
    }

    #[test]
    fn leftovers_are_listed_with_place_reason_and_code() {
        let report = json!({
            "left": [{ "path": "src/pages/a.rs", "line": 12, "text": "Vidéo | sans légende", "reason": "its type is declared there", "suggestion": "t(\"a_video\")" }],
            "reverted": [["src/b.rs", "line 3: expected `}`"]],
        });
        let s = left_section(
            &report,
            "https://github.com/acme/app/blob/glot/localize/web/",
        );
        assert!(s.contains("[`src/pages/a.rs:12`](https://github.com/acme/app/blob/glot/localize/web/src/pages/a.rs#L12)"), "{s}");
        assert!(
            s.contains("t(\"a_video\")") && s.contains("its type is declared there"),
            "{s}"
        );
        assert!(s.contains("Vidéo \\| sans légende"), "table-safe: {s}");
        assert!(
            s.contains("[`src/b.rs`]") && s.contains("expected `}`"),
            "{s}"
        );
        assert_eq!(left_section(&Value::Null, ""), "");
    }

    #[test]
    fn config_cannot_escape() {
        let d = std::env::temp_dir().join(format!("glot-cfg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let w = |files: &str| {
            std::fs::write(
                d.join("glot.toml"),
                format!("source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [{files}]\n"),
            )
            .unwrap()
        };
        w("\"messages/{locale}.json\"");
        assert!(check_config(&d).is_ok());
        w("\"../../etc/{locale}.json\"");
        assert!(check_config(&d).is_err());
        w("\"/proc/self/{locale}\"");
        assert!(check_config(&d).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
