//! Magento Open Source / Adobe Commerce (on-premises, PaaS): REST API, one store view per language.
//!
//! - Auth: an integration (System → Extensions → Integrations). With its four credentials glot signs every
//!   call with OAuth 1.0a HMAC-SHA256 (recommended). With the access token alone it sends
//!   `Authorization: Bearer`, which Commerce 2.4.4+ refuses unless *Allow OAuth Access Tokens to be used as
//!   standalone Bearer tokens* is on (`oauth/consumer/enable_integration_as_bearer`).
//! - Store views: `GET /V1/store/storeConfigs` (`code`, `locale`). Source = `settings.source_store`, else the
//!   `default` store view. A glot locale is written to every store view whose code equals it, whose locale
//!   equals it (`fr-FR` ↔ `fr_FR`) or, for a locale without region, whose locale language matches (`fr` → `fr_FR`).
//! - Pull (source store view values): products (`name`, `description`, `short_description` as HTML,
//!   `meta_title`, `meta_keyword`, `meta_description`) and categories below the root (`name`, `description`,
//!   meta fields), `searchCriteria` paging (`pageSize`/`currentPage`, `total_count`), incremental with an
//!   `updated_at gteq` filter. SKU, `url_key` and other attributes are never read or written.
//! - Push: `PUT /rest/{store_code}/V1/products/{sku}` and `PUT /rest/{store_code}/V1/categories/{id}` with only
//!   the translated attributes, so only those get store-view values (everything else keeps inheriting).
//! - CMS pages and blocks are not synced: they are assigned to store views as whole entities (one page per
//!   language), they have no per-store-view values to write into.
//! - Webhooks: none in Magento Open Source (Adobe Commerce webhooks are synchronous plugins): an extension or
//!   cron calls the account's webhook URL with its token, or signs with `X-Glot-Signature`.
//!
//! Docs: https://developer.adobe.com/commerce/webapi/get-started/authentication/gs-authentication-token,
//! https://developer.adobe.com/commerce/webapi/get-started/authentication/gs-authentication-oauth/,
//! https://developer.adobe.com/commerce/webapi/rest/use-rest/performing-searches/,
//! https://developer.adobe.com/commerce/webapi/rest/use-rest/bulk-endpoints/ (store code scopes),
//! https://developer.adobe.com/commerce/webapi/rest/tutorials/image/product-image-store-scope,
//! https://developer.adobe.com/commerce/webapi/get-started/rate-limiting
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, cms,
};

pub struct Magento;

const PAGE: u32 = 50;

/// (resource, REST list path, fields with HTML flag). `name` is a top-level field, the rest custom attributes.
const RESOURCES: &[(&str, &str, &[(&str, bool)])] = &[
    (
        "product",
        "/products",
        &[
            ("name", false),
            ("description", true),
            ("short_description", true),
            ("meta_title", false),
            ("meta_keyword", false),
            ("meta_description", false),
        ],
    ),
    (
        "category",
        "/categories/list",
        &[
            ("name", false),
            ("description", true),
            ("meta_title", false),
            ("meta_keywords", false),
            ("meta_description", false),
        ],
    ),
];

/// RFC 3986 percent-encoding (OAuth 1.0a, path segments, query values).
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

struct OAuth {
    consumer_key: String,
    consumer_secret: String,
    token: String,
    token_secret: String,
}

/// RFC 5849 signature base string: `METHOD&enc(url)&enc(sorted encoded params)`.
fn base_string(method: &str, url: &str, params: &[(String, String)]) -> String {
    let mut p: Vec<(String, String)> = params.iter().map(|(k, v)| (enc(k), enc(v))).collect();
    p.sort();
    let joined: Vec<String> = p.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!(
        "{}&{}&{}",
        method.to_uppercase(),
        enc(url),
        enc(&joined.join("&"))
    )
}

impl OAuth {
    fn header(&self, method: &str, url: &str, query: &[(String, String)]) -> String {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let mut oauth = vec![
            ("oauth_consumer_key".to_string(), self.consumer_key.clone()),
            (
                "oauth_nonce".into(),
                hex::encode(rand::random::<[u8; 16]>()),
            ),
            ("oauth_signature_method".into(), "HMAC-SHA256".into()),
            (
                "oauth_timestamp".into(),
                chrono::Utc::now().timestamp().to_string(),
            ),
            ("oauth_token".into(), self.token.clone()),
            ("oauth_version".into(), "1.0".into()),
        ];
        let mut all = oauth.clone();
        all.extend_from_slice(query);
        let key = format!("{}&{}", enc(&self.consumer_secret), enc(&self.token_secret));
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).expect("hmac key");
        mac.update(base_string(method, url, &all).as_bytes());
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        oauth.push(("oauth_signature".into(), sig));
        let parts: Vec<String> = oauth
            .iter()
            .map(|(k, v)| format!("{k}=\"{}\"", enc(v)))
            .collect();
        format!("OAuth {}", parts.join(", "))
    }
}

struct Api {
    base: String,
    client: reqwest::Client,
    oauth: Option<OAuth>,
    bearer: String,
}

impl Api {
    async fn new(creds: &Creds) -> anyhow::Result<Self> {
        let base = creds.get("url")?.trim_end_matches('/');
        let base = base.strip_suffix("/rest").unwrap_or(base).to_string();
        let token = creds.get("token")?.to_string();
        let oauth = match (
            creds.opt("consumer_key"),
            creds.opt("consumer_secret"),
            creds.opt("token_secret"),
        ) {
            (Some(k), Some(s), Some(ts)) => Some(OAuth {
                consumer_key: k.into(),
                consumer_secret: s.into(),
                token: token.clone(),
                token_secret: ts.into(),
            }),
            (None, None, None) => None,
            _ => anyhow::bail!(
                "OAuth needs the consumer key, consumer secret and access token secret together"
            ),
        };
        let client = super::client_for(&base).await?;
        Ok(Self {
            base,
            client,
            oauth,
            bearer: format!("Bearer {token}"),
        })
    }

    /// `store` = store view code, `None` = the default scope (`/rest/V1/…`).
    async fn call(
        &self,
        method: reqwest::Method,
        store: Option<&str>,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = match store {
            Some(s) => format!("{}/rest/{}/V1{path}", self.base, enc(s)),
            None => format!("{}/rest/V1{path}", self.base),
        };
        let qs: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
            .collect();
        let full = if qs.is_empty() {
            url.clone()
        } else {
            format!("{url}?{}", qs.join("&"))
        };
        let res = super::send(|| {
            let auth = match &self.oauth {
                Some(o) => o.header(method.as_str(), &url, query),
                None => self.bearer.clone(),
            };
            let r = self
                .client
                .request(method.clone(), &full)
                .header("Authorization", auth);
            match body {
                Some(b) => r.json(b),
                None => r,
            }
        })
        .await?;
        Ok(res.json().await?)
    }

    /// (code, locale) of every store view (admin excluded).
    async fn stores(&self) -> anyhow::Result<Vec<(String, String)>> {
        let v = self
            .call(reqwest::Method::GET, None, "/store/storeConfigs", &[], None)
            .await?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["id"].as_i64() != Some(0) && s["code"] != "admin")
            .filter_map(|s| {
                Some((
                    s["code"].as_str()?.to_string(),
                    s["locale"].as_str().unwrap_or_default().to_string(),
                ))
            })
            .collect())
    }
}

fn source_store<'a>(creds: &'a Creds, stores: &'a [(String, String)]) -> anyhow::Result<&'a str> {
    if let Some(s) = cms::param(creds, "source_store") {
        anyhow::ensure!(
            stores.iter().any(|(c, _)| c == s),
            "store view '{s}' not found"
        );
        return Ok(s);
    }
    stores
        .iter()
        .find(|(c, _)| c == "default")
        .or(stores.first())
        .map(|(c, _)| c.as_str())
        .ok_or_else(|| anyhow::anyhow!("no store view found"))
}

/// Store views a glot locale is written to (see the module doc).
fn targets<'a>(stores: &'a [(String, String)], source: &str, locale: &str) -> Vec<&'a str> {
    let norm = |s: &str| s.replace('-', "_").to_ascii_lowercase();
    let want = norm(locale);
    let lang = |s: &str| s.split('_').next().unwrap_or("").to_string();
    stores
        .iter()
        .filter(|(c, _)| c != source)
        .filter(|(c, l)| {
            norm(c) == want || norm(l) == want || (!want.contains('_') && lang(&norm(l)) == want)
        })
        .map(|(c, _)| c.as_str())
        .collect()
}

fn attr<'a>(o: &'a Value, field: &str) -> Option<&'a str> {
    if field == "name" {
        return o["name"].as_str();
    }
    o["custom_attributes"]
        .as_array()?
        .iter()
        .find(|a| a["attribute_code"] == field)?["value"]
        .as_str()
}

fn items_of(resource: &str, fields: &[(&str, bool)], o: &Value) -> Vec<SourceItem> {
    let Some(id) = o["id"].as_i64() else {
        return vec![];
    };
    let sku = o["sku"].as_str().unwrap_or_default();
    if resource == "product" && sku.is_empty() {
        return vec![];
    }
    let title = o["name"].as_str().unwrap_or_default();
    fields
        .iter()
        .filter_map(|(f, html)| {
            let v = attr(o, f)?;
            if v.trim().is_empty() {
                return None;
            }
            Some(SourceItem {
                external_id: format!("{resource}:{id}"),
                field: f.to_string(),
                key: super::key("magento", resource, &id.to_string(), f),
                text: v.to_string(),
                format: if *html { Format::Html } else { Format::Plain },
                context: format!("Magento {resource} \u{ab}{title}\u{bb}, field {f}"),
                updated_at: None,
                meta: json!({"type": resource, "id": id, "sku": sku}),
            })
        })
        .collect()
}

fn resources(
    creds: &Creds,
) -> Vec<&'static (&'static str, &'static str, &'static [(&'static str, bool)])> {
    let only: Option<Vec<String>> = creds
        .setting("resources")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        });
    RESOURCES
        .iter()
        .filter(|r| only.as_ref().is_none_or(|o| o.iter().any(|x| x == r.0)))
        .collect()
}

#[async_trait]
impl Connector for Magento {
    fn kind(&self) -> &'static str {
        "magento"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let stores = api.stores().await?;
        let src = source_store(creds, &stores)?.to_string();
        // Authenticated catalog read (store config may be readable more widely).
        api.call(
            reqwest::Method::GET,
            Some(&src),
            "/products",
            &[("searchCriteria[pageSize]".into(), "1".into())],
            None,
        )
        .await?;
        let locale = stores
            .iter()
            .find(|(c, _)| *c == src)
            .map(|(_, l)| l.clone())
            .filter(|l| !l.is_empty());
        let mut notes = vec![format!(
            "Store views: {}",
            stores
                .iter()
                .map(|(c, l)| format!("{c} ({l})"))
                .collect::<Vec<_>>()
                .join(", ")
        )];
        if api.oauth.is_none() {
            notes.push("Bearer access token: Commerce 2.4.4+ accepts it only with 'Allow OAuth Access Tokens to be used as standalone Bearer tokens'. Add the consumer key/secret and token secret to sign requests (OAuth 1.0a) instead.".into());
        }
        Ok(AccountInfo {
            name: format!("Magento ({})", api.base.trim_start_matches("https://")),
            source_locale: locale.map(|l| l.replace('_', "-")),
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let mut out: Vec<String> = Api::new(creds)
            .await?
            .stores()
            .await?
            .into_iter()
            .map(|(_, l)| l)
            .filter(|l| !l.is_empty())
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Cursor = `{resource index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let stores = api.stores().await?;
        let src = source_store(creds, &stores)?;
        let res = resources(creds);
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u32>()?),
            None => (0, 1),
        };
        let Some((resource, path, fields)) = res.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let mut q: Vec<(String, String)> = vec![
            ("searchCriteria[pageSize]".into(), PAGE.to_string()),
            ("searchCriteria[currentPage]".into(), page.to_string()),
            (
                "searchCriteria[sortOrders][0][field]".into(),
                "entity_id".into(),
            ),
            (
                "searchCriteria[sortOrders][0][direction]".into(),
                "ASC".into(),
            ),
        ];
        if let Some(since) = creds.since {
            let f = "searchCriteria[filter_groups][0][filters][0]";
            q.push((format!("{f}[field]"), "updated_at".into()));
            q.push((
                format!("{f}[value]"),
                since.format("%Y-%m-%d %H:%M:%S").to_string(),
            ));
            q.push((format!("{f}[condition_type]"), "gteq".into()));
        }
        let v = api
            .call(reqwest::Method::GET, Some(src), path, &q, None)
            .await?;
        let items = v["items"]
            .as_array()
            .into_iter()
            .flatten()
            // Root catalog (level 0) and the store root categories (level 1) are not shown to shoppers.
            .filter(|o| *resource != "category" || o["level"].as_i64().unwrap_or(2) >= 2)
            .flat_map(|o| items_of(resource, fields, o))
            .collect();
        let total = v["total_count"].as_u64().unwrap_or(0);
        // Magento returns the last page again past the end: stop on total_count.
        let next = if u64::from(page) * u64::from(PAGE) < total {
            Some(format!("{i}|{}", page + 1))
        } else if i + 1 < res.len() {
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
        let api = Api::new(creds).await?;
        let stores = api.stores().await?;
        let src = source_store(creds, &stores)?;
        let targets = targets(&stores, src, locale);
        anyhow::ensure!(
            !targets.is_empty(),
            "no store view for {locale} (store views: {}); add one or set settings.locale_map",
            stores
                .iter()
                .map(|(c, l)| format!("{c} {l}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let meta = &its[0].meta;
            let mut obj = serde_json::Map::new();
            let mut custom = vec![];
            for it in &its {
                if it.field == "name" {
                    obj.insert("name".into(), Value::String(it.value.clone()));
                } else {
                    custom.push(json!({"attribute_code": it.field, "value": it.value}));
                }
            }
            if !custom.is_empty() {
                obj.insert("custom_attributes".into(), Value::Array(custom));
            }
            let (path, body) = match (
                meta["type"].as_str(),
                meta["id"].as_i64(),
                meta["sku"].as_str(),
            ) {
                (Some("product"), _, Some(sku)) if !sku.is_empty() => {
                    obj.insert("sku".into(), json!(sku));
                    (format!("/products/{}", enc(sku)), json!({"product": obj}))
                }
                (Some("category"), Some(id), _) => {
                    obj.insert("id".into(), json!(id));
                    (format!("/categories/{id}"), json!({"category": obj}))
                }
                _ => {
                    report
                        .failed
                        .push((ext.to_string(), "missing metadata, sync again".into()));
                    continue;
                }
            };
            let mut err = None;
            for store in &targets {
                if let Err(e) = api
                    .call(reqwest::Method::PUT, Some(store), &path, &[], Some(&body))
                    .await
                {
                    err = Some(format!("{store}: {e:#}"));
                }
            }
            match err {
                None => report.ok += its.len(),
                Some(e) => report.failed.extend(
                    its.iter()
                        .map(|it| (format!("{}/{}", it.external_id, it.field), e.clone())),
                ),
            }
        }
        Ok(report)
    }

    /// No native webhooks: an extension or cron calls the URL with its token, a shared-secret header, or
    /// `X-Glot-Signature: sha256=<hex HMAC of the body>`.
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
            || cms::header(headers, "x-glot-signature")
                .is_some_and(|sig| super::hmac_ok(secret, body, sig))
    }

    fn push_batch(&self) -> usize {
        // Each product/category is one PUT per store view: small batches keep progress saved often.
        20
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{StatusCode, Uri},
        routing::{get, put},
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn oauth_base_string() {
        let q = vec![("searchCriteria[pageSize]".to_string(), "1".to_string())];
        let b = base_string("get", "https://m.test/rest/V1/products", &q);
        assert_eq!(
            b,
            "GET&https%3A%2F%2Fm.test%2Frest%2FV1%2Fproducts&searchCriteria%255BpageSize%255D%3D1"
        );
        assert_eq!(enc("MS 01/a"), "MS%2001%2Fa");
    }

    #[test]
    fn store_targets() {
        let s: Vec<(String, String)> = [
            ("default", "en_US"),
            ("fr", "fr_FR"),
            ("ca_fr", "fr_CA"),
            ("de", "de_DE"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(targets(&s, "default", "fr"), ["fr", "ca_fr"]);
        assert_eq!(targets(&s, "default", "fr-CA"), ["ca_fr"]);
        assert_eq!(targets(&s, "default", "de_DE"), ["de"]);
        assert!(targets(&s, "default", "en").is_empty(), "never the source");
    }

    type Log = Arc<Mutex<Vec<(String, Value)>>>;

    /// Checks an OAuth 1.0a HMAC-SHA256 header the way Commerce does (base string built independently).
    fn oauth_ok(h: &HeaderMap, method: &str, url: &str, raw_query: &str) -> bool {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let Some(a) = h.get("authorization").and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let Some(a) = a.strip_prefix("OAuth ") else {
            return false;
        };
        let dec = |s: &str| {
            let b = s.as_bytes();
            let mut out = vec![];
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%' {
                    out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                    i += 3;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).unwrap()
        };
        let mut params: HashMap<String, String> = a
            .split(", ")
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_string(), dec(v.trim_matches('"'))))
            .collect();
        let sig = params.remove("oauth_signature").unwrap_or_default();
        if params.get("oauth_signature_method").map(String::as_str) != Some("HMAC-SHA256")
            || params.get("oauth_consumer_key").map(String::as_str) != Some("ck")
            || params.get("oauth_token").map(String::as_str) != Some("tok")
        {
            return false;
        }
        let mut pairs: Vec<String> = params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .chain(raw_query.split('&').filter(|s| !s.is_empty()).map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                (dec(k), dec(v))
            }))
            .map(|(k, v)| format!("{}={}", enc(&k), enc(&v)))
            .collect();
        pairs.sort();
        let base = format!("{method}&{}&{}", enc(url), enc(&pairs.join("&")));
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"cs&ts").unwrap();
        mac.update(base.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()) == sig
    }

    #[derive(Clone)]
    struct Mock {
        log: Log,
        base: Arc<Mutex<String>>,
    }

    fn auth(m: &Mock, h: &HeaderMap, method: &str, uri: &Uri) -> Result<(), StatusCode> {
        let url = format!("{}{}", m.base.lock().unwrap(), uri.path());
        if oauth_ok(h, method, &url, uri.query().unwrap_or_default()) {
            Ok(())
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }

    fn q(uri: &Uri) -> HashMap<String, String> {
        url_q(uri.query().unwrap_or_default())
    }
    fn url_q(s: &str) -> HashMap<String, String> {
        reqwest::Url::parse(&format!("http://x/?{s}"))
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    async fn mock() -> (String, Mock) {
        let m = Mock {
            log: Default::default(),
            base: Default::default(),
        };
        let app = Router::new()
            .route(
                "/rest/V1/store/storeConfigs",
                get(|State(m): State<Mock>, h: HeaderMap, uri: Uri| async move {
                    auth(&m, &h, "GET", &uri)?;
                    Ok::<_, StatusCode>(Json(json!([
                        {"id": 1, "code": "default", "website_id": 1, "locale": "en_US", "base_currency_code": "USD", "timezone": "UTC"},
                        {"id": 2, "code": "french", "website_id": 1, "locale": "fr_FR", "base_currency_code": "USD", "timezone": "UTC"},
                        {"id": 3, "code": "german", "website_id": 1, "locale": "de_DE", "base_currency_code": "USD", "timezone": "UTC"}
                    ])))
                }),
            )
            .route(
                "/rest/{store}/V1/products",
                get(|State(m): State<Mock>, h: HeaderMap, uri: Uri, Path(store): Path<String>| async move {
                    auth(&m, &h, "GET", &uri)?;
                    assert_eq!(store, "default", "source values read in the source store view");
                    let q = q(&uri);
                    let page = q.get("searchCriteria[currentPage]").map(String::as_str).unwrap_or("1");
                    let items = match page {
                        "1" => json!([{"id": 1, "sku": "MS 01", "name": "Shirt", "type_id": "simple", "updated_at": "2026-01-01 00:00:00",
                            "custom_attributes": [
                                {"attribute_code": "description", "value": "<p>Soft <strong>cotton</strong> shirt.</p>"},
                                {"attribute_code": "url_key", "value": "shirt"},
                                {"attribute_code": "meta_title", "value": ""}]}]),
                        "2" => json!([{"id": 2, "sku": "HAT", "name": "Hat", "custom_attributes": []}]),
                        _ => json!([{"id": 2, "sku": "HAT", "name": "Hat", "custom_attributes": []}]),
                    };
                    Ok::<_, StatusCode>(Json(json!({"items": items, "search_criteria": {}, "total_count": PAGE + 1})))
                }),
            )
            .route(
                "/rest/{store}/V1/categories/list",
                get(|State(m): State<Mock>, h: HeaderMap, uri: Uri| async move {
                    auth(&m, &h, "GET", &uri)?;
                    Ok::<_, StatusCode>(Json(json!({"items": [
                        {"id": 2, "parent_id": 1, "name": "Default Category", "level": 1, "custom_attributes": []},
                        {"id": 5, "parent_id": 2, "name": "Shirts", "level": 2, "custom_attributes": [
                            {"attribute_code": "description", "value": "<p>All <a href=\"/sale\">shirts</a></p>"}]}
                    ], "total_count": 2})))
                }),
            )
            .route(
                "/rest/{store}/V1/products/{sku}",
                put(|State(m): State<Mock>, h: HeaderMap, uri: Uri, Json(b): Json<Value>| async move {
                    auth(&m, &h, "PUT", &uri)?;
                    m.log.lock().unwrap().push((format!("PUT {}", uri.path()), b));
                    Ok::<_, StatusCode>(Json(json!({"id": 1})))
                }),
            )
            .route(
                "/rest/{store}/V1/categories/{id}",
                put(|State(m): State<Mock>, h: HeaderMap, uri: Uri, Json(b): Json<Value>| async move {
                    auth(&m, &h, "PUT", &uri)?;
                    m.log.lock().unwrap().push((format!("PUT {}", uri.path()), b));
                    Ok::<_, StatusCode>(Json(json!({"id": 5})))
                }),
            )
            .with_state(m.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        *m.base.lock().unwrap() = base.clone();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (base, m)
    }

    #[tokio::test]
    async fn pull_and_push_against_mock() {
        unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
        let (base, m) = mock().await;
        let creds = Creds {
            secrets: json!({"url": format!("{base}/"), "token": "tok", "token_secret": "ts", "consumer_key": "ck", "consumer_secret": "cs"}),
            ..Default::default()
        };
        let info = Magento.test(&creds).await.unwrap();
        assert_eq!(info.source_locale.as_deref(), Some("en-US"));
        assert_eq!(
            Magento.locales(&creds).await.unwrap(),
            ["de_DE", "en_US", "fr_FR"]
        );
        let bad = Creds {
            secrets: json!({"url": base, "token": "tok"}),
            ..Default::default()
        };
        assert!(
            Magento.test(&bad).await.is_err(),
            "unsigned bearer refused by the mock"
        );

        let mut all = vec![];
        let mut cursor = None;
        loop {
            let p = Magento.pull(&creds, cursor).await.unwrap();
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
                "magento:product:1:name",
                "magento:product:1:description",
                "magento:product:2:name",
                "magento:category:5:name",
                "magento:category:5:description"
            ]
        );
        assert_eq!(all[1].format, Format::Html);
        assert_eq!(all[0].meta["sku"], "MS 01");

        let push: Vec<PushItem> = all
            .iter()
            .filter(|i| i.external_id != "product:2")
            .map(|i| PushItem {
                external_id: i.external_id.clone(),
                field: i.field.clone(),
                value: match i.field.as_str() {
                    "name" if i.external_id == "product:1" => "Chemise".into(),
                    "name" => "Chemises".into(),
                    _ => i.text.replace("Soft", "Douce").replace("All", "Toutes"),
                },
                format: i.format.clone(),
                meta: i.meta.clone(),
            })
            .collect();
        let r = Magento.push(&creds, "fr", &push).await.unwrap();
        assert_eq!(r.ok, 4);
        assert!(r.failed.is_empty());
        let log = m.log.lock().unwrap().clone();
        assert_eq!(log.len(), 2);
        let (path, b) = &log[1];
        assert_eq!(path, "PUT /rest/french/V1/products/MS%2001");
        assert_eq!(
            b,
            &json!({"product": {"sku": "MS 01", "name": "Chemise", "custom_attributes": [
                {"attribute_code": "description", "value": "<p>Douce <strong>cotton</strong> shirt.</p>"}]}})
        );
        assert_eq!(log[0].0, "PUT /rest/french/V1/categories/5");
        assert_eq!(log[0].1["category"]["id"], 5);
        assert_eq!(
            log[0].1["category"]["custom_attributes"][0]["value"],
            "<p>Toutes <a href=\"/sale\">shirts</a></p>"
        );
        assert!(
            Magento.push(&creds, "ja", &push).await.is_err(),
            "no store view"
        );
    }
}
