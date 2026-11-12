//! "Translated in glot, but my site does not show it": the cliponit incident end to end. A fixture site
//! accepts only 2-3 letter language codes (`?lang=zh`) and ignores `?lang=zh-CN`; glot must serve the catalog
//! under the site's code, notice that the site ignores its own, find the code that works, and say so.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable.
use axum::{
    Router,
    body::Body,
    extract::{Query, Request as AxumRequest},
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::collections::HashMap;
use tower::ServiceExt;

/// The site. Like cliponit: `if (!/^[a-z]{2,3}$/.test(lang)) lang = "fr"`, then a dictionary lookup.
/// `/nolang/…`: same site without `<html lang>`; `/csr/…`: a client-rendered shell, same HTML whatever the code.
async fn site(Query(q): Query<HashMap<String, String>>, req: AxumRequest) -> Response {
    let path = req.uri().path();
    if path.starts_with("/csr") {
        return Html(r#"<html lang="fr"><title>ClipOnIt</title><div id="app"></div><script src="/app.js"></script>"#).into_response();
    }
    let asked = q.get("lang").map(String::as_str).unwrap_or("fr");
    let short = (2..=3).contains(&asked.len()) && asked.bytes().all(|b| b.is_ascii_lowercase());
    let (lang, title, h1, p) = match asked {
        "en" if short => ("en", "Create short videos", "Create and publish", "Pricing"),
        "zh" if short => ("zh", "创建短视频", "创建并发布", "价格"),
        _ => ("fr", "Crée tes vidéos courtes", "Crée et publie", "Tarifs"),
    };
    let attr = if path.starts_with("/nolang") {
        String::new()
    } else {
        format!(r#" lang="{lang}""#)
    };
    Html(format!(
        r#"<!doctype html><html{attr}><head><title>{title}</title><script>window.x = "<b>not text</b>";</script></head>
           <body><h1>{h1}</h1><p>{p}</p><footer>ClipOnIt</footer></body></html>"#
    ))
    .into_response()
}

#[tokio::test]
async fn site_ignores_zh_cn_but_honors_zh() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // The fixture site is on 127.0.0.1. SAFETY: set before any server code reads it.
    unsafe {
        std::env::set_var("GLOT_EDGE_ALLOW_PRIVATE", "1");
    }
    tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(get(site)))
            .await
            .unwrap()
    });

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
    let app = glot_server::router(state, "http://localhost:3100");

    let send = |method: Method,
                uri: &str,
                cookie: Option<&str>,
                extra: &[(&str, &str)],
                body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
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
            let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            (status, headers, json)
        }
    };
    let call = |method: Method, uri: &'static str, cookie: Option<&str>, body: Option<Value>| {
        send(method, uri, cookie, &[], body)
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
        Some(json!({"name": "ClipOnIt", "slug": "clip", "source_locale": "fr", "target_locales": ["en", "zh-CN"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/clip",
        c,
        Some(json!({"public_cdn": true, "llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    call(
        Method::POST,
        "/api/projects/clip/keys",
        c,
        Some(json!({"key": "home.title", "source": "Crée et publie"})),
    )
    .await;
    sqlx::query(
        "INSERT INTO translations (key_id, locale, value, status, origin, source_hash)
         SELECT id, 'en', 'Create and publish', 'machine', 'machine', '' FROM keys
         UNION ALL SELECT id, 'zh-CN', '创建并发布', 'machine', 'machine', '' FROM keys",
    )
    .execute(&pool)
    .await
    .unwrap();
    let preview =
        |pattern: &str| json!({"base_url": base, "url_pattern": pattern, "routes": ["/"]});
    let (s, _, _) = call(
        Method::PUT,
        "/api/projects/clip/preview",
        c,
        Some(preview("{route}?lang={locale}")),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // --- A. the catalog answers under the codes a site may use; exact match first, unknown languages 404.
    for file in [
        "zh-CN",
        "zh",
        "zh-cn",
        "zh_CN",
        "zh-Hans",
        "zh-Hans-CN",
        "ZH",
    ] {
        let (s, _, m) = send(
            Method::GET,
            &format!("/cdn/clip/{file}.json"),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{file}");
        assert_eq!(m["home"]["title"], "创建并发布", "{file}");
    }
    let (_, _, m) = call(Method::GET, "/cdn/clip/en-GB.json", None, None).await;
    assert_eq!(m["home"]["title"], "Create and publish");
    let (_, _, m) = call(Method::GET, "/cdn/clip/FR.json", None, None).await;
    assert_eq!(m["home"]["title"], "Crée et publie");
    for file in ["de", "zh-TW", "nope"] {
        let (s, _, _) = send(
            Method::GET,
            &format!("/cdn/clip/{file}.json"),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{file}");
    }
    // An alias never starts an on-demand translation, even when on-demand is open to every locale.
    sqlx::query("UPDATE projects SET ondemand_locales = '{*}'")
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, m) = call(Method::GET, "/cdn/clip/zh.json?ondemand=1", None, None).await;
    assert_eq!(
        (s, &m["home"]["title"]),
        (StatusCode::OK, &json!("创建并发布"))
    );
    sqlx::query("UPDATE projects SET ondemand_locales = '{}'")
        .execute(&pool)
        .await
        .unwrap();
    // Edge TM: same resolution.
    let (s, h, _) = call(Method::GET, "/edge/v1/clip/zh/tm", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h[header::ETAG].to_str().unwrap().contains("zh-CN"));
    let (s, _, _) = call(Method::GET, "/edge/v1/clip/de/tm", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // --- B. nothing checked yet.
    let locale = |v: &Value, l: &str| -> Value {
        v["locales"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["locale"] == l)
            .unwrap()
            .clone()
    };
    let (s, _, v) = call(Method::GET, "/api/projects/clip/live-check", c, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(locale(&v, "zh-CN")["state"], "unchecked");
    let (s, _, _) = call(Method::GET, "/api/projects/clip/live-check", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // --- A visitor picks 中文: the switcher goes to ?lang=zh-CN, the page stays French, the widget reports it.
    let beacon = |b: Value, extra: &'static [(&'static str, &'static str)]| {
        send(Method::POST, "/cdn/clip/live-check", None, extra, Some(b))
    };
    let ignored = json!({"locale": "zh-CN", "code": "zh-CN", "html_lang": "fr", "applied": false, "origin": "https://cliponit.app"});
    let (s, h, _) = beacon(ignored.clone(), &[]).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(h.get(header::SET_COOKIE).is_none());
    let (_, _, v) = call(Method::GET, "/api/projects/clip/live-check", c, None).await;
    let zh = locale(&v, "zh-CN");
    assert_eq!(zh["state"], "not_displayed", "{zh}");
    assert_eq!(zh["evidence"], "visitors");
    assert_eq!(
        zh["visitors"],
        json!({"applied": 0, "ignored": 1, "html_lang": "fr", "origin": "https://cliponit.app"})
    );
    // Not probed yet: no code is guessed (only one the site was seen to honor is ever proposed).
    assert!(zh["suggestion"].is_null());
    assert_eq!(locale(&v, "en")["state"], "unchecked");
    // Crawlers, unknown projects and malformed reports are not counted.
    let (s, _, _) = beacon(ignored.clone(), &[("user-agent", "Googlebot/2.1")]).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = send(
        Method::POST,
        "/cdn/nope/live-check",
        None,
        &[],
        Some(ignored.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = beacon(json!({"locale": "zh-CN"}), &[]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let n: i64 = sqlx::query_scalar("SELECT sum(ignored)::int8 FROM live_check_beacons")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // --- The probe: en is honored; zh-CN gets the fallback page; `zh` gets a page that declares lang="zh".
    // Unambiguous → the site code is saved by glot, and the state says so.
    let (s, _, v) = call(Method::POST, "/api/projects/clip/live-check", c, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let en = locale(&v, "en");
    assert_eq!(
        (&en["state"], &en["evidence"]),
        (&json!("live"), &json!("probe")),
        "{en}"
    );
    assert_eq!(en["probe"]["html_lang"], "en");
    assert_eq!(en["probe"]["title"], "Create short videos");
    assert_eq!(en["probe"]["changed"], 0.75); // 3 of 4 texts; the footer is the same
    assert_eq!(en["mapped"], false);
    let zh = locale(&v, "zh-CN");
    assert_eq!(zh["state"], "live", "{zh}");
    assert_eq!(zh["site_code"], "zh");
    assert_eq!(
        (&zh["mapped"], &zh["auto_applied"]),
        (&json!(true), &json!(true))
    );
    assert_eq!(zh["probe"]["code"], "zh");
    assert_eq!(zh["probe"]["url"], format!("{base}/?lang=zh"));
    assert_eq!(zh["visitors"]["ignored"], 0); // what visitors saw under the old code no longer applies
    assert_eq!(v["site_codes"], json!({"zh-CN": "zh"}));

    // locales.json: `site_code` only where it differs; the existing fields are untouched.
    let (_, _, m) = call(Method::GET, "/cdn/clip/locales.json", None, None).await;
    let codes: Vec<(&str, Option<&str>)> = m["locales"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["code"].as_str().unwrap(), l["site_code"].as_str()))
        .collect();
    assert_eq!(codes, [("fr", None), ("en", None), ("zh-CN", Some("zh"))]);
    assert_eq!(m["locales"][2]["name"], "中文");

    // A browser still holding the old manifest reports under the old code: dropped. Under `zh`: counted.
    beacon(ignored.clone(), &[]).await;
    let applied = json!({"locale": "zh-CN", "code": "zh", "html_lang": "zh", "applied": true, "origin": "https://cliponit.app"});
    beacon(applied.clone(), &[]).await;
    let (_, _, v) = call(Method::GET, "/api/projects/clip/live-check", c, None).await;
    let zh = locale(&v, "zh-CN");
    assert_eq!(
        zh["visitors"],
        json!({"applied": 1, "ignored": 0, "html_lang": null, "origin": "https://cliponit.app"})
    );
    assert_eq!(
        (&zh["state"], &zh["evidence"]),
        (&json!("live"), &json!("visitors"))
    );
    // A later probe keeps saying the code was applied by glot.
    let (_, _, v) = call(
        Method::POST,
        "/api/projects/clip/live-check",
        c,
        Some(json!({"locale": "zh-CN"})),
    )
    .await;
    assert_eq!(locale(&v, "zh-CN")["auto_applied"], true);
    assert_eq!(locale(&v, "zh-CN")["probe"]["verdict"], "honored");

    // --- Site codes are validated: one code per locale, project locales only, URL-safe.
    for bad in [
        json!({"zh-CN": "en"}),
        json!({"zh-CN": "EN"}),
        json!({"de": "x"}),
        json!({"zh-CN": "zh cn"}),
        json!({"zh-CN": ""}),
    ] {
        let (s, _, e) = call(
            Method::PUT,
            "/api/projects/clip/site-codes",
            c,
            Some(bad.clone()),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} {e}");
    }
    // Wait for the background re-check a site-code change starts, so it cannot race the next step.
    let settled = |pool: PgPool| async move {
        for _ in 0..100 {
            let n: i64 =
                sqlx::query_scalar("SELECT count(*) FROM live_check_probes WHERE locale = 'zh-CN'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            if n == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("background check did not finish");
    };

    // "Keep glot's code": an explicit choice is never overridden; the alias is proposed, confirmed by the probe.
    let (s, _, _) = call(
        Method::PUT,
        "/api/projects/clip/site-codes",
        c,
        Some(json!({"zh-CN": "zh-CN"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    settled(pool.clone()).await;
    let (_, _, v) = call(Method::POST, "/api/projects/clip/live-check", c, None).await;
    let zh = locale(&v, "zh-CN");
    assert_eq!(zh["state"], "not_displayed", "{zh}");
    assert_eq!(zh["evidence"], "probe");
    assert_eq!(
        (&zh["mapped"], &zh["auto_applied"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(zh["probe"]["verdict"], "ignored");
    assert_eq!(zh["probe"]["html_lang"], "fr"); // what the site answered to ?lang=zh-CN
    assert_eq!(zh["probe"]["changed"], 0.0);
    assert_eq!(zh["probe"]["works_with"], "zh");
    assert_eq!(zh["suggestion"], json!({"code": "zh"}));
    let (_, _, m) = call(Method::GET, "/cdn/clip/locales.json", None, None).await;
    assert!(m["locales"][2].get("site_code").is_none());

    // One click: "use zh on my site".
    let (s, _, _) = call(
        Method::PUT,
        "/api/projects/clip/site-codes",
        c,
        Some(json!({"zh-CN": "zh"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    settled(pool.clone()).await;
    let (_, _, v) = call(Method::GET, "/api/projects/clip/live-check", c, None).await;
    let zh = locale(&v, "zh-CN");
    assert_eq!(
        (&zh["state"], &zh["site_code"]),
        (&json!("live"), &json!("zh")),
        "{zh}"
    );
    assert_eq!(zh["auto_applied"], false); // chosen by the user this time
    assert!(zh["suggestion"].is_null());

    // --- Client-rendered site: the HTML is the same whatever the code. Not "broken": cannot tell.
    call(
        Method::PUT,
        "/api/projects/clip/preview",
        c,
        Some(preview("/csr{route}?lang={locale}")),
    )
    .await;
    call(
        Method::PUT,
        "/api/projects/clip/site-codes",
        c,
        Some(json!({})),
    )
    .await;
    settled(pool.clone()).await;
    let (_, _, v) = call(Method::POST, "/api/projects/clip/live-check", c, None).await;
    for l in ["en", "zh-CN"] {
        let x = locale(&v, l);
        assert_eq!(
            (&x["state"], &x["probe"]["verdict"]),
            (&json!("unknown"), &json!("unknown")),
            "{x}"
        );
        assert!(x["suggestion"].is_null());
    }
    assert_eq!(v["site_codes"], json!({}));
    // …and the visitors decide.
    beacon(ignored.clone(), &[]).await;
    let (_, _, v) = call(Method::GET, "/api/projects/clip/live-check", c, None).await;
    assert_eq!(locale(&v, "zh-CN")["state"], "not_displayed");
    assert!(locale(&v, "zh-CN")["suggestion"].is_null());

    // --- A site that does not declare <html lang>: the text changes with `zh`, weaker evidence → proposed,
    // not applied.
    call(
        Method::PUT,
        "/api/projects/clip/preview",
        c,
        Some(preview("/nolang{route}?lang={locale}")),
    )
    .await;
    let (_, _, v) = call(
        Method::POST,
        "/api/projects/clip/live-check",
        c,
        Some(json!({"locale": "zh-CN"})),
    )
    .await;
    let zh = locale(&v, "zh-CN");
    assert_eq!(zh["probe"]["verdict"], "ignored", "{zh}");
    assert_eq!(zh["probe"]["works_with"], "zh");
    assert_eq!(zh["suggestion"], json!({"code": "zh"}));
    assert_eq!(v["site_codes"], json!({}));
    let (s, _, e) = call(
        Method::POST,
        "/api/projects/clip/live-check",
        c,
        Some(json!({"locale": "de"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");

    // --- With a public site URL set, reports from other origins are dropped (the browser's Origin header wins).
    sqlx::query("UPDATE preview_settings SET base_url = 'https://cliponit.app'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM live_check_beacons")
        .execute(&pool)
        .await
        .unwrap();
    beacon(ignored.clone(), &[("origin", "https://evil.example")]).await;
    beacon(json!({"locale": "zh-CN", "applied": false}), &[]).await; // no origin at all
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM live_check_beacons")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    beacon(ignored.clone(), &[("origin", "https://www.cliponit.app")]).await;
    // Only the language and the origin are kept: nothing else of the request.
    let row: (String, i64, i64, String, String) = sqlx::query_as(
        "SELECT locale, applied, ignored, html_lang, origin FROM live_check_beacons",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "zh-CN".into(),
            0,
            1,
            "fr".into(),
            "https://www.cliponit.app".into()
        )
    );
    // Daily cap: past it, reports are no-ops.
    sqlx::query("UPDATE live_check_beacons SET ignored = 1000")
        .execute(&pool)
        .await
        .unwrap();
    beacon(ignored, &[("origin", "https://cliponit.app")]).await;
    let n: i64 = sqlx::query_scalar("SELECT ignored FROM live_check_beacons")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1000);

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// Real cases, no simulated data: a copy of the scrubbed `glot_real` database (cliponit with zh-CN, orizn with
/// nl) and the real public site https://cliponit.app. Network + local DB: `GLOT_REAL=1 cargo test --test
/// live_check real_cases -- --ignored --nocapture`. Read-only on `glot_real` (`createdb -T`), GETs only on the site.
#[tokio::test]
#[ignore]
async fn real_cases() {
    if std::env::var("GLOT_REAL").as_deref() != Ok("1") {
        return;
    }
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let admin = PgPool::connect(&url).await.unwrap();
    let db = format!("glot_real_lc_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db} TEMPLATE glot_real"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state, "http://localhost:3100");
    let call = |method: Method, uri: String, cookie: Option<String>, body: Option<Value>| {
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
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            (status, headers, json)
        }
    };
    // A member of both projects in the copy (the scrubbed users have no known password).
    let email = format!("lc-{}@test.invalid", uuid::Uuid::new_v4().simple());
    let (s, h, e) = call(
        Method::POST,
        "/api/auth/signup".into(),
        None,
        Some(json!({"email": email, "password": "correct horse", "name": "LC"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{e}");
    let cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role)
         SELECT p.id, u.id, 'admin' FROM projects p, users u WHERE p.slug IN ('cliponit', 'orizn') AND u.email = $1",
    )
    .bind(&email)
    .execute(&pool)
    .await
    .unwrap();
    let c = || Some(cookie.clone());

    // A. The catalog the site's own code would ask for (`zh`) is the real zh-CN catalog.
    let (s, _, zh) = call(Method::GET, "/cdn/cliponit/zh.json".into(), None, None).await;
    let (_, _, zh_cn) = call(Method::GET, "/cdn/cliponit/zh-CN.json".into(), None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(zh.as_object().is_some_and(|o| !o.is_empty()));
    assert_eq!(zh, zh_cn);
    let (s, _, nl) = call(Method::GET, "/cdn/orizn/nl-NL.json".into(), None, None).await;
    assert_eq!(s, StatusCode::OK, "nl-NL → orizn's nl");
    assert!(nl.as_object().is_some_and(|o| !o.is_empty()));

    // B. Probe the real site, then read the verdicts.
    let (s, _, v) = call(
        Method::POST,
        "/api/projects/cliponit/live-check".into(),
        c(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    eprintln!(
        "cliponit live-check:\n{}",
        serde_json::to_string_pretty(&v).unwrap()
    );
    let (_, _, m) = call(Method::GET, "/cdn/cliponit/locales.json".into(), None, None).await;
    eprintln!("cliponit locales.json: {m}");
    let (_, _, o) = call(
        Method::GET,
        "/api/projects/orizn/live-check".into(),
        c(),
        None,
    )
    .await;
    eprintln!(
        "orizn live-check:\n{}",
        serde_json::to_string_pretty(&o).unwrap()
    );
    // Every target gets a state; none is claimed live without evidence.
    for l in v["locales"].as_array().unwrap() {
        let st = l["state"].as_str().unwrap();
        assert!(["live", "not_displayed", "unknown"].contains(&st), "{l}");
        if st == "live" {
            assert!(l["probe"]["verdict"] == "honored", "{l}");
        }
    }
    assert_eq!(o["locales"][0]["state"], "no_site");

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}
