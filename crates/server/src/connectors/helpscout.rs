//! Help Scout Docs (Docs API v1). Docs has no locales: Help Scout's documented way to publish several
//! languages is one Docs site per language (https://docs.helpscout.com/article/302-translations). glot
//! reads the source site's collections and writes each language into the collection mapped to it, the
//! translated article keeping the source's slug (slugs are unique per site, so it is the link).
//!
//! Credentials: `api_key` (Your Profile → Authentication → API Keys; HTTP Basic `{key}:X`), optional
//! `api_url` (tests). Settings: `collections` (glot locale → target collection id, required to push),
//! `source_site` (site id; default: every collection not listed as a target), `publish` (create
//! translations published; default `notpublished`).
//!
//! - Pull: `GET /v1/collections?page=N`, `GET /v1/collections/{id}/articles?page=N&pageSize=100`
//!   (`{articles: {page, pages, items}}`), then `GET /v1/articles/{id}`: `name` (plain), `text` (HTML).
//! - Push: `PUT /v1/articles/{id}` on the article with the same slug in the target collection, else
//!   `POST /v1/articles` (`collectionId`, `name`, `text`, `slug`, `status`).
//! - Webhooks: Help Scout has none for Docs: use **Sync now**, or call the card URL (token included).
//! - Rate limit: 2,000 requests per 10 minutes for one site (more with more sites); glot waits on `429`.
//!
//! Docs: https://developer.helpscout.com/docs-api/, https://developer.helpscout.com/docs-api/articles/list/,
//! https://developer.helpscout.com/docs-api/articles/create/, https://developer.helpscout.com/docs-api/articles/update/
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

use super::cms::{self, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct HelpScout;

struct Api {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let base = cms::param(c, "api_url")
            .unwrap_or("https://docsapi.helpscout.net")
            .trim_end_matches('/')
            .to_string();
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD
                .encode(format!("{}:X", required(c, "api_key")?))
        );
        Ok(Api {
            http: super::client_for(&base).await?,
            base,
            auth,
        })
    }
    async fn call(
        &self,
        m: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = format!("{}/v1{path}", self.base);
        let res = send(|| {
            let r = self
                .http
                .request(m.clone(), &url)
                .header("authorization", &self.auth);
            match body {
                Some(b) => r.json(b),
                None => r,
            }
        })
        .await?;
        Ok(res.json().await.unwrap_or(Value::Null))
    }
    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }
    /// Every item of a paginated list (`{name: {page, pages, items}}`).
    async fn all(&self, path: &str, name: &str) -> anyhow::Result<Vec<Value>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut out = vec![];
        let mut page = 1;
        loop {
            let r = self.get(&format!("{path}{sep}page={page}")).await?;
            out.extend(r[name]["items"].as_array().cloned().unwrap_or_default());
            if page >= r[name]["pages"].as_u64().unwrap_or(1) {
                return Ok(out);
            }
            page += 1;
        }
    }
}

fn targets(c: &Creds) -> BTreeMap<String, String> {
    cms::pairs(c, "collections")
}

/// Source collection ids, in a stable order.
async fn sources(api: &Api, c: &Creds) -> anyhow::Result<Vec<String>> {
    let t: Vec<String> = targets(c).into_values().collect();
    let site = cms::param(c, "source_site");
    let mut out: Vec<String> = api
        .all("/collections", "collections")
        .await?
        .iter()
        .filter(|col| site.is_none_or(|s| cms::id_str(&col["siteId"]).as_deref() == Some(s)))
        .filter_map(|col| cms::id_str(&col["id"]))
        .filter(|id| !t.contains(id))
        .collect();
    out.sort();
    Ok(out)
}

fn id_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

#[async_trait]
impl Connector for HelpScout {
    fn kind(&self) -> &'static str {
        "helpscout"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let sites = api.all("/sites", "sites").await?;
        let src = sources(&api, creds).await?;
        let t = targets(creds);
        let mut notes = vec![format!("{} source collection(s)", src.len())];
        notes.push(if t.is_empty() {
            "Docs has no languages: create one Docs site per language and map each glot locale to a collection of it in settings.collections.".into()
        } else {
            format!(
                "Target collections: {}",
                t.iter().map(|(l, c)| format!("{l} → {c}")).collect::<Vec<_>>().join(", ")
            )
        });
        Ok(AccountInfo {
            name: sites
                .first()
                .and_then(|s| s["title"].as_str().or(s["subDomain"].as_str()))
                .unwrap_or("Help Scout Docs")
                .to_string(),
            source_locale: cms::param(creds, "source_lang").map(String::from),
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(targets(creds).into_keys().collect())
    }

    /// Cursor = `{collection index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let cols = sources(&api, creds).await?;
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u64>()?),
            None => (0, 1),
        };
        let Some(col) = cols.get(i) else {
            return Ok(PullPage::default());
        };
        anyhow::ensure!(id_ok(col), "invalid collection id");
        let list = api
            .get(&format!(
                "/collections/{col}/articles?page={page}&pageSize=100"
            ))
            .await?;
        let mut items = vec![];
        for r in list["articles"]["items"].as_array().into_iter().flatten() {
            let Some(id) = cms::id_str(&r["id"]).filter(|i| id_ok(i)) else {
                continue;
            };
            if let Some(since) = creds.since {
                let upd = r["updatedAt"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
                if upd.is_some_and(|u| u < since) {
                    continue;
                }
            }
            let a = api.get(&format!("/articles/{id}")).await?;
            let a = &a["article"];
            let name = a["name"].as_str().unwrap_or("");
            let meta = json!({"id": id, "slug": a["slug"], "collection": col});
            for (f, html) in [("name", false), ("text", true)] {
                let Some(v) = a[f].as_str().filter(|v| cms::has_letter(v)) else {
                    continue;
                };
                items.push(SourceItem {
                    external_id: format!("article:{id}"),
                    field: f.to_string(),
                    key: key("helpscout", "article", &id, f),
                    text: v.to_string(),
                    format: if html { Format::Html } else { Format::Plain },
                    context: format!("Help Scout Docs article \u{ab}{name}\u{bb}, field {f}"),
                    updated_at: None,
                    meta: meta.clone(),
                });
            }
        }
        let next = if page < list["articles"]["pages"].as_u64().unwrap_or(1) {
            Some(format!("{i}|{}", page + 1))
        } else if i + 1 < cols.len() {
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
        let t = targets(creds);
        let col = t
            .get(locale)
            .or_else(|| {
                let keys: Vec<String> = t.keys().cloned().collect();
                cms::match_locale(&keys, locale).and_then(|k| t.get(k))
            })
            .filter(|c| id_ok(c))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no Docs collection for {locale}: set settings.collections.{locale}"
                )
            })?
            .clone();
        let existing: HashMap<String, String> = api
            .all(
                &format!("/collections/{col}/articles?pageSize=100"),
                "articles",
            )
            .await?
            .iter()
            .filter_map(|a| Some((a["slug"].as_str()?.to_string(), cms::id_str(&a["id"])?)))
            .collect();
        let status = if creds.setting("publish") == Some(&Value::Bool(true)) {
            "published"
        } else {
            "notpublished"
        };
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let slug = its[0].meta["slug"].as_str().unwrap_or("").to_string();
            let mut body = Map::new();
            for it in &its {
                body.insert(it.field.clone(), Value::String(it.value.clone()));
            }
            let r: anyhow::Result<()> = async {
                anyhow::ensure!(!slug.is_empty(), "missing metadata, sync again");
                match existing.get(&slug).filter(|i| id_ok(i)) {
                    Some(id) => {
                        api.call(
                            reqwest::Method::PUT,
                            &format!("/articles/{id}"),
                            Some(&Value::Object(body)),
                        )
                        .await?;
                    }
                    None => {
                        anyhow::ensure!(
                            body.contains_key("name") && body.contains_key("text"),
                            "no {locale} article yet and its name or text is not translated yet"
                        );
                        body.insert("collectionId".into(), Value::String(col.clone()));
                        body.insert("slug".into(), Value::String(slug.clone()));
                        body.insert("status".into(), Value::String(status.into()));
                        api.call(
                            reqwest::Method::POST,
                            "/articles",
                            Some(&Value::Object(body)),
                        )
                        .await?;
                    }
                }
                Ok(())
            }
            .await;
            match r {
                Ok(()) => report.ok += its.len(),
                Err(e) => report.failed.push((ext.to_string(), format!("{e:#}"))),
            }
        }
        Ok(report)
    }

    fn verify_webhook(&self, headers: &HeaderMap, _body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
    }

    fn push_batch(&self) -> usize {
        50
    }
}
