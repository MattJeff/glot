//! Sanity connector (HTTP API: GROQ query + mutations).
//!
//! Credentials: `project_id`, `dataset`, `token` (Editor). Settings: `mode` (`field` | `document`, detected
//! when absent), `source_locale` (default `en`), `locales` (else detected), `language_field` (document mode,
//! default `language`), `types` (only these document types), `fields` (document mode: only these field names),
//! `api_version` (default `v2025-02-19`), `api_host` (default `https://{project_id}.api.sanity.io`).
//!
//! The two common i18n models:
//! - **field** (`@sanity/language-filter`, objects `{en, fr}`, `sanity-plugin-internationalized-array`):
//!   the source locale's value of every localized object / array is pulled; push sets `{path}.{locale}`
//!   (objects) or replaces the `{_key: locale}` item (internationalized arrays).
//! - **document** (`@sanity/document-internationalization`): documents whose `language` is the source locale;
//!   text strings and Portable Text are pulled; push patches the linked translation (`translation.metadata`)
//!   or creates it (copy of the source with the translated fields, `language` set, metadata linked).
//!
//! Portable Text: only `span.text` is translated (`marks`, `markDefs`, `_key`s, custom blocks kept; code
//! marks protected). Drafts are left to the editors: the published document is written (and its draft, if any).
//! Webhooks: GROQ-powered webhooks signed with a secret (`sanity-webhook-signature`, put the same secret in
//! the connector's `webhook_secret`), or the dashboard URL with its token.
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use super::cms::{self, param, required};
use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key, send,
};

pub struct Sanity;

const PAGE: usize = 100;
const BASE_FILTER: &str = "!(_id in path(\"drafts.**\")) && !(_id in path(\"versions.**\")) && !(_type match \"system.*\") && !(_type match \"sanity.*\") && _type != \"translation.metadata\"";

struct Api {
    http: reqwest::Client,
    base: String,
    dataset: String,
    token: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let project = required(c, "project_id")?;
        anyhow::ensure!(
            project.chars().all(|c| c.is_ascii_alphanumeric()),
            "invalid project id"
        );
        let dataset = required(c, "dataset")?;
        anyhow::ensure!(
            dataset
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
            "invalid dataset"
        );
        let host = param(c, "api_host")
            .map(|h| h.trim_end_matches('/').to_string())
            .unwrap_or_else(|| format!("https://{project}.api.sanity.io"));
        let v = param(c, "api_version").unwrap_or("v2025-02-19");
        anyhow::ensure!(
            v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "invalid api version"
        );
        Ok(Api {
            http: super::client_for(&host).await?,
            base: format!("{host}/{v}/data"),
            dataset: dataset.to_string(),
            token: required(c, "token")?.to_string(),
        })
    }
    async fn query(&self, q: &str, params: Value) -> anyhow::Result<Value> {
        let url = format!("{}/query/{}?perspective=raw", self.base, self.dataset);
        let body = json!({"query": q, "params": params});
        let r: Value = send(|| self.http.post(&url).bearer_auth(&self.token).json(&body))
            .await?
            .json()
            .await?;
        Ok(r["result"].clone())
    }
    async fn mutate(&self, mutations: Vec<Value>) -> anyhow::Result<()> {
        let url = format!("{}/mutate/{}?visibility=async", self.base, self.dataset);
        let body = json!({ "mutations": mutations });
        send(|| self.http.post(&url).bearer_auth(&self.token).json(&body)).await?;
        Ok(())
    }
}

fn src_locale(c: &Creds) -> String {
    param(c, "source_locale").unwrap_or("en").to_string()
}
fn lang_field(c: &Creds) -> anyhow::Result<String> {
    let f = param(c, "language_field").unwrap_or("language");
    anyhow::ensure!(
        f.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "invalid language field"
    );
    Ok(f.to_string())
}
fn list(c: &Creds, name: &str) -> Option<Vec<String>> {
    c.setting(name)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

async fn mode(api: &Api, c: &Creds) -> anyhow::Result<&'static str> {
    Ok(match param(c, "mode") {
        Some("field") => "field",
        Some("document") => "document",
        _ => {
            let n = api
                .query(
                    "count(*[_type == \"translation.metadata\"][0...1])",
                    json!({}),
                )
                .await?;
            if n.as_u64().unwrap_or(0) > 0 {
                "document"
            } else {
                "field"
            }
        }
    })
}

// ---------------------------------------------------------------------------------------------- paths

/// One step of a document path: field name, `_key` of an array item, or array index.
#[derive(Debug, Clone, PartialEq)]
enum Seg {
    Field(String),
    Key(String),
    Index(usize),
}

fn path_str(p: &[Seg]) -> String {
    let mut s = String::new();
    for seg in p {
        match seg {
            Seg::Field(f) if s.is_empty() => s.push_str(f),
            Seg::Field(f) => {
                s.push('.');
                s.push_str(f);
            }
            Seg::Key(k) => s.push_str(&format!("[_key=={}]", Value::String(k.clone()))),
            Seg::Index(i) => s.push_str(&format!("[{i}]")),
        }
    }
    s
}

fn segs_json(p: &[Seg]) -> Value {
    Value::Array(
        p.iter()
            .map(|s| match s {
                Seg::Field(f) => json!(f),
                Seg::Key(k) => json!({"_key": k}),
                Seg::Index(i) => json!(i),
            })
            .collect(),
    )
}

fn segs_of(v: &Value) -> Vec<Seg> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| match s {
            Value::String(f) => Some(Seg::Field(f.clone())),
            Value::Number(n) => n.as_u64().map(|i| Seg::Index(i as usize)),
            Value::Object(o) => o
                .get("_key")
                .and_then(Value::as_str)
                .map(|k| Seg::Key(k.into())),
            _ => None,
        })
        .collect()
}

/// Mutable value at `p` in `doc`, `None` when the path does not exist.
fn at_mut<'a>(doc: &'a mut Value, p: &[Seg]) -> Option<&'a mut Value> {
    let mut v = doc;
    for seg in p {
        v = match seg {
            Seg::Field(f) => v.get_mut(f.as_str())?,
            Seg::Index(i) => v.get_mut(*i)?,
            Seg::Key(k) => v
                .as_array_mut()?
                .iter_mut()
                .find(|x| x.get("_key").and_then(Value::as_str) == Some(k))?,
        };
    }
    Some(v)
}

fn is_locale_code(s: &str) -> bool {
    let mut parts = s.splitn(2, ['-', '_']);
    let a = parts.next().unwrap_or("");
    (2..=3).contains(&a.len())
        && a.chars().all(|c| c.is_ascii_lowercase())
        && parts.next().is_none_or(|b| {
            (2..=4).contains(&b.len()) && b.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

fn is_portable_text(v: &Value) -> bool {
    v.as_array()
        .is_some_and(|a| !a.is_empty() && a.iter().any(|b| b["_type"] == "block"))
}

/// Value as a pulled format, `None` when there is nothing to translate.
fn format_of(v: &Value) -> Option<Format> {
    match v {
        Value::String(s) if cms::has_letter(s) => Some(Format::Plain),
        v if is_portable_text(v) => {
            let mut d = v.clone();
            cms::protect_code(&mut d);
            Some(Format::RichText(d))
        }
        _ => None,
    }
}

/// A localized value found in field mode.
struct Localized {
    path: Vec<Seg>,
    value: Value,
    /// `None` = object `{en, fr}`; `Some(item _type)` = internationalized array item.
    array_item: Option<String>,
}

fn walk_field(v: &Value, path: &mut Vec<Seg>, src: &str, out: &mut Vec<Localized>) {
    match v {
        Value::Object(o) => {
            let keys: Vec<&String> = o.keys().filter(|k| !k.starts_with('_')).collect();
            if o.contains_key(src) && keys.iter().all(|k| is_locale_code(k)) {
                out.push(Localized {
                    path: path.clone(),
                    value: o[src].clone(),
                    array_item: None,
                });
                return;
            }
            for k in keys {
                path.push(Seg::Field(k.clone()));
                walk_field(&o[k.as_str()], path, src, out);
                path.pop();
            }
        }
        Value::Array(a) => {
            let i18n_array = !a.is_empty()
                && a.iter().all(|x| {
                    x["_type"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("internationalizedArray"))
                        && x["_key"].as_str().is_some_and(is_locale_code)
                });
            if i18n_array {
                if let Some(item) = a.iter().find(|x| x["_key"] == src) {
                    out.push(Localized {
                        path: path.clone(),
                        value: item["value"].clone(),
                        array_item: item["_type"].as_str().map(String::from),
                    });
                }
                return;
            }
            for (i, x) in a.iter().enumerate() {
                path.push(match x["_key"].as_str() {
                    Some(k) => Seg::Key(k.into()),
                    None => Seg::Index(i),
                });
                walk_field(x, path, src, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// Field names never translated in document mode (identifiers, links, settings).
const NOT_TEXT: &[&str] = &[
    "slug", "url", "href", "link", "email", "color", "colour", "icon", "id", "key", "variant",
    "layout", "theme", "style", "type", "code", "language", "locale", "lang",
];

fn walk_doc(
    v: &Value,
    path: &mut Vec<Seg>,
    fields: Option<&[String]>,
    lang: &str,
    out: &mut Vec<(Vec<Seg>, Value)>,
) {
    match v {
        Value::Object(o) => {
            if o.contains_key("_ref")
                || matches!(
                    o.get("_type").and_then(Value::as_str),
                    Some("slug" | "code" | "reference")
                )
            {
                return;
            }
            for (k, c) in o {
                if k.starts_with('_') || k == lang {
                    continue;
                }
                path.push(Seg::Field(k.clone()));
                let wanted = match fields {
                    Some(f) => f.contains(k),
                    None => !NOT_TEXT.contains(&k.to_ascii_lowercase().as_str()),
                };
                match c {
                    Value::String(s) if wanted && (fields.is_some() || cms::looks_like_text(s)) => {
                        out.push((path.clone(), c.clone()))
                    }
                    c if wanted && is_portable_text(c) => out.push((path.clone(), c.clone())),
                    Value::Object(_) | Value::Array(_) if !is_portable_text(c) => {
                        walk_doc(c, path, fields, lang, out)
                    }
                    _ => {}
                }
                path.pop();
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                path.push(match x["_key"].as_str() {
                    Some(k) => Seg::Key(k.into()),
                    None => Seg::Index(i),
                });
                match x {
                    Value::String(s) if fields.is_none() && cms::looks_like_text(s) => {
                        out.push((path.clone(), x.clone()))
                    }
                    _ => walk_doc(x, path, fields, lang, out),
                }
                path.pop();
            }
        }
        _ => {}
    }
}

fn context(d: &Value) -> String {
    let ty = d["_type"].as_str().unwrap_or_default();
    let title = ["title", "name", "heading"]
        .iter()
        .find_map(|f| {
            let v = &d[*f];
            v.as_str().map(String::from).or_else(|| {
                v.as_object()
                    .and_then(|o| o.values().find_map(Value::as_str))
                    .map(String::from)
            })
        })
        .unwrap_or_default();
    if title.is_empty() {
        ty.to_string()
    } else {
        format!("{ty}: {title}")
    }
}

#[async_trait]
impl Connector for Sanity {
    fn kind(&self) -> &'static str {
        "sanity"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let n = api
            .query(&format!("count(*[{BASE_FILTER}])"), json!({}))
            .await?;
        let m = mode(&api, creds).await?;
        let locales = self.locales(creds).await.unwrap_or_default();
        Ok(AccountInfo {
            name: format!("{} / {}", required(creds, "project_id")?, api.dataset),
            source_locale: Some(src_locale(creds)),
            notes: vec![
                format!("{} documents", n.as_u64().unwrap_or(0)),
                if m == "document" {
                    "document-level i18n (@sanity/document-internationalization)".into()
                } else {
                    "field-level i18n (localized objects / internationalized arrays)".into()
                },
                format!("locales: {}", locales.join(", ")),
            ],
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        if let Some(l) = list(creds, "locales") {
            return Ok(l);
        }
        let api = Api::new(creds).await?;
        let src = src_locale(creds);
        let mut out = std::collections::BTreeSet::from([src.clone()]);
        if mode(&api, creds).await? == "document" {
            let lang = lang_field(creds)?;
            let r = api
                .query(
                    &format!("array::unique(*[defined({lang})].{lang})"),
                    json!({}),
                )
                .await?;
            out.extend(
                r.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from),
            );
        } else {
            let docs = api
                .query(&format!("*[{BASE_FILTER}][0...{PAGE}]"), json!({}))
                .await?;
            for d in docs.as_array().into_iter().flatten() {
                let mut found = vec![];
                collect_locales(d, &src, &mut found);
                out.extend(found);
            }
        }
        Ok(out.into_iter().collect())
    }

    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let src = src_locale(creds);
        let m = mode(&api, creds).await?;
        let lang = lang_field(creds)?;
        let mut filter = format!("{BASE_FILTER} && _id > $last");
        let mut params = Map::new();
        params.insert("last".into(), json!(cursor.clone().unwrap_or_default()));
        if let Some(t) = list(creds, "types") {
            filter.push_str(" && _type in $types");
            params.insert("types".into(), json!(t));
        }
        if m == "document" {
            filter.push_str(&format!(" && {lang} == $src"));
            params.insert("src".into(), json!(src));
        }
        if let Some(s) = creds.since {
            filter.push_str(" && _updatedAt >= $since");
            params.insert("since".into(), json!(s.to_rfc3339()));
        }
        let docs = api
            .query(
                &format!("*[{filter}] | order(_id) [0...{PAGE}]"),
                Value::Object(params),
            )
            .await?;
        let docs = docs.as_array().cloned().unwrap_or_default();
        let fields = list(creds, "fields");
        let mut items = vec![];
        for d in &docs {
            let id = d["_id"].as_str().unwrap_or_default();
            let ty = d["_type"].as_str().unwrap_or_default();
            let found: Vec<(Vec<Seg>, Value, Option<String>)> = if m == "document" {
                let mut out = vec![];
                walk_doc(d, &mut vec![], fields.as_deref(), &lang, &mut out);
                out.into_iter().map(|(p, v)| (p, v, None)).collect()
            } else {
                let mut out = vec![];
                walk_field(d, &mut vec![], &src, &mut out);
                out.into_iter()
                    .map(|l| (l.path, l.value, l.array_item))
                    .collect()
            };
            for (path, value, array_item) in found {
                let Some(format) = format_of(&value) else {
                    continue;
                };
                let field = path_str(&path);
                items.push(SourceItem {
                    external_id: id.to_string(),
                    key: key("sanity", ty, id, &field),
                    field,
                    text: value.as_str().unwrap_or_default().to_string(),
                    format,
                    context: context(d),
                    updated_at: d["_updatedAt"].as_str().and_then(|t| t.parse().ok()),
                    meta: json!({"mode": m, "type": ty, "path": segs_json(&path), "array_item": array_item}),
                });
            }
        }
        Ok(PullPage {
            items,
            next: (docs.len() == PAGE).then(|| {
                docs.last()
                    .and_then(|d| d["_id"].as_str())
                    .unwrap_or_default()
                    .to_string()
            }),
        })
    }

    async fn push(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport> {
        let api = Api::new(creds).await?;
        let mut by_doc: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_doc.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (id, its) in by_doc {
            let res = if its[0].meta["mode"] == "document" {
                push_document(&api, creds, id, locale, &its).await
            } else {
                push_fields(&api, id, locale, &its).await
            };
            match res {
                Ok(()) => report.ok += its.len(),
                Err(e) => report.failed.push((id.to_string(), format!("{e:#}"))),
            }
        }
        Ok(report)
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
            || cms::header(headers, "sanity-webhook-signature")
                .is_some_and(|s| signature_ok(s, body, secret))
    }

    fn push_batch(&self) -> usize {
        25
    }
}

fn collect_locales(v: &Value, src: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(o) => {
            let keys: Vec<&String> = o.keys().filter(|k| !k.starts_with('_')).collect();
            if o.contains_key(src) && keys.iter().all(|k| is_locale_code(k)) {
                out.extend(keys.into_iter().cloned());
                return;
            }
            o.values().for_each(|c| collect_locales(c, src, out));
        }
        Value::Array(a) => {
            for x in a {
                if x["_type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("internationalizedArray"))
                {
                    if let Some(k) = x["_key"].as_str() {
                        out.push(k.into());
                    }
                } else {
                    collect_locales(x, src, out);
                }
            }
        }
        _ => {}
    }
}

/// `sanity-webhook-signature: t=<ms>,v1=<base64url(HMAC-SHA256(secret, "<t>.<body>"))>` (webhook-toolkit).
fn signature_ok(header: &str, body: &[u8], secret: &str) -> bool {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut t = None;
    let mut v1 = None;
    for part in header.split([',', ' ']).filter(|p| !p.is_empty()) {
        if let Some(x) = part.strip_prefix("t=") {
            t = Some(x);
        } else if let Some(x) = part.strip_prefix("v1=") {
            v1 = Some(x);
        }
    }
    let (Some(t), Some(v1)) = (t, v1) else {
        return false;
    };
    let Ok(sig) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(v1.trim_end_matches('='))
    else {
        return false;
    };
    if secret.is_empty() || !t.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(t.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

fn value_of(it: &PushItem) -> Value {
    match &it.format {
        Format::RichText(doc) => cms::pushed_value(doc),
        _ => Value::String(it.value.clone()),
    }
}

/// Ids of `ids` that exist as drafts (`drafts.<id>`).
async fn drafts(api: &Api, ids: &[String]) -> anyhow::Result<Vec<String>> {
    let d: Vec<String> = ids.iter().map(|i| format!("drafts.{i}")).collect();
    let r = api.query("*[_id in $ids]._id", json!({ "ids": d })).await?;
    Ok(r.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect())
}

/// Field mode: one patch on the document (and on its draft).
async fn push_fields(api: &Api, id: &str, locale: &str, its: &[&PushItem]) -> anyhow::Result<()> {
    let mut set = Map::new();
    let mut unset = vec![];
    let mut inserts = vec![];
    for it in its {
        let path = path_str(&segs_of(&it.meta["path"]));
        match it.meta["array_item"].as_str() {
            None => {
                set.insert(format!("{path}.{locale}"), value_of(it));
            }
            Some(item_type) => {
                unset.push(Value::String(format!(
                    "{path}[_key=={}]",
                    Value::String(locale.into())
                )));
                inserts.push(json!({"insert": {"after": format!("{path}[-1]"),
                    "items": [{"_key": locale, "_type": item_type, "value": value_of(it)}]}}));
            }
        }
    }
    let mut targets = vec![id.to_string()];
    targets.extend(drafts(api, &[id.to_string()]).await?);
    let mut mutations = vec![];
    for t in &targets {
        if !set.is_empty() || !unset.is_empty() {
            mutations.push(json!({"patch": {"id": t, "set": set, "unset": unset}}));
        }
        // One insert per patch (a patch takes a single `insert`).
        for ins in &inserts {
            let mut p = ins.clone();
            p["id"] = json!(t);
            mutations.push(json!({ "patch": p }));
        }
    }
    api.mutate(mutations).await
}

/// Document mode: patch the linked translation, or create it and link it in `translation.metadata`.
async fn push_document(
    api: &Api,
    creds: &Creds,
    id: &str,
    locale: &str,
    its: &[&PushItem],
) -> anyhow::Result<()> {
    let lang = lang_field(creds)?;
    let r = api
        .query(
            "{\"doc\": *[_id == $id][0], \"meta\": *[_type == \"translation.metadata\" && references($id)][0]{_id, translations}}",
            json!({ "id": id }),
        )
        .await?;
    let src = r["doc"].clone();
    anyhow::ensure!(src.is_object(), "document {id} not found");
    let existing = r["meta"]["translations"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["_key"] == locale)
        .and_then(|t| t["value"]["_ref"].as_str())
        .map(String::from);
    if let Some(target) = existing {
        let mut set = Map::new();
        for it in its {
            set.insert(path_str(&segs_of(&it.meta["path"])), value_of(it));
        }
        let mut targets = vec![target.clone()];
        targets.extend(drafts(api, &[target]).await?);
        let mutations = targets
            .iter()
            .map(|t| json!({"patch": {"id": t, "set": set}}))
            .collect();
        return api.mutate(mutations).await;
    }
    let new_id = format!(
        "{id}-{}",
        locale.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    );
    let mut copy = src.clone();
    if let Some(o) = copy.as_object_mut() {
        for k in ["_rev", "_createdAt", "_updatedAt"] {
            o.remove(k);
        }
        o.insert("_id".into(), json!(new_id));
        o.insert(lang.clone(), json!(locale));
    }
    for it in its {
        if let Some(v) = at_mut(&mut copy, &segs_of(&it.meta["path"])) {
            *v = value_of(it);
        }
    }
    let reference = |key: &str, r: &str| {
        json!({"_key": key, "_type": "internationalizedArrayReferenceValue",
               "value": {"_type": "reference", "_ref": r, "_weak": true}})
    };
    let mut mutations = vec![json!({ "createOrReplace": copy })];
    match r["meta"]["_id"].as_str() {
        Some(meta) => mutations.push(json!({"patch": {"id": meta,
            "insert": {"after": "translations[-1]", "items": [reference(locale, &new_id)]}}})),
        None => mutations.push(json!({"createIfNotExists": {
            "_id": format!("translation.metadata.{id}"),
            "_type": "translation.metadata",
            "schemaTypes": [src["_type"]],
            "translations": [reference(src[lang.as_str()].as_str().unwrap_or("en"), id), reference(locale, &new_id)],
        }})),
    }
    api.mutate(mutations).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_mode_finds_localized_values() {
        let d = json!({"_id": "p1", "_type": "post",
            "title": {"_type": "localeString", "en": "Hello", "fr": "Bonjour"},
            "tags": [{"_key": "en", "_type": "internationalizedArrayStringValue", "value": "News today"}],
            "sections": [{"_key": "s1", "heading": {"en": "Intro", "de": ""}}],
            "slug": {"_type": "slug", "current": "hello"}});
        let mut out = vec![];
        walk_field(&d, &mut vec![], "en", &mut out);
        out.sort_by_key(|l| path_str(&l.path));
        let paths: Vec<_> = out.iter().map(|l| path_str(&l.path)).collect();
        assert_eq!(paths, ["sections[_key==\"s1\"].heading", "tags", "title"]);
        assert_eq!(
            out[1].array_item.as_deref(),
            Some("internationalizedArrayStringValue")
        );
        let mut l = vec![];
        collect_locales(&d, "en", &mut l);
        assert!(l.contains(&"fr".to_string()) && l.contains(&"de".to_string()));
    }

    #[test]
    fn document_mode_heuristics() {
        let d = json!({"_id": "a", "_type": "page", "language": "en", "title": "Pricing", "layout": "grid",
            "cta": {"label": "Start now", "href": "/signup"}, "slug": {"_type": "slug", "current": "pricing"},
            "author": {"_ref": "x"}, "items": ["Fast and safe", "x"],
            "body": [{"_type": "block", "_key": "b", "children": [{"_type": "span", "text": "Hi there"}]}]});
        let mut out = vec![];
        walk_doc(&d, &mut vec![], None, "language", &mut out);
        out.sort_by_key(|(p, _)| path_str(p));
        let paths: Vec<_> = out.iter().map(|(p, _)| path_str(p)).collect();
        assert_eq!(paths, ["body", "cta.label", "items[0]", "title"]);
        let mut c = d.clone();
        *at_mut(&mut c, &segs_of(&segs_json(&out[2].0))).unwrap() = json!("Rapide");
        assert_eq!(c["items"][0], "Rapide");
    }

    #[test]
    fn webhook_signature() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let body = br#"{"_id":"p1"}"#;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"whsec").unwrap();
        mac.update(b"1700000000000.");
        mac.update(body);
        let sig =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        let h = format!("t=1700000000000,v1={sig}");
        assert!(signature_ok(&h, body, "whsec"));
        assert!(!signature_ok(&h, b"{}", "whsec"));
        assert!(!signature_ok(&h, body, "other"));
        assert!(!signature_ok("garbage", body, "whsec"));
    }
}
