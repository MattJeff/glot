//! Universal website tag (`widget/glot.js`): the `website_snippet` setting (validated, published in
//! `/edge/v1/{project}/status`) and `"mode": "website"` discovery: refused while the tag is off (whatever the Edge
//! kill switch says), accepted when on with the usual public checks (personal data rejected), new segments
//! pending review when its `auto_translate` is off. Real Postgres (DATABASE_URL, default :5433), skipped when
//! unreachable.
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
async fn edge_website_snippet() {
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
    let seg = |text: &str| json!({"hash": h(text), "text": text, "url": "/about", "role": "p"});
    let report = |segments: Vec<Value>| {
        call(
            Method::POST,
            "/edge/v1/ed/missing",
            None,
            Some(
                json!({"key": "pk_test", "locale": "es", "mode": "website", "segments": segments}),
            ),
        )
    };

    let (_, _, k) = call(Method::GET, "/api/projects/ed/edge/snippet", c, None).await;
    assert_eq!(k["public_key"], "pk_test");

    // Default: tag off, nothing advertised, nothing accepted.
    let (_, _, st) = call(Method::GET, "/edge/v1/ed/status", None, None).await;
    assert_eq!(st["website"]["on"], false, "{st}");
    assert_eq!(st["hreflang"], json!({}));
    let (_, _, out) = report(vec![seg("Our story")]).await;
    assert_eq!(out["skipped"], "disabled", "{out}");

    let (_, _, mut settings) = call(Method::GET, "/api/projects/ed/edge/settings", c, None).await;
    assert_eq!(
        settings["website_snippet"],
        json!({"on": false, "auto_translate": true, "position": "bottom-right", "locales": [], "flags": true, "site": ""})
    );
    settings["website_snippet"] =
        json!({"on": true, "auto_translate": false, "position": "#nav", "locales": ["de"]});
    let (s, _, _) = call(
        Method::PUT,
        "/api/projects/ed/edge/settings",
        c,
        Some(settings.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "de is not a target locale");
    settings["website_snippet"]["locales"] = json!(["es"]);
    settings["enabled"] = json!(false); // the Edge kill switch does not govern the tag
    let (s, _, saved) = call(
        Method::PUT,
        "/api/projects/ed/edge/settings",
        c,
        Some(settings),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (_, _, st) = call(Method::GET, "/edge/v1/ed/status", None, None).await;
    assert_eq!(
        st["website"],
        json!({"on": true, "auto_translate": false, "position": "#nav", "locales": ["es"], "flags": true, "site": ""})
    );

    let (s, _, out) = report(vec![seg("Our story"), seg("Write to hello@shop.test")]).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{out}");
    assert_eq!(
        (out["accepted"].as_i64(), out["rejected"].as_i64()),
        (Some(1), Some(1)),
        "{out}"
    );
    let rows: Vec<(String, bool)> =
        sqlx::query_as("SELECT source_text, needs_review FROM edge_segments")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![("Our story".into(), true)],
        "review-gated, nothing personal"
    );
}
