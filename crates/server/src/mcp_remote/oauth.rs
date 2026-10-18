//! OAuth 2.1 authorization server for the remote MCP endpoint (MCP spec « Authorization »):
//! RFC 9728 protected-resource metadata, RFC 8414 server metadata, RFC 7591 Dynamic Client Registration
//! (public clients only), authorization code + PKCE S256, RFC 8707 `resource`, RFC 9207 `iss`,
//! rotating refresh tokens, RFC 7009 revocation. Consent happens in the dashboard (`web/app/[locale]/oauth`).
use axum::{
    Form, Json, Router,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::Engine;
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppQuery, AppState,
    auth::{AuthUser, sha256_hex},
    projects::{self, Role},
};

pub const SCOPES: [&str; 3] = ["read", "translate", "write"];
const ACCESS_SECS: i64 = 3600;
const REFRESH_DAYS: i64 = 30;

/// Public base URL of this server: `GLOT_PUBLIC_URL`, else derived from the request (dev / tests).
pub fn base_url(headers: &HeaderMap) -> String {
    if let Ok(u) = std::env::var("GLOT_PUBLIC_URL") {
        return u.trim_end_matches('/').to_string();
    }
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    let host = h("x-forwarded-host")
        .or(h(header::HOST.as_str()))
        .unwrap_or("localhost:8790");
    format!("{}://{host}", h("x-forwarded-proto").unwrap_or("http"))
}

pub fn resource_metadata_url(headers: &HeaderMap) -> String {
    format!("{}/.well-known/oauth-protected-resource", base_url(headers))
}

pub fn routes(web_origin: String) -> Router<AppState> {
    let public = Router::new()
        .route("/.well-known/oauth-protected-resource", get(resource_meta))
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_meta),
        )
        .route("/.well-known/oauth-authorization-server", get(server_meta))
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(server_meta),
        )
        .route("/oauth/register", post(register))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        // Browser-based MCP clients discover + exchange tokens cross-origin; nothing here reads cookies.
        .layer(tower_http::cors::CorsLayer::permissive());
    Router::new()
        .merge(public)
        .route(
            "/oauth/authorize",
            get(
                move |State(state): State<AppState>,
                      RawQuery(q): RawQuery,
                      AppQuery(p): AppQuery<AuthzParams>| {
                    authorize_redirect(state, web_origin.clone(), q, p)
                },
            ),
        )
        // Consent screen API (session cookie, same origin through the Next.js /api rewrite).
        .route("/api/oauth/authorize", get(consent_info).post(consent))
}

fn oauth_err(status: StatusCode, error: &str, desc: impl Into<String>) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": error, "error_description": desc.into() })),
    )
        .into_response()
}

async fn resource_meta(headers: HeaderMap) -> Json<Value> {
    let base = base_url(&headers);
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "scopes_supported": SCOPES,
        "bearer_methods_supported": ["header"],
        "resource_name": "glot",
        "resource_documentation": "https://github.com/MattJeff/glot/blob/main/web/content/docs/mcp-remote.md",
    }))
}

async fn server_meta(headers: HeaderMap) -> Json<Value> {
    let base = base_url(&headers);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "scopes_supported": SCOPES,
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "authorization_response_iss_parameter_supported": true,
    }))
}

// ---------- Dynamic Client Registration ----------

/// https, loopback http, or a private-use scheme (`cursor://…`, RFC 8252 §7.1). Never a fragment.
fn valid_redirect(u: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(u) else {
        return false;
    };
    if url.fragment().is_some() || u.len() > 2000 {
        return false;
    }
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        "javascript" | "data" | "file" | "vbscript" | "blob" | "about" | "ftp" | "ws" | "wss" => {
            false
        }
        _ => true,
    }
}

#[derive(Deserialize)]
struct RegisterBody {
    #[serde(default)]
    redirect_uris: Vec<String>,
    client_name: Option<String>,
}

// ponytail: open registration (RFC 7591) without rate limit or GC; add both if the table grows from abuse.
async fn register(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let Ok(b) = serde_json::from_slice::<RegisterBody>(&body) else {
        return oauth_err(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "invalid JSON body",
        );
    };
    if b.redirect_uris.is_empty()
        || b.redirect_uris.len() > 10
        || !b.redirect_uris.iter().all(|u| valid_redirect(u))
    {
        return oauth_err(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris: 1-10 https, loopback http or private-use scheme URIs without fragment",
        );
    }
    let name: String = b
        .client_name
        .unwrap_or_else(|| "MCP client".into())
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect();
    let id = format!("glot_client_{}", Uuid::new_v4().simple());
    let res =
        sqlx::query("INSERT INTO oauth_clients (id, name, redirect_uris) VALUES ($1, $2, $3)")
            .bind(&id)
            .bind(&name)
            .bind(&b.redirect_uris)
            .execute(&state.db)
            .await;
    if let Err(e) = res {
        return AppError::from(e).into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": id, "client_name": name, "redirect_uris": b.redirect_uris,
            "client_id_issued_at": Utc::now().timestamp(),
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
            "token_endpoint_auth_method": "none", "scope": SCOPES.join(" "),
        })),
    )
        .into_response()
}

// ---------- authorization ----------

#[derive(Deserialize, Clone)]
pub struct AuthzParams {
    response_type: Option<String>,
    client_id: String,
    redirect_uri: String,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

struct Validated {
    client_name: String,
    scopes: Vec<String>,
}

/// Errors here are shown, never redirected: the redirect_uri is not trusted yet.
async fn validate(state: &AppState, p: &AuthzParams) -> ApiResult<Validated> {
    let row: Option<(String, Vec<String>)> =
        sqlx::query_as("SELECT name, redirect_uris FROM oauth_clients WHERE id = $1")
            .bind(&p.client_id)
            .fetch_optional(&state.db)
            .await?;
    let Some((client_name, uris)) = row else {
        return Err(AppError::bad_request(
            "unknown client_id (register the client first)",
        ));
    };
    if !uris.contains(&p.redirect_uri) {
        return Err(AppError::bad_request(
            "redirect_uri is not registered for this client",
        ));
    }
    if p.response_type.as_deref() != Some("code") {
        return Err(AppError::bad_request("response_type must be 'code'"));
    }
    let ch = p.code_challenge.as_deref().unwrap_or("");
    if p.code_challenge_method.as_deref() != Some("S256") || !(43..=128).contains(&ch.len()) {
        return Err(AppError::bad_request(
            "PKCE required: code_challenge with code_challenge_method=S256",
        ));
    }
    let requested: Vec<String> = p
        .scope
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .filter(|s| SCOPES.contains(s))
        .map(String::from)
        .collect();
    Ok(Validated {
        client_name,
        scopes: if requested.is_empty() {
            SCOPES.map(String::from).to_vec()
        } else {
            requested
        },
    })
}

async fn authorize_redirect(
    state: AppState,
    web_origin: String,
    q: Option<String>,
    p: AuthzParams,
) -> ApiResult<Redirect> {
    validate(&state, &p).await?;
    let web = web_origin.trim_end_matches('/');
    Ok(Redirect::to(&format!(
        "{web}/oauth/authorize?{}",
        q.unwrap_or_default()
    )))
}

async fn consent_info(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AppQuery(p): AppQuery<AuthzParams>,
) -> ApiResult<Json<Value>> {
    let v = validate(&state, &p).await?;
    let host = reqwest::Url::parse(&p.redirect_uri).ok().and_then(|u| {
        u.host_str()
            .map(String::from)
            .or(Some(u.scheme().to_string()))
    });
    Ok(Json(
        json!({ "client_name": v.client_name, "redirect_host": host, "scopes": v.scopes }),
    ))
}

#[derive(Deserialize)]
pub struct ConsentBody {
    #[serde(flatten)]
    params: AuthzParams,
    approve: bool,
    project: Option<String>,
    /// Subset of the requested scopes the user kept checked.
    #[serde(default)]
    scopes: Vec<String>,
}

fn redirect_with(uri: &str, pairs: &[(&str, &str)]) -> String {
    let mut u = reqwest::Url::parse(uri).expect("validated at registration");
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
    }
    u.to_string()
}

async fn consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<ConsentBody>,
) -> ApiResult<Json<Value>> {
    let p = b.params;
    let v = validate(&state, &p).await?;
    let iss = base_url(&headers);
    let st = p.state.clone().unwrap_or_default();
    let mut pairs = vec![("iss", iss.as_str())];
    if p.state.is_some() {
        pairs.push(("state", st.as_str()));
    }
    if !b.approve {
        pairs.push(("error", "access_denied"));
        return Ok(Json(
            json!({ "redirect": redirect_with(&p.redirect_uri, &pairs) }),
        ));
    }
    let slug = b
        .project
        .ok_or_else(|| AppError::bad_request("choose a project"))?;
    let project = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let scopes: Vec<String> = v
        .scopes
        .into_iter()
        .filter(|s| b.scopes.contains(s))
        .collect();
    if scopes.is_empty() {
        return Err(AppError::bad_request("grant at least one scope"));
    }
    let code = hex::encode(rand::random::<[u8; 32]>());
    sqlx::query(
        "INSERT INTO oauth_codes (code_hash, client_id, user_id, project_id, scopes, redirect_uri, code_challenge, resource, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now() + interval '5 minutes')",
    )
    .bind(sha256_hex(&code))
    .bind(&p.client_id)
    .bind(user.id)
    .bind(project.id)
    .bind(&scopes)
    .bind(&p.redirect_uri)
    .bind(p.code_challenge.unwrap_or_default())
    .bind(&p.resource)
    .execute(&state.db)
    .await?;
    tracing::info!(user = %user.id, project = %slug, client = %p.client_id, "oauth consent");
    pairs.push(("code", code.as_str()));
    Ok(Json(
        json!({ "redirect": redirect_with(&p.redirect_uri, &pairs) }),
    ))
}

// ---------- token + revocation ----------

#[derive(Deserialize)]
struct TokenForm {
    grant_type: String,
    code: Option<String>,
    code_verifier: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
}

pub fn pkce_s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn token_response(access: &str, refresh: &str, scopes: &[String]) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "access_token": access, "token_type": "Bearer", "expires_in": ACCESS_SECS,
            "refresh_token": refresh, "scope": scopes.join(" "),
        })),
    )
        .into_response()
}

fn new_pair() -> (String, String) {
    let r = || hex::encode(rand::random::<[u8; 32]>());
    (format!("glot_at_{}", r()), format!("glot_rt_{}", r()))
}

async fn token(State(state): State<AppState>, Form(f): Form<TokenForm>) -> Response {
    match token_inner(&state, f).await {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}

async fn token_inner(state: &AppState, f: TokenForm) -> ApiResult<Response> {
    let bad = |e: &str, d: &str| Ok(oauth_err(StatusCode::BAD_REQUEST, e, d));
    let (access, refresh) = new_pair();
    let access_exp = Utc::now() + Duration::seconds(ACCESS_SECS);
    let refresh_exp = Utc::now() + Duration::days(REFRESH_DAYS);
    match f.grant_type.as_str() {
        "authorization_code" => {
            let (Some(code), Some(verifier)) = (f.code, f.code_verifier) else {
                return bad("invalid_request", "code and code_verifier are required");
            };
            // Single use: deleted whether or not the rest checks out.
            let row: Option<(String, Uuid, Uuid, Vec<String>, String, String, Option<String>, bool)> = sqlx::query_as(
                "DELETE FROM oauth_codes WHERE code_hash = $1
                 RETURNING client_id, user_id, project_id, scopes, redirect_uri, code_challenge, resource, expires_at > now()",
            )
            .bind(sha256_hex(&code))
            .fetch_optional(&state.db)
            .await?;
            let Some((
                client_id,
                user_id,
                project_id,
                scopes,
                redirect_uri,
                challenge,
                resource,
                fresh,
            )) = row
            else {
                return bad("invalid_grant", "unknown or already used code");
            };
            if !fresh
                || f.client_id.as_deref() != Some(client_id.as_str())
                || f.redirect_uri.as_deref().is_some_and(|r| r != redirect_uri)
                || pkce_s256(&verifier) != challenge
                || f.resource.is_some() && resource.is_some() && f.resource != resource
            {
                return bad(
                    "invalid_grant",
                    "code expired, or client_id / redirect_uri / code_verifier / resource mismatch",
                );
            }
            sqlx::query(
                "INSERT INTO oauth_grants (id, client_id, user_id, project_id, scopes, resource, access_hash, access_expires_at, refresh_hash, refresh_expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(Uuid::new_v4())
            .bind(&client_id)
            .bind(user_id)
            .bind(project_id)
            .bind(&scopes)
            .bind(resource.or(f.resource))
            .bind(sha256_hex(&access))
            .bind(access_exp)
            .bind(sha256_hex(&refresh))
            .bind(refresh_exp)
            .execute(&state.db)
            .await?;
            Ok(token_response(&access, &refresh, &scopes))
        }
        "refresh_token" => {
            let Some(rt) = f.refresh_token else {
                return bad("invalid_request", "refresh_token is required");
            };
            // Rotation: the old refresh token stops working as soon as it is used.
            let scopes: Option<Vec<String>> = sqlx::query_scalar(
                "UPDATE oauth_grants SET access_hash = $3, access_expires_at = $4, refresh_hash = $5, refresh_expires_at = $6
                 WHERE refresh_hash = $1 AND refresh_expires_at > now() AND ($2::text IS NULL OR client_id = $2)
                 RETURNING scopes",
            )
            .bind(sha256_hex(&rt))
            .bind(&f.client_id)
            .bind(sha256_hex(&access))
            .bind(access_exp)
            .bind(sha256_hex(&refresh))
            .bind(refresh_exp)
            .fetch_optional(&state.db)
            .await?;
            match scopes {
                Some(s) => Ok(token_response(&access, &refresh, &s)),
                None => bad("invalid_grant", "unknown, expired or revoked refresh token"),
            }
        }
        _ => bad(
            "unsupported_grant_type",
            "use authorization_code or refresh_token",
        ),
    }
}

#[derive(Deserialize)]
struct RevokeForm {
    token: String,
}

/// RFC 7009: revoking either token of a grant drops the whole grant; unknown tokens still get 200.
async fn revoke(State(state): State<AppState>, Form(f): Form<RevokeForm>) -> ApiResult<StatusCode> {
    let h = sha256_hex(f.token.trim());
    sqlx::query("DELETE FROM oauth_grants WHERE access_hash = $1 OR refresh_hash = $1")
        .bind(h)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pkce_and_redirects() {
        // RFC 7636 appendix B.
        assert_eq!(
            super::pkce_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        for ok in [
            "https://claude.ai/api/mcp/auth_callback",
            "http://localhost:3456/cb",
            "http://127.0.0.1:9/x",
            "cursor://anysphere.cursor-mcp/oauth/callback",
        ] {
            assert!(super::valid_redirect(ok), "{ok}");
        }
        for bad in [
            "http://evil.com/cb",
            "javascript:alert(1)",
            "https://a.com/#x",
            "data:text/html,x",
            "nope",
        ] {
            assert!(!super::valid_redirect(bad), "{bad}");
        }
    }
}
