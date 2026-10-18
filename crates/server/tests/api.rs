//! Integration tests against a real Postgres (DATABASE_URL, default: docker compose `postgres` on :5433).
//! Each test gets its own fresh database. Skipped (with a message) when Postgres is unreachable.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::time::Duration;
use tower::ServiceExt;

struct Ctx {
    app: Router,
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
            eprintln!(
                "SKIPPED: postgres unreachable at {url} ({e}); run `docker compose up -d postgres`"
            );
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
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    glot_server::jobs::spawn_worker(state.clone());
    Some(Ctx {
        app: glot_server::router(state, "http://localhost:3100"),
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

    /// `auth` is either a session cookie value or a `glot_` token.
    async fn call(
        &self,
        method: Method,
        uri: &str,
        auth: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        match auth {
            Some(t) if t.starts_with("glot_") => {
                req = req.header(header::AUTHORIZATION, format!("Bearer {t}"))
            }
            Some(c) => req = req.header(header::COOKIE, format!("glot_session={c}")),
            None => {}
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
        let json = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, headers, json)
    }

    async fn signup(&self, email: &str) -> String {
        let (s, h, _) = self
            .call(
                Method::POST,
                "/api/auth/signup",
                None,
                Some(json!({"email": email, "password": "correct horse", "name": "Ada"})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
        let cookie = h.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
        cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("glot_session=")
            .to_string()
    }

    async fn project(&self, auth: &str, slug: &str) {
        let (s, _, p) = self
            .call(
                Method::POST,
                "/api/projects",
                Some(auth),
                Some(json!({"name": "Demo", "slug": slug, "source_locale": "en", "target_locales": ["fr", "de"]})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{p}");
    }
}

#[tokio::test]
async fn auth_signup_login_me() {
    let Some(ctx) = setup().await else { return };
    let cookie = ctx.signup("ada@example.com").await;
    let (s, _, me) = ctx.call(Method::GET, "/api/me", Some(&cookie), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["email"], "ada@example.com");

    let (s, _, e) = ctx.call(Method::GET, "/api/me", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(e["error"]["code"], "unauthorized");

    let (s, _, e) = ctx
        .call(
            Method::POST,
            "/api/auth/signup",
            None,
            Some(json!({"email": "ADA@example.com", "password": "correct horse", "name": "X"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/auth/signup",
            None,
            Some(json!({"email": "nope", "password": "correct horse", "name": "X"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, _, e) = ctx
        .call(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "ada@example.com", "password": "wrong pass"})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(e["error"]["code"], "invalid_credentials");
    let (s, h, u) = ctx
        .call(
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": "ada@example.com", "password": "correct horse"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(u["name"], "Ada");
    assert!(h.get(header::SET_COOKIE).is_some());

    let (s, _, _) = ctx
        .call(Method::POST, "/api/auth/logout", Some(&cookie), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = ctx.call(Method::GET, "/api/me", Some(&cookie), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    ctx.finish().await;
}

#[tokio::test]
async fn bearer_tokens() {
    let Some(ctx) = setup().await else { return };
    let cookie = ctx.signup("tok@example.com").await;
    let (s, _, created) = ctx
        .call(
            Method::POST,
            "/api/tokens",
            Some(&cookie),
            Some(json!({"name": "cli"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let token = created["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("glot_"));
    let (s, _, me) = ctx.call(Method::GET, "/api/me", Some(&token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["email"], "tok@example.com");
    let (_, _, list) = ctx
        .call(Method::GET, "/api/tokens", Some(&cookie), None)
        .await;
    assert!(list[0]["last_used_at"].is_string());
    assert!(list[0].get("token").is_none());
    let id = created["api_token"]["id"].as_str().unwrap();
    let (s, _, _) = ctx
        .call(
            Method::DELETE,
            &format!("/api/tokens/{id}"),
            Some(&cookie),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = ctx.call(Method::GET, "/api/me", Some(&token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    ctx.finish().await;
}

#[tokio::test]
async fn projects_keys_translations() {
    let Some(ctx) = setup().await else { return };
    let a = ctx.signup("owner@example.com").await;
    ctx.project(&a, "demo").await;
    let (s, _, _) = ctx
        .call(Method::POST, "/api/projects", Some(&a), Some(json!({"name": "X", "slug": "Bad Slug", "source_locale": "en", "target_locales": []})))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, e) = ctx
        .call(
            Method::POST,
            "/api/projects",
            Some(&a),
            Some(json!({"name": "X", "slug": "demo", "source_locale": "en", "target_locales": []})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");

    let (s, _, row) = ctx
        .call(Method::POST, "/api/projects/demo/keys", Some(&a), Some(json!({"key": "greet.hello", "source": "Hello {name}", "description": "Header greeting"})))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{row}");
    assert_eq!(row["translations"]["fr"]["status"], "missing");

    // Missing placeholder: stored, but flagged.
    let (s, _, t) = ctx
        .call(
            Method::PUT,
            "/api/projects/demo/keys/greet.hello/translations/fr",
            Some(&a),
            Some(json!({"value": "Bonjour"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["status"], "reviewed");
    assert_eq!(t["origin"], "human");
    assert!(
        t["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "placeholder_missing"),
        "{t}"
    );

    let (_, _, issues) = ctx
        .call(Method::GET, "/api/projects/demo/issues", Some(&a), None)
        .await;
    assert_eq!(issues[0]["key"], "greet.hello");
    let (_, _, p) = ctx
        .call(Method::GET, "/api/projects/demo", Some(&a), None)
        .await;
    assert_eq!(p["stats"]["keys"], 1);
    assert_eq!(p["stats"]["locales"]["fr"]["reviewed"], 1);
    assert_eq!(p["stats"]["locales"]["fr"]["issues"], 1);
    assert_eq!(p["llm"]["has_api_key"], false);

    // Source change => stale.
    let (s, _, _) = ctx
        .call(
            Method::PATCH,
            "/api/projects/demo/keys/greet.hello",
            Some(&a),
            Some(json!({"source": "Hi {name}"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, page) = ctx
        .call(
            Method::GET,
            "/api/projects/demo/keys?locale=fr&status=stale",
            Some(&a),
            None,
        )
        .await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["translations"]["fr"]["status"], "stale");
    let (_, _, page) = ctx
        .call(
            Method::GET,
            "/api/projects/demo/keys?q=nothing",
            Some(&a),
            None,
        )
        .await;
    assert_eq!(page["total"], 0);

    let (s, _, t) = ctx
        .call(
            Method::POST,
            "/api/projects/demo/keys/greet.hello/translations/fr/review",
            Some(&a),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t["status"], "reviewed");
    let (_, _, h) = ctx
        .call(
            Method::GET,
            "/api/projects/demo/keys/greet.hello/history",
            Some(&a),
            None,
        )
        .await;
    assert_eq!(h[0]["author"], "Ada");

    // Glossary CRUD.
    let (s, _, term) = ctx
        .call(
            Method::POST,
            "/api/projects/demo/glossary",
            Some(&a),
            Some(json!({"term": "Workspace", "translations": {"fr": "Espace de travail"}})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{term}");
    let id = term["id"].as_str().unwrap();
    let (s, _, term) = ctx
        .call(
            Method::PATCH,
            &format!("/api/projects/demo/glossary/{id}"),
            Some(&a),
            Some(json!({"do_not_translate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(term["do_not_translate"], true);

    // Settings: the API key is write-only.
    let (s, _, p) = ctx
        .call(
            Method::PATCH,
            "/api/projects/demo",
            Some(&a),
            Some(json!({"llm": {"provider": "openai", "model": "gpt-x", "api_key": "sk-test"}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["llm"]["has_api_key"], true);
    assert!(!p.to_string().contains("sk-test"));

    // Access control: another user sees nothing.
    let b = ctx.signup("other@example.com").await;
    let (s, _, e) = ctx
        .call(Method::GET, "/api/projects/demo", Some(&b), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(e["error"]["code"], "not_found");
    let (_, _, list) = ctx.call(Method::GET, "/api/projects", Some(&b), None).await;
    assert_eq!(list, json!([]));

    let (s, _, _) = ctx
        .call(
            Method::DELETE,
            "/api/projects/demo/keys/greet.hello",
            Some(&a),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = ctx
        .call(Method::DELETE, "/api/projects/demo", Some(&a), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    ctx.finish().await;
}

#[tokio::test]
async fn translate_job_with_mock_provider() {
    let Some(ctx) = setup().await else { return };
    let a = ctx.signup("jobs@example.com").await;
    ctx.project(&a, "jobs").await;
    let (s, _, _) = ctx
        .call(
            Method::PATCH,
            "/api/projects/jobs",
            Some(&a),
            Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    for (k, v) in [
        ("home.title", "Welcome"),
        ("home.cta", "Save {count} files"),
    ] {
        ctx.call(
            Method::POST,
            "/api/projects/jobs/keys",
            Some(&a),
            Some(json!({"key": k, "source": v})),
        )
        .await;
    }
    let (s, _, job) = ctx
        .call(
            Method::POST,
            "/api/projects/jobs/jobs",
            Some(&a),
            Some(json!({"kind": "translate"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{job}");
    let id = job["id"].as_str().unwrap().to_string();

    let mut job = job;
    for _ in 0..300 {
        (_, _, job) = ctx
            .call(Method::GET, &format!("/api/jobs/{id}"), Some(&a), None)
            .await;
        if job["status"] == "done" || job["status"] == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["progress"]["total"], 4);
    assert_eq!(job["progress"]["done"], 4);
    assert!(job["finished_at"].is_string());

    let (_, _, page) = ctx
        .call(Method::GET, "/api/projects/jobs/keys", Some(&a), None)
        .await;
    let machine = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["translations"].as_object().unwrap().values())
        .filter(|t| t["status"] == "machine" && t["origin"] == "machine")
        .count();
    assert!(machine > 0, "{page}");

    // SSE on a finished job: one progress event then done.
    let (s, h, body) = ctx
        .call(
            Method::GET,
            &format!("/api/jobs/{id}/events"),
            Some(&a),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        h[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let body = body.as_str().unwrap();
    assert!(
        body.contains("event: progress") && body.contains("event: done"),
        "{body}"
    );

    let (_, _, jobs) = ctx
        .call(Method::GET, "/api/projects/jobs/jobs", Some(&a), None)
        .await;
    assert_eq!(jobs.as_array().unwrap().len(), 1);
    ctx.finish().await;
}

#[tokio::test]
async fn cdn_catalog() {
    let Some(ctx) = setup().await else { return };
    let a = ctx.signup("cdn@example.com").await;
    ctx.project(&a, "cdn").await;
    ctx.call(
        Method::POST,
        "/api/projects/cdn/keys",
        Some(&a),
        Some(json!({"key": "auth.login.cta", "source": "Log in"})),
    )
    .await;
    ctx.call(
        Method::PUT,
        "/api/projects/cdn/keys/auth.login.cta/translations/fr",
        Some(&a),
        Some(json!({"value": "Se connecter"})),
    )
    .await;

    let (s, _, _) = ctx.call(Method::GET, "/cdn/cdn/fr.json", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, body) = ctx
        .call(Method::GET, "/cdn/cdn/fr.json", Some(&a), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    ctx.call(
        Method::PATCH,
        "/api/projects/cdn",
        Some(&a),
        Some(json!({"public_cdn": true})),
    )
    .await;
    let (s, h, body) = ctx.call(Method::GET, "/cdn/cdn/fr.json", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, json!({"auth": {"login": {"cta": "Se connecter"}}}));
    assert!(
        h[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .starts_with("public")
    );
    let etag = h[header::ETAG].to_str().unwrap().to_string();

    let req = Request::get("/cdn/cdn/fr.json")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let res = ctx.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);

    let (s, _, _) = ctx.call(Method::GET, "/cdn/cdn/xx.json", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, _, en) = ctx.call(Method::GET, "/cdn/cdn/en.json", None, None).await;
    assert_eq!(en["auth"]["login"]["cta"], "Log in");
    ctx.finish().await;
}

#[tokio::test]
async fn sync_push_pull_with_conflicts() {
    let Some(ctx) = setup().await else { return };
    let cookie = ctx.signup("sync@example.com").await;
    let (_, _, created) = ctx
        .call(
            Method::POST,
            "/api/tokens",
            Some(&cookie),
            Some(json!({"name": "ci"})),
        )
        .await;
    let t = created["token"].as_str().unwrap().to_string();
    ctx.project(&t, "sync").await;

    let push = json!({
        "catalogs": {
            "en": {"a.hello": "Hello", "a.bye": "Bye"},
            "fr": {"a.hello": "Bonjour", "a.bye": "Au revoir"}
        },
        "lock": {"version": 1, "notes": {"a.hello": "Greeting"}, "entries": {"fr": {
            "a.hello": {"source_hash": "x", "target_hash": "y", "origin": {"kind": "machine", "model": "m1"}, "score": 90, "reviewed": false}
        }}},
        "glossary": [{"term": "glot", "do_not_translate": true}]
    });
    let (s, _, r) = ctx
        .call(
            Method::POST,
            "/api/projects/sync/sync/push",
            Some(&t),
            Some(push),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["created"], 4);
    assert_eq!(r["conflicts"], json!([]));

    // A human edit on the server wins over a different local value.
    ctx.call(
        Method::PUT,
        "/api/projects/sync/keys/a.bye/translations/fr",
        Some(&t),
        Some(json!({"value": "Salut"})),
    )
    .await;
    let push = json!({"catalogs": {"fr": {"a.bye": "À plus", "a.hello": "Coucou"}}});
    let (s, _, r) = ctx
        .call(
            Method::POST,
            "/api/projects/sync/sync/push",
            Some(&t),
            Some(push),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["updated"], 1);
    assert_eq!(
        r["conflicts"],
        json!([{"locale": "fr", "key": "a.bye", "server": "Salut", "local": "À plus"}])
    );

    let (s, _, pull) = ctx
        .call(Method::GET, "/api/projects/sync/sync/pull", Some(&t), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(pull["catalogs"]["en"]["a.hello"], "Hello");
    assert_eq!(pull["catalogs"]["fr"]["a.bye"], "Salut");
    assert_eq!(pull["catalogs"]["fr"]["a.hello"], "Coucou");
    assert_eq!(pull["lock"]["notes"]["a.hello"], "Greeting");
    assert_eq!(
        pull["lock"]["entries"]["fr"]["a.bye"]["origin"]["kind"],
        "human"
    );
    assert_eq!(pull["lock"]["entries"]["fr"]["a.bye"]["reviewed"], true);
    assert_eq!(pull["glossary"][0]["term"], "glot");

    let (s, _, _) = ctx
        .call(
            Method::POST,
            "/api/projects/sync/sync/push",
            Some(&t),
            Some(json!({"catalogs": {"zz": {}}})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    ctx.finish().await;
}

#[tokio::test]
async fn sync_push_code_context() {
    let Some(ctx) = setup().await else { return };
    let a = ctx.signup("ctx@example.com").await;
    ctx.project(&a, "ctx").await;
    let push = |context: Value| json!({"catalogs": {"en": {"form.save": "Save", "form.old": "Old"}}, "context": context});
    let (s, _, r) = ctx
        .call(
            Method::POST,
            "/api/projects/ctx/sync/push",
            Some(&a),
            Some(push(json!({"form.save": {
                "refs": [{"path": "app/page.tsx", "line": 12}],
                "ui_role": "button", "component": "SaveButton", "description": "Saves the form"
            }}))),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (_, _, page) = ctx
        .call(Method::GET, "/api/projects/ctx/keys?q=save", Some(&a), None)
        .await;
    let row = &page["items"][0];
    assert_eq!(row["refs"], json!([{"path": "app/page.tsx", "line": 12}]));
    assert_eq!(row["ui_role"], "button");
    assert_eq!(row["component"], "SaveButton");
    assert_eq!(row["description"], "Saves the form");

    // Dashboard edits the description; a push without a local note keeps it, refs move.
    ctx.call(
        Method::PATCH,
        "/api/projects/ctx/keys/form.save",
        Some(&a),
        Some(json!({"description": "Edited on server"})),
    )
    .await;
    ctx.call(
        Method::POST,
        "/api/projects/ctx/sync/push",
        Some(&a),
        Some(push(json!({"form.save": {"refs": [{"path": "app/page.tsx", "line": 20}], "ui_role": "button"}}))),
    )
    .await;
    let (_, _, page) = ctx
        .call(Method::GET, "/api/projects/ctx/keys?q=save", Some(&a), None)
        .await;
    let row = &page["items"][0];
    assert_eq!(row["description"], "Edited on server");
    assert_eq!(row["refs"][0]["line"], 20);
    assert_eq!(row["component"], Value::Null);

    // Old clients (no `context`) don't wipe stored context; pull returns the server note.
    ctx.call(
        Method::POST,
        "/api/projects/ctx/sync/push",
        Some(&a),
        Some(json!({"catalogs": {"en": {"form.save": "Save"}}})),
    )
    .await;
    let (_, _, page) = ctx
        .call(Method::GET, "/api/projects/ctx/keys?q=save", Some(&a), None)
        .await;
    assert_eq!(page["items"][0]["ui_role"], "button");
    let (_, _, pull) = ctx
        .call(Method::GET, "/api/projects/ctx/sync/pull", Some(&a), None)
        .await;
    assert_eq!(pull["lock"]["notes"]["form.save"], "Edited on server");
    ctx.finish().await;
}

#[tokio::test]
async fn openapi_and_health() {
    let Some(ctx) = setup().await else { return };
    let (s, _, body) = ctx.call(Method::GET, "/healthz", None, None).await;
    assert_eq!((s, body), (StatusCode::OK, json!("ok")));
    let (s, _, spec) = ctx.call(Method::GET, "/api/openapi.json", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(spec["paths"]["/api/projects/{slug}/keys"]["get"].is_object());
    assert!(spec["components"]["schemas"]["Project"].is_object());
    let (s, _, e) = ctx.call(Method::GET, "/api/nope", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(e["error"]["code"], "not_found");
    ctx.finish().await;
}
