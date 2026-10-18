use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, req: CompletionRequest) -> Result<Completion>;
}

/// Build the provider from config (anthropic | openai | openai-compatible | mock).
/// `api_key` overrides the env var (used by the server, which stores keys per project).
pub fn from_config(cfg: &LlmConfig, api_key: Option<String>) -> Result<Box<dyn Provider>> {
    let env_key = |default: &str| -> (String, Option<String>) {
        let var = cfg
            .api_key_env
            .clone()
            .unwrap_or_else(|| default.to_string());
        let key = api_key
            .clone()
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var(&var).ok().filter(|k| !k.is_empty()));
        (var, key)
    };
    match cfg.provider.as_str() {
        "mock" => Ok(Box::new(crate::mock::MockProvider)),
        "anthropic" => {
            let (var, key) = env_key("ANTHROPIC_API_KEY");
            let key = key.ok_or(Error::MissingApiKey(var))?;
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            Ok(Box::new(Http::new(Kind::Anthropic, base, Some(key))))
        }
        "openai" => {
            let (var, key) = env_key("OPENAI_API_KEY");
            let key = key.ok_or(Error::MissingApiKey(var))?;
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            Ok(Box::new(Http::new(Kind::OpenAi, base, Some(key))))
        }
        "openai-compatible" => {
            // Local servers (Ollama, LM Studio) need no key.
            let (_, key) = env_key("OPENAI_API_KEY");
            let base = cfg.base_url.clone().ok_or_else(|| {
                Error::Other("llm.base_url is required for provider \"openai-compatible\"".into())
            })?;
            Ok(Box::new(Http::new(Kind::Compatible, base, key)))
        }
        other => Err(Error::Other(format!(
            "unknown llm provider \"{other}\" (expected anthropic | openai | openai-compatible | mock)"
        ))),
    }
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
    client: reqwest::Client,
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
            client,
        }
    }

    fn err(&self, message: impl Into<String>) -> Error {
        Error::Provider {
            provider: self.name().into(),
            message: message.into(),
        }
    }

    fn build(&self, req: &CompletionRequest) -> reqwest::RequestBuilder {
        match self.kind {
            Kind::Anthropic => {
                let mut content: Vec<Value> = req
                    .images
                    .iter()
                    .map(|b64| json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64}}))
                    .collect();
                content.push(json!({"type": "text", "text": req.user}));
                // No assistant prefill: rejected by current Claude models. JSON is enforced by the prompt + robust parsing.
                let body = json!({
                    "model": req.model,
                    "max_tokens": req.max_tokens,
                    "system": req.system,
                    "messages": [{"role": "user", "content": content}],
                });
                let mut rb = self
                    .client
                    .post(format!("{}/v1/messages", self.base_url))
                    .header("x-api-key", self.api_key.as_deref().unwrap_or_default())
                    .header("anthropic-version", "2023-06-01");
                // Org-level user keys (sk-ant-usr-…) must name the workspace to bill.
                if let Ok(ws) = std::env::var("ANTHROPIC_WORKSPACE_ID")
                    && !ws.is_empty()
                {
                    rb = rb.header("anthropic-workspace-id", ws);
                }
                rb.json(&body)
            }
            Kind::OpenAi | Kind::Compatible => {
                let user: Value = if req.images.is_empty() {
                    json!(req.user)
                } else {
                    let mut parts: Vec<Value> = vec![json!({"type": "text", "text": req.user})];
                    parts.extend(req.images.iter().map(|b64| json!({"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{b64}")}})));
                    Value::Array(parts)
                };
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
                if req.json {
                    body["response_format"] = json!({"type": "json_object"});
                }
                let mut rb = self
                    .client
                    .post(format!("{}/chat/completions", self.base_url))
                    .json(&body);
                if let Some(k) = &self.api_key {
                    rb = rb.bearer_auth(k);
                }
                rb
            }
        }
    }

    fn parse(&self, model: &str, v: &Value) -> Result<Completion> {
        let (text, input, output) = match self.kind {
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
                (
                    text,
                    &v["usage"]["input_tokens"],
                    &v["usage"]["output_tokens"],
                )
            }
            Kind::OpenAi | Kind::Compatible => {
                let text = v["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                (
                    text,
                    &v["usage"]["prompt_tokens"],
                    &v["usage"]["completion_tokens"],
                )
            }
        };
        let model = v["model"].as_str().unwrap_or(model).to_string();
        Ok(Completion {
            text,
            model,
            input_tokens: input.as_u64().unwrap_or(0) as u32,
            output_tokens: output.as_u64().unwrap_or(0) as u32,
        })
    }
}

fn retryable(status: u16) -> bool {
    status == 408 || status == 409 || status == 429 || status >= 500
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
        let mut last = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                tracing::warn!(provider = self.name(), attempt, "retrying LLM call: {last}");
            }
            let mut wait = Duration::from_millis(1000 * 2u64.pow(attempt));
            match self.build(&req).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
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
                        let v: Value = serde_json::from_str(&body)
                            .map_err(|e| self.err(format!("invalid JSON response: {e}")))?;
                        return self.parse(&req.model, &v);
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
}
