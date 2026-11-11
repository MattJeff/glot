//! Shopify: GraphQL Admin API with a custom app's Admin API access token.
//!
//! - Pull: `translatableResources(resourceType:, first:, after:)` per resource type (products, collections,
//!   pages, blogs, articles, metafields, metaobjects, menus/links, shop, policies, online store theme), cursor
//!   pagination. Every `translatableContent` (key, value, digest, type) is a field; links, URLs, file
//!   references, JSON and handles (URL slugs) are skipped. HTML / INLINE_RICH_TEXT → `Format::Html`,
//!   RICH_TEXT_FIELD (metafield JSON) → `Format::RichText`.
//! - Push: `translationsRegister(resourceId:, translations:)` with the source `translatableContentDigest`
//!   kept from the pull (Shopify rejects a translation whose digest no longer matches the source). A target
//!   locale missing from `shopLocales` is added with `shopLocaleEnable` (unpublished: the merchant publishes it).
//! - Throttle: calculated query cost (`extensions.cost.throttleStatus`): waits for the bucket to refill
//!   before the next call, retries `THROTTLED` errors.
//! - Webhooks: `webhookSubscriptionCreate` for products/collections create/update, verified with
//!   `X-Shopify-Hmac-Sha256` (base64 HMAC-SHA256 of the body, key = the app's API secret key).
//!
//! Docs: https://shopify.dev/docs/apps/build/markets/manage-translated-content,
//! https://shopify.dev/docs/api/admin-graphql/latest/queries/translatableResources,
//! https://shopify.dev/docs/api/admin-graphql/latest/mutations/translationsRegister,
//! https://shopify.dev/docs/api/usage/limits, https://shopify.dev/docs/apps/build/webhooks/subscribe/https
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

use super::{AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem};

pub struct Shopify;

pub const API_VERSION: &str = "2025-10";

/// Synced by default (`settings.resources` overrides).
pub const RESOURCES: &[&str] = &[
    "PRODUCT",
    "PRODUCT_OPTION",
    "PRODUCT_OPTION_VALUE",
    "COLLECTION",
    "PAGE",
    "BLOG",
    "ARTICLE",
    "METAFIELD",
    "METAOBJECT",
    "MENU",
    "LINK",
    "SHOP",
    "SHOP_POLICY",
    "ONLINE_STORE_THEME",
];

const PAGE: u32 = 50;

const SKIP_TYPES: &[&str] = &[
    "URI",
    "URL",
    "LINK",
    "LIST_URL",
    "LIST_LINK",
    "FILE_REFERENCE",
    "LIST_FILE_REFERENCE",
    "JSON",
    "JSON_STRING",
];

fn endpoint(creds: &Creds) -> anyhow::Result<String> {
    let shop = creds.get("shop")?.trim_end_matches('/');
    let version = creds
        .setting("api_version")
        .and_then(Value::as_str)
        .unwrap_or(API_VERSION);
    // `http(s)://…` = a full base (tests, proxies); else a store domain (`x` or `x.myshopify.com`).
    let base = if shop.starts_with("http://") || shop.starts_with("https://") {
        shop.to_string()
    } else {
        let host = shop.trim_start_matches("www.");
        anyhow::ensure!(
            host.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.'),
            "invalid shop domain"
        );
        if host.contains('.') {
            format!("https://{host}")
        } else {
            format!("https://{host}.myshopify.com")
        }
    };
    Ok(format!("{base}/admin/api/{version}/graphql.json"))
}

/// One GraphQL call: throttled by the cost bucket, `THROTTLED` retried, other errors returned.
async fn gql(creds: &Creds, query: &str, vars: Value) -> anyhow::Result<Value> {
    let url = endpoint(creds)?;
    let token = creds.get("token")?.to_string();
    let client = super::client_for(&url).await?;
    let body = json!({"query": query, "variables": vars});
    for _ in 0..6 {
        let res = super::send(|| {
            client
                .post(&url)
                .header("X-Shopify-Access-Token", &token)
                .json(&body)
        })
        .await?;
        let v: Value = res.json().await?;
        let cost = &v["extensions"]["cost"];
        let wait = throttle_wait(cost);
        let throttled = v["errors"].as_array().is_some_and(|e| {
            e.iter()
                .any(|e| e["extensions"]["code"].as_str() == Some("THROTTLED"))
        });
        if throttled {
            tokio::time::sleep(wait.max(Duration::from_secs(1))).await;
            continue;
        }
        if let Some(errs) = v["errors"].as_array().filter(|e| !e.is_empty()) {
            let msgs: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
            anyhow::bail!("Shopify: {}", msgs.join("; "));
        }
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        return Ok(v["data"].clone());
    }
    anyhow::bail!("Shopify: still throttled after retries")
}

/// Time to wait so the bucket holds the last query's cost again (the next query is assumed similar).
fn throttle_wait(cost: &Value) -> Duration {
    let t = &cost["throttleStatus"];
    let (Some(avail), Some(rate)) = (
        t["currentlyAvailable"].as_f64(),
        t["restoreRate"].as_f64().filter(|r| *r > 0.0),
    ) else {
        return Duration::ZERO;
    };
    let need = cost["requestedQueryCost"]
        .as_f64()
        .or(cost["actualQueryCost"].as_f64())
        .unwrap_or(0.0);
    if avail >= need {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(((need - avail) / rate).min(30.0))
    }
}

fn resources(creds: &Creds) -> Vec<String> {
    match creds.setting("resources").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        None => RESOURCES.iter().map(|s| s.to_string()).collect(),
    }
}

/// `gid://shopify/Product/123` → `123` (theme ids and others: the last path segment).
fn short_id(gid: &str) -> &str {
    gid.rsplit('/').next().unwrap_or(gid)
}

const LIST: &str = "query($type: TranslatableResourceType!, $first: Int!, $after: String) {
  translatableResources(resourceType: $type, first: $first, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes { resourceId translatableContent { key value digest locale type } }
  }
}";

const REGISTER: &str = "mutation($id: ID!, $translations: [TranslationInput!]!) {
  translationsRegister(resourceId: $id, translations: $translations) {
    translations { key locale value }
    userErrors { field message code }
  }
}";

/// Fields of one resource node.
fn items_of(rtype: &str, node: &Value, translate_handles: bool) -> Vec<SourceItem> {
    let id = node["resourceId"].as_str().unwrap_or_default();
    let content = node["translatableContent"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let title = content
        .iter()
        .find(|c| c["key"] == "title" || c["key"] == "name")
        .and_then(|c| c["value"].as_str())
        .unwrap_or_default();
    let label = rtype.to_lowercase().replace('_', " ");
    let mut out = vec![];
    for c in &content {
        let (Some(key), Some(value), Some(ty)) =
            (c["key"].as_str(), c["value"].as_str(), c["type"].as_str())
        else {
            continue;
        };
        if value.trim().is_empty()
            || SKIP_TYPES.contains(&ty)
            || (key == "handle" && !translate_handles)
        {
            continue;
        }
        let format = match ty {
            "HTML" | "INLINE_RICH_TEXT" => Format::Html,
            "RICH_TEXT_FIELD" => match serde_json::from_str(value) {
                Ok(doc) => Format::RichText(doc),
                Err(_) => continue,
            },
            _ => Format::Plain,
        };
        out.push(SourceItem {
            external_id: id.to_string(),
            field: key.to_string(),
            key: super::key("shopify", &rtype.to_lowercase(), short_id(id), key),
            text: value.to_string(),
            format,
            context: if title.is_empty() {
                format!("Shopify {label}, field {key}")
            } else {
                format!("Shopify {label} \u{ab}{title}\u{bb}, field {key}")
            },
            updated_at: None,
            meta: json!({"digest": c["digest"]}),
        });
    }
    out
}

async fn shop_locales(creds: &Creds) -> anyhow::Result<Vec<(String, bool)>> {
    let d = gql(
        creds,
        "{ shopLocales { locale primary published } }",
        json!({}),
    )
    .await?;
    Ok(d["shopLocales"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| Some((l["locale"].as_str()?.to_string(), l["primary"] == true)))
        .collect())
}

#[async_trait]
impl Connector for Shopify {
    fn kind(&self) -> &'static str {
        "shopify"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let d = gql(
            creds,
            "{ shop { name } shopLocales { locale primary published } }",
            json!({}),
        )
        .await?;
        let primary = d["shopLocales"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["primary"] == true)
            .and_then(|l| l["locale"].as_str())
            .map(str::to_string);
        let mut notes = vec![];
        if creds.opt("webhook_secret").is_none() {
            notes.push(
                "No API secret key: webhooks are not registered; sync manually or periodically."
                    .into(),
            );
        }
        Ok(AccountInfo {
            name: d["shop"]["name"].as_str().unwrap_or("Shopify").to_string(),
            source_locale: primary,
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(shop_locales(creds)
            .await?
            .into_iter()
            .map(|l| l.0)
            .collect())
    }

    /// Cursor = `{resource type index}|{GraphQL endCursor}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let types = resources(creds);
        let (mut i, mut after) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, a)) => (
                i.parse::<usize>()?,
                Some(a.to_string()).filter(|a| !a.is_empty()),
            ),
            None => (0, None),
        };
        let handles = creds.setting("translate_handles") == Some(&Value::Bool(true));
        while i < types.len() {
            let d = gql(
                creds,
                LIST,
                json!({"type": types[i], "first": PAGE, "after": after}),
            )
            .await?;
            let tr = &d["translatableResources"];
            let items: Vec<SourceItem> = tr["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|n| items_of(&types[i], n, handles))
                .collect();
            let next = if tr["pageInfo"]["hasNextPage"] == true {
                Some(format!(
                    "{i}|{}",
                    tr["pageInfo"]["endCursor"].as_str().unwrap_or_default()
                ))
            } else if i + 1 < types.len() {
                Some(format!("{}|", i + 1))
            } else {
                None
            };
            if !items.is_empty() || next.is_none() {
                return Ok(PullPage { items, next });
            }
            // Empty page of this type: go on without a round trip through the sync.
            i += 1;
            after = None;
        }
        Ok(PullPage::default())
    }

    async fn push(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport> {
        if !shop_locales(creds).await?.iter().any(|l| l.0 == locale) {
            let d = gql(
                creds,
                "mutation($locale: String!) { shopLocaleEnable(locale: $locale) { shopLocale { locale published } userErrors { field message } } }",
                json!({"locale": locale}),
            )
            .await?;
            if let Some(e) = d["shopLocaleEnable"]["userErrors"]
                .as_array()
                .and_then(|e| e.first())
            {
                anyhow::bail!(
                    "cannot enable {locale}: {}",
                    e["message"].as_str().unwrap_or("error")
                );
            }
        }
        let mut by_resource: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_resource.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (id, its) in by_resource {
            let translations: Vec<Value> = its
                .iter()
                .map(|it| {
                    json!({
                        "locale": locale,
                        "key": it.field,
                        "value": it.value,
                        "translatableContentDigest": it.meta["digest"],
                    })
                })
                .collect();
            let d = match gql(
                creds,
                REGISTER,
                json!({"id": id, "translations": translations}),
            )
            .await
            {
                Ok(d) => d,
                Err(e) => {
                    for it in &its {
                        report
                            .failed
                            .push((format!("{}/{}", it.external_id, it.field), format!("{e:#}")));
                    }
                    continue;
                }
            };
            let errors = d["translationsRegister"]["userErrors"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut bad = vec![false; its.len()];
            for e in &errors {
                let msg = e["message"].as_str().unwrap_or("refused").to_string();
                // field = ["translations", "<index>", "<input field>"]; no index = the whole call.
                let idx = e["field"]
                    .as_array()
                    .and_then(|f| f.get(1))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|i| *i < its.len());
                let hit: Vec<usize> = match idx {
                    Some(i) => vec![i],
                    None => (0..its.len()).collect(),
                };
                for i in hit {
                    if !bad[i] {
                        bad[i] = true;
                        report.failed.push((
                            format!("{}/{}", its[i].external_id, its[i].field),
                            msg.clone(),
                        ));
                    }
                }
            }
            report.ok += bad.iter().filter(|b| !**b).count();
        }
        Ok(report)
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        headers
            .get("x-shopify-hmac-sha256")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|sig| super::hmac_ok(secret, body, sig))
    }

    async fn register_webhooks(&self, creds: &Creds, url: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            creds.opt("webhook_secret").is_some(),
            "no API secret key: Shopify webhooks could not be verified, none registered"
        );
        let existing = gql(
            creds,
            "{ webhookSubscriptions(first: 100) { nodes { topic uri } } }",
            json!({}),
        )
        .await?;
        let has = |topic: &str| {
            existing["webhookSubscriptions"]["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|n| n["topic"] == topic && n["uri"] == url)
        };
        for topic in [
            "PRODUCTS_CREATE",
            "PRODUCTS_UPDATE",
            "COLLECTIONS_CREATE",
            "COLLECTIONS_UPDATE",
        ] {
            if has(topic) {
                continue;
            }
            let d = gql(
                creds,
                "mutation($topic: WebhookSubscriptionTopic!, $sub: WebhookSubscriptionInput!) {
                   webhookSubscriptionCreate(topic: $topic, webhookSubscription: $sub) {
                     webhookSubscription { id } userErrors { field message } } }",
                json!({"topic": topic, "sub": {"uri": url}}),
            )
            .await?;
            if let Some(e) = d["webhookSubscriptionCreate"]["userErrors"]
                .as_array()
                .and_then(|e| e.first())
            {
                anyhow::bail!("{topic}: {}", e["message"].as_str().unwrap_or("refused"));
            }
        }
        Ok(())
    }

    fn push_batch(&self) -> usize {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_waits_for_refill() {
        let c = json!({"requestedQueryCost": 102, "actualQueryCost": 60,
            "throttleStatus": {"maximumAvailable": 1000.0, "currentlyAvailable": 2, "restoreRate": 50.0}});
        assert_eq!(throttle_wait(&c), Duration::from_secs(2));
        let ok = json!({"requestedQueryCost": 3, "throttleStatus": {"maximumAvailable": 1000.0, "currentlyAvailable": 997, "restoreRate": 50.0}});
        assert_eq!(throttle_wait(&ok), Duration::ZERO);
        assert_eq!(throttle_wait(&Value::Null), Duration::ZERO);
    }

    #[test]
    fn fields_typed_and_filtered() {
        let node = json!({"resourceId": "gid://shopify/Product/42", "translatableContent": [
            {"key": "title", "value": "Shirt", "digest": "d1", "locale": "en", "type": "SINGLE_LINE_TEXT_FIELD"},
            {"key": "body_html", "value": "<p>Soft <b>cotton</b></p>", "digest": "d2", "locale": "en", "type": "HTML"},
            {"key": "handle", "value": "shirt", "digest": "d3", "locale": "en", "type": "URI"},
            {"key": "meta_title", "value": "", "digest": "d4", "locale": "en", "type": "STRING"}]});
        let items = items_of("PRODUCT", &node, false);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "shopify:product:42:title");
        assert_eq!(items[1].format, Format::Html);
        assert_eq!(items[1].meta["digest"], "d2");
        assert!(items[0].context.contains("Shirt"));
    }

    #[test]
    fn endpoints() {
        let c = |shop: &str| Creds {
            secrets: json!({"shop": shop, "token": "t"}),
            ..Default::default()
        };
        assert_eq!(
            endpoint(&c("my-store")).unwrap(),
            format!("https://my-store.myshopify.com/admin/api/{API_VERSION}/graphql.json")
        );
        assert!(
            endpoint(&c("my-store.myshopify.com/"))
                .unwrap()
                .starts_with("https://my-store.myshopify.com/admin")
        );
        assert!(endpoint(&c("evil.com/x?y")).is_err());
    }
}
