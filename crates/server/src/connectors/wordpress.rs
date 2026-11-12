//! WordPress: REST API (`/wp-json/wp/v2`) with an application password (HTTP Basic `user:app password`).
//!
//! - Pull (`context=edit`, raw values): posts and pages (`title`, `content`, `excerpt`: Gutenberg block HTML,
//!   block comments `<!-- wp:… -->` kept byte for byte), categories and tags (`name`, `description`), menu
//!   items (`title`, opt-in: `settings.menus = true`). Paginated by `page`/`per_page` (`X-WP-TotalPages`),
//!   incremental with `modified_after` (posts, pages).
//! - Languages: the multilingual plugin is detected from the REST index (`namespaces`) or forced with
//!   `settings.plugin` (`polylang` | `wpml`).
//!   - Polylang Pro (REST API is Pro only): `GET /pll/v1/languages`; content has `lang` and `translations`
//!     (`{"en": 12, "fr": 14}`); a translation is updated in place, or created with
//!     `?lang=fr&translations[en]=12` (linked to the source). https://polylang.pro/doc/rest-api/
//!   - WPML: `lang` query parameter, `wpml_translations` on content; a missing translation is created with
//!     `?lang=fr&translation_of=12` (documented by WPML for WooCommerce Multilingual endpoints; for core posts
//!     WPML may need the translation created once in WPML, glot then keeps it updated).
//!   - No plugin: nothing to write translations into: the push fails with an explicit message.
//! - New translations are created with `settings.new_status` (default `draft`; `same` = the source's status).
//! - Human edits and rollback ([`read_back`]): the sources are re-read by id (`include`, fresh translation
//!   links), then their translations (`include` again; an id the list leaves out is read alone). The raw
//!   field of the translated object is the current translation; an empty value, or one identical to the
//!   source (a duplicated post nobody translated), counts as none. Nothing is ever deleted: a translation is
//!   a whole post or term that may hold more than what glot wrote, so `remove` is not implemented.
//! - Diagnosis: capabilities of the application password's user (`GET /wp/v2/users/me?context=edit`:
//!   `edit_posts`, `edit_others_posts`, `edit_pages`, `edit_others_pages`, then `publish_*` and
//!   `manage_categories`), and the multilingual plugin: without a REST one, the active plugins
//!   (`GET /wp/v2/plugins?status=active`, needs `activate_plugins`) tell free Polylang (no REST API) apart.
//!
//! Docs: https://developer.wordpress.org/rest-api/reference/posts/ (`include`, `status`),
//! https://developer.wordpress.org/rest-api/reference/users/ (`capabilities`, context `edit`),
//! https://developer.wordpress.org/rest-api/reference/plugins/,
//! https://developer.wordpress.org/rest-api/using-the-rest-api/authentication/#basic-authentication-with-application-passwords
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{
    AccountInfo, Check, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem,
};

pub struct WordPress;

const PER_PAGE: u32 = 50;

/// (REST collection, fields with their format).
const RESOURCES: &[(&str, &[(&str, bool)])] = &[
    (
        "posts",
        &[("title", false), ("content", true), ("excerpt", true)],
    ),
    (
        "pages",
        &[("title", false), ("content", true), ("excerpt", true)],
    ),
    ("categories", &[("name", false), ("description", true)]),
    ("tags", &[("name", false), ("description", true)]),
    ("menu-items", &[("title", false)]),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Plugin {
    Polylang,
    Wpml,
    None,
}

pub(crate) struct Site {
    pub(crate) base: String,
    pub(crate) auth: String,
    pub(crate) client: reqwest::Client,
}

impl Site {
    async fn new(creds: &Creds) -> anyhow::Result<Self> {
        let base = creds.get("url")?.trim_end_matches('/').to_string();
        let base = base
            .strip_suffix("/wp-json")
            .map(str::to_string)
            .unwrap_or(base);
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!(
                "{}:{}",
                creds.get("username")?,
                // Application passwords are shown with spaces; both forms are accepted by WordPress.
                creds.get("app_password")?.replace(' ', "")
            ))
        );
        let client = super::client_for(&base).await?;
        Ok(Self { base, auth, client })
    }

    pub(crate) async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(Value, HeaderMap)> {
        let url = format!("{}/wp-json{path}", self.base);
        let res = super::send(|| {
            let r = self
                .client
                .request(method.clone(), &url)
                .header("Authorization", &self.auth);
            match body {
                Some(b) => r.json(b),
                None => r,
            }
        })
        .await?;
        let headers = res.headers().clone();
        Ok((res.json().await?, headers))
    }

    pub(crate) async fn get(&self, path: &str) -> anyhow::Result<(Value, HeaderMap)> {
        self.call(reqwest::Method::GET, path, None).await
    }

    /// Objects of a collection (`/wp/v2/posts`, `/wc/v3/products`) by id: one `include` request per 100,
    /// then one GET per id the list left out (another language under WPML, a status the list hides).
    pub(crate) async fn by_ids(
        &self,
        collection: &str,
        ids: &[i64],
    ) -> anyhow::Result<BTreeMap<i64, Value>> {
        // Posts and pages list published content only by default; WooCommerce products list every status.
        let status = if collection.starts_with("/wp/v2/p") {
            "&status=publish,future,draft,pending,private"
        } else {
            ""
        };
        let mut out = BTreeMap::new();
        for chunk in ids.chunks(100) {
            let list: Vec<String> = chunk.iter().map(i64::to_string).collect();
            let q = format!(
                "{collection}?context=edit&per_page=100&include={}{status}",
                list.join(",")
            );
            // A refused list is not fatal: every id is then read alone.
            if let Ok((v, _)) = self.get(&q).await {
                for o in v.as_array().into_iter().flatten() {
                    if let Some(id) = o["id"].as_i64().filter(|id| chunk.contains(id)) {
                        out.insert(id, o.clone());
                    }
                }
            }
            for id in chunk {
                if !out.contains_key(id) {
                    let (o, _) = self.get(&format!("{collection}/{id}?context=edit")).await?;
                    out.insert(*id, o);
                }
            }
        }
        Ok(out)
    }

    /// Active plugins that matter (`plugin` file, lowercased), from a plugin list (`wp/v2/plugins` or
    /// WooCommerce's `system_status.active_plugins`: both have `plugin`).
    pub(crate) fn plugin_files(list: &Value) -> Vec<String> {
        list.as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["plugin"].as_str())
            .map(str::to_ascii_lowercase)
            .collect()
    }

    pub(crate) async fn plugin(&self, creds: &Creds) -> anyhow::Result<Plugin> {
        match creds.setting("plugin").and_then(Value::as_str) {
            Some("polylang") => return Ok(Plugin::Polylang),
            Some("wpml") => return Ok(Plugin::Wpml),
            Some("none") => return Ok(Plugin::None),
            _ => {}
        }
        let (index, _) = self.get("/").await?;
        let ns: Vec<&str> = index["namespaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        Ok(if ns.iter().any(|n| n.starts_with("pll/")) {
            Plugin::Polylang
        } else if ns.iter().any(|n| n.contains("wpml")) {
            Plugin::Wpml
        } else {
            Plugin::None
        })
    }

    /// (language codes, source language).
    pub(crate) async fn languages(
        &self,
        creds: &Creds,
        plugin: Plugin,
    ) -> anyhow::Result<(Vec<String>, Option<String>)> {
        let forced = creds
            .setting("source_lang")
            .and_then(Value::as_str)
            .map(str::to_string);
        match plugin {
            Plugin::Polylang => {
                let (l, _) = self.get("/pll/v1/languages").await?;
                let langs: Vec<&Value> = l.as_array().into_iter().flatten().collect();
                let codes = langs
                    .iter()
                    .filter_map(|x| x["slug"].as_str().map(str::to_string))
                    .collect::<Vec<_>>();
                let default = langs
                    .iter()
                    .find(|x| x["is_default"] == true)
                    .and_then(|x| x["slug"].as_str())
                    .map(str::to_string)
                    .or_else(|| codes.first().cloned());
                Ok((codes, forced.or(default)))
            }
            _ => {
                let codes = creds
                    .setting("languages")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                Ok((codes, forced))
            }
        }
    }
}

/// `translations` of a content object, whatever the plugin: language code → id.
pub(crate) fn translations(o: &Value) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    if let Some(m) = o["translations"].as_object() {
        for (l, id) in m {
            if let Some(id) = id
                .as_i64()
                .or_else(|| id.as_str().and_then(|s| s.parse().ok()))
            {
                out.insert(l.clone(), id);
            }
        }
    }
    // WPML: [{"locale": "fr_FR", "id": 14, …}] (or an object keyed by language).
    match &o["wpml_translations"] {
        Value::Array(a) => {
            for t in a {
                if let (Some(l), Some(id)) = (t["locale"].as_str(), t["id"].as_i64()) {
                    out.insert(l.to_string(), id);
                }
            }
        }
        Value::Object(m) => {
            for (l, t) in m {
                if let Some(id) = t["id"].as_i64().or(t.as_i64()) {
                    out.insert(l.clone(), id);
                }
            }
        }
        _ => {}
    }
    out
}

/// The translation id for `locale` (`fr` matches `fr`, `fr_FR`, `fr-FR`).
pub(crate) fn translation_for(map: &BTreeMap<String, i64>, locale: &str) -> Option<i64> {
    let base = |s: &str| {
        s.split(['_', '-'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    map.get(locale).copied().or_else(|| {
        map.iter()
            .find(|(l, _)| {
                base(l) == base(locale) && (!l.contains(['_', '-']) || !locale.contains(['_', '-']))
            })
            .map(|(_, id)| *id)
    })
}

/// Raw value of a field (`{"raw": …, "rendered": …}` with `context=edit`, a string for terms).
pub(crate) fn raw(o: &Value, field: &str) -> Option<String> {
    let v = &o[field];
    v["raw"].as_str().or(v.as_str()).map(str::to_string)
}

/// What the site holds for `items` in `locale` (see the module doc), in the same order. `place` gives the
/// REST collection and id of an item's source from its meta. Shared with WooCommerce.
pub(crate) async fn read_back(
    site: &Site,
    locale: &str,
    items: &[PushItem],
    place: impl Fn(&Value) -> Option<(String, i64)>,
) -> anyhow::Result<Vec<Option<String>>> {
    let places: Vec<Option<(String, i64)>> = items.iter().map(|i| place(&i.meta)).collect();
    let mut by_collection: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (c, id) in places.iter().flatten() {
        let ids = by_collection.entry(c).or_default();
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    // (collection, source id) → (source, translation).
    let mut found: BTreeMap<(&str, i64), (Value, Value)> = BTreeMap::new();
    for (c, ids) in by_collection {
        let mut sources = site.by_ids(c, &ids).await?;
        let links: Vec<(i64, i64)> = sources
            .iter()
            .filter_map(|(id, o)| {
                let t = translation_for(&translations(o), locale).filter(|t| t != id)?;
                Some((*id, t))
            })
            .collect();
        let targets: Vec<i64> = links.iter().map(|l| l.1).collect();
        let mut translated = site.by_ids(c, &targets).await?;
        for (id, t) in links {
            if let (Some(s), Some(o)) = (sources.remove(&id), translated.remove(&t)) {
                found.insert((c, id), (s, o));
            }
        }
    }
    Ok(items
        .iter()
        .zip(&places)
        .map(|(it, p)| {
            let (c, id) = p.as_ref()?;
            let (source, tr) = found.get(&(c.as_str(), *id))?;
            raw(tr, &it.field)
                .filter(|v| !v.trim().is_empty() && Some(v) != raw(source, &it.field).as_ref())
        })
        .collect())
}

/// The multilingual plugin as a diagnosis line; `plugins` = active plugin files when they could be listed.
/// `woo` = the WooCommerce companion plugin is needed too.
pub(crate) fn plugin_check(plugin: Plugin, plugins: Option<&[String]>, woo: bool) -> Check {
    let has = |slug: &str| plugins.is_some_and(|l| l.iter().any(|p| p.starts_with(slug)));
    let missing = |detail: &str| Check {
        fix: Some("install_plugin"),
        ..Check::fail("plugin", detail)
    };
    let (wpml, pll) = if woo {
        (
            "WPML with WooCommerce Multilingual",
            "Polylang Pro with Polylang for WooCommerce",
        )
    } else {
        ("WPML", "Polylang Pro")
    };
    match plugin {
        Plugin::Wpml if woo && plugins.is_some() && !has("woocommerce-multilingual/") => {
            Check::warn(
                "plugin",
                "WPML is active without WooCommerce Multilingual: install WooCommerce Multilingual",
                Some("install_plugin"),
            )
        }
        Plugin::Polylang if woo && plugins.is_some() && !has("polylang-wc/") => Check::warn(
            "plugin",
            "Polylang Pro is active without Polylang for WooCommerce: install Polylang for WooCommerce",
            Some("install_plugin"),
        ),
        Plugin::Wpml => Check::ok("plugin", wpml),
        Plugin::Polylang => Check::ok("plugin", pll),
        Plugin::None if has("sitepress-multilingual-cms/") => Check::warn(
            "plugin",
            "WPML is active but absent from the REST index: set the connection's plugin setting to wpml",
            None,
        ),
        Plugin::None if has("polylang/") => missing(&format!(
            "Polylang (free) is active but has no REST API: install {pll}, or {wpml}"
        )),
        Plugin::None => missing(&format!(
            "no multilingual plugin with a REST API: install {pll}, or {wpml}"
        )),
    }
}

fn items_of(kind: &str, fields: &[(&str, bool)], o: &Value) -> Vec<SourceItem> {
    let Some(id) = o["id"].as_i64() else {
        return vec![];
    };
    let title = raw(o, "title")
        .or_else(|| raw(o, "name"))
        .unwrap_or_default();
    let what = kind.trim_end_matches('s').replace("-item", " item");
    fields
        .iter()
        .filter_map(|(f, html)| {
            let v = raw(o, f)?;
            if v.trim().is_empty() {
                return None;
            }
            Some(SourceItem {
                external_id: format!("{kind}:{id}"),
                field: f.to_string(),
                key: super::key("wordpress", kind, &id.to_string(), f),
                text: v,
                format: if *html { Format::Html } else { Format::Plain },
                context: format!("WordPress {what} \u{ab}{title}\u{bb}, field {f}"),
                updated_at: None,
                meta: json!({"type": kind, "id": id, "status": o["status"], "translations": translations(o)}),
            })
        })
        .collect()
}

fn resources(creds: &Creds) -> Vec<(&'static str, &'static [(&'static str, bool)])> {
    let menus = creds.setting("menus") == Some(&Value::Bool(true));
    RESOURCES
        .iter()
        .filter(|(k, _)| *k != "menu-items" || menus)
        .copied()
        .collect()
}

#[async_trait]
impl Connector for WordPress {
    fn kind(&self) -> &'static str {
        "wordpress"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let site = Site::new(creds).await?;
        let (index, _) = site.get("/").await?;
        // Authenticated call: a wrong password fails here (the index is public).
        site.get("/wp/v2/users/me?context=edit").await?;
        let plugin = site.plugin(creds).await?;
        let (langs, src) = site.languages(creds, plugin).await?;
        let mut notes = vec![match plugin {
            Plugin::Polylang => format!("Polylang detected, languages: {}", langs.join(", ")),
            Plugin::Wpml => "WPML detected".to_string(),
            Plugin::None => "No multilingual plugin with a REST API (Polylang Pro or WPML) detected: translations cannot be written to WordPress.".to_string(),
        }];
        if plugin == Plugin::Wpml && langs.is_empty() {
            notes.push(
                "Set settings.languages and settings.source_lang (WPML has no languages endpoint)."
                    .into(),
            );
        }
        Ok(AccountInfo {
            name: index["name"].as_str().unwrap_or("WordPress").to_string(),
            source_locale: src,
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let site = Site::new(creds).await?;
        let plugin = site.plugin(creds).await?;
        Ok(site.languages(creds, plugin).await?.0)
    }

    /// Cursor = `{resource index}|{page}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let site = Site::new(creds).await?;
        let plugin = site.plugin(creds).await?;
        let (_, src) = site.languages(creds, plugin).await?;
        let res = resources(creds);
        let (i, page) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, p)) => (i.parse::<usize>()?, p.parse::<u32>()?),
            None => (0, 1),
        };
        let Some((kind, fields)) = res.get(i).copied() else {
            return Ok(PullPage::default());
        };
        let mut q = format!(
            "/wp/v2/{kind}?context=edit&per_page={PER_PAGE}&page={page}&orderby=id&order=asc"
        );
        if matches!(kind, "posts" | "pages") {
            q.push_str("&status=publish,future,draft,pending,private");
            if let Some(since) = creds.since {
                q.push_str(&format!(
                    "&modified_after={}",
                    since.format("%Y-%m-%dT%H:%M:%S")
                ));
            }
        }
        if let (Some(l), true) = (&src, plugin != Plugin::None && kind != "menu-items") {
            q.push_str(&format!("&lang={l}"));
        }
        let (list, headers) = site.get(&q).await?;
        let total: u32 = headers
            .get("x-wp-totalpages")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let items = list
            .as_array()
            .into_iter()
            .flatten()
            // Belt and braces: never pull a translation as a source.
            .filter(|o| match (&src, o["lang"].as_str()) {
                (Some(s), Some(l)) => l == s,
                _ => true,
            })
            .flat_map(|o| items_of(kind, fields, o))
            .collect();
        let next = if page < total {
            Some(format!("{i}|{}", page + 1))
        } else if i + 1 < res.len() {
            Some(format!("{}|1", i + 1))
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
        let site = Site::new(creds).await?;
        let plugin = site.plugin(creds).await?;
        anyhow::ensure!(
            plugin != Plugin::None,
            "no multilingual plugin (Polylang Pro or WPML) on this site: translations cannot be stored"
        );
        let (_, src) = site.languages(creds, plugin).await?;
        let new_status = creds
            .setting("new_status")
            .and_then(Value::as_str)
            .unwrap_or("draft");
        let mut by_resource: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_resource.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by_resource {
            let meta = &its[0].meta;
            let (Some(kind), Some(id)) = (meta["type"].as_str(), meta["id"].as_i64()) else {
                report
                    .failed
                    .push((ext.to_string(), "missing metadata, sync again".into()));
                continue;
            };
            let mut body = serde_json::Map::new();
            for it in &its {
                body.insert(it.field.clone(), Value::String(it.value.clone()));
            }
            let res: anyhow::Result<()> = async {
                // Fresh translation links (a translation created by an earlier batch is not in the pulled meta).
                let (current, _) = site.get(&format!("/wp/v2/{kind}/{id}?context=edit")).await?;
                let target = translation_for(&translations(&current), locale).filter(|t| *t != id);
                match target {
                    Some(t) => {
                        site.call(reqwest::Method::POST, &format!("/wp/v2/{kind}/{t}"), Some(&Value::Object(body.clone())))
                            .await?;
                    }
                    None => {
                        if matches!(kind, "posts" | "pages") {
                            let status = if new_status == "same" {
                                current["status"].as_str().unwrap_or("draft").to_string()
                            } else {
                                new_status.to_string()
                            };
                            body.insert("status".into(), Value::String(status));
                        }
                        if kind == "menu-items" {
                            anyhow::bail!("menu item has no {locale} translation: create the {locale} menu in the plugin first");
                        }
                        let link = match plugin {
                            Plugin::Polylang => {
                                let s = src.clone().or_else(|| current["lang"].as_str().map(str::to_string)).unwrap_or_default();
                                format!("translations[{s}]={id}")
                            }
                            _ => format!("translation_of={id}"),
                        };
                        site.call(
                            reqwest::Method::POST,
                            &format!("/wp/v2/{kind}?lang={locale}&{link}"),
                            Some(&Value::Object(body.clone())),
                        )
                        .await?;
                    }
                }
                Ok(())
            }
            .await;
            match res {
                Ok(()) => report.ok += its.len(),
                Err(e) => {
                    for it in &its {
                        report
                            .failed
                            .push((format!("{}/{}", it.external_id, it.field), format!("{e:#}")));
                    }
                }
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
        let site = Site::new(creds).await?;
        let place =
            |m: &Value| Some((format!("/wp/v2/{}", m["type"].as_str()?), m["id"].as_i64()?));
        Ok(Some(read_back(&site, locale, items, place).await?))
    }

    async fn doctor(&self, creds: &Creds) -> Vec<Check> {
        let site = match Site::new(creds).await {
            Ok(s) => s,
            Err(e) => return vec![Check::fail("permissions", format!("{e:#}"))],
        };
        let mut out = vec![];
        match site.get("/wp/v2/users/me?context=edit").await {
            Ok((me, _)) => {
                let lacks = |caps: &[&'static str]| -> Vec<&'static str> {
                    caps.iter()
                        .copied()
                        .filter(|c| me["capabilities"][*c] != true)
                        .collect()
                };
                let edit = lacks(&[
                    "edit_posts",
                    "edit_others_posts",
                    "edit_pages",
                    "edit_others_pages",
                ]);
                let more = lacks(&["publish_posts", "publish_pages", "manage_categories"]);
                if !edit.is_empty() {
                    out.push(Check {
                        fix: Some("permissions"),
                        ..Check::fail(
                            "permissions",
                            format!(
                                "the user lacks {}: use an Editor or Administrator",
                                edit.join(", ")
                            ),
                        )
                    });
                } else if !more.is_empty() {
                    out.push(Check::warn(
                        "permissions",
                        format!("the user lacks {}", more.join(", ")),
                        Some("permissions"),
                    ));
                } else {
                    out.push(Check::ok(
                        "permissions",
                        "edit and publish posts, pages, terms",
                    ));
                }
            }
            Err(e) => out.push(Check::fail("permissions", format!("{e:#}"))),
        }
        match site.plugin(creds).await {
            Ok(plugin) => {
                let plugins = match plugin {
                    // Only worth a call when there is something to explain.
                    Plugin::None => site.get("/wp/v2/plugins?status=active").await.ok(),
                    _ => None,
                }
                .map(|(l, _)| Site::plugin_files(&l));
                out.push(plugin_check(plugin, plugins.as_deref(), false));
            }
            Err(e) => out.push(Check::fail("plugin", format!("{e:#}"))),
        }
        out
    }

    /// WordPress core has no webhooks: plugins (WP Webhooks, a `save_post` hook…) call the account's webhook
    /// URL with its token, or send `X-Glot-Signature: sha256=<hex HMAC of the body>` with that token as key.
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        headers
            .get("x-glot-signature")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|sig| super::hmac_ok(secret, body, sig))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_maps() {
        let pll = json!({"translations": {"en": 12, "fr": 14}});
        assert_eq!(translation_for(&translations(&pll), "fr"), Some(14));
        let wpml = json!({"wpml_translations": [{"locale": "fr_FR", "id": 20, "post_title": "x"}]});
        assert_eq!(translation_for(&translations(&wpml), "fr"), Some(20));
        assert_eq!(translation_for(&translations(&wpml), "de"), None);
    }

    #[test]
    fn post_fields() {
        let o = json!({"id": 7, "status": "publish", "lang": "en",
            "title": {"raw": "Hello", "rendered": "Hello"},
            "content": {"raw": "<!-- wp:paragraph -->\n<p>Hi</p>\n<!-- /wp:paragraph -->", "rendered": "<p>Hi</p>"},
            "excerpt": {"raw": "", "rendered": ""}});
        let items = items_of("posts", RESOURCES[0].1, &o);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "wordpress:posts:7:title");
        assert_eq!(items[1].format, Format::Html);
        assert!(items[1].text.contains("<!-- wp:paragraph -->"));
    }
}
