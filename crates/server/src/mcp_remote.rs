//! Remote MCP: `POST /mcp` (Streamable HTTP, stateless, rmcp) over the server state of ONE project.
//! For clients that only accept remote MCP servers (Lovable, Bolt, v0, ChatGPT, Claude web).
//! Auth: an API token (`glot_…` + `?project=slug`) or an OAuth access token (`glot_at_…`, project bound at consent).
//! Tools reuse the HTTP handlers, so roles, validation and history are identical to the REST API.
use std::{collections::BTreeSet, sync::Arc, time::Instant};

use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    handler::server::{
        router::tool::ToolRouter,
        tool::{Extension, ToolCallContext},
        wrapper::Parameters,
    },
    model::*,
    service::RequestContext,
    tool, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::{AuthUser, User, sha256_hex},
    glossary, jobs, keys,
    projects::{self, Role},
    review, sync,
};

pub mod oauth;

const MAX_ITEMS: usize = 100;

/// Who is calling, resolved once per HTTP request by [`guard`] and read by every tool.
#[derive(Clone, Debug)]
pub struct McpAuth {
    pub user: User,
    pub project_id: Uuid,
    pub slug: String,
    pub scopes: Vec<String>,
    /// None = API token.
    pub client_id: Option<String>,
}

/// Scope needed by each tool (unknown tools fall through to rmcp's "tool not found").
fn scope_of(tool: &str) -> &'static str {
    match tool {
        "translate" | "review" => "translate",
        "add_string" | "set_translation" | "glossary_upsert" | "translate_repo" => "write",
        _ => "read",
    }
}

pub fn routes(state: AppState, web_origin: &str) -> Router<AppState> {
    let mut origins: Vec<String> = vec![web_origin.to_string()];
    origins.extend(
        std::env::var("GLOT_MCP_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    );
    // Origin: browsers must come from an allowed origin (DNS rebinding); server-side clients send none.
    let mut cfg = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_origins(origins)
        .enforce_origin_validation();
    // Host: rmcp defaults to loopback only. Behind a public URL, accept exactly that host.
    cfg = match std::env::var("GLOT_PUBLIC_URL")
        .ok()
        .and_then(|u| reqwest::Url::parse(&u).ok())
    {
        Some(u) => {
            let host = u.host_str().unwrap_or_default().to_string();
            let mut hosts = vec![host.clone()];
            if let Some(port) = u.port() {
                hosts.push(format!("{host}:{port}"));
            }
            cfg.with_allowed_hosts(hosts)
        }
        None => cfg.disable_allowed_hosts(),
    };
    let proto = Remote::new(state.clone());
    let mcp = StreamableHttpService::new(
        move || Ok(proto.clone()),
        Arc::new(NeverSessionManager::default()),
        cfg,
    );
    Router::new()
        .route_service("/mcp", mcp)
        .layer(axum::middleware::from_fn_with_state(state, guard))
        .merge(oauth::routes(web_origin.to_string()))
}

// ---------- HTTP guard: auth, rate limit, scope challenge ----------

fn challenge(headers: &HeaderMap, status: StatusCode, extra: &str, msg: &str) -> Response {
    let meta = oauth::resource_metadata_url(headers);
    let code = if status == StatusCode::FORBIDDEN {
        "insufficient_scope"
    } else {
        "unauthorized"
    };
    (
        status,
        [(
            header::WWW_AUTHENTICATE,
            format!("Bearer resource_metadata=\"{meta}\"{extra}"),
        )],
        Json(json!({ "error": { "code": code, "message": msg } })),
    )
        .into_response()
}

async fn resolve(
    state: &AppState,
    headers: &HeaderMap,
    query: Option<&str>,
    token: &str,
) -> ApiResult<Option<McpAuth>> {
    if token.starts_with("glot_at_") {
        let row: Option<(Uuid, String, String, Uuid, String, Vec<String>, String, Option<String>)> = sqlx::query_as(
            "UPDATE oauth_grants g SET last_used_at = now() FROM users u, projects p
             WHERE g.access_hash = $1 AND g.access_expires_at > now() AND u.id = g.user_id AND p.id = g.project_id
               AND EXISTS (SELECT 1 FROM project_members m WHERE m.project_id = p.id AND m.user_id = u.id)
             RETURNING u.id, u.email, u.name, p.id, p.slug, g.scopes, g.client_id, g.resource",
        )
        .bind(sha256_hex(token))
        .fetch_optional(&state.db)
        .await?;
        let Some((uid, email, name, project_id, slug, scopes, client_id, resource)) = row else {
            return Ok(None);
        };
        // RFC 8707 audience: a token minted for another resource is not ours.
        let base = oauth::base_url(headers);
        if resource.is_some_and(|r| {
            let r = r.trim_end_matches('/');
            r != base && r != format!("{base}/mcp")
        }) {
            return Ok(None);
        }
        return Ok(Some(McpAuth {
            user: User {
                id: uid,
                email,
                name,
            },
            project_id,
            slug,
            scopes,
            client_id: Some(client_id),
        }));
    }
    let Some(user) = crate::auth::authenticate(headers, state).await? else {
        return Ok(None);
    };
    let slug = query
        .and_then(|q| reqwest::Url::parse(&format!("http://x/?{q}")).ok())
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "project")
                .map(|(_, v)| v.into_owned())
        })
        .or_else(|| {
            headers
                .get("x-glot-project")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        })
        .ok_or_else(|| {
            AppError::bad_request(
                "API tokens must name the project: /mcp?project=<slug> or X-Glot-Project",
            )
        })?;
    let p = projects::load(state, user.id, &slug, Role::Translator).await?;
    Ok(Some(McpAuth {
        user,
        project_id: p.id,
        slug,
        scopes: oauth::SCOPES.map(String::from).to_vec(),
        client_id: None,
    }))
}

/// Shared per-token per-minute budget (same Postgres scheme as insights / feedback).
async fn rate_limited(state: &AppState, token: &str) -> ApiResult<Option<i64>> {
    let budget: i32 = std::env::var("GLOT_MCP_RATE_PER_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let (hits, window_end): (i32, i64) = sqlx::query_as(
        "INSERT INTO mcp_rate AS r (token_hash, window_start, hits) VALUES ($1, date_trunc('minute', now()), 1)
         ON CONFLICT (token_hash) DO UPDATE SET
           hits = CASE WHEN excluded.window_start > r.window_start THEN 1 ELSE r.hits + 1 END,
           window_start = greatest(r.window_start, excluded.window_start)
         RETURNING r.hits, extract(epoch FROM r.window_start + interval '1 minute')::int8",
    )
    .bind(sha256_hex(token))
    .fetch_one(&state.db)
    .await?;
    Ok((hits > budget).then(|| (window_end - chrono::Utc::now().timestamp()).max(1)))
}

/// Tool names of the `tools/call` requests in a JSON-RPC body (object or batch).
fn called_tools(body: &[u8]) -> Vec<String> {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let msgs = match v {
        Value::Array(a) => a,
        v => vec![v],
    };
    msgs.iter()
        .filter(|m| m["method"] == "tools/call")
        .filter_map(|m| m["params"]["name"].as_str().map(String::from))
        .collect()
}

async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let headers = req.headers().clone();
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or(v.strip_prefix("bearer ")))
        .map(|t| t.trim().to_string());
    let Some(token) = token else {
        return challenge(
            &headers,
            StatusCode::UNAUTHORIZED,
            &format!(", scope=\"{}\"", oauth::SCOPES.join(" ")),
            "authentication required",
        );
    };
    let auth = match resolve(&state, &headers, req.uri().query(), &token).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return challenge(
                &headers,
                StatusCode::UNAUTHORIZED,
                ", error=\"invalid_token\"",
                "invalid or expired token",
            );
        }
        Err(e) => return e.into_response(),
    };
    match rate_limited(&state, &token).await {
        Ok(None) => {}
        Ok(Some(retry)) => {
            return (
                [(header::RETRY_AFTER, retry.to_string())],
                AppError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "too many MCP requests for this token; see Retry-After",
                ),
            )
                .into_response();
        }
        Err(e) => return e.into_response(),
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 4 << 20).await else {
        return AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "request body over 4 MiB",
        )
        .into_response();
    };
    let missing: BTreeSet<&str> = called_tools(&bytes)
        .iter()
        .map(|t| scope_of(t))
        .filter(|s| !auth.scopes.iter().any(|g| g == s))
        .collect();
    if !missing.is_empty() {
        let need = missing.into_iter().collect::<Vec<_>>().join(" ");
        return challenge(
            &headers,
            StatusCode::FORBIDDEN,
            &format!(", error=\"insufficient_scope\", scope=\"{need}\""),
            &format!("this token lacks the '{need}' scope"),
        );
    }
    let mut req = Request::from_parts(parts, Body::from(bytes));
    req.extensions_mut().insert(auth);
    next.run(req).await
}

// ---------- MCP handler ----------

#[derive(Clone)]
pub struct Remote {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

fn auth_of(parts: &Parts) -> Result<McpAuth, String> {
    parts
        .extensions
        .get::<McpAuth>()
        .cloned()
        .ok_or_else(|| "unauthenticated".into())
}

fn done<T: Serialize>(r: Result<T, String>) -> CallToolResult {
    match r.map(|v| serde_json::to_string_pretty(&v).unwrap_or_default()) {
        Ok(s) => CallToolResult::success(vec![ContentBlock::text(s)]),
        Err(m) => CallToolResult::error(vec![ContentBlock::text(m)]),
    }
}

fn e(err: AppError) -> String {
    format!("{} ({}): {}", err.code, err.status.as_u16(), err.message)
}

fn from<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid arguments: {e}"))
}

// ---------- params ----------

#[derive(Deserialize, JsonSchema)]
pub struct GetContextParams {
    /// Source file path relative to the repo root: keys whose recorded usages (`glot push`) are in this file.
    pub file: Option<String>,
    /// Translation keys to inspect (e.g. `["auth.login.cta"]`).
    pub keys: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProposeParams {
    /// User-visible source text in the source locale, ICU placeholders (e.g. `Hello {name}`).
    pub text: String,
    /// Key you would like to use (dot path).
    pub suggested_key: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddStringParams {
    /// Dot-path key (e.g. `settings.billing.save`).
    pub key: String,
    /// Source text in the source locale.
    pub text: String,
    /// Note for translators (where/how it is shown, max length, tone).
    pub note: Option<String>,
    /// Replace an existing key whose text differs. Default false.
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TranslateParams {
    /// Target locales (default: all).
    pub locales: Option<Vec<String>>,
    /// Only these keys (default: everything missing or stale).
    pub keys: Option<Vec<String>>,
    /// Also retranslate up-to-date entries (human/reviewed ones are never overwritten). Default false.
    pub force: Option<bool>,
    /// Seconds to wait for the job to finish before returning its id (0-50, default 20).
    pub wait_seconds: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CheckParams {
    /// Only this locale.
    pub locale: Option<String>,
    /// Skip the first N issues. Default 0.
    pub offset: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetTranslationParams {
    /// Target locale (e.g. `fr`).
    pub locale: String,
    pub key: String,
    /// The translated text.
    pub value: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct GlossaryLookupParams {
    /// Only the terms occurring in this text (default: whole glossary).
    pub text: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GlossaryUpsertParams {
    /// Source-language term (case-insensitive match).
    pub term: String,
    /// Required translations per locale, e.g. `{"fr": "Espace de travail"}`.
    pub translations: Option<std::collections::BTreeMap<String, String>>,
    /// Keep the term verbatim in every locale (brand names).
    pub do_not_translate: Option<bool>,
    pub description: Option<String>,
    /// Replace conflicting existing translations. Default false.
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReviewParams {
    /// Target locale to review.
    pub locale: String,
    /// Only these keys (default: up to 200 translations of the locale).
    pub keys: Option<Vec<String>>,
    /// Ignore cached reviews. Default false.
    pub refresh: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TranslateRepoParams {
    /// Branch to open the PR against (default: the project's GitHub base branch).
    pub base: Option<String>,
}

#[tool_router(router = tool_router)]
impl Remote {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    fn ctx(
        &self,
        parts: &Parts,
    ) -> Result<(State<AppState>, AuthUser, AppPath<String>, McpAuth), String> {
        let a = auth_of(parts)?;
        Ok((
            State(self.state.clone()),
            AuthUser(a.user.clone()),
            AppPath(a.slug.clone()),
            a,
        ))
    }

    #[tool(
        description = "Localization overview of the connected glot project: source/target locales, key count and per-locale stats (translated / reviewed / stale / issues). Call this first."
    )]
    async fn status(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        done(
            async {
                let (s, u, p, _) = self.ctx(&parts)?;
                projects::get(s, u, p).await.map(|j| j.0).map_err(e)
            }
            .await,
        )
    }

    #[tool(
        description = "Context for keys and/or a source file (from `glot push`): source text, translator note, where each key is used, UI role, value + status + issues in every locale, and the glossary terms that apply. Use before editing UI text."
    )]
    async fn get_context(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GetContextParams>,
    ) -> CallToolResult {
        done(async {
            let (s, u, slug, a) = self.ctx(&parts)?;
            if p.file.is_none() && p.keys.is_none() {
                return Err("pass `file` and/or `keys`".into());
            }
            let mut wanted = p.keys.unwrap_or_default();
            if let Some(f) = &p.file {
                let in_file: Vec<String> = sqlx::query_scalar(
                    "SELECT key FROM keys WHERE project_id = $1
                     AND EXISTS (SELECT 1 FROM jsonb_array_elements(refs) r WHERE r->>'path' = $2) ORDER BY key",
                )
                .bind(a.project_id)
                .bind(f.trim_start_matches("./"))
                .fetch_all(&self.state.db)
                .await
                .map_err(|x| e(x.into()))?;
                wanted.extend(in_file);
            }
            wanted.dedup();
            let total = wanted.len();
            wanted.truncate(MAX_ITEMS);
            let g = glossary::load(&self.state.db, a.project_id).await.map_err(e)?;
            let mut terms = BTreeSet::new();
            let mut items = Vec::new();
            // ponytail: one search query per key (≤100); batch it if remote get_context gets hot.
            for k in &wanted {
                let q = from(json!({ "q": k, "per_page": 500 }))?;
                let page = keys::list(s.clone(), AuthUser(u.0.clone()), AppPath(slug.0.clone()), AppQuery(q)).await.map_err(e)?.0;
                match page.items.into_iter().find(|r| &r.key == k) {
                    Some(row) => {
                        terms.extend(g.matches(&row.source).into_iter().map(|t| t.term.clone()));
                        items.push(json!(row));
                    }
                    None => items.push(json!({ "key": k, "in_source_catalog": false })),
                }
            }
            let glossary: Vec<_> = g.terms.iter().filter(|t| terms.contains(&t.term)).collect();
            Ok(json!({ "keys": items, "total_keys": total, "glossary": glossary }))
        }.await)
    }

    #[tool(
        description = "Dry-run BEFORE adding a UI string: normalized key, existing keys with the same text (REUSE them), glossary terms in the text, whether the key exists with another text, ICU placeholder problems. Creates nothing."
    )]
    async fn propose_string(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ProposeParams>,
    ) -> CallToolResult {
        done(async {
            let a = auth_of(&parts)?;
            let proj = projects::load(&self.state, a.user.id, &a.slug, Role::Translator).await.map_err(e)?;
            let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1")
                .bind(a.project_id)
                .fetch_all(&self.state.db)
                .await
                .map_err(|x| e(x.into()))?;
            // Same heuristics as the local MCP: an in-memory Workspace holding only the source catalog.
            let mut source = glot_core::catalog::Catalog::new(proj.source_locale.clone());
            source.entries = rows.into_iter().collect();
            let ws = glot_llm::engine::Workspace {
                root: Default::default(),
                config: from(json!({ "source_locale": proj.source_locale, "target_locales": proj.target_locales, "files": [] }))?,
                glossary: glossary::load(&self.state.db, a.project_id).await.map_err(e)?,
                lock: Default::default(),
                catalogs: [(proj.source_locale.clone(), source)].into(),
                visual: Default::default(),
                progress: None,
                batch: false,
            };
            let prop = ws.propose(&p.text, p.suggested_key.as_deref());
            let existing = ws.source().entries.get(&prop.key).cloned();
            let syntax = glot_core::validate::placeholders(&p.text, glot_core::validate::Syntax::Icu);
            let mut advice = Vec::new();
            if let Some((k, _)) = prop.duplicates.first() {
                advice.push(format!("Reuse `{k}` instead of creating a new key (same text already exists)."));
            }
            if existing.as_ref().is_some_and(|x| *x != p.text) {
                advice.push(format!("Key `{}` already exists with a different text: pick another key or add_string with overwrite: true.", prop.key));
            }
            if let Err(i) = &syntax {
                advice.push(format!("Fix the source text first: {}", i.message));
            }
            if advice.is_empty() {
                advice.push(format!("OK: call add_string with key `{}`.", prop.key));
            }
            Ok(json!({
                "key": prop.key, "key_exists": prop.key_exists, "existing_text": existing,
                "duplicates": prop.duplicates.iter().map(|(k, v)| json!({"key": k, "text": v})).collect::<Vec<_>>(),
                "glossary_terms": prop.glossary_terms, "placeholders": syntax.as_ref().ok(), "syntax_issue": syntax.err(),
                "advice": advice,
            }))
        }.await)
    }

    #[tool(
        description = "Add (or with overwrite: true, change) a source string with an optional translator note, after propose_string. Validates ICU syntax; a changed source marks its translations stale. Then call translate."
    )]
    async fn add_string(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<AddStringParams>,
    ) -> CallToolResult {
        done(async {
            let (s, u, slug, a) = self.ctx(&parts)?;
            glot_core::validate::placeholders(&p.text, glot_core::validate::Syntax::Icu)
                .map_err(|i| format!("invalid source text: {} (nothing written)", i.message))?;
            let existing: Option<String> = sqlx::query_scalar("SELECT source FROM keys WHERE project_id = $1 AND key = $2")
                .bind(a.project_id)
                .bind(&p.key)
                .fetch_optional(&self.state.db)
                .await
                .map_err(|x| e(x.into()))?;
            let action = match &existing {
                None => {
                    let body = from(json!({ "key": p.key, "source": p.text, "description": p.note }))?;
                    let _ = keys::create(s, u, slug, AppJson(body)).await.map_err(e)?;
                    "created"
                }
                Some(x) if *x != p.text && !p.overwrite.unwrap_or(false) => {
                    return Err(format!("key `{}` already exists with text {x:?}; reuse it, pick another key, or pass overwrite: true", p.key));
                }
                Some(x) => {
                    let body = from(json!({ "source": p.text, "description": p.note }))?;
                    let _ = keys::update(s, u, AppPath((slug.0, p.key.clone())), AppJson(body)).await.map_err(e)?;
                    if *x == p.text { "unchanged" } else { "updated" }
                }
            };
            Ok(json!({ "key": p.key, "action": action, "next": format!("use `{}` in code, then call translate {{\"keys\": [\"{}\"]}}", p.key, p.key) }))
        }.await)
    }

    #[tool(
        description = "Queue a server-side translation job (missing + stale keys, glossary-guarded, validated; human/reviewed translations are never overwritten) and wait up to wait_seconds for it. Returns the job (status, progress, cost)."
    )]
    async fn translate(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<TranslateParams>,
    ) -> CallToolResult {
        done(async {
            let (s, u, slug, _) = self.ctx(&parts)?;
            let body = from(json!({ "kind": "translate", "locales": p.locales, "keys": p.keys, "force": p.force.unwrap_or(false) }))?;
            let (_, job) = jobs::create(s.clone(), AuthUser(u.0.clone()), slug, AppJson(body)).await.map_err(e)?;
            let mut job = job.0;
            let deadline = Instant::now() + std::time::Duration::from_secs(p.wait_seconds.unwrap_or(20).min(50));
            while matches!(job.status.as_str(), "queued" | "running") && Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                job = jobs::get(s.clone(), AuthUser(u.0.clone()), AppPath(job.id)).await.map_err(e)?.0;
            }
            let next = match job.status.as_str() {
                "done" => "call check to see remaining issues",
                "failed" => "read `error`; fix the project LLM settings in the dashboard",
                _ => "still running: call status later, or translate again with wait_seconds",
            };
            Ok(json!({ "job": job, "next": next }))
        }.await)
    }

    #[tool(
        description = "Validation issues stored on translations (placeholders, ICU syntax, tags, length, glossary). At most 100 per call; page with offset."
    )]
    async fn check(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CheckParams>,
    ) -> CallToolResult {
        done(async {
            let (s, u, slug, _) = self.ctx(&parts)?;
            let all: Vec<_> = keys::issues(s, u, slug).await.map_err(e)?.0
                .into_iter()
                .filter(|i| p.locale.as_ref().is_none_or(|l| *l == i.locale))
                .collect();
            let errors = all.iter().filter(|i| i.issue.severity == glot_core::validate::Severity::Error).count();
            let offset = p.offset.unwrap_or(0);
            let page: Vec<_> = all.iter().skip(offset).take(MAX_ITEMS).collect();
            let next_offset = (offset + page.len() < all.len()).then_some(offset + page.len());
            Ok(json!({ "total": all.len(), "errors": errors, "warnings": all.len() - errors, "next_offset": next_offset, "issues": page }))
        }.await)
    }

    #[tool(
        description = "Set one translation by hand (recorded as human + reviewed, never overwritten by translate). On validation errors nothing is written and the issues are returned."
    )]
    async fn set_translation(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SetTranslationParams>,
    ) -> CallToolResult {
        done(
            async {
                let (s, u, slug, a) = self.ctx(&parts)?;
                let source: String = sqlx::query_scalar(
                    "SELECT source FROM keys WHERE project_id = $1 AND key = $2",
                )
                .bind(a.project_id)
                .bind(&p.key)
                .fetch_optional(&self.state.db)
                .await
                .map_err(|x| e(x.into()))?
                .ok_or_else(|| format!("key `{}` does not exist; add_string first", p.key))?;
                let g = glossary::load(&self.state.db, a.project_id)
                    .await
                    .map_err(e)?;
                let issues = keys::check(&source, &p.value, &p.locale, &g);
                if glot_core::validate::has_errors(&issues) {
                    return Err(format!("not written, validation errors: {}", json!(issues)));
                }
                let t = keys::put_translation(
                    s,
                    u,
                    AppPath((slug.0, p.key.clone(), p.locale.clone())),
                    AppJson(from(json!({ "value": p.value }))?),
                )
                .await
                .map_err(e)?;
                Ok(json!({ "written": true, "key": p.key, "locale": p.locale, "translation": t.0 }))
            }
            .await,
        )
    }

    #[tool(
        description = "Glossary terms (required translations, forbidden variants, do-not-translate names). With `text`, only the terms occurring in it."
    )]
    async fn glossary_lookup(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GlossaryLookupParams>,
    ) -> CallToolResult {
        done(
            async {
                let a = auth_of(&parts)?;
                projects::load(&self.state, a.user.id, &a.slug, Role::Translator)
                    .await
                    .map_err(e)?;
                let g = glossary::load(&self.state.db, a.project_id)
                    .await
                    .map_err(e)?;
                let terms: Vec<_> = match &p.text {
                    Some(t) => g.matches(t),
                    None => g.terms.iter().collect(),
                };
                Ok(json!({ "terms": terms }))
            }
            .await,
        )
    }

    #[tool(
        description = "Add or update a glossary term (enforced on every future translation). If it already has a different translation for a locale, nothing is written and the conflicts are returned unless overwrite: true."
    )]
    async fn glossary_upsert(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GlossaryUpsertParams>,
    ) -> CallToolResult {
        done(async {
            let a = auth_of(&parts)?;
            projects::load(&self.state, a.user.id, &a.slug, Role::Admin).await.map_err(e)?;
            let g = glossary::load(&self.state.db, a.project_id).await.map_err(e)?;
            let existing = g.terms.iter().find(|t| t.term.eq_ignore_ascii_case(&p.term)).cloned();
            let mut term = existing.clone().unwrap_or_else(|| glot_core::glossary::Term { term: p.term.clone(), ..Default::default() });
            let new = p.translations.unwrap_or_default();
            let conflicts: Vec<Value> = new
                .iter()
                .filter_map(|(l, v)| term.translations.get(l).filter(|old| *old != v).map(|old| json!({"locale": l, "existing": old, "proposed": v})))
                .collect();
            if !conflicts.is_empty() && !p.overwrite.unwrap_or(false) {
                return Ok(json!({ "written": false, "conflicts": conflicts, "next": "ask the user which translation to keep, then call again with overwrite: true" }));
            }
            term.translations.extend(new);
            if let Some(d) = p.do_not_translate {
                term.do_not_translate = d;
            }
            if p.description.is_some() {
                term.description = p.description;
            }
            glossary::upsert(&self.state.db, a.project_id, &term).await.map_err(e)?;
            Ok(json!({ "written": true, "action": if existing.is_some() { "updated" } else { "created" }, "term": term }))
        }.await)
    }

    #[tool(
        description = "Back-translation review of translations with the project LLM: verdict (same / nuance / different), explanation and a suggestion. Cached per translation."
    )]
    async fn review(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ReviewParams>,
    ) -> CallToolResult {
        done(async {
            let (s, u, slug, _) = self.ctx(&parts)?;
            let body = from(json!({ "locale": p.locale, "keys": p.keys, "refresh": p.refresh.unwrap_or(false) }))?;
            review::review_many(s, u, slug, AppJson(body)).await.map(|j| j.0).map_err(e)
        }.await)
    }

    #[tool(
        description = "Open a GitHub pull request with the project's current catalogs (source + every target locale, plus glot.lock), using the repository and token set in the project's GitHub settings. Run translate first."
    )]
    async fn translate_repo(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<TranslateRepoParams>,
    ) -> CallToolResult {
        done(
            async {
                let (s, u, slug, a) = self.ctx(&parts)?;
                projects::load(&self.state, a.user.id, &a.slug, Role::Admin)
                    .await
                    .map_err(e)?;
                let pull = sync::pull(s, u, slug).await.map_err(e)?.0;
                open_catalog_pr(&self.state, &a, pull, p.base).await
            }
            .await,
        )
    }
}

// ---------- translate_repo: one commit on a new branch through the Git Data API ----------

async fn gh(
    token: &str,
    method: reqwest::Method,
    url: String,
    body: Option<Value>,
) -> Result<Option<Value>, String> {
    let mut req = reqwest::Client::new()
        .request(method.clone(), &url)
        .bearer_auth(token)
        .header(header::USER_AGENT, "glot")
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28");
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.map_err(|x| format!("GitHub: {x}"))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND && method == reqwest::Method::GET {
        return Ok(None);
    }
    let text = res.text().await.map_err(|x| format!("GitHub: {x}"))?;
    if !status.is_success() {
        return Err(format!(
            "GitHub {status} on {url}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(Some(serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn field(v: &Value, path: &[&str]) -> Result<String, String> {
    path.iter()
        .fold(v, |v, k| &v[*k])
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("unexpected GitHub response (missing {})", path.join(".")))
}

async fn open_catalog_pr(
    state: &AppState,
    a: &McpAuth,
    pull: sync::PullResult,
    base: Option<String>,
) -> Result<Value, String> {
    let row: Option<(Option<String>, String, String, String, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT github_repo, github_base, catalog_path, lock_path, github_token_enc FROM feedback_settings WHERE project_id = $1",
    )
    .bind(a.project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|x| e(x.into()))?;
    let Some((Some(repo), default_base, cat_pattern, lock_path, Some(enc))) = row else {
        return Err("no GitHub repository/token for this project: set them in the dashboard (project settings → GitHub)".into());
    };
    let token = crate::auth::decrypt(&state.secret, &enc)
        .ok_or("cannot decrypt the GitHub token (GLOT_SECRET_KEY changed?)")?;
    let api = format!(
        "{}/repos/{repo}",
        std::env::var("GLOT_GITHUB_API")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
    );
    let base = base.unwrap_or(default_base);
    let get = |url: String| gh(&token, reqwest::Method::GET, url, None);
    let post = |url: String, b: Value| gh(&token, reqwest::Method::POST, url, Some(b));
    let none = || "GitHub returned 404".to_string();

    let r = get(format!("{api}/git/ref/heads/{base}"))
        .await?
        .ok_or_else(|| format!("branch `{base}` not found in {repo}"))?;
    let base_sha = field(&r, &["object", "sha"])?;
    let c = get(format!("{api}/git/commits/{base_sha}"))
        .await?
        .ok_or_else(none)?;
    let base_tree = field(&c, &["tree", "sha"])?;

    use glot_core::catalog::{Catalog, Format};
    let mut tree = Vec::new();
    let mut summary = Vec::new();
    for (locale, entries) in &pull.catalogs {
        let path = cat_pattern.replace("{locale}", locale);
        // Keep the repo's layout (flat vs nested) when the file exists; nested for new files.
        let existing = get(format!("{api}/contents/{path}?ref={base_sha}")).await?;
        let format = existing
            .as_ref()
            .and_then(|v| v["content"].as_str())
            .and_then(|b64| {
                base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    b64.replace('\n', ""),
                )
                .ok()
            })
            .and_then(|b| String::from_utf8(b).ok())
            .map(|t| Catalog::detect_format(&t))
            .filter(|f| *f != Format::Auto)
            .unwrap_or(Format::JsonNested);
        let mut cat = Catalog::new(locale.clone());
        cat.entries = entries.clone();
        tree.push(json!({ "path": path, "mode": "100644", "type": "blob", "content": cat.to_json(format) }));
        summary.push(format!("| {locale} | {} |", entries.len()));
    }
    let mut lock = serde_json::to_string_pretty(&pull.lock).map_err(|x| x.to_string())?;
    lock.push('\n');
    tree.push(json!({ "path": lock_path, "mode": "100644", "type": "blob", "content": lock }));

    let t = post(
        format!("{api}/git/trees"),
        json!({ "base_tree": base_tree, "tree": tree }),
    )
    .await?
    .ok_or_else(none)?;
    let title = format!(
        "glot: update translations ({} locales)",
        pull.catalogs.len()
    );
    let commit = post(
        format!("{api}/git/commits"),
        json!({ "message": title, "tree": field(&t, &["sha"])?, "parents": [base_sha] }),
    )
    .await?
    .ok_or_else(none)?;
    let branch = format!("glot/l10n-{}", &Uuid::new_v4().simple().to_string()[..8]);
    post(
        format!("{api}/git/refs"),
        json!({ "ref": format!("refs/heads/{branch}"), "sha": field(&commit, &["sha"])? }),
    )
    .await?;
    let body = format!(
        "Catalogs exported from glot project `{}` (requested through MCP by {}).\n\n| Locale | Keys |\n|---|---|\n{}\n",
        a.slug,
        a.user.name,
        summary.join("\n")
    );
    let pr = post(
        format!("{api}/pulls"),
        json!({ "title": title, "head": branch, "base": base, "body": body }),
    )
    .await?
    .ok_or_else(none)?;
    Ok(json!({ "pr_url": field(&pr, &["html_url"])?, "branch": branch, "files": tree.len() }))
}

impl ServerHandler for Remote {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("glot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "glot (hosted) manages this project's translations. Before adding UI text: propose_string (reuse duplicates), \
                 add_string, use the key in code, translate, check. get_context explains a key or file. translate_repo opens a \
                 GitHub PR with the catalogs.",
            )
    }

    /// Only the tools the token's scopes allow.
    async fn list_tools(
        &self,
        _r: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let scopes = ctx
            .extensions
            .get::<Parts>()
            .and_then(|p| p.extensions.get::<McpAuth>())
            .map(|a| a.scopes.clone())
            .unwrap_or_default();
        Ok(ListToolsResult::with_all_items(
            self.tool_router
                .list_all()
                .into_iter()
                .filter(|t| scopes.iter().any(|s| s == scope_of(&t.name)))
                .collect(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }

    /// Every call is audited (tool name + outcome + duration, never arguments).
    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let auth = ctx
            .extensions
            .get::<Parts>()
            .and_then(|p| p.extensions.get::<McpAuth>())
            .cloned();
        let tool = req.name.to_string();
        let t0 = Instant::now();
        let res = self
            .tool_router
            .call(ToolCallContext::new(self, req, ctx))
            .await;
        if let Some(a) = auth {
            let ok = matches!(&res, Ok(CallToolResponse::Complete(r)) if r.is_error != Some(true));
            let _ = sqlx::query("INSERT INTO mcp_audit (user_id, project_id, client_id, tool, ok, ms) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(a.user.id)
                .bind(a.project_id)
                .bind(&a.client_id)
                .bind(&tool)
                .bind(ok)
                .bind(t0.elapsed().as_millis().min(i32::MAX as u128) as i32)
                .execute(&self.state.db)
                .await
                .inspect_err(|x| tracing::warn!("mcp audit insert failed: {x}"));
            tracing::info!(user = %a.user.id, project = %a.slug, tool, ok, "mcp tool call");
        }
        res
    }
}
