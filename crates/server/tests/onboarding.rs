//! Onboarding: token → push with that token → steps + milestones derived from real state, badge, hide.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn onboarding_end_to_end() {
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

    // auth: "cookie=…" or "bearer=…"
    let call = |method: Method, uri: &str, auth: Option<String>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        match auth.as_deref().and_then(|a| a.split_once('=')) {
            Some(("cookie", c)) => req = req.header(header::COOKIE, format!("glot_session={c}")),
            Some(("bearer", t)) => req = req.header(header::AUTHORIZATION, format!("Bearer {t}")),
            _ => {}
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
            let status = res.status();
            let cookie = res.headers().get(header::SET_COOKIE).map(|v| {
                v.to_str().unwrap().split(';').next().unwrap()["glot_session=".len()..].to_string()
            });
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                cookie,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let done = |o: &Value| -> Vec<String> {
        o["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["done"] == true)
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect()
    };
    let ids = |o: &Value| -> Vec<String> {
        o["achievements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect()
    };

    let (_, cookie, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "ob@x.io", "password": "correct horse", "name": "Ob"})),
    )
    .await;
    let c = Some(format!("cookie={}", cookie.unwrap()));
    let (s, _, _) = call(
        Method::POST,
        "/api/projects",
        c.clone(),
        Some(json!({"name": "Ob", "slug": "ob", "source_locale": "en", "target_locales": ["fr", "ja"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _, o) = call(Method::GET, "/api/projects/ob/onboarding", c.clone(), None).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert!(done(&o).is_empty(), "{o}");
    assert_eq!(o["keys"], 0);
    assert_eq!(o["first_push_at"], Value::Null);
    assert_eq!(
        o["locales"][1],
        json!({"locale": "ja", "translated": 0, "reviewed": 0, "missing": 0, "errors": 0})
    );

    // Not a member → 404, bad path → 400.
    let (s, _, _) = call(Method::GET, "/api/projects/ob/onboarding", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(
        Method::PATCH,
        "/api/projects/ob/onboarding",
        c.clone(),
        Some(json!({"path": "fax"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(
        Method::PATCH,
        "/api/projects/ob/onboarding",
        c.clone(),
        Some(json!({"path": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, _, tok) = call(
        Method::POST,
        "/api/projects/ob/onboarding/token",
        c.clone(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let token = tok["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("glot_"));
    let (_, _, o) = call(Method::GET, "/api/projects/ob/onboarding", c.clone(), None).await;
    assert_eq!(done(&o), ["path", "token"]);
    assert_eq!(o["path"], "terminal");

    // First push with the onboarding token: fr complete, ja partial.
    let (s, _, r) = call(
        Method::POST,
        "/api/projects/ob/sync/push",
        Some(format!("bearer={token}")),
        Some(json!({"catalogs": {
            "en": {"a": "Hello", "b": "Save {count} files"},
            "fr": {"a": "Bonjour", "b": "Enregistrer {count} fichiers"},
            "ja": {"a": "こんにちは"}
        }})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, _, o) = call(Method::GET, "/api/projects/ob/onboarding", c.clone(), None).await;
    assert_eq!(o["keys"], 2);
    assert!(o["first_push_at"].is_string());
    assert_eq!(
        done(&o),
        [
            "path",
            "token",
            "first_push",
            "first_translation",
            "zero_errors"
        ]
    );
    assert_eq!(ids(&o), ["first_locale_100", "first_push", "zero_errors"]);
    assert_eq!(o["locales"][1]["missing"], 1);

    // A placeholder error unchecks "zero errors"; the milestone stays.
    call(
        Method::PUT,
        "/api/projects/ob/keys/b/translations/ja",
        c.clone(),
        Some(json!({"value": "ファイルを保存"})),
    )
    .await;
    let (_, _, o) = call(Method::GET, "/api/projects/ob/onboarding", c.clone(), None).await;
    assert!(!done(&o).contains(&"zero_errors".to_string()), "{o}");
    assert_eq!(o["locales"][1]["errors"], 1);
    assert!(ids(&o).contains(&"zero_errors".to_string()));

    // Glossary, CDN and the README badge (public projects only; fetching it checks the step).
    call(
        Method::POST,
        "/api/projects/ob/glossary",
        c.clone(),
        Some(json!({"term": "glot", "do_not_translate": true})),
    )
    .await;
    let (s, _, _) = call(Method::GET, "/badge/ob.json", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    call(
        Method::PATCH,
        "/api/projects/ob",
        c.clone(),
        Some(json!({"public_cdn": true})),
    )
    .await;
    let (s, _, b) = call(Method::GET, "/badge/ob.json", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["schemaVersion"], 1);
    assert_eq!(b["message"], "100%"); // ja/b was set above
    let (_, _, o) = call(Method::GET, "/api/projects/ob/onboarding", c.clone(), None).await;
    for s in ["glossary", "cdn", "badge"] {
        assert!(done(&o).contains(&s.to_string()), "{s}: {o}");
    }
    assert!(ids(&o).contains(&"glossary_set".to_string()));

    call(
        Method::PATCH,
        "/api/projects/ob/onboarding",
        c.clone(),
        Some(json!({"hidden": true})),
    )
    .await;
    let (_, _, o) = call(Method::GET, "/api/projects/ob/onboarding", c.clone(), None).await;
    assert_eq!(o["hidden"], true);
    assert_eq!(o["path"], "terminal");

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
