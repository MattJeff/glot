//! Strapi connector (REST API, v4 and v5, i18n plugin).
//!
//! Credentials: `url` (the Strapi server), `token` (API token, full access or find/update/createLocalization
//! on the translated types + read on i18n and content-type-builder). Settings: `content_types` (plural API
//! ids, `single:<singular>` for single types; else every localized type found by the content-type-builder),
//! `publish` (v5: publish the locale; v4: create localizations published), `version` (4 | 5, detected).
//!
//! Pulled: localized attributes of type string / text (plain), richtext (Markdown, block syntax kept) and
//! blocks (v5 Blocks editor: text nodes only, links / formatting / code kept), in the default locale.
//! Pushed: v5 `PUT /api/{plural}/{documentId}?locale=xx` (creates or updates the locale); v4 `PUT` on the
//! known localization, else `POST /api/{plural}/{id}/localizations`. Components and dynamic zones are not
//! translated. Webhooks: a header `Authorization: Bearer <secret>` set on the Strapi webhook, an
//! `X-Webhook-Signature` HMAC of `<X-Webhook-Timestamp>.<body>`, or the dashboard URL with its token.
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct Strapi;

const PAGE: usize = 100;
const SYSTEM: &[&str] = &[
    "id",
    "documentId",
    "createdAt",
    "updatedAt",
    "publishedAt",
    "locale",
    "localizations",
    "createdBy",
    "updatedBy",
    "slug",
];

struct Api {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let url = required(c, "url")?.trim_end_matches('/').to_string();
        Ok(Api {
            http: super::client_for(&url).await?,
            base: url,
            token: required(c, "token")?.to_string(),
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
            let r = self.http.request(m.clone(), &url).bearer_auth(&self.token);
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
    async fn locales(&self) -> anyhow::Result<(String, Vec<String>)> {
        let l = self.get("/api/i18n/locales").await?;
        let items = l
            .as_array()
            .or(l["data"].as_array())
            .cloned()
            .unwrap_or_default();
        let codes: Vec<String> = items
            .iter()
            .filter_map(|i| i["code"].as_str().map(String::from))
            .collect();
        let def = items
            .iter()
            .find(|i| i["isDefault"] == true)
            .and_then(|i| i["code"].as_str())
            .or(codes.first().map(String::as_str))
            .ok_or_else(|| anyhow::anyhow!("no locale: is the i18n plugin enabled?"))?
            .to_string();
        Ok((def, codes))
    }
}

/// A translated content type: API id (plural, or singular for single types) and its localized text
/// attributes (`None` = unknown schema: string and blocks values are guessed).
#[derive(Debug, Clone)]
struct Type {
    api: String,
    single: bool,
    name: String,
    fields: Option<Vec<(String, String)>>,
}

fn localized(o: &Value) -> bool {
    o["pluginOptions"]["i18n"]["localized"] == true
}

async fn types(api: &Api, c: &Creds) -> anyhow::Result<Vec<Type>> {
    let schema = api
        .get("/api/content-type-builder/content-types")
        .await
        .ok();
    let mut found: Vec<Type> = schema
        .as_ref()
        .and_then(|s| s["data"].as_array())
        .into_iter()
        .flatten()
        .filter(|t| {
            t["uid"].as_str().is_some_and(|u| u.starts_with("api::")) && localized(&t["schema"])
        })
        .map(|t| {
            let s = &t["schema"];
            let single = s["kind"] == "singleType";
            Type {
                api: s[if single { "singularName" } else { "pluralName" }]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                single,
                name: s["displayName"].as_str().unwrap_or_default().to_string(),
                fields: Some(
                    s["attributes"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, a)| {
                            localized(a)
                                && matches!(
                                    a["type"].as_str(),
                                    Some("string" | "text" | "richtext" | "blocks")
                                )
                        })
                        .map(|(n, a)| {
                            (
                                n.clone(),
                                a["type"].as_str().unwrap_or_default().to_string(),
                            )
                        })
                        .collect(),
                ),
            }
        })
        .collect();
    let wanted: Option<Vec<String>> = c
        .setting("content_types")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    if let Some(w) = wanted {
        found = w
            .iter()
            .map(|n| {
                let (single, api) = match n.strip_prefix("single:") {
                    Some(s) => (true, s.to_string()),
                    None => (false, n.clone()),
                };
                found
                    .iter()
                    .find(|t| t.api == api)
                    .cloned()
                    .unwrap_or(Type {
                        name: api.clone(),
                        api,
                        single,
                        fields: None,
                    })
            })
            .collect();
    }
    for t in &found {
        anyhow::ensure!(
            !t.api.is_empty()
                && t.api
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "invalid content type '{}'",
            t.api
        );
    }
    Ok(found)
}

/// Attribute values of an entry to translate: (name, format).
fn fields_of(t: &Type, attrs: &Map<String, Value>) -> Vec<(String, Format)> {
    let pick = |name: &str, ty: Option<&str>, v: &Value| -> Option<Format> {
        match (ty, v) {
            (Some("richtext"), Value::String(s)) if cms::has_letter(s) => {
                Some(Format::RichText(cms::md_doc(s)))
            }
            (Some("string" | "text"), Value::String(s)) if cms::has_letter(s) => {
                Some(Format::Plain)
            }
            (None, Value::String(s)) if !SYSTEM.contains(&name) && cms::looks_like_text(s) => {
                Some(if s.contains('\n') {
                    Format::RichText(cms::md_doc(s))
                } else {
                    Format::Plain
                })
            }
            (Some("blocks") | None, Value::Array(a))
                if a.iter()
                    .any(|b| b["children"].is_array() && b["type"].is_string()) =>
            {
                let mut d = v.clone();
                cms::protect_code(&mut d);
                Some(Format::RichText(d))
            }
            _ => None,
        }
    };
    match &t.fields {
        Some(f) => f
            .iter()
            .filter_map(|(n, ty)| Some((n.clone(), pick(n, Some(ty), attrs.get(n)?)?)))
            .collect(),
        None => attrs
            .iter()
            .filter_map(|(n, v)| Some((n.clone(), pick(n, None, v)?)))
            .collect(),
    }
}

/// `{locale: id}` of an entry's localizations (v4 `localizations.data[]`, v5 `localizations[]`).
fn localizations(attrs: &Map<String, Value>) -> Map<String, Value> {
    let l = attrs.get("localizations");
    let arr = l
        .and_then(|l| l["data"].as_array().or(l.as_array()))
        .cloned()
        .unwrap_or_default();
    arr.iter()
        .filter_map(|x| {
            let loc = x["locale"]
                .as_str()
                .or(x["attributes"]["locale"].as_str())?;
            Some((loc.to_string(), x["id"].clone()))
        })
        .collect()
}

#[async_trait]
impl Connector for Strapi {
    fn kind(&self) -> &'static str {
        "strapi"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let (def, codes) = api.locales().await?;
        let t = types(&api, creds).await?;
        let mut notes = vec![format!("locales: {}", codes.join(", "))];
        if t.is_empty() {
            notes.push("no localized content type found: set content_types or give the token read access to the content-type-builder".into());
        } else {
            notes.push(format!(
                "content types: {}",
                t.iter()
                    .map(|t| t.api.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(AccountInfo {
            name: api.base.clone(),
            source_locale: Some(def),
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(Api::new(creds).await?.locales().await?.1)
    }

    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let (def, _) = api.locales().await?;
        let types = types(&api, creds).await?;
        let (ti, page) = cursor
            .as_deref()
            .and_then(|c| c.split_once(':'))
            .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
            .unwrap_or((0, 1));
        let Some(t) = types.get(ti) else {
            return Ok(PullPage::default());
        };
        let loc = urlencode(&def);
        let mut q = if t.single {
            format!("/api/{}?locale={loc}&populate=localizations", t.api)
        } else {
            format!(
                "/api/{}?locale={loc}&populate=localizations&pagination[page]={page}&pagination[pageSize]={PAGE}&sort=id:asc",
                t.api
            )
        };
        if let Some(s) = creds.since.filter(|_| !t.single) {
            q.push_str(&format!(
                "&filters[updatedAt][$gte]={}",
                s.format("%Y-%m-%dT%H:%M:%S.000Z")
            ));
        }
        let res = match api.get(&q).await {
            // A single type without content yet.
            Err(e) if t.single && e.to_string().starts_with("HTTP 404") => json!({"data": null}),
            r => r?,
        };
        let entries: Vec<Value> = match &res["data"] {
            Value::Array(a) => a.clone(),
            Value::Null => vec![],
            d => vec![d.clone()],
        };
        let mut items = vec![];
        for e in &entries {
            let v5 = e.get("attributes").is_none();
            let attrs = if v5 {
                e.as_object()
            } else {
                e["attributes"].as_object()
            };
            let Some(attrs) = attrs else { continue };
            let id = if v5 {
                e["documentId"].as_str().map(String::from)
            } else {
                e["id"].as_u64().map(|i| i.to_string())
            };
            let Some(id) = id else { continue };
            let external_id = if t.single { t.api.clone() } else { id.clone() };
            let title = ["title", "name", "heading"]
                .iter()
                .find_map(|f| attrs.get(*f).and_then(Value::as_str))
                .unwrap_or_default();
            let context = if title.is_empty() {
                t.name.clone()
            } else {
                format!("{}: {title}", t.name)
            };
            let meta = json!({"v": if v5 { 5 } else { 4 }, "api": t.api, "single": t.single, "id": id, "loc": localizations(attrs)});
            for (f, format) in fields_of(t, attrs) {
                items.push(SourceItem {
                    key: key("strapi", &t.api, &external_id, &f),
                    external_id: external_id.clone(),
                    text: attrs[&f].as_str().unwrap_or_default().to_string(),
                    field: f,
                    format,
                    context: context.clone(),
                    updated_at: attrs
                        .get("updatedAt")
                        .and_then(Value::as_str)
                        .and_then(|t| t.parse().ok()),
                    meta: meta.clone(),
                });
            }
        }
        let page_count = res["meta"]["pagination"]["pageCount"].as_u64().unwrap_or(1) as usize;
        let next = if !t.single && page < page_count && !entries.is_empty() {
            Some(format!("{ti}:{}", page + 1))
        } else if ti + 1 < types.len() {
            Some(format!("{}:1", ti + 1))
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
        let publish = creds.setting("publish") == Some(&Value::Bool(true));
        let mut by_entry: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_entry.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (id, its) in by_entry {
            let mut data = Map::new();
            for it in &its {
                let v = match &it.format {
                    Format::RichText(doc) => cms::pushed_value(doc),
                    _ => Value::String(it.value.clone()),
                };
                data.insert(it.field.clone(), v);
            }
            match push_entry(&api, &its[0].meta, locale, data, publish).await {
                Ok(()) => report.ok += its.len(),
                Err(e) => report.failed.push((id.to_string(), format!("{e:#}"))),
            }
        }
        Ok(report)
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        if cms::shared_secret_ok(headers, secret) {
            return true;
        }
        let (Some(sig), Some(ts)) = (
            cms::header(headers, "x-webhook-signature"),
            cms::header(headers, "x-webhook-timestamp"),
        ) else {
            return false;
        };
        let mut signed = format!("{ts}.").into_bytes();
        signed.extend_from_slice(body);
        !secret.is_empty() && super::hmac_ok(secret, &signed, sig)
    }

    fn push_batch(&self) -> usize {
        25
    }
}

fn urlencode(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "-_".contains(*c))
        .collect()
}

async fn push_entry(
    api: &Api,
    meta: &Value,
    locale: &str,
    data: Map<String, Value>,
    publish: bool,
) -> anyhow::Result<()> {
    let t = meta["api"].as_str().unwrap_or_default();
    let single = meta["single"] == true;
    let loc = urlencode(locale);
    let id = match &meta["id"] {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    anyhow::ensure!(
        id.chars().all(|c| c.is_ascii_alphanumeric()),
        "invalid entry id"
    );
    if meta["v"] == 5 {
        let status = if publish { "" } else { "&status=draft" };
        let path = if single {
            format!("/api/{t}?locale={loc}{status}")
        } else {
            format!("/api/{t}/{id}?locale={loc}{status}")
        };
        api.call(reqwest::Method::PUT, &path, Some(&json!({ "data": data })))
            .await?;
        return Ok(());
    }
    // v4: update the known localization, else create it.
    let put = |lid: &Value| {
        let path = if single {
            format!("/api/{t}?locale={loc}")
        } else {
            format!("/api/{t}/{}", lid.as_u64().unwrap_or(0))
        };
        (path, json!({ "data": data.clone() }))
    };
    if let Some(lid) = meta["loc"].get(locale) {
        let (path, body) = put(lid);
        api.call(reqwest::Method::PUT, &path, Some(&body)).await?;
        return Ok(());
    }
    let create = if single {
        format!("/api/{t}/localizations")
    } else {
        format!("/api/{t}/{id}/localizations")
    };
    let mut body = data.clone();
    body.insert("locale".into(), json!(locale));
    if publish {
        body.insert("publishedAt".into(), json!(chrono::Utc::now().to_rfc3339()));
    }
    match api
        .call(reqwest::Method::POST, &create, Some(&Value::Object(body)))
        .await
    {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().starts_with("HTTP 400") && !single => {
            // Created since the last pull: find it and update it.
            let cur = api
                .get(&format!("/api/{t}/{id}?populate=localizations"))
                .await?;
            let lids = localizations(cur["data"]["attributes"].as_object().ok_or(e)?);
            let lid = lids
                .get(locale)
                .ok_or_else(|| anyhow::anyhow!("localization {locale} not found"))?;
            let (path, body) = put(lid);
            api.call(reqwest::Method::PUT, &path, Some(&body)).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}
