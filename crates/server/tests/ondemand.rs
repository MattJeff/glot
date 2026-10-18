//! On-demand locales: 202 → mock job → 200 catalog, whitelist, daily cap, monthly budget.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn ondemand_locale_end_to_end() {
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
        Some(json!({"name": "D", "slug": "od", "source_locale": "en", "target_locales": ["fr"]})),
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

    // Off by default: 403 with the flag, 404 without.
    let (s, _, _) = call(Method::GET, "/cdn/od/de.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = call(Method::GET, "/cdn/od/de.json", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _, set) = call(
        Method::PUT,
        "/api/projects/od/ondemand",
        c,
        Some(json!({"locales": ["de", "it"], "monthly_budget_usd": null, "per_day": 2})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{set}");
    let (_, _, got) = call(Method::GET, "/api/projects/od/ondemand", c, None).await;
    assert_eq!(got["locales"], json!(["de", "it"]));

    // Not whitelisted.
    let (s, _, e) = call(Method::GET, "/cdn/od/ja.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(e["error"]["code"], "ondemand_not_allowed");

    // Whitelisted: 202, then the same job (no second one) until the catalog exists.
    let (s, h, _) = call(Method::GET, "/cdn/od/de.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert!(h.contains_key(header::RETRY_AFTER));
    let mut body = Value::Null;
    let mut etag = String::new();
    for _ in 0..300 {
        let (s, h, b) = call(Method::GET, "/cdn/od/de.json?ondemand=1", None, None).await;
        if s == StatusCode::OK {
            body = b;
            etag = h[header::ETAG].to_str().unwrap().to_string();
            break;
        }
        assert_eq!(s, StatusCode::ACCEPTED);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        body["home"]["title"].as_str().unwrap().contains("Welcome"),
        "{body}"
    );
    assert!(body["home"]["cta"].as_str().unwrap().contains("{count}"));
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE params ? 'ondemand_locale'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(jobs, 1);
    // Ready locale: served without the flag too, with ETag revalidation.
    let req = Request::get("/cdn/od/de.json")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_MODIFIED
    );

    // Monthly budget: 0 means nothing new may start.
    sqlx::query("UPDATE projects SET ondemand_budget_usd = 0")
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, e) = call(Method::GET, "/cdn/od/it.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED, "{e}");
    assert_eq!(e["error"]["code"], "ondemand_budget_exhausted");

    // Daily cap (2): one used by `de`, `it` takes the second, `*` lets anything through the whitelist.
    sqlx::query("UPDATE projects SET ondemand_budget_usd = NULL, ondemand_locales = '{*}'")
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, _) = call(Method::GET, "/cdn/od/it.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, h, _) = call(Method::GET, "/cdn/od/es.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(h.contains_key(header::RETRY_AFTER));
    // Junk locales never reach the queue.
    let (s, _, _) = call(Method::GET, "/cdn/od/..%2Fx.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
