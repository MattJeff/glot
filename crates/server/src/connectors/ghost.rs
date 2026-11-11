//! Ghost (Admin API, Ghost 5+). Ghost has no translations of its own: glot uses the pattern of Ghost's
//! routing docs, one post per language marked with an internal tag (`#fr`) and served by a `routes.yaml`
//! collection filtered on it (`filter: 'tag:hash-fr'`, https://docs.ghost.org/themes/routing).
//!
//! Credentials: `url` (the site), `admin_key` (custom integration's Admin API key `{id}:{secret}`), optional
//! `webhook_secret` (any string you choose: glot then registers signed Ghost webhooks). Settings:
//! `source_tag` (only posts carrying this tag are sources, e.g. `#en`), `publish` (create translations
//! published; default draft), `pages` (`false` to skip pages).
//!
//! - Auth: a short-lived JWT (HS256, `kid` = key id, `aud` = `/admin/`, 5 min), header `Authorization: Ghost <jwt>`.
//! - Pull: `GET /ghost/api/admin/{posts,pages}/?formats=lexical&include=tags&limit=50&page=N`
//!   (`meta.pagination.next`). Posts carrying another language's internal tag (`#fr`, `#pt-br`) are
//!   translations, never sources. Fields: `title`, `custom_excerpt`, `meta_title`, `meta_description`,
//!   `og_*`, `twitter_*` (plain) and `lexical` (text nodes only: formatting, links, cards and inline code kept).
//! - Push: the translation is the post with slug `{source slug}-{locale}`: updated with `PUT` and its
//!   `updated_at` (Ghost's collision check), else created with the source's tags plus `#{locale}`.
//! - Webhooks: `POST /ghost/api/admin/webhooks/` (post/page added and edited) with `webhook_secret`;
//!   `X-Ghost-Signature: sha256=<hex HMAC-SHA256 of body + t>, t=<ms>`.
//!
//! Docs: https://docs.ghost.org/admin-api, https://docs.ghost.org/admin-api/posts/updating-a-post,
//! https://docs.ghost.org/admin-api/webhooks/creating-a-webhook (signature: Ghost's webhook-trigger.js)
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct Ghost;

const PLAIN: &[&str] = &[
    "title",
    "custom_excerpt",
    "meta_title",
    "meta_description",
    "og_title",
    "og_description",
    "twitter_title",
    "twitter_description",
];
/// Lexical text format bit for inline code.
const CODE_BIT: i64 = 16;
const CODE: &str = "glot:code:";

struct Api {
    http: reqwest::Client,
    base: String,
    key_id: String,
    secret: Vec<u8>,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let base = required(c, "url")?.trim_end_matches('/').to_string();
        let (id, secret) = required(c, "admin_key")?
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("the Admin API key looks like {{id}}:{{secret}}"))?;
        Ok(Api {
            http: super::client_for(&base).await?,
            base,
            key_id: id.to_string(),
            secret: hex::decode(secret)
                .map_err(|_| anyhow::anyhow!("the Admin API key secret is not hex"))?,
        })
    }
    fn jwt(&self) -> String {
        use hmac::{Hmac, Mac};
        let b64 = |v: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let now = chrono::Utc::now().timestamp();
        let head = b64(json!({"alg": "HS256", "typ": "JWT", "kid": self.key_id})
            .to_string()
            .as_bytes());
        let claims = b64(json!({"iat": now, "exp": now + 300, "aud": "/admin/"})
            .to_string()
            .as_bytes());
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.secret).expect("any key size");
        mac.update(format!("{head}.{claims}").as_bytes());
        format!("{head}.{claims}.{}", b64(&mac.finalize().into_bytes()))
    }
    async fn call(
        &self,
        m: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = format!("{}/ghost/api/admin{path}", self.base);
        let res = send(|| {
            let r = self
                .http
                .request(m.clone(), &url)
                .header("authorization", format!("Ghost {}", self.jwt()))
                .header("accept-version", "v5.0");
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

/// Internal tag naming a language (`#fr`, `#pt-br`, `#en_US`) → the code.
fn lang_tag(name: &str) -> Option<&str> {
    let c = name.strip_prefix('#')?;
    let mut parts = c.split(['-', '_']);
    let lang = parts.next()?;
    let ok = (2..=3).contains(&lang.len())
        && lang.chars().all(|x| x.is_ascii_alphabetic())
        && parts
            .all(|p| (2..=4).contains(&p.len()) && p.chars().all(|x| x.is_ascii_alphanumeric()));
    ok.then_some(c)
}

fn tag_names(o: &Value) -> Vec<String> {
    o["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

fn is_source(o: &Value, source_tag: Option<&str>) -> bool {
    let tags = tag_names(o);
    match source_tag {
        Some(s) => tags.iter().any(|t| t.eq_ignore_ascii_case(s)),
        None => !tags.iter().any(|t| lang_tag(t).is_some()),
    }
}

/// Hides inline code (`format & 16`) from the translator; [`unprotect`] undoes it.
fn protect(v: &mut Value) {
    match v {
        Value::Object(o) => {
            let code = o
                .get("format")
                .and_then(Value::as_i64)
                .is_some_and(|f| f & CODE_BIT != 0);
            if let (true, Some(Value::String(t))) = (code, o.get("type")) {
                o.insert("type".into(), Value::String(format!("{CODE}{t}")));
            }
            o.values_mut().for_each(protect);
        }
        Value::Array(a) => a.iter_mut().for_each(protect),
        _ => {}
    }
}

fn unprotect(v: &mut Value) {
    match v {
        Value::Object(o) => {
            if let Some(t) = o
                .get("type")
                .and_then(Value::as_str)
                .and_then(|t| t.strip_prefix(CODE))
            {
                o.insert("type".into(), Value::String(t.to_string()));
            }
            o.values_mut().for_each(unprotect);
        }
        Value::Array(a) => a.iter_mut().for_each(unprotect),
        _ => {}
    }
}

fn items_of(res: &str, o: &Value) -> Vec<SourceItem> {
    let (Some(id), Some(slug)) = (o["id"].as_str(), o["slug"].as_str()) else {
        return vec![];
    };
    let title = o["title"].as_str().unwrap_or("");
    let what = res.trim_end_matches('s');
    let meta = json!({"resource": res, "id": id, "slug": slug, "tags": tag_names(o)});
    let item = |field: &str, text: String, format: Format| SourceItem {
        external_id: format!("{res}:{id}"),
        field: field.to_string(),
        key: key("ghost", res, id, field),
        text,
        format,
        context: format!("Ghost {what} \u{ab}{title}\u{bb}, field {field}"),
        updated_at: None,
        meta: meta.clone(),
    };
    let mut out: Vec<SourceItem> = PLAIN
        .iter()
        .filter_map(|f| {
            let v = o[*f].as_str()?;
            (!v.trim().is_empty()).then(|| item(f, v.to_string(), Format::Plain))
        })
        .collect();
    if let Some(mut doc) = o["lexical"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        protect(&mut doc);
        out.push(item("lexical", String::new(), Format::RichText(doc)));
    }
    out
}

fn resources(c: &Creds) -> Vec<&'static str> {
    if c.setting("pages") == Some(&Value::Bool(false)) {
        vec!["posts"]
    } else {
        vec!["posts", "pages"]
    }
}

/// `{"posts": [post]}`.
fn wrap(res: &str, post: Map<String, Value>) -> Value {
    let mut m = Map::new();
    m.insert(res.to_string(), Value::Array(vec![Value::Object(post)]));
    Value::Object(m)
}

/// Slug of the `locale` translation of `slug`.
fn translated_slug(slug: &str, locale: &str) -> String {
    format!("{slug}-{}", locale.to_ascii_lowercase().replace('_', "-"))
}

#[async_trait]
impl Connector for Ghost {
    fn kind(&self) -> &'static str {
        "ghost"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        // Authenticated: a wrong key fails here.
        api.get("/posts/?limit=1&fields=id").await?;
        let site = api.get("/site/").await.unwrap_or(Value::Null);
        let langs = self.locales(creds).await?;
        Ok(AccountInfo {
            name: site["site"]["title"]
                .as_str()
                .unwrap_or("Ghost")
                .to_string(),
            source_locale: cms::param(creds, "source_tag")
                .and_then(lang_tag)
                .map(String::from),
            notes: vec![format!(
                "Ghost has no built-in translations: each language is a post tagged #<locale> (slug <slug>-<locale>). Languages found: {}. Route them with routes.yaml collections.",
                if langs.is_empty() {
                    "none yet".into()
                } else {
                    langs.join(", ")
                }
            )],
        })
    }

    /// Languages = the internal language tags in use.
    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let api = Api::new(creds).await?;
        let tags = api
            .get("/tags/?limit=all&filter=visibility:internal")
            .await?;
        let src = cms::param(creds, "source_tag");
        let mut out: Vec<String> = tags["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["name"].as_str())
            .filter(|n| Some(*n) != src)
            .filter_map(lang_tag)
            .map(String::from)
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Cursor = `{resource index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let res = resources(creds);
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u32>()?),
            None => (0, 1),
        };
        let Some(r) = res.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let mut q = format!(
            "/{r}/?formats=lexical&include=tags&limit=50&page={page}&order=created_at%20asc"
        );
        if let Some(s) = creds.since {
            let f = format!("updated_at:>'{}'", s.format("%Y-%m-%d %H:%M:%S"));
            q.push_str(&format!("&filter={}", cms::pct(&f)));
        }
        let list = api.get(&q).await?;
        let src = cms::param(creds, "source_tag");
        let items = list[r]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| is_source(o, src))
            .flat_map(|o| items_of(r, o))
            .collect();
        let next = match list["meta"]["pagination"]["next"].as_u64() {
            Some(n) => Some(format!("{i}|{n}")),
            None if i + 1 < res.len() => Some(format!("{}|1", i + 1)),
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
        let status = if creds.setting("publish") == Some(&Value::Bool(true)) {
            "published"
        } else {
            "draft"
        };
        let tag = format!("#{}", locale.to_ascii_lowercase());
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let meta = &its[0].meta;
            let r: anyhow::Result<()> = async {
                let (Some(res), Some(slug)) = (meta["resource"].as_str(), meta["slug"].as_str())
                else {
                    anyhow::bail!("missing metadata, sync again");
                };
                let mut post = Map::new();
                for it in &its {
                    let v = match &it.format {
                        Format::RichText(doc) => {
                            let mut d = doc.clone();
                            unprotect(&mut d);
                            Value::String(d.to_string())
                        }
                        _ => Value::String(it.value.clone()),
                    };
                    post.insert(it.field.clone(), v);
                }
                let tslug = translated_slug(slug, locale);
                match api
                    .get(&format!("/{res}/slug/{tslug}/?fields=id,updated_at"))
                    .await
                {
                    Ok(found) => {
                        let t = &found[res][0];
                        let id = t["id"].as_str().ok_or_else(|| anyhow::anyhow!("no id"))?;
                        post.insert("updated_at".into(), t["updated_at"].clone());
                        api.call(
                            reqwest::Method::PUT,
                            &format!("/{res}/{id}/"),
                            Some(&wrap(res, post)),
                        )
                        .await?;
                    }
                    Err(e) if e.to_string().starts_with("HTTP 404") => {
                        anyhow::ensure!(
                            post.contains_key("title"),
                            "no {locale} post yet and its title is not translated yet"
                        );
                        let mut tags: Vec<Value> = meta["tags"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|t| t.as_str().and_then(lang_tag).is_none())
                            .map(|t| json!({"name": t}))
                            .collect();
                        tags.push(json!({"name": tag}));
                        post.insert("slug".into(), Value::String(tslug));
                        post.insert("tags".into(), Value::Array(tags));
                        post.insert("status".into(), Value::String(status.into()));
                        api.call(
                            reqwest::Method::POST,
                            &format!("/{res}/"),
                            Some(&wrap(res, post)),
                        )
                        .await?;
                    }
                    Err(e) => return Err(e),
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

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        if cms::shared_secret_ok(headers, secret) {
            return true;
        }
        // `sha256=<hex>, t=<timestamp>`; the HMAC covers `body` then `t`.
        let Some(h) = cms::header(headers, "x-ghost-signature") else {
            return false;
        };
        let mut sig = None;
        let mut ts = None;
        for part in h.split(',').map(str::trim) {
            if let Some(s) = part.strip_prefix("sha256=") {
                sig = Some(s);
            } else if let Some(t) = part.strip_prefix("t=") {
                ts = Some(t);
            }
        }
        let (Some(sig), Some(ts)) = (sig, ts) else {
            return false;
        };
        let mut signed = body.to_vec();
        signed.extend_from_slice(ts.as_bytes());
        !secret.is_empty() && super::hmac_ok(secret, &signed, sig)
    }

    async fn register_webhooks(&self, creds: &Creds, url: &str) -> anyhow::Result<()> {
        let Some(secret) = creds.opt("webhook_secret") else {
            anyhow::bail!(
                "no webhook secret: Ghost webhooks not registered (add one, or create them in Ghost with the card URL)"
            );
        };
        let api = Api::new(creds).await?;
        for event in ["post.added", "post.edited", "page.added", "page.edited"] {
            let body = json!({"webhooks": [{"event": event, "target_url": url, "secret": secret, "name": format!("glot {event}")}]});
            match api
                .call(reqwest::Method::POST, "/webhooks/", Some(&body))
                .await
            {
                // 422: already registered for this event and URL.
                Err(e) if e.to_string().starts_with("HTTP 422") => {}
                r => {
                    r?;
                }
            }
        }
        Ok(())
    }

    fn push_batch(&self) -> usize {
        25
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::rich;

    #[test]
    fn tags_and_code() {
        assert_eq!(lang_tag("#fr"), Some("fr"));
        assert_eq!(lang_tag("#pt-br"), Some("pt-br"));
        assert_eq!(lang_tag("#featured"), None);
        assert_eq!(lang_tag("fr"), None);
        let tr = json!({"tags": [{"name": "News"}, {"name": "#fr"}]});
        assert!(!is_source(&tr, None) && is_source(&json!({"tags": [{"name": "News"}]}), None));
        assert!(is_source(&json!({"tags": [{"name": "#en"}]}), Some("#en")));
        let lex = json!({"root": {"children": [{"type": "paragraph", "children": [
            {"type": "extended-text", "text": "Run ", "format": 0},
            {"type": "extended-text", "text": "npm install", "format": 16}]}]}});
        let mut d = lex.clone();
        protect(&mut d);
        let u = rich::units("k", &Format::RichText(d.clone()), "", "en");
        assert_eq!(u.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["Run"]);
        unprotect(&mut d);
        assert_eq!(d, lex);
    }
}
