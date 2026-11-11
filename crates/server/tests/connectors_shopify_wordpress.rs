//! CMS connectors end to end against simulated Shopify (GraphQL Admin API) and WordPress (REST API + Polylang
//! Pro) servers that follow the official docs: create (credentials tested, encrypted, never returned) →
//! sync (paginated pull, throttle retry) → `connector_translate` job (mock model) → push (digests, Gutenberg
//! block comments kept, Polylang linking) → idempotent re-push → webhook (HMAC / token) → source change
//! re-translated and updated in place. Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    Json, Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

type Log = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Clone, Default)]
struct Shop {
    log: Log,
    title: Arc<Mutex<String>>,
    throttled_once: Arc<Mutex<bool>>,
    locales: Arc<Mutex<Vec<String>>>,
}

fn digest(s: &str) -> String {
    glot_server::auth::sha256_hex(s)
}

async fn shopify_gql(
    State(s): State<Shop>,
    headers: HeaderMap,
    Json(b): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if headers["x-shopify-access-token"] != "shpat_test" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(
                json!({"errors": "[API] Invalid API key or access token (unrecognized login or wrong password)"}),
            ),
        );
    }
    (StatusCode::OK, shopify_data(s, b))
}

fn shopify_data(s: Shop, b: Value) -> Json<Value> {
    let q = b["query"].as_str().unwrap_or_default().to_string();
    let v = &b["variables"];
    s.log.lock().unwrap().push((q.clone(), v.clone()));
    let cost = json!({"cost": {"requestedQueryCost": 10, "actualQueryCost": 5,
        "throttleStatus": {"maximumAvailable": 1000.0, "currentlyAvailable": 990, "restoreRate": 50.0}}});
    let locales = || {
        let mut l = vec![json!({"locale": "en", "primary": true, "published": true})];
        l.extend(
            s.locales
                .lock()
                .unwrap()
                .iter()
                .map(|x| json!({"locale": x, "primary": false, "published": false})),
        );
        l
    };
    let data = if q.contains("translatableResources") {
        {
            let mut t = s.throttled_once.lock().unwrap();
            if !*t {
                *t = true;
                return Json(
                    json!({"errors": [{"message": "Throttled", "extensions": {"code": "THROTTLED"}}],
                    "extensions": {"cost": {"requestedQueryCost": 10, "throttleStatus": {"maximumAvailable": 1000.0, "currentlyAvailable": 0, "restoreRate": 1000.0}}}}),
                );
            }
        }
        let title = s.title.lock().unwrap().clone();
        let body = "<p>Soft <b>cotton</b> shirt.</p>";
        match (v["type"].as_str(), v["after"].as_str()) {
            // Two products, one per page: cursor pagination.
            (Some("PRODUCT"), None) => json!({"translatableResources": {
                "pageInfo": {"hasNextPage": true, "endCursor": "c1"},
                "nodes": [{"resourceId": "gid://shopify/Product/1", "translatableContent": [
                    {"key": "title", "value": title, "digest": digest(&title), "locale": "en", "type": "SINGLE_LINE_TEXT_FIELD"},
                    {"key": "body_html", "value": body, "digest": digest(body), "locale": "en", "type": "HTML"},
                    {"key": "handle", "value": "shirt", "digest": digest("shirt"), "locale": "en", "type": "URI"}]}]}}),
            (Some("PRODUCT"), Some("c1")) => json!({"translatableResources": {
                "pageInfo": {"hasNextPage": false, "endCursor": "c2"},
                "nodes": [{"resourceId": "gid://shopify/Product/2", "translatableContent": [
                    {"key": "title", "value": "Hat", "digest": digest("Hat"), "locale": "en", "type": "SINGLE_LINE_TEXT_FIELD"}]}]}}),
            (Some("COLLECTION"), None) => json!({"translatableResources": {
                "pageInfo": {"hasNextPage": false, "endCursor": null},
                "nodes": [{"resourceId": "gid://shopify/Collection/9", "translatableContent": [
                    {"key": "title", "value": "Summer", "digest": digest("Summer"), "locale": "en", "type": "SINGLE_LINE_TEXT_FIELD"}]}]}}),
            _ => {
                json!({"translatableResources": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": []}})
            }
        }
    } else if q.contains("translationsRegister") {
        // Digest must match the current source, as Shopify enforces.
        let title = s.title.lock().unwrap().clone();
        let mut errors = vec![];
        for (i, t) in v["translations"].as_array().unwrap().iter().enumerate() {
            let src = match (v["id"].as_str().unwrap(), t["key"].as_str().unwrap()) {
                ("gid://shopify/Product/1", "title") => title.clone(),
                ("gid://shopify/Product/1", "body_html") => {
                    "<p>Soft <b>cotton</b> shirt.</p>".into()
                }
                ("gid://shopify/Product/2", "title") => "Hat".into(),
                ("gid://shopify/Collection/9", "title") => "Summer".into(),
                _ => String::new(),
            };
            if t["translatableContentDigest"].as_str() != Some(&digest(&src)) {
                errors.push(json!({"field": ["translations", i.to_string(), "translatableContentDigest"], "message": "digest mismatch", "code": "INVALID_TRANSLATABLE_CONTENT"}));
            }
        }
        json!({"translationsRegister": {"translations": [], "userErrors": errors}})
    } else if q.contains("shopLocaleEnable") {
        s.locales
            .lock()
            .unwrap()
            .push(v["locale"].as_str().unwrap().into());
        json!({"shopLocaleEnable": {"shopLocale": {"locale": v["locale"], "published": false}, "userErrors": []}})
    } else if q.contains("webhookSubscriptionCreate") {
        json!({"webhookSubscriptionCreate": {"webhookSubscription": {"id": "gid://shopify/WebhookSubscription/1"}, "userErrors": []}})
    } else if q.contains("webhookSubscriptions") {
        json!({"webhookSubscriptions": {"nodes": []}})
    } else if q.contains("shop {") {
        json!({"shop": {"name": "Test Store"}, "shopLocales": locales()})
    } else if q.contains("shopLocales") {
        json!({"shopLocales": locales()})
    } else {
        panic!("unexpected query {q}")
    };
    Json(json!({"data": data, "extensions": cost}))
}

#[derive(Clone, Default)]
struct Wp {
    log: Log,
    content: Arc<Mutex<String>>,
    fr_id: Arc<Mutex<Option<i64>>>,
}

fn wp_auth(h: &HeaderMap) {
    use base64::Engine;
    let want = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("editor:abcdabcdabcdabcdabcdabcd")
    );
    assert_eq!(h[header::AUTHORIZATION], want.as_str());
}

fn wp_post(w: &Wp) -> Value {
    let mut tr = json!({"en": 7});
    if let Some(id) = *w.fr_id.lock().unwrap() {
        tr["fr"] = json!(id);
    }
    json!({"id": 7, "status": "publish", "lang": "en", "translations": tr,
        "title": {"raw": "Hello world", "rendered": "Hello world"},
        "content": {"raw": w.content.lock().unwrap().clone(), "rendered": ""},
        "excerpt": {"raw": "", "rendered": ""}})
}

async fn wordpress_mock(addr: &str, w: Wp) -> Router {
    let _ = addr;
    Router::new()
        .route("/wp-json/", get(|| async { Json(json!({"name": "My Blog", "namespaces": ["oembed/1.0", "wp/v2", "pll/v1"]})) }))
        .route("/wp-json/wp/v2/users/me", get(|h: HeaderMap| async move { wp_auth(&h); Json(json!({"id": 1, "name": "editor"})) }))
        .route("/wp-json/pll/v1/languages", get(|| async { Json(json!([{"slug": "en", "locale": "en_US", "is_default": true}, {"slug": "fr", "locale": "fr_FR"}])) }))
        .route(
            "/wp-json/wp/v2/{kind}",
            get(|State(w): State<Wp>, h: HeaderMap, axum::extract::Path(kind): axum::extract::Path<String>, Query(q): Query<HashMap<String, String>>| async move {
                wp_auth(&h);
                assert_eq!(q.get("context").map(String::as_str), Some("edit"));
                assert_eq!(q.get("lang").map(String::as_str), Some("en"), "only source-language content");
                let list = match kind.as_str() {
                    "posts" => json!([wp_post(&w), {"id": 100, "lang": "fr", "title": {"raw": "x"}, "content": {"raw": "x"}}]),
                    "categories" => json!([{"id": 3, "name": "News", "description": "", "lang": "en", "translations": {"en": 3}}]),
                    _ => json!([]),
                };
                ([("x-wp-totalpages", "1")], Json(list))
            })
            .post(|State(w): State<Wp>, h: HeaderMap, axum::extract::Path(kind): axum::extract::Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery, Json(b): Json<Value>| async move {
                wp_auth(&h);
                w.log.lock().unwrap().push((format!("POST /{kind}?{}", q.unwrap_or_default()), b));
                let id = if kind == "posts" { 100 } else { 30 };
                if kind == "posts" {
                    *w.fr_id.lock().unwrap() = Some(100);
                }
                (StatusCode::CREATED, Json(json!({"id": id})))
            }),
        )
        .route(
            "/wp-json/wp/v2/{kind}/{id}",
            get(|State(w): State<Wp>, axum::extract::Path((kind, _id)): axum::extract::Path<(String, i64)>| async move {
                Json(if kind == "posts" { wp_post(&w) } else { json!({"id": 3, "lang": "en", "translations": {"en": 3}}) })
            })
            .post(|State(w): State<Wp>, axum::extract::Path((kind, id)): axum::extract::Path<(String, i64)>, Json(b): Json<Value>| async move {
                w.log.lock().unwrap().push((format!("POST /{kind}/{id}"), b));
                Json(json!({"id": id}))
            }),
        )
        .with_state(w)
}

async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

async fn wait_jobs(pool: &PgPool) {
    for _ in 0..200 {
        let n: i64 =
            sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status IN ('queued', 'running')")
                .fetch_one(pool)
                .await
                .unwrap();
        if n == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("jobs did not finish");
}

/// Worker pass once the translation job is done, as the background worker would.
async fn settle(state: &glot_server::AppState, pool: &PgPool) {
    glot_server::connectors::sync::tick(state).await.unwrap();
    wait_jobs(pool).await;
    glot_server::connectors::sync::tick(state).await.unwrap();
}

#[tokio::test]
async fn shopify_and_wordpress_end_to_end() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    // SAFETY: set before any connector call; read-only elsewhere.
    unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
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

    let call = |method: Method,
                uri: String,
                cookie: Option<String>,
                body: Option<Value>,
                headers: Vec<(&'static str, String)>| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, h) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let text = String::from_utf8_lossy(&bytes).to_string();
            (
                status,
                h,
                serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
                text,
            )
        }
    };
    let (s, h, _, _) = call(
        Method::POST,
        "/api/auth/signup".into(),
        None,
        Some(json!({"email": "o@x.io", "password": "correct horse", "name": "O"})),
        vec![],
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
        Some(json!({"name": "S", "slug": "shop", "source_locale": "en", "target_locales": ["fr"]})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/shop".into(),
        c(),
        Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
        vec![],
    )
    .await;

    // ------------------------------------------------------------------ Shopify
    let shop = Shop::default();
    *shop.title.lock().unwrap() = "Shirt".into();
    let base = serve(
        Router::new()
            .route("/admin/api/{v}/graphql.json", post(shopify_gql))
            .with_state(shop.clone()),
    )
    .await;

    let (s, _, _, text) = call(
        Method::POST,
        "/api/projects/shop/connectors".into(),
        c(),
        Some(json!({"kind": "shopify", "secrets": {"shop": base, "token": "wrong"}})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad token refused: {text}");
    assert!(text.contains("HTTP 401"), "{text}");
    let (s, _, body, _) = call(Method::POST, "/api/projects/shop/connectors".into(), c(),
        Some(json!({"kind": "shopify", "secrets": {"shop": base, "token": "shpat_test", "webhook_secret": "whsec_test"},
                    "settings": {"resources": ["PRODUCT", "COLLECTION"]}})), vec![]).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["info"]["name"], "Test Store");
    assert_eq!(body["source_locale_mismatch"], false);
    assert_eq!(body["webhooks_error"], Value::Null);
    let sid = body["connector"]["id"].as_str().unwrap().to_string();
    let enc: Vec<u8> =
        sqlx::query_scalar("SELECT creds_enc FROM connector_accounts WHERE id = $1::uuid")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !String::from_utf8_lossy(&enc).contains("shpat_test"),
        "encrypted at rest"
    );
    let (_, _, _, list) = call(
        Method::GET,
        "/api/projects/shop/connectors".into(),
        c(),
        None,
        vec![],
    )
    .await;
    assert!(
        !list.contains("shpat_test") && !list.contains("whsec_test"),
        "never sent back: {list}"
    );
    assert!(list.contains("\"token\""), "field names listed");
    assert_eq!(
        shop.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(q, _)| q.contains("webhookSubscriptionCreate"))
            .count(),
        4,
        "products/collections create/update"
    );

    let (s, ..) = call(
        Method::POST,
        format!("/api/projects/shop/connectors/{sid}/sync"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    settle(&state, &pool).await;
    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM keys ORDER BY key")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        keys.contains(&"shopify:product:1:title".to_string()),
        "{keys:?}"
    );
    assert!(
        keys.contains(&"shopify:product:2:title".to_string()),
        "second page pulled: {keys:?}"
    );
    assert!(
        keys.iter()
            .any(|k| k.starts_with("shopify:product:1:body_html#")),
        "HTML split into segments"
    );
    assert!(
        !keys.iter().any(|k| k.contains("handle")),
        "handles (URL slugs) skipped"
    );
    let desc: String =
        sqlx::query_scalar("SELECT description FROM keys WHERE key = 'shopify:product:1:title'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(desc.contains("Shirt"), "context as description: {desc}");
    let kind: String = sqlx::query_scalar("SELECT kind FROM jobs LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kind, "connector_translate");

    let regs: Vec<Value> = shop
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|(q, _)| q.contains("translationsRegister"))
        .map(|x| x.1.clone())
        .collect();
    assert_eq!(regs.len(), 3, "one call per resource: {regs:?}");
    let all: Vec<&Value> = regs
        .iter()
        .flat_map(|r| r["translations"].as_array().unwrap())
        .collect();
    let find = |key: &str, id: &str| {
        regs.iter().find(|r| r["id"] == id).unwrap()["translations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["key"] == key)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        find("title", "gid://shopify/Product/1")["value"],
        "⟦fr⟧ Shirt"
    );
    assert_eq!(
        find("title", "gid://shopify/Product/1")["translatableContentDigest"],
        digest("Shirt")
    );
    assert_eq!(
        find("body_html", "gid://shopify/Product/1")["value"],
        "<p>⟦fr⟧ Soft <b>cotton</b> shirt.</p>"
    );
    assert!(all.iter().all(|t| t["locale"] == "fr"));
    assert_eq!(
        *shop.locales.lock().unwrap(),
        vec!["fr".to_string()],
        "missing locale enabled (unpublished)"
    );
    let (status, err): (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM connector_accounts WHERE id = $1::uuid")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), err), ("ok", None));

    // Idempotent: nothing changed → nothing pushed.
    let acc: glot_server::connectors::sync::AccountRow = sqlx::query_as(&format!(
        "SELECT {} FROM connector_accounts WHERE id = $1::uuid",
        glot_server::connectors::sync::COLS
    ))
    .bind(&sid)
    .fetch_one(&pool)
    .await
    .unwrap();
    let before = shop.log.lock().unwrap().len();
    let st = glot_server::connectors::sync::push_ready(&state, &acc)
        .await
        .unwrap();
    assert_eq!((st.pushed, st.unchanged, st.waiting), (0, 4, 0));
    assert_eq!(
        shop.log.lock().unwrap()[before..]
            .iter()
            .filter(|(q, _)| q.contains("translationsRegister"))
            .count(),
        0
    );

    // Webhooks: forged refused, Shopify HMAC (app secret) accepted, generic token accepted.
    let hook = format!("/api/connectors/{sid}/webhook");
    let payload = json!({"id": 1, "title": "Shirt v2"});
    let (s, ..) = call(
        Method::POST,
        hook.clone(),
        None,
        Some(payload.clone()),
        vec![("x-shopify-hmac-sha256", "bm9wZQ==".into())],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let sig = {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let mut m = Hmac::<sha2::Sha256>::new_from_slice(b"whsec_test").unwrap();
        m.update(payload.to_string().as_bytes());
        base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
    };
    *shop.title.lock().unwrap() = "Shirt v2".into();
    let (s, ..) = call(
        Method::POST,
        hook.clone(),
        None,
        Some(payload.clone()),
        vec![("x-shopify-hmac-sha256", sig)],
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let st: String =
        sqlx::query_scalar("SELECT status FROM connector_accounts WHERE id = $1::uuid")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(st, "queued");
    let before = shop.log.lock().unwrap().len();
    settle(&state, &pool).await;
    let regs: Vec<Value> = shop.log.lock().unwrap()[before..]
        .iter()
        .filter(|(q, _)| q.contains("translationsRegister"))
        .map(|x| x.1.clone())
        .collect();
    assert_eq!(regs.len(), 1, "only the changed field re-pushed: {regs:?}");
    assert_eq!(regs[0]["translations"][0]["value"], "⟦fr⟧ Shirt v2");
    assert_eq!(
        regs[0]["translations"][0]["translatableContentDigest"],
        digest("Shirt v2")
    );
    let wh: String =
        sqlx::query_scalar("SELECT webhook_secret FROM connector_accounts WHERE id = $1::uuid")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (s, ..) = call(
        Method::POST,
        format!("{hook}?token={wh}"),
        None,
        Some(json!({})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, ..) = call(
        Method::POST,
        format!("{hook}?token=nope"),
        None,
        Some(json!({})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    settle(&state, &pool).await;

    // ------------------------------------------------------------------ WordPress + Polylang Pro
    let wp = Wp::default();
    let gutenberg = "<!-- wp:heading {\"level\":2} -->\n<h2 class=\"wp-block-heading\">Welcome <em>home</em></h2>\n<!-- /wp:heading -->\n\n<!-- wp:paragraph -->\n<p>Read the <a href=\"/guide\">guide</a>.</p>\n<!-- /wp:paragraph -->";
    *wp.content.lock().unwrap() = gutenberg.into();
    let wbase = serve(wordpress_mock("", wp.clone()).await).await;
    let (s, _, body, _) = call(Method::POST, "/api/projects/shop/connectors".into(), c(),
        Some(json!({"kind": "wordpress", "secrets": {"url": wbase, "username": "editor", "app_password": "abcd abcd abcd abcd abcd abcd"}})), vec![]).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["info"]["name"], "My Blog");
    assert!(
        body["info"]["notes"][0]
            .as_str()
            .unwrap()
            .contains("Polylang")
    );
    let wid = body["connector"]["id"].as_str().unwrap().to_string();
    let (s, _, t, _) = call(
        Method::POST,
        format!("/api/projects/shop/connectors/{wid}/test"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t["locales"], json!(["en", "fr"]));
    call(
        Method::POST,
        format!("/api/projects/shop/connectors/{wid}/sync"),
        c(),
        None,
        vec![],
    )
    .await;
    settle(&state, &pool).await;
    let (err,): (Option<String>,) =
        sqlx::query_as("SELECT last_error FROM connector_accounts WHERE id = $1::uuid")
            .bind(&wid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(err, None);
    let log = wp.log.lock().unwrap().clone();
    let created = log
        .iter()
        .find(|(p, _)| p.starts_with("POST /posts?"))
        .expect("post translation created");
    assert_eq!(
        created.0, "POST /posts?lang=fr&translations[en]=7",
        "linked to the source"
    );
    assert_eq!(created.1["title"], "⟦fr⟧ Hello world");
    assert_eq!(created.1["status"], "draft");
    assert_eq!(
        created.1["content"],
        "<!-- wp:heading {\"level\":2} -->\n<h2 class=\"wp-block-heading\">⟦fr⟧ Welcome <em>home</em></h2>\n<!-- /wp:heading -->\n\n<!-- wp:paragraph -->\n<p>⟦fr⟧ Read the <a href=\"/guide\">guide</a>.</p>\n<!-- /wp:paragraph -->",
        "block comments and markup kept"
    );
    assert!(log.iter().any(
        |(p, b)| p == "POST /categories?lang=fr&translations[en]=3" && b["name"] == "⟦fr⟧ News"
    ));
    // Source change → updated in place (no duplicate).
    *wp.content.lock().unwrap() = gutenberg.replace("guide</a>.", "guide</a> today.");
    let n = wp.log.lock().unwrap().len();
    call(
        Method::POST,
        format!("/api/projects/shop/connectors/{wid}/sync"),
        c(),
        None,
        vec![],
    )
    .await;
    settle(&state, &pool).await;
    let log = wp.log.lock().unwrap()[n..].to_vec();
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].0, "POST /posts/100");
    assert!(
        log[0].1["content"]
            .as_str()
            .unwrap()
            .contains("guide</a> today.")
    );
    assert!(
        log[0].1.get("title").is_none(),
        "unchanged field not re-sent"
    );

    let (s, ..) = call(
        Method::DELETE,
        format!("/api/projects/shop/connectors/{wid}"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    drop(pool);
    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
