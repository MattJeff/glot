//! Review API (back-translation) against a real Postgres with the mock provider. Skipped when Postgres is unreachable.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn review_endpoints() {
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

    let mut cookie = String::new();
    let mut call = async |method: Method, uri: &str, body: Value| -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !cookie.is_empty() {
            req = req.header(header::COOKIE, format!("glot_session={cookie}"));
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        if let Some(c) = res.headers().get(header::SET_COOKIE) {
            let c = c.to_str().unwrap().split(';').next().unwrap();
            cookie = c.trim_start_matches("glot_session=").to_string();
        }
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    };

    call(
        Method::POST,
        "/api/auth/signup",
        json!({"email": "r@example.com", "password": "correct horse", "name": "R"}),
    )
    .await;
    let (s, _) = call(
        Method::POST,
        "/api/projects",
        json!({"name": "R", "slug": "rev", "source_locale": "en", "target_locales": ["fr"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/rev",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
    )
    .await;
    for (k, src, fr) in [
        ("a", "Open the Workspace", "⟦fr⟧ Open the Workspace"),
        ("b", "Delete it", "⟦fr⟧ Supprimer"),
        ("c", "Untranslated", ""),
    ] {
        call(
            Method::POST,
            "/api/projects/rev/keys",
            json!({"key": k, "source": src}),
        )
        .await;
        if !fr.is_empty() {
            let (s, v) = call(
                Method::PUT,
                &format!("/api/projects/rev/keys/{k}/translations/fr"),
                json!({"value": fr}),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
    }

    let (s, v) = call(
        Method::POST,
        "/api/projects/rev/review",
        json!({"locale": "fr"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2, "untranslated key skipped: {v}");
    let a = rows.iter().find(|r| r["key"] == "a").unwrap();
    assert_eq!(
        (
            a["back_translation"].as_str(),
            a["verdict"].as_str(),
            a["cached"].as_bool()
        ),
        (Some("Open the Workspace"), Some("same"), Some(false))
    );
    let b = rows.iter().find(|r| r["key"] == "b").unwrap();
    assert_eq!(
        (b["verdict"].as_str(), b["suggestion"].as_str()),
        (Some("different"), Some("⟦fr⟧ Delete it"))
    );

    // Cached on the second read.
    let (s, v) = call(
        Method::GET,
        "/api/projects/rev/keys/b/review?locale=fr",
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["cached"], true);
    // Applying the suggestion invalidates the cache (target hash changed).
    call(
        Method::PUT,
        "/api/projects/rev/keys/b/translations/fr",
        json!({"value": "⟦fr⟧ Delete it"}),
    )
    .await;
    let (_, v) = call(
        Method::GET,
        "/api/projects/rev/keys/b/review?locale=fr",
        Value::Null,
    )
    .await;
    assert_eq!(
        (v["cached"].as_bool(), v["verdict"].as_str()),
        (Some(false), Some("same"))
    );

    let (s, _) = call(
        Method::GET,
        "/api/projects/rev/keys/c/review?locale=fr",
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        Method::POST,
        "/api/projects/rev/review",
        json!({"locale": "de"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
