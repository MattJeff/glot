//! WooCommerce: REST API `wc/v3` with a REST API key (HTTP Basic `consumer_key:consumer_secret`, HTTPS only),
//! translations stored by the site's multilingual plugin, as for WordPress ([`super::wordpress`] does the
//! plugin detection, languages and translation links).
//!
//! - Pull (`context=edit`, raw values): products (`name`, `short_description`, `description`: HTML), product
//!   categories and tags (`name`, `description`). Paginated by `page`/`per_page` (`X-WP-TotalPages`),
//!   products incremental with `modified_after`. Only source-language content (`lang`). SKUs, slugs,
//!   attributes and variations are not pulled (WooCommerce Multilingual synchronizes variations from the
//!   translated parent).
//! - Push: the translation found in the item's `translations` map (fresh GET) is updated in place
//!   (`PUT /wc/v3/products/{id}`); a missing one is created linked to its source:
//!   - WPML (WooCommerce Multilingual): `POST /wc/v3/products` with `lang` and `translation_of` in the body
//!     (documented for products and taxonomy terms);
//!   - Polylang for WooCommerce: `?lang=fr&translations[en]=12`, Polylang Pro's REST linking (documented by
//!     Polylang for `wp/v2` only: best effort on `wc/v3`).
//!   - New products copy the source's `type` and `regular_price`, with `settings.new_status` (default `draft`).
//! - Human edits and rollback: [`super::wordpress::read_back`] (sources then translations by `include`, the
//!   translated object's field is the current value; empty or identical to the source = none). `remove` is
//!   not implemented: a translation is a whole product or term.
//! - Diagnosis: the multilingual plugin and its WooCommerce companion, from the REST index and
//!   `GET /wc/v3/system_status` (`active_plugins`; the key's user must be able to manage WooCommerce). The
//!   key's permission (Read / Write / Read-Write) is exposed by no endpoint: a read-only key shows on the
//!   first write only.
//! - Webhooks: with the optional `webhook_secret` credential, `product.created` / `product.updated` are
//!   subscribed (`POST /wc/v3/webhooks`, that secret) and checked with `X-WC-Webhook-Signature` (base64
//!   HMAC-SHA256 of the body). Categories and tags have no webhook topics.
//! - Rate limits: none in the REST API itself; `429`/`503` (hosts, security plugins) are retried by [`super::send`].
//!
//! Docs: https://woocommerce.github.io/woocommerce-rest-api-docs/#authentication,
//! https://developer.woocommerce.com/docs/apis/rest-api/v3/products/,
//! https://developer.woocommerce.com/docs/apis/rest-api/v3/webhooks/,
//! https://woocommerce.github.io/woocommerce-rest-api-docs/#list-all-products (`include`),
//! https://woocommerce.github.io/woocommerce-rest-api-docs/#system-status-properties,
//! https://developer.wordpress.org/rest-api/using-the-rest-api/global-parameters/#_fields,
//! https://wpml.org/documentation/related-projects/woocommerce-multilingual/using-wordpress-rest-api-woocommerce-multilingual/,
//! https://polylang.pro/doc/rest-api/
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::wordpress::{Plugin, Site, plugin_check, raw, read_back, translation_for, translations};
use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem,
};

pub struct WooCommerce;

const PER_PAGE: u32 = 100;

/// (REST collection under `/wc/v3/`, key resource name, fields with their HTML flag).
const RESOURCES: &[(&str, &str, &[(&str, bool)])] = &[
    (
        "products",
        "product",
        &[
            ("name", false),
            ("short_description", true),
            ("description", true),
        ],
    ),
    (
        "products/categories",
        "category",
        &[("name", false), ("description", true)],
    ),
    (
        "products/tags",
        "tag",
        &[("name", false), ("description", true)],
    ),
];

async fn site(creds: &Creds) -> anyhow::Result<Site> {
    let base = creds.get("url")?.trim_end_matches('/').to_string();
    let base = base
        .strip_suffix("/wp-json")
        .map(str::to_string)
        .unwrap_or(base);
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            creds.get("consumer_key")?,
            creds.get("consumer_secret")?
        ))
    );
    let client = super::client_for(&base).await?;
    Ok(Site { base, auth, client })
}

fn items_of(
    collection: &str,
    resource: &str,
    fields: &[(&str, bool)],
    o: &Value,
) -> Vec<SourceItem> {
    let Some(id) = o["id"].as_i64() else {
        return vec![];
    };
    let title = raw(o, "name").unwrap_or_default();
    fields
        .iter()
        .filter_map(|(f, html)| {
            let v = raw(o, f)?;
            if v.trim().is_empty() {
                return None;
            }
            Some(SourceItem {
                external_id: format!("{resource}:{id}"),
                field: f.to_string(),
                key: super::key("woocommerce", resource, &id.to_string(), f),
                text: v,
                format: if *html { Format::Html } else { Format::Plain },
                context: format!("WooCommerce {resource} \u{ab}{title}\u{bb}, field {f}"),
                updated_at: None,
                meta: json!({"collection": collection, "id": id, "translations": translations(o)}),
            })
        })
        .collect()
}

#[async_trait]
impl Connector for WooCommerce {
    fn kind(&self) -> &'static str {
        "woocommerce"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let site = site(creds).await?;
        let (index, _) = site.get("/").await?;
        // Authenticated: a wrong key fails here (the index is public).
        site.get("/wc/v3/products?per_page=1&context=edit").await?;
        let plugin = site.plugin(creds).await?;
        let (langs, src) = site.languages(creds, plugin).await?;
        let mut notes = vec![match plugin {
            Plugin::Polylang => format!("Polylang detected, languages: {}", langs.join(", ")),
            Plugin::Wpml => "WPML (WooCommerce Multilingual) detected".to_string(),
            Plugin::None => "No multilingual plugin with a REST API (WooCommerce Multilingual or Polylang for WooCommerce) detected: translations cannot be written.".to_string(),
        }];
        if plugin == Plugin::Wpml && langs.is_empty() {
            notes.push(
                "Set settings.languages and settings.source_lang (WPML has no languages endpoint)."
                    .into(),
            );
        }
        if creds.opt("webhook_secret").is_none() {
            notes.push(
                "No webhook secret: webhooks are not registered; sync manually or periodically."
                    .into(),
            );
        }
        Ok(AccountInfo {
            name: index["name"].as_str().unwrap_or("WooCommerce").to_string(),
            source_locale: src,
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let site = site(creds).await?;
        let plugin = site.plugin(creds).await?;
        Ok(site.languages(creds, plugin).await?.0)
    }

    /// Cursor = `{resource index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let site = site(creds).await?;
        let plugin = site.plugin(creds).await?;
        let (_, src) = site.languages(creds, plugin).await?;
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u32>()?),
            None => (0, 1),
        };
        let Some((collection, resource, fields)) = RESOURCES.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let mut q = format!(
            "/wc/v3/{collection}?context=edit&per_page={PER_PAGE}&page={page}&orderby=id&order=asc"
        );
        if collection == "products"
            && let Some(since) = creds.since
        {
            q.push_str(&format!(
                "&modified_after={}&dates_are_gmt=true",
                since.format("%Y-%m-%dT%H:%M:%S")
            ));
        }
        if let (Some(l), true) = (&src, plugin != Plugin::None) {
            q.push_str(&format!("&lang={l}"));
        }
        let (list, headers) = site.get(&q).await?;
        let total: u32 = headers
            .get("x-wp-totalpages")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let items = list
            .as_array()
            .into_iter()
            .flatten()
            // Never pull a translation as a source.
            .filter(|o| match (&src, o["lang"].as_str()) {
                (Some(s), Some(l)) => l == s,
                _ => true,
            })
            .flat_map(|o| items_of(collection, resource, fields, o))
            .collect();
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
        let site = site(creds).await?;
        let plugin = site.plugin(creds).await?;
        anyhow::ensure!(
            plugin != Plugin::None,
            "no multilingual plugin (WooCommerce Multilingual or Polylang for WooCommerce) on this site: translations cannot be stored"
        );
        let (_, src) = site.languages(creds, plugin).await?;
        let new_status = creds
            .setting("new_status")
            .and_then(Value::as_str)
            .unwrap_or("draft");
        let mut by_resource: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_resource.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by_resource {
            let meta = &its[0].meta;
            let (Some(coll), Some(id)) = (meta["collection"].as_str(), meta["id"].as_i64()) else {
                report
                    .failed
                    .push((ext.to_string(), "missing metadata, sync again".into()));
                continue;
            };
            let mut body = serde_json::Map::new();
            for it in &its {
                body.insert(it.field.clone(), Value::String(it.value.clone()));
            }
            let res: anyhow::Result<()> = async {
                // Fresh links: a translation created by an earlier batch is not in the pulled meta.
                let (current, _) = site.get(&format!("/wc/v3/{coll}/{id}?context=edit")).await?;
                match translation_for(&translations(&current), locale).filter(|t| *t != id) {
                    Some(t) => {
                        site.call(reqwest::Method::PUT, &format!("/wc/v3/{coll}/{t}"), Some(&Value::Object(body)))
                            .await?;
                    }
                    None => {
                        if coll == "products" {
                            let status = if new_status == "same" {
                                current["status"].as_str().unwrap_or("draft")
                            } else {
                                new_status
                            };
                            body.insert("status".into(), status.into());
                            for f in ["type", "regular_price"] {
                                if let Some(v) = current.get(f).filter(|v| v.is_string()) {
                                    body.insert(f.into(), v.clone());
                                }
                            }
                        } else if !body.contains_key("name") {
                            anyhow::bail!("no {locale} translation yet and no translated name to create it with");
                        }
                        let path = match plugin {
                            Plugin::Polylang => {
                                let s = src.clone().or_else(|| current["lang"].as_str().map(str::to_string)).unwrap_or_default();
                                format!("/wc/v3/{coll}?lang={locale}&translations[{s}]={id}")
                            }
                            _ => {
                                body.insert("lang".into(), locale.into());
                                body.insert("translation_of".into(), id.into());
                                format!("/wc/v3/{coll}")
                            }
                        };
                        site.call(reqwest::Method::POST, &path, Some(&Value::Object(body))).await?;
                    }
                }
                Ok(())
            }
            .await;
            match res {
                Ok(()) => report.ok += its.len(),
                Err(e) => {
                    for it in &its {
                        report
                            .failed
                            .push((format!("{}/{}", it.external_id, it.field), format!("{e:#}")));
                    }
                }
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
        let site = site(creds).await?;
        let place = |m: &Value| {
            Some((
                format!("/wc/v3/{}", m["collection"].as_str()?),
                m["id"].as_i64()?,
            ))
        };
        Ok(Some(read_back(&site, locale, items, place).await?))
    }

    async fn doctor(&self, creds: &Creds) -> Vec<Check> {
        let site = match site(creds).await {
            Ok(s) => s,
            Err(e) => return vec![Check::fail("plugin", format!("{e:#}"))],
        };
        let mut out = vec![];
        let plugins = match site
            .get("/wc/v3/system_status?_fields=active_plugins")
            .await
        {
            Ok((v, _)) => Some(Site::plugin_files(&v["active_plugins"])),
            Err(e) => {
                out.push(Check::warn(
                    "permissions",
                    format!("the key's user cannot read the system status (Administrator or Shop manager needed): {e:#}"),
                    Some("permissions"),
                ));
                None
            }
        };
        out.push(match site.plugin(creds).await {
            Ok(plugin) => plugin_check(plugin, plugins.as_deref(), true),
            Err(e) => Check::fail("plugin", format!("{e:#}")),
        });
        out
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        super::cms::shared_secret_ok(headers, secret)
            || super::cms::header(headers, "x-wc-webhook-signature")
                .is_some_and(|sig| super::hmac_ok(secret, body, sig))
    }

    async fn register_webhooks(&self, creds: &Creds, url: &str) -> anyhow::Result<()> {
        let Some(secret) = creds.opt("webhook_secret") else {
            anyhow::bail!(
                "no webhook secret: WooCommerce webhooks could not be verified, none registered"
            );
        };
        let site = site(creds).await?;
        let (existing, _) = site.get("/wc/v3/webhooks?per_page=100").await?;
        for topic in ["product.created", "product.updated"] {
            let has = existing.as_array().into_iter().flatten().any(|h| {
                h["topic"] == topic && h["delivery_url"] == url && h["status"] != "disabled"
            });
            if has {
                continue;
            }
            site.call(
                reqwest::Method::POST,
                "/wc/v3/webhooks",
                Some(&json!({"name": format!("glot: {topic}"), "topic": topic, "delivery_url": url, "secret": secret})),
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path, Query, RawQuery, State},
        http::StatusCode,
        routing::get,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

    fn auth_ok(h: &HeaderMap) -> bool {
        let want = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("ck_test:cs_test")
        );
        h.get("authorization").and_then(|v| v.to_str().ok()) == Some(want.as_str())
    }

    fn product(id: i64, lang: &str, name: &str, tr: Value) -> Value {
        json!({"id": id, "lang": lang, "name": name, "type": "simple", "status": "publish", "regular_price": "10.00",
            "sku": "SKU-1", "slug": "shirt", "short_description": "", "translations": tr,
            "description": "<p>Soft <strong>cotton</strong>.</p>\n<p><img src=\"/a.png\" alt=\"\"> Wash cold.</p>"})
    }

    /// WooCommerce + WPML (WooCommerce Multilingual) per the docs: Basic auth, `X-WP-TotalPages`, `lang`,
    /// `translations` map, `translation_of` on create.
    async fn mock(plugin: &'static str) -> (String, Log) {
        let log: Log = Arc::default();
        let ns = if plugin == "wpml" {
            json!(["wp/v2", "wc/v3", "wpml/v1"])
        } else {
            json!(["wp/v2", "wc/v3", "pll/v1"])
        };
        let app = Router::new()
            .route("/wp-json/", get(move || async move { Json(json!({"name": "Shop", "namespaces": ns})) }))
            .route("/wp-json/pll/v1/languages", get(|| async { Json(json!([{"slug": "en", "is_default": true}, {"slug": "fr"}])) }))
            .route(
                "/wp-json/wc/v3/{*rest}",
                get(|h: HeaderMap, Path(rest): Path<String>, Query(q): Query<HashMap<String, String>>| async move {
                    if !auth_ok(&h) {
                        return (StatusCode::UNAUTHORIZED, [("x-wp-totalpages", "1")], Json(json!({"code": "woocommerce_rest_cannot_view"})));
                    }
                    let page = q.get("page").map(String::as_str).unwrap_or("1");
                    let body = match (rest.as_str(), page) {
                        ("products", "1") if q.contains_key("page") => {
                            assert_eq!(q["lang"], "en");
                            assert_eq!(q["context"], "edit");
                            json!([product(1, "en", "Shirt", json!({"en": 1, "fr": 5})), product(5, "fr", "Chemise", json!({"en": 1, "fr": 5}))])
                        }
                        ("products", "2") => json!([product(2, "en", "Hat", json!({"en": 2}))]),
                        ("products", _) => json!([]),
                        ("products/1", _) => product(1, "en", "Shirt", json!({"en": 1, "fr": 5})),
                        ("products/2", _) => product(2, "en", "Hat", json!({"en": 2})),
                        ("products/categories", _) => json!([{"id": 9, "lang": "en", "name": "Summer", "description": "", "translations": {"en": 9}}]),
                        ("products/categories/9", _) => json!({"id": 9, "lang": "en", "name": "Summer", "translations": {"en": 9}}),
                        ("webhooks", _) => json!([{"id": 1, "topic": "product.created", "delivery_url": "https://glot.test/hook", "status": "active"}]),
                        _ => json!([]),
                    };
                    let pages = if rest == "products" { "2" } else { "1" };
                    (StatusCode::OK, [("x-wp-totalpages", pages)], Json(body))
                })
                .post(|State(log): State<Log>, h: HeaderMap, Path(rest): Path<String>, RawQuery(q): RawQuery, Json(b): Json<Value>| async move {
                    assert!(auth_ok(&h));
                    log.lock().unwrap().push((format!("POST {rest}?{}", q.unwrap_or_default()), String::new(), b));
                    (StatusCode::CREATED, Json(json!({"id": 50})))
                })
                .put(|State(log): State<Log>, h: HeaderMap, Path(rest): Path<String>, Json(b): Json<Value>| async move {
                    assert!(auth_ok(&h));
                    log.lock().unwrap().push((format!("PUT {rest}"), String::new(), b));
                    Json(json!({"id": 5}))
                }),
            )
            .with_state(log.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (format!("http://{addr}"), log)
    }

    fn creds(url: &str, settings: Value) -> Creds {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") });
        Creds {
            secrets: json!({"url": url, "consumer_key": "ck_test", "consumer_secret": "cs_test", "webhook_secret": "whs"}),
            settings,
            since: None,
        }
    }

    #[tokio::test]
    async fn wpml_pull_push_webhooks() {
        let (url, log) = mock("wpml").await;
        let c = creds(
            &url,
            json!({"languages": ["en", "fr"], "source_lang": "en"}),
        );
        let w = WooCommerce;
        let info = w.test(&c).await.unwrap();
        assert_eq!(info.source_locale.as_deref(), Some("en"));
        let bad = Creds {
            secrets: json!({"url": url, "consumer_key": "ck_test", "consumer_secret": "nope"}),
            ..c.clone()
        };
        assert!(w.test(&bad).await.is_err());

        // Pull: 2 product pages, then categories, then tags; the fr product is never a source.
        let mut items = vec![];
        let mut cursor = None;
        loop {
            let p = w.pull(&c, cursor).await.unwrap();
            items.extend(p.items);
            cursor = p.next;
            if cursor.is_none() {
                break;
            }
        }
        let keys: Vec<_> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "woocommerce:product:1:name",
                "woocommerce:product:1:description",
                "woocommerce:product:2:name",
                "woocommerce:product:2:description",
                "woocommerce:category:9:name"
            ]
        );
        assert_eq!(items[1].format, Format::Html);

        // Push: product 1 has a fr translation (updated in place), product 2 / category 9 are created linked.
        let html = "<p>Coton <strong>doux</strong>.</p>\n<p><img src=\"/a.png\" alt=\"\"> Laver à froid.</p>";
        let push: Vec<PushItem> = items
            .iter()
            .map(|i| PushItem {
                external_id: i.external_id.clone(),
                field: i.field.clone(),
                value: if i.field == "description" {
                    html.into()
                } else {
                    format!("{} FR", i.text)
                },
                format: i.format.clone(),
                meta: i.meta.clone(),
            })
            .collect();
        let r = w.push(&c, "fr", &push).await.unwrap();
        assert_eq!((r.ok, r.failed.len()), (5, 0));
        let snap = log.lock().unwrap().clone();
        let put = snap.iter().find(|l| l.0 == "PUT products/5").unwrap();
        assert_eq!(put.2["name"], "Shirt FR");
        assert_eq!(put.2["description"], html);
        let created = snap.iter().find(|l| l.0 == "POST products?").unwrap();
        assert_eq!(created.2["lang"], "fr");
        assert_eq!(created.2["translation_of"], 2);
        assert_eq!(created.2["status"], "draft");
        assert_eq!(created.2["regular_price"], "10.00");
        assert!(created.2.get("sku").is_none());
        let cat = snap
            .iter()
            .find(|l| l.0 == "POST products/categories?")
            .unwrap();
        assert_eq!(
            (cat.2["name"].as_str(), cat.2["translation_of"].as_i64()),
            (Some("Summer FR"), Some(9))
        );

        // Webhooks: product.created exists, only product.updated is added, with the secret.
        w.register_webhooks(&c, "https://glot.test/hook")
            .await
            .unwrap();
        let log = log.lock().unwrap();
        let hooks: Vec<_> = log
            .iter()
            .filter(|l| l.0.starts_with("POST webhooks"))
            .collect();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].2["topic"], "product.updated");
        assert_eq!(hooks[0].2["secret"], "whs");
    }

    #[tokio::test]
    async fn polylang_links_and_webhook_registration() {
        let (url, log) = mock("polylang").await;
        let c = creds(&url, json!({}));
        let w = WooCommerce;
        assert_eq!(w.locales(&c).await.unwrap(), ["en", "fr"]);
        let p = w.pull(&c, Some("1|1".into())).await.unwrap();
        let r = w
            .push(
                &c,
                "fr",
                &[PushItem {
                    value: "Été".into(),
                    ..push_of(&p.items[0])
                }],
            )
            .await
            .unwrap();
        assert_eq!(r.ok, 1);
        w.register_webhooks(&c, "https://glot.test/hook")
            .await
            .unwrap();
        let log = log.lock().unwrap();
        assert_eq!(
            log[0].0,
            "POST products/categories?lang=fr&translations[en]=9"
        );
        assert!(log[0].2.get("translation_of").is_none());
        let hooks: Vec<_> = log
            .iter()
            .filter(|l| l.0.starts_with("POST webhooks"))
            .collect();
        assert_eq!(hooks.len(), 1);
        assert_eq!(
            hooks[0].2,
            json!({"name": "glot: product.updated", "topic": "product.updated", "delivery_url": "https://glot.test/hook", "secret": "whs"})
        );
    }

    fn push_of(i: &SourceItem) -> PushItem {
        PushItem {
            external_id: i.external_id.clone(),
            field: i.field.clone(),
            value: String::new(),
            format: i.format.clone(),
            meta: i.meta.clone(),
        }
    }

    #[test]
    fn webhook_signature() {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"whs").unwrap();
        mac.update(b"{\"id\":1}");
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let mut h = HeaderMap::new();
        h.insert("x-wc-webhook-signature", sig.parse().unwrap());
        assert!(WooCommerce.verify_webhook(&h, b"{\"id\":1}", "whs"));
        assert!(!WooCommerce.verify_webhook(&h, b"{\"id\":2}", "whs"));
        assert!(!WooCommerce.verify_webhook(&HeaderMap::new(), b"{}", "whs"));
    }
}
