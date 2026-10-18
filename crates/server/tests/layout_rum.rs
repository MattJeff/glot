//! Layout RUM end to end: beacons → aggregates per key × locale × viewport band → repair loop (mock shorten,
//! validator + pixel budget) → proposed correction in the feedback inbox without a repo, one PR through the
//! GitHub App with one (fake GitHub). Real Postgres, skipped when unreachable.
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode, header},
    response::IntoResponse,
};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

const KEY: &str = include_str!("fixtures/github-app-test-key.pem");
type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

async fn fake_github(State(log): State<Log>, req: Request<Body>) -> axum::response::Response {
    let (m, p) = (req.method().to_string(), req.uri().path().to_string());
    let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
        .unwrap_or(Value::Null);
    log.lock().unwrap().push((m.clone(), p.clone(), body));
    let r = |v: Value| axum::Json(v).into_response();
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    match (m.as_str(), p.as_str()) {
        ("POST", "/app/installations/7/access_tokens") => r(json!({ "token": "ghs_test" })),
        ("GET", "/repos/acme/app/git/ref/heads/main") => r(json!({ "object": { "sha": "base1" } })),
        ("GET", "/repos/acme/app/git/commits/base1") => r(json!({ "tree": { "sha": "tree1" } })),
        ("GET", "/repos/acme/app/contents/messages/de.json") => r(json!({ "content": b64(
            r#"{"invoice":{"remind":"Zahlungserinnerung senden"},"cta":{"trial":"Kostenlose Testversion jetzt starten"}}"#
        ) })),
        ("POST", "/repos/acme/app/git/trees") => r(json!({ "sha": "tree2" })),
        ("POST", "/repos/acme/app/git/commits") => r(json!({ "sha": "commit2" })),
        ("POST", "/repos/acme/app/git/refs") => r(json!({ "ref": "x" })),
        ("POST", "/repos/acme/app/pulls") => {
            r(json!({ "number": 9, "html_url": "https://github.com/acme/app/pull/9" }))
        }
        _ => StatusCode::NOT_FOUND.into_response(), // glot.lock: absent
    }
}

#[tokio::test]
async fn layout_rum_end_to_end() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let log: Log = Default::default();
    let gh = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gh_url = format!("http://{}", gh.local_addr().unwrap());
    let fake = Router::new().fallback(fake_github).with_state(log.clone());
    tokio::spawn(async move { axum::serve(gh, fake).await.unwrap() });
    // SAFETY: set before any server code reads it; this test binary has a single test.
    unsafe { std::env::set_var("GLOT_GITHUB_API", &gh_url) };

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
    let app = glot_server::router(state.clone(), "http://localhost:3100");

    let mut cookie = String::new();
    let mut call =
        async |method: Method, uri: &str, body: Value, ua: &str| -> (StatusCode, Value) {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                // The beacon posts text/plain (no CORS preflight); the dashboard API takes JSON.
                .header(
                    header::CONTENT_TYPE,
                    if uri.starts_with("/api/insights/") {
                        "text/plain;charset=UTF-8"
                    } else {
                        "application/json"
                    },
                )
                .header(header::USER_AGENT, ua);
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
                cookie = c.to_str().unwrap().split(';').next().unwrap()["glot_session=".len()..]
                    .to_string();
            }
            let status = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            )
        };
    let ua = "Mozilla/5.0";
    let json_call = |m: Method, u: &str, b: Value| (m, u.to_string(), b);
    for (m, u, b) in [
        json_call(
            Method::POST,
            "/api/auth/signup",
            json!({"email": "l@example.com", "password": "correct horse", "name": "L"}),
        ),
        json_call(
            Method::POST,
            "/api/projects",
            json!({"name": "D", "slug": "demo", "source_locale": "en", "target_locales": ["de"]}),
        ),
        json_call(
            Method::PATCH,
            "/api/projects/demo",
            json!({"llm": {"provider": "mock", "model": "mock"}}),
        ),
        json_call(
            Method::POST,
            "/api/projects/demo/keys",
            json!({"key": "invoice.remind", "source": "Send payment reminder"}),
        ),
        json_call(
            Method::PUT,
            "/api/projects/demo/keys/invoice.remind/translations/de",
            json!({"value": "Zahlungserinnerung senden"}),
        ),
        json_call(
            Method::POST,
            "/api/projects/demo/keys",
            json!({"key": "cta.trial", "source": "Start your free trial now"}),
        ),
        json_call(
            Method::PUT,
            "/api/projects/demo/keys/cta.trial/translations/de",
            json!({"value": "Kostenlose Testversion jetzt starten"}),
        ),
        json_call(
            Method::POST,
            "/api/projects/demo/keys",
            json!({"key": "nav.home", "source": "Home"}),
        ),
    ] {
        let (s, v) = call(m, &u, b, ua).await;
        assert!(s.is_success(), "{u}: {s} {v}");
    }
    let (_, v) = call(Method::GET, "/api/projects/demo/insights", Value::Null, ua).await;
    let pk = v["public_key"].as_str().unwrap().to_string();
    let beacon = format!("/api/insights/{pk}/layout");

    // 40 mobile page views where the reminder button truncates, 10 where it fits (wider phone in the band),
    // and 4 where it overflowed only because the webfont had not loaded yet.
    let item = |o: u8, fl: u8| json!({"k": "invoice.remind", "o": o, "b": 120, "n": 180, "f": "Inter", "fs": 14, "fl": fl});
    for i in 0..54 {
        let (o, fl) = match i {
            0..40 => (1, 1),
            40..50 => (0, 1),
            _ => (1, 0),
        };
        let items = json!([item(o, fl), item(0, 1), {"k": "nav.home", "o": 0}]);
        let (s, v) = call(
            Method::POST,
            &beacon,
            json!({"locale": "de", "vw": 370, "items": items}),
            ua,
        )
        .await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    }
    // Desktop: fine.
    for _ in 0..30 {
        call(
            Method::POST,
            &beacon,
            json!({"locale": "de", "vw": 1440, "items": [item(0, 1)]}),
            ua,
        )
        .await;
    }
    // Bots are ignored, bad bodies rejected, unknown keys 404.
    let (s, _) = call(
        Method::POST,
        &beacon,
        json!({"locale": "de", "vw": 370, "items": [item(1, 1)]}),
        "Googlebot/2.1",
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(
        Method::POST,
        &beacon,
        json!({"locale": "not a locale", "vw": 370, "items": []}),
        ua,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        Method::POST,
        "/api/insights/pk_nope/layout",
        json!({"locale": "de", "vw": 1, "items": []}),
        ua,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, v) = call(
        Method::GET,
        "/api/projects/demo/insights/layout",
        Value::Null,
        ua,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let top = &v["rows"][0];
    assert_eq!(
        (
            top["key"].as_str(),
            top["vw_min"].as_i64(),
            top["vw_max"].as_i64(),
            top["views"].as_i64()
        ),
        (Some("invoice.remind"), Some(360), Some(390), Some(54)),
        "{v}"
    );
    assert_eq!(
        (top["overflows"].as_i64(), top["fallback"].as_i64()),
        (Some(44), Some(4))
    );
    assert!((top["rate"].as_f64().unwrap() - 40.0 / 54.0).abs() < 1e-9);
    assert_eq!(
        (top["box_px"].as_i64(), top["need_px"].as_i64()),
        (Some(120), Some(180))
    );
    assert_eq!(top["current"], "Zahlungserinnerung senden");
    // No PII anywhere in the table.
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT row_to_json(d)::text FROM layout_rum_daily d")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!stored.join("").contains("Mozilla") && !stored.join("").contains("Googlebot"));

    // ---- repair without a linked repo: proposed correction in the feedback inbox ----
    let (s, v) = call(
        Method::POST,
        "/api/projects/demo/insights/layout/repair",
        Value::Null,
        ua,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["created"], 1, "{v}");
    let f = &v["fixes"][0];
    assert_eq!(f["status"], "suggested", "{v}");
    let proposal = f["proposal"].as_str().unwrap();
    assert!(
        proposal.chars().count() < "Zahlungserinnerung senden".len(),
        "{proposal}"
    );
    assert_eq!(
        f["evidence"],
        "'Zahlungserinnerung senden' is truncated for 74 % of mobile DE visitors (360–390 px, 54 views in 7 days)"
    );
    let (_, fb) = call(Method::GET, "/api/projects/demo/feedback", Value::Null, ua).await;
    let g = fb["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["key"] == "invoice.remind")
        .expect("feedback group");
    assert_eq!(
        (g["status"].as_str(), g["proposal"].as_str()),
        (Some("proposed"), Some(proposal))
    );
    assert!(
        g["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Layout RUM: 'Zahlungserinnerung senden'")
    );
    // Same text is never proposed twice.
    let (_, v) = call(
        Method::POST,
        "/api/projects/demo/insights/layout/repair",
        Value::Null,
        ua,
    )
    .await;
    assert_eq!(v["created"], 0, "{v}");

    // ---- with the GitHub App link: one PR with the shortened catalog and the evidence ----
    let enc = |s: &str| glot_server::auth::encrypt(&state.secret, s);
    sqlx::query("INSERT INTO github_app (app_id, slug, client_id, client_secret_enc, private_key_enc, webhook_secret_enc, html_url) VALUES (42, 'glot-test', 'Iv1.test', $1, $2, $3, 'https://github.com/apps/glot-test')")
        .bind(enc("cs")).bind(enc(KEY)).bind(enc("whsec"))
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO github_links (project_id, installation_id, repo, branch) SELECT id, 7, 'acme/app', 'main' FROM projects WHERE slug = 'demo'")
        .execute(&pool).await.unwrap();
    let trial = json!({"k": "cta.trial", "o": 1, "b": 200, "n": 320, "f": "Inter", "fs": 16});
    for _ in 0..30 {
        call(
            Method::POST,
            &beacon,
            json!({"locale": "de", "vw": 1300, "items": [trial]}),
            ua,
        )
        .await;
    }
    let (_, v) = call(
        Method::POST,
        "/api/projects/demo/insights/layout/repair",
        Value::Null,
        ua,
    )
    .await;
    assert_eq!(v["created"], 1, "{v}");
    let f = v["fixes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "cta.trial")
        .unwrap()
        .clone();
    assert_eq!(
        (f["status"].as_str(), f["pr_url"].as_str()),
        (Some("pr"), Some("https://github.com/acme/app/pull/9")),
        "{f}"
    );
    let log = log.lock().unwrap().clone();
    let tree = &log
        .iter()
        .find(|(m, p, _)| m == "POST" && p.ends_with("/git/trees"))
        .unwrap()
        .2;
    assert_eq!(tree["tree"][0]["path"], "messages/de.json");
    let cat: Value = serde_json::from_str(tree["tree"][0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(cat["cta"]["trial"], f["proposal"]);
    assert_eq!(
        cat["invoice"]["remind"], "Zahlungserinnerung senden",
        "other keys untouched"
    );
    assert_eq!(
        tree["tree"].as_array().unwrap().len(),
        1,
        "no glot.lock in the repo: left alone"
    );
    let pr = &log
        .iter()
        .find(|(m, p, _)| m == "POST" && p.ends_with("/pulls"))
        .unwrap()
        .2;
    assert_eq!(pr["base"], "main");
    assert!(pr["body"].as_str().unwrap().contains(
        "'Kostenlose Testversion jetzt starten' is truncated for 100 % of DE visitors (1280–1440 px, 30 views in 7 days)"
    ), "{pr}");
}
