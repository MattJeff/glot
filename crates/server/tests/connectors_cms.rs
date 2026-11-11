//! Contentful, Sanity and Strapi connectors against simulated servers that follow the official APIs:
//! - Contentful CMA: https://www.contentful.com/developers/docs/references/content-management-api/
//! - Sanity HTTP API: https://www.sanity.io/docs/http-query, https://www.sanity.io/docs/http-mutations,
//!   webhook signatures: https://github.com/sanity-io/webhook-toolkit
//! - Strapi REST + i18n: https://docs.strapi.io/cms/api/rest/locale (v5), https://docs-v4.strapi.io/dev-docs/plugins/i18n (v4)
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use glot_server::connectors::{Connector, Creds, Format, PushItem, SourceItem, registry, rich};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: set once, before any connector reads it.
    ONCE.call_once(|| unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") });
}

#[derive(Default)]
struct Fake {
    data: Value,
    log: Vec<(String, String, Value, HeaderMap)>,
}
type Shared = Arc<Mutex<Fake>>;

/// A simulated CMS: (state, method, path, query, JSON body, headers) → (status, JSON).
type Handler = fn(
    &mut Fake,
    &Method,
    &str,
    &HashMap<String, String>,
    &Value,
    &HeaderMap,
) -> (StatusCode, Value);

async fn serve(data: Value, h: Handler) -> (String, Shared) {
    let s: Shared = Arc::new(Mutex::new(Fake { data, log: vec![] }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let app = Router::new()
        .fallback(
            move |State(s): State<Shared>, m: Method, u: Uri, hd: HeaderMap, b: Bytes| async move {
                let q: HashMap<String, String> = u
                    .query()
                    .unwrap_or("")
                    .split('&')
                    .filter_map(|kv| kv.split_once('='))
                    .map(|(k, v)| (urldecode(k), urldecode(v)))
                    .collect();
                let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
                let mut f = s.lock().unwrap();
                f.log
                    .push((m.to_string(), u.to_string(), body.clone(), hd.clone()));
                let (st, v) = h(&mut f, &m, u.path(), &q, &body, &hd);
                let r: Response = (st, axum::Json(v)).into_response();
                r
            },
        )
        .with_state(s.clone());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, s)
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 3 <= b.len())
            .then(|| u8::from_str_radix(&s[i + 1..i + 3], 16).ok())
            .flatten();
        match hex {
            Some(x) => {
                out.push(x);
                i += 3;
            }
            None => {
                out.push(if b[i] == b'+' { b' ' } else { b[i] });
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into()
}

fn creds(secrets: Value, settings: Value) -> Creds {
    Creds {
        secrets,
        settings,
        since: None,
    }
}

async fn pull_all(c: &dyn Connector, cr: &Creds) -> Vec<SourceItem> {
    let mut out = vec![];
    let mut cursor = None;
    loop {
        let p = c.pull(cr, cursor).await.unwrap();
        out.extend(p.items);
        match p.next {
            Some(n) => cursor = Some(n),
            None => return out,
        }
    }
}

/// What the sync would push: every unit "translated" as `FR(text)`.
fn translate(items: &[SourceItem]) -> Vec<PushItem> {
    items
        .iter()
        .map(|it| {
            let source = match &it.format {
                Format::RichText(d) => d.to_string(),
                _ => it.text.clone(),
            };
            let tr: HashMap<String, String> = rich::units(&it.key, &it.format, &source, "en")
                .into_iter()
                .map(|(k, t)| (k, format!("FR({t})")))
                .collect();
            let value =
                rich::rebuild(&it.key, &it.format, &source, "en", &|k| tr.get(k).cloned()).unwrap();
            let format = match &it.format {
                Format::RichText(_) => Format::RichText(serde_json::from_str(&value).unwrap()),
                f => f.clone(),
            };
            PushItem {
                external_id: it.external_id.clone(),
                field: it.field.clone(),
                value,
                format,
                meta: it.meta.clone(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------- Contentful

fn contentful(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if h.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer cfpat") {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"sys": {"id": "AccessTokenInvalid"}}),
        );
    }
    let Some(rest) = path.strip_prefix("/spaces/sp1/environments/master") else {
        return (StatusCode::NOT_FOUND, json!({}));
    };
    match (m.as_str(), rest) {
        ("GET", "") => (
            StatusCode::OK,
            json!({"name": "master", "sys": {"id": "master", "type": "Environment"}}),
        ),
        ("GET", "/locales") => (
            StatusCode::OK,
            json!({"sys": {"type": "Array"}, "total": 2, "items": [
            {"code": "en-US", "name": "English (United States)", "default": true, "fallbackCode": null},
            {"code": "fr", "name": "French", "default": false, "fallbackCode": "en-US"}]}),
        ),
        ("GET", "/content_types") => (
            StatusCode::OK,
            json!({"sys": {"type": "Array"}, "total": 1, "items": [{
            "sys": {"id": "blogPost", "type": "ContentType"}, "name": "Blog post", "displayField": "title",
            "fields": [
                {"id": "title", "name": "Title", "type": "Symbol", "localized": true, "required": true},
                {"id": "slug", "name": "Slug", "type": "Symbol", "localized": false},
                {"id": "summary", "name": "Summary", "type": "Text", "localized": true},
                {"id": "body", "name": "Body", "type": "RichText", "localized": true},
                {"id": "hero", "name": "Hero", "type": "Link", "linkType": "Asset", "localized": true}]}]}),
        ),
        ("GET", "/entries") => {
            let all = f.data["entries"].as_array().unwrap().clone();
            let skip: usize = q["skip"].parse().unwrap();
            // The server caps pages at 2 entries here: the connector must follow `total`.
            let items: Vec<Value> = all.iter().skip(skip).take(2).cloned().collect();
            (
                StatusCode::OK,
                json!({"sys": {"type": "Array"}, "total": all.len(), "skip": skip, "limit": 2, "items": items}),
            )
        }
        (_, r) if r.starts_with("/entries/") => {
            let mut parts = r["/entries/".len()..].split('/');
            let id = parts.next().unwrap().to_string();
            let published = parts.next() == Some("published");
            if m == "PUT" && !published && id == "e1" && f.data["conflicted"] != true {
                // Someone edits (and publishes) the entry between our GET and PUT: version moves on.
                f.data["conflicted"] = json!(true);
                let e = &mut f.data["entries"][0];
                e["sys"]["publishedVersion"] = json!(5);
                e["sys"]["version"] = json!(6);
                return (
                    StatusCode::CONFLICT,
                    json!({"sys": {"id": "VersionMismatch"}}),
                );
            }
            let entries = f.data["entries"].as_array_mut().unwrap();
            let Some(e) = entries.iter_mut().find(|e| e["sys"]["id"] == id) else {
                return (StatusCode::NOT_FOUND, json!({"sys": {"id": "NotFound"}}));
            };
            if m == "GET" {
                return (StatusCode::OK, e.clone());
            }
            let v: u64 = h["x-contentful-version"].to_str().unwrap().parse().unwrap();
            if v != e["sys"]["version"].as_u64().unwrap() {
                return (
                    StatusCode::CONFLICT,
                    json!({"sys": {"id": "VersionMismatch"}}),
                );
            }
            if published {
                e["sys"]["publishedVersion"] = json!(v);
            } else {
                e["fields"] = body["fields"].clone();
            }
            e["sys"]["version"] = json!(v + 1);
            (StatusCode::OK, e.clone())
        }
        _ => (StatusCode::NOT_FOUND, json!({})),
    }
}

fn cf_entry(id: &str, version: u64, published: Option<u64>, title: &str) -> Value {
    json!({"sys": {"id": id, "type": "Entry", "version": version, "publishedVersion": published,
            "updatedAt": "2026-11-01T10:00:00.000Z", "contentType": {"sys": {"type": "Link", "linkType": "ContentType", "id": "blogPost"}}},
        "fields": {
            "title": {"en-US": title},
            "slug": {"en-US": "free-shipping"},
            "summary": {"en-US": "## Why\n\n- Fast **delivery**\n- No fees\n\n```\ncurl https://x.io\n```"},
            "body": {"en-US": {"nodeType": "document", "data": {}, "content": [
                {"nodeType": "paragraph", "data": {}, "content": [
                    {"nodeType": "text", "value": "Free ", "marks": [{"type": "bold"}], "data": {}},
                    {"nodeType": "hyperlink", "data": {"uri": "https://x.io/ship"}, "content": [
                        {"nodeType": "text", "value": "shipping", "marks": [], "data": {}}]},
                    {"nodeType": "text", "value": " with ", "marks": [], "data": {}},
                    {"nodeType": "text", "value": "npm i glot", "marks": [{"type": "code"}], "data": {}}]},
                {"nodeType": "embedded-entry-block", "data": {"target": {"sys": {"id": "x9", "type": "Link", "linkType": "Entry"}}}, "content": []}]}},
            "hero": {"en-US": {"sys": {"type": "Link", "linkType": "Asset", "id": "img"}}}}})
}

#[tokio::test]
async fn contentful_pull_push_publish() {
    init();
    let data = json!({"entries": [cf_entry("e1", 5, Some(4), "Free shipping"), cf_entry("e2", 3, None, "Draft post"), cf_entry("e3", 2, Some(1), "Third")]});
    let (url, s) = serve(data, contentful).await;
    let c = registry("contentful").unwrap();
    let cr = creds(
        json!({"space_id": "sp1", "token": "cfpat"}),
        json!({"api_url": url, "publish": true}),
    );

    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.source_locale.as_deref(), Some("en-US"));
    assert_eq!(c.locales(&cr).await.unwrap(), ["en-US", "fr"]);
    assert!(
        c.test(&creds(
            json!({"space_id": "sp1", "token": "bad"}),
            json!({"api_url": url})
        ))
        .await
        .is_err()
    );

    let items = pull_all(c.as_ref(), &cr).await;
    assert_eq!(
        items.len(),
        9,
        "3 entries × title/summary/body (slug not localized, hero not text)"
    );
    let t = items
        .iter()
        .find(|i| i.external_id == "e1" && i.field == "title")
        .unwrap();
    assert_eq!(
        (t.key.as_str(), t.text.as_str(), &t.format),
        (
            "contentful:blogPost:e1:title",
            "Free shipping",
            &Format::Plain
        )
    );
    assert_eq!(t.context, "Blog post: Free shipping");
    let body = items
        .iter()
        .find(|i| i.external_id == "e1" && i.field == "body")
        .unwrap();
    let units: Vec<String> = rich::units(&body.key, &body.format, "", "en")
        .into_iter()
        .map(|u| u.1)
        .collect();
    assert_eq!(
        units,
        ["Free", "shipping", "with"],
        "code mark not translated"
    );

    let report = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!((report.ok, report.failed.len()), (9, 0));
    let f = s.lock().unwrap();
    let e1 = &f.data["entries"][0];
    assert_eq!(e1["fields"]["title"]["fr"], "FR(Free shipping)");
    assert_eq!(
        e1["fields"]["title"]["en-US"], "Free shipping",
        "source untouched"
    );
    assert_eq!(e1["fields"]["slug"], json!({"en-US": "free-shipping"}));
    assert_eq!(
        e1["fields"]["summary"]["fr"],
        "## FR(Why)\n\n- FR(Fast **delivery**)\n- FR(No fees)\n\n```\ncurl https://x.io\n```"
    );
    let p = &e1["fields"]["body"]["fr"]["content"][0]["content"];
    assert_eq!(
        p[0],
        json!({"nodeType": "text", "value": "FR(Free) ", "marks": [{"type": "bold"}], "data": {}})
    );
    assert_eq!(p[1]["data"]["uri"], "https://x.io/ship");
    assert_eq!(p[1]["content"][0]["value"], "FR(shipping)");
    assert_eq!(p[2]["value"], " FR(with) ");
    assert_eq!(
        p[3],
        json!({"nodeType": "text", "value": "npm i glot", "marks": [{"type": "code"}], "data": {}})
    );
    assert_eq!(
        e1["fields"]["body"]["fr"]["content"][1]["nodeType"],
        "embedded-entry-block"
    );
    // e1: conflict retried (version 6 after the concurrent edit) then published; e2 (draft) not published.
    assert_eq!(e1["sys"]["publishedVersion"], 7);
    assert_eq!(f.data["entries"][1]["sys"]["publishedVersion"], Value::Null);
    let publishes: Vec<&str> = f
        .log
        .iter()
        .filter(|l| l.1.ends_with("/published"))
        .map(|l| l.1.as_str())
        .collect();
    assert_eq!(publishes.len(), 2, "e1 and e3: {publishes:?}");
    assert!(
        f.log
            .iter()
            .filter(|l| l.0 == "PUT")
            .all(|l| l.3["content-type"] == "application/vnd.contentful.management.v1+json")
    );
    drop(f);

    let mut h = HeaderMap::new();
    assert!(!c.verify_webhook(&h, b"{}", "hooksecret"));
    h.insert("x-glot-webhook-secret", "hooksecret".parse().unwrap());
    assert!(c.verify_webhook(&h, b"{}", "hooksecret"));
    assert!(!c.verify_webhook(&h, b"{}", "other"));
}

// ---------------------------------------------------------------------------------------------- Sanity

fn sanity(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if h.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer sktoken") {
        return (StatusCode::UNAUTHORIZED, json!({"error": "Unauthorized"}));
    }
    if m == "POST" && path == "/v2025-02-19/data/mutate/production" {
        f.data["mutations"]
            .as_array_mut()
            .unwrap()
            .extend(body["mutations"].as_array().unwrap().clone());
        return (
            StatusCode::OK,
            json!({"transactionId": "tx1", "results": []}),
        );
    }
    if !(m == "POST"
        && path == "/v2025-02-19/data/query/production"
        && q.get("perspective").map(String::as_str) == Some("raw"))
    {
        return (StatusCode::NOT_FOUND, json!({}));
    }
    let query = body["query"].as_str().unwrap();
    let p = &body["params"];
    let docs = f.data["docs"].as_array().unwrap().clone();
    let result = if query.starts_with("count(*[_type == \"translation.metadata\"]") {
        json!(
            docs.iter()
                .filter(|d| d["_type"] == "translation.metadata")
                .count()
        )
    } else if query.starts_with("count(") {
        json!(docs.len())
    } else if query.starts_with("array::unique") {
        json!(["en", "fr"])
    } else if query == "*[_id in $ids]._id" {
        let ids: Vec<Value> = p["ids"].as_array().unwrap().clone();
        json!(
            docs.iter()
                .filter(|d| ids.contains(&d["_id"]))
                .map(|d| d["_id"].clone())
                .collect::<Vec<_>>()
        )
    } else if query.starts_with("{\"doc\"") {
        let id = &p["id"];
        let meta = docs.iter().find(|d| {
            d["_type"] == "translation.metadata"
                && d["translations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| &t["value"]["_ref"] == id)
        });
        json!({"doc": docs.iter().find(|d| &d["_id"] == id), "meta": meta.map(|m| json!({"_id": m["_id"], "translations": m["translations"]}))})
    } else if query.starts_with("*[!(_id in path") {
        let last = p["last"].as_str().unwrap_or("");
        let mut out: Vec<Value> = docs
            .into_iter()
            .filter(|d| {
                let id = d["_id"].as_str().unwrap();
                !id.starts_with("drafts.")
                    && d["_type"] != "translation.metadata"
                    && id > last
                    && (!query.contains("language == $src") || d["language"] == p["src"])
            })
            .collect();
        out.sort_by(|a, b| a["_id"].as_str().cmp(&b["_id"].as_str()));
        json!(out.into_iter().take(100).collect::<Vec<_>>())
    } else {
        return (
            StatusCode::BAD_REQUEST,
            json!({"error": {"description": format!("unexpected query {query}")}}),
        );
    };
    (
        StatusCode::OK,
        json!({"query": query, "result": result, "ms": 1}),
    )
}

#[tokio::test]
async fn sanity_field_level() {
    init();
    let data = json!({"mutations": [], "docs": [
        {"_id": "post-1", "_type": "post", "_rev": "r1",
         "title": {"_type": "localeString", "en": "Hello world", "fr": "Bonjour"},
         "body": {"_type": "localeBlockContent", "en": [
            {"_type": "block", "_key": "b1", "style": "normal", "markDefs": [{"_key": "l1", "_type": "link", "href": "https://x.io"}],
             "children": [{"_type": "span", "_key": "s1", "text": "Read the ", "marks": []},
                          {"_type": "span", "_key": "s2", "text": "docs", "marks": ["l1", "strong"]},
                          {"_type": "span", "_key": "s3", "text": "glot sync", "marks": ["code"]}]},
            {"_type": "image", "_key": "i1", "asset": {"_ref": "image-abc"}}]},
         "tagline": [{"_key": "en", "_type": "internationalizedArrayStringValue", "value": "Ship faster"}],
         "slug": {"_type": "slug", "current": "hello-world"}},
        {"_id": "drafts.post-1", "_type": "post", "title": {"en": "Hello world (draft)"}}]});
    let (url, s) = serve(data, sanity).await;
    let c = registry("sanity").unwrap();
    let cr = creds(
        json!({"project_id": "abc123", "dataset": "production", "token": "sktoken"}),
        json!({"api_host": url}),
    );
    let info = c.test(&cr).await.unwrap();
    assert!(
        info.notes.iter().any(|n| n.contains("field-level")),
        "{:?}",
        info.notes
    );
    assert_eq!(c.locales(&cr).await.unwrap(), ["en", "fr"]);

    let items = pull_all(c.as_ref(), &cr).await;
    let fields: Vec<&str> = items.iter().map(|i| i.field.as_str()).collect();
    assert_eq!(
        fields,
        ["title", "body", "tagline"],
        "drafts and slugs skipped"
    );
    assert_eq!(items[0].key, "sanity:post:post-1:title");
    let units: Vec<String> = rich::units(&items[1].key, &items[1].format, "", "en")
        .into_iter()
        .map(|u| u.1)
        .collect();
    assert_eq!(units, ["Read the", "docs"]);

    let report = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!(report.ok, 3);
    let f = s.lock().unwrap();
    let muts = f.data["mutations"].as_array().unwrap();
    // Published doc and its draft: one set/unset patch + one insert patch each.
    assert_eq!(muts.len(), 4, "{muts:#?}");
    let set = &muts[0]["patch"];
    assert_eq!(set["id"], "post-1");
    assert_eq!(set["set"]["title.fr"], "FR(Hello world)");
    let block = &set["set"]["body.fr"][0];
    assert_eq!(block["markDefs"][0]["href"], "https://x.io");
    assert_eq!(block["children"][0]["text"], "FR(Read the) ");
    assert_eq!(
        block["children"][1],
        json!({"_type": "span", "_key": "s2", "text": "FR(docs)", "marks": ["l1", "strong"]})
    );
    assert_eq!(
        block["children"][2],
        json!({"_type": "span", "_key": "s3", "text": "glot sync", "marks": ["code"]})
    );
    assert_eq!(set["set"]["body.fr"][1]["asset"]["_ref"], "image-abc");
    assert_eq!(set["unset"], json!(["tagline[_key==\"fr\"]"]));
    assert_eq!(
        muts[1]["patch"]["insert"],
        json!({"after": "tagline[-1]", "items": [{"_key": "fr", "_type": "internationalizedArrayStringValue", "value": "FR(Ship faster)"}]})
    );
    assert_eq!(muts[2]["patch"]["id"], "drafts.post-1");
    drop(f);

    // GROQ webhook signature (webhook-toolkit format).
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let body = br#"{"_id":"post-1","_type":"post"}"#;
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"whsec_1").unwrap();
    mac.update(b"1731340800000.");
    mac.update(body);
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    let mut h = HeaderMap::new();
    h.insert(
        "sanity-webhook-signature",
        format!("t=1731340800000,v1={sig}").parse().unwrap(),
    );
    assert!(c.verify_webhook(&h, body, "whsec_1"));
    assert!(!c.verify_webhook(&h, b"{}", "whsec_1"));
    assert!(!c.verify_webhook(&h, body, "nope"));
}

#[tokio::test]
async fn sanity_document_level() {
    init();
    let page = |id: &str, title: &str| {
        json!({"_id": id, "_type": "page", "_rev": "r", "_createdAt": "2026-01-01T00:00:00Z", "language": "en",
            "title": title, "layout": "wide", "cta": {"label": "Start now", "href": "/signup"},
            "author": {"_type": "reference", "_ref": "person-1"},
            "content": [{"_type": "block", "_key": "k1", "markDefs": [], "children": [{"_type": "span", "_key": "c1", "text": "Welcome home", "marks": ["em"]}]}]})
    };
    let data = json!({"mutations": [], "docs": [
        page("about", "About us"), page("pricing", "Pricing"),
        {"_id": "about-fr-old", "_type": "page", "language": "fr", "title": "À propos"},
        {"_id": "meta1", "_type": "translation.metadata", "translations": [
            {"_key": "en", "_type": "internationalizedArrayReferenceValue", "value": {"_type": "reference", "_ref": "about"}},
            {"_key": "fr", "_type": "internationalizedArrayReferenceValue", "value": {"_type": "reference", "_ref": "about-fr-old"}}]}]});
    let (url, s) = serve(data, sanity).await;
    let c = registry("sanity").unwrap();
    let cr = creds(
        json!({"project_id": "abc123", "dataset": "production", "token": "sktoken"}),
        json!({"api_host": url}),
    );
    assert!(
        c.test(&cr)
            .await
            .unwrap()
            .notes
            .iter()
            .any(|n| n.contains("document-level"))
    );
    let items = pull_all(c.as_ref(), &cr).await;
    let mut fields: Vec<String> = items
        .iter()
        .map(|i| format!("{}/{}", i.external_id, i.field))
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "about/content",
            "about/cta.label",
            "about/title",
            "pricing/content",
            "pricing/cta.label",
            "pricing/title"
        ]
    );
    let report = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!(
        (report.ok, report.failed.len()),
        (6, 0),
        "{:?}",
        report.failed
    );
    let f = s.lock().unwrap();
    let muts = f.data["mutations"].as_array().unwrap();
    // about: existing translation patched.
    let p = &muts[0]["patch"];
    assert_eq!(p["id"], "about-fr-old");
    assert_eq!(p["set"]["title"], "FR(About us)");
    assert_eq!(p["set"]["cta.label"], "FR(Start now)");
    assert_eq!(
        p["set"]["content"][0]["children"][0]["marks"],
        json!(["em"])
    );
    // pricing: translation created from the source, linked in a new metadata document.
    let doc = &muts[1]["createOrReplace"];
    assert_eq!(doc["_id"], "pricing-fr");
    assert_eq!(doc["language"], "fr");
    assert_eq!(doc["title"], "FR(Pricing)");
    assert_eq!(doc["layout"], "wide");
    assert_eq!(
        doc["cta"],
        json!({"label": "FR(Start now)", "href": "/signup"})
    );
    assert_eq!(doc["author"]["_ref"], "person-1");
    assert!(doc.get("_rev").is_none());
    let meta = &muts[2]["createIfNotExists"];
    assert_eq!(meta["_type"], "translation.metadata");
    assert_eq!(meta["translations"][0]["value"]["_ref"], "pricing");
    assert_eq!(meta["translations"][1]["_key"], "fr");
    assert_eq!(meta["translations"][1]["value"]["_ref"], "pricing-fr");
    assert_eq!(muts.len(), 3);
}

// ---------------------------------------------------------------------------------------------- Strapi

fn strapi_schema() -> Value {
    let loc = json!({"i18n": {"localized": true}});
    json!({"data": [
        {"uid": "api::article.article", "schema": {"kind": "collectionType", "singularName": "article", "pluralName": "articles", "displayName": "Article",
          "pluginOptions": loc, "attributes": {
            "title": {"type": "string", "pluginOptions": loc}, "slug": {"type": "uid", "pluginOptions": loc},
            "content": {"type": "richtext", "pluginOptions": loc}, "body": {"type": "blocks", "pluginOptions": loc},
            "price": {"type": "decimal"}, "sku": {"type": "string", "pluginOptions": {"i18n": {"localized": false}}}}}},
        {"uid": "api::homepage.homepage", "schema": {"kind": "singleType", "singularName": "homepage", "pluralName": "homepages", "displayName": "Homepage",
          "pluginOptions": loc, "attributes": {"headline": {"type": "string", "pluginOptions": loc}}}},
        {"uid": "plugin::users-permissions.user", "schema": {"kind": "collectionType", "pluralName": "users", "attributes": {}}}]})
}

fn strapi_article(v5: bool, id: u64, doc: &str, title: &str) -> Value {
    let attrs = json!({"title": title, "slug": "a-slug", "sku": "SKU-1", "price": 9.5, "locale": "en",
        "content": "# Title\n\nSome *text*.", "updatedAt": "2026-11-01T10:00:00.000Z",
        "body": [{"type": "paragraph", "children": [{"type": "text", "text": "Go to "},
                   {"type": "link", "url": "https://x.io", "children": [{"type": "text", "text": "our site", "bold": true}]}]},
                 {"type": "code", "children": [{"type": "text", "text": "npm run build"}]}]});
    if v5 {
        let mut a = attrs;
        a["id"] = json!(id);
        a["documentId"] = json!(doc);
        a["localizations"] = json!([]);
        a
    } else {
        let mut a = attrs;
        a["localizations"] = if id == 1 {
            json!({"data": [{"id": 9, "attributes": {"locale": "fr"}}]})
        } else {
            json!({"data": []})
        };
        json!({"id": id, "attributes": a})
    }
}

fn strapi(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if h.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer strapitok") {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"data": null, "error": {"status": 401, "name": "UnauthorizedError"}}),
        );
    }
    let v5 = f.data["v5"] == true;
    match (m.as_str(), path) {
        ("GET", "/api/i18n/locales") => (
            StatusCode::OK,
            json!([{"id": 1, "name": "English (en)", "code": "en", "isDefault": true}, {"id": 2, "name": "French (fr)", "code": "fr", "isDefault": false}]),
        ),
        ("GET", "/api/content-type-builder/content-types") => (StatusCode::OK, strapi_schema()),
        ("GET", "/api/articles") => {
            assert_eq!(q["locale"], "en");
            let page: usize = q["pagination[page]"].parse().unwrap();
            let all = [
                strapi_article(v5, 1, "d1", "First post"),
                strapi_article(v5, 2, "d2", "Second post"),
            ];
            (
                StatusCode::OK,
                json!({"data": [all[page - 1].clone()], "meta": {"pagination": {"page": page, "pageSize": 1, "pageCount": 2, "total": 2}}}),
            )
        }
        ("GET", "/api/homepage") => {
            let a = json!({"headline": "Welcome to Acme", "locale": "en"});
            let d = if v5 {
                let mut a = a;
                a["documentId"] = json!("h1");
                a["id"] = json!(3);
                a
            } else {
                json!({"id": 3, "attributes": a})
            };
            (StatusCode::OK, json!({"data": d, "meta": {}}))
        }
        ("PUT" | "POST", _) => {
            f.data["writes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"m": m.as_str(), "path": path, "q": q, "body": body}));
            (StatusCode::OK, json!({"data": {"id": 99}}))
        }
        _ => (
            StatusCode::NOT_FOUND,
            json!({"data": null, "error": {"status": 404, "name": "NotFoundError"}}),
        ),
    }
}

async fn strapi_case(v5: bool) -> Vec<Value> {
    init();
    let (url, s) = serve(json!({"v5": v5, "writes": []}), strapi).await;
    let c = registry("strapi").unwrap();
    let cr = creds(json!({"url": url, "token": "strapitok"}), json!({}));
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.source_locale.as_deref(), Some("en"));
    assert!(
        info.notes
            .iter()
            .any(|n| n == "content types: articles, homepage"),
        "{:?}",
        info.notes
    );
    let items = pull_all(c.as_ref(), &cr).await;
    let fields: Vec<String> = items
        .iter()
        .map(|i| format!("{}/{}", i.external_id, i.field))
        .collect();
    let (d1, d2) = if v5 { ("d1", "d2") } else { ("1", "2") };
    assert_eq!(
        fields,
        [
            format!("{d1}/title"),
            format!("{d1}/content"),
            format!("{d1}/body"),
            format!("{d2}/title"),
            format!("{d2}/content"),
            format!("{d2}/body"),
            "homepage/headline".into()
        ]
    );
    assert_eq!(items[0].key, format!("strapi:articles:{d1}:title"));
    let units: Vec<String> = rich::units(&items[2].key, &items[2].format, "", "en")
        .into_iter()
        .map(|u| u.1)
        .collect();
    assert_eq!(units, ["Go to", "our site"], "code block kept");
    let report = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!(
        (report.ok, report.failed.len()),
        (7, 0),
        "{:?}",
        report.failed
    );
    let mut h = HeaderMap::new();
    h.insert("authorization", "Bearer hook-secret".parse().unwrap());
    assert!(c.verify_webhook(&h, b"{}", "hook-secret") && !c.verify_webhook(&h, b"{}", "x"));
    let w = s.lock().unwrap().data["writes"].as_array().unwrap().clone();
    let data = |b: &Value| {
        if b.get("data").is_some() {
            b["data"].clone()
        } else {
            b.clone()
        }
    };
    let first = data(&w[0]["body"]);
    assert_eq!(first["title"], "FR(First post)");
    assert_eq!(first["content"], "# FR(Title)\n\nFR(Some *text*.)");
    assert_eq!(first["body"][0]["children"][1]["url"], "https://x.io");
    assert_eq!(
        first["body"][0]["children"][1]["children"][0],
        json!({"type": "text", "text": "FR(our site)", "bold": true})
    );
    assert_eq!(first["body"][1]["children"][0]["text"], "npm run build");
    assert!(
        first.get("sku").is_none() && first.get("slug").is_none() && first.get("price").is_none(),
        "only localized text"
    );
    w
}

#[tokio::test]
async fn strapi_v5() {
    let w = strapi_case(true).await;
    let paths: Vec<String> = w
        .iter()
        .map(|x| {
            format!(
                "{} {} {}",
                x["m"].as_str().unwrap(),
                x["path"].as_str().unwrap(),
                x["q"]["locale"].as_str().unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        paths,
        [
            "PUT /api/articles/d1 fr",
            "PUT /api/articles/d2 fr",
            "PUT /api/homepage fr"
        ]
    );
    assert!(
        w.iter().all(|x| x["q"]["status"] == "draft"),
        "not published unless asked"
    );
}

#[tokio::test]
async fn strapi_v4() {
    let w = strapi_case(false).await;
    let paths: Vec<String> = w
        .iter()
        .map(|x| {
            format!(
                "{} {}",
                x["m"].as_str().unwrap(),
                x["path"].as_str().unwrap()
            )
        })
        .collect();
    // Article 1 already has a fr localization (id 9): updated; article 2 and the homepage: created.
    assert_eq!(
        paths,
        [
            "PUT /api/articles/9",
            "POST /api/articles/2/localizations",
            "POST /api/homepage/localizations"
        ]
    );
    assert_eq!(w[1]["body"]["locale"], "fr");
    assert_eq!(w[1]["body"]["title"], "FR(Second post)");
}

// ---------------------------------------------------------------------------------------------- end to end

/// Contentful through the whole sync (dashboard API → pull → job with the mock model → push → re-push → webhook).
#[tokio::test]
async fn contentful_end_to_end() {
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use sqlx::{PgPool, postgres::PgConnectOptions};
    use tower::ServiceExt;
    init();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let db = format!("glot_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    glot_server::jobs::spawn_worker(state.clone());
    let app = glot_server::router(state.clone(), "http://localhost:3100");
    let call = |m: Method, uri: String, cookie: Option<String>, body: Option<Value>| {
        let mut req = Request::builder()
            .method(m)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (s, h) = (res.status(), res.headers().clone());
            let b = res.into_body().collect().await.unwrap().to_bytes();
            (
                s,
                h,
                serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null),
            )
        }
    };
    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup".into(),
        None,
        Some(json!({"email": "o@x.io", "password": "correct horse", "name": "O"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    let c = || Some(cookie.clone());
    let (s, ..) = call(
        Method::POST,
        "/api/projects".into(),
        c(),
        Some(json!({"name": "C", "slug": "cf", "source_locale": "en", "target_locales": ["fr"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/cf".into(),
        c(),
        Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;

    let data = json!({"conflicted": true, "entries": [cf_entry("e1", 5, Some(4), "Free shipping"), cf_entry("e2", 3, None, "Draft post")]});
    let (base, fake) = serve(data, contentful).await;
    let (s, _, body) = call(Method::POST, "/api/projects/cf/connectors".into(), c(),
        Some(json!({"kind": "contentful", "secrets": {"space_id": "sp1", "token": "cfpat", "api_url": base}}))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["info"]["source_locale"], "en-US");
    let id = body["connector"]["id"].as_str().unwrap().to_string();
    let (s, ..) = call(
        Method::POST,
        format!("/api/projects/cf/connectors/{id}/sync"),
        c(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let settle = || async {
        glot_server::connectors::sync::tick(&state).await.unwrap();
        for _ in 0..200 {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM jobs WHERE status IN ('queued', 'running')",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if n == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        glot_server::connectors::sync::tick(&state).await.unwrap();
    };
    settle().await;
    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM keys ORDER BY key")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        keys.contains(&"contentful:blogPost:e1:title".to_string()),
        "{keys:?}"
    );
    assert!(
        keys.iter()
            .any(|k| k.starts_with("contentful:blogPost:e1:body#")),
        "rich text split: {keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|k| k.contains("slug") || k.contains("hero")),
        "{keys:?}"
    );
    let puts = |f: &Fake| f.log.iter().filter(|l| l.0 == "PUT").count();
    {
        let f = fake.lock().unwrap();
        let e1 = &f.data["entries"][0]["fields"];
        let fr_title = e1["title"]["fr"].as_str().unwrap_or_default();
        assert!(
            fr_title.contains("Free shipping") && fr_title != "Free shipping",
            "translated: {e1}"
        );
        let p = &e1["body"]["fr"]["content"][0]["content"];
        assert_eq!(p[0]["marks"][0]["type"], "bold");
        assert_eq!(p[1]["data"]["uri"], "https://x.io/ship");
        assert_eq!(p[3]["value"], "npm i glot");
        assert!(
            e1["summary"]["fr"]
                .as_str()
                .unwrap()
                .ends_with("```\ncurl https://x.io\n```")
        );
        assert_eq!(
            puts(&f),
            2,
            "one PUT per entry, nothing published without the setting"
        );
    }
    // Nothing changed: the next push writes nothing.
    let acc: glot_server::connectors::sync::AccountRow = sqlx::query_as(&format!(
        "SELECT {} FROM connector_accounts WHERE id = $1::uuid",
        glot_server::connectors::sync::COLS
    ))
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let st = glot_server::connectors::sync::push_ready(&state, &acc)
        .await
        .unwrap();
    assert_eq!((st.pushed, st.unchanged), (0, 6));
    assert_eq!(puts(&fake.lock().unwrap()), 2);
    // Webhook: the secret header or the URL token, nothing else.
    let token = acc.webhook_secret.clone();
    let hook = |hdr: Option<&str>, q: &str| {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/connectors/{id}/webhook{q}"));
        if let Some(v) = hdr {
            req = req.header("x-glot-webhook-secret", v);
        }
        app.clone().oneshot(req.body(Body::from("{}")).unwrap())
    };
    assert_eq!(
        hook(Some("nope"), "").await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        hook(Some(&token), "").await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        hook(None, &format!("?token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    drop(pool);
    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
