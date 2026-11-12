//! No-code setup of the website tag, end to end against a fixture site (a Wix-looking page with a navigation, a
//! private area and personal data) and a real Postgres (DATABASE_URL, default :5433; skipped when unreachable):
//! - the server-side fetch refuses private addresses (SSRF) until the test-only override is set;
//! - scan: platform, language, tag not installed → installed (right project, in <head>), CSP diagnosis, private
//!   paths suggested for exclusion;
//! - crawl before go-live (tag still off): pages found through links and the sitemap, their public text stored
//!   and machine-translated (mock provider), personal data never stored; the preview endpoint then returns the
//!   page with a TM that already translates it — and only for the project's own site;
//! - a visitor's report from a private-looking path is refused unless the project includes that path;
//! - metering: the tag's page views count against the plan's `website_pages` (its own counter: Edge pages are
//!   untouched), one per report; refused while the tag is off; 402 (readable cross-origin) past the limit.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::get,
};
use glot_core::edge::tm::text_hash;
use glot_server::billing::Config;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tower::ServiceExt;

const TAG: &str = r#"<script src="http://localhost:3100/widget/glot.js" data-project="shop" data-key="pk_test" async></script>"#;

fn page(installed: bool, body: &str) -> axum::response::Html<String> {
    axum::response::Html(format!(
        r#"<!doctype html><html lang="en"><head><title>Atelier</title>
<link rel="stylesheet" href="https://static.wixstatic.com/site.css">{}</head><body>
<nav><a href="/">Shop</a> <a href="/about">About</a> <a href="/account/orders">My orders</a> <a href="/cart">Cart</a></nav>
<main>{body}</main></body></html>"#,
        if installed { TAG } else { "" }
    ))
}

async fn site(installed: Arc<AtomicBool>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let i = installed.clone();
    let b = base.clone();
    let app = Router::new()
        .route(
            "/",
            get(move || {
                let i = i.clone();
                async move {
                    page(
                        i.load(Ordering::SeqCst),
                        r#"<h1>Handmade ceramics</h1><p>Bowls, mugs and plates thrown by hand in Lyon.</p>
<p>Write to hello@shop.test or call +33 6 12 34 56 78.</p><img alt="A blue bowl" src="/b.jpg">
<form><input placeholder="Your name"></form>
<p>Ceramics are objects made from clay hardened by heat. Our workshop has been throwing bowls, mugs and plates
by hand since 2009, and every piece is glazed and fired twice before it leaves the studio for your table.</p>"#,
                    )
                }
            }),
        )
        .route(
            "/about",
            get(|| async { page(false, "<h2>Our story</h2><p>Three potters in a former silk workshop.</p>") }),
        )
        .route(
            "/visit",
            get(|| async { page(false, "<h2>Visit the workshop</h2>") }),
        )
        .route(
            "/account/orders",
            get(|| async { page(false, "<h2>Sign in to see your orders</h2>") }),
        )
        .route("/cart", get(|| async { page(false, "<p>Your cart is empty</p>") }))
        .route(
            "/sitemap.xml",
            get(move || {
                let b = b.clone();
                async move {
                    format!("<urlset><url><loc>{b}/visit</loc></url><url><loc>https://elsewhere.test/x</loc></url></urlset>")
                }
            }),
        );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    base
}

#[tokio::test]
async fn website_setup_scan_crawl_preview_and_metering() {
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
    let mut state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let cfg = Config::new(
        "sk_test_x",
        "whsec_x",
        "http://localhost:12111",
        "http://localhost:3100",
    );
    let limit = glot_server::billing::default_plans()["free"]
        .website_pages
        .expect("the free plan limits the tag's page views");
    assert!(limit > 0, "the tag works on the free plan");
    state.billing = Some(Arc::new(cfg));
    glot_server::jobs::spawn_worker(state.clone());
    let app = glot_server::router(state, "http://localhost:3100");

    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
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
        Some(json!({"name": "Shop", "slug": "shop", "source_locale": "en", "target_locales": ["es", "de"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/shop",
        c,
        Some(json!({"public_cdn": true, "llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'shop'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_test')")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

    let installed = Arc::new(AtomicBool::new(false));
    let base = site(installed.clone()).await;
    let scan = |u: String| {
        call(
            Method::POST,
            "/api/projects/shop/edge/website/scan",
            c,
            Some(json!({"url": u})),
        )
    };

    // ---- SSRF: a private address is refused; nothing is fetched.
    let (s, _, r) = scan(base.clone()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["error"], "private", "{r}");
    let (_, _, r) = call(
        Method::POST,
        "/api/projects/shop/edge/website/crawl",
        c,
        Some(json!({"url": base})),
    )
    .await;
    assert_eq!(
        (r["error"].as_str(), r["pages"].as_i64()),
        (Some("private"), Some(0)),
        "{r}"
    );
    let (_, _, r) = scan("javascript:alert(1)".into()).await;
    assert_eq!(r["error"], "invalid");
    // Test-only override (the fixture site listens on 127.0.0.1).
    unsafe { std::env::set_var("GLOT_EDGE_ALLOW_PRIVATE", "1") };

    // ---- scan: before installing anything.
    let (_, _, r) = scan(base.clone()).await;
    assert_eq!(r["error"], Value::Null, "{r}");
    assert_eq!(
        (
            r["platform"].as_str(),
            r["lang"].as_str(),
            r["title"].as_str(),
            r["tag"]["found"].as_bool()
        ),
        (Some("wix"), Some("en"), Some("Atelier"), Some(false)),
        "{r}"
    );
    assert_eq!(r["suggested_exclude"], json!(["/account*", "/cart*"]));
    assert_eq!(r["csp_blocked"], json!([]));

    // ---- crawl before go-live: the tag is off, visitors' reports are refused, glot's own crawl is not.
    let (_, _, st) = call(Method::GET, "/edge/v1/shop/status", None, None).await;
    assert_eq!(st["website"]["on"], false);
    assert_eq!(st["discover"], json!({"include": [], "exclude": []}));
    let (s, _, r) = call(
        Method::POST,
        "/api/projects/shop/edge/website/crawl",
        c,
        Some(json!({"url": base})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    // / + /about, /account/orders, /cart (links) + /visit (sitemap; the other host's URL is ignored).
    assert_eq!(
        (r["found"].as_i64(), r["pages"].as_i64()),
        (Some(5), Some(5)),
        "{r}"
    );
    assert_eq!(r["locales"], json!(["es", "de"]));
    assert_eq!(r["queued"], json!(["es", "de"]), "{r}");
    let texts: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT source_text FROM edge_segments ORDER BY 1")
            .fetch_all(&pool)
            .await
            .unwrap();
    for t in [
        "Handmade ceramics",
        "Our story",
        "Visit the workshop",
        "A blue bowl",
        "Your name",
        "Your cart is empty",
        "Atelier",
    ] {
        assert!(texts.iter().any(|x| x == t), "{t} not learnt: {texts:?}");
    }
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("hello@") || t.contains("+33")),
        "personal data stored: {texts:?}"
    );
    let (_, _, settings) = call(Method::GET, "/api/projects/shop/edge/settings", c, None).await;
    assert_eq!(
        settings["website_snippet"]["site"], base,
        "the site is remembered"
    );
    assert_eq!(settings["website_snippet"]["on"], false);

    // ---- preview: the translated page exists before the tag is installed or switched on.
    let enc = |u: &str| u.replace(':', "%3A").replace('/', "%2F");
    let preview = format!(
        "/api/projects/shop/edge/website/page?locale=es&url={}",
        enc(&format!("{base}/"))
    );
    let h1 = text_hash("en", "Handmade ceramics");
    let mut translated = Value::Null;
    for _ in 0..100 {
        let (s, _, r) = call(Method::GET, &preview, c, None).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        assert!(
            r["html"]
                .as_str()
                .unwrap()
                .contains("<h1>Handmade ceramics</h1>")
        );
        assert_eq!(
            (r["path"].as_str(), r["source"].as_str()),
            (Some("/"), Some("en"))
        );
        if !r["tm"]["t"][&h1].is_null() {
            translated = r["tm"]["t"][&h1].clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(
        !translated.is_null(),
        "the crawl's text is translated before go-live"
    );
    // Not a fetch-anything proxy: only the project's own site.
    let (s, _, r) = call(
        Method::GET,
        &format!(
            "/api/projects/shop/edge/website/page?locale=es&url={}",
            enc("https://example.com/")
        ),
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{r}");

    // ---- verification after installing: the tag is found, for this project, in <head>.
    installed.store(true, Ordering::SeqCst);
    let (_, _, r) = scan(base.clone()).await;
    assert_eq!(
        r["tag"],
        json!({"found": true, "project": "shop", "right_project": true, "key": true, "in_head": true}),
        "{r}"
    );

    // ---- go live; a visitor's report from a private-looking path is refused unless included.
    let usage = |mode: Option<&str>| {
        call(
            Method::POST,
            "/edge/v1/shop/usage",
            None,
            Some(json!({"key": "pk_test", "pages": 1, "mode": mode})),
        )
    };
    let (s, h, _) = usage(Some("website")).await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "tag off: its page views are not counted"
    );
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let mut live = settings.clone();
    live["website_snippet"]["on"] = json!(true);
    let (s, _, r) = call(
        Method::PUT,
        "/api/projects/shop/edge/settings",
        c,
        Some(live.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let report = |path: &str, text: &str| {
        call(
            Method::POST,
            "/edge/v1/shop/missing",
            None,
            Some(json!({"key": "pk_test", "locale": "es", "mode": "website",
                "segments": [{"hash": text_hash("en", text), "text": text, "url": path, "role": "p"}]})),
        )
    };
    let (_, _, r) = report("/account/orders", "Order shipped to Jane").await;
    assert_eq!(
        (r["accepted"].as_i64(), r["rejected"].as_i64()),
        (Some(0), Some(1)),
        "{r}"
    );
    let (_, _, r) = report("/pricing", "Simple pricing").await;
    assert_eq!(r["accepted"], 1, "{r}");
    live["discover_include"] = json!(["/account/help*", "/pricing"]);
    call(
        Method::PUT,
        "/api/projects/shop/edge/settings",
        c,
        Some(live),
    )
    .await;
    let (_, _, r) = report("/account/help", "How returns work").await;
    assert_eq!(r["accepted"], 1, "explicitly included: {r}");
    let (_, _, r) = report("/account/orders", "Order shipped to Jane").await;
    assert_eq!(r["accepted"], 0, "{r}");

    // ---- metering: the tag's own counter.
    let count = |kind: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT coalesce(sum(n), 0)::int8 FROM billing_usage WHERE kind = $1",
            )
            .bind(kind)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let used = || count("website_pages");
    assert_eq!(used().await, 0);
    let (s, _, _) = usage(Some("website")).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(
        Method::POST,
        "/edge/v1/shop/usage",
        None,
        Some(json!({"key": "pk_test", "pages": 500, "mode": "website"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(used().await, 2, "a browser reports one page view at a time");
    assert_eq!(
        count("edge_pages").await,
        0,
        "Edge pages are a different counter"
    );
    sqlx::query("UPDATE billing_usage SET n = $1 WHERE kind = 'website_pages'")
        .bind(limit)
        .execute(&pool)
        .await
        .unwrap();
    let (s, h, r) = usage(Some("website")).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED, "{r}");
    assert_eq!(
        h[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "*",
        "the tag can read the refusal"
    );

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
