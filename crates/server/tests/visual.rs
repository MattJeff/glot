//! Visual QA reports + preview settings, against a real Postgres (skipped when unreachable).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn visual_reports_and_preview() {
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
    let app = glot_server::router(
        glot_server::AppState::new(pool.clone(), "test-secret", false),
        "http://localhost:3100",
    );

    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (status, headers, bytes)
        }
    };
    let json_of = |b: &[u8]| serde_json::from_slice::<Value>(b).unwrap();

    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "v@x.io", "password": "correct horse", "name": "V"})),
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
        Some(json!({"name": "D", "slug": "demo", "source_locale": "en", "target_locales": ["fr", "de"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let base = "/api/projects/demo";
    let (s, _, _) = call(
        Method::GET,
        &format!("{base}/visual-reports/latest"),
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let report = json!({
        "generated_at": "2026-10-18T00:00:00Z",
        "options": {"baseUrl": "http://localhost:3000", "routes": ["/"], "locales": ["fr"], "sourceLocale": "en", "urlPattern": "/{locale}{route}", "viewports": ["1280x800"]},
        "pages": [{"locale": "fr", "route": "/", "viewport": "1280x800", "url": "http://localhost:3000/fr", "screenshot": "screenshots/fr_index.png"}],
        "issues": [{"kind": "overflow", "locale": "fr", "route": "/", "viewport": "1280x800", "selector": "button", "text": "Commencer", "rect": {"x": 1, "y": 2, "width": 3, "height": 4}}]
    });
    // Bad path and bad base64 are rejected.
    for files in [
        json!({"../x.png": "aGk="}),
        json!({"screenshots/a.png": "@@"}),
    ] {
        let (s, _, _) = call(
            Method::POST,
            &format!("{base}/visual-reports"),
            c,
            Some(json!({"report": report, "files": files})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, _, b) = call(
        Method::POST,
        &format!("{base}/visual-reports"),
        c,
        Some(json!({"report": report, "files": {"screenshots/fr_index.png": "iVBORw0KGgo="}, "label": "ci"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let meta = json_of(&b);
    let id = meta["id"].as_str().unwrap().to_string();
    assert_eq!(meta["summary"]["by_kind"]["overflow"], 1);
    assert_eq!(meta["summary"]["pages_by_locale"]["fr"], 1);

    let (s, _, b) = call(Method::GET, &format!("{base}/visual-reports"), c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(json_of(&b)[0]["label"], "ci");
    let (_, _, b) = call(
        Method::GET,
        &format!("{base}/visual-reports/latest"),
        c,
        None,
    )
    .await;
    assert_eq!(json_of(&b)["report"]["issues"][0]["text"], "Commencer");

    let (s, h, b) = call(
        Method::GET,
        &format!("{base}/visual-reports/{id}/files/screenshots/fr_index.png"),
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "image/png");
    assert_eq!(&b[..], b"\x89PNG\r\n\x1a\n");
    // Files are private.
    let (s, _, _) = call(
        Method::GET,
        &format!("{base}/visual-reports/{id}/files/screenshots/fr_index.png"),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Preview settings: defaults, then normalised update.
    let (_, _, b) = call(Method::GET, &format!("{base}/preview"), c, None).await;
    assert_eq!(json_of(&b)["url_pattern"], "/{locale}{route}");
    let (s, _, b) = call(
        Method::PUT,
        &format!("{base}/preview"),
        c,
        Some(json!({"base_url": "http://localhost:3000/", "url_pattern": "/{locale}{route}", "routes": ["/", "pricing", " "]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        json_of(&b),
        json!({"base_url": "http://localhost:3000", "url_pattern": "/{locale}{route}", "routes": ["/", "/pricing"]})
    );
    let (s, _, _) = call(
        Method::PUT,
        &format!("{base}/preview"),
        c,
        Some(json!({"base_url": "javascript:alert(1)", "url_pattern": "/{locale}{route}", "routes": []})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
