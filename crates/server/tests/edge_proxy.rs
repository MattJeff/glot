//! glot Edge subdomain proxy against a local origin: translated first byte, SEO, headers kept, Server-Timing,
//! Caddy `ask`, status/kill switch, version bump = new texts (whole page), discovery, no open proxy,
//! stale-if-error and fail-open with Postgres gone. Real Postgres (DATABASE_URL, default :5433), skipped when
//! unreachable.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use glot_core::edge::{
    segment::{SegmentOpts, segment},
    tm::{template, text_hash},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const PAGE: &str = r#"<!doctype html><html lang="en"><head><title>Simple pricing</title>
<meta name="description" content="Plans for every team">
<link rel="canonical" href="https://cliponit.test/pricing"></head>
<body><h1>Simple pricing</h1><p>Start free, upgrade when you grow.</p><button>Save</button>
<a href="/signup">Create an account</a><p>Brand new paragraph</p></body></html>"#;

/// An SPA shell: no content until JavaScript runs (`rendering: auto` → the renderer's snapshot).
const SHELL: &str = r#"<!doctype html><html lang="en"><head><title>App</title></head><body><div id="root"></div><script>boot()</script></body></html>"#;

async fn origin(hits: Arc<AtomicUsize>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let moved = format!("http://{addr}/pricing");
    let app = Router::new()
        .route(
            "/nocache",
            get(move || {
                hits.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        [
                            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                            (header::CACHE_CONTROL, "no-cache"),
                        ],
                        PAGE,
                    )
                }
            }),
        )
        .route(
            "/app",
            get(|| async { ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], SHELL) }),
        )
        .route(
            "/pricing",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CONTENT_SECURITY_POLICY, "default-src 'self'"),
                        (header::CACHE_CONTROL, "public, max-age=60"),
                    ],
                    PAGE,
                )
            }),
        )
        .route(
            "/old",
            get(move || async move {
                (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, moved)]).into_response()
            }),
        )
        .route(
            "/logo.svg",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/svg+xml")],
                    "<svg><text>Save</text></svg>",
                )
            }),
        )
        .route(
            "/login",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::SET_COOKIE, "sid=1; HttpOnly"),
                    ],
                    PAGE,
                )
            }),
        )
        .route(
            "/api/me",
            axum::routing::post(|h: HeaderMap, body: String| async move {
                (
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (
                            header::SET_COOKIE,
                            "session=abc; Path=/; HttpOnly; SameSite=Lax",
                        ),
                    ],
                    format!(
                        "<p>Save</p> {body} {}",
                        h.get(header::COOKIE)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                    ),
                )
            })
            .get(|| async { ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], PAGE) }),
        )
        .route(
            "/form",
            axum::routing::post(|body: String| async move { format!("got {body}") }),
        );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

/// The renderer service: `/snapshot?url=…/app` is queued (202) the first time, then served; anything else 202.
async fn renderer() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/snapshot",
        get(
            move |h: HeaderMap, q: axum::extract::Query<std::collections::HashMap<String, String>>| {
                let seen = seen.clone();
                async move {
                    if h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer rt") {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    if !q["url"].ends_with("/app") || seen.fetch_add(1, Ordering::SeqCst) == 0 {
                        return StatusCode::ACCEPTED.into_response();
                    }
                    (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        "<html><head><title>App</title></head><body><div id=\"root\"><p>Start free, upgrade when you grow.</p></div></body></html>",
                    )
                        .into_response()
                }
            },
        ),
    );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn edge_proxy() {
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
        std::env::set_var("GLOT_RENDERER_URL", renderer().await);
        std::env::set_var("GLOT_RENDERER_TOKEN", "rt");
    }
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state.clone(), "http://localhost:3100");
    let hits = Arc::new(AtomicUsize::new(0));
    let origin = origin(hits.clone()).await;

    let project = Uuid::new_v4();
    sqlx::query(r#"INSERT INTO projects (id, slug, name, source_locale, target_locales, public_cdn, llm) VALUES ($1, 'clip', 'Clip', 'en', '{es,fr,de}', true, '{"provider": "mock", "model": "mock"}')"#)
        .bind(project).execute(&pool).await.unwrap();
    for (host, locale, verified) in [
        ("es.cliponit.test", "es", true),
        ("fr.cliponit.test", "fr", true),
        ("de.cliponit.test", "de", true),
        ("it.cliponit.test", "es", false),
    ] {
        sqlx::query("INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at)
                     VALUES ($1, $2, $3, 'subdomain', $4, $5, 't', CASE WHEN $6 THEN now() END)")
            .bind(Uuid::new_v4()).bind(project).bind(host).bind(locale).bind(&origin).bind(verified)
            .execute(&pool).await.unwrap();
    }
    // TM: every segment of the page but the last paragraph, in es and fr.
    let es = |t: &str| match t {
        "Simple pricing" => "Precios sencillos",
        "Plans for every team" => "Planes para cada equipo",
        "Start free, upgrade when you grow." => "Empieza gratis, mejora cuando crezcas.",
        "Save" => "Guardar",
        "Create an account" => "Crear una cuenta",
        _ => "",
    };
    let locales = vec!["es".to_string(), "fr".into(), "de".into(), "en".into()];
    let tpl = template("/pricing", &locales);
    let segs = segment(PAGE, &SegmentOpts::new("en"));
    assert!(
        segs.iter().any(|s| s.text == "Brand new paragraph"),
        "{segs:?}"
    );
    for s in segs.iter().filter(|s| !es(&s.text).is_empty()) {
        assert_eq!(s.hash, text_hash("en", &s.text));
        let ctx = format!("{}|{tpl}", s.ctx.role);
        sqlx::query("INSERT INTO edge_segments (project_id, hash, context, source_text) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
            .bind(project).bind(&s.hash).bind(&ctx).bind(&s.text).execute(&pool).await.unwrap();
        for (l, v) in [
            ("es", es(&s.text).to_string()),
            ("fr", format!("FR {}", s.text)),
        ] {
            sqlx::query("INSERT INTO edge_translations (project_id, hash, context, locale, value, status) VALUES ($1, $2, $3, $4, $5, 'machine') ON CONFLICT DO NOTHING")
                .bind(project).bind(&s.hash).bind(&ctx).bind(l).bind(v).execute(&pool).await.unwrap();
        }
    }

    // es and fr ready for search engines (another page fully translated, no critical page; stays above 95 % with
    // the untranslated segments this test discovers); de is not.
    sqlx::query(r#"INSERT INTO edge_settings (project_id, critical_patterns, app_paths) VALUES ($1, '{}', '{/login,/live/,"proxy /api/"}')"#)
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    // The site carries the locale as `?lang=es` (application paths redirect with it).
    sqlx::query("INSERT INTO preview_settings (project_id, url_pattern) VALUES ($1, '{route}?lang={locale}')")
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    for i in 0..100 {
        let text = format!("Home text {i}");
        let h = text_hash("en", &text);
        sqlx::query("INSERT INTO edge_segments (project_id, hash, context, source_text) VALUES ($1, $2, 'p|/', $3)")
            .bind(project).bind(&h).bind(&text).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO edge_page_segments (project_id, path, hash, context) VALUES ($1, '/', $2, 'p|/')")
            .bind(project).bind(&h).execute(&pool).await.unwrap();
        for l in ["es", "fr"] {
            sqlx::query("INSERT INTO edge_translations (project_id, hash, context, locale, value, status) VALUES ($1, $2, 'p|/', $3, $4, 'machine')")
                .bind(project).bind(&h).bind(l).bind(format!("{l} {text}")).execute(&pool).await.unwrap();
        }
    }

    let call = |method: Method, host: &str, uri: &str, extra: &[(&str, &str)], body: &str| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, host);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let req = req.body(Body::from(body.to_string())).unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (s, h): (StatusCode, HeaderMap) = (res.status(), res.headers().clone());
            let b = res.into_body().collect().await.unwrap().to_bytes();
            (s, h, String::from_utf8_lossy(&b).into_owned())
        }
    };
    let page = |host: &'static str| {
        call(
            Method::GET,
            host,
            "/edge/proxy/pricing",
            &[("accept", "text/html")],
            "",
        )
    };

    // --- Caddy ask: verified subdomain hosts only.
    assert_eq!(
        call(
            Method::GET,
            "api",
            "/edge/ask?domain=es.cliponit.test",
            &[],
            ""
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            Method::GET,
            "api",
            "/edge/ask?domain=ES.CLIPONIT.TEST.",
            &[],
            ""
        )
        .await
        .0,
        StatusCode::OK
    );
    for d in ["it.cliponit.test", "evil.example.com", "127.0.0.1", ""] {
        assert_eq!(
            call(
                Method::GET,
                "api",
                &format!("/edge/ask?domain={d}"),
                &[],
                ""
            )
            .await
            .0,
            StatusCode::NOT_FOUND,
            "{d}"
        );
    }

    // --- No open proxy: unknown / unverified hosts.
    for h in ["evil.example.com", "it.cliponit.test", "localhost"] {
        let (s, _, _) = call(Method::GET, h, "/edge/proxy/pricing", &[], "").await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{h}");
    }

    // --- Translated first byte (no JS), SEO, headers.
    let (s, h, body) = page("es.cliponit.test").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(h["x-glot-edge"], "translated");
    for want in [
        "<title>Precios sencillos</title>",
        "<h1>Precios sencillos</h1>",
        "Empieza gratis, mejora cuando crezcas.",
        "<button>Guardar</button>",
        r#"content="Planes para cada equipo""#,
        "Brand new paragraph",
        r#"<html lang="es" dir="ltr""#,
        r#"<link rel="canonical" href="https://es.cliponit.test/pricing">"#,
        r#"hreflang="fr" href="https://fr.cliponit.test/pricing""#,
        r#"hreflang="x-default""#,
    ] {
        assert!(body.contains(want), "missing {want}:\n{body}");
    }
    assert!(
        !body.contains(r#"hreflang="de""#),
        "de is not ready: never advertised\n{body}"
    );
    assert_eq!(h[header::CONTENT_LANGUAGE], "es");
    assert_eq!(h[header::CONTENT_SECURITY_POLICY], "default-src 'self'");
    assert!(
        h.get(header::CONTENT_LENGTH)
            .is_none_or(|v| v.to_str().unwrap() == body.len().to_string())
    );
    let v1 = h["x-glot-version"].to_str().unwrap().to_string();
    assert!(v1.starts_with("es@"), "{v1}");
    assert_eq!(
        h["x-glot-coverage"],
        format!("{}/{}", segs.len() - 1, segs.len())
    );
    let timing = h["server-timing"].to_str().unwrap();
    assert!(
        timing.starts_with("glot;dur=") && timing.contains("origin;dur="),
        "{timing}"
    );

    // Client patcher: page version + same-origin script and TM (CSP 'self' friendly).
    assert!(
        body.contains(&format!(r#"<meta name="glot-tm" content="{v1}">"#)),
        "{body}"
    );
    assert!(
        body.contains(
            r#"src="/__glot/patcher.js" data-tm="/__glot/tm.json" data-source="en" data-missing="/__glot/missing"></script>"#
        ),
        "{body}"
    );
    let (s, h, tmj) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/__glot/tm.json",
        &[],
        "",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        h[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("max-age=30")
    );
    let tmj: Value = serde_json::from_str(&tmj).unwrap();
    assert_eq!(format!("es@{}", tmj["v"]), v1);
    assert!(
        tmj["t"]
            .as_object()
            .unwrap()
            .values()
            .any(|v| v == "Guardar"),
        "{tmj}"
    );

    // Page cache: same version → hit.
    let (_, h2, body2) = page("es.cliponit.test").await;
    assert_eq!(h2["x-glot-edge"], "translated; cache=hit");
    assert_eq!(body2, body);

    // --- Origin says no-cache: fetched (revalidated) on every request, the unchanged body's translation reused.
    let nc = |_: ()| {
        call(
            Method::GET,
            "es.cliponit.test",
            "/edge/proxy/nocache",
            &[],
            "",
        )
    };
    let (_, h1, b1) = nc(()).await;
    let (_, h2, b2) = nc(()).await;
    assert_eq!(h1["x-glot-edge"], "translated");
    assert_eq!(h2["x-glot-edge"], "translated; cache=hit-body");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        2,
        "origin revalidated each time"
    );
    assert_eq!(b1, b2);
    assert!(b2.contains("<h1>Precios sencillos</h1>"));
    assert_eq!(h2["x-glot-coverage"], h1["x-glot-coverage"]);
    // A session never shares it (nor feeds it).
    let (_, h3, _) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/nocache",
        &[("cookie", "session=abc")],
        "",
    )
    .await;
    assert_eq!(h3["x-glot-edge"], "translated");

    // --- SPA shell + `rendering: auto`: origin while the renderer queues the snapshot (never waited for), then the
    // snapshot, translated.
    let (_, h, b) = call(Method::GET, "es.cliponit.test", "/edge/proxy/app", &[], "").await;
    assert!(b.contains(r#"<div id="root"></div>"#), "{b}");
    assert!(
        b.contains("/__glot/patcher.js"),
        "patcher on shells too: {b}"
    );
    assert_eq!(h["x-glot-edge"], "source");
    let (_, h, b) = call(Method::GET, "es.cliponit.test", "/edge/proxy/app", &[], "").await;
    assert!(
        b.contains("<p>Empieza gratis, mejora cuando crezcas.</p>"),
        "{b}"
    );
    assert_eq!(h["x-glot-edge"], "translated");

    // --- SEO files on the translated host: robots.txt announces its sitemap, IndexNow key file.
    let (s, _, robots) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/robots.txt",
        &[],
        "",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        robots.contains("Sitemap: https://es.cliponit.test/sitemap.xml"),
        "{robots}"
    );
    let key = glot_server::edge::sitemap::indexnow_key(&state, project);
    let (s, _, k) = call(
        Method::GET,
        "es.cliponit.test",
        &format!("/edge/proxy/{key}.txt"),
        &[],
        "",
    )
    .await;
    assert_eq!((s, k.as_str()), (StatusCode::OK, key.as_str()));
    let (s, _, _) = call(
        Method::GET,
        "de.cliponit.test",
        "/edge/proxy/sitemap.xml",
        &[],
        "",
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "not ready: no sitemap");

    // --- Discovery: the untranslated paragraph is recorded (path only), in the background.
    let mut found = false;
    for _ in 0..50 {
        let r: Option<(String, Option<String>)> = sqlx::query_as("SELECT context, url_sample FROM edge_segments WHERE project_id = $1 AND source_text = 'Brand new paragraph'")
            .bind(project).fetch_optional(&pool).await.unwrap();
        if let Some((ctx, url)) = r {
            assert_eq!(ctx, "p|/pricing");
            assert_eq!(url.as_deref(), Some("/pricing"));
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(found, "missing segment not discovered");

    // --- Client discovery (`/__glot/missing`, same origin): text only the app renders; sessions never count.
    let seg = |t: &str| {
        json!({"locale": "es", "segments": [{"hash": text_hash("en", t), "text": t, "role": "p", "url": "/pricing?x=1"}]})
            .to_string()
    };
    let (s, _, _) = call(
        Method::POST,
        "es.cliponit.test",
        "/edge/proxy/__glot/missing",
        &[("cookie", "next-auth.session-token=x")],
        &seg("Signed in only"),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(
        Method::POST,
        "es.cliponit.test",
        "/edge/proxy/__glot/missing",
        &[],
        &seg("Rendered on the client"),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT source_text, url_sample FROM edge_segments WHERE project_id = $1 AND source_text IN ('Rendered on the client', 'Signed in only')",
    )
    .bind(project)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![("Rendered on the client".into(), Some("/pricing".into()))]
    );

    // --- Non-HTML, redirects, POST: passthrough (Location moved to the edge host, body forwarded).
    let (_, h, svg) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/logo.svg",
        &[],
        "",
    )
    .await;
    assert_eq!(svg, "<svg><text>Save</text></svg>");
    assert_eq!(h["x-glot-edge"], "passthrough");
    let (s, h, _) = call(Method::GET, "es.cliponit.test", "/edge/proxy/old", &[], "").await;
    assert_eq!(s, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(h[header::LOCATION], "https://es.cliponit.test/pricing");
    let (s, _, b) = call(
        Method::POST,
        "es.cliponit.test",
        "/edge/proxy/form",
        &[("content-type", "text/plain")],
        "a=1",
    )
    .await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, "got a=1"));

    // The site root (Caddy rewrites `/` to `/edge/proxy/`) reaches the origin, not the API's 404.
    let (s, h, _) = call(Method::GET, "es.cliponit.test", "/edge/proxy/", &[], "").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(h["x-glot-edge"], "passthrough");

    // --- Status endpoint (read by the worker / middleware).
    let (s, h, st) = call(Method::GET, "api", "/edge/v1/clip/status", &[], "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let st: Value = serde_json::from_str(&st).unwrap();
    assert_eq!(st["enabled"], json!(true));
    assert_eq!(st["off"], json!([]));
    assert_eq!(format!("es@{}", st["versions"]["es"]), v1);

    // --- Version bump: a translation edited in glot is live within VERSION_TTL, the whole page at the new version.
    sqlx::query("UPDATE edge_translations SET value = 'Guardar ahora' WHERE project_id = $1 AND locale = 'es' AND value = 'Guardar'")
        .bind(project).execute(&pool).await.unwrap();
    let mut new = String::new();
    for _ in 0..80 {
        let (_, h, b) = page("es.cliponit.test").await;
        if h["x-glot-version"] != v1.as_str() {
            assert!(
                b.contains("<button>Guardar ahora</button>")
                    && b.contains("<h1>Precios sencillos</h1>"),
                "{b}"
            );
            assert_eq!(h["x-glot-edge"], "translated");
            new = h["x-glot-version"].to_str().unwrap().into();
            break;
        }
        assert!(
            b.contains("<button>Guardar</button>"),
            "old version, old texts: {b}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!new.is_empty(), "version never changed");

    // --- Kill switch: per domain (fr) and project; a host not seen yet reads it at once, others within DOMAIN_TTL.
    sqlx::query("UPDATE edge_domains SET enabled = false WHERE host = 'fr.cliponit.test'")
        .execute(&pool)
        .await
        .unwrap();
    let (s, h, b) = page("fr.cliponit.test").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-glot-edge"], "off");
    assert!(
        b.contains("<h1>Simple pricing</h1>") && !b.contains("FR "),
        "{b}"
    );
    let st: Value = serde_json::from_str(
        &call(Method::GET, "api", "/edge/v1/clip/status", &[], "")
            .await
            .2,
    )
    .unwrap();
    assert_eq!(st["off"], json!(["fr.cliponit.test"]));

    // --- Application paths: back to the origin's own domain with the locale, or passed through untouched.
    let (s, h, _) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/login?next=%2Fdashboard&lang=fr",
        &[("accept", "text/html")],
        "",
    )
    .await;
    assert_eq!(s, StatusCode::FOUND);
    assert_eq!(
        h[header::LOCATION],
        format!("{origin}/login?next=%2Fdashboard&lang=es")
    );
    assert_eq!(h["x-glot-edge"], "app-redirect");
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert!(
        h[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .starts_with("glot_app_hop=/login;")
    );
    // Non-GET keeps method and body (307); children-only rule; segment match.
    let (s, h, _) = call(
        Method::POST,
        "es.cliponit.test",
        "/edge/proxy/live/abc",
        &[],
        "x=1",
    )
    .await;
    assert_eq!(s, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(h[header::LOCATION], format!("{origin}/live/abc?lang=es"));
    let (_, h, _) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/loginx",
        &[],
        "",
    )
    .await;
    assert_ne!(h["x-glot-edge"], "app-redirect");
    // Open-redirect attempts: always the domain's own origin, normalized path.
    for (uri, want) in [
        ("/edge/proxy//evil.com/%2e%2e/login", "/login?lang=es"),
        ("/edge/proxy/%2F%2Fevil.com/../login", "/login?lang=es"),
        (
            "/edge/proxy/x/../login?u=https://evil.com",
            "/login?u=https://evil.com&lang=es",
        ),
    ] {
        let (s, h, _) = call(
            Method::GET,
            "es.cliponit.test",
            uri,
            &[("x-forwarded-host", "evil.com")],
            "",
        )
        .await;
        assert_eq!(s, StatusCode::FOUND, "{uri}");
        assert_eq!(h[header::LOCATION], format!("{origin}{want}"), "{uri}");
    }
    // Loop guard: the origin sent the visitor back (hop cookie for this path) → passed through, not redirected.
    let (s, h, b) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/login?lang=es",
        &[("accept", "text/html"), ("cookie", "glot_app_hop=/login")],
        "",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-glot-edge"], "app");
    assert!(
        b.contains("<h1>Simple pricing</h1>"),
        "never translated: {b}"
    );
    assert_eq!(h[header::SET_COOKIE], "sid=1; HttpOnly");
    // Proxy rule: method, body and cookies through, Set-Cookie untouched, never translated or cached.
    let (s, h, b) = call(
        Method::POST,
        "es.cliponit.test",
        "/edge/proxy/api/me",
        &[("cookie", "session=abc")],
        "payload",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-glot-edge"], "app");
    assert_eq!(
        h[header::SET_COOKIE],
        "session=abc; Path=/; HttpOnly; SameSite=Lax"
    );
    assert_eq!(b, "<p>Save</p> payload session=abc");
    let (_, h, b) = call(
        Method::GET,
        "es.cliponit.test",
        "/edge/proxy/api/me",
        &[("accept", "text/html")],
        "",
    )
    .await;
    assert_eq!(h["x-glot-edge"], "app");
    assert!(
        b.contains("<button>Save</button>") && !b.contains("Guardar"),
        "{b}"
    );
    // The JS runtimes read the rules from the status (within their refresh interval).
    let st: Value = serde_json::from_str(
        &call(Method::GET, "api", "/edge/v1/clip/status", &[], "")
            .await
            .2,
    )
    .unwrap();
    assert_eq!(
        st["app"],
        json!({"rules": ["/login", "/live/", "proxy /api/"], "action": "redirect", "locale": "query:lang"})
    );

    // --- Postgres gone: es keeps its last TM (stale-if-error); de (domain known, TM never loaded) fails open.
    let (s, _, _) = call(
        Method::POST,
        "de.cliponit.test",
        "/edge/proxy/form",
        &[],
        "x",
    )
    .await; // caches the domain only
    assert_eq!(s, StatusCode::OK);
    pool.close().await;
    let t = std::time::Instant::now();
    let (s, h, b) = page("es.cliponit.test").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        b.contains("<button>Guardar ahora</button>"),
        "stale TM: {b}"
    );
    assert_eq!(h["x-glot-version"], new.as_str());
    let (s, h, b) = page("de.cliponit.test").await;
    assert_eq!(s, StatusCode::OK, "never a 5xx because of glot");
    assert_eq!(h["x-glot-edge"], "fail-open");
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert!(b.contains("<h1>Simple pricing</h1>"));
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    // The last state is on disk too (a restarted server with Postgres down still serves it).
    assert!(std::fs::read_dir(cache.path()).unwrap().count() >= 2);

    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
