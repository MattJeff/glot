//! PostHog importer against a local mock of PostHog's Query API (built from https://posthog.com/docs/api/queries:
//! `POST /api/projects/{id}/query/`, `Authorization: Bearer <personal key>`, `{"query": {"kind": "HogQLQuery", …}}`,
//! positional `results`). Proves: explained refusals (project key, 401, missing scope, unknown project), the key is
//! never returned, only aggregate queries are sent, only aggregates are stored, backfill then incremental sync, and
//! the opportunity engine running on the imported history with its source named. Real Postgres (skipped if absent).
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::IntoResponse,
    routing::post,
};
use chrono::{Duration, NaiveDate, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

type Log = Arc<Mutex<Vec<String>>>;

/// Ground truth of the mock: sessions, pageviews, one-page sessions and visitors per (day offset, lang, served,
/// country), and sessions with the conversion event.
/// (day offset, lang, served, country, sessions, pageviews, one-page sessions, visitors).
type PvRow = (
    i64,
    &'static str,
    &'static str,
    &'static str,
    i64,
    i64,
    i64,
    i64,
);
const PV: &[PvRow] = &[
    (1, "en", "", "US", 300, 450, 120, 280),
    (1, "de", "", "DE", 200, 260, 110, 190),
    (1, "nl", "nl", "NL", 150, 300, 30, 140),
    (40, "de", "", "AT", 50, 60, 30, 45),
];
const CONV: &[(i64, &str, &str, &str, i64)] = &[
    (1, "en", "", "US", 15),
    (1, "de", "", "DE", 2),
    (1, "nl", "nl", "NL", 9),
];

/// `timestamp >= toDateTime('A 00:00:00') AND timestamp < toDateTime('B 00:00:00')` → [A, B).
fn window(q: &str) -> Option<(NaiveDate, NaiveDate)> {
    let a = q
        .split("timestamp >= toDateTime('")
        .nth(1)?
        .get(..10)?
        .parse()
        .ok()?;
    let b = q
        .split("timestamp < toDateTime('")
        .nth(1)?
        .get(..10)?
        .parse()
        .ok()?;
    Some((a, b))
}

async fn mock(
    State(log): State<Log>,
    Path(project): Path<String>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let auth = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    match auth {
        "Bearer phx_good" => {}
        "Bearer phx_noscope" => {
            return (
                StatusCode::FORBIDDEN,
                Json(
                    json!({"type": "authentication_error", "code": "permission_denied",
                            "detail": "API key missing required scope 'query:read'"}),
                ),
            );
        }
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(
                    json!({"type": "authentication_error", "code": "authentication_failed",
                            "detail": "Incorrect authentication credentials."}),
                ),
            );
        }
    }
    if project != "4242" {
        return (StatusCode::NOT_FOUND, Json(json!({"detail": "Not found."})));
    }
    assert_eq!(body["query"]["kind"], "HogQLQuery", "{body}");
    let q = body["query"]["query"].as_str().unwrap().to_string();
    log.lock().unwrap().push(q.clone());
    let today = Utc::now().date_naive();
    let day = |off: i64| today - Duration::days(off);
    let inside = |off: i64| window(&q).is_some_and(|(a, b)| a <= day(off) && day(off) < b);
    let results: Vec<Value> = if q.contains("GROUP BY event") {
        vec![
            json!(["$pageview", 960]),
            json!(["$autocapture", 500]),
            json!(["user signed up", 26]),
        ]
    } else if q.contains("AS with_language") {
        vec![json!([960, 950])]
    } else if q.contains("event = '$pageview'") {
        PV.iter()
            .filter(|r| inside(r.0))
            .map(|r| json!([day(r.0).to_string(), r.1, r.2, r.3, r.4, r.5, r.6, r.7]))
            .collect()
    } else if q.contains("event = 'user signed up'") {
        CONV.iter()
            .filter(|r| inside(r.0))
            .map(|r| json!([day(r.0).to_string(), r.1, r.2, r.3, r.4]))
            .collect()
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"type": "validation_error", "detail": "unexpected query"})),
        );
    };
    (
        StatusCode::OK,
        Json(json!({"results": results, "is_cached": false})),
    )
}

#[tokio::test]
async fn posthog_import_end_to_end() {
    // The mock listens on 127.0.0.1: the SSRF guard allows private addresses only under this flag (tests only).
    unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
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

    let log: Log = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ph = format!("http://{}", listener.local_addr().unwrap());
    let mock_app = Router::new()
        .route("/api/projects/{id}/query/", post(mock))
        .with_state(log.clone());
    tokio::spawn(async move { axum::serve(listener, mock_app).await.unwrap() });

    let mut cookie = String::new();
    let call = |method: Method, uri: String, body: Option<Value>, cookie: String| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header(header::COOKIE, cookie)
                .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let text = String::from_utf8_lossy(&bytes).to_string();
            (
                status,
                headers,
                serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
                text,
            )
        }
    };
    let (_, h, _, _) = call(
        Method::POST,
        "/api/auth/signup".into(),
        Some(json!({"email": "p@x.io", "password": "correct horse", "name": "P"})),
        cookie.clone(),
    )
    .await;
    cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let c = |m: Method, u: &str, b: Option<Value>| call(m, u.to_string(), b, cookie.clone());
    let (s, ..) = c(
        Method::POST,
        "/api/projects",
        Some(json!({"name": "Visa", "slug": "visa", "source_locale": "en", "target_locales": ["nl"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let base = "/api/projects/visa/autopilot/posthog";

    // Not connected: an honest empty state, and Autopilot reads the glot tag (nothing yet).
    let (_, _, v, _) = c(Method::GET, base, None).await;
    assert_eq!(v["connected"], false);
    let (_, _, o, _) = c(Method::GET, "/api/projects/visa/autopilot", None).await;
    assert_eq!(
        (o["source"]["kind"].as_str(), o["total_visits"].as_i64()),
        (Some("insights"), Some(0))
    );

    // Every refusal says what to fix.
    let conn =
        |key: &str, project: &str| json!({"host": ph, "project_id": project, "api_key": key});
    let (s, _, e, _) = c(
        Method::POST,
        &format!("{base}/test"),
        Some(conn("phc_public", "4242")),
    )
    .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("posthog_project_key"))
    );
    let (s, _, e, _) = c(
        Method::POST,
        &format!("{base}/test"),
        Some(conn("phx_wrong", "4242")),
    )
    .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("posthog_unauthorized"))
    );
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("eu.posthog.com"),
        "{e}"
    );
    let (_, _, e, _) = c(
        Method::POST,
        &format!("{base}/test"),
        Some(conn("phx_noscope", "4242")),
    )
    .await;
    assert_eq!(e["error"]["code"], "posthog_scope");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("query:read")
    );
    let (_, _, e, _) = c(
        Method::POST,
        &format!("{base}/test"),
        Some(conn("phx_good", "9")),
    )
    .await;
    assert_eq!(e["error"]["code"], "posthog_project");
    let (_, _, e, _) = c(
        Method::POST,
        &format!("{base}/test"),
        Some(json!({"host": "ftp://x", "project_id": "1", "api_key": "phx_good"})),
    )
    .await;
    assert_eq!(e["error"]["code"], "validation_error");
    // A failed save stores nothing.
    let (s, ..) = c(Method::PUT, base, Some(conn("phx_noscope", "4242"))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM autopilot_posthog")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);

    // Test connection: the project's real event names, a suggested conversion, language coverage.
    let (s, _, t, _) = c(
        Method::POST,
        &format!("{base}/test"),
        Some(conn("phx_good", "4242")),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["suggested_event"], "user signed up");
    assert_eq!(
        (t["pageviews_30d"].as_i64(), t["with_language_30d"].as_i64()),
        (Some(960), Some(950))
    );
    assert!(
        t["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e[0] == "$autocapture")
    );

    // Save: the key is encrypted at rest and never comes back.
    let mut b = conn("phx_good", "4242");
    b["conversion_event"] = "user signed up".into();
    let (s, _, v, raw) = c(Method::PUT, base, Some(b)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["connected"].as_bool(), v["backfill_days"].as_i64()),
        (Some(true), Some(90))
    );
    assert!(!raw.contains("phx_good"), "key returned: {raw}");
    let enc: Vec<u8> = sqlx::query_scalar("SELECT api_key_enc FROM autopilot_posthog")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !enc.windows(8).any(|w| w == b"phx_good"),
        "key stored in clear"
    );
    let (_, _, _, raw) = c(Method::GET, base, None).await;
    assert!(!raw.contains("phx_good"));

    // Backfill: 90 days in 7-day windows, two queries each (pageviews, conversions).
    log.lock().unwrap().clear();
    let (s, _, r, _) = c(Method::POST, &format!("{base}/sync"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let queries = log.lock().unwrap().clone();
    assert_eq!(queries.len(), 2 * 13, "{r}");
    assert_eq!(r["queries"], 26);
    for q in &queries {
        assert!(
            q.starts_with("SELECT day, lang, served, country, count() AS sessions"),
            "{q}"
        );
        assert!(
            q.contains("LIMIT 50000"),
            "explicit limit (default is 100 rows): {q}"
        );
        for banned in [
            "$ip",
            "distinct_id",
            "$current_url",
            "$referrer",
            "person.properties",
            "SELECT *",
        ] {
            assert!(!q.contains(banned), "{banned} requested: {q}");
        }
        // Person and session ids are only counted inside PostHog.
        assert!(q.contains("uniq(person)") || !q.contains("person"), "{q}");
    }
    // Only aggregates are stored: these columns and nothing else.
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns WHERE table_name = 'autopilot_posthog_daily' ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        cols,
        [
            "project_id",
            "day",
            "preferred",
            "served",
            "country",
            "event",
            "n"
        ]
    );
    let n = |pref: &'static str, country: &'static str, event: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT sum(n)::int8 FROM autopilot_posthog_daily WHERE preferred = $1 AND country = $2 AND event = $3",
            )
            .bind(pref)
            .bind(country)
            .bind(event)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(n("de", "DE", "$view").await, Some(200));
    assert_eq!(
        n("de", "AT", "$view").await,
        Some(50),
        "40 days ago: inside the 90-day backfill"
    );
    assert_eq!(n("de", "DE", "$bounce").await, Some(110));
    assert_eq!(n("nl", "NL", "$conversion").await, Some(9));
    assert_eq!(n("en", "US", "$visitor").await, Some(280));

    // Autopilot now ranks on PostHog's history and says so.
    let (_, _, o, _) = c(Method::GET, "/api/projects/visa/autopilot?days=90", None).await;
    assert_eq!(o["source"]["kind"], "posthog", "{}", o["source"]);
    assert_eq!(o["source"]["goal"], "user signed up");
    assert_eq!(
        o["data_days"], 41,
        "from the first imported day (40 days ago) to today"
    );
    assert_eq!(o["total_visits"], 700);
    let de = o["opportunities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["language"] == "de")
        .expect("German ranked");
    assert_eq!(de["visits"], 250);
    assert!(
        (de["bounce_rate"].as_f64().unwrap() - 140.0 / 250.0).abs() < 1e-9,
        "{de}"
    );
    assert!((o["bounce_rate_all"].as_f64().unwrap() - 290.0 / 700.0).abs() < 1e-9);
    // Unknown served locale (no locale code in the URL, no default set) never counts as "misrouted".
    assert_eq!(o["misrouted"], json!([]));
    assert_eq!(o["posthog"]["connected"], true);
    let (_, _, o2, _) = c(
        Method::GET,
        "/api/projects/visa/autopilot?days=90&source=insights",
        None,
    )
    .await;
    assert_eq!(
        (o2["source"]["kind"].as_str(), o2["total_visits"].as_i64()),
        (Some("insights"), Some(0))
    );

    // Incremental: the next sync only re-fetches from the last synced day.
    log.lock().unwrap().clear();
    let (s, _, r, _) = c(Method::POST, &format!("{base}/sync"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (r["queries"].as_i64(), log.lock().unwrap().len()),
        (Some(2), 2)
    );
    assert_eq!(
        n("de", "DE", "$view").await,
        Some(200),
        "re-fetched days replace, never add up"
    );
    assert_eq!(n("de", "AT", "$view").await, Some(50), "older days kept");

    // A sync error is stored in plain words (the key stays out of it).
    let enc = glot_server::auth::encrypt(&state.secret, "phx_noscope");
    sqlx::query("UPDATE autopilot_posthog SET api_key_enc = $1")
        .bind(enc)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, e, _) = c(Method::POST, &format!("{base}/sync"), None).await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("posthog_scope"))
    );
    let (_, _, v, raw) = c(Method::GET, base, None).await;
    assert!(v["last_error"].as_str().unwrap().contains("query:read") && !raw.contains("phx_"));

    // Disconnect forgets the key and every imported counter.
    let (s, ..) = c(Method::DELETE, base, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let left: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM autopilot_posthog) + (SELECT count(*) FROM autopilot_posthog_daily)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
