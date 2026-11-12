//! What a merchant without developers relies on, end to end against a stateful simulated Shopify (GraphQL Admin
//! API + OAuth token endpoint, built from the official docs) and a simulated WordPress / WooCommerce site:
//! Dev Dashboard app credentials (client credentials grant, token renewed before it expires) → preview before
//! anything is translated or written → content types left out → approval → push → rollback (restore and
//! delete) → a human edit made in Shopify is kept → Liquid broken by a translation is never written → diagnosis
//! (scopes) → one-click OAuth install (state, HMAC, expiring offline token, refresh) → app uninstall and
//! `shop/redact` webhooks → WordPress application password and WooCommerce key flows.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    Form, Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

#[derive(Default)]
struct ShopState {
    /// (resource gid, key) → source value.
    content: BTreeMap<(String, String), String>,
    /// (resource gid, key, locale) → translation.
    translations: BTreeMap<(String, String, String), String>,
    scopes: Vec<&'static str>,
    /// Token requests received (form bodies).
    token_requests: Vec<HashMap<String, String>>,
    /// The only access token accepted (rotates on each grant).
    token: String,
    removed: usize,
    registered: usize,
}
type Shop = Arc<Mutex<ShopState>>;

fn digest(s: &str) -> String {
    glot_server::auth::sha256_hex(s)
}

/// `POST /admin/oauth/access_token`: client credentials, authorization code (`expiring=1`) and refresh grants.
async fn shop_token(
    State(s): State<Shop>,
    Form(f): Form<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let mut s = s.lock().unwrap();
    let ok = match f.get("grant_type").map(String::as_str) {
        Some("client_credentials") => f["client_id"] == "dev" && f["client_secret"] == "devsecret",
        Some("refresh_token") => {
            f["client_id"] == "cid"
                && f["client_secret"] == "csecret"
                && f["refresh_token"].starts_with("shprt_")
        }
        _ => {
            f["client_id"] == "cid"
                && f["client_secret"] == "csecret"
                && f["code"] == "code1"
                && f["expiring"] == "1"
        }
    };
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_request"})),
        );
    }
    s.token_requests.push(f.clone());
    let n = s.token_requests.len();
    s.token = format!("tok{n}");
    let mut body = json!({"access_token": s.token, "scope": "read_products", "expires_in": 10});
    if f.get("grant_type").map(String::as_str) != Some("client_credentials") {
        body["refresh_token"] = json!(format!("shprt_{n}"));
        body["refresh_token_expires_in"] = json!(7776000);
    }
    (StatusCode::OK, Json(body))
}

async fn shop_gql(
    State(s): State<Shop>,
    headers: HeaderMap,
    Json(b): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut s = s.lock().unwrap();
    if headers["x-shopify-access-token"] != s.token.as_str() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"errors": "[API] Invalid API key or access token"})),
        );
    }
    let q = b["query"].as_str().unwrap_or_default();
    let v = &b["variables"];
    let nodes = |s: &ShopState, prefix: &str, locale: Option<&str>| -> Vec<Value> {
        let mut by: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for ((id, key), value) in &s.content {
            if id.starts_with(prefix) {
                by.entry(id.clone())
                    .or_default()
                    .push(json!({"key": key, "value": value,
                    "digest": digest(value), "locale": "en",
                    "type": if key == "body_html" { "HTML" } else { "SINGLE_LINE_TEXT_FIELD" }}));
            }
        }
        by.into_iter()
            .map(|(id, content)| {
                let tr: Vec<Value> = s
                    .translations
                    .iter()
                    .filter(|((i, _, l), _)| *i == id && Some(l.as_str()) == locale)
                    .map(|((_, k, l), v)| json!({"key": k, "locale": l, "value": v}))
                    .collect();
                json!({"resourceId": id, "translatableContent": content, "translations": tr})
            })
            .collect()
    };
    let data = if q.contains("translatableResourcesByIds") {
        let ids: Vec<&str> = v["ids"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let all = nodes(&s, "gid://", v["locale"].as_str());
        json!({"translatableResourcesByIds": {"nodes": all.into_iter().filter(|n| ids.contains(&n["resourceId"].as_str().unwrap())).collect::<Vec<_>>()}})
    } else if q.contains("translatableResources") {
        let prefix = match v["type"].as_str() {
            Some("PRODUCT") => "gid://shopify/Product/",
            Some("PAGE") => "gid://shopify/OnlineStorePage/",
            // A type the token may not read: the others still sync.
            Some("ONLINE_STORE_THEME") => {
                return (
                    StatusCode::OK,
                    Json(
                        json!({"errors": [{"message": "Access denied for translatableResources field. Required access: `read_themes` access scope."}]}),
                    ),
                );
            }
            _ => "none",
        };
        json!({"translatableResources": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": nodes(&s, prefix, None)}})
    } else if q.contains("translationsRegister") {
        let id = v["id"].as_str().unwrap().to_string();
        let mut errors = vec![];
        for (i, t) in v["translations"].as_array().unwrap().iter().enumerate() {
            let key = t["key"].as_str().unwrap().to_string();
            let src = s
                .content
                .get(&(id.clone(), key.clone()))
                .cloned()
                .unwrap_or_default();
            if t["translatableContentDigest"].as_str() != Some(&digest(&src)) {
                errors.push(json!({"field": ["translations", i.to_string(), "translatableContentDigest"], "message": "digest mismatch"}));
                continue;
            }
            s.translations.insert(
                (id.clone(), key, t["locale"].as_str().unwrap().into()),
                t["value"].as_str().unwrap().into(),
            );
            s.registered += 1;
        }
        json!({"translationsRegister": {"translations": [], "userErrors": errors}})
    } else if q.contains("translationsRemove") {
        let id = v["id"].as_str().unwrap().to_string();
        for k in v["keys"].as_array().unwrap() {
            for l in v["locales"].as_array().unwrap() {
                if s.translations
                    .remove(&(
                        id.clone(),
                        k.as_str().unwrap().into(),
                        l.as_str().unwrap().into(),
                    ))
                    .is_some()
                {
                    s.removed += 1;
                }
            }
        }
        json!({"translationsRemove": {"userErrors": []}})
    } else if q.contains("currentAppInstallation") {
        json!({"currentAppInstallation": {"accessScopes": s.scopes.iter().map(|h| json!({"handle": h})).collect::<Vec<_>>()}})
    } else if q.contains("webhookSubscriptionCreate") {
        json!({"webhookSubscriptionCreate": {"webhookSubscription": {"id": "gid://shopify/WebhookSubscription/1"}, "userErrors": []}})
    } else if q.contains("webhookSubscriptions") {
        json!({"webhookSubscriptions": {"nodes": []}})
    } else if q.contains("shopLocales") {
        let l = json!([{"locale": "en", "primary": true, "published": true}, {"locale": "fr", "primary": false, "published": true}]);
        json!({"shop": {"name": "Test Store"}, "shopLocales": l})
    } else {
        panic!("unexpected query {q}")
    };
    (StatusCode::OK, Json(json!({"data": data})))
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

async fn settle(state: &glot_server::AppState, pool: &PgPool) {
    glot_server::connectors::sync::tick(state).await.unwrap();
    wait_jobs(pool).await;
    glot_server::connectors::sync::tick(state).await.unwrap();
}

fn hmac_hex(secret: &str, msg: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(msg.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn hmac_b64(secret: &str, body: &[u8]) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn query_param(url: &str, name: &str) -> String {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| panic!("no {name} in {url}"))
}

#[tokio::test]
async fn merchant_flow_end_to_end() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    // SAFETY: the only test of this binary; set before any connector call.
    unsafe {
        std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1");
        std::env::set_var("GLOT_SHOPIFY_CLIENT_ID", "cid");
        std::env::set_var("GLOT_SHOPIFY_CLIENT_SECRET", "csecret");
        std::env::remove_var("GLOT_WEBFLOW_CLIENT_ID");
    }
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
    let api = "/api/projects/shop/connectors";

    // ------------------------------------------------------------------ the store
    let shop: Shop = Default::default();
    {
        let mut s = shop.lock().unwrap();
        let p = |n: u32| format!("gid://shopify/Product/{n}");
        s.content.insert((p(1), "title".into()), "Shirt".into());
        s.content.insert(
            (p(1), "body_html".into()),
            "<p>Soft <b>cotton</b> shirt.</p>".into(),
        );
        s.content
            .insert((p(1), "meta_description".into()), "A soft shirt".into());
        s.content
            .insert((p(2), "title".into()), "Hat by {{ shop.name }}".into());
        s.content.insert(
            ("gid://shopify/OnlineStorePage/5".into(), "title".into()),
            "About us".into(),
        );
        s.scopes = vec![
            "read_translations",
            "write_translations",
            "read_locales",
            "write_products",
        ];
    }
    let base = serve(
        Router::new()
            .route("/admin/api/{v}/graphql.json", post(shop_gql))
            .route("/admin/oauth/access_token", post(shop_token))
            .with_state(shop.clone()),
    )
    .await;
    let tr = |id: &str, key: &str| {
        shop.lock()
            .unwrap()
            .translations
            .get(&(id.to_string(), key.to_string(), "fr".to_string()))
            .cloned()
    };
    let p1 = "gid://shopify/Product/1";
    let p2 = "gid://shopify/Product/2";

    // ------------------------------------------------------------------ Dev Dashboard app, preview first
    let (s, _, body, text) = call(
        Method::POST,
        api.into(),
        c(),
        Some(json!({"kind": "shopify", "preview": true,
            "secrets": {"shop": base, "client_id": "dev", "client_secret": "devsecret"}})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{text}");
    assert!(
        !text.contains("devsecret") && !text.contains("tok1"),
        "no secret in the answer: {text}"
    );
    assert_eq!(body["connector"]["hold"], "translate");
    assert!(body["connector"]["token_expires_at"].is_string(), "{text}");
    assert_eq!(
        body["webhooks_error"],
        Value::Null,
        "client secret signs the webhooks: {text}"
    );
    let id = body["connector"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        shop.lock().unwrap().token_requests[0]["grant_type"],
        "client_credentials"
    );

    // First sync = preview: pulled and counted, nothing translated, nothing written.
    settle(&state, &pool).await;
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    let a = &list[0];
    assert_eq!(
        (a["status"].as_str(), a["last_error"].as_str()),
        (Some("ok"), None),
        "{text}"
    );
    assert_eq!(
        a["stats"]["by_type"],
        json!({"page": 1, "product": 4}),
        "{text}"
    );
    assert_eq!(a["stats"]["resources"], json!({"page": 1, "product": 2}));
    assert_eq!(a["stats"]["to_translate"], 5);
    assert!(a["stats"]["words"].as_u64().unwrap() >= 10, "{text}");
    assert!(a["stats"]["cost_usd"].is_number());
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (jobs, shop.lock().unwrap().registered),
        (0, 0),
        "preview spends and writes nothing"
    );
    assert!(
        shop.lock().unwrap().token_requests.len() >= 2,
        "a token about to expire is renewed before use"
    );

    // The merchant leaves pages out, then approves.
    let (s, ..) = call(
        Method::PATCH,
        format!("{api}/{id}"),
        c(),
        Some(json!({"settings": {"exclude": ["page"]}})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    settle(&state, &pool).await;
    let (_, _, list, _) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(
        (
            &list[0]["stats"]["items"],
            &list[0]["stats"]["to_translate"]
        ),
        (&json!(4), &json!(4))
    );
    let (s, _, body, _) = call(
        Method::POST,
        format!("{api}/{id}/hold"),
        c(),
        Some(json!({"hold": null})),
        vec![],
    )
    .await;
    assert_eq!(
        (s, &body["hold"], body["status"].as_str()),
        (StatusCode::OK, &Value::Null, Some("queued"))
    );
    settle(&state, &pool).await;
    assert_eq!(tr(p1, "title").as_deref(), Some("⟦fr⟧ Shirt"));
    assert_eq!(
        tr(p1, "body_html").as_deref(),
        Some("<p>⟦fr⟧ Soft <b>cotton</b> shirt.</p>")
    );
    assert_eq!(
        tr(p1, "meta_description").as_deref(),
        Some("⟦fr⟧ A soft shirt"),
        "SEO field"
    );
    assert_eq!(
        tr(p2, "title").as_deref(),
        Some("⟦fr⟧ Hat by {{ shop.name }}"),
        "Liquid kept"
    );
    assert_eq!(
        tr("gid://shopify/OnlineStorePage/5", "title"),
        None,
        "excluded type not written"
    );
    let (_, _, runs, text) = call(Method::GET, format!("{api}/{id}/runs"), c(), None, vec![]).await;
    assert_eq!(runs.as_array().unwrap().len(), 1, "{text}");
    assert_eq!(runs[0]["stats"]["pushed"], 4);
    let run_a = runs[0]["id"].as_str().unwrap().to_string();

    // ------------------------------------------------------------------ update, then rollback restores
    shop.lock()
        .unwrap()
        .content
        .insert((p1.into(), "title".into()), "Blue shirt".into());
    let (s, ..) = call(Method::POST, format!("{api}/{id}/sync"), c(), None, vec![]).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    settle(&state, &pool).await;
    assert_eq!(tr(p1, "title").as_deref(), Some("⟦fr⟧ Blue shirt"));
    let (_, _, runs, _) = call(Method::GET, format!("{api}/{id}/runs"), c(), None, vec![]).await;
    let run_b = runs[0]["id"].as_str().unwrap().to_string();
    assert_ne!(run_a, run_b);
    let (s, _, _, text) = call(
        Method::POST,
        format!("{api}/{id}/runs/{run_a}/rollback"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "only the latest run: {text}");
    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/runs/{run_b}/rollback"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{text}");
    assert_eq!(
        (&body["restored"], &body["removed"]),
        (&json!(1), &json!(0)),
        "{text}"
    );
    assert_eq!(
        tr(p1, "title").as_deref(),
        Some("⟦fr⟧ Shirt"),
        "previous translation back"
    );
    let (_, _, list, _) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(
        list[0]["hold"], "push",
        "publishing paused after a rollback"
    );

    // Dry run: what a publish would write, then publish.
    let (_, _, d, text) = call(Method::GET, format!("{api}/{id}/diff"), c(), None, vec![]).await;
    assert_eq!(d["total"], 1, "{text}");
    assert_eq!(
        (&d["rows"][0]["value"], &d["rows"][0]["update"]),
        (&json!("⟦fr⟧ Blue shirt"), &json!(true))
    );
    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/publish"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(
        (s, &body["error"]),
        (StatusCode::OK, &Value::Null),
        "{text}"
    );
    assert_eq!(tr(p1, "title").as_deref(), Some("⟦fr⟧ Blue shirt"));
    call(
        Method::POST,
        format!("{api}/{id}/hold"),
        c(),
        Some(json!({"hold": null})),
        vec![],
    )
    .await;

    // ------------------------------------------------------------------ a human edit in Shopify is kept
    {
        let mut s = shop.lock().unwrap();
        s.translations.insert(
            (p1.into(), "title".into(), "fr".into()),
            "Chemise bleue, retouchée à la main".into(),
        );
        s.content
            .insert((p1.into(), "title".into()), "Red shirt".into());
    }
    call(Method::POST, format!("{api}/{id}/sync"), c(), None, vec![]).await;
    settle(&state, &pool).await;
    assert_eq!(
        tr(p1, "title").as_deref(),
        Some("Chemise bleue, retouchée à la main"),
        "never overwritten"
    );
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(list[0]["stats"]["last_push"]["kept"], 1, "{text}");
    assert_eq!(list[0]["last_error"], Value::Null, "{text}");

    // ------------------------------------------------------------------ broken Liquid is never written
    sqlx::query(
        "UPDATE translations SET value = 'Chapeau de {{ boutique.nom }}', status = 'reviewed'
         WHERE locale = 'fr' AND key_id = (SELECT id FROM keys WHERE key = 'shopify:product:2:title')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (_, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/publish"),
        c(),
        None,
        vec![],
    )
    .await;
    assert!(
        body["error"].as_str().unwrap().contains("placeholder"),
        "{text}"
    );
    assert_eq!(
        tr(p2, "title").as_deref(),
        Some("⟦fr⟧ Hat by {{ shop.name }}"),
        "store untouched"
    );
    let issue = &body["connector"]["issues"][0];
    assert_eq!(issue["fix"], "placeholders", "{text}");
    assert!(
        issue["item"].as_str().unwrap().contains("Product/2/title"),
        "{text}"
    );
    let (_, _, d, _) = call(
        Method::GET,
        format!("{api}/{id}/diff?locale=fr"),
        c(),
        None,
        vec![],
    )
    .await;
    assert!(
        d["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["fix"] == "placeholders")
    );

    // ------------------------------------------------------------------ rollbacks never undo a human edit
    let (_, _, runs, _) = call(Method::GET, format!("{api}/{id}/runs"), c(), None, vec![]).await;
    let run_c = runs[0]["id"].as_str().unwrap().to_string();
    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/runs/{run_c}/rollback"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(
        (s, &body["kept"], &body["restored"]),
        (StatusCode::OK, &json!(1), &json!(0)),
        "{text}"
    );
    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/runs/{run_a}/rollback"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{text}");
    assert_eq!(
        (&body["removed"], &body["kept"]),
        (&json!(3), &json!(1)),
        "created translations deleted: {text}"
    );
    assert_eq!(shop.lock().unwrap().removed, 3);
    assert_eq!(tr(p1, "body_html"), None);
    assert_eq!(
        tr(p1, "title").as_deref(),
        Some("Chemise bleue, retouchée à la main")
    );

    // ------------------------------------------------------------------ diagnosis
    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/doctor"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!((s, &body["ok"]), (StatusCode::OK, &json!(true)), "{text}");
    let check = |b: &Value, id: &str| {
        b["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["id"] == id)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(check(&body, "auth")[0]["status"], "ok");
    assert_eq!(
        check(&body, "translations")[0]["status"],
        "ok",
        "translations can be read back"
    );
    assert_eq!(check(&body, "scopes")[0]["status"], "ok");
    let content = check(&body, "scope_content");
    assert!(
        content
            .iter()
            .any(|c| c["detail"].as_str().unwrap().starts_with("read_themes")),
        "{text}"
    );
    assert!(
        !content
            .iter()
            .any(|c| c["detail"].as_str().unwrap().starts_with("read_products")),
        "write includes read"
    );
    shop.lock()
        .unwrap()
        .scopes
        .retain(|s| *s != "write_translations");
    let (_, _, body, text) = call(
        Method::POST,
        format!("{api}/{id}/doctor"),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(body["ok"], false, "{text}");
    let scopes = &check(&body, "scopes")[0];
    assert_eq!(
        (scopes["status"].as_str(), scopes["fix"].as_str()),
        (Some("fail"), Some("permissions"))
    );
    assert!(
        scopes["detail"]
            .as_str()
            .unwrap()
            .contains("write_translations")
    );
    shop.lock().unwrap().scopes.push("write_translations");
    call(Method::DELETE, format!("{api}/{id}"), c(), None, vec![]).await;

    // ------------------------------------------------------------------ one-click OAuth install
    let (_, _, kinds, _) = call(Method::GET, format!("{api}/oauth"), c(), None, vec![]).await;
    assert_eq!(
        kinds,
        json!(["shopify", "wordpress", "woocommerce"]),
        "webflow / hubspot: no client configured"
    );
    let (s, _, _, text) = call(
        Method::POST,
        format!("{api}/oauth/shopify/start"),
        c(),
        Some(json!({"tenant": "evil.com"})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{text}");
    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/oauth/shopify/start"),
        c(),
        Some(json!({"tenant": base})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{text}");
    let authorize = body["url"].as_str().unwrap().to_string();
    assert!(
        authorize.starts_with(&format!("{base}/admin/oauth/authorize?")),
        "{authorize}"
    );
    assert_eq!(query_param(&authorize, "client_id"), "cid");
    assert!(query_param(&authorize, "scope").contains("write_translations"));
    assert_eq!(
        query_param(&authorize, "redirect_uri"),
        "http://localhost:3100/api/connectors/oauth/shopify/callback"
    );
    let st = query_param(&authorize, "state");
    let signed = |code: &str, secret: &str| {
        let msg = format!("code={code}&shop=test.myshopify.com&state={st}&timestamp=1");
        format!(
            "/api/connectors/oauth/shopify/callback?code={code}&shop=test.myshopify.com&state={st}&timestamp=1&hmac={}",
            hmac_hex(secret, &msg)
        )
    };
    let location = |h: &HeaderMap| h[header::LOCATION].to_str().unwrap().to_string();
    // Forged signature, then a browser that is not signed in: no account, an explanation.
    let (s, h, ..) = call(Method::GET, signed("code1", "wrong"), c(), None, vec![]).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert!(
        location(&h).contains("connect_error=invalid+signature"),
        "{}",
        location(&h)
    );
    let (_, h, ..) = call(Method::GET, signed("code1", "csecret"), None, None, vec![]).await;
    assert!(
        location(&h).contains("connect_error=sign+in"),
        "{}",
        location(&h)
    );
    let (s, ..) = call(
        Method::GET,
        "/api/connectors/oauth/shopify/callback?code=code1&state=forged".into(),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, h, ..) = call(Method::GET, signed("code1", "csecret"), c(), None, vec![]).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(
        location(&h),
        "http://localhost:3100/app/shop/integrations?connected=shopify"
    );
    {
        let s = shop.lock().unwrap();
        let grant = s.token_requests.last().unwrap();
        assert_eq!(
            (grant["code"].as_str(), grant["expiring"].as_str()),
            ("code1", "1")
        );
    }
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    let a = &list[0];
    assert_eq!(
        (a["hold"].as_str(), a["name"].as_str()),
        (Some("translate"), Some("Test Store")),
        "{text}"
    );
    assert_eq!(a["settings"]["shop"], base);
    assert!(!text.contains("shprt_") && !text.contains("csecret"));
    let oid = a["id"].as_str().unwrap().to_string();
    // The 1 h token is refreshed with the rotating refresh token.
    settle(&state, &pool).await;
    {
        let s = shop.lock().unwrap();
        let refresh = s.token_requests.last().unwrap();
        assert_eq!(refresh["grant_type"], "refresh_token");
        assert!(refresh["refresh_token"].starts_with("shprt_"));
    }
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(
        (list[0]["status"].as_str(), list[0]["last_error"].as_str()),
        (Some("ok"), None),
        "{text}"
    );

    // Product webhook signed with the app's client secret → incremental sync, "webhooks work" visible.
    let hook_body = br#"{"id":1}"#;
    let (s, ..) = call(
        Method::POST,
        format!("/api/connectors/{oid}/webhook"),
        None,
        Some(json!({"id": 1})),
        vec![("x-shopify-hmac-sha256", hmac_b64("csecret", hook_body))],
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (_, _, list, _) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert!(list[0]["last_webhook_at"].is_string());
    settle(&state, &pool).await;

    // App webhooks: forged → 401; uninstall → reconnect state; compliance topics acknowledged; redact deletes.
    let app_hook = |topic: &'static str, secret: &'static str| {
        call(
            Method::POST,
            "/api/connectors/shopify/webhooks".into(),
            None,
            Some(json!({"id": 1})),
            vec![
                ("x-shopify-topic", topic.to_string()),
                ("x-shopify-shop-domain", base.clone()),
                ("x-shopify-hmac-sha256", hmac_b64(secret, hook_body)),
            ],
        )
    };
    assert_eq!(
        app_hook("app/uninstalled", "wrong").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app_hook("customers/redact", "csecret").await.0,
        StatusCode::OK
    );
    assert_eq!(
        app_hook("app/uninstalled", "csecret").await.0,
        StatusCode::OK
    );
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(
        (list[0]["status"].as_str(), &list[0]["reconnect"]),
        (Some("error"), &json!(true)),
        "{text}"
    );
    // Reconnect: same account, new tokens.
    let (_, _, body, _) = call(
        Method::POST,
        format!("{api}/oauth/shopify/start"),
        c(),
        Some(json!({"tenant": base, "account": oid})),
        vec![],
    )
    .await;
    let st2 = query_param(body["url"].as_str().unwrap(), "state");
    let msg = format!("code=code1&shop=test.myshopify.com&state={st2}&timestamp=2");
    let (s, h, ..) = call(
        Method::GET,
        format!("/api/connectors/oauth/shopify/callback?code=code1&shop=test.myshopify.com&state={st2}&timestamp=2&hmac={}", hmac_hex("csecret", &msg)),
        c(),
        None,
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::SEE_OTHER, "{}", location(&h));
    settle(&state, &pool).await;
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(list.as_array().unwrap().len(), 1, "no duplicate account");
    assert_eq!(
        (
            list[0]["id"].as_str(),
            list[0]["status"].as_str(),
            &list[0]["reconnect"]
        ),
        (Some(oid.as_str()), Some("ok"), &json!(false)),
        "{text}"
    );
    assert_eq!(app_hook("shop/redact", "csecret").await.0, StatusCode::OK);
    let (_, _, list, _) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(list, json!([]), "store data erased");

    // ------------------------------------------------------------------ WordPress and WooCommerce, no token to paste
    let hooks: Arc<Mutex<Vec<Value>>> = Default::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let wp = format!("http://{}", l.local_addr().unwrap());
    let index = json!({"name": "My Blog", "namespaces": ["wp/v2", "wc/v3"],
        "authentication": {"application-passwords": {"endpoints": {"authorization": format!("{wp}/wp-admin/authorize-application.php")}}}});
    let wp_app = Router::new()
        .route(
            "/wp-json/",
            get(move || {
                let i = index.clone();
                async move { Json(i) }
            }),
        )
        .route(
            "/wp-json/wp/v2/users/me",
            get(|h: HeaderMap| async move {
                use base64::Engine;
                let want = format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode("editor:abcdabcdabcdabcdabcdabcd")
                );
                if h[header::AUTHORIZATION] == want.as_str() {
                    (StatusCode::OK, Json(json!({"id": 1})))
                } else {
                    (StatusCode::UNAUTHORIZED, Json(json!({"code": "invalid"})))
                }
            }),
        )
        .route(
            "/wp-json/wc/v3/products",
            get(|| async { ([("x-wp-totalpages", "1")], Json(json!([]))) }),
        )
        .route(
            "/wp-json/wc/v3/webhooks",
            get(|| async { ([("x-wp-totalpages", "1")], Json(json!([]))) }).post(
                |State(hooks): State<Arc<Mutex<Vec<Value>>>>, Json(b): Json<Value>| async move {
                    hooks.lock().unwrap().push(b);
                    (StatusCode::CREATED, Json(json!({"id": 1})))
                },
            ),
        )
        .with_state(hooks.clone());
    tokio::spawn(async move { axum::serve(l, wp_app).await.unwrap() });

    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/oauth/wordpress/start"),
        c(),
        Some(json!({"tenant": wp})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{text}");
    let authorize = body["url"].as_str().unwrap().to_string();
    assert!(
        authorize.starts_with(&format!(
            "{wp}/wp-admin/authorize-application.php?app_name=glot"
        )),
        "{authorize}"
    );
    let success = query_param(&authorize, "success_url");
    assert!(
        success.starts_with("http://localhost:3100/api/connectors/oauth/wordpress/callback?state=")
    );
    let path = success
        .trim_start_matches("http://localhost:3100")
        .to_string();
    // Refused in WordPress.
    let (_, h, ..) = call(
        Method::GET,
        format!("{path}&success=false"),
        c(),
        None,
        vec![],
    )
    .await;
    assert!(location(&h).contains("connect_error="), "{}", location(&h));
    // Approved: WordPress appends site_url, user_login and password.
    let (s, h, ..) = call(Method::GET, format!("{path}&site_url={wp}&user_login=editor&password=abcd%20abcd%20abcd%20abcd%20abcd%20abcd"), c(), None, vec![]).await;
    assert_eq!(
        (s, location(&h).as_str()),
        (
            StatusCode::SEE_OTHER,
            "http://localhost:3100/app/shop/integrations?connected=wordpress"
        )
    );
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    assert_eq!(
        (list[0]["kind"].as_str(), list[0]["name"].as_str()),
        (Some("wordpress"), Some("My Blog")),
        "{text}"
    );
    assert!(
        !text.contains("abcd abcd"),
        "the application password never comes back"
    );

    let (s, _, body, text) = call(
        Method::POST,
        format!("{api}/oauth/woocommerce/start"),
        c(),
        Some(json!({"tenant": wp})),
        vec![],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{text}");
    let authorize = body["url"].as_str().unwrap().to_string();
    assert!(
        authorize.starts_with(&format!(
            "{wp}/wc-auth/v1/authorize?app_name=glot&scope=read_write"
        )),
        "{authorize}"
    );
    assert!(
        query_param(&authorize, "callback_url").ends_with("/api/connectors/oauth/woocommerce/keys")
    );
    let st = query_param(&authorize, "user_id");
    let keys = |user_id: String| {
        call(
            Method::POST,
            "/api/connectors/oauth/woocommerce/keys".into(),
            None,
            Some(
                json!({"key_id": 1, "user_id": user_id, "consumer_key": "ck_x", "consumer_secret": "cs_x", "key_permissions": "read_write"}),
            ),
            vec![],
        )
    };
    assert_eq!(keys("forged".into()).await.0, StatusCode::UNAUTHORIZED);
    let (s, _, _, text) = keys(st).await;
    assert_eq!(s, StatusCode::OK, "{text}");
    let (_, _, list, text) = call(Method::GET, api.into(), c(), None, vec![]).await;
    let woo = list
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "woocommerce")
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        woo["secret_fields"]
            .as_array()
            .unwrap()
            .contains(&json!("webhook_secret")),
        "webhook secret generated: {text}"
    );
    assert!(!text.contains("cs_x"));
    assert!(
        !hooks.lock().unwrap().is_empty(),
        "product webhooks subscribed without asking for a secret"
    );

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}
