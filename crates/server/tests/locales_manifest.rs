//! Locales manifest for language switchers: coverage, min threshold, ETag, CORS, private projects.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn locales_manifest() {
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

    let app = glot_server::router(state, "http://localhost:3100");

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
        Some(json!({"name": "D", "slug": "od", "source_locale": "en", "target_locales": ["fr", "ar"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/od",
        c,
        Some(json!({"public_cdn": true, "llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    for (k, v) in [
        ("home.title", "Welcome"),
        ("home.cta", "Save {count} files"),
    ] {
        call(
            Method::POST,
            "/api/projects/od/keys",
            c,
            Some(json!({"key": k, "source": v})),
        )
        .await;
    }
    // fr fully translated, ar half.
    sqlx::query(
        "INSERT INTO translations (key_id, locale, value, status, origin, source_hash)
         SELECT id, 'fr', source, 'machine', 'machine', '' FROM keys
         UNION ALL SELECT id, 'ar', source, 'machine', 'machine', '' FROM keys WHERE key = 'home.title'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (s, h, m) = call(Method::GET, "/cdn/od/locales.json", None, None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(
        h[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("max-age=60")
    );
    assert_eq!(m["source"], "en");
    assert!(m["updated_at"].is_string());
    assert_eq!(
        m["locales"],
        json!([
            {"code": "en", "name": "English", "dir": "ltr", "coverage": 1.0},
            {"code": "fr", "name": "Français", "dir": "ltr", "coverage": 1.0},
            {"code": "ar", "name": "العربية", "dir": "rtl", "coverage": 0.5},
        ])
    );
    let etag = h[header::ETAG].to_str().unwrap().to_string();
    let req = Request::get("/cdn/od/locales.json")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_MODIFIED
    );

    // Threshold: ar (0.5) drops out, the source always stays.
    let (_, _, m) = call(Method::GET, "/cdn/od/locales.json?min=0.95", None, None).await;
    let codes: Vec<&str> = m["locales"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["en", "fr"]);

    // A new target shows up without anything else (no redeploy), at coverage 0.
    call(
        Method::PATCH,
        "/api/projects/od",
        c,
        Some(json!({"target_locales": ["fr", "ar", "de"]})),
    )
    .await;
    let (_, h2, m) = call(Method::GET, "/cdn/od/locales.json", None, None).await;
    assert_eq!(m["locales"][3]["code"], "de");
    assert_eq!(m["locales"][3]["coverage"], 0.0);
    assert_ne!(h2[header::ETAG].to_str().unwrap(), etag);

    // Private CDN: anonymous 404, members still read it.
    call(
        Method::PATCH,
        "/api/projects/od",
        c,
        Some(json!({"public_cdn": false})),
    )
    .await;
    let (s, _, _) = call(Method::GET, "/cdn/od/locales.json", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, h, _) = call(Method::GET, "/cdn/od/locales.json", c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!h.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
