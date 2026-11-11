//! PrestaShop: Webservice API (`/api/…`, PrestaShop 1.7 / 8 / 9) with a webservice key (HTTP Basic, key as
//! user name, empty password).
//!
//! - Pull (`output_format=JSON&display=full`, paginated with `limit=offset,count`): products, categories,
//!   CMS pages (`content_management_system`) and brands (`manufacturers`). Multilingual fields come as
//!   `[{"id": "1", "value": "…"}]`; only the shop's default language (`PS_LANG_DEFAULT`, else
//!   `settings.source_lang`) is read. Descriptions and CMS content are HTML. URL slugs (`link_rewrite`) and
//!   keywords are skipped. Incremental with `filter[date_upd]` (products, categories, brands).
//! - Push: the Webservice reads XML only ("only able to output JSON, it cannot read JSON inputs") and PATCH
//!   exists from PrestaShop 8 only, so glot GETs the resource as XML, replaces the target language's
//!   `<language id="N">` node of each field (every other node kept byte for byte), drops the read-only fields
//!   (`manufacturer_name`, `quantity`, `level_depth`, `nb_products_recursive`) and PUTs the document back.
//! - Languages: `/api/languages` (`iso_code`, `language_code`, `active`); a glot locale matches
//!   `language_code` (`pt-BR` = `pt-br`) else `iso_code` (`fr` = `fr-FR`).
//! - Webhooks: none in PrestaShop; a module calls the card's token URL, or signs the body with
//!   `X-Glot-Signature: sha256=<hex HMAC>` (token as key), as for WordPress.
//!
//! Docs: https://devdocs.prestashop-project.org/8/webservice/getting-started/,
//! https://devdocs.prestashop-project.org/8/webservice/cheat-sheet/,
//! https://devdocs.prestashop-project.org/8/webservice/resources/products/,
//! https://devdocs.prestashop-project.org/8/webservice/resources/categories/,
//! https://devdocs.prestashop-project.org/8/webservice/resources/content_management_system/,
//! https://devdocs.prestashop-project.org/8/webservice/resources/languages/,
//! https://devdocs.prestashop-project.org/8/webservice/tutorials/prestashop-webservice-lib/update-resource/
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{
    AccountInfo, Connector, Creds, Format, PullPage, PushItem, PushReport, SourceItem, cms,
};

pub struct PrestaShop;

const PAGE: usize = 50;

/// (API resource, XML element of one item, fields with `true` = HTML, has `date_upd`).
type Resource = (
    &'static str,
    &'static str,
    &'static [(&'static str, bool)],
    bool,
);
const RESOURCES: &[Resource] = &[
    (
        "products",
        "product",
        &[
            ("name", false),
            ("description_short", true),
            ("description", true),
            ("meta_title", false),
            ("meta_description", false),
        ],
        true,
    ),
    (
        "categories",
        "category",
        &[
            ("name", false),
            ("description", true),
            ("meta_title", false),
            ("meta_description", false),
        ],
        true,
    ),
    (
        "content_management_system",
        "content",
        &[
            ("meta_title", false),
            ("head_seo_title", false),
            ("meta_description", false),
            ("content", true),
        ],
        false,
    ),
    (
        "manufacturers",
        "manufacturer",
        &[
            ("short_description", true),
            ("description", true),
            ("meta_title", false),
            ("meta_description", false),
        ],
        true,
    ),
];

/// Not writable through the Webservice: a PUT carrying them is refused.
const READ_ONLY: &[&str] = &[
    "manufacturer_name",
    "quantity",
    "level_depth",
    "nb_products_recursive",
];

struct Shop {
    base: String,
    auth: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct Lang {
    id: String,
    iso: String,
    code: String,
    active: bool,
}

/// Ids come as numbers or strings depending on the version.
fn id_str(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::to_string)
        .or_else(|| v.as_i64().map(|i| i.to_string()))
}

impl Shop {
    async fn open(creds: &Creds) -> anyhow::Result<Self> {
        let base = creds.get("url")?.trim_end_matches('/');
        let base = base.strip_suffix("/api").unwrap_or(base).to_string();
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{}:", creds.get("api_key")?))
        );
        let client = super::client_for(&base).await?;
        Ok(Self { base, auth, client })
    }

    async fn json(&self, path: &str) -> anyhow::Result<Value> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!("{}/api/{path}{sep}output_format=JSON", self.base);
        let res = super::send(|| self.client.get(&url).header("Authorization", &self.auth)).await?;
        // An empty list is `[]` instead of `{"products": []}`.
        Ok(res.json().await.unwrap_or(Value::Null))
    }

    async fn xml(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&str>,
    ) -> anyhow::Result<String> {
        let url = format!("{}/api/{path}", self.base);
        let res = super::send(|| {
            let r = self
                .client
                .request(method.clone(), &url)
                .header("Authorization", &self.auth);
            match body {
                Some(b) => r.header("Content-Type", "text/xml").body(b.to_string()),
                None => r,
            }
        })
        .await?;
        Ok(res.text().await?)
    }

    async fn languages(&self) -> anyhow::Result<Vec<Lang>> {
        let v = self.json("languages?display=full").await?;
        Ok(v["languages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                Some(Lang {
                    id: id_str(&l["id"])?,
                    iso: l["iso_code"].as_str()?.to_ascii_lowercase(),
                    code: l["language_code"]
                        .as_str()
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                    active: matches!(&l["active"], Value::String(s) if s == "1")
                        || l["active"] == 1
                        || l["active"] == true,
                })
            })
            .collect())
    }

    /// `PS_LANG_DEFAULT` and `PS_SHOP_NAME`; empty when the key has no access to `configurations`.
    async fn config(&self) -> BTreeMap<String, String> {
        let v = self
            .json("configurations?display=[name,value]&filter[name]=[PS_LANG_DEFAULT|PS_SHOP_NAME]")
            .await
            .unwrap_or(Value::Null);
        v["configurations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c["name"].as_str()?.to_string(), id_str(&c["value"])?)))
            .collect()
    }

    /// The source language: `settings.source_lang` (iso code), else `PS_LANG_DEFAULT`, else the first active one.
    async fn source(&self, creds: &Creds, langs: &[Lang]) -> anyhow::Result<Lang> {
        let forced = cms::param(creds, "source_lang").map(str::to_ascii_lowercase);
        let default = if forced.is_none() {
            self.config().await.remove("PS_LANG_DEFAULT")
        } else {
            None
        };
        langs
            .iter()
            .find(|l| match (&forced, &default) {
                (Some(f), _) => &l.iso == f || &l.code == f,
                (None, Some(d)) => &l.id == d,
                _ => false,
            })
            .or_else(|| langs.iter().find(|l| l.active))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no language found in PrestaShop"))
    }
}

/// The PrestaShop language of a glot/CMS locale: `language_code` first (`pt-br`), then `iso_code` (`fr`).
fn lang_for<'a>(langs: &'a [Lang], locale: &str) -> Option<&'a Lang> {
    let l = locale.to_ascii_lowercase().replace('_', "-");
    let base = l.split('-').next().unwrap_or("");
    langs
        .iter()
        .find(|x| x.code == l)
        .or_else(|| langs.iter().find(|x| x.iso == l))
        .or_else(|| langs.iter().find(|x| x.iso == base))
}

/// Value of a multilingual field in language `lang` (`[{"id","value"}]`, or a plain string).
fn ml(v: &Value, lang: &str) -> Option<String> {
    match v {
        Value::Array(a) => a
            .iter()
            .find(|x| id_str(&x["id"]).as_deref() == Some(lang))
            .and_then(|x| x["value"].as_str())
            .map(str::to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn items_of(res: &Resource, o: &Value, lang: &str) -> Vec<SourceItem> {
    let (resource, element, fields, _) = *res;
    let Some(id) = id_str(&o["id"]) else {
        return vec![];
    };
    // The invisible root category (id_parent 0) is not shown anywhere.
    if resource == "categories" && id_str(&o["id_parent"]).as_deref() == Some("0") {
        return vec![];
    }
    let title = ["name", "meta_title"]
        .iter()
        .find_map(|f| ml(&o[*f], lang).filter(|s| !s.is_empty()))
        .or_else(|| o["name"].as_str().map(str::to_string))
        .unwrap_or_default();
    let what = match element {
        "content" => "CMS page",
        "manufacturer" => "brand",
        e => e,
    };
    fields
        .iter()
        .filter_map(|(f, html)| {
            let v = ml(&o[*f], lang)?;
            if v.trim().is_empty() {
                return None;
            }
            Some(SourceItem {
                external_id: format!("{resource}:{id}"),
                field: f.to_string(),
                key: super::key("prestashop", resource, &id, f),
                text: v,
                format: if *html { Format::Html } else { Format::Plain },
                context: format!("PrestaShop {what} \u{ab}{title}\u{bb}, field {f}"),
                updated_at: None,
                meta: json!({}),
            })
        })
        .collect()
}

fn cdata(v: &str) -> String {
    format!("<![CDATA[{}]]>", v.replace("]]>", "]]]]><![CDATA[>"))
}

/// The resource XML with `<language id="{lang}">` of each field set to its value (node added when missing),
/// read-only fields removed, everything else untouched.
fn patch_xml(
    xml: &str,
    element: &str,
    lang: &str,
    values: &[(&str, &str)],
) -> anyhow::Result<String> {
    let doc = roxmltree::Document::parse(xml)?;
    let item = doc
        .root_element()
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == element)
        .ok_or_else(|| anyhow::anyhow!("unexpected XML: no <{element}>"))?;
    let node = |v: &str| format!("<language id=\"{lang}\">{}</language>", cdata(v));
    let mut edits: Vec<(std::ops::Range<usize>, String)> = vec![];
    let mut done = vec![false; values.len()];
    for child in item.children().filter(|n| n.is_element()) {
        let name = child.tag_name().name();
        let r = child.range();
        if READ_ONLY.contains(&name) {
            edits.push((r, String::new()));
            continue;
        }
        let Some(i) = values.iter().position(|(f, _)| *f == name) else {
            continue;
        };
        done[i] = true;
        let v = values[i].1;
        match child.children().find(|n| {
            n.is_element() && n.tag_name().name() == "language" && n.attribute("id") == Some(lang)
        }) {
            Some(l) => edits.push((l.range(), node(v))),
            None if xml[r.clone()].ends_with("/>") => {
                edits.push((r, format!("<{name}>{}</{name}>", node(v))))
            }
            None => {
                let at = r.end - name.len() - 3;
                edits.push((at..at, node(v)));
            }
        }
    }
    let end = item.range().end - element.len() - 3;
    for (i, (f, v)) in values.iter().enumerate() {
        if !done[i] {
            edits.push((end..end, format!("<{f}>{}</{f}>", node(v))));
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse((e.0.start, e.0.end)));
    let mut out = xml.to_string();
    for (r, s) in edits {
        out.replace_range(r, &s);
    }
    Ok(out)
}

#[async_trait]
impl Connector for PrestaShop {
    fn kind(&self) -> &'static str {
        "prestashop"
    }

    async fn test(&self, creds: &Creds) -> anyhow::Result<AccountInfo> {
        let shop = Shop::open(creds).await?;
        let langs = shop.languages().await?;
        let config = shop.config().await;
        let src = shop.source(creds, &langs).await?;
        let mut notes = vec![format!(
            "Languages: {}",
            langs
                .iter()
                .filter(|l| l.active)
                .map(|l| l.iso.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )];
        if config.is_empty() {
            notes.push(
                "The key cannot read configurations: the source language is the first active one (set settings.source_lang to change it).".into(),
            );
        }
        Ok(AccountInfo {
            name: config
                .get("PS_SHOP_NAME")
                .cloned()
                .unwrap_or_else(|| "PrestaShop".into()),
            source_locale: Some(src.iso),
            notes,
        })
    }

    async fn locales(&self, creds: &Creds) -> anyhow::Result<Vec<String>> {
        let shop = Shop::open(creds).await?;
        Ok(shop
            .languages()
            .await?
            .into_iter()
            .filter(|l| l.active)
            .map(|l| l.iso)
            .collect())
    }

    /// Cursor = `{resource index}|{offset}`.
    async fn pull(&self, creds: &Creds, cursor: Option<String>) -> anyhow::Result<PullPage> {
        let shop = Shop::open(creds).await?;
        let langs = shop.languages().await?;
        let src = shop.source(creds, &langs).await?;
        let (i, offset) = match cursor.as_deref().and_then(|c| c.split_once('|')) {
            Some((i, o)) => (i.parse::<usize>()?, o.parse::<usize>()?),
            None => (0, 0),
        };
        let Some(res) = RESOURCES.get(i) else {
            return Ok(PullPage::default());
        };
        let mut q = format!("{}?display=full&sort=[id_ASC]&limit={offset},{PAGE}", res.0);
        if let (true, Some(since)) = (res.3, creds.since) {
            q.push_str(&format!(
                "&date=1&filter[date_upd]=[{},2999-12-31 23:59:59]",
                since.format("%Y-%m-%d %H:%M:%S")
            ));
        }
        let v = shop.json(&q).await?;
        let list = v[res.0].as_array().cloned().unwrap_or_default();
        let items = list
            .iter()
            .flat_map(|o| items_of(res, o, &src.id))
            .collect();
        let next = if list.len() == PAGE {
            Some(format!("{i}|{}", offset + PAGE))
        } else if i + 1 < RESOURCES.len() {
            Some(format!("{}|0", i + 1))
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
        let shop = Shop::open(creds).await?;
        let langs = shop.languages().await?;
        let lang = lang_for(&langs, locale).ok_or_else(|| {
            anyhow::anyhow!("no PrestaShop language for {locale}: add it in International → Localization → Languages")
        })?;
        let mut by_resource: BTreeMap<&str, Vec<&PushItem>> = BTreeMap::new();
        for it in items {
            by_resource.entry(&it.external_id).or_default().push(it);
        }
        let mut report = PushReport::default();
        for (ext, its) in by_resource {
            let res: anyhow::Result<()> = async {
                let (resource, id) = ext
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("bad id {ext}"))?;
                let element = RESOURCES
                    .iter()
                    .find(|r| r.0 == resource)
                    .ok_or_else(|| anyhow::anyhow!("unknown resource {resource}"))?
                    .1;
                anyhow::ensure!(id.bytes().all(|b| b.is_ascii_digit()), "bad id {ext}");
                let path = format!("{resource}/{id}");
                let xml = shop.xml(reqwest::Method::GET, &path, None).await?;
                let values: Vec<(&str, &str)> = its
                    .iter()
                    .map(|it| (it.field.as_str(), it.value.as_str()))
                    .collect();
                let body = patch_xml(&xml, element, &lang.id, &values)?;
                shop.xml(reqwest::Method::PUT, &path, Some(&body)).await?;
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

    /// No native webhooks: a module calls the token URL, or signs with `X-Glot-Signature` / a secret header.
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
        cms::shared_secret_ok(headers, secret)
            || cms::header(headers, "x-glot-signature")
                .is_some_and(|sig| !secret.is_empty() && super::hmac_ok(secret, body, sig))
    }

    fn push_batch(&self) -> usize {
        25
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::{Path, Query, State},
        http::StatusCode,
        routing::get,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const PRODUCT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<prestashop xmlns:xlink="http://www.w3.org/1999/xlink">
<product>
	<id><![CDATA[1]]></id>
	<manufacturer_name not_filterable="true"><![CDATA[Acme]]></manufacturer_name>
	<quantity not_filterable="true"><![CDATA[12]]></quantity>
	<price><![CDATA[19.900000]]></price>
	<name><language id="1" xlink:href="http://shop.test/api/languages/1"><![CDATA[Shirt]]></language><language id="2" xlink:href="http://shop.test/api/languages/2"><![CDATA[Chemise]]></language><language id="3" xlink:href="http://shop.test/api/languages/3"><![CDATA[]]></language></name>
	<description><language id="1"><![CDATA[<p>Soft <b>cotton</b>.</p>]]></language><language id="2"><![CDATA[<p>Coton.</p>]]></language></description>
	<meta_title/>
	<associations>
		<categories nodeType="category" api="categories"><category xlink:href="http://shop.test/api/categories/2"><id><![CDATA[2]]></id></category></categories>
	</associations>
</product>
</prestashop>"#;

    #[test]
    fn xml_patch_touches_only_the_target_language() {
        let out = patch_xml(
            PRODUCT_XML,
            "product",
            "3",
            &[
                ("name", "Hemd"),
                ("description", "<p>Weiche <b>Baumwolle</b>.</p>"),
                ("meta_title", "Hemd ]]> kaufen"),
            ],
        )
        .unwrap();
        let doc = roxmltree::Document::parse(&out).unwrap();
        let get = |f: &str, l: &str| {
            doc.descendants()
                .find(|n| n.tag_name().name() == f)
                .and_then(|n| n.children().find(|c| c.attribute("id") == Some(l)))
                .and_then(|c| {
                    c.text()
                        .map(str::to_string)
                        .or_else(|| Some(c.children().filter_map(|t| t.text()).collect()))
                })
        };
        assert_eq!(get("name", "3").as_deref(), Some("Hemd"));
        assert_eq!(get("name", "1").as_deref(), Some("Shirt"));
        assert_eq!(get("name", "2").as_deref(), Some("Chemise"));
        assert_eq!(
            get("description", "3").as_deref(),
            Some("<p>Weiche <b>Baumwolle</b>.</p>")
        );
        assert_eq!(
            get("description", "1").as_deref(),
            Some("<p>Soft <b>cotton</b>.</p>")
        );
        assert_eq!(get("meta_title", "3").as_deref(), Some("Hemd ]]> kaufen"));
        assert!(!out.contains("manufacturer_name") && !out.contains("<quantity"));
        assert!(out.contains(r#"<category xlink:href="http://shop.test/api/categories/2"><id><![CDATA[2]]></id></category>"#));
        assert!(out.contains("<price><![CDATA[19.900000]]></price>"));
    }

    #[test]
    fn locale_matching() {
        let l = |id: &str, iso: &str, code: &str| Lang {
            id: id.into(),
            iso: iso.into(),
            code: code.into(),
            active: true,
        };
        let langs = [
            l("1", "en", "en-us"),
            l("2", "fr", "fr-fr"),
            l("3", "pt", "pt-br"),
        ];
        assert_eq!(lang_for(&langs, "fr").unwrap().id, "2");
        assert_eq!(lang_for(&langs, "fr-CA").unwrap().id, "2");
        assert_eq!(lang_for(&langs, "pt-BR").unwrap().id, "3");
        assert!(lang_for(&langs, "de").is_none());
    }

    #[derive(Clone, Default)]
    struct Mock {
        puts: Arc<Mutex<Vec<(String, String)>>>,
    }

    fn authed(h: &HeaderMap) -> bool {
        // Key as Basic user name, empty password.
        h.get("authorization").and_then(|v| v.to_str().ok())
            == Some(&format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("KEY123:")
            ))
    }

    async fn api(
        State(m): State<Mock>,
        h: HeaderMap,
        Path(path): Path<String>,
        Query(q): Query<HashMap<String, String>>,
        method: axum::http::Method,
        body: String,
    ) -> (StatusCode, String) {
        if !authed(&h) {
            return (StatusCode::UNAUTHORIZED, "<prestashop><errors><error><code><![CDATA[17]]></code></error></errors></prestashop>".into());
        }
        if method == axum::http::Method::PUT {
            m.puts.lock().unwrap().push((path, body));
            return (StatusCode::OK, PRODUCT_XML.into());
        }
        if q.get("output_format").map(String::as_str) != Some("JSON") {
            assert_eq!(path, "products/1");
            return (StatusCode::OK, PRODUCT_XML.into());
        }
        let v = match path.as_str() {
            "languages" => json!({"languages": [
                {"id": 1, "name": "English", "iso_code": "en", "language_code": "en-us", "active": "1"},
                {"id": 2, "name": "Français", "iso_code": "fr", "language_code": "fr-fr", "active": "1"},
                {"id": 3, "name": "Deutsch", "iso_code": "de", "language_code": "de-de", "active": "1"}]}),
            "configurations" => json!({"configurations": [
                {"id": 1, "name": "PS_LANG_DEFAULT", "value": "1"}, {"id": 2, "name": "PS_SHOP_NAME", "value": "Test Shop"}]}),
            "products" => {
                assert_eq!(q.get("display").map(String::as_str), Some("full"));
                let (offset, count) = q["limit"].split_once(',').unwrap();
                assert_eq!(count, PAGE.to_string());
                let all: Vec<Value> = (1..=PAGE as i64 + 1)
                    .map(|i| json!({"id": i, "id_manufacturer": "1",
                        "name": [{"id": "1", "value": format!("Shirt {i}")}, {"id": "2", "value": "Chemise"}],
                        "description": [{"id": "1", "value": "<p>Soft <b>cotton</b>.</p>"}, {"id": "2", "value": ""}],
                        "link_rewrite": [{"id": "1", "value": "shirt"}],
                        "meta_title": [{"id": "1", "value": ""}]}))
                    .collect();
                let o: usize = offset.parse().unwrap();
                json!({"products": all.into_iter().skip(o).take(PAGE).collect::<Vec<_>>()})
            }
            "categories" => json!({"categories": [
                {"id": 1, "id_parent": "0", "name": [{"id": "1", "value": "Root"}]},
                {"id": 2, "id_parent": "1", "name": [{"id": "1", "value": "Home"}], "description": [{"id": "1", "value": ""}]}]}),
            // Empty lists are a bare `[]`.
            _ => json!([]),
        };
        (StatusCode::OK, v.to_string())
    }

    #[tokio::test]
    async fn pull_and_push_against_a_simulated_webservice() {
        unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
        let m = Mock::default();
        let app = Router::new()
            .route("/api/{*path}", get(api).put(api))
            .with_state(m.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let creds = |key: &str| Creds {
            secrets: json!({"url": format!("http://{addr}/"), "api_key": key}),
            ..Default::default()
        };
        assert!(PrestaShop.test(&creds("wrong")).await.is_err());
        let c = creds("KEY123");
        let info = PrestaShop.test(&c).await.unwrap();
        assert_eq!(
            (info.name.as_str(), info.source_locale.as_deref()),
            ("Test Shop", Some("en"))
        );
        assert_eq!(PrestaShop.locales(&c).await.unwrap(), ["en", "fr", "de"]);

        let mut cursor = None;
        let mut all = vec![];
        let mut pages = 0;
        loop {
            let p = PrestaShop.pull(&c, cursor).await.unwrap();
            all.extend(p.items);
            pages += 1;
            cursor = p.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(pages, 5, "2 product pages, categories, cms, manufacturers");
        // 51 names + 51 descriptions + the Home category name; root, slugs and empty values skipped.
        assert_eq!(all.len(), PAGE * 2 + 2 + 1);
        let d = all
            .iter()
            .find(|i| i.key == "prestashop:products:1:description")
            .unwrap();
        assert_eq!(
            (d.format.clone(), d.text.as_str()),
            (Format::Html, "<p>Soft <b>cotton</b>.</p>")
        );
        assert!(all.iter().any(|i| i.key == "prestashop:categories:2:name"));
        assert!(
            !all.iter()
                .any(|i| i.key.contains("link_rewrite") || i.key.contains("categories:1:"))
        );

        let it = |f: &str, v: &str| PushItem {
            external_id: "products:1".into(),
            field: f.into(),
            value: v.into(),
            format: Format::Plain,
            meta: json!({}),
        };
        let r = PrestaShop
            .push(
                &c,
                "de-DE",
                &[
                    it("name", "Hemd"),
                    it("description", "<p>Weiche <b>Baumwolle</b>.</p>"),
                ],
            )
            .await
            .unwrap();
        assert_eq!((r.ok, r.failed.len()), (2, 0));
        let puts = m.puts.lock().unwrap().clone();
        assert_eq!(puts.len(), 1, "one PUT per resource");
        assert_eq!(puts[0].0, "products/1");
        let b = &puts[0].1;
        assert!(b.contains(r#"<language id="3"><![CDATA[Hemd]]></language>"#));
        assert!(b.contains(r#"<language id="2" xlink:href="http://shop.test/api/languages/2"><![CDATA[Chemise]]></language>"#));
        assert!(b.contains("<![CDATA[<p>Soft <b>cotton</b>.</p>]]>"));
        assert!(b.contains("<![CDATA[<p>Weiche <b>Baumwolle</b>.</p>]]>"));
        assert!(!b.contains("manufacturer_name"));
        assert!(PrestaShop.push(&c, "ja", &[it("name", "x")]).await.is_err());
    }

    #[test]
    fn webhook_signature() {
        let mut h = HeaderMap::new();
        assert!(!PrestaShop.verify_webhook(&h, b"{}", "tok"));
        h.insert("x-glot-webhook-secret", "tok".parse().unwrap());
        assert!(PrestaShop.verify_webhook(&h, b"{}", "tok"));
    }
}
