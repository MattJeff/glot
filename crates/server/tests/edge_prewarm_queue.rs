//! A pre-warm that discovers text ends with that text queued for translation, even when the auto-translation
//! failed minutes before (`tm::queue` refuses for 10 minutes after a failed job: the run waits the guard out).
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Html,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn prewarm_queues_translation_after_a_recent_failure() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let db = format!("glot_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let cache = tempfile::tempdir().unwrap();
    // SAFETY: set before any other thread reads the environment.
    unsafe {
        std::env::set_var("GLOT_EDGE_ALLOW_PRIVATE", "1");
        std::env::set_var("GLOT_EDGE_CACHE_DIR", cache.path());
        std::env::set_var("GLOT_EDGE_PREWARM_DELAY_MS", "0");
    }
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    glot_server::jobs::spawn_worker(state.clone());
    let app = glot_server::router(state.clone(), "http://localhost:3100");

    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let site = Router::new().fallback(|| async {
        Html("<!doctype html><html lang=\"en\"><head><title>Clip your videos</title></head><body><h1>Clip your videos</h1><p>Short clips from long videos.</p></body></html>")
    });
    tokio::spawn(async move { axum::serve(l, site).await.unwrap() });

    let call = |method: Method, uri: &str, cookie: Option<String>, body: Option<Value>| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (s, h) = (res.status(), res.headers().clone());
            let _ = res.into_body().collect().await.unwrap();
            (s, h)
        }
    };
    let (s, h) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "o@x.io", "password": "correct horse", "name": "O"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (s, _) = call(
        Method::POST,
        "/api/projects",
        Some(cookie.clone()),
        Some(json!({"name": "Site", "slug": "site", "source_locale": "en", "target_locales": ["es"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/site",
        Some(cookie),
        Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    let pid: Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'site'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at)
                 VALUES ($1, $2, 'es.site.test', 'subdomain', 'es', $3, 't', now())")
        .bind(Uuid::new_v4()).bind(pid).bind(&base)
        .execute(&pool).await.unwrap();
    // The last auto-translation failed just under 10 minutes ago: `queue` refuses for ~2 more seconds.
    sqlx::query(
        "INSERT INTO jobs (id, project_id, kind, status, params, created_at, finished_at)
         VALUES ($1, $2, 'edge_translate', 'failed', '{\"edge_locale\": \"es\", \"locales\": [\"es\"]}',
                 now() - interval '9 minutes 58 seconds', now() - interval '9 minutes 58 seconds')",
    )
    .bind(Uuid::new_v4())
    .bind(pid)
    .execute(&pool)
    .await
    .unwrap();

    let p = glot_server::projects::load_public(&state, "site")
        .await
        .unwrap()
        .unwrap();
    assert!(
        glot_server::edge::prewarm::start(&state, &p, true)
            .await
            .unwrap()
    );
    let t0 = Instant::now();
    loop {
        let st: String =
            sqlx::query_scalar("SELECT status FROM edge_prewarm WHERE project_id = $1")
                .bind(pid)
                .fetch_one(&pool)
                .await
                .unwrap();
        if st != "running" {
            assert_eq!(st, "done");
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(90), "pre-warm stuck");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (queued, translated): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM jobs WHERE project_id = $1 AND kind = 'edge_translate' AND status <> 'failed'),
                (SELECT count(*) FROM edge_translations WHERE project_id = $1 AND locale = 'es')",
    )
    .bind(pid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        queued, 1,
        "the run queued the translation once the guard lifted"
    );
    assert!(translated >= 2, "title and copy translated: {translated}");

    // The monthly budget is the project's: what other locales spent counts (it was summed per locale).
    sqlx::query(
        "INSERT INTO edge_settings (project_id, monthly_budget_usd) VALUES ($1, 1)
                 ON CONFLICT (project_id) DO UPDATE SET monthly_budget_usd = 1",
    )
    .bind(pid)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO jobs (id, project_id, kind, status, params, cost_usd, finished_at)
         VALUES ($1, $2, 'edge_translate', 'done', '{\"edge_locale\": \"de\"}', 2, now())",
    )
    .bind(Uuid::new_v4())
    .bind(pid)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM edge_translations WHERE project_id = $1 AND locale = 'es'")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();
    let settings = glot_server::edge::discover::Settings::load(&pool, pid)
        .await
        .unwrap();
    assert!(
        !glot_server::edge::tm::queue(&state, &p, "es", &settings)
            .await
            .unwrap(),
        "over the project budget: no new job, whichever locale spent it"
    );

    // A restart (deploy) kills the in-process run: the row left `running` is closed, not left to block 2 hours.
    sqlx::query(
        "UPDATE edge_prewarm SET status = 'running', finished_at = NULL WHERE project_id = $1",
    )
    .bind(pid)
    .execute(&pool)
    .await
    .unwrap();
    glot_server::jobs::requeue_orphans(&pool).await.unwrap();
    let st: String = sqlx::query_scalar("SELECT status FROM edge_prewarm WHERE project_id = $1")
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "failed", "an interrupted pre-warm is closed at startup");

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
