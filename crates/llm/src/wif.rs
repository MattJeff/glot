//! Anthropic Workload Identity Federation (WIF): keyless Claude access from CI / Kubernetes / clouds.
//!
//! Runtime: exchange the IdP's OIDC JWT (`POST /v1/oauth/token`, RFC 7523 jwt-bearer) for a short-lived
//! `sk-ant-oat01-…` bearer token, cached in memory and re-exchanged before expiry. Same env vars as
//! the official SDKs. Admin: build / validate / create federation issuers and rules (`glot wif`), and
//! decode a JWT locally to compare it with an issuer + rule (`glot wif check`).
//!
//! Never log the assertion (JWT) or the access token.
use base64::Engine as _;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::{Error, Result};

pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";
/// Audience recommended by Anthropic's GitHub Actions WIF guide (also the default the rule expects).
pub const DEFAULT_AUDIENCE: &str = "https://api.anthropic.com";
/// `anthropic-version` used by the Admin API examples for the federation endpoints.
const ADMIN_API_VERSION: &str = "2026-10-21";
/// Re-exchange this long before the access token expires.
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

const REFUSED: &str = "federation token exchange refused (Authentication failed) — see Console → Workload identity → Authentication history; common causes: iss/aud/sub mismatch, expired or replayed JWT (jti)";

fn err(message: impl Into<String>) -> Error {
    Error::Provider {
        provider: "anthropic".into(),
        message: message.into(),
    }
}

/// Where the IdP JWT comes from. Read again on every exchange (rotating tokens, jti single-use).
enum TokenSource {
    /// `ANTHROPIC_IDENTITY_TOKEN_FILE` (projected token, rewritten by the platform).
    File(PathBuf),
    /// `ANTHROPIC_IDENTITY_TOKEN`.
    Literal(String),
    /// GitHub Actions without a token file: mint a fresh OIDC token per exchange.
    GitHub {
        url: String,
        token: String,
        audience: String,
    },
}

pub struct Federation {
    base_url: String,
    rule_id: String,
    organization_id: String,
    service_account_id: String,
    workspace_id: Option<String>,
    source: TokenSource,
    client: reqwest::Client,
    /// (access token, expiry). An async mutex: concurrent batches wait for one exchange, never race two.
    cache: tokio::sync::Mutex<Option<(String, Instant)>>,
}

impl Federation {
    /// Federation from env-like lookups (empty values count as unset). `None` unless the rule,
    /// organization, service account and a token source (file, literal, or GitHub Actions OIDC) are set.
    pub fn from_lookup(base_url: &str, get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let get = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let rule_id = get("ANTHROPIC_FEDERATION_RULE_ID")?;
        let organization_id = get("ANTHROPIC_ORGANIZATION_ID")?;
        let service_account_id = get("ANTHROPIC_SERVICE_ACCOUNT_ID")?;
        let source = if let Some(p) = get("ANTHROPIC_IDENTITY_TOKEN_FILE") {
            TokenSource::File(p.into())
        } else if let Some(t) = get("ANTHROPIC_IDENTITY_TOKEN") {
            TokenSource::Literal(t)
        } else if let (Some(url), Some(token)) = (
            get("ACTIONS_ID_TOKEN_REQUEST_URL"),
            get("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
        ) {
            let audience =
                get("ANTHROPIC_FEDERATION_AUDIENCE").unwrap_or_else(|| DEFAULT_AUDIENCE.into());
            TokenSource::GitHub {
                url,
                token,
                audience,
            }
        } else {
            return None;
        };
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            rule_id,
            organization_id,
            service_account_id,
            workspace_id: get("ANTHROPIC_WORKSPACE_ID"),
            source,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
            cache: tokio::sync::Mutex::new(None),
        })
    }

    /// One federation per base URL for the whole process: several providers (routing fallback,
    /// judge, visual QA…) share the cached access token instead of each re-exchanging a JWT.
    pub fn shared_from_env(base_url: &str) -> Option<Arc<Self>> {
        static SHARED: OnceLock<Mutex<HashMap<String, Arc<Federation>>>> = OnceLock::new();
        let mut map = SHARED.get_or_init(Default::default).lock().ok()?;
        if let Some(f) = map.get(base_url) {
            return Some(f.clone());
        }
        let f = Arc::new(Self::from_lookup(base_url, |k| std::env::var(k).ok())?);
        map.insert(base_url.to_string(), f.clone());
        Some(f)
    }

    /// A valid access token: cached, or freshly exchanged when within 60 s of expiry.
    pub async fn token(&self) -> Result<String> {
        let mut cache = self.cache.lock().await;
        if let Some((t, exp)) = cache.as_ref()
            && Instant::now() + REFRESH_MARGIN < *exp
        {
            return Ok(t.clone());
        }
        let (token, ttl) = self.exchange().await?;
        *cache = Some((token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(token)
    }

    /// Drop the cached token (the API answered 401 with it): the next call exchanges a new JWT.
    pub async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }

    async fn assertion(&self) -> Result<String> {
        match &self.source {
            TokenSource::File(p) => std::fs::read_to_string(p)
                .map(|s| s.trim().to_string())
                .map_err(|e| {
                    err(format!(
                        "ANTHROPIC_IDENTITY_TOKEN_FILE {}: {e}",
                        p.display()
                    ))
                }),
            TokenSource::Literal(t) => Ok(t.clone()),
            TokenSource::GitHub {
                url,
                token,
                audience,
            } => {
                let mut u = reqwest::Url::parse(url)
                    .map_err(|e| err(format!("ACTIONS_ID_TOKEN_REQUEST_URL: {e}")))?;
                u.query_pairs_mut().append_pair("audience", audience);
                let resp = self
                    .client
                    .get(u)
                    .header("authorization", format!("bearer {token}"))
                    .send()
                    .await
                    .map_err(|e| err(format!("GitHub OIDC token request: {e}")))?;
                let status = resp.status();
                let v: Value = resp.json().await.unwrap_or_default();
                match v["value"].as_str() {
                    Some(jwt) if status.is_success() => Ok(jwt.to_string()),
                    _ => Err(err(format!(
                        "GitHub OIDC token request failed (HTTP {status}); does the job have `permissions: id-token: write`?"
                    ))),
                }
            }
        }
    }

    /// One exchange = one fresh assertion; never retried with the same JWT (jti is single-use).
    async fn exchange(&self) -> Result<(String, u64)> {
        let assertion = self.assertion().await?;
        if assertion.is_empty() {
            return Err(err("federation: the identity token is empty"));
        }
        let mut body = json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer",
            "assertion": assertion,
            "federation_rule_id": self.rule_id,
            "organization_id": self.organization_id,
            "service_account_id": self.service_account_id,
        });
        if let Some(ws) = &self.workspace_id {
            body["workspace_id"] = json!(ws);
        }
        let resp = self
            .client
            .post(format!("{}/v1/oauth/token", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| err(format!("federation token exchange: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        match status {
            200..=299 => {
                let token = v["access_token"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| err("federation token exchange: no access_token in response"))?;
                Ok((token.to_string(), v["expires_in"].as_u64().unwrap_or(300)))
            }
            400 => Err(err(format!(
                "federation token exchange rejected (400): {}",
                api_message(&v).unwrap_or(&text)
            ))),
            401 => Err(err(REFUSED)),
            s => Err(err(format!(
                "federation token exchange failed: HTTP {s}: {}",
                api_message(&v).unwrap_or(&text)
            ))),
        }
    }
}

/// `{"error": {"message": …}}` (API shape) or OAuth `error_description` / `error`.
fn api_message(v: &Value) -> Option<&str> {
    v["error"]["message"]
        .as_str()
        .or_else(|| v["error_description"].as_str())
        .or_else(|| v["error"].as_str())
}

// ---------------------------------------------------------------------------------------------
// Admin API: federation issuers and rules (`glot wif issuer|rule create`).

/// Bearer for the federation Admin endpoints (org:admin OAuth only; Admin API keys are refused):
/// `ANTHROPIC_AUTH_TOKEN`, else a federated token from the WIF env (a Console-created org:admin rule).
pub async fn admin_token(base_url: &str, get: impl Fn(&str) -> Option<String>) -> Result<String> {
    let get = |k: &str| get(k).filter(|v| !v.trim().is_empty());
    if let Some(t) = get("ANTHROPIC_AUTH_TOKEN") {
        return Ok(t.trim().to_string());
    }
    if let Some(f) = Federation::from_lookup(base_url, get) {
        return f.token().await;
    }
    let hint = if get("ANTHROPIC_API_KEY").is_some() {
        " (ANTHROPIC_API_KEY is set, but these endpoints do not accept API keys, Admin keys included)"
    } else {
        ""
    };
    Err(Error::Other(format!(
        "no org:admin credential{hint}: export ANTHROPIC_AUTH_TOKEN=$(ant auth print-credentials --profile admin --access-token) after `ant auth login --profile admin --scope org:admin`, or set the WIF env of an org:admin rule"
    )))
}

/// POST `/v1/organizations/{collection}` (federation_issuers | federation_rules); returns the created object.
pub async fn admin_create(
    base_url: &str,
    token: &str,
    collection: &str,
    body: &Value,
) -> Result<Value> {
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/v1/organizations/{collection}",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .header("anthropic-version", ADMIN_API_VERSION)
        .json(body)
        .send()
        .await
        .map_err(|e| err(format!("Admin API: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    if (200..300).contains(&status) {
        return Ok(v);
    }
    let msg = api_message(&v).unwrap_or(&text);
    let hint = match status {
        401 => " (token expired or not an org:admin OAuth token)",
        403 => {
            " (needs the org:admin scope; OAuth callers may only manage workspace:developer / workspace:inference rules)"
        }
        409 => " (a resource with this name already exists)",
        _ => "",
    };
    Err(err(format!("Admin API HTTP {status}: {msg}{hint}")))
}

// ---------------------------------------------------------------------------------------------
// Local validation (same constraints Anthropic enforces, checked before calling).

/// `^[a-z0-9-]+$`, 1–255 chars.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "name {name:?}: must match ^[a-z0-9-]+$ (1-255 chars)"
        ));
    }
    Ok(())
}

/// URL Anthropic will fetch: https, port 443, public DNS hostname (no IP literal, no localhost).
pub fn validate_fetch_url(field: &str, url: &str) -> Result<(), String> {
    let u = reqwest::Url::parse(url).map_err(|e| format!("{field}: invalid URL {url:?}: {e}"))?;
    if u.scheme() != "https" {
        return Err(format!("{field}: url must use https scheme"));
    }
    if u.port_or_known_default() != Some(443) {
        return Err(format!("{field}: url must use port 443"));
    }
    match (u.host_str(), u.domain()) {
        (None, _) => Err(format!("{field}: url has no host")),
        (Some(_), None) => Err(format!(
            "{field}: IP literals are not accepted, use a DNS hostname"
        )),
        (Some(_), Some(d)) if !d.contains('.') || d.eq_ignore_ascii_case("localhost") => {
            Err(format!("{field}: host must be a public DNS name"))
        }
        _ => Ok(()),
    }
}

/// How Anthropic gets the issuer's signing keys.
#[derive(Debug, Clone)]
pub enum Jwks {
    /// `/.well-known/openid-configuration` at `discovery_base` (else the issuer URL).
    Discovery {
        discovery_base: Option<String>,
        ca_cert_pem: Option<String>,
    },
    /// A fixed JWKS endpoint; the issuer URL is only compared with `iss`.
    ExplicitUrl {
        url: String,
        ca_cert_pem: Option<String>,
    },
    /// The JWK objects themselves (the `keys` array, not the wrapper); no fetch, no auto-refresh.
    Inline { keys: Vec<Value> },
}

/// The `keys` array of a JWKS document (`{"keys": [...]}`), or a bare array of JWKs.
pub fn jwks_keys(text: &str) -> Result<Vec<Value>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("JWKS is not JSON: {e}"))?;
    let keys = match v {
        Value::Array(a) => a,
        Value::Object(mut o) => match o.remove("keys") {
            Some(Value::Array(a)) => a,
            _ => return Err("JWKS has no `keys` array".into()),
        },
        _ => return Err("JWKS must be an object with `keys`".into()),
    };
    if keys.is_empty() {
        return Err("JWKS `keys` is empty".into());
    }
    if let Some(k) = keys.iter().find(|k| k.get("d").is_some()) {
        return Err(format!(
            "JWKS key {} contains a private part (`d`): upload public keys only",
            k["kid"]
        ));
    }
    Ok(keys)
}

#[derive(Debug, Clone)]
pub struct IssuerSpec {
    pub name: String,
    pub issuer_url: String,
    pub jwks: Jwks,
    pub check_jti: bool,
    pub max_jwt_lifetime_seconds: Option<u64>,
}

impl IssuerSpec {
    pub fn body(&self) -> Result<Value, String> {
        validate_name(&self.name)?;
        if self.issuer_url.trim().is_empty() {
            return Err("issuer_url is required".into());
        }
        let ca = |c: &Option<String>| -> Result<(), String> {
            match c {
                Some(p) if p.len() > 8192 => Err("ca_cert_pem: at most 8192 bytes".into()),
                Some(p) if !p.contains("-----BEGIN CERTIFICATE-----") => {
                    Err("ca_cert_pem: not a PEM certificate".into())
                }
                _ => Ok(()),
            }
        };
        let jwks = match &self.jwks {
            Jwks::Discovery {
                discovery_base,
                ca_cert_pem,
            } => {
                ca(ca_cert_pem)?;
                match discovery_base {
                    Some(b) => validate_fetch_url("jwks.discovery_base", b)?,
                    None => validate_fetch_url("issuer_url", &self.issuer_url)?,
                }
                let mut j = json!({"type": "discovery"});
                if let Some(b) = discovery_base {
                    j["discovery_base"] = json!(b);
                }
                if let Some(c) = ca_cert_pem {
                    j["ca_cert_pem"] = json!(c);
                }
                j
            }
            Jwks::ExplicitUrl { url, ca_cert_pem } => {
                ca(ca_cert_pem)?;
                validate_fetch_url("jwks.url", url)?;
                let mut j = json!({"type": "explicit_url", "url": url});
                if let Some(c) = ca_cert_pem {
                    j["ca_cert_pem"] = json!(c);
                }
                j
            }
            Jwks::Inline { keys } => {
                if keys.is_empty() {
                    return Err("jwks.keys: at least one key".into());
                }
                json!({"type": "inline", "keys": keys})
            }
        };
        let mut b = json!({"name": self.name, "issuer_url": self.issuer_url, "jwks": jwks});
        if !self.check_jti {
            b["check_jti"] = json!(false);
        }
        if let Some(m) = self.max_jwt_lifetime_seconds {
            if !(1..=176_400).contains(&m) {
                return Err("max_jwt_lifetime_seconds: 1 to 176400".into());
            }
            b["max_jwt_lifetime_seconds"] = json!(m);
        }
        Ok(b)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuleSpec {
    pub name: String,
    pub issuer_id: String,
    pub service_account_id: String,
    pub workspace_id: Option<String>,
    pub all_workspaces: bool,
    pub subject_prefix: Option<String>,
    pub audience: Option<String>,
    pub claims: BTreeMap<String, String>,
    pub condition: Option<String>,
    pub oauth_scope: String,
    pub token_lifetime_seconds: Option<u64>,
}

impl RuleSpec {
    pub fn body(&self) -> Result<Value, String> {
        validate_name(&self.name)?;
        if !self.issuer_id.starts_with("fdis_") {
            return Err("issuer: expected a fdis_… id".into());
        }
        if !self.service_account_id.starts_with("svac_") {
            return Err("service account: expected a svac_… id".into());
        }
        match (&self.workspace_id, self.all_workspaces) {
            (Some(_), true) => return Err("--workspace and --all-workspaces conflict".into()),
            (None, false) => {
                return Err("a rule needs --workspace wrkspc_… or --all-workspaces".into());
            }
            (Some(w), _) if !w.starts_with("wrkspc_") => {
                return Err("workspace: expected a wrkspc_… id".into());
            }
            _ => {}
        }
        if !matches!(
            self.oauth_scope.as_str(),
            "workspace:inference" | "workspace:developer"
        ) {
            return Err(
                "scope: workspace:inference or workspace:developer (other scopes need the Console)"
                    .into(),
            );
        }
        let prefix = self
            .subject_prefix
            .as_deref()
            .filter(|p| !p.trim_end_matches('*').is_empty());
        if prefix.is_none() && self.claims.is_empty() && self.condition.is_none() {
            return Err(
                "match: set --subject-prefix (not just `*`), --claim or --condition (audience alone is not enough)"
                    .into(),
            );
        }
        if let Some(t) = self.token_lifetime_seconds
            && !(60..=86_400).contains(&t)
        {
            return Err("token_lifetime_seconds: 60 to 86400".into());
        }
        let mut m = serde_json::Map::new();
        if let Some(p) = prefix {
            m.insert("subject_prefix".into(), json!(p));
        }
        if let Some(a) = &self.audience {
            m.insert("audience".into(), json!(a));
        }
        if !self.claims.is_empty() {
            m.insert("claims".into(), json!(self.claims));
        }
        if let Some(c) = &self.condition {
            m.insert("condition".into(), json!(c));
        }
        let mut b = json!({
            "name": self.name,
            "issuer_id": self.issuer_id,
            "match": m,
            "target": {"type": "service_account", "service_account_id": self.service_account_id},
            "oauth_scope": self.oauth_scope,
        });
        match &self.workspace_id {
            Some(w) => b["workspace_id"] = json!(w),
            None => b["applies_to_all_workspaces"] = json!(true),
        }
        if let Some(t) = self.token_lifetime_seconds {
            b["token_lifetime_seconds"] = json!(t);
        }
        Ok(b)
    }
}

/// GitHub Actions preset: (subject_prefix, claims) pinned to one repository, optionally one ref.
/// Fork PRs get no `id-token: write`, so `repo:OWNER/REPO:*` only matches this repo's own runs.
pub fn github_match(
    repo: &str,
    git_ref: Option<&str>,
) -> Result<(String, BTreeMap<String, String>), String> {
    let (owner, name) = repo
        .split_once('/')
        .filter(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'))
        .ok_or_else(|| format!("--repo {repo:?}: expected OWNER/REPO"))?;
    let mut claims = BTreeMap::from([("repository_owner".to_string(), owner.to_string())]);
    if let Some(r) = git_ref {
        claims.insert("ref".into(), r.to_string());
    }
    Ok((format!("repo:{owner}/{name}:*"), claims))
}

// ---------------------------------------------------------------------------------------------
// Local JWT check (`glot wif check`): decode without sending, compare with the issuer + rule.

/// (header, claims) of a compact JWT. The signature is not verified (Anthropic does that).
pub fn decode_jwt(jwt: &str) -> Result<(Value, Value), String> {
    let parts: Vec<&str> = jwt.trim().split('.').collect();
    if parts.len() != 3 {
        return Err("not a compact JWT (expected header.payload.signature)".into());
    }
    let part = |i: usize, what: &str| -> Result<Value, String> {
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[i].trim_end_matches('='))
            .map_err(|e| format!("JWT {what}: bad base64url: {e}"))?;
        serde_json::from_slice(&raw).map_err(|e| format!("JWT {what}: not JSON: {e}"))
    };
    Ok((part(0, "header")?, part(1, "payload")?))
}

/// What the JWT must satisfy (from the issuer and the rule).
#[derive(Debug, Clone, Default)]
pub struct Expect {
    pub issuer_url: Option<String>,
    pub audience: Option<String>,
    pub subject_prefix: Option<String>,
    pub claims: BTreeMap<String, String>,
    /// Issuer `max_jwt_lifetime_seconds` (default 3600).
    pub max_lifetime: Option<u64>,
}

/// One line of `glot wif check`: (passed, label, detail).
pub type CheckLine = (bool, String, String);

pub fn check_jwt(jwt: &str, expect: &Expect, now: u64) -> Result<Vec<CheckLine>, String> {
    let (h, c) = decode_jwt(jwt)?;
    let mut out: Vec<CheckLine> = vec![];
    let mut line = |ok: bool, label: &str, detail: String| out.push((ok, label.into(), detail));
    line(
        jwt.trim().len() <= 16 * 1024,
        "size",
        format!("{} bytes (max 16384)", jwt.trim().len()),
    );
    let alg = h["alg"].as_str().unwrap_or("");
    line(
        matches!(
            alg,
            "RS256" | "RS384" | "RS512" | "PS256" | "PS384" | "PS512" | "ES256" | "ES384" | "ES512"
        ),
        "alg",
        format!("{alg:?} (asymmetric RSA/ECDSA only)"),
    );
    line(
        h["kid"].as_str().is_some_and(|k| !k.is_empty()),
        "kid",
        if h["kid"].is_string() {
            format!("{}", h["kid"])
        } else {
            "missing (must match a key in the issuer's JWKS)".into()
        },
    );
    let iss = c["iss"].as_str().unwrap_or("");
    match &expect.issuer_url {
        Some(want) => line(
            iss == want,
            "iss",
            format!("{iss:?} vs issuer_url {want:?} (byte for byte)"),
        ),
        None => line(!iss.is_empty(), "iss", format!("{iss:?}")),
    }
    let auds: Vec<&str> = match &c["aud"] {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
        _ => vec![],
    };
    let want_aud = expect.audience.as_deref().unwrap_or(DEFAULT_AUDIENCE);
    line(
        auds.contains(&want_aud),
        "aud",
        format!("{auds:?} must contain {want_aud:?}"),
    );
    let sub = c["sub"].as_str().unwrap_or("");
    match &expect.subject_prefix {
        Some(p) => {
            let ok = match p.strip_suffix('*') {
                Some(pre) => sub.starts_with(pre),
                None => sub == p,
            };
            line(ok, "sub", format!("{sub:?} vs subject_prefix {p:?}"));
        }
        None => line(!sub.is_empty(), "sub", format!("{sub:?}")),
    }
    for (k, want) in &expect.claims {
        let got = c[k.as_str()].as_str();
        line(
            got == Some(want.as_str()),
            &format!("claims.{k}"),
            format!("{got:?} vs {want:?}"),
        );
    }
    let leeway = 30;
    let iat = c["iat"].as_u64();
    let exp = c["exp"].as_u64();
    line(
        iat.is_some_and(|i| i <= now + leeway),
        "iat",
        match iat {
            Some(i) if i > now + leeway => format!("{i}: in the future (clock skew?)"),
            Some(i) => format!("{i} ({}s ago)", now.saturating_sub(i)),
            None => "missing".into(),
        },
    );
    line(
        exp.is_some_and(|e| e + leeway > now),
        "exp",
        match exp {
            Some(e) if e + leeway <= now => format!("{e}: expired {}s ago", now - e),
            Some(e) => format!("{e} (in {}s)", e.saturating_sub(now)),
            None => "missing".into(),
        },
    );
    if let (Some(i), Some(e)) = (iat, exp) {
        let max = expect.max_lifetime.unwrap_or(3600);
        line(
            e.saturating_sub(i) <= max,
            "lifetime",
            format!("exp - iat = {}s (issuer max {max}s)", e.saturating_sub(i)),
        );
    }
    if let Some(nbf) = c["nbf"].as_u64() {
        line(nbf <= now + leeway, "nbf", format!("{nbf}"));
    }
    if c.get("jti").is_some() {
        line(
            true,
            "jti",
            "present: single-use — this JWT can be exchanged once (issuer check_jti)".into(),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        routing::{get, post},
    };
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Default)]
    struct Mock {
        exchanges: Mutex<Vec<Value>>,
        github_calls: AtomicU32,
        /// Status + body the token endpoint answers.
        reply: Mutex<Option<(u16, Value)>>,
        expires_in: AtomicU32,
        admin: Mutex<Vec<(String, Option<String>, Value)>>,
    }

    async fn server(m: Arc<Mock>) -> String {
        let app = Router::new()
            .route(
                "/v1/oauth/token",
                post(|State(m): State<Arc<Mock>>, Json(b): Json<Value>| async move {
                    let n = {
                        let mut ex = m.exchanges.lock().unwrap();
                        ex.push(b);
                        ex.len()
                    };
                    if let Some((s, body)) = m.reply.lock().unwrap().clone() {
                        return (StatusCode::from_u16(s).unwrap(), Json(body));
                    }
                    let ttl = m.expires_in.load(Ordering::SeqCst);
                    (
                        StatusCode::OK,
                        Json(json!({"access_token": format!("sk-ant-oat01-{n}"), "token_type": "Bearer", "expires_in": ttl, "scope": "workspace:inference"})),
                    )
                }),
            )
            .route(
                "/gh",
                get(|State(m): State<Arc<Mock>>, h: axum::http::HeaderMap, q: axum::extract::Query<BTreeMap<String, String>>| async move {
                    assert_eq!(h["authorization"], "bearer gh-req-token");
                    assert_eq!(q.0["audience"], DEFAULT_AUDIENCE);
                    let n = m.github_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    Json(json!({"value": format!("gh-jwt-{n}")}))
                }),
            )
            .route(
                "/v1/organizations/{c}",
                post(|State(m): State<Arc<Mock>>, axum::extract::Path(c): axum::extract::Path<String>, h: axum::http::HeaderMap, Json(b): Json<Value>| async move {
                    let auth = h.get("authorization").map(|v| v.to_str().unwrap().to_string());
                    m.admin.lock().unwrap().push((c.clone(), auth, b.clone()));
                    if b["name"] == "taken" {
                        return (StatusCode::CONFLICT, Json(json!({"type": "error", "error": {"type": "conflict_error", "message": "name exists"}})));
                    }
                    let id = if c == "federation_issuers" { "fdis_1" } else { "fdrl_1" };
                    (StatusCode::OK, Json(json!({"id": id, "name": b["name"]})))
                }),
            )
            .with_state(m);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        base
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    const BASE_ENV: [(&str, &str); 3] = [
        ("ANTHROPIC_FEDERATION_RULE_ID", "fdrl_r"),
        (
            "ANTHROPIC_ORGANIZATION_ID",
            "00000000-0000-0000-0000-000000000000",
        ),
        ("ANTHROPIC_SERVICE_ACCOUNT_ID", "svac_s"),
    ];

    #[test]
    fn activates_only_with_all_fields_and_a_source() {
        assert!(Federation::from_lookup("http://x", env(&BASE_ENV)).is_none());
        let mut e = BASE_ENV.to_vec();
        e.push(("ANTHROPIC_IDENTITY_TOKEN", "jwt"));
        assert!(Federation::from_lookup("http://x", env(&e)).is_some());
        e.retain(|(k, _)| *k != "ANTHROPIC_ORGANIZATION_ID");
        assert!(Federation::from_lookup("http://x", env(&e)).is_none());
        let mut e = BASE_ENV.to_vec();
        e.push(("ANTHROPIC_IDENTITY_TOKEN_FILE", "")); // empty = unset
        assert!(Federation::from_lookup("http://x", env(&e)).is_none());
    }

    #[tokio::test]
    async fn exchange_body_cache_refresh_and_file_reread() {
        let m = Arc::new(Mock::default());
        m.expires_in.store(3600, Ordering::SeqCst);
        let base = server(m.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("jwt");
        std::fs::write(&file, "jwt-one\n").unwrap();
        let mut e = BASE_ENV.to_vec();
        let path = file.to_str().unwrap().to_string();
        e.push(("ANTHROPIC_IDENTITY_TOKEN_FILE", &path));
        e.push(("ANTHROPIC_WORKSPACE_ID", "wrkspc_w"));
        let f = Arc::new(Federation::from_lookup(&base, env(&e)).unwrap());

        // Parallel callers share one exchange.
        let (a, b) = tokio::join!(f.token(), f.token());
        assert_eq!(a.unwrap(), "sk-ant-oat01-1");
        assert_eq!(b.unwrap(), "sk-ant-oat01-1");
        {
            let ex = m.exchanges.lock().unwrap();
            assert_eq!(ex.len(), 1);
            let body = &ex[0];
            assert_eq!(
                body["grant_type"],
                "urn:ietf:params:oauth:grant-type:jwt-bearer"
            );
            assert_eq!(body["assertion"], "jwt-one");
            assert_eq!(body["federation_rule_id"], "fdrl_r");
            assert_eq!(
                body["organization_id"],
                "00000000-0000-0000-0000-000000000000"
            );
            assert_eq!(body["service_account_id"], "svac_s");
            assert_eq!(body["workspace_id"], "wrkspc_w");
        }
        // Within 60 s of expiry: re-exchange, rereading the (rotated) file.
        f.invalidate().await;
        m.expires_in.store(30, Ordering::SeqCst);
        std::fs::write(&file, "jwt-two").unwrap();
        assert_eq!(f.token().await.unwrap(), "sk-ant-oat01-2");
        std::fs::write(&file, "jwt-three").unwrap();
        assert_eq!(f.token().await.unwrap(), "sk-ant-oat01-3");
        let ex = m.exchanges.lock().unwrap();
        assert_eq!(ex[1]["assertion"], "jwt-two");
        assert_eq!(ex[2]["assertion"], "jwt-three");
        assert!(ex[1].get("workspace_id").is_some());
    }

    #[tokio::test]
    async fn github_actions_fresh_token_per_exchange() {
        let m = Arc::new(Mock::default());
        m.expires_in.store(10, Ordering::SeqCst); // always "about to expire"
        let base = server(m.clone()).await;
        let url = format!("{base}/gh?api-version=2.0");
        let mut e = BASE_ENV.to_vec();
        e.push(("ACTIONS_ID_TOKEN_REQUEST_URL", &url));
        e.push(("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "gh-req-token"));
        let f = Federation::from_lookup(&base, env(&e)).unwrap();
        f.token().await.unwrap();
        f.token().await.unwrap();
        assert_eq!(m.github_calls.load(Ordering::SeqCst), 2);
        let ex = m.exchanges.lock().unwrap();
        assert_eq!(ex[0]["assertion"], "gh-jwt-1");
        assert_eq!(ex[1]["assertion"], "gh-jwt-2");
        assert!(ex[0].get("workspace_id").is_none());
    }

    #[tokio::test]
    async fn exchange_errors_are_mapped_and_not_retried() {
        let m = Arc::new(Mock::default());
        let base = server(m.clone()).await;
        let mut e = BASE_ENV.to_vec();
        e.push(("ANTHROPIC_IDENTITY_TOKEN", "eyJ.secret.jwt"));
        let f = Federation::from_lookup(&base, env(&e)).unwrap();

        *m.reply.lock().unwrap() = Some((
            401,
            json!({"type": "error", "error": {"type": "authentication_error", "message": "Authentication failed"}}),
        ));
        let msg = f.token().await.unwrap_err().to_string();
        assert!(msg.contains("Authentication history"), "{msg}");
        assert!(msg.contains("jti"));
        assert!(!msg.contains("eyJ.secret.jwt"));
        assert_eq!(
            m.exchanges.lock().unwrap().len(),
            1,
            "401 must not resend the JWT"
        );

        *m.reply.lock().unwrap() = Some((
            400,
            json!({"type": "error", "error": {"type": "invalid_request_error", "message": "workspace_id: expected wrkspc_..."}}),
        ));
        let msg = f.token().await.unwrap_err().to_string();
        assert!(
            msg.contains("(400): workspace_id: expected wrkspc_..."),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn admin_credentials_and_create() {
        assert!(
            admin_token(
                "http://x",
                env(&[("ANTHROPIC_API_KEY", "sk-ant-admin01-x")])
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("do not accept API keys")
        );
        let t = admin_token(
            "http://x",
            env(&[("ANTHROPIC_AUTH_TOKEN", "sk-ant-oat01-admin")]),
        )
        .await
        .unwrap();
        assert_eq!(t, "sk-ant-oat01-admin");

        let m = Arc::new(Mock::default());
        let base = server(m.clone()).await;
        let body = IssuerSpec {
            name: "github-actions".into(),
            issuer_url: GITHUB_ISSUER.into(),
            jwks: Jwks::Discovery {
                discovery_base: None,
                ca_cert_pem: None,
            },
            check_jti: true,
            max_jwt_lifetime_seconds: None,
        }
        .body()
        .unwrap();
        let v = admin_create(&base, &t, "federation_issuers", &body)
            .await
            .unwrap();
        assert_eq!(v["id"], "fdis_1");
        let (c, auth, b) = m.admin.lock().unwrap()[0].clone();
        assert_eq!(c, "federation_issuers");
        assert_eq!(auth.as_deref(), Some("Bearer sk-ant-oat01-admin"));
        assert_eq!(b["jwks"], json!({"type": "discovery"}));
        let mut taken = body.clone();
        taken["name"] = json!("taken");
        let e = admin_create(&base, &t, "federation_issuers", &taken)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("409") && e.contains("name exists"), "{e}");
    }

    #[test]
    fn issuer_and_rule_validation() {
        assert!(validate_name("gha-main").is_ok());
        assert!(validate_name("GHA").is_err());
        assert!(validate_fetch_url("u", "https://token.actions.githubusercontent.com").is_ok());
        assert!(
            validate_fetch_url("u", "http://example.com")
                .unwrap_err()
                .contains("https")
        );
        assert!(
            validate_fetch_url("u", "https://example.com:8443")
                .unwrap_err()
                .contains("443")
        );
        assert!(
            validate_fetch_url("u", "https://10.0.0.1")
                .unwrap_err()
                .contains("IP")
        );
        assert!(
            validate_fetch_url("u", "https://[::1]")
                .unwrap_err()
                .contains("IP")
        );

        // explicit_url / inline: the issuer URL is only compared, may be internal.
        let inline = IssuerSpec {
            name: "k8s".into(),
            issuer_url: "https://kubernetes.default.svc".into(),
            jwks: Jwks::Inline {
                keys: jwks_keys(r#"{"keys":[{"kty":"RSA","kid":"a","n":"x","e":"AQAB"}]}"#)
                    .unwrap(),
            },
            check_jti: false,
            max_jwt_lifetime_seconds: Some(7200),
        }
        .body()
        .unwrap();
        assert_eq!(inline["jwks"]["keys"][0]["kid"], "a");
        assert_eq!(inline["check_jti"], false);
        assert_eq!(inline["max_jwt_lifetime_seconds"], 7200);
        assert!(jwks_keys(r#"{"keys":[{"kty":"RSA","d":"secret"}]}"#).is_err());
        let explicit = IssuerSpec {
            name: "x".into(),
            issuer_url: "https://internal.corp".into(),
            jwks: Jwks::ExplicitUrl {
                url: "https://keys.example.com/jwks.json".into(),
                ca_cert_pem: None,
            },
            check_jti: true,
            max_jwt_lifetime_seconds: None,
        };
        assert_eq!(explicit.body().unwrap()["jwks"]["type"], "explicit_url");
        let mut disc = explicit.clone();
        disc.jwks = Jwks::Discovery {
            discovery_base: None,
            ca_cert_pem: Some("nope".into()),
        };
        assert!(disc.body().unwrap_err().contains("PEM"));

        let (prefix, claims) = github_match("acme/web", Some("refs/heads/main")).unwrap();
        let mut r = RuleSpec {
            name: "glot-ci".into(),
            issuer_id: "fdis_1".into(),
            service_account_id: "svac_1".into(),
            workspace_id: Some("wrkspc_1".into()),
            subject_prefix: Some(prefix),
            audience: Some(DEFAULT_AUDIENCE.into()),
            claims,
            oauth_scope: "workspace:inference".into(),
            token_lifetime_seconds: Some(600),
            ..Default::default()
        };
        let b = r.body().unwrap();
        assert_eq!(b["match"]["subject_prefix"], "repo:acme/web:*");
        assert_eq!(b["match"]["claims"]["ref"], "refs/heads/main");
        assert_eq!(b["match"]["claims"]["repository_owner"], "acme");
        assert_eq!(b["target"]["service_account_id"], "svac_1");
        r.token_lifetime_seconds = Some(30);
        assert!(r.body().unwrap_err().contains("60 to 86400"));
        r.token_lifetime_seconds = None;
        r.oauth_scope = "org:admin".into();
        assert!(r.body().is_err());
        r.oauth_scope = "workspace:inference".into();
        r.subject_prefix = Some("*".into());
        r.claims.clear();
        assert!(r.body().unwrap_err().contains("match"));
        assert!(github_match("acme", None).is_err());
    }

    fn jwt(header: Value, claims: Value) -> String {
        let e = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        format!("{}.{}.sig", e(&header), e(&claims))
    }

    #[test]
    fn local_jwt_check() {
        let now = 1_800_000_000;
        let t = jwt(
            json!({"alg": "RS256", "kid": "k1"}),
            json!({"iss": GITHUB_ISSUER, "aud": DEFAULT_AUDIENCE, "sub": "repo:acme/web:pull_request",
                   "repository_owner": "acme", "iat": now - 10, "exp": now + 290, "jti": "j"}),
        );
        let (prefix, claims) = github_match("acme/web", None).unwrap();
        let expect = Expect {
            issuer_url: Some(GITHUB_ISSUER.into()),
            subject_prefix: Some(prefix),
            claims,
            ..Default::default()
        };
        let lines = check_jwt(&t, &expect, now).unwrap();
        assert!(lines.iter().all(|l| l.0), "{lines:?}");
        // Wrong ref claim, expired, HMAC, no kid, long lifetime.
        let bad = jwt(
            json!({"alg": "HS256"}),
            json!({"iss": "https://other", "aud": ["x"], "sub": "repo:evil/web:ref:refs/heads/main",
                   "iat": now - 7300, "exp": now - 100}),
        );
        let failed: Vec<String> = check_jwt(&bad, &expect, now)
            .unwrap()
            .into_iter()
            .filter(|l| !l.0)
            .map(|l| l.1)
            .collect();
        for k in [
            "alg",
            "kid",
            "iss",
            "aud",
            "sub",
            "claims.repository_owner",
            "exp",
            "lifetime",
        ] {
            assert!(failed.iter().any(|f| f == k), "{k} should fail: {failed:?}");
        }
        assert!(check_jwt("nope", &expect, now).is_err());
    }
}
