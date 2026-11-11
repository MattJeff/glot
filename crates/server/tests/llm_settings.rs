//! Per-project LLM headers (anthropic_workspace_id, extra_headers): settings validation + the Review API
//! sending them to a fake Anthropic server. Skipped when Postgres is unreachable.
use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::{get, post},
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
    // `/v1/models` = the key test: good api / oauth keys pass, org keys need wrkspc_ok.
    let app = app.route(
        "/v1/models",
        get(|h: HeaderMap| async move {
            let hv = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default();
            let err = |s: StatusCode, m: &str| {
                (s, Json(json!({"type": "error", "error": {"type": "authentication_error", "message": m}})))
            };
            if hv("authorization") == "Bearer sk-ant-oat01-good"
                && hv("anthropic-beta") == "oauth-2025-04-20"
                && !h.contains_key("x-api-key")
            {
                return (StatusCode::OK, Json(json!({"data": []})));
            }
            match hv("x-api-key") {
                "sk-ant-api03-good" => (StatusCode::OK, Json(json!({"data": []}))),
                k if k.starts_with("sk-ant-usr") && hv("anthropic-workspace-id") == "wrkspc_ok" => {
                    (StatusCode::OK, Json(json!({"data": []})))
                }
                k if k.starts_with("sk-ant-usr") => err(
                    StatusCode::BAD_REQUEST,
                    "anthropic-workspace-id does not match a workspace of this key",
                ),
                _ => err(StatusCode::UNAUTHORIZED, "invalid x-api-key"),
            }
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

    // Key test (never the real API): stored org key + wrong workspace.
    let t = "/api/projects/hdr/llm/test";
    let (s, v) = call(Method::POST, t, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["code"], "revoked_or_wrong_workspace", "{v}");
    assert_eq!(v["status"], 400);
    assert_eq!(v["key_kind"], "org-user");
    let (_, v) = call(
        Method::POST,
        t,
        json!({"anthropic_workspace_id": "wrkspc_ok"}),
    )
    .await;
    assert_eq!(v["code"], "ok", "{v}");
    if std::env::var_os("ANTHROPIC_WORKSPACE_ID").is_none() {
        let (_, v) = call(
            Method::POST,
            t,
            json!({"api_key": "sk-ant-usr-x", "anthropic_workspace_id": ""}),
        )
        .await;
        assert_eq!(v["code"], "workspace_required", "{v}");
    }
    // Typed (unsaved) keys, copy-paste damage included.
    let (_, v) = call(
        Method::POST,
        t,
        json!({"api_key": " Bearer sk-ant-api03-good\n"}),
    )
    .await;
    assert_eq!(
        (v["ok"].clone(), v["key_kind"].clone()),
        (json!(true), json!("api")),
        "{v}"
    );
    let (_, v) = call(Method::POST, t, json!({"api_key": "sk-ant-api03-bad"})).await;
    assert_eq!(v["code"], "invalid_key", "{v}");
    assert_eq!(v["status"], 401);
    assert_eq!(v["provider_message"], "invalid x-api-key");
    assert!(
        !v.to_string().contains("sk-ant-api03-bad"),
        "never echo the key"
    );
    let (_, v) = call(Method::POST, t, json!({"api_key": "sk-ant-oat01-good"})).await;
    assert_eq!(
        (v["code"].clone(), v["key_kind"].clone()),
        (json!("ok"), json!("oauth")),
        "{v}"
    );
    let (_, v) = call(
        Method::POST,
        t,
        json!({"api_key": "sk-ant-api03-good", "base_url": "http://127.0.0.1:1"}),
    )
    .await;
    assert_eq!(v["code"], "network", "{v}");
    // Saved with a trailing newline: stored cleaned, so the stored key passes.
    let (s, _) = call(
        Method::PATCH,
        "/api/projects/hdr",
        json!({"llm": {"provider": "anthropic", "model": "claude-haiku-5-5", "base_url": base,
            "api_key": "  sk-ant-api03-good\n", "anthropic_workspace_id": ""}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(Method::POST, t, json!({})).await;
    assert_eq!(v["code"], "ok", "{v}");

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
