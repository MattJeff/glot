//! Feedback loop end to end: public reports → reverse index → threshold → mock proposal + back-translation →
//! accept → PR against a fake GitHub API. Skipped when Postgres is unreachable.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Method, Request, StatusCode, Uri, header},
    response::IntoResponse,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

type Log = Arc<Mutex<Vec<(String, String, String, Value)>>>;

/// Fake GitHub REST API: canned answers, records (method, path, authorization, body).
async fn fake_github(State(log): State<Log>, req: Request<Body>) -> axum::response::Response {
    let (method, uri): (Method, Uri) = (req.method().clone(), req.uri().clone());
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let bytes = req.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let path = uri.path().to_string();
    log.lock()
        .unwrap()
        .push((method.to_string(), path.clone(), auth, body));
    let r = |v: Value| axum::Json(v).into_response();
    match (method.as_str(), path.trim_start_matches("/repos/acme/app")) {
        ("GET", "/git/ref/heads/main") => r(json!({"object": {"sha": "base1"}})),
        ("GET", "/git/commits/base1") => r(json!({"tree": {"sha": "tree1"}})),
        ("GET", "/contents/messages/fr.json") => {
            r(json!({"home": {"greet": "Salut {name} !", "title": "Bon retour"}}))
        }
        ("GET", "/contents/glot.lock") => StatusCode::NOT_FOUND.into_response(),
        ("POST", "/git/trees") => r(json!({"sha": "tree2"})),
        ("POST", "/git/commits") => r(json!({"sha": "commit2"})),
        ("POST", "/git/refs") => r(json!({"ref": "x"})),
        ("POST", "/pulls") => r(json!({"html_url": "https://github.com/acme/app/pull/7"})),
        _ => StatusCode::IM_A_TEAPOT.into_response(),
    }
}

#[tokio::test]
async fn feedback_loop() {
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
    // SAFETY: set before any server code reads them; this test binary has a single test.
    unsafe {
        std::env::set_var("GLOT_GITHUB_API", &gh_url);
        std::env::set_var("GLOT_FEEDBACK_RATE_PER_MIN", "8");
    }

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
    let (s, _) = call(
        Method::POST,
        "/api/auth/signup",
        json!({"email": "f@example.com", "password": "correct horse", "name": "Fay"}),
    )
    .await;
    assert!(s.is_success());
    let (s, _) = call(
        Method::POST,
        "/api/projects",
        json!({"name": "F", "slug": "fb", "source_locale": "en", "target_locales": ["fr"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/fb",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
    )
    .await;
    for (k, src, fr) in [
        ("home.greet", "Hello {name}!", "Salut {name} !"),
        ("home.title", "Welcome back", "Bon retour"),
    ] {
        call(
            Method::POST,
            "/api/projects/fb/keys",
            json!({"key": k, "source": src}),
        )
        .await;
        let (s, v) = call(
            Method::PUT,
            &format!("/api/projects/fb/keys/{k}/translations/fr"),
            json!({"value": fr}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    let (s, v) = call(
        Method::PUT,
        "/api/projects/fb/feedback/settings",
        json!({"threshold": 2}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pk = v["public_key"].as_str().unwrap().to_string();
    assert_eq!(v["has_github_token"], false);
    let ingest = format!("/api/feedback/{pk}");

    // 1st report: matched through the placeholder ("Ana" fills {name}); url loses its query string.
    let report = json!({"locale": "fr", "text": "Salut  Ana !", "kind": "awkward", "suggestion": "Bonjour Ana !",
        "url": "https://app.example/home?email=a@b.c#x", "dom": "main>h1"});
    let (s, v) = call(Method::POST, &ingest, report.clone()).await;
    assert_eq!(
        (s, &v["matched"]),
        (StatusCode::ACCEPTED, &json!(true)),
        "{v}"
    );
    let (_, v) = call(
        Method::POST,
        &ingest,
        json!({"locale": "fr", "text": "Texte dynamique inconnu"}),
    )
    .await;
    assert_eq!(v["matched"], false);
    let (s, _) = call(
        Method::POST,
        &ingest,
        json!({"locale": "fr", "text": "x", "kind": "spam"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(Method::POST, "/api/feedback/pk_nope", report.clone()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (_, v) = call(Method::GET, "/api/projects/fb/feedback", Value::Null).await;
    let g = &v["groups"][0];
    assert_eq!(
        (g["key"].as_str(), g["status"].as_str(), g["count"].as_i64()),
        (Some("home.greet"), Some("open"), Some(1))
    );
    assert_eq!(g["reports"][0]["url"], "https://app.example/home");
    let id = g["id"].as_str().unwrap().to_string();

    // 2nd report reaches the threshold → background proposal (mock: "⟦fr⟧ source", back-translation = source).
    call(Method::POST, &ingest, report.clone()).await;
    let mut g = Value::Null;
    for _ in 0..50 {
        let (_, v) = call(Method::GET, "/api/projects/fb/feedback", Value::Null).await;
        g = v["groups"][0].clone();
        if g["status"] == "proposed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(g["status"], "proposed", "{g}");
    assert_eq!(g["proposal"], "⟦fr⟧ Hello {name}!");
    assert_eq!(g["previous"], "Salut {name} !");
    assert_eq!(
        (g["back_translation"].as_str(), g["verdict"].as_str()),
        (Some("Hello {name}!"), Some("same"))
    );

    // Accept: the validator refuses a dropped placeholder, then the proposal is written as human/reviewed.
    let (s, v) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{id}/accept"),
        json!({"value": "Bonjour !"}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, v) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{id}/accept"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (s, v["status"].as_str()),
        (StatusCode::OK, Some("accepted")),
        "{v}"
    );
    let (_, v) = call(
        Method::GET,
        "/api/projects/fb/keys?q=home.greet",
        Value::Null,
    )
    .await;
    let t = &v["items"][0]["translations"]["fr"];
    assert_eq!(
        (
            t["value"].as_str(),
            t["status"].as_str(),
            t["origin"].as_str()
        ),
        (Some("⟦fr⟧ Hello {name}!"), Some("reviewed"), Some("human"))
    );
    // A new report on the same key opens a fresh group (the accepted one is history).
    let (s, _) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{id}/reject"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // On-demand proposal + reject on another key.
    call(
        Method::POST,
        &ingest,
        json!({"locale": "fr", "text": "Bon retour", "kind": "unclear"}),
    )
    .await;
    let (_, v) = call(Method::GET, "/api/projects/fb/feedback", Value::Null).await;
    let other = v["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["key"] == "home.title")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, v) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{other}/propose"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (s, v["status"].as_str()),
        (StatusCode::OK, Some("proposed")),
        "{v}"
    );
    let (_, v) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{other}/reject"),
        Value::Null,
    )
    .await;
    assert_eq!(v["status"], "rejected");

    // PR: refused without settings, then one commit (catalog + glot.lock) and a PR on the fake GitHub.
    let (s, _) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{id}/pr"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = call(
        Method::PUT,
        "/api/projects/fb/feedback/settings",
        json!({"github_repo": "bad"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (_, v) = call(
        Method::PUT,
        "/api/projects/fb/feedback/settings",
        json!({"github_repo": "acme/app", "github_token": "ghp_secret"}),
    )
    .await;
    assert_eq!(v["has_github_token"], true);
    assert!(!v.to_string().contains("ghp_secret"), "token is write-only");
    let (s, v) = call(
        Method::POST,
        &format!("/api/projects/fb/feedback/{id}/pr"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["pr_url"], "https://github.com/acme/app/pull/7");
    let log = log.lock().unwrap().clone();
    assert!(
        log.iter()
            .all(|(_, _, auth, _)| auth == "Bearer ghp_secret")
    );
    let tree = &log.iter().find(|l| l.1.ends_with("/git/trees")).unwrap().3;
    assert_eq!(tree["base_tree"], "tree1");
    let files = tree["tree"].as_array().unwrap();
    let cat: Value = serde_json::from_str(files[0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(files[0]["path"], "messages/fr.json");
    assert_eq!(
        cat,
        json!({"home": {"greet": "⟦fr⟧ Hello {name}!", "title": "Bon retour"}})
    );
    let lock: Value = serde_json::from_str(files[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        lock["entries"]["fr"]["home.greet"]["origin"]["kind"],
        "human"
    );
    assert_eq!(lock["entries"]["fr"]["home.greet"]["reviewed"], true);
    let commit = &log
        .iter()
        .find(|l| l.1.ends_with("/git/commits"))
        .unwrap()
        .3;
    assert_eq!(commit["parents"], json!(["base1"]));
    let refs = &log.iter().find(|l| l.1.ends_with("/git/refs")).unwrap().3;
    assert_eq!(refs["sha"], "commit2");
    let pr = &log.iter().find(|l| l.1.ends_with("/pulls")).unwrap().3;
    assert_eq!(pr["base"], "main");
    let body = pr["body"].as_str().unwrap();
    assert!(
        body.contains("| `home.greet` | fr | Salut {name} ! | ⟦fr⟧ Hello {name}! |"),
        "{body}"
    );

    // Shared rate limit (8/min here): the flood ends in 429 + Retry-After.
    let mut limited = false;
    for _ in 0..10 {
        let (s, _) = call(
            Method::POST,
            &ingest,
            json!({"locale": "fr", "text": "Bon retour"}),
        )
        .await;
        limited |= s == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
