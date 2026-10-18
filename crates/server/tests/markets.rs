//! `GET /api/projects/{slug}/markets` ranks languages from existing insights (real Postgres, skipped when unreachable).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn markets_ranks_observed_demand() {
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
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            (status, headers, json)
        }
    };

    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "m@x.io", "password": "correct horse", "name": "M"})),
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
        Some(json!({"name": "D", "slug": "demo", "source_locale": "en", "target_locales": ["fr"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::POST,
        "/api/projects/demo/keys",
        c,
        Some(json!({"key": "a.b", "source": "Hello wide world"})),
    )
    .await;

    // No traffic yet: public data alone, nothing already offered is suggested.
    let (s, _, m) = call(Method::GET, "/api/projects/demo/markets", c, None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["total_visits"], 0);
    assert_eq!(m["words"], 3);
    let list = m["markets"].as_array().unwrap();
    assert!(
        list.iter()
            .all(|o| o["locale"] != "fr" && o["locale"] != "en")
    );
    assert_eq!(list[0]["best"], true);

    let (_, _, o) = call(Method::GET, "/api/projects/demo/insights", c, None).await;
    let hit = format!("/api/insights/{}", o["public_key"].as_str().unwrap());
    for (langs, n) in [("pt-BR", 30), ("en-US", 470)] {
        for _ in 0..n {
            call(
                Method::POST,
                &hit,
                None,
                Some(json!({"langs": [langs], "served": "en"})),
            )
            .await;
        }
    }
    let (s, _, m) = call(Method::GET, "/api/projects/demo/markets?days=7", c, None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["total_visits"], 500);
    let best = &m["markets"][0];
    assert_eq!(best["locale"], "pt-BR", "{m}");
    assert_eq!(best["visits"], 30);
    let summary = best["summary"].as_str().unwrap();
    assert!(
        summary.starts_with("Add Portuguese (Brazil) (pt-BR): ~6 % of your visitors"),
        "{summary}"
    );
    assert!(summary.ends_with("→ best opportunity"), "{summary}");

    // Other users' projects stay private.
    let (s, _, _) = call(Method::GET, "/api/projects/demo/markets", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    drop(pool);
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .ok();
}
