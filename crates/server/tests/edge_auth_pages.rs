//! glot Edge, pages behind sign-in: the `auth_pages` / `auth_discover` settings (validated, published in
//! `/edge/v1/{project}/status`), and app-mode discovery (`"mode": "app"`): refused unless the project opted in,
//! hash-only items kept only when the text is already known from a public page (a user's name is dropped
//! unseen), UI text capped and checked for personal data, every new segment pending review, URL sample = the
//! page template (no ids). Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use glot_core::edge::tm::text_hash;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn edge_auth_pages() {
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
    let app = glot_server::router(state, "http://localhost:3100");

    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        // Browsers post text/plain (no preflight); the API reads JSON whatever the type.
        req = req.header(header::CONTENT_TYPE, "application/json");
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers): (StatusCode, HeaderMap) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            (status, headers, json)
        }
    };

    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
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
    let c = Some(cookie.as_str());
    let (s, _, _) = call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "E", "slug": "ed", "source_locale": "en", "target_locales": ["es"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/ed",
        c,
        Some(json!({"public_cdn": true, "llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'ed'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_test')")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

    let h = |t: &str| text_hash("en", t);
    let seg = |text: &str, url: &str, role: &str| json!({"hash": h(text), "text": text, "url": url, "role": role});
    let report = |segments: Vec<Value>, extra: Value| {
        let mut b = json!({"key": "pk_test", "locale": "es", "segments": segments});
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        call(Method::POST, "/edge/v1/ed/missing", None, Some(b))
    };

    let hash_only = |text: &str, url: &str, role: &str| json!({"hash": h(text), "text": "", "url": url, "role": role});
    let app = json!({"auth": true, "mode": "app"});

    // Defaults: server-side translation of signed-in pages, no app-mode discovery.
    let (_, _, st) = call(Method::GET, "/edge/v1/ed/status", None, None).await;
    assert_eq!(st["auth"], json!({"pages": "server", "discover": false}));

    // "Settings" is known from a public page (anonymous report).
    let (s, _, out) = report(vec![seg("Settings", "/", "a")], json!({})).await;
    assert_eq!(
        (s, out["accepted"].as_i64()),
        (StatusCode::ACCEPTED, Some(1)),
        "{out}"
    );

    // App mode before opting in: nothing.
    let (_, _, out) = report(
        vec![seg("Billing history", "/dashboard", "h2")],
        app.clone(),
    )
    .await;
    assert_eq!(out["skipped"], "auth", "{out}");

    let (_, _, mut settings) = call(Method::GET, "/api/projects/ed/edge/settings", c, None).await;
    assert_eq!(
        (
            settings["auth_pages"].as_str(),
            settings["auth_discover"].as_bool()
        ),
        (Some("server"), Some(false))
    );
    settings["auth_pages"] = json!("maybe");
    let (s, _, _) = call(
        Method::PUT,
        "/api/projects/ed/edge/settings",
        c,
        Some(settings.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    settings["auth_pages"] = json!("patcher");
    settings["auth_discover"] = json!(true);
    let (s, _, saved) = call(
        Method::PUT,
        "/api/projects/ed/edge/settings",
        c,
        Some(settings),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (_, _, st) = call(Method::GET, "/edge/v1/ed/status", None, None).await;
    assert_eq!(st["auth"], json!({"pages": "patcher", "discover": true}));

    // Still nothing for an ordinary signed-in report (not app mode).
    let (_, _, out) = report(
        vec![seg("Billing history", "/dashboard", "h2")],
        json!({"auth": true}),
    )
    .await;
    assert_eq!(out["skipped"], "auth");

    let long = "Word ".repeat(30);
    let (s, _, out) = report(
        vec![
            hash_only("Settings", "/dashboard/12345?tab=me", "button"), // known text, new context → kept
            hash_only("Jane Doe", "/dashboard", "span"),                // never seen publicly → dropped
            seg("Billing history", "/dashboard", "h2"),                 // [data-glot-ui] text
            seg("Contact jane@example.com", "/dashboard", "p"),         // personal data
            seg(long.trim(), "/dashboard", "p"),                        // not UI text
            json!({"hash": h("Other"), "text": "Poisoned", "url": "/dashboard", "role": "p"}),
        ],
        json!({"auth": true, "mode": "app", "pages": [{"url": "/dashboard", "seen": [{"hash": h("Settings"), "role": "a"}]}]}),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{out}");
    assert_eq!(
        (out["accepted"].as_i64(), out["rejected"].as_i64()),
        (Some(2), Some(4)),
        "{out}"
    );
    let rows: Vec<(String, String, bool, String)> = sqlx::query_as(
        "SELECT source_text, context, needs_review, url_sample FROM edge_segments ORDER BY first_seen, context",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("Settings".into(), "a|/".into(), false, "/".into()),
            (
                "Settings".into(),
                "button|/dashboard/:id".into(),
                true,
                "/dashboard/:id".into()
            ),
            (
                "Billing history".into(),
                "h2|/dashboard".into(),
                true,
                "/dashboard".into()
            ),
        ],
        "every app-mode segment pending review, template sample, nothing personal"
    );
    let pages: i64 = sqlx::query_scalar("SELECT count(*) FROM edge_page_segments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pages, 0, "no page inventory behind sign-in");
}
