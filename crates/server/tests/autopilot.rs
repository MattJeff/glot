//! Autopilot end to end on a throwaway Postgres database (skipped when unreachable): opportunity ranking and money
//! math on a fixture with a known truth, the one-click open-market flow (locale, capped job, scheduled
//! measurement, explained failure), and the self-optimising copy loop through the real serving path (winner kept,
//! silence on noise, harmful wording withdrawn, human approval).
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

struct Ctx {
    app: Router,
    pool: PgPool,
    state: glot_server::AppState,
    cookie: String,
    admin: PgPool,
    db: String,
}

impl Ctx {
    async fn new() -> Option<Ctx> {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
        let Ok(admin) = PgPool::connect(&url).await else {
            eprintln!("SKIPPED: postgres unreachable at {url}");
            return None;
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
        let mut c = Ctx {
            app,
            pool,
            state,
            cookie: String::new(),
            admin,
            db,
        };
        let res = c
            .raw(
                Method::POST,
                "/api/auth/signup",
                Some(json!({"email": "a@x.io", "password": "correct horse", "name": "Ada"})),
            )
            .await;
        c.cookie = res.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        Some(c)
    }

    async fn raw(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header(header::COOKIE, &self.cookie)
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap()
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let res = self.raw(method, uri, body).await;
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Project `slug` (en → fr, fake LLM) with `keys` translated into French.
    async fn project(&self, slug: &str, keys: &[(&str, &str, &str)]) {
        let (s, _) = self
            .call(
                Method::POST,
                "/api/projects",
                Some(json!({"name": slug, "slug": slug, "source_locale": "en", "target_locales": ["fr"]})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
        self.call(
            Method::PATCH,
            &format!("/api/projects/{slug}"),
            Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
        )
        .await;
        for (k, src, fr) in keys {
            self.call(
                Method::POST,
                &format!("/api/projects/{slug}/keys"),
                Some(json!({"key": k, "source": src})),
            )
            .await;
            let (s, _) = self
                .call(
                    Method::PUT,
                    &format!("/api/projects/{slug}/keys/{k}/translations/fr"),
                    Some(json!({"value": fr})),
                )
                .await;
            assert_eq!(s, StatusCode::OK);
        }
        sqlx::query("UPDATE projects SET insights_rate_per_min = 1000000")
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn tick(&self) {
        glot_server::autopilot::tick(&self.state)
            .await
            .unwrap_or_else(|e| panic!("tick: {}", e.message));
    }

    async fn translation(&self, key: &str, locale: &str) -> String {
        sqlx::query_scalar(
            "SELECT t.value FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.key = $1 AND t.locale = $2",
        )
        .bind(key)
        .bind(locale)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn done(self) {
        self.pool.close().await;
        let _ = sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.db))
            .execute(&self.admin)
            .await;
    }
}

fn find<'a>(list: &'a Value, field: &str, value: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|x| x[field] == value)
        .unwrap_or_else(|| panic!("no {field}={value} in {list}"))
}

#[tokio::test]
async fn open_a_market_end_to_end() {
    let Some(c) = Ctx::new().await else { return };
    c.project(
        "shop",
        &[
            ("hero.title", "Sell everywhere", "Vendez partout"),
            ("pricing.cta", "Start now", "Commencer"),
            ("cart.empty", "Your cart is empty", "Votre panier est vide"),
        ],
    )
    .await;
    let base = "/api/projects/shop/autopilot";

    // No traffic at all: nothing is invented.
    let (s, o) = c.call(Method::GET, base, None).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!(o["opportunities"], json!([]));
    assert_eq!(o["total_visits"], 0);
    assert_eq!(o["value"], Value::Null);
    let hit = format!("/api/insights/{}", o["public_key"].as_str().unwrap());

    // Fixture with a known truth (yesterday's counters): served their own language, visitors convert at 5 %.
    // German speakers convert at 2 % on the English fallback, Japanese at 4 %, 20 Portuguese visits at 0 %.
    for (pref, served, views, conv) in [
        ("en", "en", 10_000, 500),
        ("fr", "fr", 4000, 200),
        ("de", "en", 3000, 60),
        ("ja", "en", 400, 16),
        ("pt", "en", 20, 0),
    ] {
        for (event, n) in [("$view", views), ("signup", conv)] {
            sqlx::query(
                "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
                 SELECT id, (now() AT TIME ZONE 'utc')::date - 1, $1, $2, '', '', $3, $4 FROM projects",
            )
            .bind(pref)
            .bind(served)
            .bind(event)
            .bind(n as i64)
            .execute(&c.pool)
            .await
            .unwrap();
        }
    }
    // Collection started 30 days ago (one visit of unknown language): yesterday's counters are a month of traffic.
    sqlx::query(
        "INSERT INTO insight_daily (project_id, day, preferred, served, country, arm, event, n)
         SELECT id, (now() AT TIME ZONE 'utc')::date - 29, 'und', 'en', '', '', '$view', 1 FROM projects",
    )
    .execute(&c.pool)
    .await
    .unwrap();
    let (_, o) = c.call(Method::GET, base, None).await;
    let opp = o["opportunities"].as_array().unwrap();
    let order: Vec<&str> = opp
        .iter()
        .map(|x| x["language"].as_str().unwrap())
        .collect();
    assert_eq!(order[0], "de", "{order:?}");
    assert_eq!(order.last(), Some(&"pt"), "thin data ranks last: {order:?}");
    assert_eq!(order[1], "ja");
    assert_eq!(
        opp[0]["revenue_monthly"],
        Value::Null,
        "no value known: no money shown"
    );
    let pt = opp.last().unwrap();
    assert_eq!(pt["confidence"], "insufficient");
    assert_eq!(pt["visits_missing"], 10, "says how much is missing");

    // Revenue arrives with real conversion events: 12 orders of 40 EUR → measured value, aggregate only.
    for _ in 0..12 {
        let (s, _) = c
            .call(
                Method::POST,
                &hit,
                Some(json!({"langs": ["en"], "served": "en", "event": "signup", "value": 40, "currency": "eur"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
    }
    let rev: (i64, f64, i64) = sqlx::query_as(
        "SELECT sum(n)::int8, sum(value)::float8, count(*) FROM autopilot_revenue_daily",
    )
    .fetch_one(&c.pool)
    .await
    .unwrap();
    assert_eq!(
        rev,
        (12, 480.0, 1),
        "one counter row per (day, event, currency)"
    );
    let (_, o) = c.call(Method::GET, base, None).await;
    assert_eq!(
        o["value"],
        json!({"amount": 40.0, "currency": "EUR", "basis": "measured", "events": 12})
    );
    let de = &o["opportunities"][0];
    let (mid, low, high) = (
        de["gain_monthly"]["mid"].as_f64().unwrap(),
        de["gain_monthly"]["low"].as_f64().unwrap(),
        de["gain_monthly"]["high"].as_f64().unwrap(),
    );
    // Truth: 3000 visits/month × (5 % − 2 %) = 90 conversions = 3600 EUR.
    assert!(
        (mid - 90.0).abs() < 3.0 && low > 0.0 && low < 90.0 && 90.0 < high,
        "{mid} [{low}; {high}]"
    );
    assert!((de["revenue_monthly"]["mid"].as_f64().unwrap() - mid * 40.0).abs() < 1e-6);
    assert!((de["conversion_gap"].as_f64().unwrap() - 0.6).abs() < 0.03);
    assert!(de["payback_days"].as_f64().unwrap() < 1.0);
    assert_eq!(de["confidence"], "solid");

    // A declared value overrides the measured one.
    let (s, _) = c
        .call(
            Method::PUT,
            &format!("{base}/settings"),
            Some(json!({"goal": "signup", "value_per_conversion": 100, "currency": "usd", "mode": "approval"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, o) = c.call(Method::GET, base, None).await;
    assert_eq!(o["value"]["basis"], "declared");
    assert_eq!(o["value"]["amount"], 100.0);
    let (s, e) = c
        .call(
            Method::PUT,
            &format!("{base}/settings"),
            Some(json!({"goal": "signup", "value_per_conversion": 1, "currency": "XXX", "mode": "auto"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");

    // Refusals explain themselves.
    let (s, e) = c
        .call(
            Method::POST,
            &format!("{base}/open"),
            Some(json!({"locale": "en"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("source language")
    );

    // A failed translation: the opening says why and no measurement starts.
    let (s, ja) = c
        .call(
            Method::POST,
            &format!("{base}/open"),
            Some(json!({"locale": "ja"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ja}");
    sqlx::query(
        "UPDATE jobs SET status = 'failed', error = 'budget exceeded: spent $0.9000 of $0.50'",
    )
    .execute(&c.pool)
    .await
    .unwrap();
    c.tick().await;
    let (_, o) = c.call(Method::GET, base, None).await;
    let ja = find(&o["openings"], "locale", "ja");
    assert_eq!(ja["status"], "failed");
    let why = ja["error"].as_str().unwrap();
    assert!(
        why.contains("budget exceeded") && why.contains("open the market again"),
        "{why}"
    );
    assert_eq!(ja["launch_id"], Value::Null);

    // One click on German.
    let (s, de) = c
        .call(
            Method::POST,
            &format!("{base}/open"),
            Some(json!({"locale": "de"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{de}");
    assert_eq!(de["status"], "translating");
    assert_eq!(
        de["estimate"]["locale"], "de",
        "the prediction is frozen with the opening"
    );
    assert!((de["estimate"]["gain_monthly"]["mid"].as_f64().unwrap() - mid).abs() < 1e-6);
    let (s, e) = c
        .call(
            Method::POST,
            &format!("{base}/open"),
            Some(json!({"locale": "de"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    let targets: Vec<String> = sqlx::query_scalar("SELECT target_locales FROM projects")
        .fetch_one(&c.pool)
        .await
        .unwrap();
    assert!(
        targets.contains(&"de".to_string()),
        "the language is created: {targets:?}"
    );
    let job_id: uuid::Uuid = de["job_id"].as_str().unwrap().parse().unwrap();
    let (kind, status, params): (String, String, Value) =
        sqlx::query_as("SELECT kind, status, params FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&c.pool)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), status.as_str()), ("translate", "queued"));
    assert_eq!(
        params["locales"],
        json!(["de"]),
        "the right job: this locale only"
    );
    let cap = params["budget_usd"].as_f64().unwrap();
    assert_eq!(
        cap,
        glot_server::autopilot::budget_cap(de["estimate"]["cost_usd"].as_f64().unwrap())
    );
    // Not measured before it is translated.
    c.tick().await;
    let launches: i64 = sqlx::query_scalar("SELECT count(*) FROM language_launches")
        .fetch_one(&c.pool)
        .await
        .unwrap();
    assert_eq!(launches, 0);

    // The job runs (fake LLM), then the worker schedules the measurement.
    let worker = glot_server::jobs::spawn_worker(c.state.clone());
    let mut status = String::new();
    for _ in 0..300 {
        status = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&c.pool)
            .await
            .unwrap();
        if status == "done" || status == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    worker.abort();
    assert_eq!(status, "done");
    c.tick().await;
    let (mode, days, goal, lstatus, baseline, xp): (String, i32, String, String, Value, Option<uuid::Uuid>) =
        sqlx::query_as(
            "SELECT mode, days, goal, status, baseline, experiment_id FROM language_launches WHERE locale = 'de'",
        )
        .fetch_one(&c.pool)
        .await
        .unwrap();
    assert_eq!(
        (mode.as_str(), days, goal.as_str(), lstatus.as_str()),
        ("holdout", 14, "signup", "measuring")
    );
    assert_eq!(
        baseline,
        json!({"visits": 3000, "conversions": 60}),
        "baseline = the audience before"
    );
    let (hold, pct): (Option<String>, i32) =
        sqlx::query_as("SELECT hold_locale, traffic_pct FROM insight_experiments WHERE id = $1")
            .bind(xp)
            .fetch_one(&c.pool)
            .await
            .unwrap();
    assert_eq!(
        (hold.as_deref(), pct),
        (Some("en"), 80),
        "20 % held on the fallback"
    );
    let (_, o) = c.call(Method::GET, base, None).await;
    let de = find(&o["openings"], "locale", "de");
    assert_eq!(de["status"], "measuring");
    assert_eq!(de["launch"]["causal"], true, "holdout: a measured effect");
    assert!(de["launch"]["verdict"].is_string());
    assert!(
        !o["opportunities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["language"] == "de"),
        "an opened market is no longer an opportunity"
    );
    // A German speaker is now assigned by the holdout.
    let (_, r) = c
        .call(
            Method::POST,
            &hit,
            Some(json!({"langs": ["de-DE"], "served": "en"})),
        )
        .await;
    assert!(r["arm"] == "test" || r["arm"] == "control", "{r}");
    c.done().await;
}

#[tokio::test]
async fn copy_optimises_itself_with_guardrails() {
    let Some(c) = Ctx::new().await else { return };
    c.project(
        "app",
        &[
            (
                "hero.title",
                "Clip any video",
                "Découpez n'importe quelle vidéo",
            ),
            ("pricing.cta", "Start now", "Commencer maintenant"),
            ("home.signupButton", "Create my account", "Créer mon compte"),
            ("pricing.title", "Simple pricing", "Des tarifs simples"),
            (
                "legal.terms.title",
                "Terms of service",
                "Conditions d'utilisation",
            ),
            (
                "settings.saved",
                "Your changes are saved",
                "Vos modifications sont enregistrées",
            ),
        ],
    )
    .await;
    let base = "/api/projects/app/autopilot";
    let (_, o) = c.call(Method::GET, base, None).await;
    let hit = format!("/api/insights/{}", o["public_key"].as_str().unwrap());
    let mut keys: Vec<&str> = o["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["key"].as_str().unwrap())
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "hero.title",
            "home.signupButton",
            "pricing.cta",
            "pricing.title"
        ],
        "headlines, CTAs and pricing; never legal copy or plain UI text"
    );
    assert_eq!(
        o["candidates"][0]["low_traffic"], true,
        "no traffic yet: said, not hidden"
    );

    let start = |key: &'static str, extra: Value| {
        let c = &c;
        async move {
            let mut b = json!({"key": key, "locale": "fr"});
            b.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            c.call(Method::POST, &format!("{base}/optimizations"), Some(b))
                .await
        }
    };
    let (s, e) = start("legal.terms.title", json!({})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["code"], "legal_text");

    // A: one wording is truly better (30 % vs 10 %), kept automatically. B: three wordings that convert the same.
    let (s, a) = start("hero.title", json!({"mode": "auto", "min_n": 50})).await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    assert_eq!(
        (a["status"].as_str(), a["mode"].as_str(), a["kind"].as_str()),
        (Some("running"), Some("auto"), Some("headline"))
    );
    let arms = a["experiment"]["arms"].as_array().unwrap().clone();
    assert_eq!(arms.len(), 3);
    let (s, b) = start("pricing.cta", json!({"min_n": 50})).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["mode"], "approval", "human approval is the default");
    let (a_id, b_id) = (
        a["id"].as_str().unwrap().to_string(),
        b["id"].as_str().unwrap().to_string(),
    );
    let (_, o) = c.call(Method::GET, base, None).await;
    assert_eq!(
        o["candidates"].as_array().unwrap().len(),
        2,
        "texts under test leave the candidates"
    );

    // Real serving path. Each wording converts on a fixed schedule of its own views, so the truth is exact:
    // A arm 1 = 30 %, everything else = 10 %.
    let mut seen = std::collections::BTreeMap::<(String, u64), u64>::new();
    for visitor in 0..1200 {
        let (s, r) = c
            .call(
                Method::POST,
                &hit,
                Some(json!({"langs": ["fr-FR"], "served": "fr"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        for v in r["variants"].as_array().cloned().unwrap_or_default() {
            let (id, arm) = (
                v["id"].as_str().unwrap().to_string(),
                v["arm"].as_u64().unwrap(),
            );
            let n = seen.entry((id.clone(), arm)).or_default();
            *n += 1;
            let converts = if id == a_id && arm == 1 {
                *n % 10 < 3
            } else {
                (*n).is_multiple_of(10)
            };
            if converts {
                c.call(
                    Method::POST,
                    &hit,
                    Some(json!({"langs": ["fr"], "served": "fr", "event": "signup", "variants": {id: arm}})),
                )
                .await;
            }
        }
        if visitor % 50 == 49 {
            c.tick().await;
        }
    }
    let (_, o) = c.call(Method::GET, base, None).await;
    let a = find(&o["optimizations"], "id", &a_id);
    assert_eq!(a["status"], "promoted", "{a}");
    assert_eq!(a["result"]["arm"], 1, "the truly better wording");
    let (lift, low) = (
        a["result"]["lift"].as_f64().unwrap(),
        a["result"]["lift_low"].as_f64().unwrap(),
    );
    assert!(
        low > 0.0 && lift > 0.8,
        "+{lift} [{low}; …]: the truth is +200 %"
    );
    assert!(
        a["result"]["extra_conversions_monthly"]["mid"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(
        c.translation("hero.title", "fr").await,
        arms[1]["value"].as_str().unwrap(),
        "kept as the official translation"
    );
    // B: noise. No winner, nothing withdrawn, the official translation untouched.
    let b = find(&o["optimizations"], "id", &b_id);
    assert_eq!(b["status"], "running", "{b}");
    assert_eq!(b["dropped"], json!([]));
    let (blow, bhigh) = (
        b["result"]["lift_low"].as_f64().unwrap(),
        b["result"]["lift_high"].as_f64().unwrap(),
    );
    assert!(
        blow < 0.0 && bhigh > 0.0,
        "the interval says 'no idea': [{blow}; {bhigh}]"
    );
    // … and after max_days it gives up by itself.
    sqlx::query("UPDATE autopilot_optimizations SET created_at = now() - interval '31 days' WHERE experiment_id = $1::uuid")
        .bind(&b_id)
        .execute(&c.pool)
        .await
        .unwrap();
    c.tick().await;
    let (_, o) = c.call(Method::GET, base, None).await;
    let b = find(&o["optimizations"], "id", &b_id);
    assert_eq!(
        (b["status"].as_str(), b["reason"].as_str()),
        (Some("no_winner"), Some("no_difference"))
    );
    assert_eq!(
        c.translation("pricing.cta", "fr").await,
        "Commencer maintenant"
    );
    let xp: String =
        sqlx::query_scalar("SELECT status FROM variant_experiments WHERE id = $1::uuid")
            .bind(&b_id)
            .fetch_one(&c.pool)
            .await
            .unwrap();
    assert_eq!(xp, "stopped");

    // C: a harmful wording (counters set to a known state: official 20 %, arm 1 2 %, arm 2 19.5 %).
    let (s, h) = start("home.signupButton", json!({"mode": "auto"})).await;
    assert_eq!(s, StatusCode::CREATED, "{h}");
    let h_id = h["id"].as_str().unwrap().to_string();
    let set = |arm: i32, views: i64, conv: i64| {
        let (pool, id) = (c.pool.clone(), h_id.clone());
        async move {
            sqlx::query("UPDATE variant_arms SET views = $3, conversions = $4 WHERE experiment_id = $1::uuid AND arm = $2")
                .bind(id)
                .bind(arm)
                .bind(views)
                .bind(conv)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    set(0, 400, 80).await;
    set(1, 400, 8).await;
    set(2, 400, 78).await;
    c.tick().await;
    let (_, o) = c.call(Method::GET, base, None).await;
    let h = find(&o["optimizations"], "id", &h_id);
    assert_eq!(h["status"], "running");
    let dropped = h["dropped"].as_array().unwrap();
    assert_eq!(dropped.len(), 1, "{h}");
    assert_eq!(
        (dropped[0]["arm"].as_i64(), dropped[0]["why"].as_str()),
        (Some(1), Some("worse"))
    );
    assert_eq!(dropped[0]["conversions"], 8);
    for _ in 0..40 {
        let (_, r) = c
            .call(
                Method::POST,
                &hit,
                Some(json!({"langs": ["fr"], "served": "fr"})),
            )
            .await;
        for v in r["variants"].as_array().cloned().unwrap_or_default() {
            assert!(
                !(v["id"] == h_id.as_str() && v["arm"] == 1),
                "a withdrawn wording is never served again"
            );
        }
    }
    // The last challenger turns out harmful too: full rollback to the official translation.
    set(0, 800, 160).await;
    set(2, 800, 16).await;
    c.tick().await;
    let (_, o) = c.call(Method::GET, base, None).await;
    let h = find(&o["optimizations"], "id", &h_id);
    assert_eq!(
        (h["status"].as_str(), h["reason"].as_str()),
        (Some("rolled_back"), Some("all_variants_worse"))
    );
    assert_eq!(
        c.translation("home.signupButton", "fr").await,
        "Créer mon compte"
    );
    let (_, r) = c
        .call(
            Method::POST,
            &hit,
            Some(json!({"langs": ["fr"], "served": "fr"})),
        )
        .await;
    assert!(
        !r["variants"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .any(|v| v["id"] == h_id.as_str()),
        "nothing but the official translation is served"
    );

    // D: approval mode. A winner waits for a human; approving writes it.
    let (s, d) = start("pricing.title", json!({"min_n": 50})).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    let d_id = d["id"].as_str().unwrap().to_string();
    let winner = d["experiment"]["arms"][1]["value"]
        .as_str()
        .unwrap()
        .to_string();
    sqlx::query(
        "UPDATE variant_arms SET views = 500, conversions = CASE arm WHEN 1 THEN 75 ELSE 25 END WHERE experiment_id = $1::uuid",
    )
    .bind(&d_id)
    .execute(&c.pool)
    .await
    .unwrap();
    // The next real conversion triggers the analysis (variants.rs).
    c.call(
        Method::POST,
        &hit,
        Some(json!({"langs": ["fr"], "served": "fr", "event": "signup", "variants": {&d_id: 1}})),
    )
    .await;
    c.tick().await;
    let (_, o) = c.call(Method::GET, base, None).await;
    let d = find(&o["optimizations"], "id", &d_id);
    assert_eq!(d["status"], "awaiting_approval", "{d}");
    assert_eq!(
        c.translation("pricing.title", "fr").await,
        "Des tarifs simples",
        "nothing written without a human"
    );
    let (s, d) = c
        .call(
            Method::POST,
            &format!("{base}/optimizations/{d_id}/approve"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["status"], "promoted");
    assert_eq!(c.translation("pricing.title", "fr").await, winner);
    let (s, _) = c
        .call(
            Method::POST,
            &format!("{base}/optimizations/{d_id}/approve"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = c
        .call(
            Method::POST,
            &format!("{base}/optimizations/{d_id}/stop"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "already ended");
    c.done().await;
}

/// Real data, no fixture: prints what Autopilot says about a project of the database in `GLOT_AUTOPILOT_DB`
/// (a private copy of a real database; migrations are applied to it). Read-only otherwise.
/// `GLOT_AUTOPILOT_DB=postgres://…/glot_cliponit_autopilot cargo test -p glot-server --test autopilot real -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn real_project_report() {
    let url = std::env::var("GLOT_AUTOPILOT_DB").expect("GLOT_AUTOPILOT_DB");
    let slug = std::env::var("GLOT_AUTOPILOT_SLUG").unwrap_or_else(|_| "cliponit".into());
    let pool = PgPool::connect(&url).await.unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let state = glot_server::AppState::new(pool.clone(), "report", false);
    let user: glot_server::auth::User = sqlx::query_as(
        "SELECT u.id, u.email, u.name FROM users u JOIN project_members m ON m.user_id = u.id
         JOIN projects p ON p.id = m.project_id WHERE p.slug = $1 ORDER BY (m.role = 'owner') DESC LIMIT 1",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .unwrap();
    for days in [30, 90] {
        let axum::Json(o) = glot_server::autopilot::overview(
            axum::extract::State(state.clone()),
            glot_server::auth::AuthUser(user.clone()),
            glot_server::AppPath(slug.clone()),
            glot_server::AppQuery(glot_server::autopilot::Query {
                days: Some(days),
                source: None,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("{}", e.message));
        let mut v = serde_json::to_value(&o).unwrap();
        v["public_key"] = "(hidden)".into();
        println!(
            "=== {slug}, last {days} days ===\n{}",
            serde_json::to_string_pretty(&v).unwrap()
        );
    }
}
