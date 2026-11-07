//! glot Edge dashboard routes: domains (add, validation, duplicate, DNS verification over a stub DoH resolver,
//! kill switch, delete), the Caddy fallback map + reload on every change, and `edge/metrics` in the dashboard's
//! shape from the persisted daily counters. Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    Json, Router,
    body::Body,
    extract::Query,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

type Dns = Arc<Mutex<HashMap<(String, String), String>>>;

/// DoH JSON resolver (`?name=&type=`) answering from `dns`, and a Caddy admin `/load` that records bodies.
async fn stub(dns: Dns, loads: Arc<Mutex<Vec<String>>>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/dns-query",
            get(move |Query(q): Query<HashMap<String, String>>| {
                let dns = dns.clone();
                async move {
                    let (name, kind) = (q["name"].clone(), q["type"].clone());
                    let code = if kind == "TXT" { 16 } else { 5 };
                    match dns.lock().unwrap().get(&(name.clone(), kind)) {
                        Some(v) => Json(json!({"Status": 0, "Answer": [{"name": name, "type": code, "data": v}]})),
                        None => Json(json!({"Status": 3})),
                    }
                }
            }),
        )
        .route(
            "/load",
            post(move |h: HeaderMap, body: String| {
                let loads = loads.clone();
                async move {
                    assert_eq!(h[header::CONTENT_TYPE], "text/caddyfile");
                    loads.lock().unwrap().push(body);
                    StatusCode::OK
                }
            }),
        );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn edge_domains_and_metrics() {
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
    let dns: Dns = Default::default();
    let loads: Arc<Mutex<Vec<String>>> = Default::default();
    let base = stub(dns.clone(), loads.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("edge.caddy");
    let caddyfile = dir.path().join("Caddyfile");
    std::fs::write(&caddyfile, "# test caddyfile\n").unwrap();
    // SAFETY: set before the first request, no other thread reads the environment yet.
    unsafe {
        std::env::set_var("GLOT_DOH_URL", format!("{base}/dns-query"));
        std::env::set_var("GLOT_EDGE_CNAME", "edge.glot.test");
        std::env::set_var("GLOT_CADDY_MAP", &map);
        std::env::set_var("GLOT_CADDY_ADMIN", &base);
        std::env::set_var("GLOT_CADDYFILE", &caddyfile);
    }
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state, "http://localhost:3100");

    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder()
            .method(method)
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
            let s = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                s,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let signup = |email: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::post("/api/auth/signup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"email": email, "password": "correct horse", "name": "O"}).to_string(),
                ))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::CREATED);
            res.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()["glot_session=".len()..]
                .to_string()
        }
    };
    let cookie = signup("o@x.io").await;
    let c = Some(cookie.as_str());
    let (s, _) = call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "D", "slug": "dm", "source_locale": "en", "target_locales": ["es", "fr"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let d = "/api/projects/dm/edge/domains";

    // --- Validation.
    for (body, why) in [
        (
            json!({"host": "not a host", "mode": "subdomain", "locale": "es", "origin_url": "https://origin.dm.test"}),
            "host",
        ),
        (
            json!({"host": "es.dm.test", "mode": "proxy", "locale": "es", "origin_url": "https://origin.dm.test"}),
            "mode",
        ),
        (
            json!({"host": "es.dm.test", "mode": "subdomain", "locale": "it", "origin_url": "https://origin.dm.test"}),
            "locale",
        ),
        (
            json!({"host": "es.dm.test", "mode": "subdomain", "locale": "es", "origin_url": "ftp://origin.dm.test"}),
            "scheme",
        ),
        (
            json!({"host": "es.dm.test", "mode": "subdomain", "locale": "es", "origin_url": "https://10.0.0.1"}),
            "private",
        ),
        (
            json!({"host": "es.dm.test", "mode": "subdomain", "locale": "es", "origin_url": "https://es.dm.test"}),
            "loop",
        ),
    ] {
        let (s, b) = call(Method::POST, d, c, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{why}: {b}");
    }
    // A stranger cannot touch the project.
    let other = signup("p@x.io").await;
    let (s, _) = call(Method::GET, d, Some(other.as_str()), None).await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN,
        "{s}"
    );

    // --- Add (EdgeDomain shape), duplicate.
    let (s, dom) = call(
        Method::POST,
        d,
        c,
        Some(json!({"host": "ES.dm.test.", "mode": "subdomain", "locale": "es", "origin_url": "https://origin.dm.test/pricing"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{dom}");
    assert_eq!(dom["host"], "es.dm.test");
    assert_eq!(dom["origin_url"], "https://origin.dm.test");
    assert_eq!(dom["enabled"], true);
    assert_eq!(dom["verified_at"], Value::Null);
    assert_eq!(dom["cname_target"], "edge.glot.test");
    let token = dom["verify_token"].as_str().unwrap().to_string();
    assert!(token.starts_with("glot-verify-") && token.len() > 30);
    for k in ["id", "mode", "locale", "checks", "checked_at", "created_at"] {
        assert!(dom.get(k).is_some(), "{k}: {dom}");
    }
    let id = dom["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        Method::POST,
        d,
        c,
        Some(json!({"host": "es.dm.test", "mode": "subdomain", "locale": "es", "origin_url": "https://origin.dm.test"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, sub) = call(
        Method::POST,
        d,
        c,
        Some(json!({"host": "www.dm.test", "mode": "subdirectory", "locale": "en", "origin_url": "https://origin.dm.test"})),
    )
    .await;
    assert_eq!(sub.get("cname_target"), None, "subdirectory: no CNAME");
    let (s, list) = call(Method::GET, d, c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 2);

    // --- Verify: nothing published yet → checks say why, still unverified, no map entry.
    let v = format!("{d}/{id}/verify");
    let (s, r) = call(Method::POST, &v, c, None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["verified_at"], Value::Null);
    assert_eq!(r["checks"]["txt"]["ok"], false);
    assert_eq!(r["checks"]["txt"]["expected"], token.as_str());
    assert_eq!(r["checks"]["cname"]["ok"], false);
    assert!(r["checked_at"].is_string());
    // TXT only: the CNAME is still required in subdomain mode.
    dns.lock().unwrap().insert(
        ("_glot.es.dm.test".into(), "TXT".into()),
        format!("\"{}\" \"{}\"", &token[..10], &token[10..]),
    );
    let (_, r) = call(Method::POST, &v, c, None).await;
    assert_eq!(
        (
            r["checks"]["txt"]["ok"].clone(),
            r["checks"]["cname"]["ok"].clone()
        ),
        (json!(true), json!(false))
    );
    assert_eq!(r["verified_at"], Value::Null);
    dns.lock().unwrap().insert(
        ("es.dm.test".into(), "CNAME".into()),
        "Edge.Glot.Test.".into(),
    );
    let (_, r) = call(Method::POST, &v, c, None).await;
    assert_eq!(r["checks"]["cname"]["found"], json!(["edge.glot.test"]));
    assert!(r["verified_at"].is_string(), "{r}");

    // Verified subdomain → in Caddy's fallback map, and Caddy reloaded with the Caddyfile.
    let mut ok = false;
    for _ in 0..50 {
        if std::fs::read_to_string(&map)
            .is_ok_and(|m| m.contains("\tes.dm.test origin.dm.test:443 origin.dm.test https\n"))
            && !loads.lock().unwrap().is_empty()
        {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "map: {:?}", std::fs::read_to_string(&map));
    assert_eq!(loads.lock().unwrap()[0], "# test caddyfile\n");
    // Verified hosts are what Caddy's `ask` accepts too.
    let (s, _) = call(Method::GET, "/edge/ask?domain=es.dm.test", None, None).await;
    assert_eq!(s, StatusCode::OK);
    // Subdirectory mode needs only the TXT; the map stays subdomain-only.
    let sid = sub["id"].as_str().unwrap();
    dns.lock().unwrap().insert(
        ("_glot.www.dm.test".into(), "TXT".into()),
        format!("\"{}\"", sub["verify_token"].as_str().unwrap()),
    );
    let (_, r) = call(Method::POST, &format!("{d}/{sid}/verify"), c, None).await;
    assert!(
        r["verified_at"].is_string() && r["checks"].get("cname").is_none(),
        "{r}"
    );

    // --- Metrics: proxied responses (origin unresolvable here: origin-error), dashboard shape, persisted.
    for _ in 0..3 {
        let req = Request::get("/edge/proxy/pricing")
            .header(header::HOST, "es.dm.test")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert!(res.headers().contains_key("x-glot-edge"));
    }
    let (s, m) = call(Method::GET, "/api/projects/dm/edge/metrics?days=3", c, None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["days"], 3);
    assert_eq!(m["requests"], 3, "{m}");
    assert_eq!(m["cache_hit_ratio"], 0.0);
    assert_eq!(m["fail_open"], 0);
    assert_eq!(m["by_day"].as_array().unwrap().len(), 3);
    assert_eq!(m["by_day"][2]["requests"], 3);
    for k in ["glot_p50_ms", "glot_p95_ms", "origin_p50_ms"] {
        assert!(m.get(k).is_some(), "{k}");
    }
    let n: i64 = sqlx::query_scalar("SELECT requests FROM edge_metrics_daily")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3, "persisted");

    // --- Kill switch, delete (→ map entry gone).
    let one = format!("{d}/{id}");
    let (s, r) = call(Method::PATCH, &one, c, Some(json!({"enabled": false}))).await;
    assert_eq!((s, r["enabled"].clone()), (StatusCode::OK, json!(false)));
    let (s, _) = call(Method::DELETE, &one, c, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(Method::DELETE, &one, c, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let mut gone = false;
    for _ in 0..50 {
        if std::fs::read_to_string(&map).is_ok_and(|m| !m.contains("es.dm.test")) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone);
    let (_, list) = call(Method::GET, d, c, None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
