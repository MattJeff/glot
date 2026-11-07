//! `GET /api/projects/{slug}/insights/recommendations` on fixture aggregates (real Postgres, skipped when unreachable).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn recommendations_from_aggregates() {
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
    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
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
        Some(json!({"email": "r@x.io", "password": "correct horse", "name": "R"})),
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
        Some(json!({"name": "R", "slug": "reco", "source_locale": "en", "target_locales": ["de"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    for i in 0..4 {
        call(
            Method::POST,
            "/api/projects/reco/keys",
            c,
            Some(json!({"key": format!("k.{i}"), "source": "Create your free account today"})),
        )
        .await;
    }
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'reco'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Nothing collected yet: empty list, nothing invented.
    let (s, _, r) = call(
        Method::GET,
        "/api/projects/reco/insights/recommendations",
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["total_visits"], 0);
    assert_eq!(r["conversions_tracked"], false);
    assert_eq!(r["recommendations"], json!([]));

    // Fixture aggregates, as `@glot/insights` would have written them (3 and 20 days ago).
    let fixture = |day: i32, pref: &str, served: &str, event: &str, n: i64| {
        sqlx::query(
            "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
             VALUES ($1, (now() AT TIME ZONE 'utc')::date - $2, $3, $4, '', '', $5, $6)",
        )
        .bind(pid)
        .bind(day)
        .bind(pref.to_string())
        .bind(served.to_string())
        .bind(event.to_string())
        .bind(n)
        .execute(&pool)
    };
    for (day, pref, served, event, n) in [
        (3, "en", "en", "$view", 1000),
        (3, "en", "en", "signup", 50),
        (20, "de", "de", "$view", 200),
        (3, "de", "de", "$view", 200),
        (3, "de", "de", "signup", 24),
        (20, "es", "en", "$view", 150),
        (3, "es", "en", "$view", 450),
        (3, "es", "en", "signup", 12),
        (3, "ja", "en", "$view", 8),
    ] {
        fixture(day, pref, served, event, n).await.unwrap();
    }
    let (s, _, r) = call(
        Method::GET,
        "/api/projects/reco/insights/recommendations?days=30",
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["total_visits"], 2008);
    assert_eq!(r["conversions_tracked"], true);
    let list = r["recommendations"].as_array().unwrap();
    // de is offered but untranslated (0 of 4 keys): a candidate too, behind es.
    let locales: Vec<&str> = list.iter().map(|x| x["locale"].as_str().unwrap()).collect();
    assert_eq!(locales, ["es", "de", "ja"], "{r}");
    let es = &list[0];
    assert_eq!(es["kind"], "missing");
    assert_eq!(es["served_now"], "en");
    assert_eq!(es["visits"], 600);
    assert_eq!(es["keys_to_translate"], 4);
    assert!(es["cost_usd"].as_f64().unwrap() > 0.0, "{es}");
    // Native baseline = de (6 %) since it has ≥ 100 visits; es converts at 2 % on English.
    assert_eq!(es["native"]["visits"], 400);
    assert!(es["gain_monthly"]["mid"].as_f64().unwrap() > 15.0, "{es}");
    assert!(es["gain_monthly"]["low"].as_f64().unwrap() > 0.0, "{es}");
    assert_eq!(es["confidence"], "solid");
    assert!(
        es["reasons"].as_array().unwrap().contains(&json!("rising")),
        "{es}"
    );
    assert_eq!(list[1]["kind"], "under_translated");
    assert_eq!(list[2]["confidence"], "insufficient");

    // Another goal that is never tracked: ranked on demand only, said so.
    let (_, _, r) = call(
        Method::GET,
        "/api/projects/reco/insights/recommendations?goal=purchase",
        c,
        None,
    )
    .await;
    assert_eq!(r["conversions_tracked"], false);
    assert!(r["recommendations"][0]["gain_monthly"].is_null());
    assert!(
        r["recommendations"][0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("no_conversions"))
    );
    let (s, _, _) = call(
        Method::GET,
        "/api/projects/reco/insights/recommendations?goal=%3Cx%3E",
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // One-click add: target locale + translation job capped at the estimate.
    let (s, _, _) = call(
        Method::PATCH,
        "/api/projects/reco",
        c,
        Some(json!({"target_locales": ["de", "es"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = call(
        Method::POST,
        "/api/projects/reco/jobs",
        c,
        Some(json!({"kind": "translate", "locales": ["es"], "budget_usd": -1})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, j) = call(
        Method::POST,
        "/api/projects/reco/jobs",
        c,
        Some(json!({"kind": "translate", "locales": ["es"], "budget_usd": 0.05})),
    )
    .await;
    assert!(
        s == StatusCode::CREATED || s == StatusCode::PAYMENT_REQUIRED,
        "{s} {j}"
    );
    if s == StatusCode::CREATED {
        let cap: Option<f64> =
            sqlx::query_scalar("SELECT (params->>'budget_usd')::float8 FROM jobs WHERE id = $1")
                .bind(uuid::Uuid::parse_str(j["id"].as_str().unwrap()).unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cap, Some(0.05));
    }

    let (s, _, _) = call(
        Method::GET,
        "/api/projects/reco/insights/recommendations",
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    drop(pool);
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .ok();
}
