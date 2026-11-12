//! glot Edge international SEO end to end: per-page (critical) coverage gates `ready` in the locales manifest,
//! the per-locale sitemaps / robots.txt / index, IndexNow (Bing/Yandex payload + key file), the coverage API and
//! the SEO report (raw-HTML crawl of a local origin in subdirectory mode). Skipped when Postgres is unreachable.
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use glot_core::edge::{
    segment::{SegmentOpts, segment, translate_html},
    seo,
    tm::{context, text_hash},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const HOME: &str = r#"<!doctype html><html lang="fr"><head><title>Accueil ClipOnIt</title>
<meta name="description" content="Recadrez vos vidéos en un clic">
<link rel="canonical" href="/">
</head><body><h1>Bienvenue</h1><p>Recadrez vos vidéos en un clic</p><a href="/pricing">Tarifs</a></body></html>"#;
const PRICING: &str = r#"<!doctype html><html lang="fr"><head><title>Tarifs ClipOnIt</title>
<link rel="canonical" href="/pricing">
</head><body><h1>Nos tarifs</h1><button>Commencer</button></body></html>"#;

fn es(text: &str) -> Option<String> {
    Some(
        match text {
            "Accueil ClipOnIt" => "Inicio ClipOnIt",
            "Recadrez vos vidéos en un clic" => "Recorta tus vídeos en un clic",
            "Bienvenue" => "Bienvenido",
            "Tarifs" => "Precios",
            "Tarifs ClipOnIt" => "Precios ClipOnIt",
            "Nos tarifs" => "Nuestros precios",
            "Commencer" => "Empezar",
            _ => return None,
        }
        .into(),
    )
}

#[derive(Clone, Default)]
struct Origin {
    pages: Arc<Mutex<Vec<(String, String)>>>,
    indexnow: Arc<Mutex<Option<Value>>>,
}

async fn page(State(o): State<Origin>, uri: axum::http::Uri) -> axum::response::Response {
    let path = uri.path().to_string();
    let found = o
        .pages
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, b)| b.clone());
    match found {
        Some(b) if path.ends_with(".xml") => {
            ([(header::CONTENT_TYPE, "application/xml")], b).into_response()
        }
        Some(b) if path.ends_with(".txt") => b.into_response(),
        Some(b) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], b).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn indexnow(State(o): State<Origin>, body: Bytes) -> StatusCode {
    *o.indexnow.lock().unwrap() = serde_json::from_slice(&body).ok();
    StatusCode::ACCEPTED
}

#[tokio::test]
async fn edge_seo() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    // Local origin on 127.0.0.1; IndexNow posted to it. SAFETY: set before any server code reads them.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let base = format!("http://{host}");
    unsafe {
        std::env::set_var("GLOT_EDGE_ALLOW_PRIVATE", "1");
        std::env::set_var("GLOT_INDEXNOW_URL", format!("{base}/indexnow"));
        std::env::remove_var("GLOT_RENDERER_URL");
    }
    let origin = Origin::default();
    let app_origin = Router::new()
        .route("/indexnow", post(indexnow))
        .fallback(get(page))
        .with_state(origin.clone());
    tokio::spawn(async move { axum::serve(listener, app_origin).await.unwrap() });

    // What glot Edge would serve under /es (segment translation + SEO rewrite), as the origin's /es pages.
    // /es/pricing is deliberately broken the classic proxy way: canonical to the source, title untranslated.
    let opts = SegmentOpts::new("fr");
    let bases = vec![
        ("fr".to_string(), base.clone()),
        ("es".to_string(), format!("{base}/es")),
    ];
    let noop = |_: &str| None;
    let serve = |html: &str, path: &str| {
        let t = translate_html(html, &opts, |s| es(&s.text));
        let ctx = seo::SeoCtx {
            source_locale: "fr".into(),
            locale: "es".into(),
            bases: bases.clone(),
            path: path.into(),
            tr: &noop,
            page_locales: None,
            slugs: None,
        };
        seo::rewrite(&t, &ctx)
    };
    let es_home = serve(HOME, "/");
    assert!(
        es_home.contains("<h1>Bienvenido</h1>")
            && es_home.contains(r#"<html lang="es" dir="ltr">"#),
        "{es_home}"
    );
    assert!(es_home.contains(r#"<a href="/es/pricing">"#), "{es_home}");
    let es_pricing = PRICING
        .replace(r#"lang="fr""#, r#"lang="es""#)
        .replace("<h1>Nos tarifs</h1>", "<h1>Nuestros precios</h1>")
        .replace(r#"href="/pricing""#, &format!(r#"href="{base}/pricing""#));
    // The source pages carry the alternates too (subdirectory mode: the source goes through the edge).
    let fr = |html: &str, path: &str| {
        let ctx = seo::SeoCtx {
            source_locale: "fr".into(),
            locale: "fr".into(),
            bases: bases.clone(),
            path: path.into(),
            tr: &noop,
            page_locales: None,
            slugs: None,
        };
        seo::rewrite(html, &ctx)
    };
    *origin.pages.lock().unwrap() = vec![
        ("/".into(), fr(HOME, "/")),
        ("/pricing".into(), fr(PRICING, "/pricing")),
        ("/es".into(), es_home.clone()),
        ("/es/".into(), es_home),
        ("/es/pricing".into(), es_pricing),
        (
            "/robots.txt".into(),
            format!("User-agent: *\nDisallow: /admin\nSitemap: {base}/sitemap.xml\n"),
        ),
        (
            "/sitemap.xml".into(),
            format!(
                r#"<?xml version="1.0"?><urlset><url><loc>{base}/</loc></url><url><loc>{base}/pricing</loc></url><url><loc>{base}/blog/hello-world</loc></url><url><loc>https://elsewhere.io/x</loc></url></urlset>"#
            ),
        ),
    ];

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
    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers): (StatusCode, HeaderMap) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                headers,
                String::from_utf8_lossy(&bytes).into_owned(),
            )
        }
    };
    let j = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Null);

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
    let (s, _, _) = call(Method::POST, "/api/projects", c,
        Some(json!({"name": "Clip", "slug": "clip", "source_locale": "fr", "target_locales": ["es", "de"]}))).await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/clip",
        c,
        Some(json!({"public_cdn": true})),
    )
    .await;
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'clip'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // No verified domain yet: no sitemap.
    let (s, _, _) = call(Method::GET, "/edge/v1/clip/sitemap.xml", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    sqlx::query("INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at)
                 VALUES ($1, $2, $3, 'subdirectory', 'es', $4, 't', now())")
        .bind(uuid::Uuid::new_v4()).bind(pid).bind(&host).bind(&base)
        .execute(&pool).await.unwrap();

    // Pages render these segments (as the worker / proxy reports them); es translates all but "Commencer".
    let locales = vec!["es".to_string(), "de".to_string()];
    for (path, html) in [("/", HOME), ("/pricing", PRICING)] {
        for s in segment(html, &opts) {
            let ctx = context(&s.ctx.role, path, &locales);
            assert_eq!(s.hash, text_hash("fr", &s.text));
            sqlx::query("INSERT INTO edge_segments (project_id, hash, context, source_text, url_sample) VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
                .bind(pid).bind(&s.hash).bind(&ctx).bind(&s.text).bind(path).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO edge_page_segments (project_id, path, hash, context) VALUES ($1,$2,$3,$4)")
                .bind(pid).bind(path).bind(&s.hash).bind(&ctx).execute(&pool).await.unwrap();
            if let Some(t) = es(&s.text).filter(|_| s.text != "Commencer") {
                sqlx::query("INSERT INTO edge_translations (project_id, hash, context, locale, value, status) VALUES ($1,$2,$3,'es',$4,'machine')")
                    .bind(pid).bind(&s.hash).bind(&ctx).bind(t).execute(&pool).await.unwrap();
            }
        }
    }
    // A non-critical page with nothing translated does not block readiness on its own.
    sqlx::query("INSERT INTO edge_segments (project_id, hash, context, source_text) VALUES ($1, 'b1', 'p|/blog/*', 'Un article')")
        .bind(pid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO edge_page_segments (project_id, path, hash, context) VALUES ($1, '/blog/hello-world', 'b1', 'p|/blog/*')")
        .bind(pid).execute(&pool).await.unwrap();

    // Manifest: /pricing (critical) misses one segment → es not ready, even with min=0.
    let (s, _, m) = call(Method::GET, "/cdn/clip/locales.json?all=1", None, None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let m = j(&m);
    let es_entry = m["locales"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "es")
        .unwrap()
        .clone();
    assert_eq!(es_entry["ready"], false, "{m}");
    let crit = es_entry["critical_coverage"].as_f64().unwrap();
    assert!(crit > 0.5 && crit < 0.98, "{m}");
    assert!(es_entry["version"].as_i64().unwrap() >= 1);
    // Still offered to the language switcher (its catalog is complete): Edge readiness only gates SEO.
    let (_, _, m) = call(Method::GET, "/cdn/clip/locales.json", None, None).await;
    let m = j(&m);
    let es = m["locales"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "es")
        .cloned();
    assert_eq!(es.map(|l| l["ready"].clone()), Some(json!(false)), "{m}");

    // Coverage API: the page and its missing segment, clickable by hash/context/text.
    let (s, _, cov) = call(
        Method::GET,
        "/api/projects/clip/edge/coverage?locale=es",
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{cov}");
    let cov = j(&cov);
    let pages = cov["locales"][0]["pages"].as_array().unwrap();
    assert_eq!(pages[0]["path"], "/pricing", "{cov}");
    assert_eq!(pages[0]["critical"], true);
    assert_eq!(pages[0]["missing"][0]["text"], "Commencer");
    assert!(
        pages
            .iter()
            .any(|p| p["path"] == "/blog/hello-world" && p["critical"] == false)
    );

    // Not ready: nothing advertised to search engines.
    let (s, _, _) = call(Method::GET, "/edge/v1/clip/es/sitemap.xml", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Translate the last critical segment → ready (global ≥ 0.95 needs the blog line too for the SEO bar).
    let (h, ctx): (String, String) =
        sqlx::query_as("SELECT hash, context FROM edge_segments WHERE source_text = 'Commencer'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO edge_translations (project_id, hash, context, locale, value, status) VALUES ($1,$2,$3,'es','Empezar','reviewed'), ($1,'b1','p|/blog/*','es','Un artículo','machine')")
        .bind(pid).bind(&h).bind(&ctx).execute(&pool).await.unwrap();
    let (_, _, m) = call(Method::GET, "/cdn/clip/locales.json?min=0.95", None, None).await;
    let m = j(&m);
    assert_eq!(m["locales"][1]["code"], "es", "{m}");
    assert_eq!(m["locales"][1]["ready"], true);
    assert_eq!(m["locales"][1]["critical_coverage"], 1.0);
    assert_eq!(
        m["locales"].as_array().unwrap().len(),
        2,
        "de has no translation: not ready at 0.95"
    );

    // Sitemaps: es lists the origin's own pages (not elsewhere.io) with reciprocal alternates; index; robots.
    let (s, h, x) = call(Method::GET, "/edge/v1/clip/es/sitemap.xml", None, None).await;
    assert_eq!(s, StatusCode::OK, "{x}");
    assert!(
        h[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("application/xml")
    );
    assert!(x.contains(&format!("<loc>{base}/es/pricing</loc>")), "{x}");
    assert!(x.contains(&format!(r#"hreflang="fr" href="{base}/pricing""#)));
    assert!(x.contains(&format!(
        r#"hreflang="x-default" href="{base}/blog/hello-world""#
    )));
    assert!(!x.contains("elsewhere.io") && !x.contains(r#"hreflang="de""#));
    let (_, _, idx) = call(Method::GET, "/edge/v1/clip/sitemap.xml", None, None).await;
    assert!(
        idx.contains(&format!("<loc>{base}/es/sitemap.xml</loc>")),
        "{idx}"
    );
    let (_, _, r) = call(Method::GET, "/edge/v1/clip/es/robots.txt", None, None).await;
    assert!(
        r.contains("Disallow: /admin") && r.contains(&format!("Sitemap: {base}/es/sitemap.xml")),
        "{r}"
    );
    let (s, _, _) = call(Method::GET, "/edge/v1/clip/de/sitemap.xml", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // IndexNow: Bing/Yandex payload for the es URLs; the key file the proxy serves matches.
    let (s, _, r) = call(Method::POST, "/api/projects/clip/edge/indexnow", c, None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let r = j(&r);
    assert_eq!(r["hosts"][0]["status"], 202, "{r}");
    let sent = origin.indexnow.lock().unwrap().clone().unwrap();
    assert_eq!(sent["host"], "127.0.0.1");
    assert_eq!(sent["urlList"].as_array().unwrap().len(), 3);
    assert!(
        sent["urlList"][0]
            .as_str()
            .unwrap()
            .starts_with(&format!("{base}/es/"))
    );
    let key = sent["key"].as_str().unwrap().to_string();
    let p = glot_server::projects::load_public(&state, "clip")
        .await
        .unwrap()
        .unwrap();
    let res = glot_server::edge::sitemap::special(&state, &p, "es", &format!("/{key}.txt"))
        .await
        .unwrap();
    assert_eq!(
        res.into_body().collect().await.unwrap().to_bytes(),
        key.as_bytes()
    );
    assert!(
        glot_server::edge::sitemap::special(&state, &p, "es", "/pricing")
            .await
            .is_none()
    );

    // SEO report: raw HTML (no JS) per locale.
    let (s, _, rep) = call(Method::POST, "/api/projects/clip/edge/seo-report", c, None).await;
    assert_eq!(s, StatusCode::OK, "{rep}");
    let rep = j(&rep);
    assert_eq!(rep["indexed"]["status"], "unknown");
    let page = |path: &str, locale: &str| -> Value {
        let p = rep["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == path)
            .unwrap();
        p["locales"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["locale"] == locale)
            .unwrap()
            .clone()
    };
    let home_es = page("/", "es");
    assert_eq!(home_es["technically_indexable"], true, "{home_es}");
    assert_eq!(home_es["indexed"], Value::Null);
    assert_eq!(home_es["lang"], "es");
    assert_eq!(home_es["untranslated"]["count"], 0, "{home_es}");
    assert_eq!(
        page("/", "fr")["technically_indexable"],
        true,
        "{}",
        page("/", "fr")
    );
    let pricing_es = page("/pricing", "es");
    let issues: Vec<&str> = pricing_es["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap())
        .collect();
    for want in [
        "canonical_to_source",
        "title_untranslated",
        "untranslated_segments",
        "hreflang_not_reciprocal",
        "hreflang_self_missing",
    ] {
        assert!(issues.contains(&want), "{want} in {issues:?}");
    }
    assert_eq!(pricing_es["technically_indexable"], false);
    let sample: Vec<&str> = pricing_es["untranslated"]["sample"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["text"].as_str().unwrap())
        .collect();
    assert_eq!(sample, ["Tarifs ClipOnIt", "Commencer"], "{pricing_es}");
    let render = &rep["pages"][0]["render"];
    assert_eq!(render["status"], "unknown");
    assert_eq!(render["reason"], "renderer_not_configured");
    // Stored: the dashboard reads the last one.
    let (s, _, last) = call(Method::GET, "/api/projects/clip/edge/seo-report", c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j(&last)["generated_at"], rep["generated_at"]);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
