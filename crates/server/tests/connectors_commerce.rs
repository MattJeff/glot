//! `current` / `remove` / `doctor` of the WordPress, WooCommerce, BigCommerce, PrestaShop, Magento and Shopware
//! connectors against simulated servers that follow the official APIs:
//! - WordPress REST: https://developer.wordpress.org/rest-api/reference/posts/ (`include`),
//!   https://developer.wordpress.org/rest-api/reference/users/ (`capabilities`),
//!   https://developer.wordpress.org/rest-api/reference/plugins/, https://polylang.pro/doc/rest-api/
//! - WooCommerce REST: https://woocommerce.github.io/woocommerce-rest-api-docs/#system-status-properties
//! - BigCommerce Translations: https://docs.bigcommerce.com/developer/docs/admin/store-configuration/translations/products
//! - PrestaShop Webservice: https://devdocs.prestashop-project.org/8/webservice/tutorials/testing-access/,
//!   https://devdocs.prestashop-project.org/8/webservice/cheat-sheet/
//! - Magento REST: https://developer.adobe.com/commerce/webapi/rest/use-rest/performing-searches/
//! - Shopware Admin API: https://developer.shopware.com/docs/guides/integrations-api/general-concepts/search-criteria.html
//!
//! Every read (`current`, `doctor`) is also checked to send nothing but GETs (or the POSTs that are reads:
//! GraphQL queries, Shopware searches).
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use glot_server::connectors::{Check, Creds, Format, PushItem, registry};
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
    /// Scenario switches, changed by the tests between calls.
    data: Value,
    /// (method, path, decoded query, JSON body).
    log: Vec<(String, String, HashMap<String, String>, Value)>,
}
type Shared = Arc<Mutex<Fake>>;
/// A JSON string is sent as a raw body (XML).
type Handler =
    fn(&Fake, &Method, &str, &HashMap<String, String>, &Value, &HeaderMap) -> (StatusCode, Value);

async fn serve(data: Value, h: Handler) -> (String, Shared) {
    init();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let s: Shared = Arc::new(Mutex::new(Fake { data, log: vec![] }));
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
                    .push((m.to_string(), u.path().to_string(), q.clone(), body.clone()));
                let (st, v) = h(&f, &m, u.path(), &q, &body, &hd);
                let r: Response = match v {
                    Value::String(raw) => (st, raw).into_response(),
                    v => (st, axum::Json(v)).into_response(),
                };
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

fn item(external_id: &str, field: &str, meta: Value) -> PushItem {
    PushItem {
        external_id: external_id.into(),
        field: field.into(),
        value: "glot".into(),
        format: Format::Plain,
        meta,
    }
}

fn check<'a>(checks: &'a [Check], id: &str) -> &'a Check {
    checks
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no check '{id}' in {checks:?}"))
}

/// Methods and paths logged so far, then cleared.
fn calls(s: &Shared) -> Vec<String> {
    s.lock()
        .unwrap()
        .log
        .drain(..)
        .map(|(m, p, _, _)| format!("{m} {p}"))
        .collect()
}

fn set(s: &Shared, data: Value) {
    s.lock().unwrap().data = data;
}

fn not_found() -> (StatusCode, Value) {
    (StatusCode::NOT_FOUND, json!({"code": "rest_no_route"}))
}

// --------------------------------------------------------------------------------------- WordPress

/// A Gutenberg post: block comments with attributes, a shortcode, an HTML entity, inline markup.
const GUTENBERG: &str = "<!-- wp:heading {\"level\":3,\"className\":\"is-style-x\"} -->\n<h3 class=\"wp-block-heading is-style-x\">Notre histoire &amp; nous</h3>\n<!-- /wp:heading -->\n\n<!-- wp:shortcode -->\n[contact-form-7 id=\"42\" title=\"Contact &quot;FR&quot;\"]\n<!-- /wp:shortcode -->\n\n<!-- wp:paragraph -->\n<p>Bonjour <strong>le monde</strong>&nbsp;!</p>\n<!-- /wp:paragraph -->";

fn wp_post(id: i64) -> Option<Value> {
    let field = |s: &str| json!({"raw": s, "rendered": format!("<p>{s}</p>")});
    let post = |lang: &str, tr: Value, title: &str, content: &str| {
        json!({"id": id, "lang": lang, "status": "publish", "translations": tr,
            "title": field(title), "content": field(content), "excerpt": field("")})
    };
    Some(match id {
        12 => post(
            "en",
            json!({"en": 12, "fr": 14}),
            "Our story",
            "<p>Hello</p>",
        ),
        14 => post(
            "fr",
            json!({"en": 12, "fr": 14}),
            "Notre histoire",
            GUTENBERG,
        ),
        13 => post(
            "en",
            json!({"en": 13}),
            "Untranslated",
            "<p>Nothing yet</p>",
        ),
        // A duplicate nobody translated: same values as its source.
        15 => post("en", json!({"en": 15, "fr": 16}), "Copied", "<p>Same</p>"),
        16 => post("fr", json!({"en": 15, "fr": 16}), "Copied", "<p>Same</p>"),
        _ => return None,
    })
}

fn wordpress(
    f: &Fake,
    _m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    _b: &Value,
    _h: &HeaderMap,
) -> (StatusCode, Value) {
    let ok = |v| (StatusCode::OK, v);
    match path.strip_prefix("/wp-json").unwrap_or(path) {
        "/" => ok(json!({"name": "Blog", "namespaces": f.data["namespaces"]})),
        "/wp/v2/users/me" => {
            assert_eq!(q["context"], "edit");
            ok(json!({"id": 1, "roles": ["author"], "capabilities": f.data["capabilities"]}))
        }
        "/wp/v2/plugins" => {
            assert_eq!(q["status"], "active");
            ok(f.data["plugins"].clone())
        }
        "/wp/v2/posts" => {
            assert_eq!(q["context"], "edit");
            assert!(q["status"].contains("draft"), "drafts must be listed");
            // 14 is left out of lists (a status or language the list hides): read alone.
            ok(q["include"]
                .split(',')
                .filter_map(|id| id.parse::<i64>().ok())
                .filter(|id| *id != 14)
                .filter_map(wp_post)
                .collect())
        }
        p => match p
            .strip_prefix("/wp/v2/posts/")
            .and_then(|id| wp_post(id.parse().ok()?))
        {
            Some(post) => ok(post),
            None => not_found(),
        },
    }
}

#[tokio::test]
async fn wordpress_current_and_doctor() {
    let editor = json!({"edit_posts": true, "edit_others_posts": true, "edit_pages": true, "edit_others_pages": true,
        "publish_posts": true, "publish_pages": true, "manage_categories": true});
    let (url, s) = serve(
        json!({"namespaces": ["wp/v2", "pll/v1"], "capabilities": editor}),
        wordpress,
    )
    .await;
    let c = registry("wordpress").unwrap();
    let cr = creds(
        json!({"url": url, "username": "bot", "app_password": "abcd efgh"}),
        json!({}),
    );
    let post = |id: i64, field: &str| {
        item(
            &format!("posts:{id}"),
            field,
            json!({"type": "posts", "id": id}),
        )
    };
    let items = [
        post(12, "title"),
        post(12, "content"),
        post(12, "excerpt"),
        post(13, "title"),
        post(15, "title"),
    ];
    let cur = c.current(&cr, "fr", &items).await.unwrap().unwrap();
    assert_eq!(cur[0].as_deref(), Some("Notre histoire"));
    // The Gutenberg document comes back byte for byte: comments, attributes, shortcode, entities.
    assert_eq!(cur[1].as_deref(), Some(GUTENBERG));
    assert_eq!(cur[2], None, "empty excerpt");
    assert_eq!(cur[3], None, "no translation linked");
    assert_eq!(cur[4], None, "a copy of the source is not a translation");
    // Two lists (sources, translations) and the one translation the list left out.
    assert_eq!(
        calls(&s),
        [
            "GET /wp-json/wp/v2/posts",
            "GET /wp-json/wp/v2/posts",
            "GET /wp-json/wp/v2/posts/14"
        ]
    );
    // What `current` returns is what `push` would write back: the value is the raw field.
    assert_eq!(
        cur[1],
        wp_post(14).unwrap()["content"]["raw"]
            .as_str()
            .map(str::to_string)
    );

    // A translation is a whole post: never deleted.
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());
    assert!(calls(&s).is_empty());

    let checks = c.doctor(&cr).await;
    assert_eq!(check(&checks, "permissions").status, "ok");
    assert_eq!(check(&checks, "plugin").status, "ok");
    assert_eq!(check(&checks, "plugin").detail, "Polylang Pro");

    // An Author with free Polylang: both problems named.
    set(
        &s,
        json!({"namespaces": ["wp/v2"], "capabilities": {"edit_posts": true, "publish_posts": true},
            "plugins": [{"plugin": "polylang/polylang", "status": "active", "name": "Polylang"}]}),
    );
    calls(&s);
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "permissions");
    assert_eq!((p.status, p.fix), ("fail", Some("permissions")));
    assert!(
        p.detail
            .contains("edit_others_posts, edit_pages, edit_others_pages"),
        "{}",
        p.detail
    );
    let p = check(&checks, "plugin");
    assert_eq!((p.status, p.fix), ("fail", Some("install_plugin")));
    assert!(
        p.detail.contains("Polylang (free)") && p.detail.contains("Polylang Pro"),
        "{}",
        p.detail
    );
    assert!(calls(&s).iter().all(|c| c.starts_with("GET ")));

    // An Editor who cannot manage terms, and no plugin at all.
    let mut caps = editor.clone();
    caps["manage_categories"] = json!(false);
    set(
        &s,
        json!({"namespaces": ["wp/v2"], "capabilities": caps, "plugins": []}),
    );
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "permissions");
    assert_eq!(
        (p.status, p.detail.as_str()),
        ("warn", "the user lacks manage_categories")
    );
    assert!(
        check(&checks, "plugin")
            .detail
            .starts_with("no multilingual plugin")
    );
}

// ------------------------------------------------------------------------------------- WooCommerce

fn woocommerce(
    f: &Fake,
    _m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    _b: &Value,
    _h: &HeaderMap,
) -> (StatusCode, Value) {
    let ok = |v| (StatusCode::OK, v);
    let product = |id: i64| {
        Some(match id {
            1 => {
                json!({"id": 1, "lang": "en", "name": "Shirt", "description": "<p>Soft</p>", "translations": {"en": 1, "fr": 5}})
            }
            5 => {
                json!({"id": 5, "lang": "fr", "name": "Chemise", "description": "<p>Soft</p>", "translations": {"en": 1, "fr": 5}})
            }
            2 => {
                json!({"id": 2, "lang": "en", "name": "Hat", "description": "", "translations": {"en": 2}})
            }
            _ => return None,
        })
    };
    match path.strip_prefix("/wp-json").unwrap_or(path) {
        "/" => ok(json!({"name": "Shop", "namespaces": ["wp/v2", "wc/v3", "wpml/v1"]})),
        "/wc/v3/system_status" => match f.data["plugins"].as_array() {
            Some(p) => ok(json!({"active_plugins": p})),
            None => (
                StatusCode::UNAUTHORIZED,
                json!({"code": "woocommerce_rest_cannot_view", "message": "Sorry, you cannot list resources."}),
            ),
        },
        "/wc/v3/products" => ok(q["include"]
            .split(',')
            .filter_map(|id| product(id.parse().ok()?))
            .collect()),
        _ => not_found(),
    }
}

#[tokio::test]
async fn woocommerce_current_and_doctor() {
    let plugin = |file: &str, name: &str| json!({"plugin": file, "name": name, "version": "1.0"});
    let wpml = plugin(
        "sitepress-multilingual-cms/sitepress.php",
        "WPML Multilingual CMS",
    );
    let (url, s) = serve(json!({"plugins": [wpml.clone()]}), woocommerce).await;
    let c = registry("woocommerce").unwrap();
    let cr = creds(
        json!({"url": url, "consumer_key": "ck_test", "consumer_secret": "cs_test"}),
        json!({"languages": ["en", "fr"], "source_lang": "en"}),
    );
    let product = |id: i64, field: &str| {
        item(
            &format!("product:{id}"),
            field,
            json!({"collection": "products", "id": id}),
        )
    };
    let items = [
        product(1, "name"),
        product(1, "description"),
        product(2, "name"),
    ];
    let cur = c.current(&cr, "fr", &items).await.unwrap().unwrap();
    assert_eq!(cur, [Some("Chemise".to_string()), None, None]);
    // Sources then translations, both by `include`: two calls for the batch.
    assert_eq!(
        calls(&s),
        ["GET /wp-json/wc/v3/products", "GET /wp-json/wc/v3/products"]
    );
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());

    // WPML without WooCommerce Multilingual: named.
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "plugin");
    assert_eq!((p.status, p.fix), ("warn", Some("install_plugin")));
    assert!(
        p.detail.contains("without WooCommerce Multilingual"),
        "{}",
        p.detail
    );

    set(
        &s,
        json!({"plugins": [wpml, plugin("woocommerce-multilingual/wpml-woocommerce.php", "WooCommerce Multilingual & Multicurrency")]}),
    );
    let checks = c.doctor(&cr).await;
    assert_eq!(check(&checks, "plugin").status, "ok");
    assert!(checks.iter().all(|c| c.id != "permissions"));

    // A key whose user cannot read the system status: said, the plugin is still judged from the REST index.
    set(&s, json!({}));
    calls(&s);
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "permissions");
    assert_eq!((p.status, p.fix), ("warn", Some("permissions")));
    assert_eq!(check(&checks, "plugin").status, "ok");
    assert!(calls(&s).iter().all(|c| c.starts_with("GET ")));
}

// ------------------------------------------------------------------------------------- BigCommerce

fn bigcommerce(
    f: &Fake,
    m: &Method,
    path: &str,
    _q: &HashMap<String, String>,
    b: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    assert_eq!(h["x-auth-token"], "tok");
    assert_eq!((m, path), (&Method::POST, "/graphql"));
    let query = b["query"].as_str().unwrap();
    let data = if query.contains("locales(") {
        let mut edges =
            vec![json!({"node": {"code": "en", "status": "ACTIVE", "isDefault": true}})];
        if f.data["locales"] != "default_only" {
            edges.push(json!({"node": {"code": "fr", "status": "ACTIVE", "isDefault": false}}));
        }
        json!({"store": {"locales": {"edges": edges}}})
    } else if query.contains("translations(filters: $filters, first: 50)") {
        if f.data["scope"] == "missing" {
            return (
                StatusCode::FORBIDDEN,
                json!({"status": 403, "title": "You don't have a required scope to access the endpoint"}),
            );
        }
        // Only translated resources come back; untranslated fields carry the default value.
        json!({"store": {"translations": {"edges": [{"cursor": "x", "node": {"resourceId": "bc/store/product/1", "fields": [
            {"fieldName": "name", "original": "Shirt", "translation": "Chemise"},
            {"fieldName": "description", "original": "<p>Soft</p>", "translation": "<p>Soft</p>"},
            {"fieldName": "page_title", "original": "Shirt | Shop", "translation": null}]}}]}}})
    } else if query.contains("deleteTranslations(input: $input)") {
        json!({"translation": {"deleteTranslations": {"__typename": "DeleteTranslationsResult", "errors": []}}})
    } else {
        panic!("unexpected query {query}")
    };
    (StatusCode::OK, json!({"data": data}))
}

#[tokio::test]
async fn bigcommerce_current_remove_doctor() {
    let (url, s) = serve(json!({}), bigcommerce).await;
    let c = registry("bigcommerce").unwrap();
    let cr = creds(
        json!({"store_hash": url, "token": "tok"}),
        json!({"channel_id": "3"}),
    );
    let product = |id: i64, field: &str| {
        item(
            &format!("product:{id}"),
            field,
            json!({"type": "PRODUCTS", "rid": format!("bc/store/product/{id}")}),
        )
    };
    let items = [
        product(1, "name"),
        product(1, "description"),
        product(1, "page_title"),
        product(2, "name"),
    ];
    let cur = c.current(&cr, "fr", &items).await.unwrap().unwrap();
    assert_eq!(cur, [Some("Chemise".to_string()), None, None, None]);
    {
        let f = s.lock().unwrap();
        assert_eq!(f.log.len(), 1, "one query for the batch");
        assert_eq!(
            f.log[0].3["variables"]["filters"],
            json!({"resourceType": "PRODUCTS", "channelId": "bc/store/channel/3", "localeId": "bc/store/locale/fr",
                "resourceIds": ["bc/store/product/1", "bc/store/product/2"]})
        );
    }
    calls(&s);

    // Rollback of a first write: exactly these fields, in this channel and locale.
    assert!(c.remove(&cr, "fr", &items[..2]).await.unwrap());
    {
        let f = s.lock().unwrap();
        assert_eq!(f.log.len(), 1);
        assert_eq!(
            f.log[0].3["variables"]["input"],
            json!({"resourceType": "PRODUCTS", "channelId": "bc/store/channel/3", "localeId": "bc/store/locale/fr",
                "resources": [{"resourceId": "bc/store/product/1", "fields": ["name", "description"]}]})
        );
    }
    calls(&s);

    let checks = c.doctor(&cr).await;
    assert_eq!(check(&checks, "channel").status, "ok");
    assert_eq!(check(&checks, "channel").detail, "channel 3: en → fr");
    assert_eq!(check(&checks, "permissions").status, "ok");

    set(&s, json!({"locales": "default_only", "scope": "missing"}));
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "channel");
    assert_eq!((p.status, p.fix), ("warn", Some("enable_locale")));
    assert!(
        p.detail.contains("channel 3") && p.detail.contains("(en)"),
        "{}",
        p.detail
    );
    let p = check(&checks, "permissions");
    assert_eq!((p.status, p.fix), ("fail", Some("permissions")));
    assert!(
        p.detail.contains("Store Translations") && p.detail.contains("HTTP 403"),
        "{}",
        p.detail
    );
    // The diagnosis only queries.
    let f = s.lock().unwrap();
    assert!(
        f.log
            .iter()
            .all(|l| !l.3["query"].as_str().unwrap().contains("mutation"))
    );
}

// -------------------------------------------------------------------------------------- PrestaShop

/// `GET /api/` per the docs: one element per reachable resource, one flag per method.
fn prestashop_root(limited: bool) -> String {
    let res = |name: &str, put: bool| {
        format!(
            "<{name} xlink:href=\"http://example.com/api/{name}\" get=\"true\" put=\"{put}\" post=\"false\" patch=\"false\" delete=\"false\" head=\"true\"><description>x</description></{name}>"
        )
    };
    let mut body = res("products", true) + &res("categories", !limited) + &res("languages", false);
    body += &res("content_management_system", true);
    if !limited {
        body += &res("manufacturers", true);
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<prestashop xmlns:xlink=\"http://www.w3.org/1999/xlink\"><api shopName=\"Prestashop\">{body}</api></prestashop>"
    )
}

fn prestashop(
    f: &Fake,
    _m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    _b: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    // Basic auth, key as user name, empty password ("KEY:").
    assert_eq!(h["authorization"], "Basic S0VZOg==");
    let ok = |v| (StatusCode::OK, v);
    let ml = |en: &str, fr: &str| json!([{"id": "1", "value": en}, {"id": "2", "value": fr}]);
    match path {
        "/api/" => ok(Value::String(prestashop_root(f.data["limited"] == true))),
        "/api/languages" => ok(json!({"languages": [
            {"id": 1, "iso_code": "en", "language_code": "en-us", "active": "1"},
            {"id": 2, "iso_code": "fr", "language_code": "fr-fr", "active": "1"}]})),
        "/api/configurations" => {
            ok(json!({"configurations": [{"name": "PS_LANG_DEFAULT", "value": "1"}]}))
        }
        "/api/products" => {
            assert_eq!(q["output_format"], "JSON");
            assert_eq!(q["display"], "[id,name,description]");
            assert_eq!(q["filter[id]"], "[1|2]");
            ok(json!({"products": [
                // The description is the copy PrestaShop made when French was added.
                {"id": 1, "name": ml("Shirt", "Chemise"), "description": ml("<p>Soft</p>", "<p>Soft</p>")},
                {"id": 2, "name": ml("Hat", ""), "description": ml("<p>Warm</p>", "<p>Chaud</p>")}]}))
        }
        _ => not_found(),
    }
}

#[tokio::test]
async fn prestashop_current_and_doctor() {
    let (url, s) = serve(json!({}), prestashop).await;
    let c = registry("prestashop").unwrap();
    let cr = creds(json!({"url": url, "api_key": "KEY"}), json!({}));
    let product = |id: i64, field: &str| item(&format!("products:{id}"), field, json!({}));
    let items = [
        product(1, "name"),
        product(1, "description"),
        product(2, "name"),
        product(2, "description"),
    ];
    let cur = c.current(&cr, "fr", &items).await.unwrap().unwrap();
    assert_eq!(
        cur,
        [
            Some("Chemise".to_string()),
            None,
            None,
            Some("<p>Chaud</p>".to_string())
        ]
    );
    let log = calls(&s);
    assert_eq!(
        log.iter().filter(|c| *c == "GET /api/products").count(),
        1,
        "{log:?}"
    );
    assert!(log.iter().all(|c| c.starts_with("GET ")));
    // A language the shop does not have: nothing there, no error.
    assert_eq!(
        c.current(&cr, "de", &items).await.unwrap().unwrap(),
        [None, None, None, None]
    );
    // An id that is not one is refused before any filter is built from it.
    assert!(
        c.current(&cr, "fr", &[item("products:1|9", "name", json!({}))])
            .await
            .is_err()
    );
    assert!(!c.remove(&cr, "fr", &items).await.unwrap());

    let checks = c.doctor(&cr).await;
    assert_eq!(check(&checks, "permissions").status, "ok");

    set(&s, json!({"limited": true}));
    calls(&s);
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "permissions");
    assert_eq!((p.status, p.fix), ("fail", Some("permissions")));
    assert_eq!(
        p.detail,
        "the webservice key lacks: categories PUT, manufacturers GET + PUT"
    );
    assert_eq!(calls(&s), ["GET /api/"]);
}

// ----------------------------------------------------------------------------------------- Magento

fn magento(
    f: &Fake,
    _m: &Method,
    path: &str,
    q: &HashMap<String, String>,
    _b: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    assert_eq!(h["authorization"], "Bearer tok");
    let ok = |v| (StatusCode::OK, v);
    let product = |name: &str, description: &str| {
        json!({"id": 7, "name": name, "custom_attributes": [{"attribute_code": "description", "value": description},
            {"attribute_code": "meta_title", "value": "Shirt"}]})
    };
    let by_ids = || {
        let f = "searchCriteria[filter_groups][0][filters][0]";
        assert_eq!(q[&format!("{f}[field]")], "entity_id");
        assert_eq!(q[&format!("{f}[condition_type]")], "in");
        assert_eq!(q[&format!("{f}[value]")], "7,8");
        assert_eq!(q["fields"], "items[id,name,custom_attributes]");
    };
    match path {
        "/rest/V1/store/storeConfigs" => {
            let fr = if f.data["one_locale"] == true {
                "en_US"
            } else {
                "fr_FR"
            };
            ok(
                json!([{"id": 1, "code": "default", "locale": "en_US"}, {"id": 2, "code": "fr", "locale": fr}]),
            )
        }
        // Global scope: the default values. Product 8 has no store view value: both scopes answer alike.
        "/rest/all/V1/products" => {
            by_ids();
            ok(
                json!({"items": [product("Shirt", "<p>Soft</p>"), {"id": 8, "name": "Hat"}], "total_count": 2}),
            )
        }
        "/rest/fr/V1/products" => {
            by_ids();
            ok(
                json!({"items": [product("Chemise", "<p>Soft</p>"), {"id": 8, "name": "Hat"}], "total_count": 2}),
            )
        }
        "/rest/V1/categories/list" => {
            assert_eq!(q["searchCriteria[pageSize]"], "1");
            if f.data["acl"] == "no_categories" {
                // Magento's authorization error.
                return (
                    StatusCode::FORBIDDEN,
                    json!({"message": "The consumer isn't authorized to access %resources.",
                        "parameters": {"resources": "Magento_Catalog::categories"}}),
                );
            }
            ok(json!({"items": [], "total_count": 0}))
        }
        _ => not_found(),
    }
}

#[tokio::test]
async fn magento_current_and_doctor() {
    let (url, s) = serve(json!({}), magento).await;
    let c = registry("magento").unwrap();
    let cr = creds(json!({"url": url, "token": "tok"}), json!({}));
    let product = |id: i64, field: &str| {
        item(
            &format!("product:{id}"),
            field,
            json!({"type": "product", "id": id, "sku": format!("SKU-{id}")}),
        )
    };
    let items = [
        product(7, "name"),
        product(7, "description"),
        product(8, "name"),
        product(7, "meta_title"),
    ];
    let cur = c.current(&cr, "fr_FR", &items).await.unwrap().unwrap();
    // Only the value that differs from the global scope is a store view translation.
    assert_eq!(cur, [Some("Chemise".to_string()), None, None, None]);
    assert_eq!(
        calls(&s),
        [
            "GET /rest/V1/store/storeConfigs",
            "GET /rest/all/V1/products",
            "GET /rest/fr/V1/products"
        ]
    );
    // No store view for the locale: nothing there.
    assert_eq!(
        c.current(&cr, "de_DE", &items).await.unwrap().unwrap(),
        [None, None, None, None]
    );
    assert!(!c.remove(&cr, "fr_FR", &items).await.unwrap());

    let checks = c.doctor(&cr).await;
    let v = check(&checks, "store_views");
    assert_eq!((v.status, v.detail.as_str()), ("ok", "fr (fr_FR)"));
    assert_eq!(check(&checks, "permissions").status, "ok");

    set(&s, json!({"one_locale": true, "acl": "no_categories"}));
    calls(&s);
    let checks = c.doctor(&cr).await;
    let v = check(&checks, "store_views");
    assert_eq!((v.status, v.fix), ("warn", Some("enable_locale")));
    assert!(v.detail.contains("en_US"), "{}", v.detail);
    let p = check(&checks, "permissions");
    assert_eq!((p.status, p.fix), ("fail", Some("permissions")));
    assert!(
        p.detail.contains("Magento_Catalog::categories") && p.detail.contains("HTTP 403"),
        "{}",
        p.detail
    );
    assert!(calls(&s).iter().all(|c| c.starts_with("GET ")));
}

// ---------------------------------------------------------------------------------------- Shopware

const SW_SYSTEM: &str = "2fbb5fe2e29a4d70aa5854ce7ce3e20b";
const SW_DE: &str = "1f0c9a4e8f6a4d2b9c3e5a7b1d2e3f40";
const SW_FR: &str = "9d1c0a4e8f6a4d2b9c3e5a7b1d2e3f41";
const SW_P1: &str = "aa0c9a4e8f6a4d2b9c3e5a7b1d2e3f01";
const SW_P2: &str = "aa0c9a4e8f6a4d2b9c3e5a7b1d2e3f02";

fn shopware(
    f: &Fake,
    m: &Method,
    path: &str,
    _q: &HashMap<String, String>,
    b: &Value,
    h: &HeaderMap,
) -> (StatusCode, Value) {
    let ok = |v| (StatusCode::OK, v);
    assert_eq!(m, Method::POST, "searches and the token only");
    if path == "/api/oauth/token" {
        assert_eq!(b["grant_type"], "client_credentials");
        return ok(json!({"token_type": "Bearer", "expires_in": 600, "access_token": "tok-1"}));
    }
    assert_eq!(h["authorization"], "Bearer tok-1");
    match path {
        "/api/search/language" => ok(json!({"total": 3, "data": [
            {"id": SW_SYSTEM, "locale": {"code": "en-GB"}},
            {"id": SW_DE, "locale": {"code": "de-DE"}},
            {"id": SW_FR, "locale": {"code": "fr-FR"}}]})),
        "/api/search/product" => {
            assert_eq!(b["ids"], json!([SW_P1, SW_P2]));
            assert_eq!(
                b["associations"]["translations"]["filter"],
                json!([{"type": "equals", "field": "languageId", "value": SW_DE}])
            );
            // Product 2 has no German row; product 1's description is inherited (null).
            ok(json!({"total": 2, "data": [
                {"id": SW_P1, "translations": [{"languageId": SW_DE, "name": "Hemd", "description": null}]},
                {"id": SW_P2, "translations": []}]}))
        }
        "/api/search/integration" => {
            assert_eq!(
                b["filter"][0],
                json!({"type": "equals", "field": "accessKey", "value": "SWIAKEY"})
            );
            match &f.data["integration"] {
                Value::Null => (
                    StatusCode::FORBIDDEN,
                    json!({"errors": [{"code": "FRAMEWORK__MISSING_PRIVILEGE_ERROR", "detail": "{\"message\":\"Missing privilege\",\"missingPrivileges\":[\"integration:read\"]}"}]}),
                ),
                i => ok(json!({"total": 1, "data": [i]})),
            }
        }
        "/api/search/sales-channel" => ok(json!({"total": 2, "data": [
            {"id": "c1", "active": true, "languages": [{"id": SW_SYSTEM}, {"id": SW_DE}]},
            // An inactive channel offers nothing.
            {"id": "c2", "active": false, "languages": [{"id": SW_FR}]}]})),
        _ => not_found(),
    }
}

#[tokio::test]
async fn shopware_current_and_doctor() {
    let (url, s) = serve(
        json!({"integration": {"admin": true, "aclRoles": []}}),
        shopware,
    )
    .await;
    let c = registry("shopware").unwrap();
    let cr = creds(
        json!({"url": url, "client_id": "SWIAKEY", "client_secret": "secret"}),
        json!({}),
    );
    let product = |id: &str, field: &str| {
        item(
            &format!("product:{id}"),
            field,
            json!({"entity": "product", "id": id}),
        )
    };
    let items = [
        product(SW_P1, "name"),
        product(SW_P1, "description"),
        product(SW_P2, "name"),
    ];
    let cur = c.current(&cr, "de-DE", &items).await.unwrap().unwrap();
    assert_eq!(cur, [Some("Hemd".to_string()), None, None]);
    assert_eq!(
        calls(&s),
        [
            "POST /api/oauth/token",
            "POST /api/search/language",
            "POST /api/search/product"
        ]
    );
    // The default language is the source, an unknown one has nothing: no search at all.
    assert_eq!(
        c.current(&cr, "en-GB", &items).await.unwrap().unwrap(),
        [None, None, None]
    );
    assert_eq!(
        c.current(&cr, "it-IT", &items).await.unwrap().unwrap(),
        [None, None, None]
    );
    assert!(calls(&s).iter().all(|c| !c.ends_with("/product")));
    assert!(!c.remove(&cr, "de-DE", &items).await.unwrap());

    // Administrator integration; French is offered by no active sales channel.
    let checks = c.doctor(&cr).await;
    assert_eq!(
        check(&checks, "permissions").detail,
        "administrator integration"
    );
    let ch = check(&checks, "sales_channels");
    assert_eq!((ch.status, ch.fix), ("warn", Some("enable_locale")));
    assert_eq!(ch.detail, "offered by no sales channel: fr-FR");

    // A role that reads everything but updates only categories and properties.
    let mut privileges = vec!["language:read".to_string(), "locale:read".into()];
    for e in [
        "product",
        "category",
        "property_group",
        "property_group_option",
    ] {
        privileges.extend([format!("{e}:read"), format!("{e}_translation:read")]);
        if e != "product" {
            privileges.push(format!("{e}:update"));
        }
    }
    set(
        &s,
        json!({"integration": {"admin": false, "aclRoles": [{"privileges": privileges}]}}),
    );
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "permissions");
    assert_eq!((p.status, p.fix), ("fail", Some("permissions")));
    assert_eq!(p.detail, "the integration's role lacks: product:update");

    // An integration that cannot read its own record: said, not failed.
    set(&s, json!({}));
    let checks = c.doctor(&cr).await;
    let p = check(&checks, "permissions");
    assert_eq!(p.status, "warn");
    assert!(p.detail.contains("integration:read"), "{}", p.detail);
}
