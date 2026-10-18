//! Batch rebalance of segmented experiments on real Postgres (counts written directly into `xp_counts`):
//! weights indexed by arm number, winner per cell, snapshot persisted, SRM pauses.
use sqlx::{PgPool, postgres::PgConnectOptions};
use uuid::Uuid;

async fn experiment(db: &PgPool, project: Uuid, allocation: &str, arms: &[(i32, bool)]) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO xp_experiments (id, project_id, name, status, allocation, min_n, started_at)
         VALUES ($1, $2, $3, 'running', $4, 500, now() - interval '10 days')",
    )
    .bind(id)
    .bind(project)
    .bind(format!("xp {allocation}"))
    .bind(allocation)
    .execute(db)
    .await
    .unwrap();
    for (arm, approved) in arms {
        sqlx::query(
            "INSERT INTO xp_arms (experiment_id, arm, intent, approved) VALUES ($1, $2, 'i', $3)",
        )
        .bind(id)
        .bind(arm)
        .bind(approved)
        .execute(db)
        .await
        .unwrap();
    }
    id
}

/// `days` days of `views` exposures and `conv` goal conversions per (cell, arm).
async fn counts(db: &PgPool, id: Uuid, cell: &str, arm: i32, views: i64, conv: i64) {
    for d in 0..10 {
        for (event, n) in [("$exposure", views), ("signup", conv)] {
            sqlx::query(
                "INSERT INTO xp_counts (experiment_id, cell, arm, day, event, n)
                 VALUES ($1, $2, $3, (now() AT TIME ZONE 'utc')::date - $4::int, $5, $6)",
            )
            .bind(id)
            .bind(cell)
            .bind(arm)
            .bind(d)
            .bind(event)
            .bind(n)
            .execute(db)
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn rebalance_publishes_weights_decides_and_pauses_on_srm() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let name = format!("glot_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let db = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&name))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&db).await.unwrap();
    let state = glot_server::AppState::new(db.clone(), "test-secret", false);
    let project = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO projects (id, slug, name, source_locale, target_locales, llm)
         VALUES ($1, 'xp', 'xp', 'en', '{de}', '{}')",
    )
    .bind(project)
    .execute(&db)
    .await
    .unwrap();

    // Bandit, arms 0, 1, 3 served (2 unapproved), two cells; arm 3 converts at 10 % vs 5 %.
    let a = experiment(
        &db,
        project,
        "bandit",
        &[(0, true), (1, true), (2, false), (3, true)],
    )
    .await;
    for cell in ["*", "seg"] {
        sqlx::query("INSERT INTO xp_cells (experiment_id, cell, weights) VALUES ($1, $2, '{0.34,0.33,0,0.33}')")
            .bind(a)
            .bind(cell)
            .execute(&db)
            .await
            .unwrap();
        for (arm, conv) in [(0, 50), (1, 50), (3, 100)] {
            counts(&db, a, cell, arm, 1000, conv).await;
        }
    }
    let an = glot_server::experiments::rebalance::rebalance(&state, a)
        .await
        .unwrap()
        .unwrap();
    assert!(!an.paused && an.decision_look, "{an:?}");
    assert_eq!(an.everywhere, Some(3));
    let cells: Vec<(String, Vec<f64>, i32, Option<i32>)> = sqlx::query_as(
        "SELECT cell, weights, looks, winner FROM xp_cells WHERE experiment_id = $1 ORDER BY cell",
    )
    .bind(a)
    .fetch_all(&db)
    .await
    .unwrap();
    for (cell, w, looks, winner) in &cells {
        assert_eq!(w.len(), 4, "{cell}: {w:?}");
        assert_eq!(w[2], 0.0);
        assert!(
            (w.iter().sum::<f64>() - 1.0).abs() < 1e-9 && w.iter().all(|x| *x == 0.0 || *x >= 0.1)
        );
        assert_eq!((*looks, *winner), (1, Some(3)));
    }
    // Same day again: weights not republished, winner kept.
    glot_server::experiments::rebalance::rebalance(&state, a)
        .await
        .unwrap();
    let published: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM xp_analyses WHERE experiment_id = $1 AND weights IS NOT NULL",
    )
    .bind(a)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(published, 1);
    let latest: serde_json::Value = sqlx::query_scalar(
        "SELECT analysis FROM xp_analyses WHERE experiment_id = $1 ORDER BY at DESC LIMIT 1",
    )
    .bind(a)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(latest["global"]["arms"].as_array().unwrap().len(), 4);
    assert_eq!(latest["arm_ids"], serde_json::json!([0, 1, 3]));

    // Fixed 50/50 but 10 000 vs 9 000 exposures a day: sample ratio mismatch → paused, no winner.
    let b = experiment(&db, project, "fixed", &[(0, true), (1, true)]).await;
    sqlx::query(
        "INSERT INTO xp_cells (experiment_id, cell, weights) VALUES ($1, '*', '{0.5,0.5}')",
    )
    .bind(b)
    .execute(&db)
    .await
    .unwrap();
    counts(&db, b, "*", 0, 10_000, 500).await;
    counts(&db, b, "*", 1, 9_000, 900).await;
    let an = glot_server::experiments::rebalance::rebalance(&state, b)
        .await
        .unwrap()
        .unwrap();
    assert!(an.paused && !an.decision_look && an.everywhere.is_none());
    let (status, winner): (String, Option<i32>) = sqlx::query_as(
        "SELECT e.status, c.winner FROM xp_experiments e JOIN xp_cells c ON c.experiment_id = e.id WHERE e.id = $1",
    )
    .bind(b)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((status.as_str(), winner), ("paused", None));
    // Paused experiments are no longer analysed.
    assert!(
        glot_server::experiments::rebalance::rebalance(&state, b)
            .await
            .unwrap()
            .is_none()
    );

    db.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await;
}

/// `GET …/xp/{id}/results` computes a live analysis from `xp_counts`: counters show up before any worker tick,
/// and nothing is written (no snapshot, no look spent).
#[tokio::test]
async fn results_are_live_before_any_snapshot() {
    use axum::{
        body::Body,
        http::{Request, header},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let name = format!("glot_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let db = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&name))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&db).await.unwrap();
    let app = glot_server::router(
        glot_server::AppState::new(db.clone(), "test-secret", false),
        "http://localhost:3100",
    );
    let mut cookie = String::new();
    let mut call = async |method: &str, uri: &str, body: serde_json::Value| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !cookie.is_empty() {
            req = req.header(header::COOKIE, format!("glot_session={cookie}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        if let Some(c) = res.headers().get(header::SET_COOKIE) {
            let c = c.to_str().unwrap().split(';').next().unwrap();
            cookie = c.trim_start_matches("glot_session=").to_string();
        }
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
        )
    };
    let body =
        serde_json::json!({"email": "o@example.com", "password": "correct horse", "name": "N"});
    assert!(call("POST", "/api/auth/signup", body).await.0.is_success());
    let body = serde_json::json!({"name": "L", "slug": "live", "source_locale": "en", "target_locales": ["de"]});
    assert!(call("POST", "/api/projects", body).await.0.is_success());
    let project: Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'live'")
        .fetch_one(&db)
        .await
        .unwrap();
    let id = experiment(&db, project, "bandit", &[(0, true), (1, true)]).await;
    sqlx::query(
        "INSERT INTO xp_cells (experiment_id, cell, weights) VALUES ($1, '*', '{0.5,0.5}')",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    counts(&db, id, "*", 0, 30, 2).await;
    counts(&db, id, "*", 1, 40, 3).await;

    let (s, r) = call(
        "GET",
        &format!("/api/projects/live/xp/{id}/results"),
        serde_json::Value::Null,
    )
    .await;
    assert!(s.is_success(), "{s} {r}");
    assert_eq!(r["latest"]["live"], true, "{r}");
    assert_eq!(r["latest"]["global"]["views"], 700, "{r}");
    assert_eq!(
        r["latest"]["global"]["arms"][1]["mean"]
            .as_f64()
            .map(|m| m > 0.0),
        Some(true)
    );
    assert_eq!(r["history"], serde_json::json!([]));
    let (snapshots, looks): (i64, i32) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM xp_analyses), (SELECT looks FROM xp_cells WHERE experiment_id = $1)",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((snapshots, looks), (0, 0));

    db.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await;
}
