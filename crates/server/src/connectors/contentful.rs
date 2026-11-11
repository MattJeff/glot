//! Contentful connector (Content Management API).
//!
//! Credentials: `space_id`, `token` (personal access token or CMA token), optional `environment`
//! (default `master`) and `api_url` (default `https://api.contentful.com`, `https://api.eu.contentful.com`
//! for EU data residency). Settings: `publish` (publish entries that were published and up to date before the
//! update), `content_types` (only these content type ids).
//!
//! Pulled: the `localized` fields of type Symbol (plain), Text (Markdown, structure kept) and RichText (text
//! nodes, marks / hyperlinks / embedded entries kept), in the space's default locale. Pushed: the entry is read,
//! the locale's values set, and `PUT` with `X-Contentful-Version` (retried once on a version conflict).
//! Webhooks: the webhook URL shown in the dashboard (token included), or a secret header
//! `X-Glot-Webhook-Secret` set on the Contentful webhook.
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

use super::cms::{self, param, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct Contentful;

const PAGE: usize = 100;
const MIME: &str = "application/vnd.contentful.management.v1+json";

struct Api {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let url = param(c, "api_url")
            .unwrap_or("https://api.contentful.com")
            .trim_end_matches('/');
        let space = required(c, "space_id")?;
        anyhow::ensure!(
            space.chars().all(|c| c.is_ascii_alphanumeric()),
            "invalid space id"
        );
        let env = param(c, "environment").unwrap_or("master");
        anyhow::ensure!(
            env.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
            "invalid environment"
        );
        Ok(Api {
            http: super::client_for(url).await?,
            base: format!("{url}/spaces/{space}/environments/{env}"),
            token: required(c, "token")?.to_string(),
        })
    }
    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("content-type", MIME)
    }
    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        Ok(send(|| self.req(reqwest::Method::GET, path))
            .await?
            .json()
            .await?)
    }
    async fn default_locale(&self) -> anyhow::Result<(String, Vec<String>)> {
        let l = self.get("/locales?limit=1000").await?;
        let items = l["items"].as_array().cloned().unwrap_or_default();
        let codes = items
            .iter()
            .filter_map(|i| i["code"].as_str().map(String::from))
            .collect();
        let def = items
            .iter()
            .find(|i| i["default"] == true)
            .and_then(|i| i["code"].as_str())
            .ok_or_else(|| anyhow::anyhow!("no default locale in the environment"))?;
        Ok((def.to_string(), codes))
    }
}

/// (content type id → (name, display field, [(field id, type)])) of the localized translatable fields.
type Types = HashMap<String, (String, String, Vec<(String, String)>)>;

async fn content_types(api: &Api) -> anyhow::Result<Types> {
    let ct = api.get("/content_types?limit=1000").await?;
    Ok(ct["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let fields = t["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|f| f["localized"] == true && f["disabled"] != true && f["omitted"] != true)
                .filter_map(|f| {
                    let ty = f["type"].as_str()?;
                    matches!(ty, "Symbol" | "Text" | "RichText").then(|| {
                        (
                            f["id"].as_str().unwrap_or_default().to_string(),
                            ty.to_string(),
                        )
                    })
                })
                .collect();
            (
                t["sys"]["id"].as_str().unwrap_or_default().to_string(),
                (
                    t["name"].as_str().unwrap_or_default().to_string(),
                    t["displayField"].as_str().unwrap_or_default().to_string(),
                    fields,
                ),
            )
        })
        .collect())
}

#[async_trait]
impl Connector for Contentful {
    fn kind(&self) -> &'static str {
        "contentful"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let env = api.get("").await?;
        let (def, codes) = api.default_locale().await?;
        let types = content_types(&api).await?;
        let localized: usize = types.values().map(|t| t.2.len()).sum();
        let mut notes = vec![format!("{} locales: {}", codes.len(), codes.join(", "))];
        if localized == 0 {
            notes.push("no localized text field: enable \"Enable localization of this field\" on the fields to translate".into());
        }
        Ok(AccountInfo {
            name: env["name"]
                .as_str()
                .or(env["sys"]["id"].as_str())
                .unwrap_or("Contentful")
                .to_string(),
            source_locale: Some(def),
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(Api::new(creds).await?.default_locale().await?.1)
    }

    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let (def, _) = api.default_locale().await?;
        let types = content_types(&api).await?;
        let only: Option<Vec<String>> = creds
            .setting("content_types")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let skip: usize = cursor.as_deref().and_then(|c| c.parse().ok()).unwrap_or(0);
        let mut q = format!("/entries?skip={skip}&limit={PAGE}&order=sys.createdAt,sys.id");
        if let Some(s) = creds.since {
            q.push_str(&format!(
                "&sys.updatedAt%5Bgte%5D={}",
                s.format("%Y-%m-%dT%H:%M:%SZ")
            ));
        }
        if let Some(o) = &only {
            q.push_str(&format!("&sys.contentType.sys.id%5Bin%5D={}", o.join(",")));
        }
        let page = api.get(&q).await?;
        let entries = page["items"].as_array().cloned().unwrap_or_default();
        let total = page["total"].as_u64().unwrap_or(0) as usize;
        let mut items = vec![];
        for e in &entries {
            let id = e["sys"]["id"].as_str().unwrap_or_default();
            let ct = e["sys"]["contentType"]["sys"]["id"]
                .as_str()
                .unwrap_or_default();
            let Some((ct_name, display, fields)) = types.get(ct) else {
                continue;
            };
            let title = e["fields"][display.as_str()][&def]
                .as_str()
                .unwrap_or_default();
            let context = if title.is_empty() {
                ct_name.clone()
            } else {
                format!("{ct_name}: {title}")
            };
            for (f, ty) in fields {
                let v = &e["fields"][f.as_str()][&def];
                let format = match (ty.as_str(), v) {
                    ("Symbol", Value::String(s)) if cms::has_letter(s) => Format::Plain,
                    ("Text", Value::String(s)) if cms::has_letter(s) => {
                        Format::RichText(cms::md_doc(s))
                    }
                    ("RichText", Value::Object(_)) => {
                        let mut d = v.clone();
                        cms::protect_code(&mut d);
                        Format::RichText(d)
                    }
                    _ => continue,
                };
                items.push(SourceItem {
                    external_id: id.to_string(),
                    field: f.clone(),
                    key: key("contentful", ct, id, f),
                    text: v.as_str().unwrap_or_default().to_string(),
                    format,
                    context: context.clone(),
                    updated_at: e["sys"]["updatedAt"].as_str().and_then(|t| t.parse().ok()),
                    meta: json!({"content_type": ct}),
                });
            }
        }
        let next = skip + entries.len();
        Ok(PullPage {
            items,
            next: (!entries.is_empty() && next < total).then(|| next.to_string()),
        })
    }

    async fn push(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport> {
        let api = Api::new(creds).await?;
        let publish = creds.setting("publish") == Some(&Value::Bool(true));
        let mut by_entry: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_entry.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (id, its) in by_entry {
            match push_entry(&api, id, locale, &its, publish).await {
                Ok(()) => report.ok += its.len(),
                Err(e) => report.failed.push((id.to_string(), format!("{e:#}"))),
            }
        }
        Ok(report)
    }

    fn verify_webhook(&self, headers: &HeaderMap, _body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
    }

    fn push_batch(&self) -> usize {
        25
    }
}

async fn push_entry(
    api: &Api,
    id: &str,
    locale: &str,
    its: &[&PushItem],
    publish: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
        "invalid entry id"
    );
    let path = format!("/entries/{id}");
    for attempt in 0..2 {
        let mut e = api.get(&path).await?;
        let version = e["sys"]["version"].as_u64().unwrap_or(0);
        // Published and unchanged since: publishedVersion == version - 1.
        let was_live = e["sys"]["publishedVersion"]
            .as_u64()
            .is_some_and(|p| p + 1 == version);
        let fields = e["fields"]
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("entry without fields"))?;
        for it in its {
            let v = match &it.format {
                Format::RichText(doc) => cms::pushed_value(doc),
                _ => Value::String(it.value.clone()),
            };
            let f = fields.entry(it.field.clone()).or_insert_with(|| json!({}));
            if let Some(o) = f.as_object_mut() {
                o.insert(locale.to_string(), v);
            }
        }
        let body = json!({"fields": e["fields"]}).to_string();
        let res = send(|| {
            api.req(reqwest::Method::PUT, &path)
                .header("x-contentful-version", version)
                .body(body.clone())
        })
        .await;
        let res = match res {
            // Edited meanwhile: re-read and re-apply.
            Err(e) if attempt == 0 && e.to_string().starts_with("HTTP 409") => continue,
            r => r?,
        };
        let updated: Value = res.json().await?;
        if publish && was_live {
            let v = updated["sys"]["version"].as_u64().unwrap_or(version + 1);
            send(|| {
                api.req(reqwest::Method::PUT, &format!("{path}/published"))
                    .header("x-contentful-version", v)
            })
            .await?;
        }
        return Ok(());
    }
    anyhow::bail!("version conflict")
}
