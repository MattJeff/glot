//! Segmented experiments, delivery + ingestion (XP3): `GET /cdn/{slug}/experiments.json` (ETag/304, no sensitive
//! fields, bots served the same), `POST /api/insights/{pk}[/batch]` (dedup, bots, validity filters, revenue,
//! shared rate limit, measured throughput). Real Postgres, skipped when unreachable.
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    extra: &[(&str, &str)],
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let req = req
        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (s, h) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (s, h, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn xp_config_and_ingest() {
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
    let app = glot_server::router(
        glot_server::AppState::new(pool.clone(), "test-secret", false),
        "http://localhost:3100",
    );

    let (s, h, _) = call(
        &app,
        Method::POST,
        "/api/auth/signup",
        Some(json!({"email": "x@x.io", "password": "correct horse", "name": "X"})),
        &[],
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
    let c = &[("cookie", cookie.as_str())][..];
    let (s, _, _) = call(
        &app,
        Method::POST,
        "/api/projects",
        Some(json!({"name": "D", "slug": "demo", "source_locale": "en", "target_locales": ["de"]})),
        c,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    for (k, v) in [("home.hero.title", "Hello"), ("home.hero.cta", "Start")] {
        let (s, _, _) = call(
            &app,
            Method::POST,
            "/api/projects/demo/keys",
            Some(json!({"key": k, "source": v})),
            c,
        )
        .await;
        assert!(s.is_success(), "{s}");
    }
    let pid: Uuid = sqlx::query_scalar("UPDATE projects SET public_cdn = true, insights_rate_per_min = 1000000, xp_holdout_pct = 5 RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_xp')")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

    // A running experiment written directly (the REST lifecycle is XP1's test): segment DE + '*', arm 2 unapproved.
    let (xp, seg, hold) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let script = format!(
        "INSERT INTO xp_segments (id, project_id, name, rule) VALUES ('{seg}', '{pid}', 'DE', '{{\"all\":[{{\"attr\":\"country\",\"in\":[\"DE\"]}}]}}');
         INSERT INTO xp_experiments (id, project_id, name, status, locales, cells, goal, guardrails, metric, currency, facts)
           VALUES ('{xp}', '{pid}', 'Secret name', 'running', '{{en,de}}', '{{{seg}}}', 'purchase', '{{refund}}', 'revenue', 'EUR', '[\"2 000 teams\"]');
         INSERT INTO xp_experiments (id, project_id, kind, name, status) VALUES ('{hold}', '{pid}', 'holdout', 'holdout', 'running');
         INSERT INTO xp_keys (experiment_id, project_id, key, source, live) VALUES
           ('{xp}', '{pid}', 'home.hero.title', 'Hello', true), ('{xp}', '{pid}', 'home.hero.cta', 'Start', true);
         INSERT INTO xp_arms (experiment_id, arm, intent, approved) VALUES
           ('{xp}', 0, 'official', true), ('{xp}', 1, 'social proof intent', true), ('{xp}', 2, 'draft intent', false);
         INSERT INTO xp_values VALUES ('{xp}', 1, 'home.hero.title', 'de', 'Hallo, 2 000 Teams'),
           ('{xp}', 1, 'home.hero.title', 'en', 'Hi, 2 000 teams'), ('{xp}', 2, 'home.hero.title', 'de', 'UNAPPROVED');
         INSERT INTO xp_cells (experiment_id, cell, weights) VALUES
           ('{xp}', '{seg}', '{{0.2,0.6,0.2}}'), ('{xp}', '*', '{{0.5,0.5,0}}');"
    );
    sqlx::raw_sql(&script).execute(&pool).await.unwrap();

    // ---------- config ----------
    let cfg_uri = "/cdn/demo/experiments.json";
    let (s, h, cfg) = call(&app, Method::GET, cfg_uri, None, &[]).await;
    assert_eq!(s, StatusCode::OK, "{cfg}");
    assert_eq!(
        h[header::CACHE_CONTROL],
        "public, max-age=60, stale-while-revalidate=600"
    );
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let etag = h[header::ETAG].to_str().unwrap().to_string();
    assert_eq!(cfg["v"], 1);
    assert_eq!(cfg["holdout"], json!({"id": hold, "pct": 5}));
    assert_eq!(cfg["segments"][0]["id"], json!(seg));
    let e = &cfg["experiments"][0];
    assert_eq!(cfg["experiments"].as_array().unwrap().len(), 1);
    assert_eq!(
        e["arms"],
        json!([{"arm": 0}, {"arm": 1, "values": {
        "de": {"home.hero.title": "Hallo, 2 000 Teams"}, "en": {"home.hero.title": "Hi, 2 000 teams"}}}])
    );
    assert_eq!(e["keys"], json!(["home.hero.cta", "home.hero.title"]));
    // Cells in priority order, weights renormalized over approved arms.
    assert_eq!(e["cells"][0]["id"], json!(seg.to_string()));
    let w: Vec<f64> = serde_json::from_value(e["cells"][0]["weights"].clone()).unwrap();
    assert!(
        (w[0] - 0.25).abs() < 1e-9 && (w[1] - 0.75).abs() < 1e-9,
        "{w:?}"
    );
    assert_eq!(
        e["cells"][1],
        json!({"id": "*", "weights": [0.5, 0.5], "count": true})
    );
    let raw = cfg.to_string();
    for secret in ["Secret name", "intent", "UNAPPROVED", "facts"] {
        assert!(!raw.contains(secret), "config leaks {secret}: {raw}");
    }
    // 304 on revalidation; a crawler gets exactly the same config (no cloaking).
    let (s, _, _) = call(
        &app,
        Method::GET,
        cfg_uri,
        None,
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!(s, StatusCode::NOT_MODIFIED);
    let (s, h, _) = call(
        &app,
        Method::GET,
        cfg_uri,
        None,
        &[("user-agent", "Googlebot/2.1")],
    )
    .await;
    assert_eq!(
        (s, h[header::ETAG].to_str().unwrap()),
        (StatusCode::OK, etag.as_str())
    );
    // Private project: hidden from anonymous callers.
    sqlx::query("UPDATE projects SET public_cdn = false")
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, _) = call(&app, Method::GET, cfg_uri, None, &[]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, h, _) = call(&app, Method::GET, cfg_uri, None, c).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "private, no-cache");
    sqlx::query("UPDATE projects SET public_cdn = true")
        .execute(&pool)
        .await
        .unwrap();

    // ---------- ingestion ----------
    let counts = |cell: &str, arm: i32, event: &str| {
        let (pool, cell, event) = (pool.clone(), cell.to_string(), event.to_string());
        async move {
            sqlx::query_as::<_, (i64, f64, f64)>(
                "SELECT coalesce(sum(n), 0)::int8, coalesce(sum(value_sum), 0), coalesce(sum(value_sq), 0)
                 FROM xp_counts WHERE cell = $1 AND arm = $2 AND event = $3 AND experiment_id = $4",
            )
            .bind(cell)
            .bind(arm)
            .bind(event)
            .bind(xp)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let batch_uri = "/api/insights/pk_xp/batch";
    let expo = |i: usize| {
        json!({"served": "de", "dedup": format!("doc-{i:08}"), "exposures": [
            {"id": xp, "cell": "*", "arm": (i % 2) as i32},
            {"id": xp, "cell": "*", "arm": 1},          // same experiment twice in a beacon: counted once
            {"id": hold, "cell": "*", "arm": 1}]})
    };
    // Throughput: 50 batches × 100 beacons (sequential, one connection pool, debug build).
    let t = std::time::Instant::now();
    for b in 0..50 {
        let hits: Vec<Value> = (0..100).map(|i| expo(b * 100 + i)).collect();
        let (s, _, r) = call(&app, Method::POST, batch_uri, Some(json!(hits)), &[]).await;
        assert_eq!((s, &r["accepted"]), (StatusCode::OK, &json!(100)), "{r}");
    }
    let secs = t.elapsed().as_secs_f64();
    println!(
        "xp ingest throughput: {:.0} beacons/s ({:.0} exposures/s) over 5000 beacons",
        5000.0 / secs,
        10000.0 / secs
    );
    assert_eq!(counts("*", 0, "$exposure").await.0, 2500);
    assert_eq!(counts("*", 1, "$exposure").await.0, 2500);
    let hold_n: i64 =
        sqlx::query_scalar("SELECT sum(n)::int8 FROM xp_counts WHERE experiment_id = $1")
            .bind(hold)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(hold_n, 5000);

    // Dedup: the same beacons replayed (retry) are not counted again, nor duplicates inside a batch.
    let mut hits: Vec<Value> = (0..99).map(expo).collect();
    hits.push(expo(4999));
    let (s, _, r) = call(&app, Method::POST, batch_uri, Some(json!(hits)), &[]).await;
    assert_eq!(
        (s, r),
        (StatusCode::OK, json!({"accepted": 0, "duplicates": 100}))
    );
    let fresh = vec![expo(90_000), expo(90_000)];
    let (_, _, r) = call(&app, Method::POST, batch_uri, Some(json!(fresh)), &[]).await;
    assert_eq!(r, json!({"accepted": 1, "duplicates": 1}));
    assert_eq!(counts("*", 0, "$exposure").await.0, 2501);

    // Bots: same 200, nothing counted (batch and single hit).
    let ua = [(
        "user-agent",
        "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
    )];
    let (s, _, r) = call(
        &app,
        Method::POST,
        batch_uri,
        Some(json!([expo(91_000)])),
        &ua,
    )
    .await;
    assert_eq!(
        (s, r),
        (StatusCode::OK, json!({"accepted": 0, "duplicates": 0}))
    );
    let single = json!({"served": "de", "exposures": [{"id": xp, "cell": seg, "arm": 1}]});
    let (s, _, _) = call(
        &app,
        Method::POST,
        "/api/insights/pk_xp",
        Some(single.clone()),
        &ua,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(counts(&seg.to_string(), 1, "$exposure").await.0, 0);
    // A real browser hit counts (and still counts its legacy $view).
    let (s, _, _) = call(&app, Method::POST, "/api/insights/pk_xp", Some(single), &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(counts(&seg.to_string(), 1, "$exposure").await.0, 1);

    // Not counted: unapproved arm, unknown cell, uncovered locale, foreign experiment.
    let bad = json!([
        {"served": "de", "exposures": [{"id": xp, "cell": "*", "arm": 2}]},
        {"served": "de", "exposures": [{"id": xp, "cell": "nope", "arm": 1}]},
        {"served": "fr", "exposures": [{"id": xp, "cell": "*", "arm": 1}]},
        {"served": "de", "exposures": [{"id": Uuid::new_v4(), "cell": "*", "arm": 1}]}]);
    let (s, _, _) = call(&app, Method::POST, batch_uri, Some(bad), &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(counts("*", 2, "$exposure").await.0, 0);
    assert_eq!(counts("nope", 1, "$exposure").await.0, 0);
    assert_eq!(counts("*", 1, "$exposure").await.0, 2500);

    // Conversions: goal with revenue converted to EUR, guardrail counted, other events ignored.
    let conv = json!([
        {"served": "de", "event": "purchase", "value": 108.0, "currency": "USD", "exposures": [{"id": xp, "cell": "*", "arm": 1}]},
        {"served": "de", "event": "purchase", "value": 50.0, "currency": "EUR", "exposures": [{"id": xp, "cell": "*", "arm": 1}]},
        {"served": "de", "event": "refund", "exposures": [{"id": xp, "cell": "*", "arm": 1}]},
        {"served": "de", "event": "newsletter", "exposures": [{"id": xp, "cell": "*", "arm": 1}]}]);
    let (s, _, _) = call(&app, Method::POST, batch_uri, Some(conv), &[]).await;
    assert_eq!(s, StatusCode::OK);
    let (n, sum, sq) = counts("*", 1, "purchase").await;
    assert_eq!(n, 2);
    assert!(
        (sum - 150.0).abs() < 1e-6 && (sq - 12_500.0).abs() < 1e-6,
        "{sum} {sq}"
    );
    assert_eq!(counts("*", 1, "refund").await.0, 1);
    assert_eq!(counts("*", 1, "newsletter").await.0, 0);

    // Validation.
    let (s, _, _) = call(
        &app,
        Method::POST,
        batch_uri,
        Some(json!((0..101).map(expo).collect::<Vec<_>>())),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(&app, Method::POST, batch_uri, Some(json!([{"served": "de", "event": "purchase", "value": -1.0, "exposures": [{"id": xp, "cell": "*", "arm": 1}]}])), &[]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(
        &app,
        Method::POST,
        "/api/insights/pk_nope/batch",
        Some(json!([])),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Shared rate limit: a batch spends one hit per beacon.
    sqlx::query("UPDATE projects SET insights_rate_per_min = 150")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM insight_rate")
        .execute(&pool)
        .await
        .unwrap();
    let hits: Vec<Value> = (0..100).map(|i| expo(95_000 + i)).collect();
    let (s, _, _) = call(&app, Method::POST, batch_uri, Some(json!(hits)), &[]).await;
    assert_eq!(s, StatusCode::OK);
    let hits: Vec<Value> = (100..200).map(|i| expo(95_000 + i)).collect();
    let (s, h, _) = call(&app, Method::POST, batch_uri, Some(json!(hits)), &[]).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(h.contains_key(header::RETRY_AFTER));

    // No IP / UA / visitor id anywhere: counters only.
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns WHERE table_name IN ('xp_counts', 'xp_dedup')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        cols.iter()
            .all(|c| !["ip", "user_agent", "visitor", "route"].contains(&c.as_str())),
        "{cols:?}"
    );

    // A changed source unpublishes the experiment (and changes the ETag).
    sqlx::query("UPDATE keys SET source = 'Hello there' WHERE key = 'home.hero.title'")
        .execute(&pool)
        .await
        .unwrap();
    let (_, h, cfg) = call(&app, Method::GET, cfg_uri, None, &[]).await;
    assert_eq!(cfg["experiments"], json!([]));
    assert_ne!(h[header::ETAG].to_str().unwrap(), etag);

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}
