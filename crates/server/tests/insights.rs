//! Insights: ingestion, aggregation, anonymization, experiments (real Postgres, skipped when unreachable).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn insights_end_to_end() {
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
    let app = glot_server::router(
        glot_server::AppState::new(pool.clone(), "test-secret", false),
        "http://localhost:3100",
    );

    let call = |method: Method,
                uri: &str,
                cookie: Option<&str>,
                body: Option<Value>,
                extra: &[(&str, &str)]| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
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

    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "i@x.io", "password": "correct horse", "name": "I"})),
        &[("content-type", "application/json")],
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
    let ct = &[("content-type", "application/json")][..];
    let (s, _, _) = call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "D", "slug": "demo", "source_locale": "en", "target_locales": ["fr"]})),
        ct,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _, _) = call(
        Method::POST,
        "/api/projects/demo/keys",
        c,
        Some(json!({"key": "a.b", "source": "Hello world"})),
        ct,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let base = "/api/projects/demo/insights";
    let (s, _, o) = call(Method::GET, base, c, None, &[]).await;
    assert_eq!(s, StatusCode::OK);
    let key = o["public_key"].as_str().unwrap().to_string();
    assert!(key.starts_with("pk_"));
    assert_eq!(o["total_visits"], 0);

    let hit = format!("/api/insights/{key}");
    // text/plain body (what the snippet sends), PII-looking headers that must never be stored.
    let send = |body: Value, extra: Vec<(&'static str, &'static str)>| {
        let hit = hit.clone();
        let call = &call;
        async move {
            let mut h = vec![
                ("content-type", "text/plain;charset=UTF-8"),
                ("x-forwarded-for", "203.0.113.77"),
                ("user-agent", "SecretBrowser/1.0"),
            ];
            h.extend(extra);
            call(Method::POST, &hit, None, Some(body), &h).await
        }
    };
    for _ in 0..6 {
        let (s, h, r) = send(
            json!({"langs": ["de-AT", "en"], "served": "en", "route": "/pricing?email=a@b.c"}),
            vec![("cf-ipcountry", "AT")],
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(r, json!({"locale": null, "arm": null}));
        assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }
    for _ in 0..3 {
        send(json!({"langs": ["en-US"], "served": "en"}), vec![]).await;
    }
    send(json!({"langs": ["fr-FR"], "served": "fr"}), vec![]).await;
    send(
        json!({"langs": ["en-US"], "served": "en", "event": "signup"}),
        vec![],
    )
    .await;
    send(
        json!({"langs": ["fr"], "served": "fr", "event": "signup"}),
        vec![],
    )
    .await;
    // Accept-Language fallback when the body has no usable language.
    send(
        json!({"langs": [], "served": "en"}),
        vec![("accept-language", "ja-JP,ja;q=0.9")],
    )
    .await;

    // Validation & unknown key.
    let (s, _, _) = send(json!({"served": "../x"}), vec![]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = send(json!({"served": "en", "event": "<script>"}), vec![]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(
        Method::POST,
        "/api/insights/pk_nope",
        None,
        Some(json!({"served": "en"})),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Aggregation: one row per (day, preferred, served, country, arm, event), counters only.
    let rows: Vec<(String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT preferred, served, country, arm, event, n FROM insight_daily ORDER BY preferred, event",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(rows.contains(&(
        "de".into(),
        "en".into(),
        "AT".into(),
        "".into(),
        "$view".into(),
        6
    )));
    assert!(rows.contains(&(
        "ja".into(),
        "en".into(),
        "".into(),
        "".into(),
        "$view".into(),
        1
    )));
    // Anonymization: nothing identifying anywhere in the insight tables.
    let dump: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(t::text, '|'), '') FROM (SELECT * FROM insight_daily) t",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for needle in ["203.0.113", "SecretBrowser", "pricing", "a@b.c"] {
        assert!(!dump.contains(needle), "{needle} leaked into storage");
    }

    let (_, _, o) = call(Method::GET, &format!("{base}?goal=signup"), c, None, &[]).await;
    assert_eq!(o["total_visits"], 11);
    let de = &o["demand"][0];
    assert_eq!(de["language"], "de");
    assert_eq!(de["offered"], false);
    assert_eq!(de["visits"], 6);
    assert_eq!(de["top_countries"][0], json!(["AT", 6]));
    assert!(de["est_cost_usd"].as_f64().unwrap() > 0.0);
    // Matched visitors convert at 2/4, German-preferring at 0/6 → 3 extra conversions over 30 days.
    assert!((de["est_extra_conversions"].as_f64().unwrap() - 3.0).abs() < 1e-9);
    let en = o["funnel"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["locale"] == "en")
        .unwrap();
    assert_eq!(en["is_source"], true);
    assert_eq!(en["visits"], 10);
    assert_eq!(en["conversions"], 1);
    assert_eq!(o["events"], json!(["signup"]));

    // Experiments: de at 100 % → every German visitor lands in the test arm and gets `de`.
    let (s, _, _) = call(
        Method::POST,
        &format!("{base}/experiments"),
        c,
        Some(json!({"locale": "en-GB", "traffic_pct": 50, "days": 14})),
        ct,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "source language");
    let (s, _, e) = call(
        Method::POST,
        &format!("{base}/experiments"),
        c,
        Some(json!({"locale": "de", "traffic_pct": 100, "days": 14})),
        ct,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(e["verdict"], "collecting");
    let exp_id = e["id"].as_str().unwrap().to_string();
    let (s, _, _) = call(
        Method::POST,
        &format!("{base}/experiments"),
        c,
        Some(json!({"locale": "de", "traffic_pct": 10, "days": 14})),
        ct,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, _, p) = call(Method::GET, "/api/projects/demo", c, None, &[]).await;
    assert_eq!(p["target_locales"], json!(["fr", "de"]));

    let (_, _, r) = send(json!({"langs": ["de"], "served": "en"}), vec![]).await;
    assert_eq!(r, json!({"locale": "de", "arm": "test"}));
    send(
        json!({"langs": ["de"], "served": "de", "event": "signup", "arm": "test"}),
        vec![],
    )
    .await;
    let (_, _, r) = send(
        json!({"langs": ["de"], "served": "en", "arm": "control"}),
        vec![],
    )
    .await;
    assert_eq!(r, json!({"locale": null, "arm": "control"}));
    let (_, _, r) = send(json!({"langs": ["fr"], "served": "fr"}), vec![]).await;
    assert_eq!(r["arm"], Value::Null, "other languages are not enrolled");

    let (_, _, o) = call(Method::GET, base, c, None, &[]).await;
    let x = &o["experiments"][0];
    assert_eq!(
        x["test"],
        json!({"visits": 1, "conversions": 1, "rate": 1.0})
    );
    assert_eq!(x["control"]["visits"], 1);
    assert_eq!(x["verdict"], "collecting");
    assert_eq!(
        o["demand"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["language"] == "de")
            .unwrap()["offered"],
        true
    );

    // Decide: removed drops the locale; key rotation invalidates the old key.
    let (s, _, _) = call(
        Method::PATCH,
        &format!("{base}/experiments/{exp_id}"),
        c,
        Some(json!({"status": "removed"})),
        ct,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, p) = call(Method::GET, "/api/projects/demo", c, None, &[]).await;
    assert_eq!(p["target_locales"], json!(["fr"]));
    let (s, _, r) = call(Method::POST, &format!("{base}/rotate-key"), c, None, &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_ne!(r["public_key"], key.as_str());
    let (s, _, _) = send(json!({"served": "en"}), vec![]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    drop(pool);
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .ok();
}

/// Two router instances (two AppState, same database) share one per-project budget; old windows are reused, not piled up.
#[tokio::test]
async fn insights_rate_limit_shared_across_instances() {
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
    let project = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO projects (id, slug, name, source_locale, target_locales, llm, insights_rate_per_min)
         VALUES ($1, 'rl', 'RL', 'en', '{}', '{}', 50)",
    )
    .bind(project)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_rl')")
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    let apps: Vec<_> = (0..2)
        .map(|_| {
            glot_server::router(
                glot_server::AppState::new(pool.clone(), "test-secret", false),
                "http://localhost:3100",
            )
        })
        .collect();
    let hit = {
        let apps = apps.clone();
        move |i: usize| {
            let app = apps[i % 2].clone();
            let uri = if i >= 1_000_000 {
                "/api/insights/pk_bench"
            } else {
                "/api/insights/pk_rl"
            };
            async move {
                let req = Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header("content-type", "text/plain")
                    .body(Body::from(r#"{"langs":["de"],"served":"en"}"#))
                    .unwrap();
                let res = app.oneshot(req).await.unwrap();
                let retry = res
                    .headers()
                    .get(header::RETRY_AFTER)
                    .map(|v| v.to_str().unwrap().parse::<u64>().unwrap());
                (res.status(), retry)
            }
        }
    };
    // Stay clear of a minute boundary so the burst lands in one window.
    let left: f64 = sqlx::query_scalar("SELECT 60 - extract(second FROM now())::float8")
        .fetch_one(&pool)
        .await
        .unwrap();
    if left < 15.0 {
        tokio::time::sleep(std::time::Duration::from_secs_f64(left + 1.0)).await;
    }

    // 200 concurrent hits split across both instances: exactly the budget (50) goes through.
    let mut set = tokio::task::JoinSet::new();
    for i in 0..200 {
        set.spawn(hit.clone()(i));
    }
    let results = set.join_all().await;
    let ok = results.iter().filter(|r| r.0 == StatusCode::OK).count();
    assert_eq!(ok, 50, "{results:?}");
    for (s, retry) in &results {
        if *s != StatusCode::OK {
            assert_eq!(*s, StatusCode::TOO_MANY_REQUESTS);
            assert!((1..=60).contains(&retry.unwrap()), "{retry:?}");
        }
    }
    let n: i64 = sqlx::query_scalar("SELECT sum(n)::int8 FROM insight_daily")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 50);

    // Cleanup: a newer window reuses the project's single row (counter reset), the table never grows.
    // (The local block cache is keyed by public key, so test the reset with a fresh key on the same project.)
    sqlx::query("UPDATE insight_rate SET window_start = window_start - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE insight_keys SET public_key = 'pk_rl2'")
        .execute(&pool)
        .await
        .unwrap();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/insights/pk_rl2")
        .body(Body::from(r#"{"served":"en"}"#))
        .unwrap();
    assert_eq!(
        apps[1].clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
    let rows: Vec<(i32,)> = sqlx::query_as("SELECT hits FROM insight_rate")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows, vec![(1,)]);
    sqlx::query("DELETE FROM projects")
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM insight_rate")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "rows go away with the project");

    // Throughput (informative): accepted path, budget out of reach.
    if std::env::var("GLOT_BENCH").is_ok() {
        sqlx::query(
            "INSERT INTO projects (id, slug, name, source_locale, target_locales, llm, insights_rate_per_min)
             VALUES ($1, 'rl', 'RL', 'en', '{}', '{}', 100000000)",
        )
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO insight_keys (project_id, public_key) VALUES ($1, 'pk_bench')")
            .bind(project)
            .execute(&pool)
            .await
            .unwrap();
        let t = std::time::Instant::now();
        let n = 20_000;
        let mut set = tokio::task::JoinSet::new();
        for w in 0..64 {
            let hit = hit.clone();
            set.spawn(async move {
                for i in 0..n / 64 {
                    assert_eq!(hit(1_000_000 + w + i).await.0, StatusCode::OK);
                }
            });
        }
        set.join_all().await;
        eprintln!(
            "accepted: {:.0} hits/s",
            n as f64 / t.elapsed().as_secs_f64()
        );
        sqlx::query("UPDATE projects SET insights_rate_per_min = 1")
            .execute(&pool)
            .await
            .unwrap();
        let t = std::time::Instant::now();
        let mut set = tokio::task::JoinSet::new();
        for w in 0..64 {
            let hit = hit.clone();
            set.spawn(async move {
                for i in 0..n / 64 {
                    hit(1_000_000 + w + i).await;
                }
            });
        }
        set.join_all().await;
        eprintln!(
            "rejected: {:.0} hits/s",
            n as f64 / t.elapsed().as_secs_f64()
        );
    }

    drop(pool);
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .ok();
}
