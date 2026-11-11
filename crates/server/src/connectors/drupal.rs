//! Drupal 10/11 (core JSON:API + Content Translation): nodes and taxonomy terms.
//!
//! Credentials: `url` (the site), and either `username` + `password` (core `basic_auth` module) or `token`
//! (OAuth bearer, e.g. Simple OAuth). JSON:API must allow writes (`/admin/config/services/jsonapi` → *Accept
//! all JSON:API create, read, update, and delete operations*). Settings: `resources` (`["node--article",
//! "taxonomy_term--tags"]`; default: every `node--*` and `taxonomy_term--*` of the JSON:API index),
//! `source_lang`.
//!
//! - Pull: `GET /jsonapi/{entity}/{bundle}?page[limit]=50&sort=drupal_internal__…` following `links.next`.
//!   Fields: `title` / `name` (plain) and every formatted text attribute (`{value, format, summary}`:
//!   `body`, text_long fields) as HTML, tags kept; the text format is kept as is.
//! - Push: translations are written with `PATCH /{langcode}/jsonapi/{entity}/{bundle}/{uuid}` (the URL
//!   language prefix selects the translation). JSON:API cannot **create** a translation: glot reads the
//!   translation first (`GET /{langcode}/jsonapi/…`) and, when Drupal answers with another language (no
//!   translation yet), reports it so an editor adds it once (Translate tab); glot keeps it updated afterwards.
//! - Webhooks: Drupal has none built in; a custom module or ECA calls the card URL (token included) or sends
//!   `X-Glot-Webhook-Secret`.
//!
//! Docs: https://www.drupal.org/docs/core-modules-and-themes/core-modules/jsonapi-module/translations,
//! https://www.drupal.org/docs/core-modules-and-themes/core-modules/jsonapi-module/updating-existing-resources-patch,
//! https://www.drupal.org/docs/core-modules-and-themes/core-modules/jsonapi-module/pagination
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct Drupal;

const MIME: &str = "application/vnd.api+json";

struct Api {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let base = required(c, "url")?.trim_end_matches('/').to_string();
        let auth = match c.opt("token") {
            Some(t) => format!("Bearer {t}"),
            None => format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!(
                    "{}:{}",
                    required(c, "username")?,
                    required(c, "password")?
                ))
            ),
        };
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
        let url = format!("{}{path}", self.base);
        let res = send(|| {
            let r = self
                .http
                .request(m.clone(), &url)
                .header("authorization", &self.auth)
                .header("accept", MIME);
            match body {
                Some(b) => r.header("content-type", MIME).body(b.to_string()),
                None => r,
            }
        })
        .await?;
        Ok(res.json().await.unwrap_or(Value::Null))
    }
    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }
    /// A `links.*.href` of this site → its path (absolute URLs elsewhere are refused).
    fn path_of(&self, href: &str) -> anyhow::Result<String> {
        let u = reqwest::Url::parse(href)?;
        let b = reqwest::Url::parse(&self.base)?;
        anyhow::ensure!(
            u.host_str() == b.host_str(),
            "JSON:API link to another host"
        );
        let prefix = b.path().trim_end_matches('/');
        let p = u.path().strip_prefix(prefix).unwrap_or(u.path());
        Ok(match u.query() {
            Some(q) => format!("{p}?{q}"),
            None => p.to_string(),
        })
    }
}

/// `node--article` → (`node`, `article`), identifiers only.
fn split_type(t: &str) -> Option<(&str, &str)> {
    let (e, b) = t.split_once("--")?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (ok(e) && ok(b)).then_some((e, b))
}

async fn resources(api: &Api, c: &Creds) -> anyhow::Result<Vec<String>> {
    if let Some(a) = c.setting("resources").and_then(Value::as_array) {
        return Ok(a
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect());
    }
    let idx = api.get("/jsonapi").await?;
    let mut out: Vec<String> = idx["links"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.keys())
        .filter(|k| k.starts_with("node--") || k.starts_with("taxonomy_term--"))
        .cloned()
        .collect();
    out.sort();
    Ok(out)
}

fn items_of(ty: &str, o: &Value) -> Vec<SourceItem> {
    let Some(id) = o["id"].as_str() else {
        return vec![];
    };
    let a = &o["attributes"];
    let title = a["title"].as_str().or(a["name"].as_str()).unwrap_or("");
    let meta = |f: Value| json!({"type": ty, "id": id, "formats": f});
    let mut formats = Map::new();
    let mut out = vec![];
    let mut add = |field: String, text: &str, html: bool| {
        if cms::has_letter(text) {
            out.push(SourceItem {
                external_id: format!("{ty}:{id}"),
                key: key("drupal", ty, id, &field),
                context: format!(
                    "Drupal {} \u{ab}{title}\u{bb}, field {field}",
                    ty.replace("--", " ")
                ),
                field,
                text: text.to_string(),
                format: if html { Format::Html } else { Format::Plain },
                updated_at: None,
                meta: Value::Null,
            })
        }
    };
    for f in ["title", "name"] {
        if let Some(v) = a[f].as_str() {
            add(f.to_string(), v, false);
        }
    }
    for (name, v) in a.as_object().into_iter().flatten() {
        if let (Some(val), true) = (v["value"].as_str(), v.get("format").is_some()) {
            formats.insert(name.clone(), v["format"].clone());
            add(format!("{name}.value"), val, true);
            if let Some(s) = v["summary"].as_str() {
                add(format!("{name}.summary"), s, true);
            }
        }
    }
    let m = meta(Value::Object(formats));
    out.into_iter()
        .map(|mut i| {
            i.meta = m.clone();
            i
        })
        .collect()
}

#[async_trait]
impl Connector for Drupal {
    fn kind(&self) -> &'static str {
        "drupal"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let res = resources(&api, creds).await?;
        let langs = self.locales(creds).await?;
        let mut source = cms::param(creds, "source_lang").map(String::from);
        if let (None, Some((e, b))) = (&source, res.first().and_then(|t| split_type(t))) {
            let p = api.get(&format!("/jsonapi/{e}/{b}?page[limit]=1")).await?;
            source = p["data"][0]["attributes"]["langcode"]
                .as_str()
                .map(String::from);
        }
        Ok(AccountInfo {
            name: api.base.clone(),
            source_locale: source,
            notes: vec![
                format!("Resources: {}", res.join(", ")),
                format!("Languages: {}", langs.join(", ")),
                "JSON:API cannot create translations: add each translation once in Drupal (Translate tab); glot then keeps it updated.".into(),
            ],
        })
    }

    /// `configurable_language` config entities (read-only in JSON:API), without `und` / `zxx`.
    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let api = Api::new(creds).await?;
        let l = api
            .get("/jsonapi/configurable_language/configurable_language")
            .await?;
        Ok(l["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["attributes"]["drupal_internal__id"].as_str())
            .filter(|c| !matches!(*c, "und" | "zxx"))
            .map(String::from)
            .collect())
    }

    /// Cursor = `{resource index}|{next path}` (empty = first page).
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let res = resources(&api, creds).await?;
        let (i, next) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.to_string()),
            None => (0, String::new()),
        };
        let Some(ty) = res.get(i) else {
            return Ok(PullPage::default());
        };
        let (e, b) = split_type(ty).ok_or_else(|| anyhow::anyhow!("invalid resource '{ty}'"))?;
        let path = if next.is_empty() {
            format!(
                "/jsonapi/{e}/{b}?page[limit]=50&sort=drupal_internal__{}",
                if e == "node" { "nid" } else { "tid" }
            )
        } else {
            anyhow::ensure!(next.starts_with("/jsonapi/"), "invalid cursor");
            next
        };
        let page = api.get(&path).await?;
        let items = page["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|o| items_of(ty, o))
            .collect();
        let next = match page["links"]["next"]["href"].as_str() {
            Some(h) => Some(format!("{i}|{}", api.path_of(h)?)),
            None if i + 1 < res.len() => Some(format!("{}|", i + 1)),
            None => None,
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
        anyhow::ensure!(
            locale
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "invalid langcode"
        );
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let meta = &its[0].meta;
            let r: anyhow::Result<()> = async {
                let (Some(ty), Some(id)) = (meta["type"].as_str(), meta["id"].as_str()) else {
                    anyhow::bail!("missing metadata, sync again");
                };
                let (e, b) = split_type(ty).ok_or_else(|| anyhow::anyhow!("invalid type"))?;
                anyhow::ensure!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'), "invalid uuid");
                let path = format!("/{locale}/jsonapi/{e}/{b}/{id}");
                let cur = api.get(&path).await?;
                let cur = &cur["data"]["attributes"];
                anyhow::ensure!(
                    cur["langcode"].as_str() == Some(locale),
                    "no {locale} translation of this content yet: add it once in Drupal (Translate tab), glot keeps it updated afterwards"
                );
                let mut attrs = Map::new();
                for it in &its {
                    match it.field.split_once('.') {
                        Some((f, part)) => {
                            let slot = attrs.entry(f.to_string()).or_insert_with(|| {
                                let mut v = cur[f].clone();
                                if !v.is_object() {
                                    v = json!({"format": meta["formats"][f]});
                                }
                                v.as_object_mut().map(|o| o.remove("processed"));
                                v
                            });
                            slot[part] = Value::String(it.value.clone());
                        }
                        None => {
                            attrs.insert(it.field.clone(), Value::String(it.value.clone()));
                        }
                    }
                }
                let body = json!({"data": {"type": ty, "id": id, "attributes": attrs}});
                api.call(reqwest::Method::PATCH, &path, Some(&body)).await?;
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
        25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_fields() {
        let o = json!({"type": "node--article", "id": "1f0a-22", "attributes": {
            "title": "Hello", "langcode": "en", "path": {"alias": "/hello"},
            "body": {"value": "<p>Hi <em>all</em></p>", "format": "basic_html", "processed": "x", "summary": ""}}});
        let items = items_of("node--article", &o);
        let f: Vec<_> = items.iter().map(|i| i.field.as_str()).collect();
        assert_eq!(f, ["title", "body.value"]);
        assert_eq!(items[1].meta["formats"]["body"], "basic_html");
        assert_eq!(split_type("node--article"), Some(("node", "article")));
        assert_eq!(split_type("node--../x"), None);
    }
}
