//! Language launches: holdout assignment, explicit choice, verdict + roll-out, before/after caveat, share tokens
//! (real Postgres, skipped when unreachable).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn language_launch_end_to_end() {
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
    let app = glot_server::router(state.clone(), "http://localhost:3100");

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
        Some(json!({"email": "l@x.io", "password": "correct horse", "name": "L"})),
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
        Some(json!({"name": "Clip", "slug": "clip", "source_locale": "en", "target_locales": ["fr"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::POST,
        "/api/projects/clip/keys",
        c,
        Some(json!({"key": "a.b", "source": "Make a clip from any video"})),
    )
    .await;
    let (_, _, o) = call(Method::GET, "/api/projects/clip/insights", c, None).await;
    let hit = format!("/api/insights/{}", o["public_key"].as_str().unwrap());

    // Before the launch: Spanish speakers served English (the baseline), English speakers served English.
    for (pref, views, conv) in [("es", 300, 6), ("en", 600, 30)] {
        for (event, n) in [("$view", views), ("signup", conv)] {
            sqlx::query(
                "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
                 SELECT id, (now() AT TIME ZONE 'utc')::date - 3, $1, 'en', '', '', $2, $3 FROM projects WHERE slug = 'clip'",
            )
            .bind(pref)
            .bind(event)
            .bind(n as i64)
            .execute(&pool)
            .await
            .unwrap();
        }
    }

    // ---------- holdout launch ----------
    let base = "/api/projects/clip/launches";
    for bad in [
        json!({"locale": "en-GB"}),
        json!({"locale": "es", "holdout_pct": 80}),
        json!({"locale": "es", "mode": "magic"}),
        json!({"locale": "es", "days": 3}),
    ] {
        let (s, _, _) = call(Method::POST, base, c, Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, _, l) = call(
        Method::POST,
        base,
        c,
        Some(json!({"locale": "es", "holdout_pct": 20, "days": 14})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{l}");
    let id = l["id"].as_str().unwrap().to_string();
    assert_eq!(l["mode"], "holdout");
    assert_eq!(l["causal"], true);
    assert_eq!(l["fallback"], "en");
    assert_eq!(l["verdict"], "collecting");
    assert_eq!(l["cost_estimated"], true, "no job yet: dry-run estimate");
    assert!(l["cost_usd"].as_f64().unwrap() > 0.0);
    assert!(l["share_token"].is_null());
    assert_eq!(l["baseline"], json!({"visits": 300, "conversions": 6}));
    assert_eq!(
        l["recommendation"]["language"], "es",
        "glot's recommendation, snapshotted at launch: {l}"
    );
    let (s, _, _) = call(Method::POST, base, c, Some(json!({"locale": "es"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "one measurement per language");
    let (_, _, p) = call(Method::GET, "/api/projects/clip", c, None).await;
    assert!(
        p["target_locales"]
            .as_array()
            .unwrap()
            .contains(&json!("es"))
    );
    let (_, _, o) = call(Method::GET, "/api/projects/clip/insights", c, None).await;
    assert_eq!(
        o["experiments"].as_array().unwrap().len(),
        0,
        "launch holdouts are not legacy experiments"
    );

    // Assignment split: ~80 % get the new locale, ~20 % are held on the fallback (the response says so).
    let (mut test, mut control) = (0, 0);
    for _ in 0..400 {
        let (s, _, r) = call(
            Method::POST,
            &hit,
            None,
            Some(json!({"langs": ["es-MX"], "served": "en"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        match r["arm"].as_str().unwrap() {
            "test" => {
                test += 1;
                assert_eq!(r["locale"], "es");
            }
            "control" => {
                control += 1;
                assert_eq!(r["locale"], "en", "held out: stay on the fallback");
            }
            a => panic!("{a}"),
        }
    }
    let share = test as f64 / 400.0;
    assert!((0.72..=0.88).contains(&share), "test share {share}");
    // Other languages are not in the launch.
    let (_, _, r) = call(
        Method::POST,
        &hit,
        None,
        Some(json!({"langs": ["de"], "served": "en"})),
    )
    .await;
    assert!(r["arm"].is_null() && r["locale"].is_null());

    // Explicit choice: never assigned; a mid-session pick withdraws the session from its arm.
    let (_, _, r) = call(
        Method::POST,
        &hit,
        None,
        Some(json!({"langs": ["es"], "served": "es", "explicit": true})),
    )
    .await;
    assert!(r["arm"].is_null() && r["locale"].is_null(), "{r}");
    let (_, _, r) = call(
        Method::POST,
        &hit,
        None,
        Some(json!({"langs": ["es"], "served": "es", "explicit": true, "arm": "control"})),
    )
    .await;
    assert!(r["arm"].is_null() && r["locale"].is_null(), "{r}");
    // Its later conversion carries no arm.
    call(
        Method::POST,
        &hit,
        None,
        Some(json!({"langs": ["es"], "served": "es", "event": "signup", "explicit": true})),
    )
    .await;
    for arm in ["test", "test", "control"] {
        call(
            Method::POST,
            &hit,
            None,
            Some(json!({"langs": ["es"], "served": "es", "event": "signup", "arm": arm})),
        )
        .await;
    }
    let (s, _, d) = call(Method::GET, &format!("{base}/{id}"), c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(d["explicit"], json!([1, 0]));
    assert_eq!(d["arms"][0]["visits"], control - 1);
    assert_eq!(d["arms"][1]["visits"], test);
    assert_eq!(d["arms"][0]["conversions"], 1);
    assert_eq!(d["arms"][1]["conversions"], 2);
    assert_eq!(d["verdict"], "collecting", "day 0: time guard");
    assert!(d["srm_p"].as_f64().unwrap() > 0.001);

    // Two weeks of a clear effect, then the end date: the worker rolls out and freezes the verdict.
    sqlx::query(
        "UPDATE language_launches SET launched_at = now() - interval '15 days', ends_at = now() - interval '1 hour'
         WHERE id = $1::uuid",
    )
    .bind(&id)
    .execute(&pool)
    .await
    .unwrap();
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'clip'")
        .fetch_one(&pool)
        .await
        .unwrap();
    for (arm, views, conv) in [("control", 2000, 60), ("test", 8000, 360)] {
        for (event, n) in [("$view", views), ("signup", conv)] {
            sqlx::query(
                "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
                 VALUES ($1, (now() AT TIME ZONE 'utc')::date - 7, 'es', 'es', '', $2, $3, $4)",
            )
            .bind(pid)
            .bind(arm)
            .bind(event)
            .bind(n as i64)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    glot_server::launches::tick(&state).await.unwrap();
    let (_, _, d) = call(Method::GET, &format!("{base}/{id}"), c, None).await;
    assert_eq!(d["status"], "rolled_out", "{d}");
    assert_eq!(d["verdict"], "probably_positive");
    assert!(d["lift_lo"].as_f64().unwrap() > 0.0);
    assert!(!d["daily"].as_array().unwrap().is_empty());
    // Rolled out: nobody is held any more.
    let (_, _, r) = call(
        Method::POST,
        &hit,
        None,
        Some(json!({"langs": ["es"], "served": "en"})),
    )
    .await;
    assert!(r["arm"].is_null() && r["locale"].is_null(), "{r}");

    // ---------- share ----------
    let (s, _, t) = call(Method::POST, &format!("{base}/{id}/share"), c, None).await;
    assert_eq!(s, StatusCode::OK);
    let token = t["token"].as_str().unwrap().to_string();
    let (_, _, t2) = call(Method::POST, &format!("{base}/{id}/share"), c, None).await;
    assert_eq!(t2["token"], token.as_str(), "sharing twice keeps the link");
    let public = format!("/api/public/launches/{token}");
    let (s, h, pubd) = call(Method::GET, &public, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert_eq!(pubd["project"], "Clip");
    assert_eq!(pubd["verdict"], "probably_positive");
    for private in ["id", "share_token", "project_id", "slug"] {
        assert!(pubd.get(private).is_none(), "{private} leaked");
    }
    let tampered = format!("{}0", &token[..token.len() - 1]);
    let (s, _, _) = call(
        Method::GET,
        &format!("/api/public/launches/{tampered}"),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call(Method::GET, "/api/public/launches/nope", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call(Method::DELETE, &format!("{base}/{id}/share"), c, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(Method::GET, &public, None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "revoked");
    let (_, _, t3) = call(Method::POST, &format!("{base}/{id}/share"), c, None).await;
    assert_ne!(t3["token"], token.as_str(), "re-sharing draws a new link");
    let (s, _, _) = call(Method::GET, &public, None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the revoked link stays dead");

    // ---------- before/after ----------
    let (s, _, ba) = call(
        Method::POST,
        base,
        c,
        Some(json!({"locale": "de", "mode": "before_after", "days": 7})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{ba}");
    assert_eq!(ba["causal"], false, "before/after is labelled non-causal");
    assert_eq!(ba["holdout_pct"], 0);
    let ba_id = ba["id"].as_str().unwrap().to_string();
    // No holdout: German visitors are never assigned.
    let (_, _, r) = call(
        Method::POST,
        &hit,
        None,
        Some(json!({"langs": ["de"], "served": "de"})),
    )
    .await;
    assert!(r["arm"].is_null());
    sqlx::query(
        "UPDATE language_launches SET launched_at = now() - interval '8 days', ends_at = now() - interval '1 hour'
         WHERE id = $1::uuid",
    )
    .bind(&ba_id)
    .execute(&pool)
    .await
    .unwrap();
    for (ago, views, conv) in [(12, 3000, 90), (3, 3000, 150)] {
        for (event, n) in [("$view", views), ("signup", conv)] {
            sqlx::query(
                "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
                 VALUES ($1, (now() AT TIME ZONE 'utc')::date - $2, 'de', 'en', '', '', $3, $4)",
            )
            .bind(pid)
            .bind(ago)
            .bind(event)
            .bind(n as i64)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    glot_server::launches::tick(&state).await.unwrap();
    let (_, _, ba) = call(Method::GET, &format!("{base}/{ba_id}"), c, None).await;
    assert_eq!(ba["status"], "done", "{ba}");
    assert_eq!(
        ba["verdict"], "higher_after",
        "never 'positive' without a holdout"
    );
    assert_eq!(ba["causal"], false);
    assert_eq!(ba["arms"][0]["visits"], 3000);
    assert_eq!(ba["arms"][1]["conversions"], 150);

    let (_, _, all) = call(Method::GET, base, c, None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .ok();
}
