//! Job queue: short/urgent kinds before long `translate`, one running job per project and kind family,
//! the fast lane never takes long jobs, and `queue_position` follows claim order.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable. No worker runs: `claim` is driven by hand.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn priority_fairness_and_queue_position() {
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
    let app = glot_server::router(state, "http://localhost:3100");
    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        let req = req
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                headers,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "q@x.io", "password": "correct horse", "name": "Q"})),
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
    let (s, _, p) = call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "A", "slug": "qa", "source_locale": "en", "target_locales": ["es"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let a: Uuid = p["id"].as_str().unwrap().parse().unwrap();
    let mut other = vec![];
    for slug in ["qb", "qc"] {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO projects (id, slug, name, source_locale, target_locales, llm) VALUES ($1, $2, $2, 'en', '{de}', '{}')",
        )
        .bind(id)
        .bind(slug)
        .execute(&db)
        .await
        .unwrap();
        other.push(id);
    }
    let (b, cc) = (other[0], other[1]);

    // Oldest first: the big translates were queued before the urgent jobs.
    let mut ids = vec![];
    for (i, (project, kind)) in [
        (a, "translate"),
        (b, "translate"),
        (a, "edge_translate"),
        (a, "edge_translate"),
        (cc, "github_localize"),
        (cc, "github_update"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO jobs (id, project_id, kind, status, created_at) VALUES ($1, $2, $3, 'queued', now() - make_interval(mins => 60 - $4))",
        )
        .bind(id)
        .bind(project)
        .bind(kind)
        .bind(i as i32)
        .execute(&db)
        .await
        .unwrap();
        ids.push(id);
    }
    let [t_a, t_b, e1, e2, g1, g2] = ids[..] else {
        unreachable!()
    };

    let positions = |c: Option<&str>| {
        let call = &call;
        let c = c.map(str::to_string);
        async move {
            let (s, _, list) = call(Method::GET, "/api/projects/qa/jobs", c.as_deref(), None).await;
            assert_eq!(s, StatusCode::OK);
            list.as_array()
                .unwrap()
                .iter()
                .map(|j| {
                    (
                        j["id"].as_str().unwrap().parse::<Uuid>().unwrap(),
                        j["queue_position"].as_i64(),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>()
        }
    };
    // Global claim order: e1, e2, g1, g2 (urgent, FIFO), then t_a, t_b. Project A's view shows its own jobs.
    let pos = positions(c).await;
    assert_eq!(pos[&e1], Some(1));
    assert_eq!(pos[&e2], Some(2));
    assert_eq!(pos[&t_a], Some(5));
    let (_, _, one) = call(Method::GET, &format!("/api/jobs/{e2}"), c, None).await;
    assert_eq!(one["queue_position"], 2);

    let claim = |fast| {
        let db = db.clone();
        async move {
            glot_server::jobs::claim(&db, fast)
                .await
                .unwrap()
                .map(|r| glot_server::jobs::Job::from(&r).id)
        }
    };
    // Fast lane: urgent edge job first; A's second edge job (another locale) only once nothing else is
    // claimable, i.e. right away here (GitHub and translate are long); then nothing.
    assert_eq!(claim(true).await, Some(e1));
    assert_eq!(
        claim(true).await,
        Some(e2),
        "idle worker: A's next edge job, not left waiting"
    );
    assert_eq!(claim(true).await, None);
    // Regular workers: GitHub before translate; C's second GitHub job waits for its first; A's translate runs
    // alongside A's edge job (other family); then B's translate.
    assert_eq!(claim(false).await, Some(g1));
    assert_eq!(claim(false).await, Some(t_a));
    assert_eq!(claim(false).await, Some(t_b));
    assert_eq!(
        claim(false).await,
        None,
        "g2 waits for its repository's running job"
    );

    let pos = positions(c).await;
    assert_eq!(pos[&e1], None, "running: no position");
    assert_eq!(pos[&t_a], None);

    sqlx::query("UPDATE jobs SET status = 'failed' WHERE id = $1")
        .bind(g1)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(claim(false).await, Some(g2));

    drop(db);
    let _ = sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
