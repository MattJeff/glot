//! Segmented experiments API (contract: docs/design/segmented-copy-experiments-contract.md §4), real Postgres,
//! billing on (free → pro): draft → readiness per cell → quotas → start → pause/resume → promote → segments/settings.
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::Arc;
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
async fn segmented_experiments_api() {
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
    let mut state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    state.billing = Some(Arc::new(glot_server::billing::Config::new(
        "sk_test",
        "whsec",
        "http://localhost:12111",
        "http://localhost:3100",
    )));
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
        json!({"name": "X", "slug": "xp", "source_locale": "en", "target_locales": ["fr", "de"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    sqlx::query("INSERT INTO project_members (project_id, user_id, role)
                 SELECT p.id, u.id, 'translator' FROM projects p, users u WHERE p.slug = 'xp' AND u.email = 't@example.com'")
        .execute(&pool).await.unwrap();
    for (k, src) in [
        ("home.hero.title", "Ship {count} apps"),
        ("home.hero.cta", "Start free"),
        ("pricing.title", "Pricing"),
    ] {
        let (s, _) = call(
            &app,
            &mut owner,
            Method::POST,
            "/api/projects/xp/keys",
            json!({"key": k, "source": src}),
        )
        .await;
        assert!(s.is_success(), "{k}");
    }
    let base = "/api/projects/xp/xp";

    // Validation at the boundary.
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        base,
        json!({"name": "x", "keys": ["nope"]}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        base,
        json!({"name": "x", "prefix": "home.hero",
        "cells": [{"name": "bad", "rule": {"all": [{"attr": "ip", "eq": "1.2.3.4"}]}}]}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        &app,
        &mut tr,
        Method::POST,
        base,
        json!({"name": "x", "prefix": "home.hero"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Draft: prefix → key group, inline segment, one arm partially filled.
    let rule = json!({"all": [{"attr": "country", "in": ["DE", "AT", "CH"]}, {"attr": "source", "eq": "search"}]});
    let (s, x) = call(&app, &mut owner, Method::POST, base, json!({
        "name": "Hero", "prefix": "home.hero", "cells": [{"name": "DACH search", "rule": rule}],
        "allocation": "segmented", "facts": ["2 000 teams"],
        "arms": [{"intent": "social proof", "values": {"fr": {"home.hero.title": "Livrez {n} apps"}}}]})).await;
    assert_eq!(s, StatusCode::CREATED, "{x}");
    assert_eq!(x["status"], "draft");
    assert_eq!(x["keys"].as_array().unwrap().len(), 2);
    assert_eq!(x["locales"], json!(["de", "en", "fr"]));
    assert_eq!(x["cells"][0]["name"], "DACH search");
    assert_eq!(x["arms"][0]["intent"], "official");
    let id = x["id"].as_str().unwrap().to_string();
    let seg = x["cells"][0]["id"].as_str().unwrap().to_string();
    let xu = format!("{base}/{id}");

    // Readiness: report per (arm, key, locale); broken placeholder flagged.
    let (s, r) = call(
        &app,
        &mut tr,
        Method::GET,
        &format!("{xu}/check"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["ready"], false);
    assert_eq!(r["cells"].as_array().unwrap().len(), 6);
    let fr_title = r["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == "home.hero.title" && c["locale"] == "fr")
        .unwrap();
    assert_eq!(fr_title["ok"], false);
    assert!(
        !fr_title["issues"].as_array().unwrap().is_empty(),
        "{fr_title}"
    );
    let (s, e) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "not_ready");
    assert!(e["readiness"]["problems"].as_array().unwrap().len() >= 2);

    // Translator fixes the texts (cannot approve).
    let vals = json!({
        "en": {"home.hero.title": "Join 2 000 teams shipping {count} apps", "home.hero.cta": "Start free today"},
        "fr": {"home.hero.title": "Livrez {count} apps", "home.hero.cta": "Essai gratuit"},
        "de": {"home.hero.title": "{count} Apps ausliefern", "home.hero.cta": "Gratis starten"}});
    let (s, _) = call(
        &app,
        &mut tr,
        Method::PUT,
        &format!("{xu}/arms/1"),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, a) = call(
        &app,
        &mut tr,
        Method::PUT,
        &format!("{xu}/arms/1"),
        json!({"values": vals}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!(a["values"]["fr"]["home.hero.title"], "Livrez {count} apps");
    let (s, _) = call(
        &app,
        &mut tr,
        Method::PUT,
        &format!("{xu}/arms/1"),
        json!({"values": {"it": {"home.hero.cta": "x"}}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, r) = call(
        &app,
        &mut tr,
        Method::GET,
        &format!("{xu}/check"),
        Value::Null,
    )
    .await;
    assert_eq!(r["ready"], true, "{r}");

    // Quotas: free plan has no segmentation; pro allows 2 segments.
    let (s, e) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED, "{e}");
    assert_eq!(e["error"]["code"], "plan_required");
    sqlx::query("INSERT INTO billing_accounts (user_id, plan, status) SELECT id, 'pro', 'active' FROM users WHERE email = 'o@example.com'")
        .execute(&pool).await.unwrap();
    let (s, _) = call(
        &app,
        &mut tr,
        Method::POST,
        &format!("{xu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{x}");
    assert_eq!(x["status"], "running");
    assert_eq!(x["state"].as_array().unwrap().len(), 2);
    assert_eq!(x["state"][0]["cell"], seg.as_str());
    assert_eq!(x["state"][1]["cell"], "*");
    assert_eq!(x["state"][1]["weights"], json!([0.5, 0.5]));

    // Running: shape frozen, name editable.
    let (s, _) = call(
        &app,
        &mut owner,
        Method::PATCH,
        &xu,
        json!({"keys": ["pricing.title"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, x) = call(
        &app,
        &mut owner,
        Method::PATCH,
        &xu,
        json!({"name": "Hero v2"}),
    )
    .await;
    assert_eq!((s, x["name"].as_str()), (StatusCode::OK, Some("Hero v2")));
    let (s, _) = call(
        &app,
        &mut owner,
        Method::PUT,
        &format!("{xu}/arms/1"),
        json!({"values": vals}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // One live experiment per key: a second one on the CTA cannot start; the legacy path refuses it too.
    let (s, y) = call(&app, &mut owner, Method::POST, base, json!({"name": "CTA", "keys": ["home.hero.cta"],
        "locales": ["fr"], "review": true, "arms": [{"intent": "benefit", "values": {"fr": {"home.hero.cta": "Gratuit"}}}]})).await;
    assert_eq!(s, StatusCode::CREATED, "{y}");
    assert_eq!(y["arms"][1]["approved"], false);
    let yu = format!("{base}/{}", y["id"].as_str().unwrap());
    let (s, _) = call(
        &app,
        &mut owner,
        Method::PUT,
        &format!("{yu}/arms/1"),
        json!({"approved": true}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, e) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{yu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already live"),
        "{e}"
    );
    call(
        &app,
        &mut owner,
        Method::PUT,
        "/api/projects/xp/keys/home.hero.cta/translations/fr",
        json!({"value": "Commencer"}),
    )
    .await;
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        "/api/projects/xp/experiments",
        json!({"key": "home.hero.cta", "locale": "fr"}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Pause / resume / personalize the DACH cell / promote everywhere.
    let (_, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/pause"),
        Value::Null,
    )
    .await;
    assert_eq!(x["status"], "paused");
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/pause"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/resume"),
        Value::Null,
    )
    .await;
    assert_eq!(x["status"], "running");
    let (s, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/promote"),
        json!({"scope": "segment", "arm": 1, "cell": seg}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{x}");
    assert_eq!(x["status"], "personalized");
    assert_eq!(x["state"][0]["weights"], json!([0.0, 1.0]));
    assert_eq!(x["state"][0]["personalized"], true);
    let (s, x) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{xu}/promote"),
        json!({"scope": "all", "arm": 1}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{x}");
    assert_eq!(x["status"], "promoted");
    let fr: String = sqlx::query_scalar(
        "SELECT t.value FROM translations t JOIN keys k ON k.id = t.key_id
                                         WHERE k.key = 'home.hero.title' AND t.locale = 'fr'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fr, "Livrez {count} apps");
    let src: String = sqlx::query_scalar("SELECT source FROM keys WHERE key = 'home.hero.title'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        src, "Ship {count} apps",
        "the source locale is never rewritten by a promotion"
    );
    // Keys freed: the CTA experiment can start now (pro: 2 live max 5).
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{yu}/start"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, &mut owner, Method::DELETE, &yu, Value::Null).await;
    assert_eq!(s, StatusCode::CONFLICT);
    call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{yu}/stop"),
        Value::Null,
    )
    .await;
    let (s, _) = call(&app, &mut owner, Method::DELETE, &yu, Value::Null).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Segments.
    let (s, l) = call(
        &app,
        &mut tr,
        Method::GET,
        &format!("{base}/segments"),
        Value::Null,
    )
    .await;
    assert_eq!((s, l.as_array().unwrap().len()), (StatusCode::OK, 1));
    let (s, _) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{base}/segments"),
        json!({"name": "DACH search", "rule": rule}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, m) = call(
        &app,
        &mut owner,
        Method::POST,
        &format!("{base}/segments"),
        json!({"name": "Mobile", "rule": {"all": [{"attr": "device", "eq": "mobile"}]}}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, m) = call(
        &app,
        &mut owner,
        Method::PATCH,
        &format!("{base}/segments/{}", m["id"].as_str().unwrap()),
        json!({"name": "Phones"}),
    )
    .await;
    assert_eq!((s, m["name"].as_str()), (StatusCode::OK, Some("Phones")));
    let (s, _) = call(
        &app,
        &mut owner,
        Method::DELETE,
        &format!("{base}/segments/{seg}"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "used by a started experiment");
    let (s, _) = call(
        &app,
        &mut owner,
        Method::DELETE,
        &format!("{base}/segments/{}", m["id"].as_str().unwrap()),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Settings: owner only; holdout pseudo-experiment.
    let (s, _) = call(
        &app,
        &mut tr,
        Method::PUT,
        &format!("{base}/settings"),
        json!({"holdout_pct": 5}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, st) = call(
        &app,
        &mut owner,
        Method::PUT,
        &format!("{base}/settings"),
        json!({"holdout_pct": 5, "persistence": "session"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{st}");
    assert_eq!(
        (st["holdout_pct"].as_i64(), st["persistence"].as_str()),
        (Some(5), Some("session"))
    );
    assert!(st["holdout_id"].is_string());
    let (s, _) = call(
        &app,
        &mut owner,
        Method::PUT,
        &format!("{base}/settings"),
        json!({"holdout_pct": 50}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, l) = call(&app, &mut tr, Method::GET, base, Value::Null).await;
    assert_eq!(
        l.as_array().unwrap().len(),
        1,
        "holdout and deleted experiments are not listed"
    );

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db}"))
        .execute(&admin)
        .await
        .ok();
}
