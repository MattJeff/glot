//! HubSpot CMS (CMS API v3): blog posts, website pages and landing pages, translated as HubSpot
//! multi-language variations.
//!
//! Credentials: `token` (private app access token, scope `content`), optional `webhook_secret` (the app's
//! client secret, for `X-HubSpot-Signature` v1), `api_url` (tests). Settings: `resources` (`["blog_posts",
//! "site_pages", "landing_pages"]`), `languages` (target languages when none exists yet), `source_lang`,
//! `publish` (push the draft live after each write; default: drafts only).
//!
//! - Pull: `GET /cms/v3/{blogs/posts | pages/site-pages | pages/landing-pages}?limit=100&after=…`
//!   (`paging.next.after`), `updatedAfter` for incremental syncs. Variations (`translatedFromId` set) are
//!   never pulled as sources. Blog posts: `name` (title), `htmlTitle`, `metaDescription`, `postSummary` and
//!   `postBody` (HTML). Pages: `htmlTitle`, `metaDescription` and every `html` string of `widgets` and
//!   `layoutSections` (rich text modules of drag-and-drop areas), addressed by JSON pointer, structure kept.
//! - Push: the variation for the language is found in the source's `translations` map, else created with
//!   `POST …/multi-language/create-language-variation` (`{"id", "language"}`). Values are written to its
//!   draft (`PATCH …/{id}/draft`), and with `publish` pushed live (`POST …/{id}/draft/push-live`).
//! - Webhooks: no CMS content webhooks for private apps; a workflow "Send a webhook" action or any caller
//!   uses the card URL with its token, a `X-Glot-Webhook-Secret` header, or `X-HubSpot-Signature` v1.
//!
//! Docs: https://developers.hubspot.com/docs/api-reference/cms-posts-v3/guide,
//! https://developers.hubspot.com/docs/api-reference/cms-pages-v3/guide,
//! https://developers.hubspot.com/docs/guides/apps/authentication/validating-requests
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, param, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct HubSpot;

/// (resource, API path, top-level fields: (name, html)).
const RESOURCES: &[(&str, &str, &[(&str, bool)])] = &[
    (
        "blog_posts",
        "/cms/v3/blogs/posts",
        &[
            ("name", false),
            ("htmlTitle", false),
            ("metaDescription", false),
            ("postSummary", true),
            ("postBody", true),
        ],
    ),
    (
        "site_pages",
        "/cms/v3/pages/site-pages",
        &[("htmlTitle", false), ("metaDescription", false)],
    ),
    (
        "landing_pages",
        "/cms/v3/pages/landing-pages",
        &[("htmlTitle", false), ("metaDescription", false)],
    ),
];
/// Page content containers walked for `html` strings.
const CONTAINERS: &[&str] = &["widgets", "layoutSections"];

struct Api {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let base = param(c, "api_url")
            .unwrap_or("https://api.hubapi.com")
            .trim_end_matches('/')
            .to_string();
        Ok(Api {
            http: super::client_for(&base).await?,
            token: required(c, "token")?.to_string(),
            base,
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
}

fn resources(c: &Creds) -> Vec<(&'static str, &'static str, &'static [(&'static str, bool)])> {
    let only: Option<Vec<&str>> = c
        .setting("resources")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect());
    RESOURCES
        .iter()
        .filter(|r| only.as_ref().is_none_or(|o| o.contains(&r.0)))
        .copied()
        .collect()
}

fn is_variation(o: &Value) -> bool {
    match &o["translatedFromId"] {
        Value::Null => false,
        Value::String(s) => !s.is_empty() && s != "0",
        Value::Number(n) => n.as_i64() != Some(0),
        _ => true,
    }
}

fn escape(k: &str) -> String {
    k.replace('~', "~0").replace('/', "~1")
}

/// (JSON pointer, html) of every non-empty `html` string under `v`.
fn html_strings(v: &Value, ptr: &str, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(o) => {
            for (k, c) in o {
                let p = format!("{ptr}/{}", escape(k));
                match c {
                    Value::String(s) if k == "html" && cms::has_letter(s) => {
                        out.push((p, s.clone()))
                    }
                    _ => html_strings(c, &p, out),
                }
            }
        }
        Value::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, c)| html_strings(c, &format!("{ptr}/{i}"), out)),
        _ => {}
    }
}

fn items_of(res: &str, fields: &[(&str, bool)], o: &Value) -> Vec<SourceItem> {
    let Some(id) = cms::id_str(&o["id"]) else {
        return vec![];
    };
    let title = o["name"].as_str().unwrap_or("");
    let what = res.trim_end_matches('s').replace('_', " ");
    let item = |field: String, text: String, html: bool| SourceItem {
        external_id: format!("{res}:{id}"),
        key: key("hubspot", res, &id, &field),
        context: format!("HubSpot {what} \u{ab}{title}\u{bb}, field {field}"),
        field,
        text,
        format: if html { Format::Html } else { Format::Plain },
        updated_at: None,
        meta: json!({"resource": res, "id": id}),
    };
    let mut out: Vec<SourceItem> = fields
        .iter()
        .filter_map(|(f, html)| {
            let v = o[*f].as_str()?;
            (!v.trim().is_empty()).then(|| item(f.to_string(), v.to_string(), *html))
        })
        .collect();
    if res != "blog_posts" {
        for c in CONTAINERS {
            let mut found = vec![];
            html_strings(&o[*c], &format!("/{c}"), &mut found);
            out.extend(found.into_iter().map(|(p, s)| item(p, s, true)));
        }
    }
    out
}

/// Languages seen in the account (source and variations), for `locales` when none is configured.
async fn seen_languages(api: &Api, c: &Creds) -> anyhow::Result<(Vec<String>, Option<String>)> {
    let mut langs = vec![];
    let mut source = None;
    for (_, path, _) in resources(c) {
        let page = api.get(&format!("{path}?limit=100&archived=false")).await?;
        for o in page["results"].as_array().into_iter().flatten() {
            let l = o["language"].as_str().unwrap_or("").to_string();
            if !is_variation(o) && !l.is_empty() && source.is_none() {
                source = Some(l.clone());
            }
            langs.push(l);
            langs.extend(
                o["translations"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.keys().cloned()),
            );
        }
    }
    langs.retain(|l| !l.is_empty());
    langs.sort();
    langs.dedup();
    Ok((langs, source))
}

fn configured(c: &Creds) -> Vec<String> {
    c.setting("languages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Connector for HubSpot {
    fn kind(&self) -> &'static str {
        "hubspot"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let (langs, source) = seen_languages(&api, creds).await?;
        let src = cms::param(creds, "source_lang")
            .map(String::from)
            .or(source);
        let mut notes = vec![format!("Languages in use: {}", langs.join(", "))];
        if src.is_none() {
            notes.push("Set a language on your source pages and posts (HubSpot needs it to create language variations).".into());
        }
        Ok(AccountInfo {
            name: "HubSpot CMS".into(),
            source_locale: src,
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let mut l = configured(creds);
        if l.is_empty() {
            l = seen_languages(&Api::new(creds).await?, creds).await?.0;
        }
        Ok(l)
    }

    /// Cursor = `{resource index}|{after}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let res = resources(creds);
        let (i, after) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, a)) => (i.parse::<usize>()?, a.to_string()),
            None => (0, String::new()),
        };
        let Some((name, path, fields)) = res.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let mut q = format!("{path}?limit=100&archived=false&sort=id");
        if !after.is_empty() {
            q.push_str(&format!("&after={after}"));
        }
        if let Some(s) = creds.since {
            q.push_str(&format!("&updatedAfter={}", s.format("%Y-%m-%dT%H:%M:%SZ")));
        }
        let page = api.get(&q).await?;
        let items = page["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| !is_variation(o) && o["archivedInDashboard"] != true)
            .flat_map(|o| items_of(name, fields, o))
            .collect();
        let next = match page["paging"]["next"]["after"].as_str() {
            Some(a) if !a.is_empty() => Some(format!("{i}|{a}")),
            _ if i + 1 < res.len() => Some(format!("{}|", i + 1)),
            _ => None,
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
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let meta = &its[0].meta;
            let r: anyhow::Result<()> = async {
                let (Some(res), Some(id)) = (meta["resource"].as_str(), meta["id"].as_str()) else {
                    anyhow::bail!("missing metadata, sync again");
                };
                let path = RESOURCES
                    .iter()
                    .find(|r| r.0 == res)
                    .map(|r| r.1)
                    .ok_or_else(|| anyhow::anyhow!("unknown resource {res}"))?;
                // Fresh translations map: a variation created by an earlier batch is not in the pulled data.
                let src = api.get(&format!("{path}/{id}")).await?;
                let langs: Vec<String> = src["translations"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                let target = match cms::match_locale(&langs, locale)
                    .and_then(|l| cms::id_str(&src["translations"][l]["id"]))
                    .filter(|t| t != id)
                {
                    Some(t) => t,
                    None => {
                        let v = api
                            .call(
                                reqwest::Method::POST,
                                &format!("{path}/multi-language/create-language-variation"),
                                Some(&json!({"id": id, "language": locale.to_ascii_lowercase()})),
                            )
                            .await?;
                        cms::id_str(&v["id"]).ok_or_else(|| {
                            anyhow::anyhow!("HubSpot created no {locale} variation")
                        })?
                    }
                };
                let mut draft = api.get(&format!("{path}/{target}/draft")).await?;
                let mut body = Map::new();
                for it in &its {
                    if it.field.starts_with('/') {
                        let slot = draft.pointer_mut(&it.field).ok_or_else(|| {
                            anyhow::anyhow!(
                                "{} not found in the {locale} variation (its layout differs from the source)",
                                it.field
                            )
                        })?;
                        *slot = Value::String(it.value.clone());
                    } else {
                        body.insert(it.field.clone(), Value::String(it.value.clone()));
                    }
                }
                for c in CONTAINERS {
                    if its.iter().any(|it| it.field.starts_with(&format!("/{c}/"))) {
                        body.insert(c.to_string(), draft[*c].take());
                    }
                }
                api.call(
                    reqwest::Method::PATCH,
                    &format!("{path}/{target}/draft"),
                    Some(&Value::Object(body)),
                )
                .await?;
                if publish {
                    api.call(
                        reqwest::Method::POST,
                        &format!("{path}/{target}/draft/push-live"),
                        None,
                    )
                    .await?;
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

    /// Shared secret (header or token in the URL), or `X-HubSpot-Signature` v1: hex SHA-256 of
    /// `client secret + body`.
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        use sha2::Digest;
        if cms::shared_secret_ok(headers, secret) {
            return true;
        }
        let Some(sig) = cms::header(headers, "x-hubspot-signature") else {
            return false;
        };
        let mut h = sha2::Sha256::new();
        h.update(secret.as_bytes());
        h.update(body);
        !secret.is_empty()
            && super::secret_eq(&sig.to_ascii_lowercase(), &hex::encode(h.finalize()))
    }

    fn push_batch(&self) -> usize {
        // Private apps: 100-190 requests per 10 s; 3 to 5 calls per resource.
        20
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_modules_by_pointer() {
        let o = json!({"id": "7", "name": "Home", "htmlTitle": "Welcome", "translatedFromId": null,
            "widgets": {"hero/main": {"body": {"html": "<h1>Hi <b>there</b></h1>", "css_class": "x"}}},
            "layoutSections": {"dnd_area": {"rows": [{"0": {"params": {"html": "<p>Body</p>", "img": "a.png"}}}]}}});
        let items = items_of("site_pages", RESOURCES[1].2, &o);
        let f: Vec<_> = items.iter().map(|i| i.field.as_str()).collect();
        assert_eq!(
            f,
            [
                "htmlTitle",
                "/widgets/hero~1main/body/html",
                "/layoutSections/dnd_area/rows/0/0/params/html"
            ]
        );
        assert_eq!(
            o.pointer(&items[1].field).unwrap(),
            "<h1>Hi <b>there</b></h1>"
        );
        assert!(!is_variation(&o) && is_variation(&json!({"translatedFromId": "12"})));
    }
}
