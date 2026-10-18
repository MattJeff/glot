//! Live review RV4: AI rewrite of an annotation (rephrase → apply) on a real Postgres with the mock provider.
//! Skipped when Postgres is unreachable.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn rephrase_and_apply() {
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
        json!({"email": "rw@example.com", "password": "correct horse", "name": "RW"}),
    )
    .await;
    let (s, v) = call(
        Method::POST,
        "/api/projects",
        json!({"name": "RW", "slug": "rw", "source_locale": "en", "target_locales": ["fr", "de"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    call(
        Method::PATCH,
        "/api/projects/rw",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
    )
    .await;
    let (s, v) = call(
        Method::POST,
        "/api/projects/rw/keys",
        json!({"key": "cart.pay", "source": "Pay {amount} now"}),
    )
    .await;
    assert!(s.is_success(), "{v}");
    for (l, t) in [
        ("fr", "Payer {amount} maintenant"),
        ("de", "Jetzt {amount} zahlen"),
    ] {
        let (s, v) = call(
            Method::PUT,
            &format!("/api/projects/rw/keys/cart.pay/translations/{l}"),
            json!({"value": t}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (s, ann) = call(
        Method::POST,
        "/api/projects/rw/review/annotations",
        json!({"anchor": {"key": "cart.pay", "selector": "#pay", "text": "Payer {amount} maintenant",
                "url": "https://shop.example/fr/cart", "locale": "fr", "viewport": {"w": 390, "h": 844}},
               "body": "Trop long sur mobile"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{ann}");
    let id = ann["id"].as_str().unwrap().to_string();
    let a = format!("/api/projects/rw/review/annotations/{id}");

    // Bad input: segment preset without a segment.
    let (s, _) = call(
        Method::POST,
        &format!("{a}/rephrase"),
        json!({"preset": "segment"}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    let (s, r) = call(
        Method::POST,
        &format!("{a}/rephrase"),
        json!({"preset": "shorter"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["cached"], false);
    let props = r["proposals"].as_array().unwrap();
    assert_eq!(props.len(), 3, "{r}");
    for p in props {
        assert_eq!(p["valid"], true, "{p}");
        for l in ["en", "fr", "de"] {
            assert!(
                p["variants"][l]["value"]
                    .as_str()
                    .unwrap()
                    .contains("{amount}"),
                "{p}"
            );
        }
        assert!(p["variants"]["de"]["back_translation"].is_string(), "{p}");
    }
    assert!(r["usage"]["input_tokens"].as_u64().unwrap() > 0);
    // The first valid proposal became the annotation's suggestion.
    let (_, got) = call(Method::GET, &a, Value::Null).await;
    assert_eq!(got["suggestion"], props[0]["text"]);

    // Same (key, instruction, context): served from cache.
    let (s, r2) = call(
        Method::POST,
        &format!("{a}/rephrase"),
        json!({"preset": "shorter"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (r2["cached"].as_bool(), &r2["rewrite_id"]),
        (Some(true), &r["rewrite_id"])
    );

    // Apply proposal 2: every target locale written, annotation resolved, history keeps the original.
    let (s, ap) = call(
        Method::POST,
        &format!("{a}/rephrase/apply"),
        json!({"rewrite_id": r["rewrite_id"], "proposal_id": 2}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{ap}");
    assert_eq!(ap["annotation"]["status"], "resolved");
    assert_eq!(ap["source_text"], props[1]["variants"]["en"]["value"]);
    let (_, page) = call(Method::GET, "/api/projects/rw/keys", Value::Null).await;
    let k = &page["items"][0];
    for l in ["fr", "de"] {
        assert_eq!(
            k["translations"][l]["value"], props[1]["variants"][l]["value"],
            "{k}"
        );
        assert_eq!(k["translations"][l]["status"], "reviewed");
    }
    let (_, h) = call(
        Method::GET,
        "/api/projects/rw/keys/cart.pay/history",
        Value::Null,
    )
    .await;
    assert!(
        h.to_string().contains("Payer {amount} maintenant"),
        "original kept: {h}"
    );

    let (s, _) = call(
        Method::POST,
        &format!("{a}/rephrase/apply"),
        json!({"rewrite_id": r["rewrite_id"], "proposal_id": 9}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
