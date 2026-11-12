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
//! - Read back (`current`): the variation's draft (`GET …/{variation id}/draft`), compared with the source
//!   it was cloned from: a value identical to the source's is the clone, not a translation. `remove` is not
//!   implemented: a translation is a whole page / post variation, never deleted by glot.
//! - Diagnosis (`doctor`): the token's scopes (`POST /oauth/v2/private-apps/get/access-token-info`, else
//!   `GET /oauth/v1/access-tokens/{token}` for OAuth tokens) must include `content`; a source language must
//!   be set for HubSpot to create variations.
//!
//! Docs: https://developers.hubspot.com/docs/guides/apps/private-apps/overview (access token info),
//! https://developers.hubspot.com/docs/api-reference/legacy/authentication/oauth-tokens/v1/guide,
//! https://developers.hubspot.com/docs/api-reference/cms-posts-v3/guide,
//! https://developers.hubspot.com/docs/api-reference/cms-pages-v3/guide,
//! https://developers.hubspot.com/docs/guides/apps/authentication/validating-requests
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, param, required};
use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key,
    send,
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

fn api_path(res: &str) -> anyhow::Result<&'static str> {
    RESOURCES
        .iter()
        .find(|r| r.0 == res)
        .map(|r| r.1)
        .ok_or_else(|| anyhow::anyhow!("unknown resource {res}"))
}

/// Id of the `locale` variation of a source object (its `translations` map), if it exists.
fn variation(src: &Value, id: &str, locale: &str) -> Option<String> {
    let langs: Vec<String> = src["translations"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    cms::match_locale(&langs, locale)
        .and_then(|l| cms::id_str(&src["translations"][l]["id"]))
        .filter(|t| t != id)
}

/// A field of a page / post: top-level name, or JSON pointer into its modules.
fn field<'a>(o: &'a Value, name: &str) -> Option<&'a str> {
    if name.starts_with('/') {
        o.pointer(name)
    } else {
        o.get(name)
    }
    .and_then(Value::as_str)
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
                let path = api_path(res)?;
                // Fresh translations map: a variation created by an earlier batch is not in the pulled data.
                let src = api.get(&format!("{path}/{id}")).await?;
                let target = match variation(&src, id, locale) {
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

    async fn current(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<Option<Vec<Option<String>>>> {
        let api = Api::new(creds).await?;
        // (source, draft of its `locale` variation) per source object.
        let mut seen: BTreeMap<&str, (Value, Option<Value>)> = BTreeMap::new();
        for it in items {
            if seen.contains_key(it.external_id.as_str()) {
                continue;
            }
            let (Some(res), Some(id)) = (it.meta["resource"].as_str(), it.meta["id"].as_str())
            else {
                anyhow::bail!("missing metadata, sync again");
            };
            let path = api_path(res)?;
            let src = api.get(&format!("{path}/{id}")).await?;
            let draft = match variation(&src, id, locale) {
                Some(t) => Some(api.get(&format!("{path}/{t}/draft")).await?),
                None => None,
            };
            seen.insert(&it.external_id, (src, draft));
        }
        Ok(Some(
            items
                .iter()
                .map(|it| {
                    let (src, draft) = &seen[it.external_id.as_str()];
                    let v = field(draft.as_ref()?, &it.field)?;
                    // Still the content cloned from the source: not translated.
                    (!v.trim().is_empty() && field(src, &it.field) != Some(v))
                        .then(|| v.to_string())
                })
                .collect(),
        ))
    }

    async fn doctor(&self, creds: &Creds) -> Vec<Check> {
        let api = match Api::new(creds).await {
            Ok(a) => a,
            Err(e) => return vec![Check::fail("scopes", format!("{e:#}"))],
        };
        // Private app token, else OAuth access token.
        let info = match api
            .call(
                reqwest::Method::POST,
                "/oauth/v2/private-apps/get/access-token-info",
                Some(&json!({"tokenKey": api.token})),
            )
            .await
        {
            Ok(i) => Ok(i),
            Err(_) => {
                api.get(&format!("/oauth/v1/access-tokens/{}", cms::pct(&api.token)))
                    .await
            }
        };
        let mut out = vec![match info {
            Ok(i) => {
                let has = i["scopes"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|x| x == "content"));
                if has {
                    Check::ok("scopes", "content")
                } else {
                    Check {
                        fix: Some("permissions"),
                        ..Check::fail(
                            "scopes",
                            "missing scope: content (CMS pages and blog posts)",
                        )
                    }
                }
            }
            Err(e) => Check::warn(
                "scopes",
                format!("token scopes could not be read: {e:#}"),
                None,
            ),
        }];
        out.push(match seen_languages(&api, creds).await {
            Ok((_, None)) if cms::param(creds, "source_lang").is_none() => Check::warn(
                "multi_language",
                "no source page or post has a language set: HubSpot cannot create language variations",
                Some("enable_locale"),
            ),
            Ok((langs, _)) => Check::ok("multi_language", langs.join(", ")),
            Err(e) => Check::fail("multi_language", format!("{e:#}")),
        });
        out
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
