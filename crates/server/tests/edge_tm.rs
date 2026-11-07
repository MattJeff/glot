//! glot Edge TM: missing report → bounded discovery → `edge_translate` job (mock model, validators) → versioned TM
//! snapshot; contexts ("Save" button vs "Save" in a price), catalog defaults, pending review on sensitive pages,
//! one monotonic version shared with the catalog, kill switch, daily cap, personal data and forged hashes refused.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use glot_core::edge::tm::{Tm, context, text_hash};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn edge_tm_end_to_end() {
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
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    glot_server::jobs::spawn_worker(state.clone());
    let app = glot_server::router(state, "http://localhost:3100");

    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        // Browsers post text/plain (no preflight); the API reads JSON whatever the type.
        req = req.header(header::CONTENT_TYPE, "application/json");
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers): (StatusCode, HeaderMap) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            (status, headers, json)
        }
    };

    let (s, h, _) = call(
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
        .unwrap()["glot_session=".len()..]
        .to_string();
    let c = Some(cookie.as_str());
    let (s, _, _) = call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "E", "slug": "ed", "source_locale": "en", "target_locales": ["es"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/ed",
        c,
        Some(json!({"public_cdn": true, "llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'ed'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_test')")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

    // A catalog key whose source is a page text: its translation is the TM default for that text.
    call(
        Method::POST,
        "/api/projects/ed/keys",
        c,
        Some(json!({"key": "nav.pricing", "source": "Pricing"})),
    )
    .await;
    let (s, _, _) = call(
        Method::PUT,
        "/api/projects/ed/keys/nav.pricing/translations/es",
        c,
        Some(json!({"value": "Precios"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let tm_get = |q: &'static str| call(Method::GET, q, None, None);
    let (s, h, body) = tm_get("/edge/v1/ed/es/tm").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(
        h[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("stale-if-error")
    );
    let tm: Tm = serde_json::from_value(body).unwrap();
    let v1 = tm.v;
    assert!(v1 >= 1, "catalog writes bump the shared version");
    assert_eq!(h["x-glot-version"], v1.to_string().as_str());
    let pricing = text_hash("en", "Pricing");
    assert_eq!(tm.lookup(&pricing, "a|/"), Some("Precios"));

    let h = |t: &str| text_hash("en", t);
    let seg = |text: &str, url: &str, role: &str| json!({"hash": h(text), "text": text, "url": url, "role": role});
    let report = |segments: Vec<Value>, extra: Value| {
        let mut b = json!({"key": "pk_test", "locale": "es", "segments": segments});
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        call(Method::POST, "/edge/v1/ed/missing", None, Some(b))
    };

    // Wrong key, unknown locale.
    let (s, _, _) = call(
        Method::POST,
        "/edge/v1/ed/missing",
        None,
        Some(json!({"key": "pk_nope", "locale": "es", "segments": []})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call(
        Method::POST,
        "/edge/v1/ed/missing",
        None,
        Some(json!({"key": "pk_test", "locale": "de", "segments": []})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Visitor with a session: nothing discovered.
    let (s, _, out) = report(vec![seg("Your invoices", "/", "h1")], json!({"auth": true})).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(out["skipped"], "auth");

    let (s, _, out) = report(
        vec![
            seg("Save", "/", "button"),
            seg("Save", "https://x.io/pricing?utm=me", "td"),
            seg("Read our <0>terms</0>", "/", "p"),
            seg("Pay now", "/checkout", "button"),
            // Personal data and a forged hash are refused.
            seg("Contact jane.doe@example.com", "/", "p"),
            json!({"hash": h("Something else"), "text": "Poisoned", "url": "/", "role": "p"}),
        ],
        json!({"pages": [{"url": "/pricing", "seen": [{"hash": h("Save"), "role": "td"}, {"hash": pricing, "role": "a"}]}]}),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{out}");
    assert_eq!(
        (out["accepted"].as_i64(), out["rejected"].as_i64()),
        (Some(4), Some(2)),
        "{out}"
    );
    assert_eq!(out["queued"], true);
    let url_sample: String =
        sqlx::query_scalar("SELECT url_sample FROM edge_segments WHERE context = 'td|/pricing'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(url_sample, "/pricing", "no query string stored");
    let page: i64 =
        sqlx::query_scalar("SELECT count(*) FROM edge_page_segments WHERE path = '/pricing'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(page, 2);

    // The job runs (mock model), then the TM serves the new texts at a newer version.
    let mut tm = Tm::default();
    for _ in 0..300 {
        let done: bool = sqlx::query_scalar(
            "SELECT coalesce(bool_and(status IN ('done', 'failed')), false) FROM jobs WHERE kind = 'edge_translate'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM jobs WHERE kind = 'edge_translate'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "done", "{error:?}");
    let (_, _, body) = tm_get("/edge/v1/ed/es/tm").await;
    tm = serde_json::from_value(body).unwrap_or(tm);
    assert!(tm.v > v1);
    let l = vec!["es".to_string(), "en".to_string()];
    let save_btn = tm.lookup(&h("Save"), &context("button", "/", &l)).unwrap();
    assert!(save_btn.contains("Save"), "{save_btn}");
    let terms = tm.lookup(&h("Read our <0>terms</0>"), "p|/").unwrap();
    assert!(
        terms.contains("<0>") && terms.contains("</0>"),
        "tags kept: {terms}"
    );
    // Sensitive page: translated but pending, not served.
    assert_eq!(tm.lookup(&h("Pay now"), "button|/checkout"), None);
    let (_, _, pending) = call(
        Method::GET,
        "/api/projects/ed/edge/segments?locale=es&status=pending",
        c,
        None,
    )
    .await;
    assert_eq!(
        pending["segments"].as_array().unwrap().len(),
        1,
        "{pending}"
    );
    assert_eq!(pending["segments"][0]["source"], "Pay now");

    // Same text, two contexts, two translations: each context gets its own; an unknown context is ambiguous.
    let (s, _, out) = call(
        Method::PUT,
        &format!("/api/projects/ed/edge/segments/{}/es", h("Save")),
        c,
        Some(json!({"context": "td|/pricing", "value": "Ahorra"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    // Broken tags are refused for a live page.
    let (s, _, _) = call(
        Method::PUT,
        &format!(
            "/api/projects/ed/edge/segments/{}/es",
            h("Read our <0>terms</0>")
        ),
        c,
        Some(json!({"context": "p|/", "value": "Lee nuestros <0>términos"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    // Review the pending one: served from now on.
    let (s, _, _) = call(
        Method::PUT,
        &format!("/api/projects/ed/edge/segments/{}/es", h("Pay now")),
        c,
        Some(json!({"context": "button|/checkout", "value": "Pagar ahora"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, hd, body) = tm_get("/edge/v1/ed/es/tm").await;
    let tm2: Tm = serde_json::from_value(body).unwrap();
    assert!(tm2.v > tm.v);
    assert_eq!(tm2.lookup(&h("Save"), "td|/pricing"), Some("Ahorra"));
    assert_eq!(tm2.lookup(&h("Save"), "button|/"), Some(save_btn));
    assert_eq!(tm2.lookup(&h("Save"), "h1|/"), None);
    assert_eq!(
        tm2.lookup(&h("Pay now"), "button|/checkout"),
        Some("Pagar ahora")
    );

    // Revalidation: same version → 304 (ETag or ?since).
    let etag = hd[header::ETAG].to_str().unwrap().to_string();
    let req = Request::get("/edge/v1/ed/es/tm")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_MODIFIED
    );
    let (s, _, _) = call(
        Method::GET,
        &format!("/edge/v1/ed/es/tm?since={}", tm2.v),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_MODIFIED);
    // A catalog edit moves the same version: catalogs and edge TM never disagree.
    call(
        Method::PUT,
        "/api/projects/ed/keys/nav.pricing/translations/es",
        c,
        Some(json!({"value": "Tarifas"})),
    )
    .await;
    let (s, _, body) = call(
        Method::GET,
        &format!("/edge/v1/ed/es/tm?since={}", tm2.v),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let tm3: Tm = serde_json::from_value(body).unwrap();
    assert!(tm3.v > tm2.v);
    assert_eq!(tm3.lookup(&pricing, "a|/"), Some("Tarifas"));

    // Known segment: counted, not duplicated.
    let (_, _, out) = report(vec![seg("Save", "/", "button")], json!({})).await;
    assert_eq!(
        (out["known"].as_i64(), out["accepted"].as_i64()),
        (Some(1), Some(0))
    );

    // Settings: discovery zones, daily cap, kill switch.
    let (_, _, mut settings) = call(Method::GET, "/api/projects/ed/edge/settings", c, None).await;
    assert_eq!(settings["enabled"], true);
    settings["discover_exclude"] = json!(["/admin*"]);
    settings["daily_cap"] = json!(4);
    let (s, _, e) = call(
        Method::PUT,
        "/api/projects/ed/edge/settings",
        c,
        Some(settings.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    let (_, _, out) = report(
        vec![
            seg("Admin panel", "/admin/x", "h1"),
            seg("New copy", "/", "p"),
        ],
        json!({}),
    )
    .await;
    assert_eq!(
        (out["accepted"].as_i64(), out["rejected"].as_i64()),
        (Some(0), Some(2)),
        "{out}"
    );
    settings["enabled"] = json!(false);
    call(
        Method::PUT,
        "/api/projects/ed/edge/settings",
        c,
        Some(settings),
    )
    .await;
    let (_, _, out) = report(vec![seg("New copy", "/", "p")], json!({})).await;
    assert_eq!(out["skipped"], "disabled");

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
