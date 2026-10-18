//! Transactional email against a fake Resend API: payloads, retries, idempotency, rate limit,
//! verification + full password-reset flow, job notification. Skipped when Postgres is unreachable.
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

#[derive(Default)]
struct Fake {
    /// (Idempotency-Key, Authorization, body) per request.
    seen: Vec<(String, String, Value)>,
    /// Status codes to answer next (then 200).
    script: Vec<u16>,
}
type Shared = Arc<Mutex<Fake>>;

async fn fake_resend(
    State(f): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut f = f.lock().unwrap();
    let h = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    f.seen
        .push((h("idempotency-key"), h("authorization"), body));
    let code = if f.script.is_empty() {
        200
    } else {
        f.script.remove(0)
    };
    let n = f.seen.len();
    (
        StatusCode::from_u16(code).unwrap(),
        Json(json!({"id": format!("re_{n}"), "message": "scripted"})),
    )
}

struct Ctx {
    app: Router,
    state: glot_server::AppState,
    fake: Shared,
    admin: PgPool,
    pool: PgPool,
    db_name: String,
}

async fn setup() -> Option<Ctx> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let admin = match PgPool::connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SKIPPED: postgres unreachable at {url} ({e})");
            return None;
        }
    };
    let db_name = format!("glot_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&admin)
        .await
        .unwrap();
    let opts: PgConnectOptions = url.parse::<PgConnectOptions>().unwrap().database(&db_name);
    let pool = PgPool::connect_with(opts).await.unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();

    let fake: Shared = Default::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let app = Router::new()
        .route("/emails", axum::routing::post(fake_resend))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let mut state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let mut m = glot_server::email::Mailer::new(
        "re_test_key",
        &base,
        "glot <noreply@mail.example.com>",
        "http://localhost:3100",
    );
    m.reply_to = Some("support@example.com".into());
    m.retry_base = Duration::ZERO;
    m.rate_per_hour = 6;
    state.mailer = Some(m);
    glot_server::jobs::spawn_worker(state.clone());
    glot_server::email::spawn_worker(state.clone());
    Some(Ctx {
        app: glot_server::router(state.clone(), "http://localhost:3100"),
        state,
        fake,
        admin,
        pool,
        db_name,
    })
}

impl Ctx {
    async fn finish(self) {
        self.pool.close().await;
        let _ = sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.db_name))
            .execute(&self.admin)
            .await;
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        let body = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let res = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, json)
    }

    fn cookie(h: &HeaderMap) -> String {
        h.get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("glot_session=")
            .to_string()
    }

    /// Waits until the fake has received `n` requests.
    async fn wait_requests(&self, n: usize) -> Vec<(String, String, Value)> {
        for _ in 0..100 {
            let seen = self.fake.lock().unwrap().seen.clone();
            if seen.len() >= n {
                return seen;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "fake Resend got {} requests, expected {n}",
            self.fake.lock().unwrap().seen.len()
        );
    }

    async fn outbox(&self, template: &str) -> Vec<(String, i32, bool)> {
        sqlx::query_as(
            "SELECT status, attempts, html IS NULL FROM email_outbox WHERE template = $1 ORDER BY created_at",
        )
        .bind(template)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }
}

fn token_of(body: &Value) -> String {
    let text = body["text"].as_str().unwrap();
    let i = text.find("token=").expect("link with token") + 6;
    text[i..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect()
}

#[tokio::test]
async fn verification_reset_retries_idempotency() {
    let Some(ctx) = setup().await else { return };

    // Signup in French => verification email queued and delivered with the full payload.
    let (s, h, _) = ctx
        .call(
            Method::POST,
            "/api/auth/signup",
            None,
            Some(json!({"email": "ada@example.com", "password": "correct horse", "name": "Ada <3", "locale": "fr"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let cookie = Ctx::cookie(&h);
    let seen = ctx.wait_requests(1).await;
    let (idem, auth, body) = &seen[0];
    assert_eq!(auth, "Bearer re_test_key");
    assert!(
        uuid::Uuid::parse_str(idem).is_ok(),
        "Idempotency-Key = outbox id"
    );
    assert_eq!(body["from"], "glot <noreply@mail.example.com>");
    assert_eq!(body["to"], json!(["ada@example.com"]));
    assert_eq!(body["reply_to"], "support@example.com");
    assert_eq!(body["subject"], "Confirmez votre adresse e-mail"); // fr catalog
    let html = body["html"].as_str().unwrap();
    assert!(html.contains("lang=\"fr\"") && html.contains("Ada &lt;3") && !html.contains("Ada <3"));
    assert!(
        body["text"]
            .as_str()
            .unwrap()
            .contains("http://localhost:3100/fr/verify-email?token=")
    );
    let verify_token = token_of(body);

    let (_, _, me) = ctx.call(Method::GET, "/api/me", Some(&cookie), None).await;
    assert_eq!(me["email_verified"], false);
    assert_eq!(me["email_enabled"], true);
    assert_eq!(me["locale"], "fr");

    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/verify-email",
            None,
            Some(json!({"token": verify_token})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, me) = ctx.call(Method::GET, "/api/me", Some(&cookie), None).await;
    assert_eq!(me["email_verified"], true);
    let (s, _, e) = ctx
        .call(
            Method::POST,
            "/api/auth/verify-email",
            None,
            Some(json!({"token": verify_token})),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_token"))
    );
    // Sent rows have their bodies (single-use links) cleared.
    assert_eq!(ctx.outbox("verify").await, vec![("sent".into(), 1, true)]);

    // Forgot password: unknown address => 204 and nothing sent.
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/forgot-password",
            None,
            Some(json!({"email": "nobody@example.com"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Retries: two 5xx/429 then success, same Idempotency-Key every time.
    ctx.fake.lock().unwrap().script = vec![500, 429];
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/forgot-password",
            None,
            Some(json!({"email": "ADA@example.com"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let seen = ctx.wait_requests(4).await;
    assert_eq!(seen.len(), 4, "unknown address must not send");
    assert!(
        seen[1..4].iter().all(|r| r.0 == seen[1].0),
        "same Idempotency-Key on retries"
    );
    assert_ne!(seen[1].0, seen[0].0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(ctx.outbox("reset").await, vec![("sent".into(), 3, true)]);
    let reset_token = token_of(&seen[3].2);
    assert!(
        seen[3].2["text"]
            .as_str()
            .unwrap()
            .contains("/fr/reset-password?token=")
    );

    // Reset: too short, then OK => sessions revoked, old password dead, token single-use.
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/reset-password",
            None,
            Some(json!({"token": reset_token, "password": "short"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/reset-password",
            None,
            Some(json!({"token": reset_token, "password": "new battery staple"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = ctx.call(Method::GET, "/api/me", Some(&cookie), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "reset signs out every session");
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "ada@example.com", "password": "correct horse"})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, h, _) = ctx
        .call(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "ada@example.com", "password": "new battery staple"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let cookie = Ctx::cookie(&h);
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/reset-password",
            None,
            Some(json!({"token": reset_token, "password": "another one!"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "single use");
    let seen = ctx.wait_requests(5).await;
    assert_eq!(
        seen[4].2["subject"],
        "Votre mot de passe glot a été modifié"
    );

    // Expired token is refused.
    sqlx::query(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, email, expires_at)
                 SELECT $1, id, 'reset', email, now() - interval '1 minute' FROM users",
    )
    .bind(glot_server::auth::sha256_hex("expired"))
    .execute(&ctx.pool)
    .await
    .unwrap();
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/reset-password",
            None,
            Some(json!({"token": "expired", "password": "new battery staple"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Permanent 4xx: no retry.
    ctx.fake.lock().unwrap().script = vec![422];
    let (s, _, _) = ctx
        .call(
            Method::PUT,
            "/api/me/password",
            Some(&cookie),
            Some(json!({"current_password": "new battery staple", "password": "third password"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    ctx.wait_requests(6).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rows = ctx.outbox("password_changed").await;
    assert_eq!(rows.last().unwrap(), &("failed".into(), 1, true));
    assert_eq!(ctx.fake.lock().unwrap().seen.len(), 6);
    let (s, _, _) = ctx.call(Method::GET, "/api/me", Some(&cookie), None).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "changing the password keeps the current session"
    );

    // Idempotency of enqueues: same key twice => one row.
    let to = || glot_server::email::To {
        email: "ada@example.com",
        name: "Ada",
        locale: "en",
    };
    assert!(
        glot_server::email::send(&ctx.state, to(), "verify", &[], "http://x", "same-key").await
    );
    assert!(
        !glot_server::email::send(&ctx.state, to(), "verify", &[], "http://x", "same-key").await
    );

    // Rate limit: 6 per recipient per hour. 5 rows so far for ada (verify, reset, 2x password_changed, same-key).
    let count = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM email_outbox WHERE to_email = 'ada@example.com'",
        )
        .fetch_one(&ctx.pool)
        .await
        .unwrap()
    };
    assert_eq!(count().await, 5);
    assert!(glot_server::email::send(&ctx.state, to(), "verify", &[], "http://x", "rl-1").await);
    assert!(!glot_server::email::send(&ctx.state, to(), "verify", &[], "http://x", "rl-2").await);
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/forgot-password",
            None,
            Some(json!({"email": "ada@example.com"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (n, n2) = (6, count().await);
    assert_eq!(n, n2, "over budget: not queued");

    ctx.finish().await;
}

#[tokio::test]
async fn email_change_and_job_notification() {
    let Some(ctx) = setup().await else { return };
    let (_, h, _) = ctx
        .call(
            Method::POST,
            "/api/auth/signup",
            None,
            Some(json!({"email": "old@example.com", "password": "correct horse", "name": "Bob"})),
        )
        .await;
    let a = Ctx::cookie(&h);
    ctx.wait_requests(1).await;

    // Email change: wrong password, then confirmation sent to the new address; old one notified on confirm.
    let (s, _, _) = ctx
        .call(
            Method::PUT,
            "/api/me/email",
            Some(&a),
            Some(json!({"email": "new@example.com", "password": "nope nope"})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, r) = ctx
        .call(
            Method::PUT,
            "/api/me/email",
            Some(&a),
            Some(json!({"email": "new@example.com", "password": "correct horse"})),
        )
        .await;
    assert_eq!((s, r["pending"].as_bool()), (StatusCode::OK, Some(true)));
    let seen = ctx.wait_requests(2).await;
    assert_eq!(seen[1].2["to"], json!(["new@example.com"]));
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/verify-email",
            None,
            Some(json!({"token": token_of(&seen[1].2)})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, me) = ctx.call(Method::GET, "/api/me", Some(&a), None).await;
    assert_eq!(
        (me["email"].as_str(), me["email_verified"].as_bool()),
        (Some("new@example.com"), Some(true))
    );
    let seen = ctx.wait_requests(3).await;
    assert_eq!(seen[2].2["to"], json!(["old@example.com"]));
    assert!(
        seen[2].2["text"]
            .as_str()
            .unwrap()
            .contains("new@example.com")
    );

    // Job notification is opt-in.
    let (s, _, me) = ctx
        .call(
            Method::PATCH,
            "/api/me",
            Some(&a),
            Some(json!({"notify_job_done": true})),
        )
        .await;
    assert_eq!(
        (s, me["notify_job_done"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    ctx.call(
        Method::POST,
        "/api/projects",
        Some(&a),
        Some(json!({"name": "Demo", "slug": "demo", "source_locale": "en", "target_locales": ["fr"]})),
    )
    .await;
    ctx.call(
        Method::PATCH,
        "/api/projects/demo",
        Some(&a),
        Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    ctx.call(
        Method::POST,
        "/api/projects/demo/keys",
        Some(&a),
        Some(json!({"key": "home.title", "source": "Welcome"})),
    )
    .await;
    let (s, _, job) = ctx
        .call(
            Method::POST,
            "/api/projects/demo/jobs",
            Some(&a),
            Some(json!({"kind": "translate"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{job}");
    let seen = ctx.wait_requests(4).await;
    let b = &seen[3].2;
    assert_eq!(b["subject"], "Translation finished in Demo");
    assert!(b["text"].as_str().unwrap().contains("1 of 1 strings"));
    assert!(
        b["text"]
            .as_str()
            .unwrap()
            .contains("http://localhost:3100/app/demo/jobs")
    );

    ctx.finish().await;
}

#[tokio::test]
async fn disabled_without_api_key() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        return;
    };
    let db_name = format!("glot_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db_name))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let state = glot_server::AppState::new(pool.clone(), "s", false);
    assert!(glot_server::email::spawn_worker(state.clone()).is_none());
    let app = glot_server::router(state, "http://localhost:3100");
    let req = |m: Method, uri: &str, b: Value, c: Option<&str>| {
        let mut r = Request::builder()
            .method(m)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = c {
            r = r.header(header::COOKIE, c.to_string());
        }
        r.body(Body::from(b.to_string())).unwrap()
    };
    let res = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/auth/signup",
            json!({"email": "d@example.com", "password": "correct horse", "name": "D"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let c = res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let res = app
        .clone()
        .oneshot(req(Method::GET, "/api/me", Value::Null, Some(&c)))
        .await
        .unwrap();
    let me: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(me["email_enabled"], false);
    let res = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/auth/forgot-password",
            json!({"email": "d@example.com"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db_name} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
