//! HubSpot CMS, Drupal, Ghost, Craft CMS, Zendesk Guide and Help Scout Docs connectors against simulated
//! servers that follow the official APIs:
//! - HubSpot CMS v3: https://developers.hubspot.com/docs/api-reference/latest/cms/blogs/posts/guide
//! - Drupal JSON:API: https://www.drupal.org/docs/core-modules-and-themes/core-modules/jsonapi-module/translations
//! - Ghost Admin API: https://docs.ghost.org/admin-api
//! - Craft GraphQL: https://craftcms.com/docs/5.x/development/graphql.html
//! - Zendesk Help Center: https://developer.zendesk.com/api-reference/help_center/help-center-api/translations/
//! - Help Scout Docs: https://developer.helpscout.com/docs-api/
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use base64::Engine;
use glot_server::connectors::{Connector, Creds, Format, PushItem, SourceItem, registry, rich};
use hmac::{Hmac, Mac};
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
    url: String,
    data: Value,
    log: Vec<(String, String, Value)>,
}
type Shared = Arc<Mutex<Fake>>;
type Handler = fn(
    &mut Fake,
    &Method,
    &str,
    &HashMap<String, String>,
    &Value,
    &HeaderMap,
) -> (StatusCode, Value);

async fn serve(data: Value, h: Handler) -> (String, Shared) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let s: Shared = Arc::new(Mutex::new(Fake {
        url: url.clone(),
        data,
        log: vec![],
    }));
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
                f.log.push((m.to_string(), u.to_string(), body.clone()));
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
    for _ in 0..100 {
        let p = c.pull(cr, cursor).await.unwrap();
        out.extend(p.items);
        match p.next {
            Some(n) => cursor = Some(n),
            None => return out,
        }
    }
    panic!("pagination never ends")
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

fn basic(user: &str, pw: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pw}"))
    )
}

fn auth_is(h: &HeaderMap, want: &str) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some(want)
}

fn hmac_raw(secret: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(secret).unwrap();
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

fn find<'a>(items: &'a [SourceItem], ext: &str, field: &str) -> &'a SourceItem {
    items
        .iter()
        .find(|i| i.external_id == ext && i.field == field)
        .unwrap_or_else(|| panic!("{ext}/{field} not pulled"))
}

// ---------------------------------------------------------------------------------------------- Zendesk

fn zendesk(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if !auth_is(h, &basic("ed@acme.io/token", "zt")) {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"error": "Couldn't authenticate you"}),
        );
    }
    let Some(p) = path.strip_prefix("/api/v2") else {
        return (StatusCode::NOT_FOUND, json!({}));
    };
    if p == "/users/me.json" {
        return (StatusCode::OK, json!({"user": {"id": 42, "role": "admin"}}));
    }
    if p == "/help_center/locales.json" {
        return (
            StatusCode::OK,
            json!({"locales": ["en-us", "fr"], "default_locale": "en-us"}),
        );
    }
    for res in ["categories", "sections", "articles"] {
        if p == format!("/help_center/en-us/{res}.json") {
            assert!(q["page[size]"].parse::<u32>().unwrap() <= 100);
            // One item per page: the connector must follow `meta.after_cursor`.
            let all = f.data[res].as_array().unwrap().clone();
            let at: usize = q
                .get("page[after]")
                .map(|c| c[1..].parse().unwrap())
                .unwrap_or(0);
            let more = at + 1 < all.len();
            return (
                StatusCode::OK,
                json!({res: all.get(at).into_iter().collect::<Vec<_>>(),
                    "meta": {"has_more": more, "after_cursor": more.then(|| format!("c{}", at + 1)), "before_cursor": null}}),
            );
        }
    }
    let rest = p.strip_prefix("/help_center/").unwrap_or("");
    let parts: Vec<&str> = rest.split('/').collect();
    if let ([res, id, "translations.json"], &Method::POST) = (&parts[..], m) {
        {
            let t = &body["translation"];
            if t["title"].as_str().is_none() {
                return (StatusCode::BAD_REQUEST, json!({"error": "RecordInvalid"}));
            }
            let k = format!("{res}:{id}:{}", t["locale"].as_str().unwrap());
            f.data["translations"][k] = t.clone();
            return (StatusCode::CREATED, json!({"translation": t}));
        }
    }
    if let [res, id, "translations", loc] = parts[..] {
        let k = format!("{res}:{id}:{}", loc.trim_end_matches(".json"));
        if *m == Method::PUT {
            let Some(t) = f.data["translations"].get_mut(&k) else {
                return (StatusCode::NOT_FOUND, json!({"error": "RecordNotFound"}));
            };
            for (a, v) in body["translation"].as_object().unwrap() {
                t[a] = v.clone();
            }
            return (StatusCode::OK, json!({"translation": t}));
        }
    }
    (StatusCode::NOT_FOUND, json!({}))
}

#[tokio::test]
async fn zendesk_pull_push_webhook() {
    init();
    let data = json!({
        "categories": [{"id": 1, "name": "General", "description": ""}],
        "sections": [{"id": 2, "name": "FAQ", "description": "Common questions"}],
        "articles": [
            {"id": 10, "title": "Reset your password", "body": "<p>Click <a href=\"/reset\">here</a> to <b>reset</b> it.</p>", "locale": "en-us", "source_locale": "en-us"},
            {"id": 11, "title": "Billing", "body": "<p>Pay by card.</p>", "locale": "en-us", "source_locale": "en-us"},
            {"id": 12, "title": "Bonjour", "body": "<p>Écrit en français</p>", "locale": "en-us", "source_locale": "fr"}],
        "translations": {"articles:10:fr": {"locale": "fr", "title": "Ancien", "body": "<p>Ancien</p>", "draft": false}}});
    let (url, s) = serve(data, zendesk).await;
    let c = registry("zendesk").unwrap();
    let cr = creds(
        json!({"subdomain": url, "email": "ed@acme.io", "token": "zt"}),
        json!({}),
    );
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.source_locale.as_deref(), Some("en-us"));
    assert_eq!(c.locales(&cr).await.unwrap(), ["en-us", "fr"]);
    let bad = creds(
        json!({"subdomain": url, "email": "ed@acme.io", "token": "no"}),
        json!({}),
    );
    assert!(c.test(&bad).await.is_err());

    let items = pull_all(c.as_ref(), &cr).await;
    let ids: Vec<String> = items
        .iter()
        .map(|i| format!("{}/{}", i.external_id, i.field))
        .collect();
    assert_eq!(
        ids,
        [
            "categories:1/name",
            "sections:2/name",
            "sections:2/description",
            "articles:10/title",
            "articles:10/body",
            "articles:11/title",
            "articles:11/body"
        ],
        "empty description and the article written in French are not sources"
    );
    let body = find(&items, "articles:10", "body");
    assert_eq!(body.format, Format::Html);
    assert_eq!(body.key, "zendesk:articles:10:body");
    let units: Vec<String> = rich::units(&body.key, &body.format, &body.text, "en")
        .into_iter()
        .map(|u| u.1)
        .collect();
    assert_eq!(units, ["Click <0>here</0> to <1>reset</1> it."]);

    let mut push = translate(&items);
    // Article 11's title is not translated yet: its body alone cannot create the fr translation.
    push.retain(|p| !(p.external_id == "articles:11" && p.field == "title"));
    let r = c.push(&cr, "fr-FR", &push).await.unwrap();
    assert_eq!(r.ok, 5);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].0, "articles:11");
    {
        let f = s.lock().unwrap();
        let t = &f.data["translations"];
        assert_eq!(t["articles:10:fr"]["title"], "FR(Reset your password)");
        assert_eq!(
            t["articles:10:fr"]["body"],
            "<p>FR(Click <a href=\"/reset\">here</a> to <b>reset</b> it.)</p>"
        );
        assert_eq!(
            t["articles:10:fr"]["draft"], false,
            "an existing translation keeps its state"
        );
        assert_eq!(
            t["sections:2:fr"],
            json!({"locale": "fr", "title": "FR(FAQ)", "body": "FR(Common questions)"})
        );
        assert_eq!(t["categories:1:fr"]["title"], "FR(General)");
    }
    let r = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!((r.ok, r.failed.len()), (7, 0));
    assert_eq!(
        s.lock().unwrap().data["translations"]["articles:11:fr"]["draft"],
        true,
        "new article translations are drafts"
    );
    assert!(
        c.push(&cr, "de", &push).await.is_err(),
        "locale not in the Help Center"
    );

    let payload = br#"{"type":"zen:event-type:article.published"}"#;
    let ts = "2026-11-11T10:00:00Z";
    let mut signed = ts.as_bytes().to_vec();
    signed.extend_from_slice(payload);
    let sig = base64::engine::general_purpose::STANDARD.encode(hmac_raw(b"whsec", &signed));
    let mut h = HeaderMap::new();
    h.insert(
        "x-zendesk-webhook-signature",
        HeaderValue::from_str(&sig).unwrap(),
    );
    h.insert(
        "x-zendesk-webhook-signature-timestamp",
        HeaderValue::from_static(ts),
    );
    assert!(c.verify_webhook(&h, payload, "whsec"));
    assert!(!c.verify_webhook(&h, b"{}", "whsec"));
    assert!(!c.verify_webhook(&h, payload, "other"));
}

// ---------------------------------------------------------------------------------------------- HubSpot

const HS: &[&str] = &[
    "/cms/v3/blogs/posts",
    "/cms/v3/pages/site-pages",
    "/cms/v3/pages/landing-pages",
];

fn hubspot(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if !auth_is(h, "Bearer pat-na1-x") {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"status": "error", "category": "INVALID_AUTHENTICATION"}),
        );
    }
    let Some(base) = HS.iter().find(|b| path.starts_with(**b)) else {
        return (StatusCode::NOT_FOUND, json!({}));
    };
    let rest = &path[base.len()..];
    let list = f.data[*base].as_array_mut().unwrap();
    match (m.as_str(), rest) {
        ("GET", "") => {
            // Two objects per page: the connector must follow `paging.next.after`.
            let at: usize = q.get("after").map(|a| a.parse().unwrap()).unwrap_or(0);
            let page: Vec<Value> = list.iter().skip(at).take(2).cloned().collect();
            let next =
                (at + 2 < list.len()).then(|| json!({"next": {"after": (at + 2).to_string()}}));
            (
                StatusCode::OK,
                json!({"total": list.len(), "results": page, "paging": next}),
            )
        }
        ("POST", "/multi-language/create-language-variation") => {
            let src_id = body["id"].as_str().unwrap().to_string();
            let lang = body["language"].as_str().unwrap().to_string();
            let i = list
                .iter()
                .position(|o| o["id"] == src_id.as_str())
                .unwrap();
            let mut v = list[i].clone();
            let id = format!("{}{}", src_id, list.len());
            v["id"] = json!(id);
            v["language"] = json!(lang);
            v["translatedFromId"] = json!(src_id);
            v["translations"] = json!({});
            list[i]["translations"][&lang] = json!({"id": id, "name": v["name"]});
            list.push(v.clone());
            (StatusCode::CREATED, v)
        }
        _ => {
            let mut parts = rest.trim_start_matches('/').splitn(2, '/');
            let id = parts.next().unwrap();
            let tail = parts.next().unwrap_or("");
            let Some(o) = list.iter_mut().find(|o| o["id"] == id) else {
                return (
                    StatusCode::NOT_FOUND,
                    json!({"category": "OBJECT_NOT_FOUND"}),
                );
            };
            match (m.as_str(), tail) {
                ("GET", "") | ("GET", "draft") => (StatusCode::OK, o.clone()),
                ("PATCH", "draft") => {
                    for (k, v) in body.as_object().unwrap() {
                        o[k] = v.clone();
                    }
                    (StatusCode::OK, o.clone())
                }
                ("POST", "draft/push-live") => {
                    o["live"] = json!(true);
                    (StatusCode::NO_CONTENT, Value::Null)
                }
                _ => (StatusCode::NOT_FOUND, json!({})),
            }
        }
    }
}

#[tokio::test]
async fn hubspot_variations() {
    init();
    let post = |id: &str, name: &str, from: Value, lang: &str, tr: Value| {
        json!({"id": id, "name": name, "htmlTitle": format!("{name} | Blog"), "metaDescription": "", "postSummary": "",
            "postBody": "<p>Hello <strong>you</strong>, <a href=\"https://x.io\">read more</a>.</p>",
            "language": lang, "translatedFromId": from, "translations": tr, "archivedInDashboard": false})
    };
    let data = json!({
        "/cms/v3/blogs/posts": [
            post("100", "Hello", Value::Null, "en", json!({"fr": {"id": "101"}})),
            post("101", "Bonjour", json!("100"), "fr", json!({})),
            post("102", "Second post", Value::Null, "en", json!({}))],
        "/cms/v3/pages/site-pages": [{"id": "200", "name": "Home", "htmlTitle": "Welcome", "metaDescription": "Our site",
            "language": "en", "translatedFromId": null, "translations": {},
            "widgets": {"hero": {"body": {"html": "<h1>Fast <em>sites</em></h1>", "css_class": "big"}, "type": "rich_text"}},
            "layoutSections": {"dnd_area": {"rows": [{"0": {"type": "custom_widget", "params": {"html": "<p>Body text</p>", "img": {"src": "a.png"}}}}]}}}],
        "/cms/v3/pages/landing-pages": []});
    let (url, s) = serve(data, hubspot).await;
    let c = registry("hubspot").unwrap();
    let cr = creds(
        json!({"token": "pat-na1-x"}),
        json!({"api_url": url, "publish": true}),
    );
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.source_locale.as_deref(), Some("en"));
    assert_eq!(c.locales(&cr).await.unwrap(), ["en", "fr"]);
    assert!(
        c.test(&creds(json!({"token": "bad"}), json!({"api_url": url})))
            .await
            .is_err()
    );

    let items = pull_all(c.as_ref(), &cr).await;
    assert!(
        items.iter().all(|i| i.external_id != "blog_posts:101"),
        "variations are not sources"
    );
    assert_eq!(
        items.len(),
        3 + 3 + 4,
        "2 posts × name/htmlTitle/postBody + page htmlTitle/meta/2 modules"
    );
    assert_eq!(
        find(&items, "blog_posts:100", "postBody").format,
        Format::Html
    );
    let module = find(
        &items,
        "site_pages:200",
        "/layoutSections/dnd_area/rows/0/0/params/html",
    );
    assert_eq!(
        module.key,
        "hubspot:site_pages:200:/layoutSections/dnd_area/rows/0/0/params/html"
    );

    let r = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!((r.ok, r.failed.len()), (10, 0), "{:?}", r.failed);
    let f = s.lock().unwrap();
    let posts = f.data["/cms/v3/blogs/posts"].as_array().unwrap();
    let v101 = posts.iter().find(|o| o["id"] == "101").unwrap();
    assert_eq!(v101["name"], "FR(Hello)");
    assert_eq!(
        v101["postBody"],
        "<p>FR(Hello <strong>you</strong>, <a href=\"https://x.io\">read more</a>.)</p>"
    );
    assert_eq!(v101["live"], true, "publish pushes the draft live");
    let created = posts
        .iter()
        .find(|o| o["translatedFromId"] == "102")
        .unwrap();
    assert_eq!(
        (created["language"].as_str(), created["name"].as_str()),
        (Some("fr"), Some("FR(Second post)"))
    );
    assert_eq!(
        posts.iter().find(|o| o["id"] == "100").unwrap()["name"],
        "Hello",
        "source untouched"
    );
    let pages = f.data["/cms/v3/pages/site-pages"].as_array().unwrap();
    let pv = pages
        .iter()
        .find(|o| o["translatedFromId"] == "200")
        .unwrap();
    assert_eq!(
        pv["widgets"]["hero"]["body"],
        json!({"html": "<h1>FR(Fast <em>sites</em>)</h1>", "css_class": "big"})
    );
    assert_eq!(
        pv["layoutSections"]["dnd_area"]["rows"][0]["0"]["params"]["html"],
        "<p>FR(Body text)</p>"
    );
    assert_eq!(
        pv["layoutSections"]["dnd_area"]["rows"][0]["0"]["params"]["img"]["src"],
        "a.png"
    );
    assert_eq!(
        pages[0]["widgets"]["hero"]["body"]["html"],
        "<h1>Fast <em>sites</em></h1>"
    );
    let creates = f
        .log
        .iter()
        .filter(|l| l.1.contains("create-language-variation"))
        .count();
    assert_eq!(
        creates, 2,
        "post 102 and page 200; post 100 already had its fr variation"
    );
    drop(f);

    use sha2::Digest;
    let mut hd = HeaderMap::new();
    let sig = hex::encode(sha2::Sha256::digest(b"appsecret[{\"objectId\":1}]"));
    hd.insert("x-hubspot-signature", HeaderValue::from_str(&sig).unwrap());
    assert!(c.verify_webhook(&hd, b"[{\"objectId\":1}]", "appsecret"));
    assert!(!c.verify_webhook(&hd, b"[]", "appsecret"));
}

// ---------------------------------------------------------------------------------------------- Ghost

const GHOST_KEY: &str =
    "6489a1b2c3d4e5f6a7b8c9d0:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Checks the Admin API JWT like Ghost does (kid, aud, exp, HS256 with the hex-decoded secret).
fn ghost_auth(h: &HeaderMap) -> bool {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let Some(t) = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Ghost "))
    else {
        return false;
    };
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let (id, secret) = GHOST_KEY.split_once(':').unwrap();
    let head: Value = serde_json::from_slice(&b64.decode(parts[0]).unwrap()).unwrap();
    let claims: Value = serde_json::from_slice(&b64.decode(parts[1]).unwrap()).unwrap();
    let sig = hmac_raw(
        &hex::decode(secret).unwrap(),
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
    );
    head["alg"] == "HS256"
        && head["kid"] == id
        && claims["aud"] == "/admin/"
        && claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap() <= 300
        && b64.encode(sig) == parts[2]
        && h.get("accept-version").is_some()
}

fn ghost(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if !ghost_auth(h) {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"errors": [{"type": "UnauthorizedError"}]}),
        );
    }
    let Some(p) = path.strip_prefix("/ghost/api/admin") else {
        return (StatusCode::NOT_FOUND, json!({}));
    };
    match (m.as_str(), p) {
        ("GET", "/site/") => {
            return (
                StatusCode::OK,
                json!({"site": {"title": "Field Notes", "version": "5.100"}}),
            );
        }
        ("GET", "/tags/") => {
            assert_eq!(q["filter"], "visibility:internal");
            return (
                StatusCode::OK,
                json!({"tags": [{"name": "#fr", "visibility": "internal"}, {"name": "#featured", "visibility": "internal"}]}),
            );
        }
        ("POST", "/webhooks/") => {
            let w = &body["webhooks"][0];
            let hooks = f.data["webhooks"].as_array_mut().unwrap();
            if hooks
                .iter()
                .any(|x| x["event"] == w["event"] && x["target_url"] == w["target_url"])
            {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    json!({"errors": [{"type": "ValidationError", "message": "Target URL has already been used for this event."}]}),
                );
            }
            hooks.push(w.clone());
            return (StatusCode::CREATED, json!({"webhooks": [w]}));
        }
        _ => {}
    }
    let res = if p.starts_with("/posts/") {
        "posts"
    } else if p.starts_with("/pages/") {
        "pages"
    } else {
        return (StatusCode::NOT_FOUND, json!({}));
    };
    let rest = &p[res.len() + 2..];
    let list = f.data[res].as_array_mut().unwrap();
    match (m.as_str(), rest) {
        ("GET", "") => {
            // One post per page: the connector must follow `meta.pagination.next`.
            let page: usize = match q.get("page") {
                Some(p) => {
                    assert_eq!(q["formats"], "lexical");
                    p.parse().unwrap()
                }
                None => 1,
            };
            let pages = list.len().max(1);
            (
                StatusCode::OK,
                json!({res: list.get(page - 1).into_iter().collect::<Vec<_>>(),
                "meta": {"pagination": {"page": page, "limit": 1, "pages": pages, "total": list.len(), "next": (page < pages).then_some(page + 1), "prev": null}}}),
            )
        }
        ("POST", "") => {
            let mut post = body[res][0].clone();
            post["id"] = json!(format!("new{}", list.len()));
            post["updated_at"] = json!("2026-11-11T10:00:00.000Z");
            list.push(post.clone());
            (StatusCode::CREATED, json!({res: [post]}))
        }
        ("GET", r) if r.starts_with("slug/") => {
            let slug = r[5..].trim_end_matches('/');
            match list.iter().find(|o| o["slug"] == slug) {
                Some(o) => (StatusCode::OK, json!({res: [o]})),
                None => (
                    StatusCode::NOT_FOUND,
                    json!({"errors": [{"type": "NotFoundError"}]}),
                ),
            }
        }
        ("PUT", r) => {
            let id = r.trim_end_matches('/');
            let Some(o) = list.iter_mut().find(|o| o["id"] == id) else {
                return (StatusCode::NOT_FOUND, json!({}));
            };
            let edit = &body[res][0];
            if edit["updated_at"] != o["updated_at"] {
                return (
                    StatusCode::CONFLICT,
                    json!({"errors": [{"type": "UpdateCollisionError"}]}),
                );
            }
            for (k, v) in edit.as_object().unwrap() {
                o[k] = v.clone();
            }
            o["updated_at"] = json!("2026-11-11T11:00:00.000Z");
            (StatusCode::OK, json!({res: [o]}))
        }
        _ => (StatusCode::NOT_FOUND, json!({})),
    }
}

#[tokio::test]
async fn ghost_language_tags() {
    init();
    let lexical = json!({"root": {"type": "root", "children": [
        {"type": "paragraph", "children": [
            {"type": "extended-text", "text": "Install it with ", "format": 0},
            {"type": "extended-text", "text": "npm i glot", "format": 16},
            {"type": "link", "url": "https://x.io", "children": [{"type": "extended-text", "text": "the docs", "format": 1}]}]},
        {"type": "image", "src": "https://x.io/a.png", "caption": ""}]}}).to_string();
    let data = json!({
        "posts": [
            {"id": "p1", "slug": "hello", "title": "Hello", "custom_excerpt": "Short intro", "lexical": lexical, "tags": [{"name": "News"}], "updated_at": "2026-11-01T10:00:00.000Z", "status": "published"},
            {"id": "p2", "slug": "hello-fr", "title": "Bonjour", "lexical": "{}", "tags": [{"name": "#fr"}], "updated_at": "2026-11-02T10:00:00.000Z", "status": "published"},
            {"id": "p3", "slug": "second", "title": "Second", "custom_excerpt": null, "lexical": null, "tags": [{"name": "News"}, {"name": "#featured"}], "updated_at": "2026-11-03T10:00:00.000Z", "status": "draft"}],
        "pages": [],
        "webhooks": []});
    let (url, s) = serve(data, ghost).await;
    let c = registry("ghost").unwrap();
    let cr = creds(
        json!({"url": url, "admin_key": GHOST_KEY, "webhook_secret": "ghsec"}),
        json!({}),
    );
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.name, "Field Notes");
    assert_eq!(
        c.locales(&cr).await.unwrap(),
        ["fr"],
        "#featured is not a language"
    );
    let wrong = format!(
        "{}:{}",
        GHOST_KEY.split(':').next().unwrap(),
        "ab".repeat(32)
    );
    assert!(
        c.test(&creds(json!({"url": url, "admin_key": wrong}), json!({})))
            .await
            .is_err()
    );

    let items = pull_all(c.as_ref(), &cr).await;
    let ids: Vec<String> = items
        .iter()
        .map(|i| format!("{}/{}", i.external_id, i.field))
        .collect();
    assert_eq!(
        ids,
        [
            "posts:p1/title",
            "posts:p1/custom_excerpt",
            "posts:p1/lexical",
            "posts:p3/title"
        ],
        "#fr post is a translation, #featured is not"
    );
    let lex = find(&items, "posts:p1", "lexical");
    let units: Vec<String> = rich::units(&lex.key, &lex.format, "", "en")
        .into_iter()
        .map(|u| u.1)
        .collect();
    assert_eq!(
        units,
        ["Install it with", "the docs"],
        "inline code not translated"
    );

    let r = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!((r.ok, r.failed.len()), (4, 0), "{:?}", r.failed);
    {
        let f = s.lock().unwrap();
        let posts = f.data["posts"].as_array().unwrap();
        let p2 = posts.iter().find(|o| o["id"] == "p2").unwrap();
        assert_eq!(p2["title"], "FR(Hello)");
        assert_eq!(
            p2["tags"],
            json!([{"name": "#fr"}]),
            "an update leaves tags alone"
        );
        let doc: Value = serde_json::from_str(p2["lexical"].as_str().unwrap()).unwrap();
        let para = &doc["root"]["children"][0]["children"];
        assert_eq!(
            para[0],
            json!({"type": "extended-text", "text": "FR(Install it with) ", "format": 0})
        );
        assert_eq!(
            para[1],
            json!({"type": "extended-text", "text": "npm i glot", "format": 16})
        );
        assert_eq!(para[2]["url"], "https://x.io");
        assert_eq!(para[2]["children"][0]["text"], "FR(the docs)");
        assert_eq!(doc["root"]["children"][1]["src"], "https://x.io/a.png");
        let new = posts.iter().find(|o| o["slug"] == "second-fr").unwrap();
        assert_eq!(new["title"], "FR(Second)");
        assert_eq!(new["status"], "draft");
        assert_eq!(
            new["tags"],
            json!([{"name": "News"}, {"name": "#featured"}, {"name": "#fr"}])
        );
        assert_eq!(posts[0]["title"], "Hello", "source untouched");
    }
    // Second push: both translations exist now, updated with their current `updated_at`.
    let r = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!((r.ok, r.failed.len()), (4, 0), "{:?}", r.failed);
    assert_eq!(s.lock().unwrap().data["posts"].as_array().unwrap().len(), 4);

    c.register_webhooks(&cr, "https://glot.example/api/connectors/x/webhook")
        .await
        .unwrap();
    c.register_webhooks(&cr, "https://glot.example/api/connectors/x/webhook")
        .await
        .unwrap();
    let hooks = s.lock().unwrap().data["webhooks"].clone();
    assert_eq!(
        hooks.as_array().unwrap().len(),
        4,
        "registered once per event"
    );
    assert_eq!(hooks[0]["secret"], "ghsec");

    let payload = br#"{"post":{"current":{"id":"p1"}}}"#;
    let ts = "1762855200000";
    let mut signed = payload.to_vec();
    signed.extend_from_slice(ts.as_bytes());
    let sig = hex::encode(hmac_raw(b"ghsec", &signed));
    let mut h = HeaderMap::new();
    h.insert(
        "x-ghost-signature",
        HeaderValue::from_str(&format!("sha256={sig}, t={ts}")).unwrap(),
    );
    assert!(c.verify_webhook(&h, payload, "ghsec"));
    assert!(!c.verify_webhook(&h, payload, "nope"));
}

// ---------------------------------------------------------------------------------------------- Drupal

fn drupal(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if !auth_is(h, &basic("editor", "pw")) {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"errors": [{"status": "401"}]}),
        );
    }
    let base = f.url.clone();
    match (m.as_str(), path) {
        ("GET", "/jsonapi") => {
            return (
                StatusCode::OK,
                json!({"links": {
                "self": {"href": format!("{base}/jsonapi")},
                "node--article": {"href": format!("{base}/jsonapi/node/article")},
                "taxonomy_term--tags": {"href": format!("{base}/jsonapi/taxonomy_term/tags")},
                "user--user": {"href": format!("{base}/jsonapi/user/user")}}}),
            );
        }
        ("GET", "/jsonapi/configurable_language/configurable_language") => {
            return (
                StatusCode::OK,
                json!({"data": (["en", "fr", "und", "zxx"].iter()).map(|c| json!({"type": "configurable_language--configurable_language", "attributes": {"drupal_internal__id": c}})).collect::<Vec<_>>()}),
            );
        }
        _ => {}
    }
    let (lang, rest) = match path.strip_prefix("/fr/") {
        Some(r) => ("fr", format!("/{r}")),
        None => ("en", path.to_string()),
    };
    let parts: Vec<&str> = rest.trim_start_matches('/').split('/').collect();
    let ["jsonapi", e, b, tail @ ..] = &parts[..] else {
        return (StatusCode::NOT_FOUND, json!({}));
    };
    let ty = format!("{e}--{b}");
    let list = f.data[&ty].as_array_mut().unwrap();
    match (m.as_str(), tail) {
        ("GET", []) => {
            // One resource per page: the connector must follow `links.next.href`.
            let at: usize = q
                .get("page[offset]")
                .map(|o| o.parse().unwrap())
                .unwrap_or(0);
            let data: Vec<Value> = list.get(at).map(|o| o["en"].clone()).into_iter().collect();
            let mut links = json!({});
            if at + 1 < list.len() {
                links["next"] = json!({"href": format!("{base}/jsonapi/{e}/{b}?page%5Boffset%5D={}&page%5Blimit%5D=1", at + 1)});
            }
            (StatusCode::OK, json!({"data": data, "links": links}))
        }
        (_, [id]) => {
            let Some(o) = list.iter_mut().find(|o| o["en"]["id"] == *id) else {
                return (StatusCode::NOT_FOUND, json!({}));
            };
            // No translation: Drupal serves the default language.
            let has = o.get(lang).is_some();
            if *m == Method::GET {
                return (
                    StatusCode::OK,
                    json!({"data": if has { o[lang].clone() } else { o["en"].clone() }}),
                );
            }
            if *m == Method::PATCH {
                if h.get("content-type").and_then(|v| v.to_str().ok())
                    != Some("application/vnd.api+json")
                {
                    return (StatusCode::UNSUPPORTED_MEDIA_TYPE, json!({}));
                }
                assert!(
                    has,
                    "PATCH on a missing translation would overwrite the source"
                );
                assert_eq!(body["data"]["id"], *id);
                for (k, v) in body["data"]["attributes"].as_object().unwrap() {
                    o[lang]["attributes"][k] = v.clone();
                }
                return (StatusCode::OK, json!({"data": o[lang].clone()}));
            }
            (StatusCode::METHOD_NOT_ALLOWED, json!({}))
        }
        _ => (StatusCode::NOT_FOUND, json!({})),
    }
}

#[tokio::test]
async fn drupal_translations() {
    init();
    let node = |id: &str, lang: &str, title: &str| {
        json!({"type": "node--article", "id": id, "attributes": {"drupal_internal__nid": 1, "langcode": lang, "title": title, "status": true,
            "path": {"alias": "/hello", "langcode": lang},
            "body": {"value": "<p>Hello <strong>world</strong></p>", "format": "basic_html", "processed": "<p>Hello <strong>world</strong></p>", "summary": "A short summary"}}})
    };
    let data = json!({
        "node--article": [
            {"en": node("aaaa-0001", "en", "Hello"), "fr": node("aaaa-0001", "fr", "Bonjour")},
            {"en": node("bbbb-0002", "en", "Untranslated")}],
        "taxonomy_term--tags": [{"en": {"type": "taxonomy_term--tags", "id": "cccc-0003", "attributes": {"langcode": "en", "name": "Travel",
            "description": {"value": "<p>Trips</p>", "format": "basic_html", "processed": "<p>Trips</p>"}}},
            "fr": {"type": "taxonomy_term--tags", "id": "cccc-0003", "attributes": {"langcode": "fr", "name": "Voyage", "description": {"value": "", "format": "basic_html"}}}}]});
    let (url, s) = serve(data, drupal).await;
    let c = registry("drupal").unwrap();
    let cr = creds(
        json!({"url": url, "username": "editor", "password": "pw"}),
        json!({}),
    );
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.source_locale.as_deref(), Some("en"));
    assert_eq!(c.locales(&cr).await.unwrap(), ["en", "fr"]);
    assert!(
        c.test(&creds(
            json!({"url": url, "username": "editor", "password": "x"}),
            json!({})
        ))
        .await
        .is_err()
    );

    let items = pull_all(c.as_ref(), &cr).await;
    let ids: Vec<String> = items
        .iter()
        .map(|i| format!("{}/{}", i.external_id, i.field))
        .collect();
    assert_eq!(
        ids,
        [
            "node--article:aaaa-0001/title",
            "node--article:aaaa-0001/body.value",
            "node--article:aaaa-0001/body.summary",
            "node--article:bbbb-0002/title",
            "node--article:bbbb-0002/body.value",
            "node--article:bbbb-0002/body.summary",
            "taxonomy_term--tags:cccc-0003/name",
            "taxonomy_term--tags:cccc-0003/description.value"
        ]
    );
    assert_eq!(
        find(&items, "node--article:aaaa-0001", "body.value").format,
        Format::Html
    );

    let r = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!(r.ok, 5);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].0, "node--article:bbbb-0002");
    assert!(r.failed[0].1.contains("Translate tab"), "{}", r.failed[0].1);
    let f = s.lock().unwrap();
    let fr = &f.data["node--article"][0]["fr"]["attributes"];
    assert_eq!(fr["title"], "FR(Hello)");
    assert_eq!(
        fr["body"],
        json!({"value": "<p>FR(Hello <strong>world</strong>)</p>", "format": "basic_html", "summary": "FR(A short summary)"})
    );
    assert_eq!(fr["path"]["alias"], "/hello", "untouched");
    assert_eq!(
        f.data["node--article"][0]["en"]["attributes"]["title"], "Hello",
        "source untouched"
    );
    let term = &f.data["taxonomy_term--tags"][0]["fr"]["attributes"];
    assert_eq!(
        term["description"],
        json!({"value": "<p>FR(Trips)</p>", "format": "basic_html"})
    );
}

// ---------------------------------------------------------------------------------------------- Craft

fn craft(
    f: &mut Fake,
    m: &Method,
    path: &str,
    _q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if *m != Method::POST || path != "/api" {
        return (StatusCode::NOT_FOUND, json!({}));
    }
    if !auth_is(h, "Bearer craft-token") {
        return (
            StatusCode::OK,
            json!({"errors": [{"message": "Invalid Authorization Header"}]}),
        );
    }
    let q = body["query"].as_str().unwrap();
    let v = &body["variables"];
    let entries = f.data["entries"].clone();
    if q.contains("__type(name") {
        assert_eq!(v["t"], "blog_article_Entry");
        let s = |n: &str| json!({"name": n, "type": {"kind": "SCALAR", "name": "String", "ofType": null}});
        return (
            StatusCode::OK,
            json!({"data": {"__type": {"fields": [
            s("id"), s("title"), s("slug"), s("uri"), s("summary"), s("body"),
            {"name": "heroImage", "type": {"kind": "LIST", "name": null, "ofType": {"name": "AssetInterface"}}},
            {"name": "price", "type": {"kind": "SCALAR", "name": "Number", "ofType": null}}]}}}),
        );
    }
    if q.trim_start().starts_with("mutation") {
        assert!(
            q.contains("save_blog_article_Entry(id: $id, siteId: $siteId"),
            "{q}"
        );
        f.data["saved"].as_array_mut().unwrap().push(v.clone());
        return (
            StatusCode::OK,
            json!({"data": {"save_blog_article_Entry": {"id": v["id"]}}}),
        );
    }
    if q.contains("entry(id: $id, site: $s)") {
        let site_id = match v["s"][0].as_str() {
            Some("french") => json!(2),
            _ => Value::Null,
        };
        let entry = (!site_id.is_null()).then(|| json!({"siteId": site_id}));
        return (StatusCode::OK, json!({"data": {"entry": entry}}));
    }
    if q.contains("$ids") {
        assert!(
            q.contains("... on blog_article_Entry { summary body }"),
            "{q}"
        );
        let ids = v["ids"].as_array().unwrap();
        let out: Vec<Value> = entries
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| ids.contains(&e["id"]))
            .map(|e| json!({"id": e["id"], "summary": e["summary"], "body": e["body"]}))
            .collect();
        return (StatusCode::OK, json!({"data": {"entries": out}}));
    }
    if q.contains("limit: 1") {
        return (
            StatusCode::OK,
            json!({"data": {"entries": [{"siteHandle": "default", "language": "en-US"}]}}),
        );
    }
    let off = v["offset"].as_u64().unwrap() as usize;
    let lim = v["limit"].as_u64().unwrap() as usize;
    let page: Vec<Value> = entries.as_array().unwrap().iter().skip(off).take(lim)
        .map(|e| json!({"id": e["id"], "__typename": "blog_article_Entry", "title": e["title"], "sectionHandle": "blog", "typeHandle": "article"})).collect();
    (StatusCode::OK, json!({"data": {"entries": page}}))
}

#[tokio::test]
async fn craft_sites() {
    init();
    let entries: Vec<Value> = (1..=51)
        .map(|i| json!({"id": i.to_string(), "title": format!("Post {i}"), "summary": "Plain summary", "body": "<p>Rich <a href=\"/x\">body</a></p>"}))
        .collect();
    let (url, s) = serve(json!({"entries": entries, "saved": []}), craft).await;
    let c = registry("craft").unwrap();
    let cr = creds(
        json!({"url": format!("{url}/api"), "token": "craft-token"}),
        json!({"sites": {"fr": "french", "de": "german"}}),
    );
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.source_locale.as_deref(), Some("en-US"));
    assert_eq!(c.locales(&cr).await.unwrap(), ["de", "fr"]);
    assert!(
        c.test(&creds(
            json!({"url": format!("{url}/api"), "token": "x"}),
            json!({})
        ))
        .await
        .is_err()
    );

    let items = pull_all(c.as_ref(), &cr).await;
    assert_eq!(
        items.len(),
        51 * 3,
        "two pages (50 + 1) × title/summary/body; slug and uri skipped"
    );
    assert_eq!(find(&items, "entry:51", "body").format, Format::Html);
    assert_eq!(find(&items, "entry:1", "summary").format, Format::Plain);

    let one: Vec<PushItem> = translate(&items)
        .into_iter()
        .filter(|p| p.external_id == "entry:1")
        .collect();
    let r = c.push(&cr, "fr", &one).await.unwrap();
    assert_eq!((r.ok, r.failed.len()), (3, 0), "{:?}", r.failed);
    let saved = s.lock().unwrap().data["saved"][0].clone();
    assert_eq!(
        saved,
        json!({"id": "1", "siteId": 2, "title": "FR(Post 1)", "summary": "FR(Plain summary)", "body": "<p>FR(Rich <a href=\"/x\">body</a>)</p>"})
    );
    let r = c.push(&cr, "de", &one).await.unwrap();
    assert!(
        r.failed[0]
            .1
            .contains("not enabled in the Craft site 'german'"),
        "{:?}",
        r.failed
    );
}

// ---------------------------------------------------------------------------------------------- Help Scout

fn helpscout(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if !auth_is(h, &basic("hskey", "X")) {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"code": 401, "error": "Invalid API Key"}),
        );
    }
    let page: usize = q.get("page").map(|p| p.parse().unwrap()).unwrap_or(1);
    let paged = |name: &str, all: Vec<Value>, per: usize| {
        let pages = all.len().div_ceil(per).max(1);
        let items: Vec<Value> = all.into_iter().skip((page - 1) * per).take(per).collect();
        json!({name: {"page": page, "pages": pages, "count": items.len(), "items": items}})
    };
    match (m.as_str(), path) {
        ("GET", "/v1/sites") => {
            return (
                StatusCode::OK,
                paged(
                    "sites",
                    vec![
                        json!({"id": "s1", "title": "Acme Help"}),
                        json!({"id": "s2", "title": "Acme Aide"}),
                    ],
                    50,
                ),
            );
        }
        ("GET", "/v1/collections") => {
            return (
                StatusCode::OK,
                paged(
                    "collections",
                    vec![
                        json!({"id": "cen", "siteId": "s1", "name": "Guides"}),
                        json!({"id": "cfr", "siteId": "s2", "name": "Guides FR"}),
                    ],
                    50,
                ),
            );
        }
        ("POST", "/v1/articles") => {
            let mut a = body.clone();
            if a["collectionId"].is_null() || a["name"].is_null() || a["text"].is_null() {
                return (StatusCode::BAD_REQUEST, json!({"code": 400}));
            }
            a["id"] = json!(format!("n{}", f.data["articles"].as_array().unwrap().len()));
            f.data["articles"].as_array_mut().unwrap().push(a);
            return (StatusCode::CREATED, Value::Null);
        }
        _ => {}
    }
    if let Some(col) = path
        .strip_prefix("/v1/collections/")
        .and_then(|p| p.strip_suffix("/articles"))
    {
        let refs: Vec<Value> = f.data["articles"].as_array().unwrap().iter().filter(|a| a["collectionId"] == col)
            .map(|a| json!({"id": a["id"], "slug": a["slug"], "name": a["name"], "status": a["status"]})).collect();
        // One article per page for the source collection: the connector must follow `pages`.
        return (
            StatusCode::OK,
            paged("articles", refs, if col == "cen" { 1 } else { 100 }),
        );
    }
    if let Some(id) = path.strip_prefix("/v1/articles/") {
        let Some(a) = f.data["articles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["id"] == id)
        else {
            return (StatusCode::NOT_FOUND, json!({}));
        };
        if *m == Method::GET {
            return (StatusCode::OK, json!({"article": a}));
        }
        if *m == Method::PUT {
            for (k, v) in body.as_object().unwrap() {
                a[k] = v.clone();
            }
            return (StatusCode::OK, Value::Null);
        }
    }
    (StatusCode::NOT_FOUND, json!({}))
}

#[tokio::test]
async fn helpscout_site_per_language() {
    init();
    let data = json!({"articles": [
        {"id": "a1", "collectionId": "cen", "slug": "getting-started", "name": "Getting started", "text": "<p>Open <b>Settings</b>.</p>", "status": "published"},
        {"id": "a2", "collectionId": "cen", "slug": "billing", "name": "Billing", "text": "<p>Invoices</p>", "status": "published"},
        {"id": "f1", "collectionId": "cfr", "slug": "getting-started", "name": "Ancien", "text": "<p>Ancien</p>", "status": "published"}]});
    let (url, s) = serve(data, helpscout).await;
    let c = registry("helpscout").unwrap();
    // Target collections as typed in the dashboard form.
    let cr = creds(
        json!({"api_key": "hskey", "collections": "fr: cfr"}),
        json!({"api_url": url}),
    );
    let info = c.test(&cr).await.unwrap();
    assert_eq!(info.name, "Acme Help");
    assert_eq!(c.locales(&cr).await.unwrap(), ["fr"]);
    assert!(
        c.test(&creds(json!({"api_key": "bad"}), json!({"api_url": url})))
            .await
            .is_err()
    );

    let items = pull_all(c.as_ref(), &cr).await;
    let ids: Vec<String> = items
        .iter()
        .map(|i| format!("{}/{}", i.external_id, i.field))
        .collect();
    assert_eq!(
        ids,
        [
            "article:a1/name",
            "article:a1/text",
            "article:a2/name",
            "article:a2/text"
        ],
        "the fr collection is a target, not a source"
    );

    let r = c.push(&cr, "fr", &translate(&items)).await.unwrap();
    assert_eq!((r.ok, r.failed.len()), (4, 0), "{:?}", r.failed);
    let arts = s.lock().unwrap().data["articles"]
        .as_array()
        .unwrap()
        .clone();
    let f1 = arts.iter().find(|a| a["id"] == "f1").unwrap();
    assert_eq!(
        (f1["name"].as_str(), f1["text"].as_str()),
        (
            Some("FR(Getting started)"),
            Some("<p>FR(Open <b>Settings</b>.)</p>")
        )
    );
    let new = arts
        .iter()
        .find(|a| a["collectionId"] == "cfr" && a["slug"] == "billing")
        .unwrap();
    assert_eq!(
        (new["name"].as_str(), new["status"].as_str()),
        (Some("FR(Billing)"), Some("notpublished"))
    );
    assert_eq!(arts[0]["name"], "Getting started", "source untouched");
    assert!(
        c.push(&cr, "de", &translate(&items)).await.is_err(),
        "no collection for de"
    );
}

// ---------------------------------------------------------------------------------------------- current / remove / doctor

fn item(ext: &str, field: &str, meta: Value) -> PushItem {
    PushItem {
        external_id: ext.into(),
        field: field.into(),
        value: String::new(),
        format: Format::Plain,
        meta,
    }
}

fn statuses(checks: &[glot_server::connectors::Check]) -> Vec<(&str, &str, Option<&str>)> {
    checks
        .iter()
        .map(|k| (k.id.as_str(), k.status, k.fix))
        .collect()
}

/// Zendesk with List / Delete Translation, and a configurable role and locale list.
fn zendesk_read(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    let p = path.strip_prefix("/api/v2").unwrap_or("");
    if p == "/users/me.json" && f.data["role"].is_string() {
        return (
            StatusCode::OK,
            json!({"user": {"id": 42, "role": f.data["role"]}}),
        );
    }
    if p == "/help_center/locales.json" && f.data["locales"].is_array() {
        return (
            StatusCode::OK,
            json!({"locales": f.data["locales"], "default_locale": "en-us"}),
        );
    }
    let parts: Vec<&str> = p
        .strip_prefix("/help_center/")
        .unwrap_or("")
        .split('/')
        .collect();
    match (m.as_str(), &parts[..]) {
        ("GET", [res, id, "translations.json"]) => {
            let prefix = format!("{res}:{id}:");
            let list: Vec<Value> = f.data["translations"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .map(|(_, t)| t.clone())
                .collect();
            (StatusCode::OK, json!({"translations": list}))
        }
        ("DELETE", ["translations", id]) => {
            let id: i64 = id.trim_end_matches(".json").parse().unwrap();
            f.data["translations"]
                .as_object_mut()
                .unwrap()
                .retain(|_, t| t["id"] != id);
            (StatusCode::NO_CONTENT, Value::Null)
        }
        _ => zendesk(f, m, path, q, body, h),
    }
}

#[tokio::test]
async fn zendesk_current_remove_doctor() {
    init();
    let data = json!({"categories": [], "sections": [], "articles": [], "translations": {
        "articles:10:en-us": {"id": 1, "locale": "en-us", "title": "Reset your password", "body": "<p>Click</p>"},
        "articles:10:fr": {"id": 2, "locale": "fr", "title": "Réinitialiser", "body": "<p>Cliquez</p>"},
        "sections:2:fr": {"id": 3, "locale": "fr", "title": "FAQ (fr)", "body": ""},
        "articles:11:fr": {"id": 4, "locale": "fr", "title": "Facturation", "body": "<p>Écrit par un humain</p>"}}});
    let (url, s) = serve(data, zendesk_read).await;
    let c = registry("zendesk").unwrap();
    let cr = creds(
        json!({"subdomain": url, "email": "ed@acme.io", "token": "zt"}),
        json!({}),
    );
    let meta = |res: &str, id: i64| json!({"resource": res, "id": id});
    let items = [
        item("articles:10", "title", meta("articles", 10)),
        item("articles:10", "body", meta("articles", 10)),
        item("sections:2", "name", meta("sections", 2)),
        item("sections:2", "description", meta("sections", 2)),
        item("categories:1", "name", meta("categories", 1)),
    ];
    assert_eq!(
        c.current(&cr, "fr-FR", &items).await.unwrap().unwrap(),
        [
            Some("Réinitialiser".to_string()),
            Some("<p>Cliquez</p>".to_string()),
            Some("FAQ (fr)".to_string()),
            None,
            None
        ],
        "empty description and missing translation: none"
    );
    assert!(
        c.current(&cr, "de", &items).await.is_err(),
        "locale not enabled: nothing is assumed"
    );

    // A body glot wrote into a translation whose title it did not write: the translation stays.
    let body_only = [item("articles:10", "body", meta("articles", 10))];
    assert!(!c.remove(&cr, "fr", &body_only).await.unwrap());
    // A title glot wrote, but the body was written by someone else: stays too.
    let title_only = [item("articles:11", "title", meta("articles", 11))];
    assert!(!c.remove(&cr, "fr", &title_only).await.unwrap());
    assert_eq!(
        s.lock().unwrap().data["translations"]
            .as_object()
            .unwrap()
            .len(),
        4
    );
    // Translations glot created as a whole: deleted, and only those.
    assert!(c.remove(&cr, "fr", &items).await.unwrap());
    {
        let f = s.lock().unwrap();
        let left: Vec<&String> = f.data["translations"].as_object().unwrap().keys().collect();
        assert_eq!(left, ["articles:10:en-us", "articles:11:fr"]);
        let deletes: Vec<&str> = f
            .log
            .iter()
            .filter(|l| l.0 == "DELETE")
            .map(|l| l.1.as_str())
            .collect();
        assert_eq!(
            deletes,
            [
                "/api/v2/help_center/translations/2.json",
                "/api/v2/help_center/translations/3.json"
            ]
        );
    }

    assert_eq!(
        statuses(&c.doctor(&cr).await),
        [("role", "ok", None), ("help_center_locales", "ok", None)]
    );
    {
        let mut f = s.lock().unwrap();
        f.data["role"] = json!("agent");
        f.data["locales"] = json!(["en-us"]);
        f.log.clear();
    }
    let checks = c.doctor(&cr).await;
    assert_eq!(
        statuses(&checks),
        [
            ("role", "warn", Some("permissions")),
            ("help_center_locales", "warn", Some("enable_locale"))
        ]
    );
    assert!(checks[1].detail.starts_with("only en-us is enabled"));
    assert!(s.lock().unwrap().log.iter().all(|l| l.0 == "GET"));
}

/// HubSpot with the private app token info endpoint (`data.scopes`).
fn hubspot_token(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    if *m == Method::POST && path == "/oauth/v2/private-apps/get/access-token-info" {
        assert_eq!(body["tokenKey"], "pat-na1-x");
        return (
            StatusCode::OK,
            json!({"userId": 1, "hubId": 2, "appId": 3, "scopes": f.data["scopes"]}),
        );
    }
    hubspot(f, m, path, q, body, h)
}

#[tokio::test]
async fn hubspot_current_doctor() {
    init();
    let post = |id: &str, name: &str, from: Value, lang: &str, tr: Value| {
        json!({"id": id, "name": name, "htmlTitle": format!("{name} | Blog"), "metaDescription": "", "postSummary": "",
            "postBody": "<p>Hello <strong>you</strong></p>",
            "language": lang, "translatedFromId": from, "translations": tr, "archivedInDashboard": false})
    };
    let data = json!({
        "scopes": ["content", "oauth"],
        "/cms/v3/blogs/posts": [
            post("100", "Hello", Value::Null, "en", json!({"fr": {"id": "101"}})),
            // The variation HubSpot cloned from the source: only its name and title were translated.
            post("101", "Bonjour", json!("100"), "fr", json!({})),
            post("102", "Second post", Value::Null, "en", json!({}))],
        "/cms/v3/pages/site-pages": [], "/cms/v3/pages/landing-pages": []});
    let (url, s) = serve(data, hubspot_token).await;
    let c = registry("hubspot").unwrap();
    let cr = creds(json!({"token": "pat-na1-x"}), json!({"api_url": url}));
    let meta = |id: &str| json!({"resource": "blog_posts", "id": id});
    let items = [
        item("blog_posts:100", "name", meta("100")),
        item("blog_posts:100", "htmlTitle", meta("100")),
        item("blog_posts:100", "postBody", meta("100")),
        item("blog_posts:102", "name", meta("102")),
    ];
    assert_eq!(
        c.current(&cr, "fr", &items).await.unwrap().unwrap(),
        [
            Some("Bonjour".to_string()),
            Some("Bonjour | Blog".to_string()),
            None,
            None
        ],
        "the cloned body is not a translation; 102 has no variation"
    );
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());
    assert_eq!(
        s.lock().unwrap().data["/cms/v3/blogs/posts"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "current creates no variation"
    );

    assert_eq!(
        statuses(&c.doctor(&cr).await),
        [("scopes", "ok", None), ("multi_language", "ok", None)]
    );
    s.lock().unwrap().data["scopes"] = json!(["crm.objects.contacts.read"]);
    let checks = c.doctor(&cr).await;
    assert_eq!(
        (checks[0].status, checks[0].fix, checks[0].detail.as_str()),
        (
            "fail",
            Some("permissions"),
            "missing scope: content (CMS pages and blog posts)"
        )
    );
}

#[tokio::test]
async fn ghost_current_doctor() {
    init();
    let lexical = json!({"root": {"type": "root", "children": [{"type": "paragraph", "children": [
        {"type": "extended-text", "text": "Installez avec ", "format": 0},
        {"type": "extended-text", "text": "npm i glot", "format": 16}]}]}});
    let data = json!({
        "posts": [
            {"id": "p1", "slug": "hello", "title": "Hello", "custom_excerpt": "Short intro", "tags": [{"name": "News"}]},
            {"id": "p2", "slug": "hello-fr", "title": "Bonjour", "custom_excerpt": null, "lexical": lexical.to_string(), "tags": [{"name": "#fr"}]}],
        "pages": [], "webhooks": []});
    let (url, s) = serve(data, ghost).await;
    let c = registry("ghost").unwrap();
    let cr = creds(json!({"url": url, "admin_key": GHOST_KEY}), json!({}));
    let meta = |slug: &str| json!({"resource": "posts", "id": "x", "slug": slug, "tags": []});
    let mut body = item("posts:p1", "lexical", meta("hello"));
    body.format = Format::RichText(json!({}));
    let items = [
        item("posts:p1", "title", meta("hello")),
        item("posts:p1", "custom_excerpt", meta("hello")),
        body,
        item("posts:p3", "title", meta("second")),
    ];
    let cur = c.current(&cr, "fr", &items).await.unwrap().unwrap();
    assert_eq!(cur[0].as_deref(), Some("Bonjour"));
    assert_eq!(cur[1], None, "no excerpt on the translated post");
    // The Lexical document in the shape push receives: inline code protected, restored on push.
    let doc: Value = serde_json::from_str(cur[2].as_deref().unwrap()).unwrap();
    let nodes = &doc["root"]["children"][0]["children"];
    assert_eq!(nodes[0]["type"], "extended-text");
    assert_eq!(nodes[1]["type"], "glot:code:extended-text");
    assert_eq!(cur[3], None, "no second-fr post");
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());

    let checks = c.doctor(&cr).await;
    assert_eq!(
        statuses(&checks),
        [("version", "ok", None), ("language_tags", "ok", None)]
    );
    assert_eq!(
        (checks[0].detail.as_str(), checks[1].detail.as_str()),
        ("Ghost 5.100", "#fr")
    );
    assert!(s.lock().unwrap().log.iter().all(|l| l.0 == "GET"));
}

#[tokio::test]
async fn drupal_current_doctor() {
    init();
    let node = |lang: &str, title: &str, body: &str| {
        json!({"type": "node--article", "id": "aaaa-0001", "attributes": {"langcode": lang, "title": title,
            "content_translation_source": "en", "content_translation_outdated": false,
            "body": {"value": body, "format": "basic_html", "summary": ""}}})
    };
    let mut other = node("en", "Untranslated", "<p>Text</p>");
    other["id"] = json!("bbbb-0002");
    let data = json!({
        // The fr translation was added in the Translate tab: its title was translated, its body is still the source's.
        "node--article": [{"en": node("en", "Hello", "<p>Hello</p>"), "fr": node("fr", "Bonjour", "<p>Hello</p>")}, {"en": other}],
        // Content Translation is not enabled for taxonomy terms: no content_translation_* field.
        "taxonomy_term--tags": [{"en": {"type": "taxonomy_term--tags", "id": "cccc-0003", "attributes": {"langcode": "en", "name": "Travel"}}}]});
    let (url, s) = serve(data, drupal).await;
    let c = registry("drupal").unwrap();
    let cr = creds(
        json!({"url": url, "username": "editor", "password": "pw"}),
        json!({}),
    );
    let meta =
        |id: &str| json!({"type": "node--article", "id": id, "formats": {"body": "basic_html"}});
    let items = [
        item("node--article:aaaa-0001", "title", meta("aaaa-0001")),
        item("node--article:aaaa-0001", "body.value", meta("aaaa-0001")),
        item("node--article:aaaa-0001", "body.summary", meta("aaaa-0001")),
        item("node--article:bbbb-0002", "title", meta("bbbb-0002")),
    ];
    assert_eq!(
        c.current(&cr, "fr", &items).await.unwrap().unwrap(),
        [Some("Bonjour".to_string()), None, None, None],
        "source text left in the translation, and Drupal's fallback to English, are not translations"
    );
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());

    let checks = c.doctor(&cr).await;
    assert_eq!(
        statuses(&checks),
        [
            ("language_modules", "ok", None),
            ("content_translation", "warn", Some("install_plugin"))
        ]
    );
    assert!(
        checks[1]
            .detail
            .starts_with("Content Translation is not enabled for: taxonomy_term "),
        "{}",
        checks[1].detail
    );
    assert!(s.lock().unwrap().log.iter().all(|l| l.0 == "GET"));
}

/// Craft with per-site content (`data.french` = overrides by entry id), and the schema's mutations.
fn craft_sites_mock(
    f: &mut Fake,
    m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    body: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    let query = body["query"].as_str().unwrap_or("");
    let v = &body["variables"];
    let site = v["s"][0].as_str().unwrap_or("default");
    if query.contains("__schema") {
        return (
            StatusCode::OK,
            json!({"data": {"__schema": {"mutationType": {"fields": f.data["mutations"]}}}}),
        );
    }
    if query.contains("limit: 1") && site != "default" {
        let known = site == "french";
        return (
            StatusCode::OK,
            json!({"data": {"entries": if known { json!([{"siteId": 2}]) } else { json!([]) }}}),
        );
    }
    if query.contains("$ids") {
        let ids = v["ids"].as_array().unwrap();
        let out: Vec<Value> = f.data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| ids.contains(&e["id"]))
            .map(|e| {
                let mut e = e.clone();
                let over = &f.data[site][e["id"].as_str().unwrap()];
                for (k, val) in over.as_object().into_iter().flatten() {
                    e[k] = val.clone();
                }
                e["__typename"] = json!("blog_article_Entry");
                e
            })
            .collect();
        return (StatusCode::OK, json!({"data": {"entries": out}}));
    }
    craft(f, m, path, q, body, h)
}

#[tokio::test]
async fn craft_current_doctor() {
    init();
    let entry = |id: &str| json!({"id": id, "title": format!("Post {id}"), "summary": "Plain summary", "body": "<p>Body</p>"});
    let data = json!({"entries": [entry("1"), entry("2")], "saved": [],
        // Entry 1 is translated in the French site, except its body; entry 2 only holds the propagated content.
        "french": {"1": {"title": "Article 1", "summary": "Résumé"}},
        "mutations": [{"name": "save_blog_article_Entry"}, {"name": "deleteEntry"}]});
    let (url, s) = serve(data, craft_sites_mock).await;
    let c = registry("craft").unwrap();
    let cr = creds(
        json!({"url": format!("{url}/api"), "token": "craft-token"}),
        json!({"sites": {"fr": "french", "de": "german"}}),
    );
    let meta = |id: &str| json!({"id": id, "section": "blog", "type": "article"});
    let items = [
        item("entry:1", "title", meta("1")),
        item("entry:1", "summary", meta("1")),
        item("entry:1", "body", meta("1")),
        item("entry:2", "title", meta("2")),
    ];
    assert_eq!(
        c.current(&cr, "fr", &items).await.unwrap().unwrap(),
        [
            Some("Article 1".to_string()),
            Some("Résumé".to_string()),
            None,
            None
        ],
        "content propagated from the source site is not a translation"
    );
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());

    let checks = c.doctor(&cr).await;
    assert_eq!(
        statuses(&checks),
        [
            ("sites", "warn", Some("enable_locale")),
            ("mutations", "ok", None)
        ]
    );
    assert!(checks[0].detail.contains("german (de)") && !checks[0].detail.contains("french"));
    s.lock().unwrap().data["mutations"] = json!([{"name": "deleteEntry"}]);
    let checks = c.doctor(&cr).await;
    assert_eq!(
        (checks[1].id.as_str(), checks[1].status, checks[1].fix),
        ("mutations", "fail", Some("permissions"))
    );
    assert_eq!(s.lock().unwrap().data["saved"], json!([]), "nothing saved");
}

#[tokio::test]
async fn helpscout_current_doctor() {
    init();
    let data = json!({"articles": [
        {"id": "a1", "collectionId": "cen", "slug": "getting-started", "name": "Getting started", "text": "<p>Open</p>"},
        {"id": "f1", "collectionId": "cfr", "slug": "getting-started", "name": "Bien démarrer", "text": "<p>Ouvrez</p>"}]});
    let (url, s) = serve(data, helpscout).await;
    let c = registry("helpscout").unwrap();
    let cr = creds(
        json!({"api_key": "hskey", "collections": "fr: cfr"}),
        json!({"api_url": url}),
    );
    let meta = |slug: &str| json!({"id": "x", "slug": slug, "collection": "cen"});
    let items = [
        item("article:a1", "name", meta("getting-started")),
        item("article:a1", "text", meta("getting-started")),
        item("article:a2", "name", meta("billing")),
    ];
    assert_eq!(
        c.current(&cr, "fr", &items).await.unwrap().unwrap(),
        [
            Some("Bien démarrer".to_string()),
            Some("<p>Ouvrez</p>".to_string()),
            None
        ]
    );
    assert!(
        c.current(&cr, "de", &items).await.is_err(),
        "no collection for de"
    );
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());
    assert!(
        s.lock()
            .unwrap()
            .log
            .iter()
            .any(|l| l.1 == "/v1/articles/f1?draft=true"),
        "the draft holds the last write"
    );

    assert_eq!(
        statuses(&c.doctor(&cr).await),
        [("collections", "ok", None)]
    );
    let unmapped = creds(json!({"api_key": "hskey"}), json!({"api_url": url}));
    assert_eq!(
        statuses(&c.doctor(&unmapped).await),
        [("collections", "warn", Some("enable_locale"))]
    );
    let wrong = creds(
        json!({"api_key": "hskey", "collections": "fr: cfr, de: gone"}),
        json!({"api_url": url}),
    );
    let checks = c.doctor(&wrong).await;
    assert_eq!(
        (checks[0].status, checks[0].fix, checks[0].detail.as_str()),
        ("fail", Some("not_found"), "collection not found: de → gone")
    );
    assert!(s.lock().unwrap().log.iter().all(|l| l.0 == "GET"));
}
