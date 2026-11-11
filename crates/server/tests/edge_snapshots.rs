//! SPA snapshots imported from the CI (`POST /edge/v1/{project}/snapshots`, `glot prerender --upload`): auth and
//! validation, then the proxy serves the imported snapshot of an empty SPA shell, translated, with no renderer
//! service configured (no browser on the glot server). Real Postgres (DATABASE_URL), skipped when unreachable.
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    routing::get,
};
use glot_core::edge::tm::{template, text_hash};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;
use uuid::Uuid;

const SHELL: &str = r#"<!DOCTYPE html><html><head><title>App</title></head><body><div id="root"></div><script src="/app.js"></script></body></html>"#;
const SNAPSHOT: &str = r#"<!DOCTYPE html><html><head><title>App</title></head><body><div id="root"><h1>Simple pricing</h1><p>Start free, upgrade when you grow.</p></div><script src="/app.js"></script></body></html>"#;

#[tokio::test]
async fn edge_snapshots_import_and_serve() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let db = format!("glot_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let cache = tempfile::tempdir().unwrap();
    // SAFETY: set before the proxy's first use, no other thread reads the environment yet.
    unsafe {
        std::env::set_var("GLOT_EDGE_ALLOW_PRIVATE", "1");
        std::env::set_var("GLOT_EDGE_CACHE_DIR", cache.path());
        std::env::remove_var("GLOT_RENDERER_URL");
    }
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", l.local_addr().unwrap());
    let site = Router::new().fallback(get(|| async {
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], SHELL)
    }));
    tokio::spawn(async move { axum::serve(l, site).await.unwrap() });

    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state, "http://localhost:3100");
    let call =
        |method: Method, host: &str, uri: &str, auth: Option<String>, body: Option<Value>| {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::HOST, host)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(a) = auth {
                req = req.header(header::AUTHORIZATION, a);
            }
            let req = req
                .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                .unwrap();
            let app = app.clone();
            async move {
                let res = app.oneshot(req).await.unwrap();
                let s = res.status();
                let edge = res
                    .headers()
                    .get("x-glot-edge")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let b = res.into_body().collect().await.unwrap().to_bytes();
                (s, edge, String::from_utf8_lossy(&b).into_owned())
            }
        };

    // A user, an API token, a project (owner), and a stranger.
    let signup = |email: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::post("/api/auth/signup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"email": email, "password": "correct horse", "name": "O"}).to_string(),
                ))
                .unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::CREATED);
            let cookie = res.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string();
            let req = Request::post("/api/tokens")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &cookie)
                .body(Body::from(json!({"name": "ci"}).to_string()))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let b = res.into_body().collect().await.unwrap().to_bytes();
            let v: Value = serde_json::from_slice(&b).unwrap();
            (cookie, format!("Bearer {}", v["token"].as_str().unwrap()))
        }
    };
    let (cookie, token) = signup("o@x.io").await;
    let (_, stranger) = signup("p@x.io").await;
    let req = Request::post("/api/projects")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &cookie)
        .body(Body::from(
            json!({"name": "S", "slug": "spa", "source_locale": "en", "target_locales": ["es"]})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::CREATED
    );
    let (project,): (Uuid,) = sqlx::query_as("SELECT id FROM projects WHERE slug = 'spa'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ep = "/edge/v1/spa/snapshots";

    // --- Auth: a token is required, and it must be a project admin's.
    let body = json!({"snapshots": [{"path": "/pricing?utm=x", "html": SNAPSHOT, "rendered_at": 1731300000000i64}]});
    let (s, _, _) = call(Method::POST, "api", ep, None, Some(body.clone())).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(Method::POST, "api", ep, Some(stranger), Some(body.clone())).await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN,
        "{s}"
    );

    // --- Validation: bad paths and empty pages are rejected one by one, the rest imported.
    let bad = json!({"snapshots": [
        {"path": "pricing", "html": SNAPSHOT},
        {"path": "//evil.test/x", "html": SNAPSHOT},
        {"path": "/empty", "html": "  "},
    ]});
    let (s, _, b) = call(Method::POST, "api", ep, Some(token.clone()), Some(bad)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v: Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["imported"], 0);
    assert_eq!(v["rejected"].as_array().unwrap().len(), 3);
    let too_many = json!({"snapshots": (0..101).map(|i| json!({"path": format!("/p{i}"), "html": "x"})).collect::<Vec<_>>()});
    let (s, _, _) = call(Method::POST, "api", ep, Some(token.clone()), Some(too_many)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, _, b) = call(Method::POST, "api", ep, Some(token.clone()), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(serde_json::from_str::<Value>(&b).unwrap()["imported"], 1);
    let (path, at): (String, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT path, rendered_at FROM edge_snapshots WHERE project_id = $1")
            .bind(project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(path, "/pricing", "query stripped");
    assert_eq!(at.timestamp_millis(), 1731300000000);

    // --- The proxy: an empty SPA shell at the origin, `rendering: auto`, no renderer service → the imported
    // snapshot, translated in the first response.
    sqlx::query("INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at)
                 VALUES ($1, $2, 'es.spa.test', 'subdomain', 'es', $3, 't', now())")
        .bind(Uuid::new_v4()).bind(project).bind(&origin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO edge_settings (project_id, critical_patterns) VALUES ($1, '{}')")
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    let tpl = template("/pricing", &["es".to_string(), "en".to_string()]);
    for (role, text, es) in [
        ("h1", "Simple pricing", "Precios sencillos"),
        (
            "p",
            "Start free, upgrade when you grow.",
            "Empieza gratis, mejora cuando crezcas.",
        ),
    ] {
        let (h, ctx) = (text_hash("en", text), format!("{role}|{tpl}"));
        sqlx::query("INSERT INTO edge_segments (project_id, hash, context, source_text) VALUES ($1, $2, $3, $4)")
            .bind(project).bind(&h).bind(&ctx).bind(text).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO edge_translations (project_id, hash, context, locale, value, status) VALUES ($1, $2, $3, 'es', $4, 'machine')")
            .bind(project).bind(&h).bind(&ctx).bind(es).execute(&pool).await.unwrap();
    }
    let (s, edge, b) = call(
        Method::GET,
        "es.spa.test",
        "/edge/proxy/pricing",
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.contains("<h1>Precios sencillos</h1>"), "{b}");
    assert!(
        b.contains("<p>Empieza gratis, mejora cuando crezcas.</p>"),
        "{b}"
    );
    assert_eq!(edge, "translated");
    // A page with no imported snapshot: the origin shell (fail-open, never an error).
    let (s, _, b) = call(Method::GET, "es.spa.test", "/edge/proxy/other", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.contains(r#"<div id="root"></div>"#), "{b}");

    // --- Re-import replaces.
    let v2 = SNAPSHOT.replace("Simple pricing", "Brand new title");
    let (s, _, _) = call(
        Method::POST,
        "api",
        ep,
        Some(token),
        Some(json!({"snapshots": [{"path": "/pricing", "html": v2}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (html,): (String,) =
        sqlx::query_as("SELECT html FROM edge_snapshots WHERE project_id = $1")
            .bind(project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(html.contains("Brand new title"));
}
