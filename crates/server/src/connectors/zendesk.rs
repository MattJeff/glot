//! Zendesk Guide (Help Center API): categories, sections and articles, translated per Help Center locale.
//!
//! Credentials: `subdomain` (`acme`, or the full `https://acme.zendesk.com`), `email` and `token` (Admin Center
//! → Apps and integrations → Zendesk API → API token; HTTP Basic `{email}/token:{token}`), optional
//! `webhook_secret` (signing secret of a Zendesk webhook). Settings: `publish` (new article translations are
//! created published; default: draft).
//!
//! - Pull: `GET /api/v2/help_center/{default locale}/{categories|sections|articles}?page[size]=100`, cursor
//!   pagination (`meta.has_more`, `meta.after_cursor`). Articles: `title` (plain) and `body` (HTML, tags kept);
//!   sections and categories: `name`, `description`. Articles written in another locale are skipped.
//! - Push: `PUT /api/v2/help_center/{resource}/{id}/translations/{locale}` (update), or on `404`
//!   `POST /api/v2/help_center/{resource}/{id}/translations` (create, with `title` required by Zendesk).
//! - Webhooks: `X-Zendesk-Webhook-Signature` = base64 HMAC-SHA256 of `timestamp + body` with the webhook's
//!   signing secret (`X-Zendesk-Webhook-Signature-Timestamp`), or the card URL with its token.
//! - Read back (`current`): `GET /api/v2/help_center/{resource}/{id}/translations` (a translation only exists
//!   once written: no fallback to the source locale).
//! - Removed (`remove`): `DELETE /api/v2/help_center/translations/{translation id}`, only when glot wrote the
//!   whole translation (its title is rolled back and no other field holds text); else unsupported.
//! - Diagnosis (`doctor`): the user's role (`GET /api/v2/users/me`: an admin, or an agent with Guide
//!   permissions on the articles) and the Help Center having more than its default locale.
//!
//! Docs: https://developer.zendesk.com/api-reference/help_center/help-center-api/articles/,
//! https://developer.zendesk.com/api-reference/ticketing/users/users/,
//! https://developer.zendesk.com/api-reference/help_center/help-center-api/translations/,
//! https://developer.zendesk.com/api-reference/introduction/pagination/,
//! https://developer.zendesk.com/documentation/webhooks/verifying/
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, required};
use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key,
    send,
};

pub struct Zendesk;

/// (resource, fields: (Zendesk field, translation field, html)).
const RESOURCES: &[(&str, &[(&str, &str, bool)])] = &[
    (
        "categories",
        &[("name", "title", false), ("description", "body", false)],
    ),
    (
        "sections",
        &[("name", "title", false), ("description", "body", false)],
    ),
    (
        "articles",
        &[("title", "title", false), ("body", "body", true)],
    ),
];

struct Api {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let sub = required(c, "subdomain")?.trim_end_matches('/');
        let base = if sub.contains("://") {
            sub.to_string()
        } else {
            anyhow::ensure!(
                sub.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "invalid Zendesk subdomain"
            );
            format!("https://{sub}.zendesk.com")
        };
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!(
                "{}/token:{}",
                required(c, "email")?,
                required(c, "token")?
            ))
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
        let url = format!("{}/api/v2{path}", self.base);
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
    /// (default locale, every Help Center locale).
    async fn locales(&self) -> anyhow::Result<(String, Vec<String>)> {
        let l = self.get("/help_center/locales.json").await?;
        let all: Vec<String> = l["locales"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        let def = l["default_locale"]
            .as_str()
            .map(String::from)
            .or_else(|| all.first().cloned())
            .ok_or_else(|| anyhow::anyhow!("the Help Center has no locale"))?;
        Ok((def, all))
    }
}

/// The Help Center locale matching a glot locale.
fn help_locale(all: &[String], locale: &str) -> anyhow::Result<String> {
    Ok(cms::match_locale(all, locale)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "locale {locale} is not enabled in the Help Center (Guide admin → Settings → Language settings)"
            )
        })?
        .to_string())
}

/// (resource, id, fields) of an item's meta.
fn target(
    meta: &Value,
) -> anyhow::Result<(&str, i64, &'static [(&'static str, &'static str, bool)])> {
    let (Some(r), Some(id)) = (meta["resource"].as_str(), meta["id"].as_i64()) else {
        anyhow::bail!("missing metadata, sync again");
    };
    let fields = RESOURCES
        .iter()
        .find(|(n, _)| *n == r)
        .map(|x| x.1)
        .ok_or_else(|| anyhow::anyhow!("unknown resource {r}"))?;
    Ok((r, id, fields))
}

/// The `loc` translation of each resource of `items` (`Null` = none), keyed by external id.
async fn translations<'a>(
    api: &Api,
    loc: &str,
    items: &'a [PushItem],
) -> anyhow::Result<BTreeMap<&'a str, Value>> {
    let mut out = BTreeMap::new();
    for it in items {
        if out.contains_key(it.external_id.as_str()) {
            continue;
        }
        let (r, id, _) = target(&it.meta)?;
        let list = api
            .get(&format!("/help_center/{r}/{id}/translations.json"))
            .await?;
        let t = list["translations"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| {
                t["locale"]
                    .as_str()
                    .is_some_and(|l| l.eq_ignore_ascii_case(loc))
            })
            .cloned()
            .unwrap_or(Value::Null);
        out.insert(it.external_id.as_str(), t);
    }
    Ok(out)
}

/// The translation field (`title` / `body`) an item is written to.
fn translation_field(it: &PushItem) -> Option<&'static str> {
    let (_, _, fields) = target(&it.meta).ok()?;
    fields.iter().find(|(f, _, _)| *f == it.field).map(|x| x.1)
}

fn items_of(res: &str, fields: &[(&str, &str, bool)], o: &Value, def: &str) -> Vec<SourceItem> {
    let Some(id) = o["id"].as_i64() else {
        return vec![];
    };
    if o["source_locale"]
        .as_str()
        .is_some_and(|s| !s.eq_ignore_ascii_case(def))
    {
        return vec![];
    }
    let title = o["title"].as_str().or(o["name"].as_str()).unwrap_or("");
    let what = res.trim_end_matches('s').replace("ie", "y");
    fields
        .iter()
        .filter_map(|(f, _, html)| {
            let v = o[*f].as_str()?;
            (!v.trim().is_empty()).then(|| SourceItem {
                external_id: format!("{res}:{id}"),
                field: f.to_string(),
                key: key("zendesk", res, &id.to_string(), f),
                text: v.to_string(),
                format: if *html { Format::Html } else { Format::Plain },
                context: format!("Zendesk Help Center {what} \u{ab}{title}\u{bb}, field {f}"),
                updated_at: None,
                meta: json!({"resource": res, "id": id}),
            })
        })
        .collect()
}

#[async_trait]
impl Connector for Zendesk {
    fn kind(&self) -> &'static str {
        "zendesk"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let me = api.get("/users/me.json").await?;
        anyhow::ensure!(
            me["user"]["id"].is_i64(),
            "Zendesk did not accept the email / API token"
        );
        let (def, all) = api.locales().await?;
        Ok(AccountInfo {
            name: api.base.clone(),
            source_locale: Some(def),
            notes: vec![format!("Help Center locales: {}", all.join(", "))],
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(Api::new(creds).await?.locales().await?.1)
    }

    /// Cursor = `{resource index}|{after_cursor}` (empty = first page).
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let (def, _) = api.locales().await?;
        let (i, after) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, a)) => (i.parse::<usize>()?, a.to_string()),
            None => (0, String::new()),
        };
        let Some((res, fields)) = RESOURCES.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let mut q = format!("/help_center/{def}/{res}.json?page[size]=100");
        if !after.is_empty() {
            q.push_str(&format!("&page[after]={after}"));
        }
        let page = api.get(&q).await?;
        let items = page[res]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|o| items_of(res, fields, o, &def))
            .collect();
        let next = match page["meta"]["after_cursor"].as_str() {
            Some(c) if page["meta"]["has_more"] == true => Some(format!("{i}|{c}")),
            _ if i + 1 < RESOURCES.len() => Some(format!("{}|", i + 1)),
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
        let (_, all) = api.locales().await?;
        let loc = help_locale(&all, locale)?;
        let draft = creds.setting("publish") != Some(&Value::Bool(true));
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let meta = &its[0].meta;
            let res: anyhow::Result<()> = async {
                let (r, id, fields) = target(meta)?;
                let mut t = Map::new();
                for it in &its {
                    if let Some((_, tf, _)) = fields.iter().find(|(f, _, _)| *f == it.field) {
                        t.insert(tf.to_string(), Value::String(it.value.clone()));
                    }
                }
                let path = format!("/help_center/{r}/{id}/translations");
                match api
                    .call(
                        reqwest::Method::PUT,
                        &format!("{path}/{loc}.json"),
                        Some(&json!({"translation": t})),
                    )
                    .await
                {
                    Err(e) if e.to_string().starts_with("HTTP 404") => {
                        anyhow::ensure!(
                            t.contains_key("title"),
                            "no {loc} translation yet and its title is not translated yet"
                        );
                        t.insert("locale".into(), Value::String(loc.clone()));
                        if r == "articles" {
                            t.insert("draft".into(), Value::Bool(draft));
                        }
                        api.call(
                            reqwest::Method::POST,
                            &format!("{path}.json"),
                            Some(&json!({"translation": t})),
                        )
                        .await?;
                        Ok(())
                    }
                    r => r.map(|_| ()),
                }
            }
            .await;
            match res {
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
        let loc = help_locale(&api.locales().await?.1, locale)?;
        let found = translations(&api, &loc, items).await?;
        Ok(Some(
            items
                .iter()
                .map(|it| {
                    let v = found[it.external_id.as_str()][translation_field(it)?].as_str()?;
                    (!v.trim().is_empty()).then(|| v.to_string())
                })
                .collect(),
        ))
    }

    async fn remove(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<bool> {
        let api = Api::new(creds).await?;
        let (def, all) = api.locales().await?;
        let loc = help_locale(&all, locale)?;
        anyhow::ensure!(
            !loc.eq_ignore_ascii_case(&def),
            "{loc} is the default locale"
        );
        let found = translations(&api, &loc, items).await?;
        // Zendesk deletes a translation as a whole: only when everything it holds is being rolled back.
        let mut ids = vec![];
        for (ext, t) in &found {
            let Some(id) = t["id"].as_i64() else { continue };
            let rolled: Vec<&str> = items
                .iter()
                .filter(|i| i.external_id == *ext)
                .filter_map(translation_field)
                .collect();
            let kept_body = !rolled.contains(&"body")
                && t["body"].as_str().is_some_and(|b| !b.trim().is_empty());
            if !rolled.contains(&"title") || kept_body {
                return Ok(false);
            }
            ids.push(id);
        }
        for id in ids {
            api.call(
                reqwest::Method::DELETE,
                &format!("/help_center/translations/{id}.json"),
                None,
            )
            .await?;
        }
        Ok(true)
    }

    async fn doctor(&self, creds: &Creds) -> Vec<Check> {
        let api = match Api::new(creds).await {
            Ok(a) => a,
            Err(e) => return vec![Check::fail("role", format!("{e:#}"))],
        };
        let mut out = vec![match api.get("/users/me.json").await {
            Ok(me) => match me["user"]["role"].as_str().unwrap_or("") {
                "admin" => Check::ok("role", "admin"),
                "agent" => Check::warn(
                    "role",
                    "agent: writing translations needs Guide admin, or management permissions on the articles",
                    Some("permissions"),
                ),
                r => Check {
                    fix: Some("permissions"),
                    ..Check::fail(
                        "role",
                        format!(
                            "role '{r}' cannot edit the Help Center: use an admin or a Guide agent"
                        ),
                    )
                },
            },
            Err(e) => Check::fail("role", format!("{e:#}")),
        }];
        out.push(match api.locales().await {
            Ok((def, all)) if all.len() < 2 => Check::warn(
                "help_center_locales",
                format!("only {def} is enabled in the Help Center: add languages (Guide admin → Settings → Language settings)"),
                Some("enable_locale"),
            ),
            Ok((_, all)) => Check::ok("help_center_locales", all.join(", ")),
            Err(e) => Check::fail("help_center_locales", format!("{e:#}")),
        });
        out
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        if cms::shared_secret_ok(headers, secret) {
            return true;
        }
        let (Some(sig), Some(ts)) = (
            cms::header(headers, "x-zendesk-webhook-signature"),
            cms::header(headers, "x-zendesk-webhook-signature-timestamp"),
        ) else {
            return false;
        };
        let mut signed = ts.as_bytes().to_vec();
        signed.extend_from_slice(body);
        !secret.is_empty() && super::hmac_ok(secret, &signed, sig)
    }

    fn push_batch(&self) -> usize {
        // Zendesk allows 200-700 requests per minute depending on the plan; one or two calls per resource.
        40
    }
}
