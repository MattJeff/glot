//! Billing: signed Stripe webhooks → entitlements (subscribe, grace, cancel), quotas on the cloud endpoints,
//! and Checkout / Portal / meter events against stripe-mock (`docker run -p 12111:12111 stripe/stripe-mock`).
//! Real Postgres (DATABASE_URL, default :5433); each part is skipped when its dependency is unreachable.
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use glot_server::billing::{self, Config};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::Arc;
use tower::ServiceExt;

const HOOK: &str = "whsec_test_glot";
const MOCK: &str = "http://localhost:12111";

async fn setup(cfg: Config) -> Option<(Router, PgPool, PgPool, String)> {
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
    let mut state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    state.billing = Some(Arc::new(cfg));
    // No job worker: jobs stay queued (a worker would call a real LLM for the platform-key project).
    Some((
        glot_server::router(state, "http://localhost:3100"),
        admin,
        pool,
        db,
    ))
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, format!("glot_session={c}"));
    }
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    let req = req
        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn hook(app: &Router, event: &Value, secret: &str) -> StatusCode {
    let body = event.to_string();
    let sig = billing::sign(secret, body.as_bytes(), chrono::Utc::now().timestamp());
    let req = Request::post("/api/billing/webhook")
        .header("stripe-signature", sig)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

async fn signup(app: &Router, email: &str) -> (String, String) {
    let req = Request::post("/api/auth/signup")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"email": email, "password": "correct horse", "name": "O"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let cookie = res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    let user: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (cookie, user["id"].as_str().unwrap().to_string())
}

fn sub_event(id: &str, ty: &str, created: i64, user: &str, status: &str, lookup: &str) -> Value {
    json!({
        "id": id, "type": ty, "created": created,
        "data": {"object": {
            "id": "sub_1", "object": "subscription", "customer": "cus_A", "status": status,
            "cancel_at_period_end": false, "metadata": {"glot_user_id": user},
            "items": {"data": [
                {"id": "si_base", "current_period_end": 1_900_000_000i64,
                 "price": {"id": "price_x", "lookup_key": lookup, "recurring": {"usage_type": "licensed"}}},
                {"id": "si_llm", "current_period_end": 1_900_000_000i64,
                 "price": {"id": "price_m", "lookup_key": "glot_llm_strings", "recurring": {"usage_type": "metered"}}}
            ]}
        }}
    })
}

#[tokio::test]
async fn webhooks_drive_entitlements_and_quotas() {
    let mut cfg = Config::new("sk_test_x", HOOK, MOCK, "http://localhost:3100");
    cfg.plans.get_mut("free").unwrap().cdn_requests = Some(3);
    let Some((app, admin, pool, db)) = setup(cfg).await else {
        return;
    };
    let (cookie, user) = signup(&app, "o@x.io").await;
    let c = Some(cookie.as_str());
    let plan = |b: &Value| b["plan"].as_str().unwrap_or_default().to_string();

    // Free: one cloud project.
    let mk = |slug: &str| json!({"name": "P", "slug": slug, "source_locale": "en", "target_locales": ["fr"]});
    assert_eq!(
        call(&app, Method::POST, "/api/projects", c, Some(mk("pa")))
            .await
            .0,
        StatusCode::CREATED
    );
    let (s, e) = call(&app, Method::POST, "/api/projects", c, Some(mk("pb"))).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(e["error"]["code"], "plan_limit");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("/app/settings/billing")
    );

    // On-demand is not in Free (this attempt is also CDN request 1 of 3).
    call(
        &app,
        Method::PATCH,
        "/api/projects/pa",
        c,
        Some(json!({"public_cdn": true})),
    )
    .await;
    call(
        &app,
        Method::PUT,
        "/api/projects/pa/ondemand",
        c,
        Some(json!({"locales": ["de"], "monthly_budget_usd": null, "per_day": 5})),
    )
    .await;
    let (s, e) = call(&app, Method::GET, "/cdn/pa/de.json?ondemand=1", None, None).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED, "{e}");
    assert_eq!(e["error"]["code"], "plan_required");

    // Free CDN quota (3 here), then a clear 402.
    for _ in 0..2 {
        assert_eq!(
            call(&app, Method::GET, "/cdn/pa/en.json", None, None)
                .await
                .0,
            StatusCode::OK
        );
    }
    let (s, e) = call(&app, Method::GET, "/cdn/pa/en.json", None, None).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(e["error"]["code"], "quota_exceeded");

    // Server translation on glot's key: Free allowance used up → 402 pointing at BYOK; BYOK is never limited.
    sqlx::query("INSERT INTO billing_usage (user_id, month, kind, n) VALUES ($1::uuid, date_trunc('month', now())::date, 'llm_strings', 500)")
        .bind(&user).execute(&pool).await.unwrap();
    let (s, e) = call(
        &app,
        Method::POST,
        "/api/projects/pa/jobs",
        c,
        Some(json!({"kind": "translate"})),
    )
    .await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);
    assert!(
        e["error"]["message"].as_str().unwrap().contains("BYOK"),
        "{e}"
    );
    call(&app, Method::PATCH, "/api/projects/pa", c, Some(json!({"llm": {"provider": "anthropic", "model": "claude-haiku-5-5", "api_key": "sk-ant-own"}}))).await;
    let (s, e) = call(
        &app,
        Method::POST,
        "/api/projects/pa/jobs",
        c,
        Some(json!({"kind": "translate"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{e}");

    let (_, b) = call(&app, Method::GET, "/api/billing", c, None).await;
    assert_eq!(plan(&b), "free");
    assert_eq!(b["usage"]["cdn_requests"], 4);
    assert_eq!(b["usage"]["projects"], 1);
    assert_eq!(b["limits"]["projects"], 1);

    // Webhook: bad signature refused, nothing applied.
    let created = sub_event(
        "evt_1",
        "customer.subscription.created",
        1000,
        &user,
        "active",
        "glot_pro_month",
    );
    assert_eq!(
        hook(&app, &created, "whsec_wrong").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        plan(&call(&app, Method::GET, "/api/billing", c, None).await.1),
        "free"
    );

    // Subscription created → Pro entitlements.
    assert_eq!(hook(&app, &created, HOOK).await, StatusCode::OK);
    let (_, b) = call(&app, Method::GET, "/api/billing", c, None).await;
    assert_eq!(plan(&b), "pro", "{b}");
    assert_eq!(b["metered"], true);
    assert!(b["current_period_end"].is_string());
    assert_eq!(
        call(&app, Method::POST, "/api/projects", c, Some(mk("pb")))
            .await
            .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(&app, Method::GET, "/cdn/pa/en.json", None, None)
            .await
            .0,
        StatusCode::OK
    );

    // Same event again: acknowledged, applied once.
    assert_eq!(hook(&app, &created, HOOK).await, StatusCode::OK);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM stripe_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // Payment fails → past_due within grace keeps Pro; after grace → Free; paid → back to Pro.
    let failed = json!({"id": "evt_2", "type": "invoice.payment_failed", "created": 1100, "data": {"object": {"customer": "cus_A"}}});
    assert_eq!(hook(&app, &failed, HOOK).await, StatusCode::OK);
    let past_due = sub_event(
        "evt_3",
        "customer.subscription.updated",
        1101,
        &user,
        "past_due",
        "glot_pro_month",
    );
    assert_eq!(hook(&app, &past_due, HOOK).await, StatusCode::OK);
    let (_, b) = call(&app, Method::GET, "/api/billing", c, None).await;
    assert_eq!(
        (plan(&b).as_str(), b["status"].as_str()),
        ("pro", Some("past_due"))
    );
    assert!(b["grace_until"].is_string());
    sqlx::query("UPDATE billing_accounts SET grace_until = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        plan(&call(&app, Method::GET, "/api/billing", c, None).await.1),
        "free"
    );
    let paid = json!({"id": "evt_4", "type": "invoice.paid", "created": 1200, "data": {"object": {"customer": "cus_A"}}});
    assert_eq!(hook(&app, &paid, HOOK).await, StatusCode::OK);
    let active = sub_event(
        "evt_5",
        "customer.subscription.updated",
        1201,
        &user,
        "active",
        "glot_team_year",
    );
    assert_eq!(hook(&app, &active, HOOK).await, StatusCode::OK);
    let (_, b) = call(&app, Method::GET, "/api/billing", c, None).await;
    assert_eq!(plan(&b), "team");
    assert!(b["grace_until"].is_null());

    // An older event delivered late is ignored.
    let stale = sub_event(
        "evt_6",
        "customer.subscription.updated",
        1150,
        &user,
        "unpaid",
        "glot_pro_month",
    );
    assert_eq!(hook(&app, &stale, HOOK).await, StatusCode::OK);
    assert_eq!(
        plan(&call(&app, Method::GET, "/api/billing", c, None).await.1),
        "team"
    );

    // Cancellation → Free; existing projects stay, new ones are refused.
    let deleted = sub_event(
        "evt_7",
        "customer.subscription.deleted",
        1300,
        &user,
        "canceled",
        "glot_team_year",
    );
    assert_eq!(hook(&app, &deleted, HOOK).await, StatusCode::OK);
    let (_, b) = call(&app, Method::GET, "/api/billing", c, None).await;
    assert_eq!(
        (plan(&b).as_str(), b["status"].as_str()),
        ("free", Some("canceled"))
    );
    assert_eq!(
        call(&app, Method::GET, "/api/projects/pb", c, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::POST, "/api/projects", c, Some(mk("pc")))
            .await
            .0,
        StatusCode::PAYMENT_REQUIRED
    );

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}

#[tokio::test]
async fn checkout_portal_and_meter_against_stripe_mock() {
    if reqwest::get(format!("{MOCK}/v1/customers")).await.is_err() {
        eprintln!("SKIPPED: stripe-mock unreachable at {MOCK}");
        return;
    }
    let mut cfg = Config::new("sk_test_123", HOOK, MOCK, "http://localhost:3100");
    cfg.automatic_tax = true;
    cfg.meter_event = Some("glot_llm_strings".into());
    let Some((app, admin, pool, db)) = setup(cfg).await else {
        return;
    };
    let (cookie, user) = signup(&app, "m@x.io").await;
    let c = Some(cookie.as_str());

    let (s, r) = call(
        &app,
        Method::POST,
        "/api/billing/checkout",
        c,
        Some(json!({"plan": "pro", "interval": "year"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(
        r["url"].as_str().is_some_and(|u| u.starts_with("http")),
        "{r}"
    );
    assert_eq!(
        call(&app, Method::GET, "/api/billing", c, None).await.1["has_customer"],
        true
    );
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/billing/checkout",
        c,
        Some(json!({"plan": "gold"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, r) = call(&app, Method::POST, "/api/billing/portal", c, None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["url"].is_string());
    let (s, r) = call(&app, Method::GET, "/api/billing/invoices", c, None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r.is_array());

    // Existing subscription: checkout swaps the plan price instead of opening a second subscription.
    hook(
        &app,
        &sub_event(
            "evt_a",
            "customer.subscription.created",
            1,
            &user,
            "active",
            "glot_pro_month",
        ),
        HOOK,
    )
    .await;
    let (s, r) = call(
        &app,
        Method::POST,
        "/api/billing/checkout",
        c,
        Some(json!({"plan": "team"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["url"].as_str().unwrap().ends_with("?changed=1"));

    // Metered overage on glot's key: reported to the meter without failing the job path.
    let (s, e) = call(
        &app,
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "P", "slug": "pm", "source_locale": "en"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{e}");
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'pm'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/projects/pm/jobs",
        c,
        Some(json!({"kind": "translate"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    sqlx::query("INSERT INTO billing_usage (user_id, month, kind, n) VALUES ($1::uuid, date_trunc('month', now())::date, 'llm_strings', 9990)")
        .bind(&user).execute(&pool).await.unwrap();
    let mut state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let mut cfg = Config::new("sk_test_123", HOOK, MOCK, "http://localhost:3100");
    cfg.meter_event = Some("glot_llm_strings".into());
    state.billing = Some(Arc::new(cfg));
    billing::after_job(&state, pid, uuid::Uuid::new_v4(), 25).await;
    let (_, b) = call(&app, Method::GET, "/api/billing", c, None).await;
    assert_eq!(b["usage"]["llm_strings"], 10_015);

    let (s, p) = call(&app, Method::GET, "/api/billing/plans", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(p["plans"]["free"]["projects"], 1);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
