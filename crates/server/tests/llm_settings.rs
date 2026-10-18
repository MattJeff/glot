//! Per-project LLM headers (anthropic_workspace_id, extra_headers): settings validation + the Review API
//! sending them to a fake Anthropic server. Skipped when Postgres is unreachable.
use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::post,
};
use glot_llm::provider::{CompletionRequest, Provider};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

type Seen = Arc<Mutex<Vec<HeaderMap>>>;

/// Minimal `/v1/messages` answering with the mock provider; records request headers.
async fn fake_anthropic() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let s = seen.clone();
    let app = Router::new().route(
        "/v1/messages",
        post(move |h: HeaderMap, Json(b): Json<Value>| async move {
            s.lock().unwrap().push(h);
            let req = CompletionRequest {
                model: b["model"].as_str().unwrap_or_default().into(),
                system: b["system"][0]["text"].as_str().unwrap_or_default().into(),
                user: b["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                max_tokens: 1000,
                json: true,
                images: vec![],
            };
            let c = glot_llm::mock::MockProvider.complete(req).await.unwrap();
            Json(json!({"model": b["model"], "stop_reason": "end_turn",
                "content": [{"type": "text", "text": c.text}],
                "usage": {"input_tokens": 10, "output_tokens": 10}}))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, seen)
}

#[tokio::test]
async fn project_llm_headers() {
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
    let (base, seen) = fake_anthropic().await;

    call(
        Method::POST,
        "/api/auth/signup",
        json!({"email": "h@example.com", "password": "correct horse", "name": "H"}),
    )
    .await;
    call(
        Method::POST,
        "/api/projects",
        json!({"name": "H", "slug": "hdr", "source_locale": "en", "target_locales": ["fr"]}),
    )
    .await;

    // Credentials are not "extra headers".
    let (s, v) = call(
        Method::PATCH,
        "/api/projects/hdr",
        json!({"llm": {"provider": "anthropic", "model": "claude-haiku-5-5", "extra_headers": {"x-api-key": "sk"}}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let (s, p) = call(
        Method::PATCH,
        "/api/projects/hdr",
        json!({"llm": {"provider": "anthropic", "model": "claude-haiku-5-5", "api_key": "sk-ant-usr-test",
            "base_url": base, "anthropic_workspace_id": "wrkspc_123", "extra_headers": {"X-Team": "growth"}}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["llm"]["anthropic_workspace_id"], "wrkspc_123");
    assert_eq!(p["llm"]["extra_headers"]["x-team"], "growth");
    // Omitted on a later update: kept.
    let (_, p) = call(
        Method::PATCH,
        "/api/projects/hdr",
        json!({"llm": {"provider": "anthropic", "model": "claude-haiku-5-5", "base_url": base}}),
    )
    .await;
    assert_eq!(p["llm"]["anthropic_workspace_id"], "wrkspc_123");

    call(
        Method::POST,
        "/api/projects/hdr/keys",
        json!({"key": "a", "source": "Delete it"}),
    )
    .await;
    call(
        Method::PUT,
        "/api/projects/hdr/keys/a/translations/fr",
        json!({"value": "Supprimer"}),
    )
    .await;
    let (s, v) = call(
        Method::POST,
        "/api/projects/hdr/review",
        json!({"locale": "fr"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let h = seen.lock().unwrap().last().cloned().expect("LLM called");
    assert_eq!(h["anthropic-workspace-id"], "wrkspc_123");
    assert_eq!(h["x-team"], "growth");
    assert_eq!(h["x-api-key"], "sk-ant-usr-test");

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
