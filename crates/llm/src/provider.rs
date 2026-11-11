use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

use glot_core::config::LlmConfig;

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    /// Ask for a JSON object answer.
    pub json: bool,
    /// Optional images (base64 PNG) for vision judging / visual QA.
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    pub model: String,
    /// Uncached input tokens.
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Prompt-cache writes (Anthropic `cache_creation_input_tokens`).
    #[serde(default)]
    pub cache_creation_tokens: u32,
    /// Prompt-cache hits (Anthropic `cache_read_input_tokens`, OpenAI `cached_tokens`).
    #[serde(default)]
    pub cache_read_tokens: u32,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, req: CompletionRequest) -> Result<Completion>;
}

/// Build the provider from config (anthropic | openai | openai-compatible | mock).
/// `api_key` overrides the env var (used by the server, which stores keys per project).
pub fn from_config(cfg: &LlmConfig, api_key: Option<String>) -> Result<Box<dyn Provider>> {
    from_config_with(cfg, api_key, &BTreeMap::new())
}

/// `from_config` + extra non-secret HTTP headers sent on every call (e.g. `anthropic-workspace-id`
/// from the server's per-project settings; `ANTHROPIC_WORKSPACE_ID` stays the fallback).
pub fn from_config_with(
    cfg: &LlmConfig,
    api_key: Option<String>,
    headers: &BTreeMap<String, String>,
) -> Result<Box<dyn Provider>> {
    if cfg.provider == "mock" {
        return Ok(Box::new(crate::mock::MockProvider));
    }
    if cfg.provider == "mock-noisy" {
        return Ok(Box::new(crate::mock::NoisyMock::from_env()));
    }
    Ok(Box::new(http(cfg, api_key, headers)?))
}

/// Message Batches client (`glot translate --batch`, -50 %): Anthropic only.
pub fn batch_client(
    cfg: &LlmConfig,
    api_key: Option<String>,
    headers: &BTreeMap<String, String>,
) -> Result<BatchClient> {
    if cfg.provider != "anthropic" {
        return Err(Error::Other(format!(
            "--batch needs llm.provider = \"anthropic\" (Message Batches API), got \"{}\"",
            cfg.provider
        )));
    }
    Ok(BatchClient(http(cfg, api_key, headers)?))
}

fn http(
    cfg: &LlmConfig,
    api_key: Option<String>,
    headers: &BTreeMap<String, String>,
) -> Result<Http> {
    let env_key = |default: &str| -> (String, Option<String>) {
        let var = cfg
            .api_key_env
            .clone()
            .unwrap_or_else(|| default.to_string());
        let key = api_key
            .as_deref()
            .map(clean_key)
            .filter(|k| !k.is_empty())
            .or_else(|| {
                std::env::var(&var)
                    .ok()
                    .map(|k| clean_key(&k))
                    .filter(|k| !k.is_empty())
            });
        (var, key)
    };
    let mut h = match cfg.provider.as_str() {
        "anthropic" => {
            let (var, key) = env_key("ANTHROPIC_API_KEY");
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            let mut h = Http::new(Kind::Anthropic, base, key);
            anthropic_auth(
                &mut h,
                &var,
                |k| std::env::var(k).ok(),
                crate::wif::Federation::shared_from_env,
            )?;
            h
        }
        "openai" => {
            let (var, key) = env_key("OPENAI_API_KEY");
            let key = key.ok_or(Error::MissingApiKey(var))?;
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            Http::new(Kind::OpenAi, base, Some(key))
        }
        "openai-compatible" => {
            // Local servers (Ollama, LM Studio) need no key.
            let (_, key) = env_key("OPENAI_API_KEY");
            let base = cfg.base_url.clone().ok_or_else(|| {
                Error::Other("llm.base_url is required for provider \"openai-compatible\"".into())
            })?;
            Http::new(Kind::Compatible, base, key)
        }
        "local" => {
            // Ollama / vLLM / LM Studio (OpenAI-compatible). Key only if `api_key_env` names one (vLLM --api-key).
            let (_, key) = env_key("GLOT_LOCAL_API_KEY");
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".into());
            let mut h = Http::new(Kind::Compatible, base, key);
            h.local = true;
            h
        }
        other => {
            return Err(Error::Other(format!(
                "unknown llm provider \"{other}\" (expected anthropic | openai | openai-compatible | local | mock)"
            )));
        }
    };
    h.headers = headers
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    // Org-level user keys (sk-ant-usr-…) must name the workspace to bill. Federated tokens are
    // already workspace-scoped (ANTHROPIC_WORKSPACE_ID went into the exchange instead).
    if h.kind == Kind::Anthropic
        && h.api_key.is_some()
        && !h.headers.iter().any(|(k, _)| k == "anthropic-workspace-id")
        && let Ok(ws) = std::env::var("ANTHROPIC_WORKSPACE_ID")
        && !ws.is_empty()
    {
        h.headers.push(("anthropic-workspace-id".into(), ws));
    }
    if h.kind == Kind::Anthropic
        && h.api_key.as_deref().map(key_kind) == Some(KeyKind::OrgUser)
        && !h.headers.iter().any(|(k, _)| k == "anthropic-workspace-id")
    {
        return Err(Error::Other(WORKSPACE_REQUIRED.into()));
    }
    Ok(h)
}

/// Org-level user key without a workspace: refused before any call (the API would answer 400/401).
pub const WORKSPACE_REQUIRED: &str = "organization key (sk-ant-usr…) needs a workspace ID: set it in Project → Settings → LLM (or ANTHROPIC_WORKSPACE_ID) — clé d'organisation : renseignez l'ID d'espace de travail";

/// Anthropic `anthropic-beta` value OAuth access tokens need on `Authorization: Bearer`.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// What a credential is, by prefix (the key itself is never returned or logged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyKind {
    /// `sk-ant-api…` (and any non-Anthropic key): `x-api-key` / OpenAI Bearer.
    Api,
    /// `sk-ant-oat…` OAuth access token: `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`.
    Oauth,
    /// `sk-ant-usr…` organization user key: `x-api-key` + `anthropic-workspace-id`.
    OrgUser,
}

pub fn key_kind(k: &str) -> KeyKind {
    if k.starts_with("sk-ant-oat") {
        KeyKind::Oauth
    } else if k.starts_with("sk-ant-usr") {
        KeyKind::OrgUser
    } else {
        KeyKind::Api
    }
}

/// Undo copy-paste damage: zero-width chars, whitespace/newlines (anywhere: keys have none),
/// surrounding quotes, a leading `Bearer ` or `NAME=` (`.env` line).
pub fn clean_key(raw: &str) -> String {
    let mut s: &str = raw.trim_matches(|c: char| c.is_whitespace() || is_zero_width(c));
    if let Some((name, rest)) = s.split_once('=')
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        s = rest;
    }
    loop {
        let t = s.trim_matches(|c: char| c.is_whitespace() || is_zero_width(c));
        let t = match t.get(..7) {
            Some(p) if p.eq_ignore_ascii_case("bearer ") => &t[7..],
            _ => t,
        };
        let t = ['"', '\'', '`']
            .iter()
            .find_map(|q| t.strip_prefix(*q).and_then(|x| x.strip_suffix(*q)))
            .unwrap_or(t);
        if t == s {
            break;
        }
        s = t;
    }
    s.chars()
        .filter(|c| !c.is_whitespace() && !is_zero_width(*c))
        .collect()
}

fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}'
    )
}

/// Anthropic credential precedence (as the SDKs): explicit / configured API key, then
/// `ANTHROPIC_AUTH_TOKEN` (Bearer), then Workload Identity Federation env, else missing key.
fn anthropic_auth(
    h: &mut Http,
    var: &str,
    get: impl Fn(&str) -> Option<String>,
    federation: impl FnOnce(&str) -> Option<std::sync::Arc<crate::wif::Federation>>,
) -> Result<()> {
    if h.api_key.as_deref().map(key_kind) == Some(KeyKind::Oauth) {
        // OAuth access token pasted as the "API key": Bearer, never x-api-key.
        h.bearer = h.api_key.take();
        return Ok(());
    }
    if h.api_key.is_some() {
        return Ok(());
    }
    let get = |k: &str| get(k).map(|v| clean_key(&v)).filter(|v| !v.is_empty());
    if let Some(t) = get("ANTHROPIC_AUTH_TOKEN") {
        h.bearer = Some(t);
    } else if let Some(f) = federation(&h.base_url) {
        h.federation = Some(f);
    } else {
        return Err(Error::MissingApiKey(format!(
            "{var} (or ANTHROPIC_AUTH_TOKEN, or Workload Identity Federation: ANTHROPIC_FEDERATION_RULE_ID + ANTHROPIC_ORGANIZATION_ID + ANTHROPIC_SERVICE_ACCOUNT_ID + ANTHROPIC_IDENTITY_TOKEN_FILE)"
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Anthropic,
    OpenAi,
    Compatible,
}

struct Http {
    kind: Kind,
    base_url: String,
    api_key: Option<String>,
    /// Anthropic `ANTHROPIC_AUTH_TOKEN`: `Authorization: Bearer` instead of `x-api-key`.
    bearer: Option<String>,
    /// Anthropic Workload Identity Federation: Bearer token exchanged (and refreshed) per call.
    federation: Option<std::sync::Arc<crate::wif::Federation>>,
    headers: Vec<(String, String)>,
    client: reqwest::Client,
    /// `provider = "local"`: completions are tagged `local:<model>` so they cost $0.
    local: bool,
}

const ATTEMPTS: u32 = 3;

impl Http {
    fn new(kind: Kind, base_url: String, api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest client");
        Self {
            kind,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            bearer: None,
            federation: None,
            headers: vec![],
            client,
            local: false,
        }
    }

    fn err(&self, message: impl Into<String>) -> Error {
        Error::Provider {
            provider: self.name().into(),
            message: message.into(),
        }
    }

    /// Messages API body. The system prompt (rules + glossary, identical for every batch of a locale)
    /// is the stable prefix, marked `cache_control`; the per-batch payload comes after it.
    fn anthropic_body(req: &CompletionRequest, structured: bool) -> Value {
        let mut content: Vec<Value> = req
            .images
            .iter()
            .map(|b64| json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64}}))
            .collect();
        content.push(json!({"type": "text", "text": req.user}));
        // No assistant prefill: rejected by current Claude models. Translation batches get a structured
        // output schema; everything else relies on the prompt + robust parsing.
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "system": [{"type": "text", "text": req.system, "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": content}],
        });
        if structured && let Some(schema) = translations_schema(req) {
            body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
        }
        body
    }

    fn with_headers(&self, mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.kind == Kind::Anthropic {
            rb = rb.header("anthropic-version", "2023-06-01");
            if let Some(k) = &self.api_key {
                rb = rb.header("x-api-key", k);
            } else if let Some(t) = &self.bearer {
                rb = rb.bearer_auth(t);
                if key_kind(t) == KeyKind::Oauth {
                    rb = rb.header("anthropic-beta", OAUTH_BETA);
                }
            } // federation: Bearer added per attempt in `send`
        } else if let Some(k) = &self.api_key {
            rb = rb.bearer_auth(k);
        }
        for (k, v) in &self.headers {
            rb = rb.header(k, v);
        }
        rb
    }

    fn build(&self, req: &CompletionRequest, structured: bool) -> reqwest::RequestBuilder {
        match self.kind {
            Kind::Anthropic => self.with_headers(
                self.client
                    .post(format!("{}/v1/messages", self.base_url))
                    .json(&Self::anthropic_body(req, structured)),
            ),
            Kind::OpenAi | Kind::Compatible => {
                let user: Value = if req.images.is_empty() {
                    json!(req.user)
                } else {
                    let mut parts: Vec<Value> = vec![json!({"type": "text", "text": req.user})];
                    parts.extend(req.images.iter().map(|b64| json!({"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{b64}")}})));
                    Value::Array(parts)
                };
                // System first: OpenAI caches identical prompt prefixes (≥ 1024 tokens) automatically.
                let mut body = json!({
                    "model": req.model,
                    "messages": [{"role": "system", "content": req.system}, {"role": "user", "content": user}],
                });
                // Official API wants max_completion_tokens (reasoning models); compatible servers know max_tokens.
                let field = if self.kind == Kind::OpenAi {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                };
                body[field] = json!(req.max_tokens);
                if let Some(schema) = translations_schema(req).filter(|_| structured) {
                    body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "translations", "strict": true, "schema": schema}});
                } else if req.json {
                    body["response_format"] = json!({"type": "json_object"});
                }
                self.with_headers(
                    self.client
                        .post(format!("{}/chat/completions", self.base_url))
                        .json(&body),
                )
            }
        }
    }

    fn parse(&self, model: &str, v: &Value) -> Result<Completion> {
        let n = |v: &Value| v.as_u64().unwrap_or(0) as u32;
        let (text, input, output, write, read) = match self.kind {
            Kind::Anthropic => {
                if v["stop_reason"] == "refusal" {
                    return Err(self.err("model refused the request"));
                }
                let text = v["content"]
                    .as_array()
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter(|b| b["type"] == "text")
                            .filter_map(|b| b["text"].as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                let u = &v["usage"];
                (
                    text,
                    n(&u["input_tokens"]),
                    n(&u["output_tokens"]),
                    n(&u["cache_creation_input_tokens"]),
                    n(&u["cache_read_input_tokens"]),
                )
            }
            Kind::OpenAi | Kind::Compatible => {
                let text = v["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                // prompt_tokens includes the cached part: split it out.
                let u = &v["usage"];
                let cached = n(&u["prompt_tokens_details"]["cached_tokens"]);
                (
                    text,
                    n(&u["prompt_tokens"]).saturating_sub(cached),
                    n(&u["completion_tokens"]),
                    0,
                    cached,
                )
            }
        };
        let model = v["model"].as_str().unwrap_or(model).to_string();
        let model = if self.local {
            format!("local:{model}")
        } else {
            model
        };
        Ok(Completion {
            text,
            model,
            input_tokens: input,
            output_tokens: output,
            cache_creation_tokens: write,
            cache_read_tokens: read,
        })
    }
}

fn retryable(status: u16) -> bool {
    status == 408 || status == 409 || status == 429 || status >= 500
}

impl Http {
    /// Send with retries on 408/409/429/5xx and transport errors; returns the 2xx body.
    async fn send(&self, mk: impl Fn() -> reqwest::RequestBuilder) -> Result<String> {
        let mut last = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                tracing::warn!(provider = self.name(), attempt, "retrying LLM call: {last}");
            }
            let mut wait = Duration::from_millis(1000 * 2u64.pow(attempt));
            let mut rb = mk();
            if let Some(f) = &self.federation {
                // Exchange errors are final: retrying would resend a single-use JWT.
                rb = rb.bearer_auth(f.token().await?);
            }
            match rb.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 401
                        && let Some(f) = &self.federation
                    {
                        f.invalidate().await; // revoked / expired early: next call re-exchanges
                    }
                    if let Some(s) = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        wait = Duration::from_secs(s.min(60));
                    }
                    let body = resp.text().await.unwrap_or_default();
                    if (200..300).contains(&status) {
                        return Ok(body);
                    } else if retryable(status) {
                        last = format!("HTTP {status}: {body}");
                    } else {
                        return Err(self.err(format!("HTTP {status}: {body}")));
                    }
                }
                Err(e) if e.is_timeout() || e.is_connect() || e.is_request() => {
                    last = e.to_string()
                }
                Err(e) => return Err(self.err(e.to_string())),
            }
            if attempt + 1 < ATTEMPTS {
                tokio::time::sleep(wait).await;
            }
        }
        Err(self.err(format!("giving up after {ATTEMPTS} attempts: {last}")))
    }

    fn json(&self, body: &str) -> Result<Value> {
        serde_json::from_str(body).map_err(|e| self.err(format!("invalid JSON response: {e}")))
    }
}

#[async_trait]
impl Provider for Http {
    fn name(&self) -> &str {
        match self.kind {
            Kind::Anthropic => "anthropic",
            Kind::OpenAi => "openai",
            Kind::Compatible => "openai-compatible",
        }
    }

    async fn complete(&self, req: CompletionRequest) -> Result<Completion> {
        let structured = translations_schema(&req).is_some();
        let body = match self.send(|| self.build(&req, structured)).await {
            // Model/server without structured outputs (older Claude, Ollama, proxies): plain JSON mode.
            Err(Error::Provider { message, .. })
                if structured && message.starts_with("HTTP 400") && schema_rejected(&message) =>
            {
                tracing::warn!(
                    provider = self.name(),
                    "structured output refused, falling back to JSON mode: {message}"
                );
                self.send(|| self.build(&req, false)).await?
            }
            r => r?,
        };
        self.parse(&req.model, &self.json(&body)?)
    }
}

/// Above this many keys a batch is sent without a schema (compiled-grammar size limits).
const MAX_SCHEMA_KEYS: usize = 200;

/// Strict JSON schema `{"translations": {<key>: string, …}}` for a translation batch (keys read from the
/// prompt payload), so Anthropic (`output_config.format`) and OpenAI (`response_format: json_schema`)
/// constrain decoding and parse retries go away. `None` for other calls; the lenient parser stays as fallback.
pub fn translations_schema(req: &CompletionRequest) -> Option<Value> {
    if !req.json {
        return None;
    }
    let payload = crate::translate::extract_payload(&req.user)?;
    if payload["task"] != "translate" {
        return None;
    }
    let keys: Vec<&str> = payload["items"]
        .as_array()?
        .iter()
        .filter_map(|i| i["key"].as_str())
        .collect();
    if keys.is_empty() || keys.len() > MAX_SCHEMA_KEYS {
        return None;
    }
    let props: serde_json::Map<String, Value> = keys
        .iter()
        .map(|k| (k.to_string(), json!({"type": "string"})))
        .collect();
    Some(json!({
        "type": "object",
        "properties": {"translations": {
            "type": "object",
            "properties": props,
            "required": keys,
            "additionalProperties": false,
        }},
        "required": ["translations"],
        "additionalProperties": false,
    }))
}

fn schema_rejected(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "output_config",
        "json_schema",
        "response_format",
        "schema",
        "structured",
    ]
    .iter()
    .any(|w| m.contains(w))
}

/// Result of [`probe`]: the cheapest real check of a credential (never contains the key).
#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub ok: bool,
    /// ok | missing_key | invalid_key | revoked_or_wrong_workspace | workspace_required |
    /// oauth_token_needs_bearer | network | provider_error | config
    pub code: &'static str,
    pub status: Option<u16>,
    pub provider_message: Option<String>,
    pub hint: &'static str,
    pub key_kind: Option<KeyKind>,
}

fn probe_hint(code: &str) -> &'static str {
    match code {
        "ok" => "The provider accepted the key.",
        "missing_key" => "No API key: paste one in Project → Settings → LLM.",
        "invalid_key" => {
            "The provider rejected the key: copy the whole key again (Anthropic: console.anthropic.com → API keys, sk-ant-api…) or create a new one."
        }
        "revoked_or_wrong_workspace" => {
            "The key is not allowed here: it may be revoked or disabled, or belong to another workspace than the one set."
        }
        "workspace_required" => {
            "Organization key (sk-ant-usr…): set the workspace ID (wrkspc_…) in the LLM settings."
        }
        "oauth_token_needs_bearer" => {
            "OAuth token: it is sent as Authorization: Bearer (automatic for sk-ant-oat… tokens); it may have expired, log in again or use an API key."
        }
        "network" => "Could not reach the provider: check the base URL and the network.",
        "config" => "The LLM settings are incomplete: see the message.",
        _ => "The provider answered with an error: see the message.",
    }
}

fn probe_code(status: u16, message: &str, workspace_set: bool) -> &'static str {
    let m = message.to_ascii_lowercase();
    if (200..300).contains(&status) {
        "ok"
    } else if (status == 400 || status == 401 || status == 403) && m.contains("workspace") {
        if workspace_set {
            "revoked_or_wrong_workspace"
        } else {
            "workspace_required"
        }
    } else if status == 401 && (m.contains("oauth") || m.contains("bearer")) {
        "oauth_token_needs_bearer"
    } else if status == 401 {
        "invalid_key"
    } else if status == 403 {
        "revoked_or_wrong_workspace"
    } else {
        "provider_error"
    }
}

/// Check a credential with the cheapest authenticated call (`GET /v1/models`, same auth headers
/// as completions). One attempt, no retries, no tokens spent.
pub async fn probe(
    cfg: &LlmConfig,
    api_key: Option<String>,
    headers: &BTreeMap<String, String>,
) -> Probe {
    let key = api_key.as_deref().map(clean_key).filter(|k| !k.is_empty());
    let done = |code: &'static str, status: Option<u16>, msg: Option<String>| Probe {
        ok: code == "ok",
        code,
        status,
        provider_message: msg.map(|m| {
            let m = match &key {
                Some(k) => m.replace(k.as_str(), "***"),
                None => m,
            };
            m.chars().take(300).collect()
        }),
        hint: probe_hint(code),
        key_kind: key.as_deref().map(key_kind),
    };
    if cfg.provider.starts_with("mock") {
        return done("ok", None, None);
    }
    let h = match http(cfg, key.clone(), headers) {
        Ok(h) => h,
        Err(Error::MissingApiKey(_)) => return done("missing_key", None, None),
        Err(Error::Other(m)) if m == WORKSPACE_REQUIRED => {
            return done("workspace_required", None, Some(m));
        }
        Err(e) => return done("config", None, Some(e.to_string())),
    };
    let workspace_set = h.headers.iter().any(|(k, _)| k == "anthropic-workspace-id");
    let url = match h.kind {
        Kind::Anthropic => format!("{}/v1/models", h.base_url),
        Kind::OpenAi | Kind::Compatible => format!("{}/models", h.base_url),
    };
    let mut rb = h
        .with_headers(h.client.get(url))
        .timeout(Duration::from_secs(15));
    if let Some(f) = &h.federation {
        match f.token().await {
            Ok(t) => rb = rb.bearer_auth(t),
            Err(e) => return done("invalid_key", None, Some(e.to_string())),
        }
    }
    match rb.send().await {
        Err(e) => done("network", None, Some(e.to_string())),
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .or(v["error"].as_str())
                        .map(String::from)
                })
                .unwrap_or(body);
            let code = probe_code(status, &msg, workspace_set);
            done(
                code,
                Some(status),
                Some(msg).filter(|m| code != "ok" && !m.is_empty()),
            )
        }
    }
}

/// Anthropic Message Batches API: submit, poll, fetch results (billed at 50 %).
pub struct BatchClient(Http);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchStatus {
    pub id: String,
    /// in_progress | canceling | ended
    pub processing_status: String,
    /// processing / succeeded / errored / canceled / expired.
    #[serde(default)]
    pub request_counts: BTreeMap<String, u64>,
    pub results_url: Option<String>,
}

impl BatchStatus {
    pub fn ended(&self) -> bool {
        self.processing_status == "ended"
    }
}

impl BatchClient {
    /// `requests`: (custom_id `^[a-zA-Z0-9_-]{1,64}$`, request).
    pub async fn submit(&self, requests: &[(String, CompletionRequest)]) -> Result<BatchStatus> {
        let h = &self.0;
        let body = json!({
            "requests": requests.iter().map(|(id, r)| json!({"custom_id": id, "params": Http::anthropic_body(r, true)})).collect::<Vec<_>>(),
        });
        let url = format!("{}/v1/messages/batches", h.base_url);
        let text = h
            .send(|| h.with_headers(h.client.post(&url).json(&body)))
            .await?;
        serde_json::from_str(&text).map_err(|e| h.err(format!("invalid batch response: {e}")))
    }

    pub async fn status(&self, id: &str) -> Result<BatchStatus> {
        let h = &self.0;
        let url = format!("{}/v1/messages/batches/{id}", h.base_url);
        let text = h.send(|| h.with_headers(h.client.get(&url))).await?;
        serde_json::from_str(&text).map_err(|e| h.err(format!("invalid batch response: {e}")))
    }

    /// custom_id -> completion, for succeeded requests only (errored/expired ones are simply absent:
    /// the pipeline redoes them with a direct call).
    pub async fn results(&self, status: &BatchStatus) -> Result<BTreeMap<String, Completion>> {
        let h = &self.0;
        let url = status
            .results_url
            .clone()
            .unwrap_or_else(|| format!("{}/v1/messages/batches/{}/results", h.base_url, status.id));
        let text = h.send(|| h.with_headers(h.client.get(&url))).await?;
        let mut out = BTreeMap::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v = h.json(line)?;
            let id = v["custom_id"].as_str().unwrap_or_default().to_string();
            let r = &v["result"];
            if r["type"] != "succeeded" {
                tracing::warn!(
                    custom_id = id,
                    "batch request {}: {}",
                    r["type"],
                    r["error"]
                );
                continue;
            }
            let model = r["message"]["model"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            match h.parse(&model, &r["message"]) {
                Ok(c) => {
                    out.insert(id, c);
                }
                Err(e) => tracing::warn!(custom_id = id, "batch request: {e}"),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr_req() -> CompletionRequest {
        CompletionRequest {
            model: "claude-haiku-5-5".into(),
            system: "sys".into(),
            user: "Translate.\n\n<payload>\n{\"task\": \"translate\", \"items\": [{\"key\": \"a.b\", \"source\": \"Hi\"}, {\"key\": \"c\", \"source\": \"Yo\"}]}\n</payload>".into(),
            max_tokens: 100,
            json: true,
            images: vec![],
        }
    }

    #[test]
    fn schema_for_translation_batches_only() {
        let s = translations_schema(&tr_req()).unwrap();
        let t = &s["properties"]["translations"];
        assert_eq!(t["required"], json!(["a.b", "c"]));
        assert_eq!(t["additionalProperties"], false);
        assert_eq!(t["properties"]["a.b"]["type"], "string");
        let mut judge = tr_req();
        judge.user = "<payload>{\"task\": \"judge\", \"items\": []}</payload>".into();
        assert!(translations_schema(&judge).is_none());
        let mut plain = tr_req();
        plain.json = false;
        assert!(translations_schema(&plain).is_none());
    }

    #[test]
    fn request_bodies_carry_schema() {
        let a = Http::anthropic_body(&tr_req(), true);
        assert_eq!(a["output_config"]["format"]["type"], "json_schema");
        assert!(
            Http::anthropic_body(&tr_req(), false)
                .get("output_config")
                .is_none()
        );
        let h = Http::new(Kind::OpenAi, "http://x".into(), Some("k".into()));
        let body: Value = serde_json::from_slice(
            h.build(&tr_req(), true)
                .build()
                .unwrap()
                .body()
                .unwrap()
                .as_bytes()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        let body: Value = serde_json::from_slice(
            h.build(&tr_req(), false)
                .build()
                .unwrap()
                .body()
                .unwrap()
                .as_bytes()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["response_format"]["type"], "json_object");
        assert!(schema_rejected(
            "HTTP 400: output_config.format: unsupported for this model"
        ));
        assert!(!schema_rejected("HTTP 400: max_tokens too large"));
    }

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    const WIF: [(&str, &str); 4] = [
        ("ANTHROPIC_FEDERATION_RULE_ID", "fdrl_r"),
        (
            "ANTHROPIC_ORGANIZATION_ID",
            "00000000-0000-0000-0000-000000000000",
        ),
        ("ANTHROPIC_SERVICE_ACCOUNT_ID", "svac_s"),
        ("ANTHROPIC_IDENTITY_TOKEN", "jwt"),
    ];

    fn anthropic(key: Option<&str>, env: &[(&str, &str)]) -> Result<Http> {
        let mut h = Http::new(Kind::Anthropic, "http://x".into(), key.map(String::from));
        let get = lookup(env);
        let fed = |b: &str| crate::wif::Federation::from_lookup(b, &get).map(std::sync::Arc::new);
        anthropic_auth(&mut h, "ANTHROPIC_API_KEY", &get, fed)?;
        Ok(h)
    }

    #[test]
    fn credential_precedence() {
        let mut env = WIF.to_vec();
        env.push(("ANTHROPIC_AUTH_TOKEN", "tok"));
        // API key wins over everything.
        let h = anthropic(Some("sk-ant-api"), &env).unwrap();
        assert!(h.federation.is_none() && h.bearer.is_none());
        let r = h.build(&tr_req(), false).build().unwrap();
        assert_eq!(r.headers()["x-api-key"], "sk-ant-api");
        assert!(r.headers().get("authorization").is_none());
        // Then ANTHROPIC_AUTH_TOKEN (Bearer, no x-api-key).
        let h = anthropic(None, &env).unwrap();
        let r = h.build(&tr_req(), false).build().unwrap();
        assert_eq!(r.headers()["authorization"], "Bearer tok");
        assert!(r.headers().get("x-api-key").is_none());
        assert_eq!(r.headers()["anthropic-version"], "2023-06-01");
        // Then federation; an empty ANTHROPIC_AUTH_TOKEN does not count.
        let mut env = WIF.to_vec();
        env.push(("ANTHROPIC_AUTH_TOKEN", ""));
        assert!(anthropic(None, &env).unwrap().federation.is_some());
        // Nothing: missing key, naming the federation option.
        let e = anthropic(None, &WIF[..3]).err().unwrap().to_string();
        assert!(e.contains("ANTHROPIC_FEDERATION_RULE_ID"), "{e}");
    }

    #[test]
    fn keys_are_cleaned() {
        let k = "sk-ant-api03-abc";
        for raw in [
            "sk-ant-api03-abc\n",
            "  sk-ant-api03-abc  ",
            "\"sk-ant-api03-abc\"",
            "'sk-ant-api03-abc'\r\n",
            "Bearer sk-ant-api03-abc",
            "bearer  \"sk-ant-api03-abc\"",
            "\u{200B}sk-ant-api03-abc\u{FEFF}",
            "sk-ant-api03-\nabc",
            "ANTHROPIC_API_KEY=sk-ant-api03-abc",
            "ANTHROPIC_API_KEY=\"sk-ant-api03-abc\"\n",
        ] {
            assert_eq!(clean_key(raw), k, "{raw:?}");
        }
        assert_eq!(clean_key(" \n "), "");
    }

    #[test]
    fn auth_headers_by_key_prefix() {
        let headers = |key: &str| {
            anthropic(Some(&clean_key(key)), &[])
                .unwrap()
                .build(&tr_req(), false)
                .build()
                .unwrap()
                .headers()
                .clone()
        };
        assert_eq!(key_kind("sk-ant-api03-x"), KeyKind::Api);
        let h = headers("sk-ant-api03-x\n");
        assert_eq!(h["x-api-key"], "sk-ant-api03-x");
        assert!(h.get("authorization").is_none() && h.get("anthropic-beta").is_none());
        // OAuth access token: Bearer + oauth beta, never x-api-key.
        assert_eq!(key_kind("sk-ant-oat01-x"), KeyKind::Oauth);
        let h = headers(" sk-ant-oat01-x ");
        assert_eq!(h["authorization"], "Bearer sk-ant-oat01-x");
        assert_eq!(h["anthropic-beta"], OAUTH_BETA);
        assert!(h.get("x-api-key").is_none());
        // Org user key: x-api-key, and refused up front without a workspace.
        assert_eq!(key_kind("sk-ant-usr01-x"), KeyKind::OrgUser);
        let cfg = LlmConfig {
            provider: "anthropic".into(),
            base_url: Some("http://x".into()),
            ..Default::default()
        };
        if std::env::var_os("ANTHROPIC_WORKSPACE_ID").is_none() {
            let e = http(&cfg, Some("sk-ant-usr01-x".into()), &BTreeMap::new())
                .err()
                .unwrap()
                .to_string();
            assert!(e.contains("clé d'organisation"), "{e}");
        }
        let ws = BTreeMap::from([("anthropic-workspace-id".into(), "wrkspc_1".into())]);
        let r = http(&cfg, Some("sk-ant-usr01-x".into()), &ws)
            .unwrap()
            .build(&tr_req(), false)
            .build()
            .unwrap();
        assert_eq!(r.headers()["x-api-key"], "sk-ant-usr01-x");
        assert_eq!(r.headers()["anthropic-workspace-id"], "wrkspc_1");
        assert_eq!(probe_code(401, "invalid x-api-key", false), "invalid_key");
        assert_eq!(
            probe_code(400, "workspace_id is required", false),
            "workspace_required"
        );
        assert_eq!(probe_code(403, "", true), "revoked_or_wrong_workspace");
    }

    #[tokio::test]
    async fn federated_bearer_on_messages() {
        use axum::{Json, Router, http::HeaderMap, routing::post};
        type Seen = std::sync::Arc<std::sync::Mutex<Vec<(Option<String>, bool)>>>;
        let seen: Seen = Default::default();
        let s2 = seen.clone();
        let app = Router::new()
            .route(
                "/v1/oauth/token",
                post(|| async {
                    Json(json!({"access_token": "sk-ant-oat01-x", "token_type": "Bearer", "expires_in": 3600}))
                }),
            )
            .route(
                "/v1/messages",
                post(move |h: HeaderMap| {
                    let s = s2.clone();
                    async move {
                        s.lock().unwrap().push((
                            h.get("authorization").map(|v| v.to_str().unwrap().to_string()),
                            h.contains_key("x-api-key") || h.contains_key("anthropic-workspace-id"),
                        ));
                        Json(json!({"model": "m", "content": [{"type": "text", "text": "hi"}], "usage": {}}))
                    }
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let mut h = Http::new(Kind::Anthropic, base, None);
        let mut env = WIF.to_vec();
        env.push(("ANTHROPIC_WORKSPACE_ID", "wrkspc_w"));
        let get = lookup(&env);
        anthropic_auth(&mut h, "ANTHROPIC_API_KEY", &get, |b| {
            crate::wif::Federation::from_lookup(b, &get).map(std::sync::Arc::new)
        })
        .unwrap();
        let mut req = tr_req();
        req.json = false;
        assert_eq!(h.complete(req.clone()).await.unwrap().text, "hi");
        h.complete(req).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        for (auth, extra) in seen.iter() {
            assert_eq!(auth.as_deref(), Some("Bearer sk-ant-oat01-x"));
            assert!(
                !extra,
                "no x-api-key / workspace header with a federated token"
            );
        }
    }
}
