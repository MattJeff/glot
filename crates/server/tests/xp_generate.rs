//! AI variants by intention + copy policy + personalization without a test (real Postgres, mock LLM; skipped
//! when Postgres is unreachable).
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

async fn call(
    app: &Router,
    cookie: &mut String,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
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
        *cookie = c
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("glot_session=")
            .to_string();
    }
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn generate_policy_personalize() {
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
    let c = &mut String::new();
    let (s, _) = call(
        &app,
        c,
        Method::POST,
        "/api/auth/signup",
        json!({"email": "g@example.com", "password": "correct horse", "name": "G"}),
    )
    .await;
    assert!(s.is_success());
    let (s, _) = call(
        &app,
        c,
        Method::POST,
        "/api/projects",
        json!({"name": "G", "slug": "gx", "source_locale": "en", "target_locales": ["fr", "de"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        &app,
        c,
        Method::PATCH,
        "/api/projects/gx",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
    )
    .await;
    for (k, src, fr) in [
        (
            "home.hero.title",
            "Translate your app",
            "Traduisez votre app",
        ),
        ("home.hero.cta", "Start free", "Commencer"),
    ] {
        call(
            &app,
            c,
            Method::POST,
            "/api/projects/gx/keys",
            json!({"key": k, "source": src}),
        )
        .await;
        let (s, _) = call(
            &app,
            c,
            Method::PUT,
            &format!("/api/projects/gx/keys/{k}/translations/fr"),
            json!({"value": fr}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }
    let base = "/api/projects/gx/xp";

    // Policy settings: validated, round-trip.
    let (s, _) = call(
        &app,
        c,
        Method::PUT,
        &format!("{base}/policy"),
        json!({"rules": {"nope": "off"}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let pol = json!({"rules": {"superlative": "error"}, "forbidden": {"de": ["kostenlos"]}, "patterns": {}});
    let (s, got) = call(&app, c, Method::PUT, &format!("{base}/policy"), pol.clone()).await;
    assert_eq!((s, &got), (StatusCode::OK, &pol));
    assert_eq!(
        call(&app, c, Method::GET, &format!("{base}/policy"), Value::Null)
            .await
            .1,
        pol
    );

    let (s, x) = call(&app, c, Method::POST, base, json!({
        "name": "Hero DACH", "prefix": "home.hero", "locales": ["en", "fr", "de"],
        "cells": [{"name": "DACH", "rule": {"all": [{"attr": "country", "in": ["DE", "AT", "CH"]}]}}],
        "facts": ["2,000 teams"],
    })).await;
    assert_eq!(s, StatusCode::CREATED, "{x}");
    let id = x["id"].as_str().unwrap().to_string();

    let (s, _) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/generate"),
        json!({"intents": ["dark pattern"]}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, g) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/generate"),
        json!({"intents": ["social proof", "segment"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{g}");
    let arms = g["arms"].as_array().unwrap();
    assert_eq!(arms.len(), 2, "{g}");
    assert_eq!(arms[0]["intent"], "social proof");
    for l in ["en", "fr", "de"] {
        assert_eq!(
            arms[0]["values"][l].as_object().unwrap().len(),
            2,
            "{l}: {g}"
        );
    }
    assert!(
        arms[0]["values"]["de"]["home.hero.cta"]
            .as_str()
            .unwrap()
            .starts_with("⟦de⟧")
    );
    // Regenerating an intent replaces its arm (no new slot).
    let (_, g2) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/generate"),
        json!({"intents": ["social_proof"], "locales": ["en"]}),
    )
    .await;
    assert_eq!(g2["arms"][0]["arm"], arms[0]["arm"], "{g2}");

    // A hand-edited fake urgency is refused at personalization (policy, not only the validator).
    let arm = arms[0]["arm"].as_i64().unwrap();
    let (s, _) = call(
        &app,
        c,
        Method::PUT,
        &format!("{base}/{id}/arms/{arm}"),
        json!({"values": {"en": {"home.hero.title": "Only 2 seats left!"}}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let cell = x["cells"][0]["id"].as_str().unwrap().to_string();
    let (s, e) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/personalize"),
        json!({"arm": arm, "cell": cell}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(
        e["error"]["message"].as_str().unwrap().contains("urgency"),
        "{e}"
    );
    call(
        &app,
        c,
        Method::PUT,
        &format!("{base}/{id}/arms/{arm}"),
        json!({"values": {"en": {"home.hero.title": "Translate your app (social proof)"}}}),
    )
    .await;

    // Personalization: the segment gets the arm at 100 %, everyone else the official text, nothing counted.
    let (s, p) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/personalize"),
        json!({"arm": arm, "cell": cell}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"], "personalized");
    let st = p["state"].as_array().unwrap();
    assert_eq!(st.len(), 2);
    for row in st {
        assert_eq!(row["personalized"], true);
        let w: Vec<f64> = serde_json::from_value(row["weights"].clone()).unwrap();
        let hot = if row["cell"] == cell.as_str() {
            arm as usize
        } else {
            0
        };
        assert_eq!(w[hot], 1.0, "{row}");
    }
    let (s, _) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/generate"),
        json!({"intents": ["simplicity"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "generation is for drafts");
    let (s, _) = call(
        &app,
        c,
        Method::POST,
        &format!("{base}/{id}/stop"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    drop(pool);
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
