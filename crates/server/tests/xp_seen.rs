//! "Seen" exposures on real Postgres: a client beacon `{seen: true, visible_ms}` counts `$seen` (not `$exposure`),
//! results report `seen` / mean `visible_ms` per arm, and `exposure = 'seen'` makes them the stats denominator.
use glot_server::experiments::{
    Exposure,
    ingest::{XpHit, record},
    rebalance::rebalance,
};
use sqlx::{PgPool, postgres::PgConnectOptions};
use uuid::Uuid;

#[tokio::test]
async fn seen_beacons_counted_reported_and_used_as_denominator() {
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
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO xp_experiments (id, project_id, name, status, allocation, min_n, started_at, exposure)
         VALUES ($1, $2, 'seen', 'running', 'fixed', 500, now() - interval '10 days', 'seen')",
    )
    .bind(id)
    .bind(project)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO xp_arms (experiment_id, arm, intent) VALUES ($1, 0, 'official'), ($1, 1, 'b')",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO xp_cells (experiment_id, cell, weights) VALUES ($1, '*', '{0.5,0.5}')",
    )
    .bind(id)
    .execute(&db)
    .await
    .unwrap();

    // Ingest: one render exposure + one seen beacon (visible_ms clamped to 60 s).
    let x = |seen: bool, visible_ms: Option<f64>| Exposure {
        id,
        cell: "*".into(),
        arm: 1,
        seen,
        visible_ms,
    };
    let hits = vec![
        XpHit {
            served: "en".into(),
            exposures: vec![x(false, None)],
            ..Default::default()
        },
        XpHit {
            served: "en".into(),
            exposures: vec![x(true, Some(5000.0))],
            ..Default::default()
        },
        XpHit {
            served: "en".into(),
            exposures: vec![x(true, Some(9e9))],
            ..Default::default()
        },
    ];
    assert_eq!(record(&state, project, hits).await.unwrap(), (3, 0));
    let rows: Vec<(String, i64, f64)> = sqlx::query_as(
        "SELECT event, n, value_sum FROM xp_counts WHERE experiment_id = $1 ORDER BY event",
    )
    .bind(id)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![("$exposure".into(), 1, 0.0), ("$seen".into(), 2, 65_000.0)]
    );
    sqlx::query("DELETE FROM xp_counts")
        .execute(&db)
        .await
        .unwrap();

    // 10 days: 1000 rendered / 500 seen / 50 signups per arm per day ⇒ seen-denominator rate = 10 %, not 5 %.
    for arm in [0, 1] {
        for (event, n, ms) in [
            ("$exposure", 1000, 0.0),
            ("$seen", 500, 500.0 * 2000.0),
            ("signup", 50, 0.0),
        ] {
            sqlx::query(
                "INSERT INTO xp_counts (experiment_id, cell, arm, day, event, n, value_sum)
                 SELECT $1, '*', $2, (now() AT TIME ZONE 'utc')::date - d, $3, $4, $5 FROM generate_series(0, 9) d",
            )
            .bind(id)
            .bind(arm)
            .bind(event)
            .bind(n)
            .bind(ms)
            .execute(&db)
            .await
            .unwrap();
        }
    }
    let a = rebalance(&state, id).await.unwrap().unwrap();
    assert!(!a.paused, "SRM must use the seen series too: {a:?}");
    for arm in &a.global.arms {
        assert!((arm.mean - 0.1).abs() < 0.01, "{arm:?}");
        assert_eq!(arm.seen, Some(5000));
        assert_eq!(arm.visible_ms, Some(2000.0));
    }
    assert_eq!(a.cells[0].views, 10_000);

    // Same counts, `render` denominator: 5 %, `seen` still reported.
    sqlx::query("UPDATE xp_experiments SET exposure = 'render' WHERE id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let a = rebalance(&state, id).await.unwrap().unwrap();
    assert!(
        (a.global.arms[1].mean - 0.05).abs() < 0.01,
        "{:?}",
        a.global.arms[1]
    );
    assert_eq!(a.global.arms[1].seen, Some(5000));
    assert_eq!(a.cells[0].views, 20_000);

    drop(db);
    let _ = sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
