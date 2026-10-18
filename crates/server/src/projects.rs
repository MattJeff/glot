use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState, auth::AuthUser, validate_locale, validate_slug,
};

pub const PROVIDERS: &[&str] = &["anthropic", "openai", "openai-compatible", "mock"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Translator,
    Admin,
    Owner,
}

/// LLM settings as stored (the API key lives encrypted in `projects.api_key_enc`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStored {
    pub provider: String,
    pub model: String,
    pub fallback_model: Option<String>,
    pub judge_model: Option<String>,
    pub base_url: Option<String>,
    pub budget_usd: Option<f64>,
    /// Sent as `anthropic-workspace-id` (org-level user keys must name the workspace to bill).
    /// Falls back to the `ANTHROPIC_WORKSPACE_ID` env var when unset.
    #[serde(default)]
    pub anthropic_workspace_id: Option<String>,
    /// Extra non-secret HTTP headers sent with every LLM call of the project.
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
}

impl LlmStored {
    /// Headers for `glot_llm::provider::from_config_with` (jobs + Review API).
    pub fn headers(&self) -> BTreeMap<String, String> {
        let mut h = self.extra_headers.clone();
        if let Some(ws) = &self.anthropic_workspace_id {
            h.insert("anthropic-workspace-id".into(), ws.clone());
        }
        h
    }
}

impl Default for LlmStored {
    fn default() -> Self {
        let d = glot_core::config::LlmConfig::default();
        Self {
            provider: d.provider,
            model: d.model,
            fallback_model: d.fallback_model,
            judge_model: d.judge_model,
            base_url: None,
            budget_usd: None,
            anthropic_workspace_id: None,
            extra_headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub source_locale: String,
    pub target_locales: Vec<String>,
    pub style: Option<String>,
    pub public_cdn: bool,
    pub llm: DbJson<LlmStored>,
    pub api_key_enc: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LlmView {
    pub provider: String,
    pub model: String,
    pub fallback_model: Option<String>,
    pub judge_model: Option<String>,
    pub has_api_key: bool,
    pub budget_usd: Option<f64>,
    pub anthropic_workspace_id: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
}

#[derive(Serialize, Default, sqlx::FromRow, utoipa::ToSchema)]
pub struct LocaleStats {
    pub translated: i64,
    pub reviewed: i64,
    pub stale: i64,
    pub issues: i64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Stats {
    pub keys: i64,
    pub locales: BTreeMap<String, LocaleStats>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Project {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub source_locale: String,
    pub target_locales: Vec<String>,
    pub style: Option<String>,
    pub public_cdn: bool,
    pub created_at: DateTime<Utc>,
    pub llm: LlmView,
    pub stats: Stats,
}

const PROJECT_COLS: &str = "p.id, p.slug, p.name, p.source_locale, p.target_locales, p.style, p.public_cdn, p.llm, p.api_key_enc, p.created_at";

/// Load a project the user is a member of, requiring at least `min` role.
/// Non-members get 404 (no existence leak).
pub async fn load(state: &AppState, user: Uuid, slug: &str, min: Role) -> ApiResult<ProjectRow> {
    #[derive(sqlx::FromRow)]
    struct WithRole {
        #[sqlx(flatten)]
        p: ProjectRow,
        role: String,
    }
    let row: Option<WithRole> = sqlx::query_as(&format!(
        "SELECT {PROJECT_COLS}, m.role FROM projects p JOIN project_members m ON m.project_id = p.id
         WHERE p.slug = $1 AND m.user_id = $2"
    ))
    .bind(slug)
    .bind(user)
    .fetch_optional(&state.db)
    .await?;
    let Some(WithRole { p, role }) = row else {
        return Err(AppError::not_found("project not found"));
    };
    let role = match role.as_str() {
        "owner" => Role::Owner,
        "admin" => Role::Admin,
        _ => Role::Translator,
    };
    if role < min {
        return Err(AppError::forbidden());
    }
    Ok(p)
}

pub async fn load_public(state: &AppState, slug: &str) -> ApiResult<Option<ProjectRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {PROJECT_COLS} FROM projects p WHERE p.slug = $1"
    ))
    .bind(slug)
    .fetch_optional(&state.db)
    .await?)
}

async fn view(state: &AppState, p: ProjectRow) -> ApiResult<Project> {
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM keys WHERE project_id = $1")
        .bind(p.id)
        .fetch_one(&state.db)
        .await?;
    let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT t.locale,
                count(*) FILTER (WHERE t.status <> 'stale'),
                count(*) FILTER (WHERE t.status = 'reviewed'),
                count(*) FILTER (WHERE t.status = 'stale'),
                count(*) FILTER (WHERE jsonb_array_length(t.issues) > 0)
         FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 GROUP BY t.locale",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let mut locales: BTreeMap<String, LocaleStats> = p
        .target_locales
        .iter()
        .map(|l| (l.clone(), LocaleStats::default()))
        .collect();
    for (loc, translated, reviewed, stale, issues) in rows {
        if let Some(s) = locales.get_mut(&loc) {
            *s = LocaleStats {
                translated,
                reviewed,
                stale,
                issues,
            };
        }
    }
    let llm = p.llm.0;
    Ok(Project {
        id: p.id,
        slug: p.slug,
        name: p.name,
        source_locale: p.source_locale,
        target_locales: p.target_locales,
        style: p.style,
        public_cdn: p.public_cdn,
        created_at: p.created_at,
        llm: LlmView {
            provider: llm.provider,
            model: llm.model,
            fallback_model: llm.fallback_model,
            judge_model: llm.judge_model,
            has_api_key: p.api_key_enc.is_some(),
            budget_usd: llm.budget_usd,
            anthropic_workspace_id: llm.anthropic_workspace_id,
            extra_headers: llm.extra_headers,
        },
        stats: Stats { keys, locales },
    })
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> ApiResult<Json<Vec<Project>>> {
    let rows: Vec<ProjectRow> = sqlx::query_as(&format!(
        "SELECT {PROJECT_COLS} FROM projects p JOIN project_members m ON m.project_id = p.id
         WHERE m.user_id = $1 ORDER BY p.created_at DESC"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for p in rows {
        out.push(view(&state, p).await?);
    }
    Ok(Json(out))
}

fn clean_targets(source: &str, targets: Vec<String>) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in targets {
        let t = t.trim().to_string();
        validate_locale(&t)?;
        if t == source {
            return Err(AppError::bad_request(
                "target_locales must not contain the source locale",
            ));
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    slug: String,
    source_locale: String,
    #[serde(default)]
    target_locales: Vec<String>,
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<Project>)> {
    validate_slug(&b.slug)?;
    validate_locale(&b.source_locale)?;
    let name = b.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::bad_request("name is required (max 100 chars)"));
    }
    let targets = clean_targets(&b.source_locale, b.target_locales)?;
    let mut tx = state.db.begin().await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO projects (id, slug, name, source_locale, target_locales, llm) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(id)
        .bind(&b.slug)
        .bind(name)
        .bind(&b.source_locale)
        .bind(&targets)
        .bind(DbJson(LlmStored::default()))
        .execute(&mut *tx)
        .await
        .map_err(|e| match AppError::from(e) {
            err if err.status == StatusCode::CONFLICT => AppError::new(StatusCode::CONFLICT, "slug_taken", "slug already taken"),
            err => err,
        })?;
    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let p = load(&state, user.id, &b.slug, Role::Translator).await?;
    Ok((StatusCode::CREATED, Json(view(&state, p).await?)))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Project>> {
    let p = load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(view(&state, p).await?))
}

#[derive(Deserialize)]
pub struct LlmSettings {
    provider: String,
    model: String,
    fallback_model: Option<String>,
    judge_model: Option<String>,
    /// Write-only. Empty string clears the stored key; absent keeps it.
    api_key: Option<String>,
    base_url: Option<String>,
    budget_usd: Option<f64>,
    /// Absent keeps the stored value, "" clears it.
    anthropic_workspace_id: Option<String>,
    /// Absent keeps the stored headers; non-secret only (auth headers are refused).
    extra_headers: Option<BTreeMap<String, String>>,
}

/// Header names that carry credentials or are managed by the HTTP client: never user-set.
fn secret_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "authorization",
        "proxy-authorization",
        "cookie",
        "host",
        "content-length",
        "content-type",
        "anthropic-version",
    ]
    .contains(&n.as_str())
        || ["key", "token", "secret", "auth", "password"]
            .iter()
            .any(|w| n.contains(w))
}

fn clean_headers(h: BTreeMap<String, String>) -> ApiResult<BTreeMap<String, String>> {
    if h.len() > 20 {
        return Err(AppError::bad_request("extra_headers: at most 20 headers"));
    }
    let mut out = BTreeMap::new();
    for (k, v) in h {
        let k = k.trim().to_ascii_lowercase();
        if k.is_empty()
            || k.len() > 64
            || !k
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(AppError::bad_request(format!(
                "extra_headers: invalid header name \"{k}\""
            )));
        }
        if secret_header(&k) {
            return Err(AppError::bad_request(format!(
                "extra_headers: \"{k}\" looks like a credential; use api_key instead"
            )));
        }
        let v = v.trim().to_string();
        if v.len() > 256 || v.chars().any(char::is_control) {
            return Err(AppError::bad_request(format!(
                "extra_headers: invalid value for \"{k}\""
            )));
        }
        out.insert(k, v);
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: Option<String>,
    target_locales: Option<Vec<String>>,
    llm: Option<LlmSettings>,
    style: Option<String>,
    public_cdn: Option<bool>,
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<UpdateBody>,
) -> ApiResult<Json<Project>> {
    let mut p = load(&state, user.id, &slug, Role::Admin).await?;
    if let Some(name) = b.name {
        let name = name.trim().to_string();
        if name.is_empty() || name.len() > 100 {
            return Err(AppError::bad_request("name is required (max 100 chars)"));
        }
        p.name = name;
    }
    if let Some(t) = b.target_locales {
        p.target_locales = clean_targets(&p.source_locale, t)?;
    }
    if let Some(style) = b.style {
        p.style = Some(style).filter(|s| !s.trim().is_empty());
    }
    if let Some(public) = b.public_cdn {
        p.public_cdn = public;
    }
    if let Some(l) = b.llm {
        if !PROVIDERS.contains(&l.provider.as_str()) {
            return Err(AppError::bad_request(format!(
                "provider must be one of {}",
                PROVIDERS.join(", ")
            )));
        }
        if l.model.trim().is_empty() {
            return Err(AppError::bad_request("model is required"));
        }
        if l.budget_usd.is_some_and(|b| !b.is_finite() || b < 0.0) {
            return Err(AppError::bad_request("budget_usd must be >= 0"));
        }
        match l.api_key.as_deref() {
            Some("") => p.api_key_enc = None,
            Some(k) => p.api_key_enc = Some(crate::auth::encrypt(&state.secret, k)),
            None => {}
        }
        let nonempty = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
        let prev = p.llm.0.clone();
        let anthropic_workspace_id = match l.anthropic_workspace_id {
            None => prev.anthropic_workspace_id,
            Some(ws) => {
                let ws = ws.trim().to_string();
                if ws.len() > 128
                    || !ws
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err(AppError::bad_request("invalid anthropic_workspace_id"));
                }
                Some(ws).filter(|w| !w.is_empty())
            }
        };
        let extra_headers = match l.extra_headers {
            None => prev.extra_headers,
            Some(h) => clean_headers(h)?,
        };
        p.llm = DbJson(LlmStored {
            provider: l.provider,
            model: l.model,
            fallback_model: nonempty(l.fallback_model),
            judge_model: nonempty(l.judge_model),
            base_url: nonempty(l.base_url),
            budget_usd: l.budget_usd,
            anthropic_workspace_id,
            extra_headers,
        });
    }
    sqlx::query(
        "UPDATE projects SET name = $2, target_locales = $3, style = $4, public_cdn = $5, llm = $6, api_key_enc = $7 WHERE id = $1",
    )
    .bind(p.id)
    .bind(&p.name)
    .bind(&p.target_locales)
    .bind(&p.style)
    .bind(p.public_cdn)
    .bind(&p.llm)
    .bind(&p.api_key_enc)
    .execute(&state.db)
    .await?;
    Ok(Json(view(&state, p).await?))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<StatusCode> {
    let p = load(&state, user.id, &slug, Role::Owner).await?;
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(p.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
