//! One-click connect and token upkeep: the merchant clicks **Connect**, approves on the platform, and comes
//! back connected. No token to copy, no scope list to read.
//!
//! - **Shopify**, glot's own app (`GLOT_SHOPIFY_CLIENT_ID` / `_SECRET`): authorization code grant with
//!   expiring offline tokens (`expiring=1`: 1 h access token + 90 day rotating refresh token), install callback
//!   verified by HMAC-SHA256 of the sorted query with the client secret, `shop` checked against
//!   `^[a-zA-Z0-9][a-zA-Z0-9\-]*\.myshopify\.com$`.
//!   https://shopify.dev/docs/apps/build/authentication-authorization/access-tokens/authorization-code-grant
//!   https://shopify.dev/docs/apps/build/authentication-authorization/access-tokens/offline-access-tokens
//! - **Shopify**, the merchant's own Dev Dashboard app (credentials `client_id` + `client_secret`, the only
//!   kind a merchant can create since January 1, 2026): client credentials grant, token valid 24 h
//!   (`expires_in: 86399`), renewed before it expires. Legacy `shpat_` tokens (no `expires_at`) are used as is.
//!   https://shopify.dev/docs/apps/build/authentication-authorization/access-tokens/client-credentials-grant
//! - **Shopify** app webhooks (`POST /api/connectors/shopify/webhooks`): `app/uninstalled` and the mandatory
//!   compliance topics (`customers/data_request`, `customers/redact`, `shop/redact`), `401` on a bad HMAC.
//!   https://shopify.dev/docs/apps/build/compliance/privacy-law-compliance
//! - **Webflow** (`GLOT_WEBFLOW_CLIENT_ID` / `_SECRET`): `https://webflow.com/oauth/authorize` →
//!   `POST https://api.webflow.com/oauth/access_token`. https://developers.webflow.com/data/reference/oauth-app
//! - **HubSpot** (`GLOT_HUBSPOT_CLIENT_ID` / `_SECRET`): `https://app.hubspot.com/oauth/authorize` →
//!   `POST https://api.hubapi.com/oauth/v1/token` (form), access token ~30 min, refreshed with the refresh token.
//!   https://developers.hubspot.com/docs/api/working-with-oauth
//! - **WordPress**, no glot credentials at all: core's Application Passwords authorization
//!   (`authentication.application-passwords.endpoints.authorization` of the REST index, usually
//!   `/wp-admin/authorize-application.php`), which sends the user back with `user_login` and `password`.
//!   https://make.wordpress.org/core/2020/11/05/application-passwords-integration-guide/
//! - **WooCommerce**, no glot credentials either: `/wc-auth/v1/authorize`, which POSTs a REST API key to
//!   `callback_url` (HTTPS). https://woocommerce.github.io/woocommerce-rest-api-docs/#rest-api-keys
//!
//! The flow state is an AES-GCM sealed blob (project, user, tenant, expiry): nothing is stored server side,
//! and the browser that finishes the flow must be signed in as the user who started it.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    routing::{get, post},
};
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{Creds, sync::AccountRow};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

/// Kinds with a Connect button. WordPress and WooCommerce need nothing from glot's operator.
pub const KINDS: &[&str] = &["shopify", "webflow", "hubspot", "wordpress", "woocommerce"];

/// Scopes glot's Shopify app asks for: translations and locales to write, the rest to read what is translated.
pub const SHOPIFY_SCOPES: &str = "read_translations,write_translations,read_locales,write_locales,read_products,read_content,read_online_store_pages,read_online_store_navigation,read_themes,read_metaobjects";
const WEBFLOW_SCOPES: &str = "sites:read sites:write pages:read pages:write cms:read cms:write";
/// Stable id of glot in WordPress's application passwords list (any UUID, "honor system").
const WP_APP_ID: &str = "6f1d1c2e-7b0a-5d43-9a57-676c6f740001";
const STATE_MINUTES: i64 = 30;
/// Tokens are renewed this long before they expire.
const REFRESH_MARGIN: i64 = 300;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// glot's OAuth client for `kind`: `GLOT_{KIND}_CLIENT_ID` / `GLOT_{KIND}_CLIENT_SECRET`.
pub fn client(kind: &str) -> Option<(String, String)> {
    let k = kind.to_uppercase();
    Some((
        env(&format!("GLOT_{k}_CLIENT_ID"))?,
        env(&format!("GLOT_{k}_CLIENT_SECRET"))?,
    ))
}

pub fn available(kind: &str) -> bool {
    matches!(kind, "wordpress" | "woocommerce") || (KINDS.contains(&kind) && client(kind).is_some())
}

/// Platform base URL; `GLOT_{KIND}_OAUTH_BASE` replaces it for tests only (`GLOT_CONNECTORS_ALLOW_PRIVATE=1`).
fn base(kind: &str, default: &str) -> String {
    env(&format!("GLOT_{}_OAUTH_BASE", kind.to_uppercase()))
        .filter(|_| super::allow_private())
        .unwrap_or_else(|| default.into())
}

fn web() -> String {
    crate::github_app::web_origin()
}

fn callback_url(kind: &str) -> String {
    format!("{}/api/connectors/oauth/{kind}/callback", web())
}

/// Secret the platform signs its webhooks with: the one the user gave, else the app's client secret (their
/// own Dev Dashboard app, or glot's app for accounts connected by OAuth).
pub fn signing_secret(kind: &str, creds: &Creds) -> Option<String> {
    creds
        .opt("webhook_secret")
        .or_else(|| creds.opt("client_secret"))
        .map(str::to_string)
        .or_else(|| {
            (creds.secrets["oauth"] == true)
                .then(|| client(kind))
                .flatten()
                .map(|c| c.1)
        })
}

// ---------------------------------------------------------------------------------------------- state

#[derive(Serialize, Deserialize)]
struct Flow {
    project: Uuid,
    user: Uuid,
    kind: String,
    /// Shop domain or site URL (empty for platforms that pick the account on their side).
    tenant: String,
    /// Reconnect: the account whose credentials are replaced.
    account: Option<Uuid>,
    exp: i64,
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

fn seal(state: &AppState, f: &Flow) -> String {
    B64.encode(crate::auth::encrypt(
        &state.secret,
        &serde_json::to_string(f).expect("serializable"),
    ))
}

fn open(state: &AppState, kind: &str, sealed: &str) -> Option<Flow> {
    let raw = B64.decode(sealed.trim()).ok()?;
    let f: Flow = serde_json::from_str(&crate::auth::decrypt(&state.secret, &raw)?).ok()?;
    (f.kind == kind && f.exp > Utc::now().timestamp()).then_some(f)
}

// ---------------------------------------------------------------------------------------------- tenants

/// `my-store`, `my-store.myshopify.com`, `https://my-store.myshopify.com/` → `my-store.myshopify.com`.
/// Anything else is refused (the token request carries the client secret).
pub fn shopify_shop(input: &str) -> anyhow::Result<String> {
    let s = input.trim().trim_end_matches('/');
    if super::allow_private() && s.starts_with("http://") {
        return Ok(s.to_string());
    }
    let s = s
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let s = s.split('/').next().unwrap_or(s).to_ascii_lowercase();
    let host = if s.contains('.') {
        s
    } else {
        format!("{s}.myshopify.com")
    };
    let name = host.strip_suffix(".myshopify.com").unwrap_or("");
    anyhow::ensure!(
        name.starts_with(|c: char| c.is_ascii_alphanumeric())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "enter your store's myshopify.com domain (my-store.myshopify.com)"
    );
    Ok(host)
}

fn shop_base(shop: &str) -> String {
    if shop.starts_with("http") {
        shop.to_string()
    } else {
        format!("https://{shop}")
    }
}

fn site_url(input: &str) -> anyhow::Result<String> {
    let s = input.trim().trim_end_matches('/');
    let s = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let u = reqwest::Url::parse(&s).map_err(|_| anyhow::anyhow!("invalid site URL"))?;
    anyhow::ensure!(
        u.scheme() == "https" || super::allow_private(),
        "the site must use https"
    );
    Ok(s)
}

fn with_params(url: &str, params: &[(&str, &str)]) -> anyhow::Result<String> {
    Ok(reqwest::Url::parse_with_params(url, params)
        .map_err(|_| anyhow::anyhow!("invalid URL '{url}'"))?
        .into())
}

// ---------------------------------------------------------------------------------------------- tokens

/// `POST` a token request (form), answer as JSON. Errors never carry the request.
async fn token_request(url: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
    let client = super::client_for(url).await?;
    let res = super::send(|| {
        client
            .post(url)
            .header("accept", "application/json")
            .form(form)
    })
    .await?;
    Ok(res.json().await?)
}

/// Token response → the secrets to store (`token`, and when the platform gives them `refresh_token`,
/// `expires_at`, `refresh_expires_at`, `scope`).
fn token_secrets(v: &Value) -> anyhow::Result<Value> {
    let token = v["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow::anyhow!("the platform returned no access token"))?;
    let now = Utc::now().timestamp();
    let mut out = json!({"token": token});
    if let Some(s) = v["expires_in"].as_i64() {
        out["expires_at"] = json!(now + s);
    }
    if let Some(r) = v["refresh_token"].as_str() {
        out["refresh_token"] = json!(r);
    }
    if let Some(s) = v["refresh_token_expires_in"].as_i64() {
        out["refresh_expires_at"] = json!(now + s);
    }
    if let Some(s) = v["scope"].as_str() {
        out["scope"] = json!(s);
    }
    Ok(out)
}

fn merge(into: &mut Value, from: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), from) {
        a.extend(b);
    }
}

/// A fresh token for secrets that can renew themselves; `None` = static credentials.
async fn renew(kind: &str, secrets: &Value) -> anyhow::Result<Option<Value>> {
    let s = |k: &str| secrets[k].as_str().filter(|v| !v.is_empty());
    let v = match kind {
        "shopify" => {
            let url = format!(
                "{}/admin/oauth/access_token",
                shop_base(&shopify_shop(s("shop").unwrap_or_default())?)
            );
            if let (Some(id), Some(secret)) = (s("client_id"), s("client_secret")) {
                token_request(
                    &url,
                    &[
                        ("grant_type", "client_credentials"),
                        ("client_id", id),
                        ("client_secret", secret),
                    ],
                )
                .await?
            } else if let (Some(rt), Some((id, secret))) = (s("refresh_token"), client(kind)) {
                token_request(
                    &url,
                    &[
                        ("grant_type", "refresh_token"),
                        ("client_id", &id),
                        ("client_secret", &secret),
                        ("refresh_token", rt),
                    ],
                )
                .await?
            } else {
                return Ok(None);
            }
        }
        "hubspot" => {
            let (Some(rt), Some((id, secret))) = (s("refresh_token"), client(kind)) else {
                return Ok(None);
            };
            token_request(
                &format!("{}/oauth/v1/token", base(kind, "https://api.hubapi.com")),
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", &id),
                    ("client_secret", &secret),
                    ("refresh_token", rt),
                ],
            )
            .await?
        }
        _ => return Ok(None),
    };
    token_secrets(&v).map(Some)
}

/// Credentials as posted → credentials ready to use: a Shopify Dev Dashboard app (`client_id` +
/// `client_secret`, no token) gets its first token here.
pub async fn prepare(kind: &str, mut secrets: Value) -> anyhow::Result<Value> {
    let empty = |k: &str| secrets[k].as_str().is_none_or(|v| v.trim().is_empty());
    if kind == "shopify" && empty("token") && !empty("client_id") {
        let fresh = renew(kind, &secrets)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing credential 'client_secret'"))?;
        merge(&mut secrets, fresh);
    }
    Ok(secrets)
}

/// The account's credentials, with the access token renewed (and saved, encrypted) when it expires within
/// five minutes. A refusal by the platform reads `reconnect needed: …` (the dashboard shows Reconnect).
pub async fn fresh(state: &AppState, a: &AccountRow) -> anyhow::Result<Creds> {
    let mut creds = super::sync::creds(state, a)?;
    let Some(exp) = creds.secrets["expires_at"].as_i64() else {
        return Ok(creds);
    };
    if exp > Utc::now().timestamp() + REFRESH_MARGIN {
        return Ok(creds);
    }
    let new = match renew(&a.kind, &creds.secrets).await {
        Ok(Some(n)) => n,
        Ok(None) => anyhow::bail!("reconnect needed: the access token expired"),
        // Refused (revoked, uninstalled, refresh token expired): only the merchant can fix it.
        Err(e) if format!("{e:#}").contains("HTTP 4") => {
            anyhow::bail!("reconnect needed: the platform refused to renew the access ({e:#})")
        }
        Err(e) => return Err(e.context("renewing the access token")),
    };
    merge(&mut creds.secrets, new);
    sqlx::query("UPDATE connector_accounts SET creds_enc = $2 WHERE id = $1")
        .bind(a.id)
        .bind(crate::auth::encrypt(
            &state.secret,
            &creds.secrets.to_string(),
        ))
        .execute(&state.db)
        .await?;
    Ok(creds)
}

// ---------------------------------------------------------------------------------------------- routes

pub fn routes() -> Router<AppState> {
    let base = "/api/projects/{slug}/connectors/oauth";
    Router::new()
        .route(base, get(kinds))
        .route(&format!("{base}/{{kind}}/start"), post(start))
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/connectors/oauth/{kind}/callback", get(callback))
        .route("/api/connectors/oauth/woocommerce/keys", post(woo_keys))
        .route(
            "/api/connectors/shopify/webhooks",
            post(shopify_app_webhook),
        )
}

/// Kinds the dashboard shows a Connect button for (the others, and unconfigured ones, keep the manual form).
async fn kinds(AuthUser(_): AuthUser) -> Json<Vec<&'static str>> {
    Json(KINDS.iter().copied().filter(|k| available(k)).collect())
}

#[derive(Deserialize, Default)]
pub struct StartBody {
    /// Shopify: store domain. WordPress / WooCommerce: site URL.
    #[serde(default)]
    tenant: String,
    #[serde(default)]
    account: Option<Uuid>,
}

fn bad(e: anyhow::Error) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "connector_error", format!("{e:#}"))
}

/// Where to send the browser to approve the connection.
async fn start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, kind)): AppPath<(String, String)>,
    AppJson(b): AppJson<StartBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if !available(&kind) {
        return Err(AppError::bad_request(format!(
            "one-click connect is not configured for '{kind}': use the manual form"
        )));
    }
    let tenant = match kind.as_str() {
        "shopify" => shopify_shop(&b.tenant).map_err(bad)?,
        "wordpress" | "woocommerce" => site_url(&b.tenant).map_err(bad)?,
        _ => String::new(),
    };
    let sealed = seal(
        &state,
        &Flow {
            project: p.id,
            user: user.id,
            kind: kind.clone(),
            tenant: tenant.clone(),
            account: b.account,
            exp: Utc::now().timestamp() + STATE_MINUTES * 60,
        },
    );
    let cb = callback_url(&kind);
    let id = client(&kind).map(|c| c.0).unwrap_or_default();
    let url = match kind.as_str() {
        "shopify" => with_params(
            &format!("{}/admin/oauth/authorize", shop_base(&tenant)),
            &[
                ("client_id", &id),
                ("scope", SHOPIFY_SCOPES),
                ("redirect_uri", &cb),
                ("state", &sealed),
            ],
        ),
        "webflow" => with_params(
            &format!("{}/oauth/authorize", base(&kind, "https://webflow.com")),
            &[
                ("response_type", "code"),
                ("client_id", &id),
                ("scope", WEBFLOW_SCOPES),
                ("redirect_uri", &cb),
                ("state", &sealed),
            ],
        ),
        "hubspot" => with_params(
            &format!("{}/oauth/authorize", base(&kind, "https://app.hubspot.com")),
            &[
                ("client_id", &id),
                ("redirect_uri", &cb),
                ("scope", "content"),
                ("state", &sealed),
            ],
        ),
        "wordpress" => wordpress_authorize(&tenant, &cb, &sealed).await,
        _ => with_params(
            &format!("{tenant}/wc-auth/v1/authorize"),
            &[
                ("app_name", "glot"),
                ("scope", "read_write"),
                ("user_id", &sealed),
                (
                    "return_url",
                    &format!(
                        "{}/app/{}/integrations?connected=woocommerce",
                        web(),
                        p.slug
                    ),
                ),
                (
                    "callback_url",
                    &format!(
                        "{}/api/connectors/oauth/woocommerce/keys",
                        super::api::public_url()
                    ),
                ),
            ],
        ),
    }
    .map_err(bad)?;
    Ok(Json(json!({"url": url})))
}

/// The site's own Application Passwords authorization page, read from its REST index.
async fn wordpress_authorize(site: &str, cb: &str, sealed: &str) -> anyhow::Result<String> {
    let index = format!("{site}/wp-json/");
    let client = super::client_for(&index).await?;
    let v: Value = super::send(|| client.get(&index))
        .await
        .map_err(|e| anyhow::anyhow!("{site} has no reachable WordPress REST API ({e:#})"))?
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("{site} did not answer as a WordPress REST API"))?;
    let auth = v["authentication"]["application-passwords"]["endpoints"]["authorization"]
        .as_str()
        .ok_or_else(|| {
            anyhow::anyhow!("application passwords are disabled on this site (they need HTTPS): use the manual form or the glot plugin")
        })?;
    // The page must be on the site itself: the index is content the site controls.
    let same_host = |a: &str, b: &str| {
        let host = |u: &str| {
            reqwest::Url::parse(u)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
        };
        host(a).is_some() && host(a) == host(b)
    };
    anyhow::ensure!(same_host(auth, site), "unexpected authorization URL");
    with_params(
        auth,
        &[
            ("app_name", "glot"),
            ("app_id", WP_APP_ID),
            ("success_url", &with_params(cb, &[("state", sealed)])?),
        ],
    )
}

fn pairs(raw: Option<String>) -> BTreeMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{}", raw.unwrap_or_default()))
        .map(|u| u.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

/// Shopify's `hmac` over the other query parameters, sorted, `k=v` joined by `&`, hex, client secret as key.
fn shopify_query_ok(q: &BTreeMap<String, String>, secret: &str) -> bool {
    let msg = q
        .iter()
        .filter(|(k, _)| *k != "hmac")
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    q.get("hmac")
        .is_some_and(|h| h.len() == 64 && super::hmac_ok(secret, msg.as_bytes(), h))
}

async fn slug_of(state: &AppState, project: Uuid) -> ApiResult<String> {
    sqlx::query_scalar("SELECT slug FROM projects WHERE id = $1")
        .bind(project)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))
}

/// Code / password → credentials, checked against the platform.
async fn exchange(
    kind: &str,
    f: &Flow,
    q: &BTreeMap<String, String>,
) -> anyhow::Result<(Value, Value)> {
    let get = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if kind == "wordpress" {
        anyhow::ensure!(
            q.get("success").map(String::as_str) != Some("false"),
            "the connection was refused in WordPress"
        );
        let (Some(user), Some(pass)) = (get("user_login"), get("password")) else {
            anyhow::bail!("WordPress returned no application password");
        };
        return Ok((
            json!({"url": f.tenant, "username": user, "app_password": pass}),
            json!({}),
        ));
    }
    if let Some(e) = get("error") {
        anyhow::bail!(
            "the platform refused: {}",
            get("error_description").unwrap_or(e)
        );
    }
    let code = get("code").ok_or_else(|| anyhow::anyhow!("no authorization code"))?;
    let (id, secret) = client(kind).ok_or_else(|| anyhow::anyhow!("not configured"))?;
    let cb = callback_url(kind);
    match kind {
        "shopify" => {
            anyhow::ensure!(
                shopify_query_ok(q, &secret),
                "invalid signature on the Shopify callback"
            );
            let shop = shopify_shop(get("shop").unwrap_or_default())?;
            anyhow::ensure!(
                shop == f.tenant || super::allow_private(),
                "Shopify answered for another store than the one you entered"
            );
            let v = token_request(
                &format!("{}/admin/oauth/access_token", shop_base(&f.tenant)),
                &[
                    ("client_id", &id),
                    ("client_secret", &secret),
                    ("code", code),
                    ("expiring", "1"),
                ],
            )
            .await?;
            let mut s = token_secrets(&v)?;
            merge(&mut s, json!({"shop": f.tenant, "oauth": true}));
            Ok((s, json!({"shop": f.tenant})))
        }
        "webflow" => {
            let v = token_request(
                &format!(
                    "{}/oauth/access_token",
                    base(kind, "https://api.webflow.com")
                ),
                &[
                    ("client_id", &id),
                    ("client_secret", &secret),
                    ("code", code),
                    ("grant_type", "authorization_code"),
                    ("redirect_uri", &cb),
                ],
            )
            .await?;
            let mut s = token_secrets(&v)?;
            merge(&mut s, json!({"oauth": true}));
            Ok((s, json!({})))
        }
        _ => {
            let v = token_request(
                &format!("{}/oauth/v1/token", base(kind, "https://api.hubapi.com")),
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", &id),
                    ("client_secret", &secret),
                    ("redirect_uri", &cb),
                    ("code", code),
                ],
            )
            .await?;
            let mut s = token_secrets(&v)?;
            merge(&mut s, json!({"oauth": true}));
            Ok((s, json!({})))
        }
    }
}

/// Creates the account (first sync held for preview), or replaces the credentials of the one reconnected.
async fn finish(state: &AppState, f: &Flow, secrets: Value, settings: Value) -> ApiResult<()> {
    let slug = slug_of(state, f.project).await?;
    let p = projects::load(state, f.user, &slug, Role::Admin).await?;
    super::api::connect(
        state,
        &p,
        &f.kind,
        None,
        secrets,
        settings,
        Some("translate"),
        f.account,
    )
    .await
    .map(|_| ())
}

/// The platform sends the browser back here. Always answers with a redirect to the Integrations page.
async fn callback(
    State(state): State<AppState>,
    AppPath(kind): AppPath<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> ApiResult<Redirect> {
    let q = pairs(raw);
    let f = q
        .get("state")
        .and_then(|s| open(&state, &kind, s))
        .ok_or_else(|| {
            AppError::bad_request(
                "this connection link expired: start again from the Integrations page",
            )
        })?;
    let slug = slug_of(&state, f.project).await?;
    let page = format!("{}/app/{slug}/integrations", web());
    let fail = |msg: String| {
        Redirect::to(
            &with_params(&page, &[("connect_error", &msg)]).unwrap_or_else(|_| page.clone()),
        )
    };
    let user = crate::auth::authenticate(&headers, &state)
        .await
        .ok()
        .flatten();
    if user.is_none_or(|u| u.id != f.user) {
        return Ok(fail(
            "sign in to glot with the account that started the connection, then retry".into(),
        ));
    }
    let res = match exchange(&kind, &f, &q).await {
        Ok((secrets, settings)) => finish(&state, &f, secrets, settings)
            .await
            .map_err(|e| e.message),
        Err(e) => Err(format!("{e:#}")),
    };
    Ok(match res {
        Ok(()) => Redirect::to(&format!("{page}?connected={kind}")),
        Err(e) => {
            tracing::warn!(kind = %kind, "connector oauth callback failed: {e}");
            fail(e)
        }
    })
}

#[derive(Deserialize)]
pub struct WooKeys {
    /// Our sealed flow state (WooCommerce echoes `user_id`).
    user_id: String,
    consumer_key: String,
    consumer_secret: String,
}

/// WooCommerce posts the generated REST API key here, server to server; the sealed `user_id` authenticates it.
async fn woo_keys(
    State(state): State<AppState>,
    AppJson(b): AppJson<WooKeys>,
) -> ApiResult<StatusCode> {
    let f = open(&state, "woocommerce", &b.user_id).ok_or_else(AppError::unauthorized)?;
    finish(
        &state,
        &f,
        json!({"url": f.tenant, "consumer_key": b.consumer_key, "consumer_secret": b.consumer_secret}),
        json!({}),
    )
    .await?;
    Ok(StatusCode::OK)
}

/// Webhooks of glot's Shopify app itself (declared in `integrations/shopify-app/shopify.app.toml`).
/// glot stores no customer data: the two `customers/*` topics are acknowledged; `shop/redact` deletes the
/// store's connector; `app/uninstalled` drops its tokens and asks for a reconnect.
async fn shopify_app_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let h = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    };
    let ok = client("shopify")
        .is_some_and(|(_, secret)| super::hmac_ok(&secret, &body, h("x-shopify-hmac-sha256")));
    if !ok {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "webhook signature invalid",
        ));
    }
    let shop = h("x-shopify-shop-domain").to_ascii_lowercase();
    match h("x-shopify-topic") {
        "shop/redact" => {
            sqlx::query(
                "DELETE FROM connector_accounts WHERE kind = 'shopify' AND settings->>'shop' = $1",
            )
            .bind(&shop)
            .execute(&state.db)
            .await?;
        }
        "app/uninstalled" => {
            let rows: Vec<AccountRow> = sqlx::query_as(&format!(
                "SELECT {} FROM connector_accounts WHERE kind = 'shopify' AND settings->>'shop' = $1",
                super::sync::COLS
            ))
            .bind(&shop)
            .fetch_all(&state.db)
            .await?;
            for a in rows {
                let mut secrets = super::sync::creds(&state, &a)
                    .map(|c| c.secrets)
                    .unwrap_or_else(|_| json!({}));
                for k in ["token", "refresh_token"] {
                    secrets[k] = json!("");
                }
                secrets["expires_at"] = json!(0);
                sqlx::query(
                    "UPDATE connector_accounts SET creds_enc = $2, status = 'error',
                       last_error = 'reconnect needed: the glot app was uninstalled from this store' WHERE id = $1",
                )
                .bind(a.id)
                .bind(crate::auth::encrypt(&state.secret, &secrets.to_string()))
                .execute(&state.db)
                .await?;
            }
        }
        _ => {}
    }
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_domains() {
        assert_eq!(shopify_shop("My-Store").unwrap(), "my-store.myshopify.com");
        assert_eq!(
            shopify_shop("https://my-store.myshopify.com/admin").unwrap(),
            "my-store.myshopify.com"
        );
        for bad in [
            "evil.com",
            "my-store.myshopify.com.evil.com",
            "-x.myshopify.com",
            "a b",
            "",
        ] {
            assert!(shopify_shop(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn shopify_callback_hmac() {
        use hmac::{Hmac, Mac};
        let mut q: BTreeMap<String, String> = [
            ("code", "c1"),
            ("shop", "a.myshopify.com"),
            ("state", "s"),
            ("timestamp", "1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(b"code=c1&shop=a.myshopify.com&state=s&timestamp=1");
        q.insert("hmac".into(), hex::encode(mac.finalize().into_bytes()));
        assert!(shopify_query_ok(&q, "secret"));
        assert!(!shopify_query_ok(&q, "other"));
        q.insert("shop".into(), "b.myshopify.com".into());
        assert!(!shopify_query_ok(&q, "secret"));
    }

    #[test]
    fn token_fields() {
        let s = token_secrets(&json!({"access_token": "t", "expires_in": 3600, "refresh_token": "r", "refresh_token_expires_in": 7776000, "scope": "a,b"})).unwrap();
        assert_eq!(s["token"], "t");
        assert!(s["expires_at"].as_i64().unwrap() > Utc::now().timestamp() + 3500);
        assert_eq!(s["refresh_token"], "r");
        assert!(token_secrets(&json!({"error": "invalid_request"})).is_err());
    }
}
