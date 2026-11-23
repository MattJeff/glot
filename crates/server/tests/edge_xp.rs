//! Copy experiments on Edge segments (`edge:<hash>` keys, hand-written arms) served by the Edge proxy
//! (`edge/xp.rs`): sticky arm + cookie, bots and DNT get the official text and are not counted, the page cache
//! never mixes arms, a server-side conversion forwarding `glot_xp` credits the arm. Real Postgres (DATABASE_URL,
//! default :5433), skipped when unreachable.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::get,
};
use base64::Engine;
use glot_core::edge::{
    segment::{SegmentOpts, segment},
    tm::template,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const PAGE: &str = r#"<!doctype html><html lang="en"><head><title>Clip maker</title></head>
<body><h1>Clip maker</h1><button>Get started</button></body></html>"#;
const BROWSER: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 Safari/605.1.15";

#[tokio::test]
async fn edge_segment_experiment() {
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
    // SAFETY: set before the proxy's first use, no other thread reads the environment yet.
    unsafe {
        std::env::set_var("GLOT_EDGE_ALLOW_PRIVATE", "1");
        std::env::set_var("GLOT_EDGE_CACHE_DIR", cache.path());
    }
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state, "http://localhost:3100");

    // Origin: a cacheable page; remembers the `x-glot-xp` it was sent.
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", l.local_addr().unwrap());
    let s2 = seen.clone();
    let site = Router::new().route(
        "/",
        get(move |h: HeaderMap| {
            s2.lock()
                .unwrap()
                .push(h.get("x-glot-xp").map(|v| v.to_str().unwrap().to_string()));
            async {
                (
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, "public, max-age=60"),
                    ],
                    PAGE,
                )
            }
        }),
    );
    tokio::spawn(async move { axum::serve(l, site).await.unwrap() });

    let call = |method: Method, uri: &str, h: &[(&str, &str)], body: Value| {
        let mut req = Request::builder().method(method).uri(uri);
        for (k, v) in h {
            req = req.header(*k, *v);
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let req = req.body(body).unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (s, h) = (res.status(), res.headers().clone());
            let b = res.into_body().collect().await.unwrap().to_bytes();
            (s, h, String::from_utf8_lossy(&b).into_owned())
        }
    };

    // Owner + project `wanted` (en → es), Edge domain, the page's segments with their official es text.
    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        &[("content-type", "application/json")],
        json!({"email": "o@example.com", "password": "correct horse", "name": "O"}),
    )
    .await;
    assert!(s.is_success());
    let session = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    sqlx::query("UPDATE users SET email_verified_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let owner = |m: Method, uri: String, body: Value| {
        let session = session.clone();
        let call = &call;
        async move {
            let (s, _, b) = call(
                m,
                &uri,
                &[("content-type", "application/json"), ("cookie", &session)],
                body,
            )
            .await;
            (s, serde_json::from_str::<Value>(&b).unwrap_or(Value::Null))
        }
    };
    let (s, _) = owner(
        Method::POST,
        "/api/projects".into(),
        json!({"name": "Wanted", "slug": "wanted", "source_locale": "en", "target_locales": ["es"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let project: Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'wanted'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE projects SET public_cdn = true WHERE id = $1")
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at)
                 VALUES ($1, $2, 'es.wanted.test', 'subdomain', 'es', $3, 't', now())")
        .bind(Uuid::new_v4()).bind(project).bind(&origin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_wanted')")
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    let tpl = template("/", &["es".to_string(), "en".into()]);
    let mut cta = String::new();
    for s in segment(PAGE, &SegmentOpts::new("en")) {
        let ctx = format!("{}|{tpl}", s.ctx.role);
        let es = match s.text.as_str() {
            "Get started" => {
                cta = s.hash.clone();
                "Empezar"
            }
            _ => "Creador de clips",
        };
        sqlx::query("INSERT INTO edge_segments (project_id, hash, context, source_text) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
            .bind(project).bind(&s.hash).bind(&ctx).bind(&s.text).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO edge_translations (project_id, hash, context, locale, value, status) VALUES ($1, $2, $3, 'es', $4, 'reviewed') ON CONFLICT DO NOTHING")
            .bind(project).bind(&s.hash).bind(&ctx).bind(es).execute(&pool).await.unwrap();
    }
    assert!(!cta.is_empty());

    // --- The experiment: `edge:<hash>`, a hand-written es arm, everyone on it (fixed 0/1), sticky per session.
    let base = "/api/projects/wanted/xp".to_string();
    let (s, _) = owner(
        Method::POST,
        base.clone(),
        json!({"name": "x", "keys": ["edge:nope"]}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = owner(
        Method::PUT,
        format!("{base}/settings"),
        json!({"persistence": "session"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let key = format!("edge:{cta}");
    let (s, x) = owner(
        Method::POST,
        base.clone(),
        json!({"name": "CTA es", "keys": [&key], "locales": ["es"], "goal": "signup",
               "allocation": "fixed", "weights": [0.0, 1.0],
               "arms": [{"intent": "urgency", "values": {"es": {(key.clone()): "Empieza gratis ahora"}}}]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{x}");
    assert_eq!(x["keys"][0]["source"], "Get started");
    let id = x["id"].as_str().unwrap().to_string();
    let (s, x) = owner(Method::POST, format!("{base}/{id}/start"), Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{x}");
    let (s, x) = owner(
        Method::POST,
        base.clone(),
        json!({"name": "dup", "keys": [&key]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = owner(
        Method::POST,
        format!("{base}/{}/start", x["id"].as_str().unwrap()),
        Value::Null,
    )
    .await;
    assert_ne!(s, StatusCode::OK, "one live experiment per key");
    let (_, _, cfg) = call(
        Method::GET,
        "/cdn/wanted/experiments.json",
        &[],
        Value::Null,
    )
    .await;
    assert!(cfg.contains(&id), "{cfg}");

    let page = |extra: &[(&str, &str)]| {
        let mut h = vec![
            ("host", "es.wanted.test"),
            ("accept", "text/html"),
            ("sec-fetch-dest", "document"),
        ];
        h.extend_from_slice(extra);
        let h: Vec<(String, String)> = h
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let call = &call;
        async move {
            let h: Vec<(&str, &str)> = h.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            call(Method::GET, "/edge/proxy/", &h, Value::Null).await
        }
    };
    let counts = |event: &'static str| {
        let (pool, id) = (pool.clone(), id.clone());
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT coalesce(sum(n), 0)::int8 FROM xp_counts WHERE experiment_id = $1::uuid AND arm = 1 AND event = $2",
            )
            .bind(id)
            .bind(event)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    // Googlebot first: official text, no assignment, no cookie; its page lands in the page cache.
    let (s, h, b) = page(&[("user-agent", "Mozilla/5.0 (compatible; Googlebot/2.1)")]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        b.contains("Empezar") && !b.contains("Empieza gratis ahora"),
        "{b}"
    );
    assert!(h.get(header::SET_COOKIE).is_none());
    assert_eq!(seen.lock().unwrap().last(), Some(&None));

    // A visitor: the arm's text (not the cached official page), sticky cookie, `x-glot-xp` to the origin.
    let (s, h, b) = page(&[("user-agent", BROWSER)]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        b.contains("Empieza gratis ahora") && !b.contains("Empezar"),
        "{b}"
    );
    let set = h[header::SET_COOKIE].to_str().unwrap().to_string();
    assert_eq!(
        set,
        format!("glot_xp={id}.1.*; Path=/; SameSite=Lax; Secure")
    );
    assert!(
        h[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .starts_with("private")
    );
    let sent = seen
        .lock()
        .unwrap()
        .last()
        .cloned()
        .flatten()
        .expect("x-glot-xp");
    let sent: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sent)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(sent["a"][0]["arm"], 1);
    assert_eq!(sent["o"][key.as_str()], "Empieza gratis ahora");
    let mut n = 0;
    for _ in 0..50 {
        n = counts("$exposure").await;
        if n > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(n, 1);

    // Same session: same arm, no new cookie, not counted again; a forged `x-glot-xp` never reaches the origin.
    let cookie = set.split(';').next().unwrap().to_string();
    let (_, h, b) = page(&[
        ("user-agent", BROWSER),
        ("cookie", &cookie),
        ("x-glot-xp", "forged"),
    ])
    .await;
    assert!(b.contains("Empieza gratis ahora"), "{b}");
    assert!(h.get(header::SET_COOKIE).is_none());
    assert_ne!(
        seen.lock().unwrap().last().cloned().flatten().as_deref(),
        Some("forged")
    );
    // Do Not Track and bots again (from the page cache): official text, nothing counted.
    let (_, h, b) = page(&[("user-agent", BROWSER), ("dnt", "1")]).await;
    assert!(
        b.contains("Empezar") && h.get(header::SET_COOKIE).is_none(),
        "{b}"
    );
    let (_, h, b) = page(&[("user-agent", "Googlebot/2.1")]).await;
    assert!(b.contains("Empezar") && h.get(header::SET_COOKIE).is_none());
    assert_eq!(h["x-glot-edge"], "translated; cache=hit");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(counts("$exposure").await, 1);

    // Server-side conversion: the origin's server forwards the visitor's cookie value.
    let value = cookie.trim_start_matches("glot_xp=");
    let (s, _, b) = call(
        Method::POST,
        "/api/insights/pk_wanted",
        &[
            ("user-agent", "cliponit-server/1"),
            ("content-type", "application/json"),
        ],
        json!({"served": "es", "event": "signup", "glot_xp": value, "dedup": "order-12345"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(counts("signup").await, 1);
}
