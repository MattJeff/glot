//! Shopware 6: Admin API with an integration (Settings → System → Integrations: access key ID + secret).
//!
//! - Auth: `POST /api/oauth/token` (`grant_type=client_credentials`, `client_id` = access key ID,
//!   `client_secret` = secret access key) → short-lived Bearer token (`expires_in` 600 s), fetched once per
//!   connector call (a pull page or a push batch).
//! - Pull: `POST /api/search/{entity}` (`page`, `limit`, `total-count-mode: 1`, `includes`), read in the system
//!   language (no `sw-language-id` header = the shop's default language, the source). Products (`name`,
//!   `description` HTML, `metaTitle`, `metaDescription`, `keywords`; main products only, `parentId = null`:
//!   variants inherit), categories (`name`, `description` HTML, meta fields), property groups (`name`,
//!   `description` HTML) and their options (`name`). Incremental: `updatedAt >= since` or `createdAt >= since`.
//! - Languages: `POST /api/search/language` with the `locale` association (`locale.code`, `de-DE`).
//! - Push: `PATCH /api/{entity}/{id}` with `{"translations": {"<languageId>": {field: value}}}`: the DAL's
//!   translations association, indexed by language id; only the given language's translation row is written.
//! - Webhooks: Shopware sends them from apps (`shopware-shop-signature`, hex HMAC-SHA256 of the body with the
//!   app secret) or from Flow Builder's *Call webhook* action (Evolve / Beyond plans): no API to register one,
//!   the user pastes the card's URL. Verified by that signature, `X-Glot-Webhook-Secret` / Bearer, or
//!   `X-Glot-Signature`.
//! - Rate limits: the Admin API has no documented quota; `429` / `503` are retried by [`super::send`].
//!
//! Docs: https://developer.shopware.com/docs/guides/development/integrations-api/auth-api-requests.html,
//! https://developer.shopware.com/docs/guides/integrations-api/general-concepts/search-criteria.html,
//! https://developer.shopware.com/docs/guides/integrations-api/general-concepts/request-headers.html,
//! https://developer.shopware.com/docs/resources/references/adr/2022-03-29-specify-priority-of-translations-in-dal-write-payloads.html,
//! https://developer.shopware.com/docs/resources/references/core-reference/dal-reference/filters-reference.html,
//! https://developer.shopware.com/docs/guides/plugins/apps/webhook.html,
//! https://docs.shopware.com/en/shopware-6-en/features/webhook-actions-in-flow-builder
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, cms,
};

pub struct Shopware;

/// `Defaults::LANGUAGE_SYSTEM`: the shop's default language, the source.
pub const SYSTEM_LANGUAGE: &str = "2fbb5fe2e29a4d70aa5854ce7ce3e20b";

const LIMIT: u32 = 100;

/// (entity, translatable fields with `true` = HTML).
const ENTITIES: &[(&str, &[(&str, bool)])] = &[
    (
        "product",
        &[
            ("name", false),
            ("description", true),
            ("metaTitle", false),
            ("metaDescription", false),
            ("keywords", false),
        ],
    ),
    (
        "category",
        &[
            ("name", false),
            ("description", true),
            ("metaTitle", false),
            ("metaDescription", false),
            ("keywords", false),
        ],
    ),
    ("property_group", &[("name", false), ("description", true)]),
    ("property_group_option", &[("name", false)]),
];

struct Api {
    base: String,
    token: String,
    client: reqwest::Client,
}

impl Api {
    async fn new(creds: &Creds) -> anyhow::Result<Self> {
        let base = creds.get("url")?.trim_end_matches('/');
        let base = base.strip_suffix("/api").unwrap_or(base).to_string();
        let client = super::client_for(&base).await?;
        let body = json!({
            "grant_type": "client_credentials",
            "client_id": creds.get("client_id")?,
            "client_secret": creds.get("client_secret")?,
        });
        let url = format!("{base}/api/oauth/token");
        let res = super::send(|| client.post(&url).json(&body)).await?;
        let v: Value = res.json().await?;
        let token = v["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Shopware: no access_token in the token response"))?
            .to_string();
        Ok(Self {
            base,
            token,
            client,
        })
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = format!("{}/api{path}", self.base);
        let res = super::send(|| {
            let r = self
                .client
                .request(method.clone(), &url)
                .bearer_auth(&self.token)
                // Plain JSON (`{"total", "data"}`), not JSON:API.
                .header("Accept", "application/json");
            match body {
                Some(b) => r.json(b),
                None => r,
            }
        })
        .await?;
        let text = res.text().await?;
        Ok(if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)?
        })
    }

    async fn search(&self, entity: &str, criteria: &Value) -> anyhow::Result<Value> {
        self.call(
            reqwest::Method::POST,
            &format!("/search/{}", entity.replace('_', "-")),
            Some(criteria),
        )
        .await
    }

    /// (language id, locale code) of every language.
    async fn languages(&self) -> anyhow::Result<Vec<(String, String)>> {
        let v = self
            .search(
                "language",
                &json!({"limit": 500, "associations": {"locale": {}},
                    "includes": {"language": ["id", "name", "locale"], "locale": ["code"]}}),
            )
            .await?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                Some((
                    l["id"].as_str()?.to_string(),
                    l["locale"]["code"].as_str()?.to_string(),
                ))
            })
            .collect())
    }
}

/// The language id for a glot / mapped locale: exact code first (`fr-FR`), else same language (`fr` ↔ `fr-FR`).
fn language_for<'a>(langs: &'a [(String, String)], locale: &str) -> Option<&'a str> {
    let base = |s: &str| {
        s.split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let norm = |s: &str| s.replace('_', "-").to_ascii_lowercase();
    langs
        .iter()
        .find(|(_, c)| norm(c) == norm(locale))
        .or_else(|| {
            langs.iter().find(|(id, c)| {
                id != SYSTEM_LANGUAGE
                    && base(c) == base(locale)
                    && (!c.contains('-') || !locale.contains(['-', '_']))
            })
        })
        .map(|(id, _)| id.as_str())
}

fn criteria(entity: &str, fields: &[(&str, bool)], page: u32, creds: &Creds) -> Value {
    let mut include: Vec<&str> = vec!["id"];
    include.extend(fields.iter().map(|f| f.0));
    let mut filter = vec![];
    if entity == "product" {
        filter.push(json!({"type": "equals", "field": "parentId", "value": null}));
    }
    if let Some(since) = creds.since {
        let t = since.format("%Y-%m-%dT%H:%M:%S%.3f+00:00").to_string();
        filter.push(json!({"type": "multi", "operator": "or", "queries": [
            {"type": "range", "field": "updatedAt", "parameters": {"gte": t}},
            {"type": "range", "field": "createdAt", "parameters": {"gte": t}}]}));
    }
    json!({
        "page": page,
        "limit": LIMIT,
        "total-count-mode": 1,
        "sort": [{"field": "id", "order": "ASC"}],
        "filter": filter,
        "includes": {entity: include},
    })
}

fn items_of(entity: &str, fields: &[(&str, bool)], o: &Value) -> Vec<SourceItem> {
    let Some(id) = o["id"].as_str() else {
        return vec![];
    };
    let title = o["name"].as_str().unwrap_or_default();
    let what = entity.replace('_', " ");
    fields
        .iter()
        .filter_map(|(f, html)| {
            let v = o[*f].as_str()?;
            if v.trim().is_empty() {
                return None;
            }
            Some(SourceItem {
                external_id: format!("{entity}:{id}"),
                field: f.to_string(),
                key: super::key("shopware", entity, id, f),
                text: v.to_string(),
                format: if *html { Format::Html } else { Format::Plain },
                context: if title.is_empty() {
                    format!("Shopware {what}, field {f}")
                } else {
                    format!("Shopware {what} \u{ab}{title}\u{bb}, field {f}")
                },
                updated_at: None,
                meta: json!({"entity": entity, "id": id}),
            })
        })
        .collect()
}

#[async_trait]
impl Connector for Shopware {
    fn kind(&self) -> &'static str {
        "shopware"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let version = api
            .call(reqwest::Method::GET, "/_info/version", None)
            .await
            .ok()
            .and_then(|v| v["version"].as_str().map(str::to_string));
        let langs = api.languages().await?;
        let source = langs
            .iter()
            .find(|(id, _)| id == SYSTEM_LANGUAGE)
            .map(|l| l.1.clone());
        let host = reqwest::Url::parse(&api.base)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "Shopware".into());
        let codes: Vec<&str> = langs.iter().map(|l| l.1.as_str()).collect();
        Ok(AccountInfo {
            name: match version {
                Some(v) => format!("{host} (Shopware {v})"),
                None => host,
            },
            source_locale: source,
            notes: vec![format!("Languages: {}", codes.join(", "))],
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let api = Api::new(creds).await?;
        Ok(api.languages().await?.into_iter().map(|l| l.1).collect())
    }

    /// Cursor = `{entity index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u32>()?),
            None => (0, 1),
        };
        let Some((entity, fields)) = ENTITIES.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let api = Api::new(creds).await?;
        let v = api
            .search(entity, &criteria(entity, fields, page, creds))
            .await?;
        let rows = v["data"].as_array().cloned().unwrap_or_default();
        let total = v["total"].as_u64().unwrap_or(0);
        let items = rows
            .iter()
            .flat_map(|o| items_of(entity, fields, o))
            .collect();
        let next = if (page as u64) * (LIMIT as u64) < total && !rows.is_empty() {
            Some(format!("{i}|{}", page + 1))
        } else if i + 1 < ENTITIES.len() {
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
        let langs = api.languages().await?;
        let lang = language_for(&langs, locale)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no Shopware language for '{locale}': add it in Settings → Languages (or map it with settings.locale_map)"
                )
            })?
            .to_string();
        anyhow::ensure!(
            lang != SYSTEM_LANGUAGE,
            "'{locale}' is the shop's default language: glot never overwrites the source"
        );
        let mut by_entity: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_entity.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by_entity {
            let meta = &its[0].meta;
            let (Some(entity), Some(id)) = (meta["entity"].as_str(), meta["id"].as_str()) else {
                report
                    .failed
                    .push((ext.to_string(), "missing metadata, sync again".into()));
                continue;
            };
            let fields: serde_json::Map<String, Value> = its
                .iter()
                .map(|it| (it.field.clone(), Value::String(it.value.clone())))
                .collect();
            let body = json!({"translations": {lang.as_str(): fields}});
            let path = format!("/{}/{id}", entity.replace('_', "-"));
            match api.call(reqwest::Method::PATCH, &path, Some(&body)).await {
                Ok(_) => report.ok += its.len(),
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

    /// App webhooks (`shopware-shop-signature`, hex HMAC-SHA256 of the body, key = app secret given as
    /// `webhook_secret`), or Flow Builder *Call webhook* with the card's token (URL, header or signature).
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        if secret.is_empty() {
            return false;
        }
        cms::shared_secret_ok(headers, secret)
            || ["shopware-shop-signature", "x-glot-signature"]
                .iter()
                .filter_map(|h| cms::header(headers, h))
                .any(|sig| super::hmac_ok(secret, body, sig))
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
        extract::{Path, State},
        http::StatusCode,
        routing::{get, patch, post},
    };
    use std::sync::{Arc, Mutex};

    const DE: &str = "1f0c9a4e8f6a4d2b9c3e5a7b1d2e3f40";

    #[derive(Clone, Default)]
    struct Mock {
        log: Arc<Mutex<Vec<(String, Value)>>>,
    }

    fn bearer(h: &HeaderMap) -> Result<(), StatusCode> {
        (h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer tok-1")
            && h.get("accept").and_then(|v| v.to_str().ok()) == Some("application/json"))
        .then_some(())
        .ok_or(StatusCode::UNAUTHORIZED)
    }

    async fn token(Json(b): Json<Value>) -> (StatusCode, Json<Value>) {
        if b["grant_type"] == "client_credentials"
            && b["client_id"] == "SWIAKEY"
            && b["client_secret"] == "secret"
        {
            (
                StatusCode::OK,
                Json(json!({"token_type": "Bearer", "expires_in": 600, "access_token": "tok-1"})),
            )
        } else {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"errors": [{"code": "10", "title": "Unauthorized"}]})),
            )
        }
    }

    async fn search(
        State(m): State<Mock>,
        h: HeaderMap,
        Path(entity): Path<String>,
        Json(c): Json<Value>,
    ) -> Result<Json<Value>, StatusCode> {
        bearer(&h)?;
        assert!(
            h.get("sw-language-id").is_none(),
            "source read in the system language"
        );
        m.log
            .lock()
            .unwrap()
            .push((format!("search {entity}"), c.clone()));
        let page = c["page"].as_u64().unwrap_or(1);
        Ok(Json(match entity.as_str() {
            "language" => {
                assert!(c["associations"]["locale"].is_object());
                json!({"total": 2, "data": [
                    {"id": SYSTEM_LANGUAGE, "name": "English", "locale": {"code": "en-GB"}},
                    {"id": DE, "name": "Deutsch", "locale": {"code": "de-DE"}}]})
            }
            "product" => {
                assert_eq!(
                    c["filter"][0],
                    json!({"type": "equals", "field": "parentId", "value": null})
                );
                assert_eq!(c["total-count-mode"], 1);
                // 2 products, limit 100 simulated as 1 per page through `total`.
                let id = if page == 1 { "a1" } else { "a2" };
                json!({"total": 101, "data": [{"id": id, "name": format!("Shirt {page}"),
                    "description": "<p>Soft <b>cotton</b>.</p>", "metaTitle": null, "keywords": ""}]})
            }
            "category" => {
                json!({"total": 1, "data": [{"id": "c1", "name": "Summer", "description": null}]})
            }
            _ => json!({"total": 0, "data": []}),
        }))
    }

    async fn write(
        State(m): State<Mock>,
        h: HeaderMap,
        Path((entity, id)): Path<(String, String)>,
        Json(b): Json<Value>,
    ) -> Result<StatusCode, StatusCode> {
        bearer(&h)?;
        m.log
            .lock()
            .unwrap()
            .push((format!("PATCH {entity}/{id}"), b));
        Ok(if id == "gone" {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::NO_CONTENT
        })
    }

    async fn server() -> (String, Mock) {
        unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
        let m = Mock::default();
        let app = Router::new()
            .route("/api/oauth/token", post(token))
            .route(
                "/api/_info/version",
                get(|| async { Json(json!({"version": "6.6.10.0"})) }),
            )
            .route("/api/search/{entity}", post(search))
            .route("/api/{entity}/{id}", patch(write))
            .with_state(m.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (format!("http://{addr}"), m)
    }

    fn creds(url: &str, secret: &str) -> Creds {
        Creds {
            secrets: json!({"url": format!("{url}/"), "client_id": "SWIAKEY", "client_secret": secret}),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn pull_and_push_against_admin_api() {
        let (url, m) = server().await;
        assert!(Shopware.test(&creds(&url, "wrong")).await.is_err());
        let c = creds(&url, "secret");
        let info = Shopware.test(&c).await.unwrap();
        assert_eq!(info.source_locale.as_deref(), Some("en-GB"));
        assert!(info.name.contains("Shopware 6.6.10.0"));
        assert_eq!(Shopware.locales(&c).await.unwrap(), ["en-GB", "de-DE"]);

        let mut items = vec![];
        let mut cursor = None;
        loop {
            let p = Shopware.pull(&c, cursor).await.unwrap();
            items.extend(p.items);
            cursor = p.next;
            if cursor.is_none() {
                break;
            }
        }
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "shopware:product:a1:name",
                "shopware:product:a1:description",
                "shopware:product:a2:name",
                "shopware:product:a2:description",
                "shopware:category:c1:name",
            ]
        );
        assert_eq!(items[1].format, Format::Html);
        let pages = m
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.0 == "search product")
            .count();
        assert_eq!(pages, 2);

        // HTML rebuilt with tags kept, written in the language's translation row only.
        let src = &items[1];
        let units = super::super::rich::units(&src.key, &src.format, &src.text, "en");
        let html = super::super::rich::rebuild(&src.key, &src.format, &src.text, "en", &|k| {
            (k == units[0].0).then(|| "Weiche <0>Baumwolle</0>.".to_string())
        })
        .unwrap();
        assert_eq!(html, "<p>Weiche <b>Baumwolle</b>.</p>");
        let push = |id: &str, field: &str, value: &str, entity: &str| PushItem {
            external_id: format!("{entity}:{id}"),
            field: field.into(),
            value: value.into(),
            format: Format::Plain,
            meta: json!({"entity": entity, "id": id}),
        };
        let r = Shopware
            .push(
                &c,
                "de",
                &[
                    push("a1", "name", "Hemd", "product"),
                    push("a1", "description", &html, "product"),
                    push("o1", "name", "Rot", "property_group_option"),
                    push("gone", "name", "x", "category"),
                ],
            )
            .await
            .unwrap();
        assert_eq!(r.ok, 3);
        assert_eq!(r.failed.len(), 1);
        assert!(r.failed[0].0.starts_with("category:gone"));
        let log = m.log.lock().unwrap().clone();
        let w = &log.iter().find(|l| l.0 == "PATCH product/a1").unwrap().1;
        assert_eq!(
            *w,
            json!({"translations": {DE: {"name": "Hemd", "description": "<p>Weiche <b>Baumwolle</b>.</p>"}}})
        );
        assert!(log.iter().any(|l| l.0 == "PATCH property-group-option/o1"));
        // Unknown language and the source language are refused.
        assert!(
            Shopware
                .push(&c, "ja", &[push("a1", "name", "x", "product")])
                .await
                .is_err()
        );
        assert!(
            Shopware
                .push(&c, "en", &[push("a1", "name", "x", "product")])
                .await
                .is_err()
        );
    }

    #[test]
    fn incremental_criteria_and_language_match() {
        let c = Creds {
            since: Some(chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
            ..Default::default()
        };
        let q = criteria("category", ENTITIES[1].1, 3, &c);
        assert_eq!(q["page"], 3);
        assert_eq!(
            q["filter"][0]["queries"][0]["parameters"]["gte"],
            "2023-11-14T22:13:20.000+00:00"
        );
        assert_eq!(q["includes"]["category"][0], "id");
        let langs = vec![
            (SYSTEM_LANGUAGE.to_string(), "en-GB".to_string()),
            ("x".to_string(), "fr-FR".to_string()),
            ("y".to_string(), "pt-BR".to_string()),
        ];
        assert_eq!(language_for(&langs, "fr"), Some("x"));
        assert_eq!(language_for(&langs, "pt_BR"), Some("y"));
        assert_eq!(language_for(&langs, "pt-PT"), None);
        assert_eq!(language_for(&langs, "en"), None);
    }

    #[test]
    fn webhook_signatures() {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"appsecret").unwrap();
        mac.update(b"{\"data\":{}}");
        let sig = hex::encode(mac.finalize().into_bytes());
        let mut h = HeaderMap::new();
        h.insert("shopware-shop-signature", sig.parse().unwrap());
        assert!(Shopware.verify_webhook(&h, b"{\"data\":{}}", "appsecret"));
        assert!(!Shopware.verify_webhook(&h, b"{\"data\":1}", "appsecret"));
        assert!(!Shopware.verify_webhook(&h, b"{\"data\":{}}", ""));
        let mut t = HeaderMap::new();
        t.insert("x-glot-webhook-secret", "tok".parse().unwrap());
        assert!(Shopware.verify_webhook(&t, b"", "tok"));
    }
}
