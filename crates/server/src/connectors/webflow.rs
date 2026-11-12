//! Webflow connector: Data API v2 with Localization.
//!
//! - Locales: `GET /sites/{site_id}` → `locales.primary` / `locales.secondary[]` (`id` for pages and components,
//!   `cmsLocaleId` for the CMS, `tag` = BCP 47 code).
//! - CMS: `GET /sites/{site_id}/collections`, `GET /collections/{id}` (schema: `PlainText` → plain, `RichText` →
//!   HTML; the `slug` field is left alone), `GET /collections/{id}/items` (primary locale, 100 per page);
//!   push = `PATCH /collections/{id}/items` with `items[{id, cmsLocaleId, fieldData}]` (≤ 100), then, when the
//!   `publish` setting is on, `POST /collections/{id}/items/publish` with `items[{id, cmsLocaleIds}]`.
//! - Static page content: `GET /sites/{site_id}/pages`, `GET /pages/{id}/dom` (text nodes as HTML, component
//!   instance property overrides, form placeholders and button labels); push = `POST /pages/{id}/dom?localeId=`.
//! - Components: `GET /sites/{site_id}/components`, `GET|POST /sites/{site_id}/components/{id}/dom?localeId=`.
//! - Webhooks: `x-webflow-signature` = hex HMAC-SHA256(secret, `{x-webflow-timestamp}:{body}`), timestamp in
//!   ms within 5 minutes. Dashboard-created webhooks are unsigned: they use the account's `?token=` URL.
//!
//! - Read back (`current`): `GET /collections/{id}/items/{item_id}?cmsLocaleId=`, `GET /pages/{id}/dom?localeId=`
//!   and the component DOM, each compared with the primary locale: a secondary locale answers with the
//!   primary content until it is localized, so an identical value is "no translation". `remove` is not
//!   implemented: the API has no call that resets a localized value to the inherited one.
//! - Diagnosis (`doctor`): `GET /token/introspect` (site, workspace and OAuth tokens) → `authorization.scope`
//!   against the scopes glot needs (`sites:read`, `cms:read|write`, `pages:read|write`,
//!   `components:read|write`), and whether the site has secondary locales (Localization).
//!
//! Docs: https://developers.webflow.com/data/reference/cms/collection-items/staged-items/get-item,
//! https://developers.webflow.com/data/reference/pages-and-components/pages/get-content,
//! https://developers.webflow.com/data/reference/token/introspect,
//! https://developers.webflow.com/data/reference/scopes
//!
//! Cursor: `cms:{collection index}:{offset}` → `page:{index}` → `component:{index}` (one listing call each).
//! Credentials: `token` (site API token or OAuth token), optional `site_id` (else the token's only site).
//! Settings: `publish` (bool), `api_base` (tests only, honoured with `GLOT_CONNECTORS_ALLOW_PRIVATE=1`).
use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem,
};

pub struct Webflow;

const KIND: &str = "webflow";
const API: &str = "https://api.webflow.com/v2";
/// Scopes a full sync needs.
const SCOPES: &[&str] = &[
    "sites:read",
    "cms:read",
    "cms:write",
    "pages:read",
    "pages:write",
    "components:read",
    "components:write",
];

struct Api {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let base = c
            .setting("api_base")
            .and_then(Value::as_str)
            .filter(|_| super::allow_private())
            .unwrap_or(API)
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            http: super::client_for(&base).await?,
            base,
            token: c.get("token")?.to_string(),
        })
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = format!("{}{path}", self.base);
        let res = super::send(|| {
            let r = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&self.token)
                .header("accept", "application/json");
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

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }

    /// The configured site, else the token's only site.
    async fn site_id(&self, c: &Creds) -> anyhow::Result<String> {
        if let Some(id) = c.opt("site_id") {
            return Ok(id.to_string());
        }
        let sites = self.get("/sites").await?;
        let sites = sites["sites"].as_array().cloned().unwrap_or_default();
        match sites.as_slice() {
            [one] => Ok(str_of(&one["id"]).to_string()),
            [] => anyhow::bail!("this token has access to no site"),
            _ => anyhow::bail!("this token has access to several sites: set the site ID"),
        }
    }
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn total(v: &Value) -> u64 {
    v["pagination"]["total"].as_u64().unwrap_or(0)
}

fn updated(v: &Value) -> Option<DateTime<Utc>> {
    v["lastUpdated"].as_str()?.parse().ok()
}

/// Secondary locale of the site matching a glot locale (`fr` ↔ `fr-FR`): (localeId, cmsLocaleId).
fn locale_ids(site: &Value, locale: &str) -> anyhow::Result<(String, String)> {
    let sec = site["locales"]["secondary"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let lang = |s: &str| {
        s.split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let hit = sec
        .iter()
        .find(|l| str_of(&l["tag"]).eq_ignore_ascii_case(locale))
        .or_else(|| sec.iter().find(|l| lang(str_of(&l["tag"])) == lang(locale)))
        .ok_or_else(|| anyhow::anyhow!("no secondary locale '{locale}' on this Webflow site"))?;
    Ok((
        str_of(&hit["id"]).to_string(),
        str_of(&hit["cmsLocaleId"]).to_string(),
    ))
}

/// Translatable parts of a page/component DOM: (field, value, format, label).
fn dom_fields(nodes: &[Value]) -> Vec<(String, String, Format, &'static str)> {
    let mut out = vec![];
    for n in nodes {
        let id = str_of(&n["id"]);
        let mut put = |field: String, v: &Value, f: Format, label| {
            if let Some(s) = v.as_str().filter(|s| !s.trim().is_empty()) {
                out.push((field, s.to_string(), f, label));
            }
        };
        match str_of(&n["type"]) {
            "text" => put(id.into(), &n["text"]["html"], Format::Html, "text"),
            "component-instance" => {
                for o in n["propertyOverrides"].as_array().into_iter().flatten() {
                    let pid = str_of(&o["propertyId"]);
                    // Rich text properties come back as HTML, others as plain text.
                    let html = &o["text"]["html"];
                    if html.as_str().is_some_and(|h| h.contains('<')) {
                        put(
                            format!("{id}/{pid}"),
                            html,
                            Format::Html,
                            "component property",
                        );
                    } else {
                        put(
                            format!("{id}/{pid}"),
                            &o["text"]["text"],
                            Format::Plain,
                            "component property",
                        );
                    }
                }
            }
            "text-input" => put(
                format!("{id}/placeholder"),
                &n["placeholder"],
                Format::Plain,
                "input placeholder",
            ),
            "submit-button" => {
                put(
                    format!("{id}/value"),
                    &n["value"],
                    Format::Plain,
                    "button label",
                );
                put(
                    format!("{id}/waitingText"),
                    &n["waitingText"],
                    Format::Plain,
                    "button waiting label",
                );
            }
            "search-button" => put(
                format!("{id}/value"),
                &n["value"],
                Format::Plain,
                "search button label",
            ),
            // Images (alt via assets), selects (choices are values too): left to Webflow.
            _ => {}
        }
    }
    out
}

/// Every node of a DOM endpoint (paginated by 100).
async fn all_nodes(api: &Api, path: &str) -> anyhow::Result<Vec<Value>> {
    let mut nodes = vec![];
    loop {
        let sep = if path.contains('?') { '&' } else { '?' };
        let v = api
            .get(&format!("{path}{sep}limit=100&offset={}", nodes.len()))
            .await?;
        let page = v["nodes"].as_array().cloned().unwrap_or_default();
        let done = page.is_empty() || nodes.len() + page.len() >= total(&v) as usize;
        nodes.extend(page);
        if done {
            return Ok(nodes);
        }
    }
}

/// Field → value of one resource (`page/{id}`, `component/{id}`, `{collection}/{item}`) in a locale
/// (`None` = primary): the lookup `current` compares between the two.
async fn values(
    api: &Api,
    site: &str,
    ext: &str,
    locale: Option<(&str, &str)>,
) -> anyhow::Result<BTreeMap<String, String>> {
    let (res, id) = ext.split_once('/').unwrap_or(("", ext));
    let dom = |path: String| async move {
        let path = match locale {
            Some((locale_id, _)) => format!("{path}?localeId={locale_id}"),
            None => path,
        };
        anyhow::Ok(
            dom_fields(&all_nodes(api, &path).await?)
                .into_iter()
                .map(|(field, value, ..)| (field, value))
                .collect(),
        )
    };
    match res {
        "page" => dom(format!("/pages/{id}/dom")).await,
        "component" => dom(format!("/sites/{site}/components/{id}/dom")).await,
        cid => {
            let q = locale
                .map(|(_, cms)| format!("?cmsLocaleId={cms}"))
                .unwrap_or_default();
            let item = match api.get(&format!("/collections/{cid}/items/{id}{q}")).await {
                // The item does not exist in this locale.
                Err(e) if e.to_string().starts_with("HTTP 404") => Value::Null,
                r => r?,
            };
            Ok(item["fieldData"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect())
        }
    }
}

/// DOM update body for one page/component: fields grouped per node.
fn dom_update(items: &[&PushItem]) -> Value {
    let mut nodes: BTreeMap<&str, serde_json::Map<String, Value>> = BTreeMap::new();
    for it in items {
        let (node, part) = it.field.split_once('/').unwrap_or((&it.field, ""));
        let n = nodes.entry(node).or_default();
        n.insert("nodeId".into(), json!(node));
        match part {
            "" => {
                n.insert("text".into(), json!(it.value));
            }
            "placeholder" | "value" | "waitingText" => {
                n.insert(part.into(), json!(it.value));
            }
            pid => {
                let o = n.entry("propertyOverrides").or_insert_with(|| json!([]));
                if let Some(a) = o.as_array_mut() {
                    a.push(json!({"propertyId": pid, "text": it.value}));
                }
            }
        }
    }
    json!({"nodes": nodes.into_values().collect::<Vec<_>>()})
}

fn dom_items(resource: &str, id: &str, title: &str, nodes: &[Value]) -> Vec<SourceItem> {
    dom_fields(nodes)
        .into_iter()
        .map(|(field, text, format, label)| SourceItem {
            external_id: format!("{resource}/{id}"),
            key: super::key(KIND, resource, id, &field),
            field,
            text,
            format,
            context: format!("Webflow {resource} “{title}”, {label}"),
            updated_at: None,
            meta: Value::Null,
        })
        .collect()
}

#[async_trait]
impl Connector for Webflow {
    fn kind(&self) -> &'static str {
        KIND
    }

    async fn test(&self, c: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(c).await?;
        let site = api
            .get(&format!("/sites/{}", api.site_id(c).await?))
            .await?;
        let mut notes = vec![];
        if site["locales"]["secondary"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            notes.push(
                "No secondary locale on this site: add locales in Webflow (Localization) first."
                    .into(),
            );
        }
        Ok(AccountInfo {
            name: str_of(&site["displayName"]).to_string(),
            source_locale: site["locales"]["primary"]["tag"]
                .as_str()
                .map(str::to_string),
            notes,
        })
    }

    async fn locales(&self, c: &Creds) -> anyhow::Result<Vec<String>> {
        let api = Api::new(c).await?;
        let site = api
            .get(&format!("/sites/{}", api.site_id(c).await?))
            .await?;
        Ok(site["locales"]["secondary"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l["tag"].as_str().map(str::to_string))
            .collect())
    }

    async fn pull(&self, c: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(c).await?;
        let site = api.site_id(c).await?;
        let cursor = cursor.unwrap_or_else(|| "cms:0:0".into());
        let parts: Vec<&str> = cursor.split(':').collect();
        let num = |i: usize| {
            parts
                .get(i)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let fresh = |v: &Value| match (c.since, updated(v)) {
            (Some(s), Some(u)) => u >= s,
            _ => true,
        };
        match parts[0] {
            "cms" => {
                let (ci, offset) = (num(1) as usize, num(2));
                let cols = api.get(&format!("/sites/{site}/collections")).await?;
                let Some(col) = cols["collections"]
                    .as_array()
                    .and_then(|a| a.get(ci))
                    .cloned()
                else {
                    return Ok(PullPage {
                        items: vec![],
                        next: Some("page:0".into()),
                    });
                };
                let cid = str_of(&col["id"]);
                let schema = api.get(&format!("/collections/{cid}")).await?;
                let fields: Vec<(String, Format)> = schema["fields"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|f| {
                        f["isEditable"].as_bool() != Some(false) && str_of(&f["slug"]) != "slug"
                    })
                    .filter_map(|f| {
                        let format = match str_of(&f["type"]) {
                            "PlainText" => Format::Plain,
                            "RichText" => Format::Html,
                            _ => return None,
                        };
                        Some((str_of(&f["slug"]).to_string(), format))
                    })
                    .collect();
                let page = api
                    .get(&format!(
                        "/collections/{cid}/items?limit=100&offset={offset}"
                    ))
                    .await?;
                let list = page["items"].as_array().cloned().unwrap_or_default();
                let col_name = str_of(&col["displayName"]);
                let mut items = vec![];
                for it in list
                    .iter()
                    .filter(|i| i["isArchived"].as_bool() != Some(true) && fresh(i))
                {
                    let iid = str_of(&it["id"]);
                    let title = it["fieldData"]["name"].as_str().unwrap_or(iid);
                    for (slug, format) in &fields {
                        let Some(text) = it["fieldData"][slug.as_str()]
                            .as_str()
                            .filter(|s| !s.trim().is_empty())
                        else {
                            continue;
                        };
                        items.push(SourceItem {
                            external_id: format!("{cid}/{iid}"),
                            field: slug.clone(),
                            key: super::key(KIND, "item", iid, slug),
                            text: text.to_string(),
                            format: format.clone(),
                            context: format!("Webflow CMS {col_name} “{title}”, field {slug}"),
                            updated_at: updated(it),
                            meta: Value::Null,
                        });
                    }
                }
                let next = if !list.is_empty() && offset + (list.len() as u64) < total(&page) {
                    format!("cms:{ci}:{}", offset + list.len() as u64)
                } else {
                    format!("cms:{}:0", ci + 1)
                };
                Ok(PullPage {
                    items,
                    next: Some(next),
                })
            }
            "page" => {
                let i = num(1);
                let v = api
                    .get(&format!("/sites/{site}/pages?limit=1&offset={i}"))
                    .await?;
                let Some(p) = v["pages"].as_array().and_then(|a| a.first()).cloned() else {
                    return Ok(PullPage {
                        items: vec![],
                        next: Some("component:0".into()),
                    });
                };
                let next = Some(format!("page:{}", i + 1));
                if p["archived"].as_bool() == Some(true) || !fresh(&p) {
                    return Ok(PullPage {
                        items: vec![],
                        next,
                    });
                }
                let pid = str_of(&p["id"]);
                let nodes = all_nodes(&api, &format!("/pages/{pid}/dom")).await?;
                Ok(PullPage {
                    items: dom_items("page", pid, str_of(&p["title"]), &nodes),
                    next,
                })
            }
            "component" => {
                let i = num(1);
                let v = api
                    .get(&format!("/sites/{site}/components?limit=1&offset={i}"))
                    .await?;
                let Some(comp) = v["components"].as_array().and_then(|a| a.first()).cloned() else {
                    return Ok(PullPage::default());
                };
                let next = Some(format!("component:{}", i + 1));
                // Workspace library components cannot be updated from this site.
                if comp["readonly"].as_bool() == Some(true) {
                    return Ok(PullPage {
                        items: vec![],
                        next,
                    });
                }
                let id = str_of(&comp["id"]);
                let nodes = all_nodes(&api, &format!("/sites/{site}/components/{id}/dom")).await?;
                Ok(PullPage {
                    items: dom_items("component", id, str_of(&comp["name"]), &nodes),
                    next,
                })
            }
            _ => anyhow::bail!("bad cursor '{cursor}'"),
        }
    }

    async fn push(
        &self,
        c: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport> {
        let api = Api::new(c).await?;
        let site = api.site_id(c).await?;
        let (locale_id, cms_locale) =
            locale_ids(&api.get(&format!("/sites/{site}")).await?, locale)?;
        let publish = c
            .setting("publish")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut groups: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            groups.entry(&it.external_id).or_default().push(it);
        }
        // CMS items, batched per collection; pages and components, one DOM update each.
        let mut collections: BTreeMap<&str, Vec<(&str, &str, Vec<&PushItem>)>> = BTreeMap::new();
        let mut report = PushReport::default();
        for (ext, its) in groups {
            let (res, id) = ext.split_once('/').unwrap_or(("", ext));
            let path = match res {
                "page" => format!("/pages/{id}/dom?localeId={locale_id}"),
                "component" => format!("/sites/{site}/components/{id}/dom?localeId={locale_id}"),
                cid => {
                    collections.entry(cid).or_default().push((ext, id, its));
                    continue;
                }
            };
            let r = api
                .call(reqwest::Method::POST, &path, Some(&dom_update(&its)))
                .await;
            match r {
                Ok(v) => match v["errors"].as_array().filter(|e| !e.is_empty()) {
                    Some(e) => report.failed.push((
                        ext.to_string(),
                        e.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join("; "),
                    )),
                    None => report.ok += its.len(),
                },
                Err(e) => report.failed.push((ext.to_string(), format!("{e:#}"))),
            }
        }
        for (cid, list) in collections {
            let body = json!({"items": list.iter().map(|(_, id, its)| json!({
                "id": id,
                "cmsLocaleId": cms_locale,
                "fieldData": its.iter().map(|i| (i.field.clone(), json!(i.value))).collect::<serde_json::Map<_, _>>(),
            })).collect::<Vec<_>>()});
            match api
                .call(
                    reqwest::Method::PATCH,
                    &format!("/collections/{cid}/items"),
                    Some(&body),
                )
                .await
            {
                Ok(_) => report.ok += list.iter().map(|x| x.2.len()).sum::<usize>(),
                Err(e) => {
                    let e = format!("{e:#}");
                    report
                        .failed
                        .extend(list.iter().map(|(ext, ..)| (ext.to_string(), e.clone())));
                    continue;
                }
            }
            if publish {
                let body = json!({"items": list.iter().map(|(_, id, _)| json!({"id": id, "cmsLocaleIds": [cms_locale]})).collect::<Vec<_>>()});
                // Written but not published is not a push failure: the next site publish shows it.
                if let Err(e) = api
                    .call(
                        reqwest::Method::POST,
                        &format!("/collections/{cid}/items/publish"),
                        Some(&body),
                    )
                    .await
                {
                    tracing::warn!(collection = cid, "webflow publish failed: {e:#}");
                }
            }
        }
        Ok(report)
    }

    // ponytail: two reads per resource (locale + primary); list items with a comma-separated
    // `cmsLocaleId` if large CMS batches hit the rate limit.
    async fn current(
        &self,
        c: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<Option<Vec<Option<String>>>> {
        let api = Api::new(c).await?;
        let site = api.site_id(c).await?;
        let (locale_id, cms_locale) =
            locale_ids(&api.get(&format!("/sites/{site}")).await?, locale)?;
        type Fields = BTreeMap<String, String>;
        let mut seen: BTreeMap<&str, (Fields, Fields)> = BTreeMap::new();
        for it in items {
            let ext = it.external_id.as_str();
            if !seen.contains_key(ext) {
                let there = values(&api, &site, ext, Some((&locale_id, &cms_locale))).await?;
                let primary = values(&api, &site, ext, None).await?;
                seen.insert(ext, (there, primary));
            }
        }
        Ok(Some(
            items
                .iter()
                .map(|it| {
                    let (there, primary) = &seen[it.external_id.as_str()];
                    there
                        .get(&it.field)
                        .filter(|v| !v.trim().is_empty() && primary.get(&it.field) != Some(v))
                        .cloned()
                })
                .collect(),
        ))
    }

    async fn doctor(&self, c: &Creds) -> Vec<Check> {
        let api = match Api::new(c).await {
            Ok(a) => a,
            Err(e) => return vec![Check::fail("scopes", format!("{e:#}"))],
        };
        let mut out = vec![];
        match api.get("/token/introspect").await {
            Ok(t) => {
                let granted: Vec<&str> = str_of(&t["authorization"]["scope"])
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .collect();
                let missing: Vec<&str> = SCOPES
                    .iter()
                    .copied()
                    .filter(|s| !granted.iter().any(|g| g.eq_ignore_ascii_case(s)))
                    .collect();
                out.push(if missing.is_empty() {
                    Check::ok("scopes", SCOPES.join(", "))
                } else {
                    Check {
                        fix: Some("permissions"),
                        ..Check::fail("scopes", format!("missing scope: {}", missing.join(", ")))
                    }
                });
            }
            Err(e) => out.push(Check::warn(
                "scopes",
                format!("token scopes could not be read: {e:#}"),
                None,
            )),
        }
        let site = async { api.get(&format!("/sites/{}", api.site_id(c).await?)).await };
        match site.await {
            Ok(site) => {
                let tags: Vec<&str> = site["locales"]["secondary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|l| str_of(&l["tag"]))
                    .collect();
                out.push(if tags.is_empty() {
                    Check::warn(
                        "localization",
                        "no secondary locale: enable Localization on the site and add the target locales",
                        Some("enable_locale"),
                    )
                } else {
                    Check::ok("localization", tags.join(", "))
                });
            }
            Err(e) => out.push(Check::fail("localization", format!("{e:#}"))),
        }
        out
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
        let (Some(ts), Some(sig)) = (h("x-webflow-timestamp"), h("x-webflow-signature")) else {
            return false;
        };
        let Ok(ms) = ts.trim().parse::<i64>() else {
            return false;
        };
        if (Utc::now().timestamp_millis() - ms).abs() > 300_000 {
            return false;
        }
        let mut signed = format!("{}:", ts.trim()).into_bytes();
        signed.extend_from_slice(body);
        super::hmac_ok(secret, &signed, sig)
    }

    fn push_batch(&self) -> usize {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path, Query},
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(String, Value)>>>;

    /// Simulated Webflow Data API v2 (response shapes from developers.webflow.com/data/reference).
    async fn mock() -> (String, Log) {
        let log: Log = Arc::default();
        let site = || {
            json!({"id": "site1", "displayName": "Acme", "locales": {
                "primary": {"id": "loc-en", "cmsLocaleId": "cms-en", "tag": "en"},
                "secondary": [{"id": "loc-fr", "cmsLocaleId": "cms-fr", "tag": "fr-FR"}]}})
        };
        let rec = |log: &Log, what: String, v: Value| log.lock().unwrap().push((what, v));
        let app = Router::new()
            .route("/sites", get(move || async move { Json(json!({"sites": [site()]})) }))
            .route("/sites/site1", get(move || async move { Json(site()) }))
            .route(
                "/sites/site1/collections",
                get(|| async { Json(json!({"collections": [{"id": "col1", "displayName": "Posts", "slug": "posts"}]})) }),
            )
            .route(
                "/collections/col1",
                get(|| async {
                    Json(json!({"id": "col1", "displayName": "Posts", "fields": [
                        {"id": "f1", "slug": "name", "displayName": "Name", "type": "PlainText", "isEditable": true, "isRequired": true},
                        {"id": "f2", "slug": "slug", "displayName": "Slug", "type": "PlainText", "isEditable": true, "isRequired": true},
                        {"id": "f3", "slug": "body", "displayName": "Body", "type": "RichText", "isEditable": true, "isRequired": false},
                        {"id": "f4", "slug": "cover", "displayName": "Cover", "type": "Image", "isEditable": true, "isRequired": false}]}))
                }),
            )
            .route(
                "/collections/col1/items",
                get(|Query(q): Query<BTreeMap<String, String>>| async move {
                    let offset: usize = q.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);
                    let all = [
                        json!({"id": "i1", "cmsLocaleId": "cms-en", "lastUpdated": "2026-01-01T00:00:00.000Z", "isArchived": false, "isDraft": false,
                            "fieldData": {"name": "Hello", "slug": "hello", "body": "<p>Read <a href=\"/t\">terms</a></p>"}}),
                        json!({"id": "i2", "cmsLocaleId": "cms-en", "lastUpdated": "2026-06-01T00:00:00.000Z", "isArchived": true, "isDraft": false,
                            "fieldData": {"name": "Old", "slug": "old"}}),
                    ];
                    // One item per page, to exercise pagination.
                    Json(json!({"items": all.get(offset).into_iter().collect::<Vec<_>>(), "pagination": {"limit": 1, "offset": offset, "total": 2}}))
                })
                .patch({
                    let log = log.clone();
                    move |Json(b): Json<Value>| async move {
                        rec(&log, "PATCH /collections/col1/items".into(), b.clone());
                        Json(json!({"items": b["items"], "pagination": {"limit": 100, "offset": 0, "total": 1}}))
                    }
                }),
            )
            .route(
                "/collections/col1/items/{id}",
                get(|Path(id): Path<String>, Query(q): Query<BTreeMap<String, String>>| async move {
                    // i1: name localized, body still the primary content; i3 does not exist in French.
                    let fr = q.get("cmsLocaleId").map(String::as_str) == Some("cms-fr");
                    let item = match (id.as_str(), fr) {
                        ("i1", true) => json!({"id": "i1", "cmsLocaleId": "cms-fr",
                            "fieldData": {"name": "Bonjour (édité)", "slug": "hello", "body": "<p>Read <a href=\"/t\">terms</a></p>"}}),
                        ("i1", false) | ("i3", false) => json!({"id": id, "cmsLocaleId": "cms-en",
                            "fieldData": {"name": "Hello", "slug": "hello", "body": "<p>Read <a href=\"/t\">terms</a></p>"}}),
                        _ => return (axum::http::StatusCode::NOT_FOUND, Json(json!({"code": "resource_not_found"}))),
                    };
                    (axum::http::StatusCode::OK, Json(item))
                }),
            )
            .route(
                "/token/introspect",
                get(|h: HeaderMap| async move {
                    // The token `wf-readonly` lacks every write scope.
                    let scope = if h["authorization"] == "Bearer wf-readonly" {
                        "sites:read,cms:read,pages:read,components:read"
                    } else {
                        "sites:read,cms:read,cms:write,pages:read,pages:write,components:read,components:write"
                    };
                    Json(json!({"authorization": {"id": "a1", "grantType": "client_credentials", "rateLimit": 60, "scope": scope,
                        "authorizedTo": {"siteIds": ["site1"], "workspaceIds": [], "userIds": []}}}))
                }),
            )
            .route(
                "/collections/col1/items/publish",
                post({
                    let log = log.clone();
                    move |Json(b): Json<Value>| async move {
                        rec(&log, "publish".into(), b);
                        (axum::http::StatusCode::ACCEPTED, Json(json!({"publishedItemIds": ["i1"], "errors": []})))
                    }
                }),
            )
            .route(
                "/sites/site1/pages",
                get(|Query(q): Query<BTreeMap<String, String>>| async move {
                    let offset: usize = q.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);
                    let pages = [json!({"id": "p1", "title": "Home", "slug": "", "collectionId": null, "draft": false, "archived": false, "lastUpdated": "2026-01-01T00:00:00Z"})];
                    Json(json!({"pages": pages.get(offset).into_iter().collect::<Vec<_>>(), "pagination": {"limit": 1, "offset": offset, "total": 1}}))
                }),
            )
            .route(
                "/pages/{id}/dom",
                get(|Path(id): Path<String>, Query(q): Query<BTreeMap<String, String>>| async move {
                    assert_eq!(id, "p1");
                    let offset: usize = q.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);
                    // The French DOM: n1 is localized, everything else still answers with the primary content.
                    let n1 = if q.get("localeId").map(String::as_str) == Some("loc-fr") {
                        "<h1>Bienvenue</h1>"
                    } else {
                        "<h1>Welcome <em>home</em></h1>"
                    };
                    let nodes = [
                        json!({"id": "n1", "type": "text", "text": {"html": n1, "text": "Welcome home"}}),
                        json!({"id": "n2", "type": "image", "image": {"alt": "x", "assetId": "a"}}),
                        json!({"id": "n3", "type": "component-instance", "componentId": "c1", "propertyOverrides": [
                            {"propertyId": "pr1", "type": "Plain Text", "label": "Title", "text": {"html": null, "text": "Buy now"}}]}),
                        json!({"id": "n4", "type": "text-input", "placeholder": "Your email"}),
                        json!({"id": "n5", "type": "submit-button", "value": "Send", "waitingText": "Sending…"}),
                    ];
                    // Two nodes per page.
                    let page: Vec<_> = nodes.iter().skip(offset).take(2).cloned().collect();
                    Json(json!({"pageId": "p1", "nodes": page, "pagination": {"limit": 2, "offset": offset, "total": nodes.len()}}))
                })
                .post({
                    let log = log.clone();
                    move |Path(id): Path<String>, Query(q): Query<BTreeMap<String, String>>, Json(b): Json<Value>| async move {
                        rec(&log, format!("POST /pages/{id}/dom?localeId={}", q["localeId"]), b);
                        Json(json!({"errors": []}))
                    }
                }),
            )
            .route(
                "/sites/site1/components",
                get(|Query(q): Query<BTreeMap<String, String>>| async move {
                    let offset: usize = q.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);
                    let comps = [
                        json!({"id": "c1", "name": "Footer", "readonly": false}),
                        json!({"id": "c2", "name": "Library", "readonly": true}),
                    ];
                    Json(json!({"components": comps.get(offset).into_iter().collect::<Vec<_>>(), "pagination": {"limit": 1, "offset": offset, "total": 2}}))
                }),
            )
            .route(
                "/sites/site1/components/{id}/dom",
                get(|Path(id): Path<String>| async move {
                    assert_eq!(id, "c1", "readonly components are not read");
                    Json(json!({"componentId": "c1", "nodes": [{"id": "cn1", "type": "text", "text": {"html": "<p>© Acme</p>", "text": "© Acme"}}],
                        "pagination": {"limit": 100, "offset": 0, "total": 1}}))
                })
                .post({
                    let log = log.clone();
                    move |Path(id): Path<String>, Query(q): Query<BTreeMap<String, String>>, Json(b): Json<Value>| async move {
                        rec(&log, format!("POST /components/{id}/dom?localeId={}", q["localeId"]), b);
                        Json(json!({"errors": ["node cn9 not found"]}))
                    }
                }),
            )
            ;
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (format!("http://{addr}"), log)
    }

    fn creds(base: &str) -> Creds {
        // SAFETY: tests only; every test sets the same value.
        unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
        Creds {
            secrets: json!({"token": "wf-token"}),
            settings: json!({"api_base": base, "publish": true}),
            since: None,
        }
    }

    async fn pull_all(c: &Creds) -> Vec<SourceItem> {
        let (mut out, mut cursor) = (vec![], None);
        for _ in 0..50 {
            let p = Webflow.pull(c, cursor).await.unwrap();
            out.extend(p.items);
            cursor = p.next;
            if cursor.is_none() {
                return out;
            }
        }
        panic!("pull does not end");
    }

    #[tokio::test]
    async fn test_locales_pull_push() {
        let (base, log) = mock().await;
        let c = creds(&base);
        let info = Webflow.test(&c).await.unwrap();
        assert_eq!(
            (info.name.as_str(), info.source_locale.as_deref()),
            ("Acme", Some("en"))
        );
        assert!(info.notes.is_empty());
        assert_eq!(Webflow.locales(&c).await.unwrap(), ["fr-FR"]);

        let items = pull_all(&c).await;
        let got: Vec<(&str, &str, &str, &str)> = items
            .iter()
            .map(|i| {
                (
                    i.external_id.as_str(),
                    i.field.as_str(),
                    i.key.as_str(),
                    i.format.name(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("col1/i1", "name", "webflow:item:i1:name", "plain"),
                ("col1/i1", "body", "webflow:item:i1:body", "html"),
                ("page/p1", "n1", "webflow:page:p1:n1", "html"),
                ("page/p1", "n3/pr1", "webflow:page:p1:n3/pr1", "plain"),
                (
                    "page/p1",
                    "n4/placeholder",
                    "webflow:page:p1:n4/placeholder",
                    "plain"
                ),
                ("page/p1", "n5/value", "webflow:page:p1:n5/value", "plain"),
                (
                    "page/p1",
                    "n5/waitingText",
                    "webflow:page:p1:n5/waitingText",
                    "plain"
                ),
                ("component/c1", "cn1", "webflow:component:c1:cn1", "html"),
            ],
            "slug, image fields, archived items, image nodes and readonly components are skipped"
        );
        assert_eq!(items[0].context, "Webflow CMS Posts “Hello”, field name");
        // Text node HTML goes through segments and comes back with its tags intact.
        let n1 = &items[2];
        let u = super::super::rich::units(&n1.key, &n1.format, &n1.text, "en");
        assert_eq!(
            u.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
            ["Welcome <0>home</0>"]
        );
        let tr = |k: &str| (k == u[0].0).then(|| "Bienvenue <0>chez vous</0>".to_string());
        assert_eq!(
            super::super::rich::rebuild(&n1.key, &n1.format, &n1.text, "en", &tr).unwrap(),
            "<h1>Bienvenue <em>chez vous</em></h1>"
        );

        let push = |ext: &str, field: &str, value: &str| PushItem {
            external_id: ext.into(),
            field: field.into(),
            value: value.into(),
            format: Format::Plain,
            meta: Value::Null,
        };
        let r = Webflow
            .push(
                &c,
                "fr",
                &[
                    push("col1/i1", "name", "Bonjour"),
                    push(
                        "col1/i1",
                        "body",
                        "<p>Lisez nos <a href=\"/t\">conditions</a></p>",
                    ),
                    push("page/p1", "n1", "<h1>Bienvenue <em>chez vous</em></h1>"),
                    push("page/p1", "n3/pr1", "Acheter"),
                    push("page/p1", "n5/value", "Envoyer"),
                    push("component/c1", "cn9", "x"),
                ],
            )
            .await
            .unwrap();
        assert_eq!(r.ok, 5);
        assert_eq!(
            r.failed,
            [(
                "component/c1".to_string(),
                "\"node cn9 not found\"".to_string()
            )]
        );
        let log = log.lock().unwrap().clone();
        let find = |what: &str| {
            log.iter()
                .find(|(w, _)| w == what)
                .map(|x| x.1.clone())
                .unwrap_or_else(|| panic!("no {what}: {log:?}"))
        };
        assert_eq!(
            find("POST /pages/p1/dom?localeId=loc-fr"),
            json!({"nodes": [
                {"nodeId": "n1", "text": "<h1>Bienvenue <em>chez vous</em></h1>"},
                {"nodeId": "n3", "propertyOverrides": [{"propertyId": "pr1", "text": "Acheter"}]},
                {"nodeId": "n5", "value": "Envoyer"}]})
        );
        assert_eq!(
            find("PATCH /collections/col1/items"),
            json!({"items": [{"id": "i1", "cmsLocaleId": "cms-fr", "fieldData": {
                "name": "Bonjour", "body": "<p>Lisez nos <a href=\"/t\">conditions</a></p>"}}]})
        );
        assert_eq!(
            find("publish"),
            json!({"items": [{"id": "i1", "cmsLocaleIds": ["cms-fr"]}]})
        );

        // Unknown locale: a clear error, nothing written.
        assert!(
            Webflow
                .push(&c, "de", &[push("col1/i1", "name", "Hallo")])
                .await
                .unwrap_err()
                .to_string()
                .contains("'de'")
        );

        // Incremental pull: items and pages untouched since are skipped.
        let mut inc = c.clone();
        inc.since = Some("2026-03-01T00:00:00Z".parse().unwrap());
        let items = pull_all(&inc).await;
        assert!(
            items.iter().all(|i| i.external_id == "component/c1"),
            "{items:?}"
        );
    }

    #[tokio::test]
    async fn current_and_doctor() {
        let (base, log) = mock().await;
        let c = creds(&base);
        let item = |ext: &str, field: &str| PushItem {
            external_id: ext.into(),
            field: field.into(),
            value: "x".into(),
            format: Format::Plain,
            meta: Value::Null,
        };
        let cur = Webflow
            .current(
                &c,
                "fr",
                &[
                    item("col1/i1", "name"),
                    item("col1/i1", "body"),
                    item("col1/i3", "name"),
                    item("page/p1", "n1"),
                    item("page/p1", "n5/value"),
                    item("component/c1", "cn1"),
                ],
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            cur,
            [
                Some("Bonjour (édité)".to_string()),
                None, // same as the primary locale: inherited, not translated
                None, // no French item
                Some("<h1>Bienvenue</h1>".to_string()),
                None,
                None,
            ]
        );
        assert!(log.lock().unwrap().is_empty(), "current writes nothing");
        assert!(
            Webflow
                .current(&c, "de", &[item("col1/i1", "name")])
                .await
                .is_err()
        );

        let checks = Webflow.doctor(&c).await;
        assert!(checks.iter().all(|k| k.status == "ok"), "{checks:?}");
        assert_eq!(checks[1].detail, "fr-FR");
        let mut ro = c.clone();
        ro.secrets = json!({"token": "wf-readonly"});
        let checks = Webflow.doctor(&ro).await;
        assert_eq!(
            (checks[0].id.as_str(), checks[0].status, checks[0].fix),
            ("scopes", "fail", Some("permissions"))
        );
        assert_eq!(
            checks[0].detail,
            "missing scope: cms:write, pages:write, components:write"
        );
        assert!(log.lock().unwrap().is_empty(), "doctor writes nothing");
    }

    #[test]
    fn webhook_signature() {
        use hmac::{Hmac, Mac};
        let body = br#"{"triggerType":"collection_item_changed","payload":{"id":"i1"}}"#;
        let sign = |secret: &str, ts: &str| {
            let mut m = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
            m.update(format!("{ts}:").as_bytes());
            m.update(body);
            hex::encode(m.finalize().into_bytes())
        };
        let headers = |ts: &str, sig: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-webflow-timestamp", ts.parse().unwrap());
            h.insert("x-webflow-signature", sig.parse().unwrap());
            h
        };
        let now = Utc::now().timestamp_millis().to_string();
        assert!(Webflow.verify_webhook(&headers(&now, &sign("s", &now)), body, "s"));
        assert!(!Webflow.verify_webhook(&headers(&now, &sign("other", &now)), body, "s"));
        assert!(!Webflow.verify_webhook(&headers(&now, &sign("s", &now)), b"{}", "s"));
        let old = (Utc::now().timestamp_millis() - 600_000).to_string();
        assert!(
            !Webflow.verify_webhook(&headers(&old, &sign("s", &old)), body, "s"),
            "replay older than 5 min"
        );
        assert!(!Webflow.verify_webhook(&HeaderMap::new(), body, "s"));
    }

    #[test]
    fn locale_matching() {
        let site = json!({"locales": {"secondary": [{"id": "a", "cmsLocaleId": "b", "tag": "pt-BR"}, {"id": "c", "cmsLocaleId": "d", "tag": "fr"}]}});
        assert_eq!(
            locale_ids(&site, "pt-br").unwrap(),
            ("a".into(), "b".into())
        );
        assert_eq!(
            locale_ids(&site, "fr-CA").unwrap(),
            ("c".into(), "d".into())
        );
        assert!(locale_ids(&site, "de").is_err());
    }
}
