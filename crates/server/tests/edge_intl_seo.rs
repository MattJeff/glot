//! glot Edge as an international-SEO product, end to end against a fixture site (real Postgres, mock provider):
//! a page is not advertised before it is complete, the pre-warm translates the site before its first visitor
//! (robots.txt, application paths, budget respected), everything a crawler reads is translated, translated
//! slugs with redirects and reciprocal hreflang, conditional requests, and what the proxy costs.
//! Run with `-- --nocapture` to see the measurements.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, Uri, header},
    response::IntoResponse,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

fn home() -> String {
    let features: String = (1..=12)
        .map(|i| format!("<li>Feature number {i} explained in a full sentence.</li>"))
        .collect();
    format!(
        r#"<!doctype html><html lang="en"><head><title>Clip your videos</title>
<meta name="description" content="Short clips from long videos">
<meta property="og:title" content="Clip your videos"><meta property="og:description" content="Short clips from long videos">
<meta property="og:locale" content="en_US"><meta property="og:url" content="/">
<meta name="twitter:title" content="Clip your videos"><meta name="twitter:description" content="Short clips from long videos">
<link rel="canonical" href="/"></head>
<body><nav aria-label="Main menu"><a href="/pricing">Pricing</a> <a href="/blog/my-first-post">Blog</a>
<a href="/private">Private area</a> <a href="/login">Sign in</a> <a href="/search?q=x">Search</a> <a href="/about">About us</a></nav>
<h1>Clip your videos</h1><img src="/hero.png" alt="A video editor timeline"><ul>{features}</ul></body></html>"#
    )
}

const PRICING: &str = r#"<!doctype html><html lang="en"><head><title>Simple pricing</title>
<meta name="description" content="Plans for every team">
<script type="application/ld+json">{"@context":"https://schema.org","@type":"Product","name":"Simple pricing","description":"A plan for every team, billed monthly.","inLanguage":"en","url":"/pricing","offers":{"@type":"Offer","price":"19.00","priceCurrency":"USD"}}</script>
</head><body><a href="/">Home</a><h1>Simple pricing</h1><p>From $19.00 a month, billed yearly.</p>
<pre><code>npm install clip</code></pre><p>Start free, upgrade when you grow.</p><!--extra--></body></html>"#;

const POST: &str = r#"<!doctype html><html lang="en"><head><title>Our first post</title></head>
<body><h1>Our first post</h1><p>We shipped the editor today.</p><p>Write to jane.doe@site.test for the beta.</p></body></html>"#;

const DRAFT: &str = r#"<!doctype html><html lang="en"><head><title>Draft</title><meta name="robots" content="noindex"></head><body><p>Not ready to be read.</p></body></html>"#;

#[derive(Clone, Default)]
struct Origin {
    hits: Arc<Mutex<HashMap<String, usize>>>,
    extra: Arc<Mutex<String>>,
    sitemap: Arc<Mutex<String>>,
}

async fn serve(State(o): State<Origin>, uri: Uri) -> axum::response::Response {
    let path = uri.path().to_string();
    *o.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let html = |b: String| {
        (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            b,
        )
            .into_response()
    };
    match path.as_str() {
        "/" => html(home()),
        "/pricing" => html(PRICING.replace("<!--extra-->", &o.extra.lock().unwrap())),
        "/blog/my-first-post" => html(POST.into()),
        "/draft" => html(DRAFT.into()),
        "/about" | "/private" | "/login" | "/search" => html("<html><head><title>Other</title></head><body><p>Some other page here.</p></body></html>".into()),
        "/robots.txt" => "User-agent: *\nDisallow: /private\n".into_response(),
        "/sitemap.xml" => (
            [(header::CONTENT_TYPE, "application/xml")],
            o.sitemap.lock().unwrap().clone(),
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[tokio::test(flavor = "multi_thread")]
async fn edge_intl_seo() {
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
        std::env::set_var("GLOT_EDGE_PREWARM_DELAY_MS", "0");
    }
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    glot_server::jobs::spawn_worker(state.clone());
    let app = glot_server::router(state.clone(), "http://localhost:3100");

    // --- The source site: sitemap = / + /pricing + /draft (noindex); the blog post is only linked.
    let origin = Origin::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let sitemap = format!(
        r#"<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>{base}/</loc></url><url><loc>{base}/pricing</loc></url><url><loc>{base}/draft</loc></url></urlset>"#
    );
    let site = Router::new().fallback(serve).with_state(origin.clone());
    tokio::spawn(async move { axum::serve(l, site).await.unwrap() });
    *origin.sitemap.lock().unwrap() = sitemap;
    let hits = |p: &str| origin.hits.lock().unwrap().get(p).copied().unwrap_or(0);

    let call =
        |method: Method, host: &str, uri: &str, extra: &[(&str, &str)], body: Option<Value>| {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::HOST, host)
                .header(header::CONTENT_TYPE, "application/json");
            for (k, v) in extra {
                req = req.header(*k, *v);
            }
            let req = req
                .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                .unwrap();
            let app = app.clone();
            async move {
                let res = app.oneshot(req).await.unwrap();
                let (s, h): (StatusCode, HeaderMap) = (res.status(), res.headers().clone());
                let b = res.into_body().collect().await.unwrap().to_bytes();
                (s, h, String::from_utf8_lossy(&b).into_owned())
            }
        };
    let page = |host: &'static str, path: &str| {
        call(
            Method::GET,
            host,
            &format!("/edge/proxy{path}"),
            &[("accept", "text/html")],
            None,
        )
    };

    // --- A user, a project on the mock provider, three verified subdomains (es twice: see the slug change).
    let (s, h, _) = call(
        Method::POST,
        "api",
        "/api/auth/signup",
        &[],
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
    let auth = [("cookie", cookie.as_str())];
    let (s, _, _) = call(
        Method::POST,
        "api",
        "/api/projects",
        &auth,
        Some(json!({"name": "Site", "slug": "site", "source_locale": "en", "target_locales": ["es", "fr"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "api",
        "/api/projects/site",
        &auth,
        Some(json!({"public_cdn": true, "llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    let pid: Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'site'")
        .fetch_one(&pool)
        .await
        .unwrap();
    for (host, locale) in [
        ("es.site.test", "es"),
        ("fr.site.test", "fr"),
        ("es-b.site.test", "es"),
    ] {
        sqlx::query("INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at)
                     VALUES ($1, $2, $3, 'subdomain', $4, $5, 't', now())")
            .bind(Uuid::new_v4()).bind(pid).bind(host).bind(locale).bind(&base)
            .execute(&pool).await.unwrap();
    }
    // Opt-in translated slugs; a small pre-warm budget (3 pages) to see it respected.
    let (s, _, settings) = call(
        Method::GET,
        "api",
        "/api/projects/site/edge/settings",
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let mut settings: Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(settings["prewarm"], true, "pre-warm is on by default");
    assert_eq!(
        settings["translated_slugs"], false,
        "translated slugs are opt-in"
    );
    settings["prewarm_pages"] = json!(3);
    settings["translated_slugs"] = json!(true);
    let (s, _, b) = call(
        Method::PUT,
        "api",
        "/api/projects/site/edge/settings",
        &auth,
        Some(settings),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // --- BEFORE: the very first visitor of a page nobody translated. It is served (fail-open, source text) but
    // it is NOT advertised: canonical to the source page, no hreflang, nothing in the sitemap.
    // (The first request of a cold process may run out of glot's 400 ms budget: served as the origin wrote it,
    // `no-store`, and declared a duplicate of the source page in a `Link` header.)
    let (mut s, mut h, mut body) = page("fr.site.test", "/pricing").await;
    for _ in 0..20 {
        if h["x-glot-edge"] != "fail-open" {
            break;
        }
        assert_eq!(
            h[header::LINK],
            format!(r#"<{base}/pricing>; rel="canonical""#).as_str()
        );
        assert_eq!(h[header::CACHE_CONTROL], "no-store");
        (s, h, body) = page("fr.site.test", "/pricing").await;
    }
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-glot-index"], "pending");
    let before = h["x-glot-coverage"].to_str().unwrap().to_string();
    assert!(before.starts_with("0/"), "{before}");
    assert!(
        body.contains(&format!(r#"<link rel="canonical" href="{base}/pricing">"#)),
        "an untranslated page is a duplicate of the source:\n{body}"
    );
    assert!(!body.contains("hreflang"), "{body}");
    assert_eq!(body.matches(r#"rel="canonical""#).count(), 1);

    // --- That visit started the pre-warm (it also starts when a domain is verified, or from the dashboard).
    let t0 = Instant::now();
    loop {
        let st: Option<String> =
            sqlx::query_scalar("SELECT status FROM edge_prewarm WHERE project_id = $1")
                .bind(pid)
                .fetch_optional(&pool)
                .await
                .unwrap();
        if st.as_deref() == Some("done") {
            break;
        }
        assert_ne!(st.as_deref(), Some("failed"));
        assert!(
            t0.elapsed() < Duration::from_secs(90),
            "pre-warm stuck: {st:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (s, _, st) = call(
        Method::GET,
        "api",
        "/api/projects/site/edge/prewarm",
        &auth,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let st: Value = serde_json::from_str(&st).unwrap();
    assert_eq!(st["pages_done"], 3, "{st}");
    // Budget: 3 pages (/, /pricing, then the first link). robots.txt, application paths, query strings, noindex.
    assert!(
        hits("/blog/my-first-post") >= 1,
        "a linked page that is not in the sitemap is found"
    );
    assert_eq!(hits("/about"), 0, "over the budget of 3 pages");
    assert_eq!(hits("/private"), 0, "robots.txt disallows it");
    assert_eq!(hits("/login"), 0, "an application path is never crawled");
    assert_eq!(
        hits("/search"),
        0,
        "URLs with a query string are not crawled"
    );
    assert!(
        st["skipped"].as_i64().unwrap() >= 1,
        "the noindex draft is skipped: {st}"
    );
    // A second run right away is refused for the automatic path (once a day), accepted on demand.
    let p = glot_server::projects::load_public(&state, "site")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !glot_server::edge::prewarm::start(&state, &p, false)
            .await
            .unwrap()
    );
    // Personal data never reached glot, the draft neither.
    let texts: Vec<String> =
        sqlx::query_scalar("SELECT source_text FROM edge_segments WHERE project_id = $1")
            .bind(pid)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        !texts
            .iter()
            .any(|t| t.contains('@') || t.contains("Not ready")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t == "A plan for every team, billed monthly."),
        "JSON-LD sentence discovered: {texts:?}"
    );

    // --- AFTER: the first visit ever of the Spanish site. Source path → 301 to the translated slug.
    let (s, h, _) = page("es.site.test", "/pricing").await;
    assert_eq!(s, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(h[header::LOCATION], "/es-pricing");
    assert_eq!(h["x-glot-edge"], "slug-redirect");
    // A data request of the app (no `Accept: text/html`) is not redirected.
    let (s, _, _) = call(
        Method::GET,
        "es.site.test",
        "/edge/proxy/pricing",
        &[("accept", "*/*")],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let t = Instant::now();
    let (s, h, body) = page("es.site.test", "/es-pricing").await;
    let first_visit = t.elapsed();
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(
        h["x-glot-edge"].to_str().unwrap().starts_with("translated"),
        "{h:?}"
    );
    assert_eq!(h["x-glot-index"], "ready", "{body}");
    let after = h["x-glot-coverage"].to_str().unwrap().to_string();
    let (done, total) = after.split_once('/').unwrap();
    assert_eq!(done, total, "complete on the first visit: {after}");
    eprintln!("MEASURE first-visit coverage /pricing: before pre-warm {before}, after {after}");
    for want in [
        r#"<html lang="es" dir="ltr">"#,
        "<title>⟦es⟧ Simple pricing</title>",
        r#"<meta name="description" content="⟦es⟧ Plans for every team">"#,
        "<h1>⟦es⟧ Simple pricing</h1>",
        // Numbers and code are kept as written.
        "$19.00",
        "<pre><code>npm install clip</code></pre>",
        // Canonical + reciprocal hreflang with each locale's own slug, x-default = the source.
        r#"<link rel="canonical" href="https://es.site.test/es-pricing">"#,
        r#"<link rel="alternate" hreflang="es" href="https://es.site.test/es-pricing">"#,
        r#"<link rel="alternate" hreflang="fr" href="https://fr.site.test/fr-pricing">"#,
        // JSON-LD: texts translated (the sentence only exists there), URL localized, price untouched.
        r#""name":"⟦es⟧ Simple pricing""#,
        r#""description":"⟦es⟧ A plan for every team, billed monthly.""#,
        r#""inLanguage":"es""#,
        r#""url":"https://es.site.test/es-pricing""#,
        r#""price":"19.00""#,
    ] {
        assert!(body.contains(want), "missing {want}:\n{body}");
    }
    assert!(
        body.contains(&format!(r#"hreflang="en" href="{base}/pricing""#)),
        "{body}"
    );
    assert!(
        body.contains(&format!(r#"hreflang="x-default" href="{base}/pricing""#)),
        "{body}"
    );
    assert_eq!(h[header::CONTENT_LANGUAGE], "es");

    // Home: every crawler-facing text, and internal links carrying the translated slugs.
    let (_, h, body) = page("es.site.test", "/").await;
    assert_eq!(h["x-glot-index"], "ready");
    for want in [
        "<title>⟦es⟧ Clip your videos</title>",
        r#"property="og:title" content="⟦es⟧ Clip your videos""#,
        r#"property="og:description" content="⟦es⟧ Short clips from long videos""#,
        r#"name="twitter:title" content="⟦es⟧ Clip your videos""#,
        r#"name="twitter:description" content="⟦es⟧ Short clips from long videos""#,
        r#"property="og:locale" content="es_ES""#,
        r#"property="og:url" content="https://es.site.test/""#,
        r#"alt="⟦es⟧ A video editor timeline""#,
        r#"aria-label="⟦es⟧ Main menu""#,
        r#"<a href="/es-pricing">"#,
        r#"<a href="/es-blog/es-my-first-post">"#,
    ] {
        assert!(body.contains(want), "missing {want}:\n{body}");
    }
    // /about was never read (budget): it has no slug, and its Spanish page is not advertised anywhere.
    assert!(body.contains(r#"<a href="/about">"#), "{body}");

    // A page reached only through a link, whose slug text ("My first post") is on no page: translated like a
    // segment, then written. Personal data is served as is and does not block the page.
    let (s, h, body) = page("es.site.test", "/es-blog/es-my-first-post").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-glot-index"], "ready", "{body}");
    assert!(body.contains("<h1>⟦es⟧ Our first post</h1>"), "{body}");
    assert!(
        body.contains("Write to jane.doe@site.test for the beta."),
        "{body}"
    );

    // --- Sitemap of the Spanish site: only complete pages, slugged, each with its alternates.
    let (s, _, xml) = call(
        Method::GET,
        "es.site.test",
        "/edge/proxy/sitemap.xml",
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        xml.contains("<loc>https://es.site.test/es-pricing</loc>"),
        "{xml}"
    );
    assert!(
        xml.contains(r#"hreflang="fr" href="https://fr.site.test/fr-pricing""#),
        "{xml}"
    );
    assert!(
        !xml.contains("draft"),
        "a page glot never completed is not listed:\n{xml}"
    );

    // --- Conditional requests: a crawler that already has the page gets a 304.
    let (_, h, body) = page("es.site.test", "/es-pricing").await;
    let etag = h[header::ETAG].to_str().unwrap().to_string();
    assert!(etag.starts_with("W/\""), "{etag}");
    let (s, h304, b304) = call(
        Method::GET,
        "es.site.test",
        "/edge/proxy/es-pricing",
        &[("accept", "text/html"), ("if-none-match", &etag)],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_MODIFIED);
    assert!(b304.is_empty());
    assert_eq!(h304[header::ETAG], etag.as_str());
    eprintln!(
        "MEASURE conditional request: 200 = {} bytes, 304 = 0 bytes",
        body.len()
    );

    // --- What glot costs: the origin alone vs through the proxy (loopback, in-process), 30 requests each.
    let http = reqwest::Client::new();
    let (mut direct, mut proxied, mut glot) = (vec![], vec![], vec![]);
    for _ in 0..30 {
        let t = Instant::now();
        http.get(format!("{base}/pricing"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        direct.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let (_, h, _) = page("es.site.test", "/es-pricing").await;
        proxied.push(t.elapsed().as_secs_f64() * 1000.0);
        let timing = h["server-timing"].to_str().unwrap();
        glot.push(
            timing["glot;dur=".len()..]
                .split(';')
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap(),
        );
    }
    let (direct, proxied, glot) = (median(direct), median(proxied), median(glot));
    eprintln!(
        "MEASURE median ms: origin alone {direct:.2}, through glot {proxied:.2} (glot's own part {glot:.2}); first visit after pre-warm {:.2}",
        first_visit.as_secs_f64() * 1000.0
    );
    assert!(glot < 50.0, "glot's own part of a warm request: {glot} ms");

    // --- The page changes on the origin: it leaves the index cluster until the new text is translated, then
    // comes back by itself.
    *origin.extra.lock().unwrap() = "<p>A brand new paragraph nobody translated yet.</p>".into();
    let (_, h, body) = page("es.site.test", "/es-pricing").await;
    assert_eq!(h["x-glot-index"], "pending");
    assert!(
        body.contains(&format!(r#"<link rel="canonical" href="{base}/pricing">"#)),
        "{body}"
    );
    assert!(!body.contains("hreflang"), "{body}");
    assert!(
        body.contains("<h1>⟦es⟧ Simple pricing</h1>"),
        "still served translated: {body}"
    );
    let t0 = Instant::now();
    loop {
        let (_, h, body) = page("es.site.test", "/es-pricing").await;
        if h["x-glot-index"] == "ready" {
            assert!(body.contains("⟦es⟧ A brand new paragraph nobody translated yet."));
            assert!(body.contains(r#"rel="canonical" href="https://es.site.test/es-pricing""#));
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(60), "never translated");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    eprintln!(
        "MEASURE new paragraph: advertised again after {:.1} s",
        t0.elapsed().as_secs_f64()
    );

    // --- A slug is stable; replacing it by hand keeps the old address working (301). A slug cannot take the
    // address of a real page.
    let put = |slug: &'static str| {
        call(
            Method::PUT,
            "api",
            "/api/projects/site/edge/slugs",
            &auth,
            Some(json!({"locale": "es", "source_path": "/pricing", "slug": slug})),
        )
    };
    let (s, _, b) = put("/blog/my-first-post").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, _, b) = put("/Précios").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, _, b) = put("/precios").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, _, list) = call(
        Method::GET,
        "api",
        "/api/projects/site/edge/slugs",
        &auth,
        None,
    )
    .await;
    let list: Value = serde_json::from_str(&list).unwrap();
    assert!(
        list["slugs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["locale"] == "es"
                && x["source_path"] == "/pricing"
                && x["slug"] == "/precios"
                && x["origin"] == "human"),
        "{list}"
    );
    // (another Spanish host: its domain row is read now, the first one is cached for 15 s)
    for old in ["/es-pricing", "/pricing"] {
        let (s, h, _) = page("es-b.site.test", old).await;
        assert_eq!(s, StatusCode::MOVED_PERMANENTLY, "{old}");
        assert_eq!(h[header::LOCATION], "/precios", "{old}");
    }
    let (s, _, body) = page("es-b.site.test", "/precios").await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("<h1>⟦es⟧ Simple pricing</h1>"), "{body}");
    assert!(
        body.contains(r#"rel="canonical" href="https://es.site.test/precios""#),
        "{body}"
    );

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
