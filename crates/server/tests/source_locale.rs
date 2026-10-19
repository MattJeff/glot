//! Changing a project's source locale (PATCH source_locale): keys re-rooted on the new source's
//! translations, old source kept as a reviewed target, dependent translations stale, sync guarded.
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
async fn change_source_locale() {
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
        json!({"email": "src@example.com", "password": "correct horse", "name": "S"}),
    )
    .await;
    call(
        Method::POST,
        "/api/projects",
        json!({"name": "Clips", "slug": "clips", "source_locale": "en", "target_locales": ["de", "fr"]}),
    )
    .await;
    let (s, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({
            "catalogs": {
                "en": {"a": "Hello", "b": "Bye"},
                "fr": {"a": "Bonjour"},
                "de": {"a": "Hallo", "b": "Tschüss"}
            },
            "glossary": [
                {"term": "clip", "translations": {"fr": "extrait", "de": "Clip"}},
                {"term": "glot", "do_not_translate": true}
            ]
        }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");

    let (s, _) = call(
        Method::PATCH,
        "/api/projects/clips",
        json!({"source_locale": "not a locale"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, p) = call(
        Method::PATCH,
        "/api/projects/clips",
        json!({"source_locale": "fr"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["source_locale"], "fr");
    assert_eq!(p["target_locales"], json!(["de", "en"]));

    let source = async |key: &str| -> String {
        sqlx::query_scalar("SELECT source FROM keys WHERE key = $1")
            .bind(key)
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let tr = async |key: &str, locale: &str| -> Option<(String, String, String)> {
        sqlx::query_as(
            "SELECT t.value, t.status, t.origin FROM translations t JOIN keys k ON k.id = t.key_id
             WHERE k.key = $1 AND t.locale = $2",
        )
        .bind(key)
        .bind(locale)
        .fetch_optional(&pool)
        .await
        .unwrap()
    };
    // Re-rooted on the French text; "b" has no French yet and keeps its old text.
    assert_eq!(source("a").await, "Bonjour");
    assert_eq!(source("b").await, "Bye");
    assert_eq!(tr("a", "fr").await, None);
    let reviewed = |v: &str| Some((v.into(), "reviewed".into(), "human".into()));
    assert_eq!(tr("a", "en").await, reviewed("Hello"));
    assert_eq!(tr("b", "en").await, reviewed("Bye"));
    // Translated from a source that changed: stale, value kept. Unchanged source: untouched.
    assert_eq!(tr("a", "de").await.unwrap().0, "Hallo");
    assert_eq!(tr("a", "de").await.unwrap().1, "stale");
    assert_ne!(tr("b", "de").await.unwrap().1, "stale");
    assert_eq!(p["stats"]["locales"]["en"]["reviewed"], 2);

    let (_, pull) = call(Method::GET, "/api/projects/clips/sync/pull", Value::Null).await;
    assert_eq!(pull["source_locale"], "fr");
    assert_eq!(pull["catalogs"]["fr"]["a"], "Bonjour");
    assert_eq!(pull["catalogs"]["en"]["a"], "Hello");
    assert_eq!(pull["lock"]["entries"]["en"]["a"]["reviewed"], true);
    let terms: Vec<&str> = pull["glossary"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["term"].as_str().unwrap())
        .collect();
    assert!(
        terms.contains(&"extrait") && terms.contains(&"glot"),
        "{terms:?}"
    );

    // A client still configured with the old source is refused instead of merged.
    let (s, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"source_locale": "en", "catalogs": {"en": {"a": "Hello"}}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{r}");
    let (s, r) = call(
        Method::POST,
        "/api/projects/clips/sync/push",
        json!({"source_locale": "fr", "catalogs": {"fr": {"b": "Au revoir"}}}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(source("b").await, "Au revoir");

    // Back to English, dropping French explicitly.
    let (s, p) = call(
        Method::PATCH,
        "/api/projects/clips",
        json!({"source_locale": "en", "target_locales": ["de", "en"]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["source_locale"], "en");
    assert_eq!(p["target_locales"], json!(["de"]));
    assert_eq!(source("a").await, "Hello");
    assert_eq!(tr("a", "en").await, None);
    assert_eq!(tr("a", "fr").await, reviewed("Bonjour"));

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
