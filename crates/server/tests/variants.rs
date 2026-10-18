//! Copy experiments end to end (real Postgres, mock LLM; skipped when Postgres is unreachable):
//! generate validated variants → serve via the insights beacon → attribute conversions → winner → promote.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

#[tokio::test]
async fn variants_end_to_end() {
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

    let mut cookie = String::new();
    let mut call = async |method: Method, uri: &str, body: Value| -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !cookie.is_empty() {
            req = req.header(header::COOKIE, format!("glot_session={cookie}"));
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        if let Some(c) = res.headers().get(header::SET_COOKIE) {
            let c = c.to_str().unwrap().split(';').next().unwrap();
            cookie = c.trim_start_matches("glot_session=").to_string();
        }
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    };

    let (s, _) = call(
        Method::POST,
        "/api/auth/signup",
        json!({"email": "v@example.com", "password": "correct horse", "name": "Vic"}),
    )
    .await;
    assert!(s.is_success());
    let (s, _) = call(
        Method::POST,
        "/api/projects",
        json!({"name": "V", "slug": "vx", "source_locale": "en", "target_locales": ["fr", "de"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    call(
        Method::PATCH,
        "/api/projects/vx",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
    )
    .await;
    for (k, src, fr) in [
        (
            "pricing.cta",
            "Start your {days}-day free trial",
            "Commencez votre essai gratuit de {days} jours",
        ),
        ("pricing.title", "Simple pricing", "Des tarifs simples"),
    ] {
        call(
            Method::POST,
            "/api/projects/vx/keys",
            json!({"key": k, "source": src}),
        )
        .await;
        let (s, _) = call(
            Method::PUT,
            &format!("/api/projects/vx/keys/{k}/translations/fr"),
            json!({"value": fr}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }

    let base = "/api/projects/vx/experiments";
    // Control arm = official translation: none yet for de.
    let (s, _) = call(
        Method::POST,
        base,
        json!({"key": "pricing.cta", "locale": "de"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Guardrail: a width budget nothing fits → every candidate rejected, nothing created or served.
    let tiny =
        json!({"max_width_px": 12.0, "max_lines": 1, "font": {"family": "Inter", "size_px": 14.0}});
    let (s, e) = call(
        Method::POST,
        base,
        json!({"key": "pricing.cta", "locale": "fr", "fit": tiny}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["code"], "no_valid_variant");

    let (s, exp) = call(
        Method::POST,
        base,
        json!({"key": "pricing.cta", "locale": "fr", "variants": 2, "min_n": 50, "explore_pct": 100}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{exp}");
    let id = exp["id"].as_str().unwrap().to_string();
    let arms = exp["arms"].as_array().unwrap();
    assert_eq!(arms.len(), 3);
    assert_eq!(arms[0]["is_control"], true);
    assert_eq!(
        arms[0]["value"],
        "Commencez votre essai gratuit de {days} jours"
    );
    assert!(
        arms[1]["value"].as_str().unwrap().contains("{days}"),
        "placeholders kept"
    );
    assert_eq!(exp["rejected"], json!([]));
    let (s, _) = call(
        Method::POST,
        base,
        json!({"key": "pricing.cta", "locale": "fr"}),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "one live experiment per key and locale"
    );

    // Second experiment, stopped right away: never served again.
    let (_, other) = call(
        Method::POST,
        base,
        json!({"key": "pricing.title", "locale": "fr", "min_n": 50}),
    )
    .await;
    let other_id = other["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        Method::POST,
        &format!("{base}/{other_id}/stop"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (_, o) = call(Method::GET, "/api/projects/vx/insights", Value::Null).await;
    let pk = o["public_key"].as_str().unwrap().to_string();
    sqlx::query("UPDATE projects SET insights_rate_per_min = 1000000")
        .execute(&pool)
        .await
        .unwrap();
    let hit = format!("/api/insights/{pk}");

    // Simulated visitors: arm 1 converts 40 %, the others 4 % (deterministic schedule).
    let mut served = [0usize; 3];
    let mut seen_other = false;
    let mut decided_at = None;
    for visitor in 0..1500usize {
        let (s, r) = call(
            Method::POST,
            &hit,
            json!({"langs": ["fr-FR"], "served": "fr"}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let v = r["variants"].as_array().cloned().unwrap_or_default();
        seen_other |= v.iter().any(|a| a["id"] == other_id.as_str());
        let Some(a) = v.iter().find(|a| a["id"] == id.as_str()) else {
            continue;
        };
        let arm = a["arm"].as_u64().unwrap() as usize;
        assert_eq!(a["key"], "pricing.cta");
        assert_eq!(
            a["value"], arms[arm]["value"],
            "serves only stored (validated) arms"
        );
        served[arm] += 1;
        let converts = if arm == 1 {
            visitor % 5 < 2
        } else {
            visitor % 25 == 0
        };
        if converts {
            let (s, _) = call(
                Method::POST,
                &hit,
                json!({"langs": ["fr"], "served": "fr", "event": "signup", "variants": {&id: arm}}),
            )
            .await;
            assert_eq!(s, StatusCode::OK);
        }
        if decided_at.is_none() {
            let (st,): (String,) =
                sqlx::query_as("SELECT status FROM variant_experiments WHERE id = $1::uuid")
                    .bind(&id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            if st == "decided" {
                decided_at = Some(visitor);
            }
        }
    }
    assert!(!seen_other, "a stopped experiment is never served");
    let decided_at = decided_at.expect("a winner must be found");
    assert!(
        served.iter().all(|n| *n >= 50),
        "burn-in reached min_n on every arm: {served:?}"
    );

    let (s, list) = call(Method::GET, base, Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    let e = list
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(e["status"], "decided", "{e}");
    assert_eq!(e["winner"], 1);
    assert!(e["arms"][1]["p_best"].as_f64().unwrap() > 0.95);
    assert!(e["arms"][1]["lift"].as_f64().unwrap() > 1.0, "≈ +900 %");
    // Losers stopped: after the decision every explored visitor gets the winner.
    let views_after: i64 = sqlx::query_scalar(
        "SELECT sum(views)::bigint FROM variant_arms WHERE experiment_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (_, r) = call(Method::POST, &hit, json!({"served": "fr"})).await;
    assert_eq!(r["variants"][0]["arm"], 1);
    let again: i64 = sqlx::query_scalar(
        "SELECT sum(views)::bigint FROM variant_arms WHERE experiment_id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        again, views_after,
        "a decided experiment no longer accumulates"
    );
    eprintln!("winner after {decided_at} visitors, served {served:?}");

    // Promote: the winner becomes the official translation (history, reviewed), the experiment closes.
    let (s, p) = call(Method::POST, &format!("{base}/{id}/promote"), Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"], "promoted");
    let (_, keys) = call(Method::GET, "/api/projects/vx/keys", Value::Null).await;
    let cta = keys["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["key"] == "pricing.cta")
        .unwrap()
        .clone();
    assert_eq!(cta["translations"]["fr"]["value"], arms[1]["value"]);
    assert_eq!(cta["translations"]["fr"]["status"], "reviewed");
    let (_, r) = call(Method::POST, &hit, json!({"served": "fr"})).await;
    assert!(r.get("variants").is_none(), "nothing live any more: {r}");
    let (s, _) = call(Method::POST, &format!("{base}/{id}/promote"), Value::Null).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // A changed source stops serving variants validated against the old one.
    let (s, _) = call(
        Method::POST,
        base,
        json!({"key": "pricing.title", "locale": "fr", "min_n": 50}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    sqlx::query("UPDATE keys SET source = 'Pricing' WHERE key = 'pricing.title'")
        .execute(&pool)
        .await
        .unwrap();
    let (_, r) = call(Method::POST, &hit, json!({"served": "fr"})).await;
    assert!(r.get("variants").is_none(), "{r}");

    // Privacy: counters only.
    let dump: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(t::text, '|'), '') FROM (SELECT * FROM variant_arms) t",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!dump.contains("fr-FR"));
}
