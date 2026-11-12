//! BigCommerce: REST Catalog API v3 to read, Translations Admin GraphQL API to write, with a store API
//! account access token (`X-Auth-Token`), base `https://api.bigcommerce.com/stores/{store_hash}`.
//!
//! - Pull: products (`/v3/catalog/products`), categories (`/v3/catalog/trees/categories`) and brands
//!   (`/v3/catalog/brands`), `page`/`limit` pagination (`meta.pagination.total_pages`), incremental for
//!   products (`date_modified:min`). Fields: `name`, `description` (HTML, products and categories),
//!   `page_title`, `meta_description`, `search_keywords`. URLs, SKUs, images, `meta_keywords` (arrays) skipped.
//! - Push: `translation.updateTranslations(input: {resourceType, channelId, localeId, entities})` per
//!   resource type, ids `bc/store/{product|category|brand}/{id}`, `bc/store/channel/{channel_id}`,
//!   `bc/store/locale/{code}`. A target locale missing from the channel is added with `addLocale` (INACTIVE:
//!   the merchant activates it). The default locale is never written (BigCommerce forbids it).
//! - Human edits and rollback: `store.translations(filters: {resourceType, channelId, localeId, resourceIds})`
//!   returns `fields { fieldName original translation }` per resource (50 results per request, a push batch);
//!   a `translation` that is empty or equal to `original` (the API falls back to the default value) counts as
//!   none. `translation.deleteTranslations(input: {…, resources: [{resourceId, fields}]})` deletes exactly
//!   the fields a rolled-back run had created.
//! - Diagnosis: the channel's locales (a channel with its default locale only has nothing to translate into)
//!   and the *Store Translations* OAuth scope, proven by reading; BigCommerce exposes no list of a token's
//!   scopes, so `modify` (needed to write) shows on the first push only.
//! - Web pages (`/v3/content/pages`) have no translation API: not synced.
//! - Rate limits: on `429` waits `X-Rate-Limit-Time-Reset-Ms` then retries; when
//!   `X-Rate-Limit-Requests-Left` reaches 0, waits for the window reset before the next call.
//! - Webhooks: BigCommerce does not sign callbacks but sends custom `headers` set on the hook. With the
//!   optional `webhook_secret` credential, glot subscribes `store/product/*` and `store/category/*`
//!   created/updated (`POST /v3/hooks`, HTTPS on 443 only) with `X-Glot-Webhook-Secret: <secret>`.
//!
//! Docs: https://docs.bigcommerce.com/developer/docs/admin/store-configuration/translations/overview,
//! https://docs.bigcommerce.com/developer/docs/admin/store-configuration/translations/products,
//! https://docs.bigcommerce.com/developer/docs/admin/store-configuration/translations/product-listings,
//! https://docs.bigcommerce.com/developer/api-reference/graphql/admin/mutations/translation/update-translations,
//! https://docs.bigcommerce.com/developer/api-reference/graphql/admin/mutations/translation/delete-translations,
//! https://docs.bigcommerce.com/developer/api-reference/graphql/admin/types/inputs/translation-filters-input,
//! https://docs.bigcommerce.com/developer/docs/admin/store-configuration/settings/locales-configuration,
//! https://docs.bigcommerce.com/docs/rest-catalog/category-trees/categories,
//! https://docs.bigcommerce.com/docs/start/best-practices/api-rate-limits,
//! https://docs.bigcommerce.com/docs/integrations/webhooks, https://docs.bigcommerce.com/docs/integrations/webhooks/events
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, cms,
};

pub struct BigCommerce;

const LIMIT: u32 = 50;

/// (REST path, glot resource, GraphQL resourceType, resourceId segment, id field, fields (name, html)).
type Res = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, bool)],
);
const RESOURCES: &[Res] = &[
    (
        "/v3/catalog/products",
        "product",
        "PRODUCTS",
        "product",
        "id",
        &[
            ("name", false),
            ("description", true),
            ("page_title", false),
            ("meta_description", false),
            ("search_keywords", false),
        ],
    ),
    (
        "/v3/catalog/trees/categories",
        "category",
        "CATEGORIES",
        "category",
        "category_id",
        &[
            ("name", false),
            ("description", true),
            ("page_title", false),
            ("meta_description", false),
            ("search_keywords", false),
        ],
    ),
    (
        "/v3/catalog/brands",
        "brand",
        "BRANDS",
        "brand",
        "id",
        &[
            ("name", false),
            ("page_title", false),
            ("meta_description", false),
            ("search_keywords", false),
        ],
    ),
];

struct Store {
    base: String,
    token: String,
    channel: String,
    client: reqwest::Client,
}

impl Store {
    async fn new(creds: &Creds) -> anyhow::Result<Self> {
        let hash = creds.get("store_hash")?.trim_end_matches('/');
        // `http(s)://…` = a full base (tests, proxies); else the store hash (`stores/{hash}` in the API path).
        let base = if hash.starts_with("http://") || hash.starts_with("https://") {
            hash.to_string()
        } else {
            anyhow::ensure!(
                hash.bytes().all(|b| b.is_ascii_alphanumeric()),
                "invalid store hash"
            );
            format!("https://api.bigcommerce.com/stores/{hash}")
        };
        let channel = cms::param(creds, "channel_id")
            .map(str::to_string)
            .or_else(|| creds.setting("channel_id").map(|v| v.to_string()))
            .unwrap_or_else(|| "1".into());
        anyhow::ensure!(
            channel.bytes().all(|b| b.is_ascii_digit()),
            "invalid channel id"
        );
        Ok(Self {
            client: super::client_for(&base).await?,
            token: creds.get("token")?.to_string(),
            base,
            channel,
        })
    }

    fn channel_gid(&self) -> String {
        format!("bc/store/channel/{}", self.channel)
    }

    /// One call: 429 retried after `X-Rate-Limit-Time-Reset-Ms`, quota exhaustion waited out afterwards.
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = format!("{}{path}", self.base);
        let reset = |h: &reqwest::header::HeaderMap| {
            h.get("x-rate-limit-time-reset-ms")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|ms| Duration::from_millis(ms.min(30_000)))
        };
        let mut backoff = Duration::from_secs(1);
        for attempt in 0..5 {
            let mut r = self
                .client
                .request(method.clone(), &url)
                .header("X-Auth-Token", &self.token)
                .header("Accept", "application/json");
            if let Some(b) = body {
                r = r.json(b);
            }
            let res = r.send().await.map_err(|e| {
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
            if s.as_u16() == 429 && attempt < 4 {
                // No headers = store under load: back off.
                let wait = reset(res.headers()).unwrap_or(backoff);
                tokio::time::sleep(wait).await;
                backoff *= 2;
                continue;
            }
            if !s.is_success() {
                let body: String = res
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect();
                anyhow::bail!("HTTP {}: {body}", s.as_u16());
            }
            let left = res
                .headers()
                .get("x-rate-limit-requests-left")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<i64>().ok());
            if left.is_some_and(|l| l <= 0)
                && let Some(w) = reset(res.headers())
            {
                tokio::time::sleep(w).await;
            }
            if s.as_u16() == 204 {
                return Ok(Value::Null);
            }
            return Ok(res.json().await?);
        }
        anyhow::bail!("BigCommerce: still rate limited after retries")
    }

    async fn gql(&self, query: &str, vars: Value) -> anyhow::Result<Value> {
        let v = self
            .call(
                reqwest::Method::POST,
                "/graphql",
                Some(&json!({"query": query, "variables": vars})),
            )
            .await?;
        if let Some(errs) = v["errors"].as_array().filter(|e| !e.is_empty()) {
            let msgs: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
            anyhow::bail!("BigCommerce: {}", msgs.join("; "));
        }
        Ok(v["data"].clone())
    }

    /// (code, is default) of the channel's locales.
    async fn locales(&self) -> anyhow::Result<Vec<(String, bool)>> {
        let d = self
            .gql(
                "query($channel: ID!) { store { locales(input: {channelId: $channel}) { edges { node { code status isDefault } } } } }",
                json!({"channel": self.channel_gid()}),
            )
            .await?;
        Ok(d["store"]["locales"]["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some((
                    e["node"]["code"].as_str()?.to_string(),
                    e["node"]["isDefault"] == true,
                ))
            })
            .collect())
    }
}

fn items_of(r: &Res, o: &Value) -> Vec<SourceItem> {
    let (_, resource, rtype, seg, idf, fields) = *r;
    let Some(id) = o[idf].as_i64() else {
        return vec![];
    };
    let title = o["name"].as_str().unwrap_or_default();
    fields
        .iter()
        .filter_map(|(f, html)| {
            let v = o[*f].as_str()?;
            if !cms::has_letter(v) {
                return None;
            }
            Some(SourceItem {
                external_id: format!("{resource}:{id}"),
                field: f.to_string(),
                key: super::key("bigcommerce", resource, &id.to_string(), f),
                text: v.to_string(),
                format: if *html { Format::Html } else { Format::Plain },
                context: format!("BigCommerce {resource} \u{ab}{title}\u{bb}, field {f}"),
                updated_at: None,
                meta: json!({"type": rtype, "rid": format!("bc/store/{seg}/{id}")}),
            })
        })
        .collect()
}

const UPDATE: &str = "mutation($input: UpdateTranslationsInput!) {
  translation { updateTranslations(input: $input) { __typename errors { __typename ... on Error { message } } } }
}";

const READ: &str = "query($filters: TranslationFiltersInput!) {
  store { translations(filters: $filters, first: 50) { edges { node { resourceId fields { fieldName original translation } } } } }
}";

const DELETE: &str = "mutation($input: DeleteTranslationsInput!) {
  translation { deleteTranslations(input: $input) { __typename errors { __typename ... on Error { message } } } }
}";

/// Items by GraphQL resourceType then resourceId; an item without metadata is an error (never skipped).
fn by_type(items: &[PushItem]) -> anyhow::Result<BTreeMap<&str, BTreeMap<&str, Vec<&PushItem>>>> {
    let mut out: BTreeMap<&str, BTreeMap<&str, Vec<&PushItem>>> = BTreeMap::new();
    for it in items {
        let (Some(t), Some(rid)) = (it.meta["type"].as_str(), it.meta["rid"].as_str()) else {
            anyhow::bail!("{}: missing metadata, sync again", it.external_id);
        };
        out.entry(t).or_default().entry(rid).or_default().push(it);
    }
    Ok(out)
}

#[async_trait]
impl Connector for BigCommerce {
    fn kind(&self) -> &'static str {
        "bigcommerce"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let s = Store::new(creds).await?;
        let info = s.call(reqwest::Method::GET, "/v2/store", None).await?;
        let locales = s.locales().await?;
        let mut notes = vec![];
        if creds.opt("webhook_secret").is_none() {
            notes.push(
                "No webhook secret: webhooks are not registered; sync manually or periodically."
                    .into(),
            );
        }
        Ok(AccountInfo {
            name: info["name"].as_str().unwrap_or("BigCommerce").to_string(),
            source_locale: locales.iter().find(|l| l.1).map(|l| l.0.clone()),
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(Store::new(creds)
            .await?
            .locales()
            .await?
            .into_iter()
            .map(|l| l.0)
            .collect())
    }

    /// Cursor = `{resource index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let s = Store::new(creds).await?;
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u32>()?),
            None => (0, 1),
        };
        let Some(r) = RESOURCES.get(i) else {
            return Ok(PullPage::default());
        };
        let mut q = format!("{}?page={page}&limit={LIMIT}", r.0);
        if r.1 == "product" {
            q.push_str(
                "&include_fields=name,description,page_title,meta_description,search_keywords",
            );
            if let Some(since) = creds.since {
                q.push_str(&format!(
                    "&date_modified:min={}",
                    since.format("%Y-%m-%dT%H:%M:%SZ")
                ));
            }
        }
        let v = s.call(reqwest::Method::GET, &q, None).await?;
        let items = v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|o| items_of(r, o))
            .collect();
        let total = v["meta"]["pagination"]["total_pages"].as_u64().unwrap_or(1) as u32;
        let next = if page < total {
            Some(format!("{i}|{}", page + 1))
        } else if i + 1 < RESOURCES.len() {
            Some(format!("{}|1", i + 1))
        } else {
            None
        };
        Ok(PullPage { items, next })
    }

    async fn push(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport> {
        let s = Store::new(creds).await?;
        let locales = s.locales().await?;
        match locales.iter().find(|l| l.0 == locale) {
            Some((_, true)) => anyhow::bail!(
                "{locale} is the channel's default locale: BigCommerce takes no translations for it"
            ),
            Some(_) => {}
            None => {
                let d = s
                    .gql(
                        "mutation($input: AddLocaleInput!) { locale { addLocale(input: $input) { locale { code status } errors { ... on Error { message } } } } }",
                        json!({"input": {"channelId": s.channel_gid(), "code": locale, "status": "INACTIVE"}}),
                    )
                    .await?;
                if let Some(e) = d["locale"]["addLocale"]["errors"]
                    .as_array()
                    .and_then(|e| e.first())
                {
                    anyhow::bail!(
                        "cannot add {locale}: {}",
                        e["message"].as_str().unwrap_or("error")
                    );
                }
            }
        }
        // resourceType → resourceId → fields.
        let mut by_type: BTreeMap<&str, BTreeMap<&str, Vec<&PushItem>>> = BTreeMap::new();
        for it in items {
            let (Some(t), Some(rid)) = (it.meta["type"].as_str(), it.meta["rid"].as_str()) else {
                continue;
            };
            by_type
                .entry(t)
                .or_default()
                .entry(rid)
                .or_default()
                .push(it);
        }
        let mut report = PushReport::default();
        for it in items.iter().filter(|it| it.meta["rid"].as_str().is_none()) {
            report.failed.push((
                format!("{}/{}", it.external_id, it.field),
                "missing metadata, sync again".into(),
            ));
        }
        for (rtype, entities) in by_type {
            let its: Vec<&PushItem> = entities.values().flatten().copied().collect();
            let input = json!({
                "resourceType": rtype,
                "channelId": s.channel_gid(),
                "localeId": format!("bc/store/locale/{locale}"),
                "entities": entities.iter().map(|(rid, fs)| json!({
                    "resourceId": rid,
                    "fields": fs.iter().map(|f| json!({"fieldName": f.field, "value": f.value})).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
            // ponytail: errors carry only a message, so any error fails the whole batch; the valid part was
            // saved by BigCommerce (partial success) and the re-push next sync is idempotent.
            let err = match s.gql(UPDATE, json!({"input": input})).await {
                Ok(d) => d["translation"]["updateTranslations"]["errors"]
                    .as_array()
                    .filter(|e| !e.is_empty())
                    .map(|e| {
                        e.iter()
                            .filter_map(|x| x["message"].as_str())
                            .collect::<Vec<_>>()
                            .join("; ")
                    }),
                Err(e) => Some(format!("{e:#}")),
            };
            match err {
                None => report.ok += its.len(),
                Some(msg) => report.failed.extend(
                    its.iter()
                        .map(|it| (format!("{}/{}", it.external_id, it.field), msg.clone())),
                ),
            }
        }
        Ok(report)
    }

    async fn current(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<Option<Vec<Option<String>>>> {
        let s = Store::new(creds).await?;
        // (resourceId, fieldName) → translation.
        let mut found: BTreeMap<(String, String), String> = BTreeMap::new();
        for (rtype, entities) in by_type(items)? {
            let d = s
                .gql(
                    READ,
                    json!({"filters": {
                        "resourceType": rtype,
                        "channelId": s.channel_gid(),
                        "localeId": format!("bc/store/locale/{locale}"),
                        "resourceIds": entities.keys().collect::<Vec<_>>(),
                    }}),
                )
                .await?;
            for e in d["store"]["translations"]["edges"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let Some(rid) = e["node"]["resourceId"].as_str() else {
                    continue;
                };
                for f in e["node"]["fields"].as_array().into_iter().flatten() {
                    if let (Some(name), Some(t)) =
                        (f["fieldName"].as_str(), f["translation"].as_str())
                        && !t.is_empty()
                        && f["original"] != f["translation"]
                    {
                        found.insert((rid.to_string(), name.to_string()), t.to_string());
                    }
                }
            }
        }
        Ok(Some(
            items
                .iter()
                .map(|it| {
                    let rid = it.meta["rid"].as_str()?.to_string();
                    found.get(&(rid, it.field.clone())).cloned()
                })
                .collect(),
        ))
    }

    async fn remove(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<bool> {
        let s = Store::new(creds).await?;
        for (rtype, entities) in by_type(items)? {
            let input = json!({
                "resourceType": rtype,
                "channelId": s.channel_gid(),
                "localeId": format!("bc/store/locale/{locale}"),
                "resources": entities.iter().map(|(rid, fs)| json!({
                    "resourceId": rid,
                    "fields": fs.iter().map(|f| f.field.as_str()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
            let d = s.gql(DELETE, json!({"input": input})).await?;
            if let Some(e) = d["translation"]["deleteTranslations"]["errors"]
                .as_array()
                .and_then(|e| e.first())
            {
                anyhow::bail!("{rtype}: {}", e["message"].as_str().unwrap_or("refused"));
            }
        }
        Ok(true)
    }

    async fn doctor(&self, creds: &Creds) -> Vec<Check> {
        let s = match Store::new(creds).await {
            Ok(s) => s,
            Err(e) => return vec![Check::fail("channel", format!("{e:#}"))],
        };
        let locales = match s.locales().await {
            Ok(l) => l,
            Err(e) => return vec![Check::fail("channel", format!("{e:#}"))],
        };
        let targets: Vec<&str> = locales
            .iter()
            .filter(|l| !l.1)
            .map(|l| l.0.as_str())
            .collect();
        let default = locales.iter().find(|l| l.1).map_or("?", |l| l.0.as_str());
        let mut out = vec![if targets.is_empty() {
            Check::warn(
                "channel",
                format!(
                    "channel {} has its default locale only ({default}): add a locale to it",
                    s.channel
                ),
                Some("enable_locale"),
            )
        } else {
            Check::ok(
                "channel",
                format!("channel {}: {default} → {}", s.channel, targets.join(", ")),
            )
        }];
        // Any locale proves the scope (the default one is read, never written).
        let filters = json!({
            "resourceType": "PRODUCTS",
            "channelId": s.channel_gid(),
            "localeId": format!("bc/store/locale/{}", targets.first().copied().unwrap_or(default)),
        });
        out.push(match s.gql(READ, json!({"filters": filters})).await {
            Ok(_) => Check::ok(
                "permissions",
                "Store Translations readable (writing needs its modify permission)",
            ),
            Err(e) => Check {
                fix: Some("permissions"),
                ..Check::fail(
                    "permissions",
                    format!("the API account lacks the Store Translations (modify) scope: {e:#}"),
                )
            },
        });
        out
    }

    fn verify_webhook(&self, headers: &HeaderMap, _body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
    }

    async fn register_webhooks(&self, creds: &Creds, url: &str) -> anyhow::Result<()> {
        let secret = creds.opt("webhook_secret").ok_or_else(|| {
            anyhow::anyhow!("no webhook secret: BigCommerce webhooks could not be authenticated, none registered")
        })?;
        let s = Store::new(creds).await?;
        let existing = s.call(reqwest::Method::GET, "/v3/hooks", None).await?;
        for scope in [
            "store/product/created",
            "store/product/updated",
            "store/category/created",
            "store/category/updated",
        ] {
            let has = existing["data"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|h| h["scope"] == scope && h["destination"] == url);
            if has {
                continue;
            }
            s.call(
                reqwest::Method::POST,
                "/v3/hooks",
                Some(
                    &json!({"scope": scope, "destination": url, "is_active": true,
                    "headers": {"X-Glot-Webhook-Secret": secret}}),
                ),
            )
            .await?;
        }
        Ok(())
    }

    fn push_batch(&self) -> usize {
        50
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Query, State},
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn fields_typed_and_filtered() {
        let o = json!({"id": 42, "name": "Shirt", "sku": "SH-1", "description": "<p>Soft <b>cotton</b></p>",
            "page_title": "", "meta_description": "123", "search_keywords": "shirt, cotton", "custom_url": {"url": "/shirt/"}});
        let items = items_of(&RESOURCES[0], &o);
        assert_eq!(
            items.iter().map(|i| i.field.as_str()).collect::<Vec<_>>(),
            ["name", "description", "search_keywords"]
        );
        assert_eq!(items[0].key, "bigcommerce:product:42:name");
        assert_eq!(items[1].format, Format::Html);
        assert_eq!(items[1].meta["rid"], "bc/store/product/42");
        assert_eq!(items[1].meta["type"], "PRODUCTS");
        let cat = items_of(&RESOURCES[1], &json!({"category_id": 7, "name": "Summer"}));
        assert_eq!(cat[0].meta["rid"], "bc/store/category/7");
    }

    #[derive(Clone, Default)]
    struct Mock {
        gql: Arc<Mutex<Vec<Value>>>,
        hooks: Arc<Mutex<Vec<Value>>>,
        limited: Arc<Mutex<bool>>,
    }

    fn auth(h: &HeaderMap) -> bool {
        h.get("x-auth-token").and_then(|v| v.to_str().ok()) == Some("tok")
    }

    async fn products(
        State(m): State<Mock>,
        h: HeaderMap,
        Query(q): Query<HashMap<String, String>>,
    ) -> axum::response::Response {
        if !auth(&h) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"status": 401, "title": "Unauthorized"})),
            )
                .into_response();
        }
        {
            let mut l = m.limited.lock().unwrap();
            if !*l {
                *l = true;
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [
                        ("x-rate-limit-time-reset-ms", "50"),
                        ("x-rate-limit-requests-left", "0"),
                    ],
                    "",
                )
                    .into_response();
            }
        }
        let page: u32 = q["page"].parse().unwrap();
        let data = if page == 1 {
            json!([{"id": 1, "name": "Shirt", "description": "<p>Soft <b>cotton</b> shirt.</p>", "sku": "S1"}])
        } else {
            json!([{"id": 2, "name": "Hat", "description": ""}])
        };
        (
            [
                ("x-rate-limit-requests-left", "100"),
                ("x-rate-limit-time-reset-ms", "1000"),
            ],
            Json(
                json!({"data": data, "meta": {"pagination": {"total": 2, "count": 1, "per_page": 1,
                "current_page": page, "total_pages": 2, "links": {}}}}),
            ),
        )
            .into_response()
    }

    async fn graphql(State(m): State<Mock>, h: HeaderMap, Json(b): Json<Value>) -> Json<Value> {
        assert!(auth(&h));
        let q = b["query"].as_str().unwrap_or_default();
        m.gql.lock().unwrap().push(b.clone());
        Json(if q.contains("updateTranslations") {
            json!({"data": {"translation": {"updateTranslations": {"__typename": "UpdateTranslationsResult", "errors": []}}}})
        } else if q.contains("addLocale") {
            json!({"data": {"locale": {"addLocale": {"locale": {"code": "fr", "status": "INACTIVE"}, "errors": []}}}})
        } else {
            assert_eq!(b["variables"]["channel"], "bc/store/channel/1");
            json!({"data": {"store": {"locales": {"edges": [{"node": {"code": "en", "status": "ACTIVE", "isDefault": true}}]}}}})
        })
    }

    async fn serve() -> (String, Mock) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") });
        let m = Mock::default();
        let app = Router::new()
            .route("/v3/catalog/products", get(products))
            .route(
                "/v3/catalog/trees/categories",
                get(|| async {
                    Json(json!({"data": [{"category_id": 9, "name": "Summer", "description": "<p>Hot</p>"}],
                        "meta": {"pagination": {"total_pages": 1}}}))
                }),
            )
            .route(
                "/v3/catalog/brands",
                get(|| async { Json(json!({"data": [], "meta": {"pagination": {"total_pages": 0}}})) }),
            )
            .route("/v2/store", get(|| async { Json(json!({"name": "Test Store"})) }))
            .route("/graphql", post(graphql))
            .route(
                "/v3/hooks",
                get(|| async { Json(json!({"data": []})) }).post(
                    |State(m): State<Mock>, Json(b): Json<Value>| async move {
                        m.hooks.lock().unwrap().push(b);
                        Json(json!({"data": {"id": 1}}))
                    },
                ),
            )
            .with_state(m.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (format!("http://{addr}"), m)
    }

    #[tokio::test]
    async fn pull_push_webhooks_against_mock() {
        let (base, m) = serve().await;
        let creds = Creds {
            secrets: json!({"store_hash": base, "token": "tok", "webhook_secret": "whs"}),
            ..Default::default()
        };
        let c = BigCommerce;
        let info = c.test(&creds).await.unwrap();
        assert_eq!(info.name, "Test Store");
        assert_eq!(info.source_locale.as_deref(), Some("en"));

        // Pull: 429 waited out, two product pages, then categories, then brands.
        let mut all = vec![];
        let mut cursor = None;
        loop {
            let p = c.pull(&creds, cursor).await.unwrap();
            all.extend(p.items);
            cursor = p.next;
            if cursor.is_none() {
                break;
            }
        }
        let keys: Vec<&str> = all.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "bigcommerce:product:1:name",
                "bigcommerce:product:1:description",
                "bigcommerce:product:2:name",
                "bigcommerce:category:9:name",
                "bigcommerce:category:9:description",
            ]
        );

        let push: Vec<PushItem> = all
            .iter()
            .filter(|i| i.external_id != "product:2")
            .map(|i| PushItem {
                external_id: i.external_id.clone(),
                field: i.field.clone(),
                value: if i.field == "description" {
                    i.text.replace("Soft", "Doux").replace("shirt", "t-shirt")
                } else {
                    format!("{} FR", i.text)
                },
                format: i.format.clone(),
                meta: i.meta.clone(),
            })
            .collect();
        let r = c.push(&creds, "fr", &push).await.unwrap();
        assert_eq!(r.ok, 4);
        assert!(r.failed.is_empty());
        let calls = m.gql.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|b| b["query"].as_str().unwrap().contains("addLocale")
                    && b["variables"]["input"]["code"] == "fr")
        );
        let updates: Vec<&Value> = calls
            .iter()
            .filter(|b| b["query"].as_str().unwrap().contains("updateTranslations"))
            .map(|b| &b["variables"]["input"])
            .collect();
        assert_eq!(updates.len(), 2, "one call per resource type");
        let p = updates
            .iter()
            .find(|u| u["resourceType"] == "PRODUCTS")
            .unwrap();
        assert_eq!(p["channelId"], "bc/store/channel/1");
        assert_eq!(p["localeId"], "bc/store/locale/fr");
        assert_eq!(p["entities"][0]["resourceId"], "bc/store/product/1");
        let desc = p["entities"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["fieldName"] == "description")
            .unwrap();
        assert_eq!(
            desc["value"], "<p>Doux <b>cotton</b> t-shirt.</p>",
            "HTML kept"
        );

        // Default locale refused.
        assert!(c.push(&creds, "en", &push).await.is_err());

        c.register_webhooks(&creds, "https://glot.example/api/connectors/x/webhook")
            .await
            .unwrap();
        let hooks = m.hooks.lock().unwrap().clone();
        assert_eq!(hooks.len(), 4);
        assert_eq!(hooks[0]["headers"]["X-Glot-Webhook-Secret"], "whs");
        let mut h = HeaderMap::new();
        h.insert("x-glot-webhook-secret", "whs".parse().unwrap());
        assert!(c.verify_webhook(&h, b"{}", "whs"));
        assert!(!c.verify_webhook(&HeaderMap::new(), b"{}", "whs"));

        let bad = Creds {
            secrets: json!({"store_hash": base, "token": "nope"}),
            ..Default::default()
        };
        assert!(c.pull(&bad, Some("0|1".into())).await.is_err());
        assert!(
            Store::new(&Creds {
                secrets: json!({"store_hash": "evil.com/x", "token": "t"}),
                ..Default::default()
            })
            .await
            .is_err()
        );
    }
}
