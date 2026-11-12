//! CMS connectors: pull content from a CMS (Shopify, WordPress, Webflow, Framer, Contentful, Sanity, Strapi),
//! mirror it as glot keys, translate with the regular jobs, push validated translations back.
//!
//! A connector only speaks its CMS's API ([`Connector`]); everything else is generic ([`sync`]):
//! paginated pull → `connector_items` + glot keys (HTML and rich text split into segment keys, structure kept)
//! → `connector_translate` job → push of the translations that are `machine` or `reviewed` (never `stale`,
//! never partial), idempotent per locale (sha256 of the pushed value), resumable (cursor saved per page),
//! triggered manually, by a webhook (`POST /api/connectors/{id}/webhook`) or periodically (push only).
//!
//! Adding a connector: a module implementing [`Connector`], one line in [`registry`], one entry in
//! `web/components/connectors/index.ts` (`CONNECTORS`).
use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub mod api;
pub mod bigcommerce;
pub mod cms;
pub mod contentful;
pub mod craft;
pub mod drupal;
pub mod ghost;
pub mod helpscout;
pub mod hubspot;
pub mod magento;
pub mod oauth;
pub mod prestashop;
pub mod quality;
pub mod rich;
pub mod sanity;
pub mod shopify;
pub mod shopware;
pub mod strapi;
pub mod sync;
pub mod webflow;
pub mod woocommerce;
pub mod wordpress;
pub mod zendesk;

/// Decrypted credentials (`secrets`, the JSON object the dashboard form posted) and non-secret `settings`.
#[derive(Debug, Clone, Default)]
pub struct Creds {
    pub secrets: Value,
    pub settings: Value,
    /// Incremental pull (webhook-triggered): only content changed since then, when the CMS can filter.
    /// `None` = full pull. Connectors that cannot filter ignore it (the sync is idempotent anyway).
    pub since: Option<DateTime<Utc>>,
}

impl Creds {
    /// A required, non-empty string secret (`token`, `shop`, …), trimmed.
    pub fn get(&self, name: &str) -> anyhow::Result<&str> {
        self.opt(name)
            .ok_or_else(|| anyhow::anyhow!("missing credential '{name}'"))
    }
    pub fn opt(&self, name: &str) -> Option<&str> {
        self.secrets
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
    pub fn setting(&self, name: &str) -> Option<&Value> {
        self.settings.get(name)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountInfo {
    /// Human name of the connected site/store/space.
    pub name: String,
    /// The CMS's default (source) locale, when it has one: compared with the project's source locale.
    pub source_locale: Option<String>,
    /// Anything the user should know (plugin detected, missing scope…), shown on the card.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// How a field's value is structured. Html and RichText are never sent whole to the translator: they are
/// split into text segments (placeholders for inline tags / marks) and rebuilt on push.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Plain,
    Html,
    /// JSON document (Contentful Rich Text, Sanity Portable Text, Strapi Blocks, Lexical): text nodes are
    /// translated in place, marks/links/structure kept (see [`rich`]).
    RichText(Value),
}

impl Format {
    pub fn name(&self) -> &'static str {
        match self {
            Format::Plain => "plain",
            Format::Html => "html",
            Format::RichText(_) => "rich",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourceItem {
    /// CMS id of the resource (`gid://shopify/Product/1`, `post:12`).
    pub external_id: String,
    /// Field of the resource (`title`, `body_html`).
    pub field: String,
    /// Stable glot key: `{kind}:{resource}:{id}:{field}` ([`key`]).
    pub key: String,
    /// Source value: plain text or HTML. For [`Format::RichText`], ignored (the document is in `format`).
    pub text: String,
    pub format: Format,
    /// Content type and title, stored as the key description (translator context).
    pub context: String,
    pub updated_at: Option<DateTime<Utc>>,
    /// Connector data needed again on push (e.g. Shopify's `translatableContentDigest`), stored as is.
    pub meta: Value,
}

#[derive(Debug, Clone)]
pub struct PushItem {
    pub external_id: String,
    pub field: String,
    /// Translated value (plain text, HTML, or the rich text document serialized as JSON).
    pub value: String,
    /// Same shape as pulled; `RichText` carries the translated document.
    pub format: Format,
    /// The item's [`SourceItem::meta`] from the last pull.
    pub meta: Value,
}

#[derive(Debug, Clone, Default)]
pub struct PullPage {
    pub items: Vec<SourceItem>,
    /// Opaque cursor of the next page; `None` = last page.
    pub next: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PushReport {
    pub ok: usize,
    /// (external_id or `external_id/field`, error) of the items the CMS refused.
    pub failed: Vec<(String, String)>,
}

#[async_trait]
pub trait Connector: Send + Sync {
    fn kind(&self) -> &'static str;
    /// Checks the credentials (one cheap authenticated call).
    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo>;
    /// Locales enabled in the CMS (its own codes).
    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>>;
    /// One page of source content; `cursor` = previous page's `next`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage>;
    /// Writes translations of `locale` (the CMS's code, after the account's `locale_map`).
    async fn push(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport>;
    /// Authenticity of an incoming webhook. `secret` = the credential `webhook_secret` when the user gave
    /// one (e.g. Shopify's app secret), else the account's generated secret.
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool;
    /// Subscribes the CMS's webhooks to `url` when its API allows it (best effort, after create/test).
    async fn register_webhooks(&self, _creds: &Creds, _url: &str) -> anyhow::Result<()> {
        Ok(())
    }
    /// Items per `push` call.
    fn push_batch(&self) -> usize {
        50
    }
    /// The platform's current translation of each item in `locale` (same order; `None` = none yet).
    /// `Ok(None)` = this connector cannot read translations back: no human-edit protection, no rollback.
    async fn current(
        &self,
        _creds: &Creds,
        _locale: &str,
        _items: &[PushItem],
    ) -> anyhow::Result<Option<Vec<Option<String>>>> {
        Ok(None)
    }
    /// Deletes the translations of `items` in `locale` (rollback of a first write). `false` = unsupported.
    async fn remove(
        &self,
        _creds: &Creds,
        _locale: &str,
        _items: &[PushItem],
    ) -> anyhow::Result<bool> {
        Ok(false)
    }
    /// Platform-specific diagnosis (granted scopes, plugins, API capabilities), on top of the generic checks
    /// of `api::doctor` (auth, languages, read, translations readable, webhooks).
    async fn doctor(&self, _creds: &Creds) -> Vec<Check> {
        vec![]
    }
}

/// One line of the connection diagnosis ("Test connection").
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    /// `ok` | `warn` | `fail`
    pub status: &'static str,
    pub detail: String,
    /// What to do about it ([`fix_code`]), translated by the dashboard.
    pub fix: Option<&'static str>,
}

impl Check {
    pub fn ok(id: &str, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: "ok",
            detail: detail.into(),
            fix: None,
        }
    }
    pub fn warn(id: &str, detail: impl Into<String>, fix: Option<&'static str>) -> Self {
        Self {
            status: "warn",
            fix,
            ..Self::ok(id, detail)
        }
    }
    pub fn fail(id: &str, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            status: "fail",
            fix: fix_code(&detail),
            ..Self::ok(id, detail)
        }
    }
}

/// The fix a platform error calls for, as a code the dashboard translates (`connectors.fix.<code>`).
pub fn fix_code(error: &str) -> Option<&'static str> {
    let e = error.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| e.contains(w));
    Some(
        if has(&[
            "reconnect",
            "http 401",
            "invalid api key",
            "unauthorized",
            "invalid_token",
        ]) {
            "reconnect"
        } else if has(&[
            "http 403",
            "scope",
            "access denied",
            "forbidden",
            "permission",
            "not allowed",
        ]) {
            "permissions"
        } else if has(&["http 429", "throttled", "rate limit"]) {
            "rate_limit"
        } else if has(&["placeholder", "variable"]) {
            "placeholders"
        } else if has(&["markup"]) {
            "markup"
        } else if has(&["digest", "http 409", "version", "conflict"]) {
            "resync"
        } else if has(&["http 404", "not found"]) {
            "not_found"
        } else if has(&["locale", "language"]) {
            "enable_locale"
        } else if has(&[
            "timeout",
            "unreachable",
            "cannot resolve",
            "not a public address",
        ]) {
            "unreachable"
        } else {
            return None;
        },
    )
}

/// Every connector kind, for the dashboard and the API.
pub const KINDS: &[&str] = &[
    "shopify",
    "wordpress",
    "webflow",
    "framer",
    "contentful",
    "sanity",
    "strapi",
    "bigcommerce",
    "woocommerce",
    "prestashop",
    "magento",
    "shopware",
    "hubspot",
    "ghost",
    "zendesk",
    "drupal",
    "craft",
    "helpscout",
];

pub fn registry(kind: &str) -> Option<Box<dyn Connector>> {
    match kind {
        "shopify" => Some(Box::new(shopify::Shopify)),
        "wordpress" => Some(Box::new(wordpress::WordPress)),
        "contentful" => Some(Box::new(contentful::Contentful)),
        "sanity" => Some(Box::new(sanity::Sanity)),
        "strapi" => Some(Box::new(strapi::Strapi)),
        "webflow" => Some(Box::new(webflow::Webflow)),
        "bigcommerce" => Some(Box::new(bigcommerce::BigCommerce)),
        "woocommerce" => Some(Box::new(woocommerce::WooCommerce)),
        "prestashop" => Some(Box::new(prestashop::PrestaShop)),
        "magento" => Some(Box::new(magento::Magento)),
        "shopware" => Some(Box::new(shopware::Shopware)),
        "hubspot" => Some(Box::new(hubspot::HubSpot)),
        "ghost" => Some(Box::new(ghost::Ghost)),
        "zendesk" => Some(Box::new(zendesk::Zendesk)),
        "drupal" => Some(Box::new(drupal::Drupal)),
        "craft" => Some(Box::new(craft::Craft)),
        "helpscout" => Some(Box::new(helpscout::HelpScout)),
        // Agents G / H: "webflow" | "framer" | "contentful" | "sanity" | "strapi" => Some(Box::new(…)),
        _ => None,
    }
}

/// Whether `key` mirrors CMS content (`{kind}:{resource}:{id}:{field}`): validated as plain text with `{0}`
/// placeholders, never as an ICU message (`{{ shop.name }}`, a lone `{` or `<` are just text there).
pub fn is_key(key: &str) -> bool {
    key.split_once(':')
        .is_some_and(|(kind, _)| KINDS.contains(&kind))
}

/// Stable glot key of a field.
pub fn key(kind: &str, resource: &str, id: &str, field: &str) -> String {
    format!("{kind}:{resource}:{id}:{field}")
}

// ---------------------------------------------------------------------------------------------- HTTP

/// Private addresses are refused unless `GLOT_CONNECTORS_ALLOW_PRIVATE=1` (local development and tests:
/// simulated CMS servers on 127.0.0.1).
pub fn allow_private() -> bool {
    std::env::var("GLOT_CONNECTORS_ALLOW_PRIVATE").is_ok_and(|v| v == "1")
}

/// HTTP client for a user-supplied CMS URL: http(s) only, public addresses only (DNS checked and pinned),
/// no redirects, 30 s timeout.
pub async fn client_for(url: &str) -> anyhow::Result<reqwest::Client> {
    let u = reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("invalid URL '{url}'"))?;
    anyhow::ensure!(
        matches!(u.scheme(), "http" | "https"),
        "URL must be http(s)"
    );
    let host = u
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?
        .to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> =
        match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
            Ok(ip) => vec![std::net::SocketAddr::new(ip, port)],
            Err(_) => tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|_| anyhow::anyhow!("cannot resolve {host}"))?
                .collect(),
        };
    anyhow::ensure!(!addrs.is_empty(), "cannot resolve {host}");
    anyhow::ensure!(
        allow_private()
            || addrs
                .iter()
                .all(|a| crate::review_live::probe::is_public(a.ip())),
        "{host} is not a public address"
    );
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .resolve_to_addrs(&host, &addrs)
        .user_agent("glot-connectors/1")
        .build()?)
}

/// Sends `req` (rebuilt by `make` on each try), retrying 429 / 503 per `Retry-After` (else 1, 2, 4 s; 4 tries).
/// Errors carry the status and the start of the body, never the request (credentials).
pub async fn send(make: impl Fn() -> reqwest::RequestBuilder) -> anyhow::Result<reqwest::Response> {
    let mut wait = 1u64;
    for attempt in 0..4 {
        let res = make().send().await.map_err(|e| {
            anyhow::anyhow!(
                "request failed: {}",
                if e.is_timeout() {
                    "timeout"
                } else {
                    "unreachable"
                }
            )
        })?;
        let s = res.status();
        if (s.as_u16() == 429 || s.as_u16() == 503) && attempt < 3 {
            let after = res
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|x| x.clamp(0.0, 60.0))
                .unwrap_or(wait as f64);
            tokio::time::sleep(Duration::from_secs_f64(after)).await;
            wait *= 2;
            continue;
        }
        if !s.is_success() {
            let body = res.text().await.unwrap_or_default();
            let body: String = body.chars().take(300).collect();
            anyhow::bail!("HTTP {}: {body}", s.as_u16());
        }
        return Ok(res);
    }
    unreachable!()
}

/// Constant-time HMAC-SHA256 check of `body` against `sig` (hex or base64, optional `sha256=` prefix).
pub fn hmac_ok(secret: &str, body: &[u8], sig: &str) -> bool {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let sig = sig.trim();
    let sig = sig.strip_prefix("sha256=").unwrap_or(sig);
    let Some(given) = hex::decode(sig)
        .ok()
        .or_else(|| base64::engine::general_purpose::STANDARD.decode(sig).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&given).is_ok()
}

/// Constant-time string equality (shared-secret headers / tokens).
pub fn secret_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_hex_and_base64() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"s3cret").unwrap();
        mac.update(b"{\"id\":1}");
        let raw = mac.finalize().into_bytes();
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        assert!(hmac_ok("s3cret", b"{\"id\":1}", &b64));
        assert!(hmac_ok(
            "s3cret",
            b"{\"id\":1}",
            &format!("sha256={}", hex::encode(raw))
        ));
        assert!(!hmac_ok("s3cret", b"{\"id\":2}", &b64));
        assert!(!hmac_ok("other", b"{\"id\":1}", &b64));
        assert!(!hmac_ok("s3cret", b"x", "not a sig"));
        assert!(secret_eq("abc", "abc") && !secret_eq("abc", "abd") && !secret_eq("abc", "ab"));
    }
}
