//! Craft CMS (GraphQL API, Craft 4 and 5): entries, one Craft **site** per language.
//!
//! Credentials: `url` (the GraphQL endpoint, e.g. `https://example.com/api`), `token` (a GraphQL schema token
//! whose schema can query the sections and **save** their entries in every site), `sites` (glot locale →
//! Craft site handle: `fr: french, de: german`, or the setting `{"fr": "french"}`; default: the locale
//! with `-` → `_`). Settings: `source_site`
//! (handle; default: the primary site), `sections` (section handles to sync; default all).
//!
//! - Pull: `entries(site, limit: 50, offset)`, then per entry type the `String` fields found by
//!   introspection (Plain Text, CKEditor and Redactor return strings; HTML values keep their tags), minus
//!   system fields (`slug`, `uri`, `url`…). Matrix / Neo / relation fields are not synced.
//! - Push: `save_{section}_{type}_Entry(id, siteId, title, …fields)` for the entry in the target site, whose
//!   `siteId` comes from `entry(id, site)` (the section must be enabled in that site, with fields
//!   translatable per site).
//! - Webhooks: Craft has none built in; the first-party Webhooks plugin calls the card URL (token included)
//!   or sends `X-Glot-Webhook-Secret`.
//! - Read back (`current`): the same entries queried in the target site and in the source site. Craft
//!   propagates an entry to every site with the source content, so a value identical to the source site's
//!   is "no translation". `remove` is not implemented: a site's field value cannot be unset, only overwritten.
//! - Diagnosis (`doctor`): every mapped site answers with entries, and the token's schema exposes
//!   `save_…_Entry` mutations (introspection of the mutation type).
//!
//! Docs: https://craftcms.com/docs/5.x/development/graphql.html (queries, `site` argument, mutations),
//! https://craftcms.com/docs/5.x/reference/element-types/entries.html, https://plugins.craftcms.com/webhooks
use async_trait::async_trait;
use axum::http::HeaderMap;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

use super::cms::{self, required};
use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, key,
    send,
};

pub struct Craft;

const PAGE: usize = 50;
/// String fields of every entry that are not content.
const SYSTEM: &[&str] = &[
    "id",
    "uid",
    "slug",
    "uri",
    "url",
    "siteHandle",
    "sectionHandle",
    "typeHandle",
    "status",
    "language",
    "ref",
    "postDate",
    "expiryDate",
    "dateCreated",
    "dateUpdated",
    "title",
    "siteId",
    "sectionId",
    "typeId",
    "authorId",
    "canonicalId",
    "canonicalUid",
    "sourceId",
    "sourceUid",
    "draftId",
    "revisionId",
    "draftName",
    "draftNotes",
    "fieldId",
    "ownerId",
    "searchScore",
];

struct Api {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl Api {
    async fn new(c: &Creds) -> anyhow::Result<Self> {
        let url = required(c, "url")?.to_string();
        Ok(Api {
            http: super::client_for(&url).await?,
            token: required(c, "token")?.to_string(),
            url,
        })
    }
    async fn gql(&self, query: &str, vars: Value) -> anyhow::Result<Value> {
        let body = json!({"query": query, "variables": vars});
        let res: Value = send(|| {
            self.http
                .post(&self.url)
                .bearer_auth(&self.token)
                .json(&body)
        })
        .await?
        .json()
        .await?;
        if let Some(e) = res["errors"].as_array().and_then(|a| a.first()) {
            anyhow::bail!("GraphQL: {}", e["message"].as_str().unwrap_or("error"));
        }
        Ok(res["data"].clone())
    }
}

fn ident_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn site_for(c: &Creds, locale: &str) -> String {
    cms::pairs(c, "sites")
        .remove(locale)
        .unwrap_or_else(|| locale.replace('-', "_"))
}

fn source_site(c: &Creds) -> Option<String> {
    cms::param(c, "source_site").map(String::from)
}

/// Content `String` fields of a GraphQL entry type.
async fn text_fields(api: &Api, ty: &str) -> anyhow::Result<Vec<String>> {
    let d = api
        .gql(
            "query($t: String!) { __type(name: $t) { fields { name type { kind name ofType { name } } } } }",
            json!({"t": ty}),
        )
        .await?;
    Ok(d["__type"]["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| {
            let t = &f["type"];
            t["name"] == "String" || (t["kind"] == "NON_NULL" && t["ofType"]["name"] == "String")
        })
        .filter_map(|f| f["name"].as_str())
        .filter(|n| !SYSTEM.contains(n) && !n.starts_with("__"))
        .map(String::from)
        .collect())
}

/// Text field values of `list` (entries with `id` and `__typename`) in `site`, by entry id. `fields_of`
/// caches the text fields of each entry type.
async fn typed_values(
    api: &Api,
    site: &Value,
    list: &[Value],
    fields_of: &mut HashMap<String, Vec<String>>,
) -> anyhow::Result<HashMap<String, Value>> {
    let mut by_type: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for e in list {
        if let Some(t) = e["__typename"].as_str().filter(|t| ident_ok(t)) {
            by_type.entry(t.to_string()).or_default().push(&e["id"]);
        }
    }
    let mut values = HashMap::new();
    for (ty, ids) in &by_type {
        if !fields_of.contains_key(ty) {
            fields_of.insert(ty.clone(), text_fields(api, ty).await?);
        }
        let fields = &fields_of[ty];
        if fields.is_empty() {
            continue;
        }
        let q = format!(
            "query($s: [String], $ids: [QueryArgument]) {{ entries(site: $s, id: $ids) {{ id ... on {ty} {{ {} }} }} }}",
            fields.join(" ")
        );
        let r = api.gql(&q, json!({"s": site, "ids": ids})).await?;
        for e in r["entries"].as_array().into_iter().flatten() {
            if let Some(id) = cms::id_str(&e["id"]) {
                values.insert(id, e.clone());
            }
        }
    }
    Ok(values)
}

/// `title` and text fields of the entries `ids` in `site`, by entry id.
async fn entries_in(
    api: &Api,
    site: &Value,
    ids: &[&str],
    fields_of: &mut HashMap<String, Vec<String>>,
) -> anyhow::Result<HashMap<String, Value>> {
    let d = api
        .gql(
            "query($s: [String], $ids: [QueryArgument]) { entries(site: $s, id: $ids) { id __typename title } }",
            json!({"s": site, "ids": ids}),
        )
        .await?;
    let list = d["entries"].as_array().cloned().unwrap_or_default();
    let mut values = typed_values(api, site, &list, fields_of).await?;
    for e in &list {
        if let Some(id) = cms::id_str(&e["id"]) {
            values.entry(id).or_insert_with(|| json!({}))["title"] = e["title"].clone();
        }
    }
    Ok(values)
}

fn looks_html(s: &str) -> bool {
    s.contains('<') && s.contains("</") || s.contains("<br") || s.contains("<img")
}

#[async_trait]
impl Connector for Craft {
    fn kind(&self) -> &'static str {
        "craft"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let api = Api::new(creds).await?;
        let d = api
            .gql(
                "query($s: [String]) { entries(site: $s, limit: 1) { siteHandle language } }",
                json!({"s": source_site(creds).map(|s| vec![s])}),
            )
            .await?;
        let first = &d["entries"][0];
        let sites: Vec<String> = cms::pairs(creds, "sites").into_values().collect();
        Ok(AccountInfo {
            name: format!(
                "Craft ({})",
                first["siteHandle"].as_str().unwrap_or("primary site")
            ),
            source_locale: first["language"].as_str().map(String::from),
            notes: vec![if sites.is_empty() {
                "Each glot locale is written to the Craft site of the same handle (fr-CA → fr_CA): map them in settings.sites.".into()
            } else {
                format!("Target sites: {}", sites.join(", "))
            }],
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        Ok(cms::pairs(creds, "sites").into_keys().collect())
    }

    /// Cursor = offset.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let api = Api::new(creds).await?;
        let offset: usize = cursor.as_deref().unwrap_or("0").parse()?;
        let sections: Option<Vec<String>> = creds
            .setting("sections")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            });
        let site = source_site(creds).map(|s| vec![s]);
        let mut vars = json!({"s": site, "limit": PAGE, "offset": offset, "sec": sections});
        if let Some(since) = creds.since {
            vars["upd"] = json!(format!(">= {}", since.to_rfc3339()));
        }
        let d = api
            .gql(
                "query($s: [String], $limit: Int, $offset: Int, $sec: [String], $upd: [String]) {
                   entries(site: $s, limit: $limit, offset: $offset, section: $sec, dateUpdated: $upd, orderBy: \"id asc\") {
                     id __typename title sectionHandle typeHandle } }",
                vars.clone(),
            )
            .await?;
        let list = d["entries"].as_array().cloned().unwrap_or_default();
        let mut fields_of: HashMap<String, Vec<String>> = HashMap::new();
        let values = typed_values(&api, &vars["s"], &list, &mut fields_of).await?;
        let mut items = vec![];
        for e in &list {
            let (Some(id), Some(ty)) = (cms::id_str(&e["id"]), e["__typename"].as_str()) else {
                continue;
            };
            let title = e["title"].as_str().unwrap_or("");
            let meta = json!({"id": id, "section": e["sectionHandle"], "type": e["typeHandle"]});
            let mut push = |f: &str, v: &str| {
                if cms::has_letter(v) {
                    items.push(SourceItem {
                        external_id: format!("entry:{id}"),
                        field: f.to_string(),
                        key: key("craft", "entry", &id, f),
                        text: v.to_string(),
                        format: if looks_html(v) {
                            Format::Html
                        } else {
                            Format::Plain
                        },
                        context: format!(
                            "Craft entry \u{ab}{title}\u{bb} ({}), field {f}",
                            e["sectionHandle"].as_str().unwrap_or("")
                        ),
                        updated_at: None,
                        meta: meta.clone(),
                    })
                }
            };
            push("title", title);
            let full = values.get(&id).cloned().unwrap_or(Value::Null);
            for f in fields_of.get(ty).into_iter().flatten() {
                if let Some(v) = full[f].as_str() {
                    push(f, v);
                }
            }
        }
        let next = (list.len() == PAGE).then(|| (offset + PAGE).to_string());
        Ok(PullPage { items, next })
    }

    async fn push(
        &self,
        creds: &Creds,
        locale: &str,
        items: &[PushItem],
    ) -> anyhow::Result<PushReport> {
        let api = Api::new(creds).await?;
        let site = site_for(creds, locale);
        let mut by: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by {
            let meta = &its[0].meta;
            let r: anyhow::Result<()> = async {
                let (Some(id), Some(sec), Some(ty)) =
                    (meta["id"].as_str(), meta["section"].as_str(), meta["type"].as_str())
                else {
                    anyhow::bail!("missing metadata, sync again");
                };
                anyhow::ensure!(ident_ok(sec) && ident_ok(ty), "invalid section or entry type handle");
                let t = api
                    .gql(
                        "query($id: [QueryArgument], $s: [String]) { entry(id: $id, site: $s) { siteId } }",
                        json!({"id": [id], "s": [site]}),
                    )
                    .await?;
                let site_id = t["entry"]["siteId"].clone();
                anyhow::ensure!(
                    !site_id.is_null(),
                    "entry not enabled in the Craft site '{site}' (enable the section there, or map {locale} in settings.sites)"
                );
                let mut decl = vec!["$id: ID".to_string(), "$siteId: Int".to_string()];
                let mut args = vec!["id: $id".to_string(), "siteId: $siteId".to_string()];
                let mut vars = Map::new();
                vars.insert("id".into(), json!(id));
                vars.insert("siteId".into(), site_id);
                for it in &its {
                    anyhow::ensure!(ident_ok(&it.field), "invalid field handle");
                    decl.push(format!("${}: String", it.field));
                    args.push(format!("{0}: ${0}", it.field));
                    vars.insert(it.field.clone(), Value::String(it.value.clone()));
                }
                let q = format!(
                    "mutation({}) {{ save_{sec}_{ty}_Entry({}) {{ id }} }}",
                    decl.join(", "),
                    args.join(", ")
                );
                api.gql(&q, Value::Object(vars)).await?;
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
        let mut ids: Vec<&str> = items.iter().filter_map(|i| i.meta["id"].as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut fields_of = HashMap::new();
        let site = json!([site_for(creds, locale)]);
        let there = entries_in(&api, &site, &ids, &mut fields_of).await?;
        let source_site = json!(source_site(creds).map(|s| vec![s]));
        let source = entries_in(&api, &source_site, &ids, &mut fields_of).await?;
        Ok(Some(
            items
                .iter()
                .map(|it| {
                    let id = it.meta["id"].as_str()?;
                    let v = there.get(id)?[it.field.as_str()].as_str()?;
                    // Still the content propagated from the source site: not translated.
                    let propagated =
                        source.get(id).and_then(|e| e[it.field.as_str()].as_str()) == Some(v);
                    (!v.trim().is_empty() && !propagated).then(|| v.to_string())
                })
                .collect(),
        ))
    }

    async fn doctor(&self, creds: &Creds) -> Vec<Check> {
        let api = match Api::new(creds).await {
            Ok(a) => a,
            Err(e) => return vec![Check::fail("sites", format!("{e:#}"))],
        };
        let mut out = vec![];
        let sites = cms::pairs(creds, "sites");
        let mut empty = vec![];
        for (locale, handle) in &sites {
            let r = api
                .gql(
                    "query($s: [String]) { entries(site: $s, limit: 1) { siteId } }",
                    json!({"s": [handle]}),
                )
                .await;
            if !r.is_ok_and(|d| d["entries"].as_array().is_some_and(|a| !a.is_empty())) {
                empty.push(format!("{handle} ({locale})"));
            }
        }
        out.push(if sites.is_empty() {
            Check::warn(
                "sites",
                "no locale → site mapping (settings.sites): Craft needs one site per language",
                Some("enable_locale"),
            )
        } else if !empty.is_empty() {
            Check::warn(
                "sites",
                format!(
                    "no entry readable in the site: {} (create the site, enable the sections in it and allow it in the token's schema)",
                    empty.join(", ")
                ),
                Some("enable_locale"),
            )
        } else {
            Check::ok("sites", sites.values().cloned().collect::<Vec<_>>().join(", "))
        });
        out.push(
            match api
                .gql("{ __schema { mutationType { fields { name } } } }", json!({}))
                .await
            {
                Ok(d) => {
                    let n = d["__schema"]["mutationType"]["fields"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f["name"].as_str())
                        .filter(|n| n.starts_with("save_") && n.ends_with("_Entry"))
                        .count();
                    if n == 0 {
                        Check {
                            fix: Some("permissions"),
                            ..Check::fail(
                                "mutations",
                                "the token's GraphQL schema allows no entry mutation: allow saving entries of the sections to translate (GraphQL → Schemas)",
                            )
                        }
                    } else {
                        Check::ok("mutations", format!("{n} entry mutation(s)"))
                    }
                }
                Err(e) => Check::warn("mutations", format!("schema not readable: {e:#}"), None),
            },
        );
        out
    }

    fn verify_webhook(&self, headers: &HeaderMap, _body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
    }

    fn push_batch(&self) -> usize {
        25
    }
}
