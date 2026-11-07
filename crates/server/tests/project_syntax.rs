//! Per-project message syntax: validation follows it, `glot push` adopts glot.toml's while the
//! project has the default, the dashboard pins it, and every change rechecks stored issues.
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
async fn project_syntax() {
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
        json!({"email": "syntax@example.com", "password": "correct horse", "name": "S"}),
    )
    .await;
    call(
        Method::POST,
        "/api/projects",
        json!({"name": "Clips", "slug": "clips", "source_locale": "en", "target_locales": ["fr"]}),
    )
    .await;
    let codes = async |key: &str| -> Vec<String> {
        let v: Value = sqlx::query_scalar(
            "SELECT t.issues FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.key = $1 AND t.locale = 'fr'",
        )
        .bind(key)
        .fetch_one(&pool)
        .await
        .unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|i| i["code"].as_str().unwrap().to_string())
            .collect()
    };
    let catalogs = json!({
        "en": {"fill": "Fill in the {{parameters}} for {name}", "tag": "Press <key> to start"},
        "fr": {"fill": "Remplissez les {{parameters}} pour {name}", "tag": "Appuyez sur <key> pour commencer"}
    });

    // An old client (no syntax field) keeps the ICU default: `{{parameters}}` is invalid ICU.
    let (s, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"catalogs": catalogs}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["syntax"], "icu");
    assert!(codes("fill").await.contains(&"syntax_error".into()));

    // Push with glot.toml's `syntax = "plain"` adopts it (still the default) and rechecks what's stored.
    let (s, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"catalogs": catalogs, "syntax": "plain"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["syntax"], "plain");
    assert_eq!(codes("fill").await, Vec::<String>::new());
    assert_eq!(codes("tag").await, Vec::<String>::new());

    // Plain still guards `{name}`: dropping it is an error.
    let (s, t) = call(
        Method::PUT,
        "/api/projects/clips/keys/fill/translations/fr",
        json!({"value": "Remplissez les {{parameters}}"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert!(
        codes("fill").await.contains(&"placeholder_missing".into()),
        "{t}"
    );
    call(
        Method::PUT,
        "/api/projects/clips/keys/fill/translations/fr",
        json!({"value": "Remplissez les {{parameters}} pour {name}"}),
    )
    .await;

    // Not the default any more: a push only changes it with --set-syntax.
    let (_, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"catalogs": {}, "syntax": "icu"}),
    )
    .await;
    assert_eq!(r["syntax"], "plain");
    let (_, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"catalogs": {}, "syntax": "icu", "set_syntax": true}),
    )
    .await;
    assert_eq!(r["syntax"], "icu");
    assert!(codes("fill").await.contains(&"syntax_error".into()));

    // Dashboard: unknown names refused; a change revalidates and pins the choice against plain pushes.
    let (s, _) = call(
        Method::PATCH,
        "/api/projects/clips",
        json!({"syntax": "jinja"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, p) = call(
        Method::PATCH,
        "/api/projects/clips",
        json!({"syntax": "plain"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["syntax"], "plain");
    assert_eq!(codes("fill").await, Vec::<String>::new());
    let (_, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"catalogs": {}, "syntax": "i18next"}),
    )
    .await;
    assert_eq!(r["syntax"], "plain");
    let (_, pull) = call(Method::GET, "/api/projects/clips/sync/pull", Value::Null).await;
    assert_eq!(pull["syntax"], "plain");

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
