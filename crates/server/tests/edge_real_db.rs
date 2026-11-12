//! Field check of glot Edge's international SEO on a COPY of real data (never the original, never production
//! writes): `GLOT_REAL_COPY_URL=postgres://…/my_copy cargo test -p glot-server --test edge_real_db -- --ignored
//! --nocapture`. Readiness per page before / after a pre-warm of the real origin (read-only GETs, polite: one per
//! second, `GLOT_REAL_PAGES` pages), then what each page would be served with (`x-glot-index`, canonical).
//! Auto-translation is switched off in the copy first: no model is ever called, every figure comes from stored data.
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use tower::ServiceExt;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "field check: needs GLOT_REAL_COPY_URL (a copy of real data) and network access"]
async fn edge_real_db() {
    let Ok(url) = std::env::var("GLOT_REAL_COPY_URL") else {
        eprintln!("SKIPPED: GLOT_REAL_COPY_URL not set");
        return;
    };
    assert!(
        !url.trim_end_matches('/').ends_with("/glot_real"),
        "use a copy (CREATE DATABASE x TEMPLATE glot_real), never the original"
    );
    let pages: i32 = std::env::var("GLOT_REAL_PAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let pool = PgPool::connect(&url).await.unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let cache = tempfile::tempdir().unwrap();
    // SAFETY: set before the proxy's first use.
    unsafe { std::env::set_var("GLOT_EDGE_CACHE_DIR", cache.path()) };
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state.clone(), "http://localhost:3100");

    for slug in ["cliponit", "orizn"] {
        eprintln!("\n=== {slug}");
        let Some(p) = glot_server::projects::load_public(&state, slug)
            .await
            .unwrap()
        else {
            eprintln!("no such project");
            continue;
        };
        let hosts: Vec<(String, String)> = sqlx::query_as(
            "SELECT host, locale FROM edge_domains WHERE project_id = $1 AND mode = 'subdomain'
             AND verified_at IS NOT NULL AND enabled ORDER BY host",
        )
        .bind(p.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        if hosts.is_empty() {
            eprintln!(
                "REAL {slug}: no verified Edge domain: nothing is served or advertised, nothing to measure"
            );
            continue;
        }
        // No model call in this copy: what is discovered stays untranslated.
        sqlx::query(
            "INSERT INTO edge_settings (project_id, auto_translate, prewarm_pages) VALUES ($1, false, $2)
             ON CONFLICT (project_id) DO UPDATE SET auto_translate = false, prewarm_pages = $2",
        )
        .bind(p.id)
        .bind(pages)
        .execute(&pool)
        .await
        .unwrap();
        let show = |label: &str, r: &glot_server::edge::seo_report::ReadyPages| {
            for (l, ready) in r {
                eprintln!("REAL {slug} {label}: {l} pages ready {}", ready.len());
            }
        };
        let (rd, before) = glot_server::edge::seo_report::readiness_pages(&state, &p)
            .await
            .unwrap();
        let seen: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT path) FROM edge_page_segments WHERE project_id = $1",
        )
        .bind(p.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        eprintln!("REAL {slug} before: {seen} pages known to glot");
        for r in &rd {
            eprintln!(
                "REAL {slug} before: {} language coverage {:.3}, critical {:?}, language ready {}",
                r.locale,
                r.coverage(),
                r.critical_coverage,
                r.ready(0.95)
            );
        }
        show("before", &before);

        let t = Instant::now();
        assert!(
            glot_server::edge::prewarm::start(&state, &p, true)
                .await
                .unwrap()
        );
        let st = loop {
            let st: (String, i32, i32, i32, Option<String>) = sqlx::query_as(
                "SELECT status, pages_done, skipped, segments, error FROM edge_prewarm WHERE project_id = $1",
            )
            .bind(p.id)
            .fetch_one(&pool)
            .await
            .unwrap();
            if st.0 != "running" {
                break st;
            }
            assert!(t.elapsed() < Duration::from_secs(600), "pre-warm stuck");
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        eprintln!(
            "REAL {slug} pre-warm: {} in {:.0} s, {} pages read, {} skipped, {} new segments, error {:?}",
            st.0,
            t.elapsed().as_secs_f64(),
            st.1,
            st.2,
            st.3,
            st.4
        );
        let (_, after) = glot_server::edge::seo_report::readiness_pages(&state, &p)
            .await
            .unwrap();
        let paths: Vec<String> = sqlx::query_scalar(
            "SELECT path FROM edge_page_segments WHERE project_id = $1 GROUP BY path ORDER BY path",
        )
        .bind(p.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        eprintln!("REAL {slug} after: {} pages known to glot", paths.len());
        show("after", &after);

        // What each page is served with now (proxy in-process, origin = the real site, GET only).
        let (host, locale) = &hosts[0];
        let (mut ready, mut pending) = (0, 0);
        for path in paths.iter().take(40) {
            let req = Request::get(format!("/edge/proxy{path}"))
                .header(header::HOST, host)
                .header(header::ACCEPT, "text/html")
                .body(Body::empty())
                .unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            let (s, h): (StatusCode, HeaderMap) = (res.status(), res.headers().clone());
            let body = res.into_body().collect().await.unwrap().to_bytes();
            let body = String::from_utf8_lossy(&body);
            let canonical = body
                .split(r#"<link rel="canonical" href=""#)
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or("-");
            let v = |k: &str| {
                h.get(k)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string()
            };
            match v("x-glot-index").as_str() {
                "ready" => ready += 1,
                "pending" => pending += 1,
                _ => {}
            }
            eprintln!(
                "REAL {slug} {locale} {path}: {s} {} coverage {} index {} canonical {canonical} hreflang {}",
                v("x-glot-edge"),
                v("x-glot-coverage"),
                v("x-glot-index"),
                body.matches("hreflang=").count()
            );
        }
        eprintln!(
            "REAL {slug} {locale}: {ready} pages advertised, {pending} declared copies of the source"
        );
    }
    pool.close().await;
}
