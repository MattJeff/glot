//! Segmented experiments, dashboard lifecycle: readiness blocker codes, idempotent named segments (wizard
//! autosave), duplicate, archive / unarchive, delete only drafts and archived experiments. Real Postgres.
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
    m: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(m)
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
        *cookie = c.trim_start_matches("glot_session=").to_string();
    }
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn experiment_lifecycle() {
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
    let (mut owner, mut tr) = (String::new(), String::new());
    for (c, email) in [(&mut owner, "o@example.com"), (&mut tr, "t@example.com")] {
        let (s, _) = call(
            &app,
            c,
            Method::POST,
            "/api/auth/signup",
            json!({"email": email, "password": "correct horse", "name": "N"}),
        )
        .await;
        assert!(s.is_success());
    }
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        "/api/projects",
        json!({"name": "X", "slug": "xl", "source_locale": "en", "target_locales": ["fr"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    sqlx::query("INSERT INTO project_members (project_id, user_id, role)
                 SELECT p.id, u.id, 'translator' FROM projects p, users u WHERE p.slug = 'xl' AND u.email = 't@example.com'")
        .execute(&pool).await.unwrap();
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        "/api/projects/xl/keys",
        json!({"key": "home.title", "source": "Ship faster"}),
    )
    .await;
    assert!(s.is_success());
    let base = "/api/projects/xl/xp";

    // Duration estimate: no traffic yet -> three lifts, no days; cliponit-like traffic -> +20 % takes over a year.
    let (s, e) = call(
        &app,
        &mut tr,
        Method::POST,
        &format!("{base}/estimate"),
        json!({"arms": 2}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    assert_eq!(
        (
            e["lifts"].as_array().unwrap().len(),
            &e["detectable_lift"],
            &e["warn_days"]
        ),
        (3, &Value::Null, &json!(56.0))
    );
    sqlx::query("INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
                 SELECT id, current_date - 1, 'en', 'en', '', '', ev, n FROM projects, (VALUES ('$view', 26185), ('signup', 52)) v(ev, n)
                 WHERE slug = 'xl'")
        .execute(&pool).await.unwrap();
    let (_, e) = call(
        &app,
        &mut tr,
        Method::POST,
        &format!("{base}/estimate"),
        json!({"arms": 2, "goal": "signup"}),
    )
    .await;
    let d20 = e["lifts"][1]["days"].as_f64().unwrap();
    let lift = e["detectable_lift"].as_f64().unwrap();
    assert!(
        (400.0..650.0).contains(&d20) && (0.55..0.8).contains(&lift),
        "{e}"
    );
    let (_, half) = call(
        &app,
        &mut tr,
        Method::POST,
        &format!("{base}/estimate"),
        json!({"arms": 2, "goal": "signup", "traffic_pct": 50}),
    )
    .await;
    assert!(
        (half["lifts"][1]["days"].as_f64().unwrap() / d20 - 2.0).abs() < 0.01,
        "{half}"
    );

    // Draft with a named segment: re-sending the same {name, rule} (autosave) reuses it instead of a 409.
    let cells =
        json!([{"name": "Phones", "rule": {"all": [{"attr": "device", "in": ["mobile"]}]}}]);
    let (s, x) = call(&app, &mut owner, Method::POST, base, json!({"name": "Hero", "keys": ["home.title"],
        "cells": cells, "arms": [{"intent": "test", "values": {"fr": {"home.title": "Livrez plus vite"}}}]})).await;
    assert_eq!(s, StatusCode::CREATED, "{x}");
    let id = x["id"].as_str().unwrap().to_string();
    let seg = x["cells"][0]["id"].clone();
    let xu = format!("{base}/{id}");
    let (s, x) = call(
        &app,
        &mut owner,
        Method::PATCH,
        &xu,
        json!({"cells": cells, "goal": "signup"}),
    )
    .await;
    assert_eq!((s, &x["cells"][0]["id"]), (StatusCode::OK, &seg), "{x}");
    let tablets =
        json!([{"name": "Phones", "rule": {"all": [{"attr": "device", "in": ["tablet"]}]}}]);
    let (s, x) = call(
        &app,
        &mut owner,
        Method::PATCH,
        &xu,
        json!({"cells": tablets}),
    )
    .await;
    assert_eq!(
        (s, &x["cells"][0]["rule"]["all"][0]["in"][0]),
        (StatusCode::OK, &json!("tablet"))
    );

    // Readiness codes: the English arm text is missing -> exactly where.
    let (_, r) = call(
        &app,
        &mut tr,
        Method::GET,
        &format!("{xu}/check"),
        Value::Null,
    )
    .await;
    assert_eq!(r["ready"], false);
    assert_eq!(
        r["blockers"],
        json!([{"code": "cell_missing", "arm": 1, "key": "home.title", "locale": "en"}]),
        "{r}"
    );

    // Duplicate: a new draft with the same definition and texts; translators cannot.
    let (s, _) = call(
        &app,
        &mut tr,
        Method::POST,
        &format!("{xu}/duplicate"),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, d) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/duplicate"),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    assert_eq!(
        (d["name"].as_str(), d["status"].as_str()),
        (Some("Hero (copy)"), Some("draft"))
    );
    assert_ne!(d["id"], x["id"]);
    assert_eq!(
        d["arms"][1]["values"]["fr"]["home.title"],
        "Livrez plus vite"
    );
    assert_eq!(d["cells"][0]["id"], seg);
    let du = format!("{base}/{}", d["id"].as_str().unwrap());
    let (s, d2) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{du}/duplicate"),
        json!({"name": "Hero v3"}),
    )
    .await;
    assert_eq!(
        (s, d2["name"].as_str()),
        (StatusCode::CREATED, Some("Hero v3"))
    );

    // A draft is deletable at once, but not archivable.
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{du}/archive"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(&app, &mut owner, Method::DELETE, &du, Value::Null).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Start -> running cannot be archived or deleted; stop -> archive keeps results -> delete.
    call(&app, &mut owner, Method::PATCH, &xu, json!({"cells": []})).await;
    call(
        &app,
        &mut owner,
        Method::PUT,
        &format!("{xu}/arms/1"),
        json!({"values": {"en": {"home.title": "Ship much faster"}}}),
    )
    .await;
    let (s, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{x}");
    for (m, path) in [
        (Method::POST, format!("{xu}/archive")),
        (Method::DELETE, xu.clone()),
    ] {
        let (s, _) = call(&app, &mut owner, m, &path, Value::Null).await;
        assert_eq!(s, StatusCode::CONFLICT, "{path}");
    }
    call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/stop"),
        Value::Null,
    )
    .await;
    let (s, _) = call(&app, &mut owner, Method::DELETE, &xu, Value::Null).await;
    assert_eq!(s, StatusCode::CONFLICT, "stopped, not archived");
    let (s, _) = call(
        &app,
        &mut tr,
        Method::POST,
        &format!("{xu}/archive"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/archive"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(x["archived_at"].is_string());
    let (_, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/unarchive"),
        Value::Null,
    )
    .await;
    assert!(x["archived_at"].is_null());
    let (s, _) = call(&app, &mut owner, Method::DELETE, &xu, Value::Null).await;
    assert_eq!(s, StatusCode::CONFLICT);
    call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/archive"),
        Value::Null,
    )
    .await;
    let (s, _) = call(&app, &mut owner, Method::DELETE, &xu, Value::Null).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, l) = call(&app, &mut owner, Method::GET, base, Value::Null).await;
    assert_eq!(l.as_array().unwrap().len(), 1, "only Hero v3 is left");

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db}"))
        .execute(&admin)
        .await
        .ok();
}
