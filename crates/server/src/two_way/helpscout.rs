//! Help Scout conversations (Mailbox API 2.0): the helpdesk adapter of two-way translation. The Help Scout
//! *Docs* connector (`connectors/helpscout.rs`) translates help-center articles; this is the inbox side.
//!
//! Without leaving Help Scout:
//! - a customer writes → webhook → glot posts a **note** with the translation in the team's language;
//! - the agent answers with a note starting with `/glot ` in their own language → webhook → glot translates
//!   it and, per the project's mode, leaves a **draft reply** (suggest), keeps it for a click in the glot
//!   inbox (one_click) or sends the reply (auto, unless flagged); a note reports the back-translation.
//!
//! - Webhook: `X-HelpScout-Signature` = base64(HMAC-SHA1(secret, raw body)), event in `X-HelpScout-Event`
//!   (`convo.created`, `convo.customer.reply.created`, `convo.note.created`); the body is the v2 conversation.
//!   https://developer.helpscout.com/webhooks/
//! - Auth: OAuth2 client credentials, `POST /v2/oauth2/token` (form: grant_type, client_id, client_secret) →
//!   `access_token`, valid 2 days. https://developer.helpscout.com/mailbox-api/overview/authentication/
//! - Threads: `GET /v2/conversations/{id}/threads` → `_embedded.threads`, newest first (`type` customer |
//!   message | note …). https://developer.helpscout.com/mailbox-api/endpoints/conversations/threads/list/
//! - Note: `POST /v2/conversations/{id}/notes` `{text}` → 201.
//!   https://developer.helpscout.com/mailbox-api/endpoints/conversations/threads/note/
//! - Reply: `POST /v2/conversations/{id}/reply` `{customer: {id}, text, draft}` → 201 + `Resource-Id`.
//!   https://developer.helpscout.com/mailbox-api/endpoints/conversations/threads/reply/
//!
//! The API base is `https://api.helpscout.net`; `GLOT_HELPSCOUT_API` replaces it for tests only
//! (honoured with `GLOT_CONNECTORS_ALLOW_PRIVATE=1`, like the connectors).
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use super::{Conversation, Incoming, Message, Settings};
use crate::{ApiResult, AppError, AppPath, AppState, projects::ProjectRow};

/// What agents start a note with to have it translated and answered to the customer.
pub const COMMAND: &str = "/glot";

pub fn sign(secret: &str, body: &[u8]) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(ring::hmac::sign(&key, body))
}

pub fn verify(secret: &str, body: &[u8], signature: &str) -> bool {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret.as_bytes());
    base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .is_ok_and(|sig| ring::hmac::verify(&key, body, &sig).is_ok())
}

struct Api {
    http: reqwest::Client,
    base: String,
    app_id: String,
    app_secret: String,
}

/// app id → (access token, expiry). Help Scout asks not to mint a token per call.
static TOKENS: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(Default::default);

impl Api {
    async fn new(state: &AppState, s: &Settings) -> anyhow::Result<Self> {
        let base = std::env::var("GLOT_HELPSCOUT_API")
            .ok()
            .filter(|_| crate::connectors::allow_private())
            .unwrap_or_else(|| "https://api.helpscout.net".into());
        let missing = || {
            anyhow::anyhow!("Help Scout app ID and secret are not set (Conversations → Settings)")
        };
        Ok(Api {
            http: crate::connectors::client_for(&base).await?,
            base: base.trim_end_matches('/').to_string(),
            app_id: s.helpscout_app_id.clone().ok_or_else(missing)?,
            app_secret: s
                .helpscout_app_secret_enc
                .as_ref()
                .and_then(|e| crate::auth::decrypt(&state.secret, e))
                .ok_or_else(missing)?,
        })
    }

    async fn token(&self, fresh: bool) -> anyhow::Result<String> {
        if !fresh
            && let Some((t, until)) = TOKENS.lock().unwrap().get(&self.app_id)
            && *until > Instant::now()
        {
            return Ok(t.clone());
        }
        let res = self
            .http
            .post(format!("{}/v2/oauth2/token", self.base))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.app_id.as_str()),
                ("client_secret", self.app_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Help Scout is unreachable"))?;
        anyhow::ensure!(
            res.status().is_success(),
            "Help Scout refused the app ID / secret ({}): check them in Help Scout → Your Profile → My Apps",
            res.status()
        );
        let v: Value = res.json().await?;
        let token = v["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Help Scout returned no access token"))?
            .to_string();
        let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60);
        TOKENS.lock().unwrap().insert(
            self.app_id.clone(),
            (token.clone(), Instant::now() + Duration::from_secs(ttl)),
        );
        Ok(token)
    }

    /// One authenticated call; an expired token (401) is renewed once.
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<reqwest::Response> {
        for fresh in [false, true] {
            let token = self.token(fresh).await?;
            let mut req = self
                .http
                .request(method.clone(), format!("{}{path}", self.base))
                .bearer_auth(token);
            if let Some(b) = body {
                req = req.json(b);
            }
            let res = req
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("Help Scout is unreachable"))?;
            if res.status() == StatusCode::UNAUTHORIZED && !fresh {
                continue;
            }
            let status = res.status();
            if !status.is_success() {
                let text: String = res
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect();
                anyhow::bail!("Help Scout answered {status} on {path}: {text}");
            }
            return Ok(res);
        }
        unreachable!("the second pass always returns")
    }
}

/// Help Scout bodies are HTML: the text a person reads, paragraphs and `<br>` as line breaks.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            rest = &rest[i..];
            break;
        };
        let tag = rest[i + 1..i + j].trim_start_matches('/').to_lowercase();
        if ["br", "p", "div", "li", "tr"]
            .iter()
            .any(|t| tag.starts_with(t))
            && !out.ends_with('\n')
        {
            out.push('\n');
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>")
}

/// Create the reply thread in Help Scout: a draft (`draft = true`, never sent by glot) or the real reply.
pub(super) async fn deliver(
    state: &AppState,
    s: &Settings,
    conv: &Conversation,
    msg: Message,
    draft: bool,
) -> ApiResult<Message> {
    let outcome = async {
        let api = Api::new(state, s).await?;
        let customer = conv.meta.0["customer_id"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("this conversation has no Help Scout customer id"))?;
        let body = json!({
            "customer": { "id": customer },
            "text": html(msg.translation.as_deref().unwrap_or(&msg.original)),
            "draft": draft,
        });
        api.call(
            reqwest::Method::POST,
            &format!("/v2/conversations/{}/reply", conv.external_id),
            Some(&body),
        )
        .await?;
        anyhow::Ok(())
    }
    .await;
    super::delivered(
        state,
        msg.id,
        outcome
            .map(|_| {
                if draft {
                    ("helpscout_draft", false)
                } else {
                    ("helpscout", true)
                }
            })
            .map_err(|e| e.to_string()),
    )
    .await
}

async fn note(api: &Api, conversation: &str, text: &str) -> anyhow::Result<()> {
    api.call(
        reqwest::Method::POST,
        &format!("/v2/conversations/{conversation}/notes"),
        Some(&json!({ "text": text })),
    )
    .await?;
    Ok(())
}

/// `POST /api/two-way/{slug}/helpscout`. Verified, then processed in the background: Help Scout retries a
/// webhook that is slow to answer, and a translation can take longer than its patience.
pub async fn webhook(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let (p, s) = super::public_project(&state, &slug).await?;
    let key = super::secret(
        &state,
        &s.helpscout_webhook_secret_enc,
        "the Help Scout webhook secret",
    )?;
    let sig = headers
        .get("x-helpscout-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify(&key, &body, sig) {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "X-HelpScout-Signature does not match the webhook secret saved in glot",
        ));
    }
    let event = headers
        .get("x-helpscout-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let payload: Value = super::parse(&body)?;
    tokio::spawn(async move {
        if let Err(e) = process(&state, &p, &s, &event, &payload).await {
            tracing::warn!(project = %p.slug, event, "helpscout webhook failed: {e}");
        }
    });
    Ok(StatusCode::OK)
}

async fn process(
    state: &AppState,
    p: &ProjectRow,
    s: &Settings,
    event: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    let wanted = match event {
        "convo.created" | "convo.customer.reply.created" => "customer",
        "convo.note.created" => "note",
        _ => return Ok(()),
    };
    let conversation = match &payload["id"] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => anyhow::bail!("payload without a conversation id"),
    };
    let api = Api::new(state, s).await?;
    // The webhook body is the conversation; its threads are embedded or fetched (newest first either way).
    let threads = match payload["_embedded"]["threads"].as_array() {
        Some(t) if !t.is_empty() => t.clone(),
        _ => {
            let v: Value = api
                .call(
                    reqwest::Method::GET,
                    &format!("/v2/conversations/{conversation}/threads"),
                    None,
                )
                .await?
                .json()
                .await?;
            v["_embedded"]["threads"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        }
    };
    let Some(thread) = threads.iter().find(|t| t["type"] == wanted) else {
        return Ok(());
    };
    let text = html_to_text(thread["body"].as_str().unwrap_or(""));
    let thread_id = thread["id"].to_string();
    let err = |e: AppError| anyhow::anyhow!(e.message);

    if wanted == "customer" {
        let incoming = Incoming {
            conversation_id: conversation.clone(),
            message_id: Some(thread_id),
            text,
            subject: payload["subject"].as_str().map(str::to_owned),
            // The numeric customer id is what the reply endpoint needs; no name, no address is kept.
            meta: json!({ "customer_id": payload["primaryCustomer"]["id"] }),
            ..Default::default()
        };
        let (msg, new) = super::receive(state, p, s, "helpscout", incoming)
            .await
            .map_err(err)?;
        if new && let Some(tr) = &msg.translation {
            let from = msg.original_locale.as_deref().unwrap_or("?");
            let mut body = format!("<b>glot · {from} → {}</b><br>{}", s.team(p), html(tr));
            if msg.flags.iter().any(|f| f == "language_uncertain") {
                body += "<br><i>glot is not sure of the language of this message.</i>";
            }
            body += &format!(
                "<br><i>Answer in your language with a note starting with {COMMAND} and glot writes the reply in {from}.</i>"
            );
            note(&api, &conversation, &body).await?;
        }
        return Ok(());
    }

    // An agent's note: only `/glot …` is ours (glot's own notes never start with it, so no loop).
    let Some(reply) = text
        .strip_prefix(COMMAND)
        .map(str::trim)
        .filter(|r| !r.is_empty())
    else {
        return Ok(());
    };
    let report = match reply_from_note(state, p, s, &conversation, reply, &thread_id).await {
        Ok(m) => {
            let outcome = match (m.status.as_str(), m.delivered_via.as_deref()) {
                ("sent", _) => "Sent to the customer.".to_string(),
                ("failed", _) => format!("Not sent: {}", m.error.as_deref().unwrap_or("delivery failed")),
                (_, Some("helpscout_draft")) => "Saved as a draft reply: read it, then send it yourself.".to_string(),
                ("needs_review", _) => "Not sent: glot is not confident in this translation. Check it in the glot inbox.".to_string(),
                _ => "Waiting for a click on Send in the glot inbox.".to_string(),
            };
            format!(
                "<b>glot · {} → {}</b><br>{}<br><b>Back-translation:</b> {}<br><i>{outcome}</i>",
                m.original_locale.as_deref().unwrap_or("?"),
                m.translation_locale.as_deref().unwrap_or("?"),
                html(m.translation.as_deref().unwrap_or("")),
                html(
                    m.back_translation
                        .as_deref()
                        .unwrap_or("(same language, nothing translated)")
                ),
            )
        }
        Err(e) => format!(
            "<b>glot</b> could not translate this reply: {}",
            html(&e.to_string())
        ),
    };
    note(&api, &conversation, &report).await
}

async fn reply_from_note(
    state: &AppState,
    p: &ProjectRow,
    s: &Settings,
    conversation: &str,
    text: &str,
    thread_id: &str,
) -> anyhow::Result<Message> {
    let conv: Conversation = sqlx::query_as(
        "SELECT id, channel, external_id, customer_locale, meta FROM two_way_conversations
         WHERE project_id = $1 AND channel = 'helpscout' AND external_id = $2",
    )
    .bind(p.id)
    .bind(conversation)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        anyhow::anyhow!("glot has not seen a customer message in this conversation yet")
    })?;
    super::reply(state, p, s, &conv, text, false, Some(thread_id))
        .await
        .map_err(|e| anyhow::anyhow!(e.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_and_html() {
        let sig = sign("s3cret", b"{\"id\":1}");
        assert!(verify("s3cret", b"{\"id\":1}", &sig));
        assert!(!verify("other", b"{\"id\":1}", &sig));
        assert!(!verify("s3cret", b"{\"id\":2}", &sig));
        assert!(!verify("s3cret", b"{\"id\":1}", "not base64 !"));
        assert_eq!(
            html_to_text("<p>注文が届きません。</p><p>A &amp; B<br>3 &lt; 4</p>"),
            "注文が届きません。\nA & B\n3 < 4"
        );
    }
}
