//! Billing for the hosted cloud (Stripe): plans, subscription state from webhooks, monthly quotas on the
//! cloud endpoints (CDN/OTA, insights, remote MCP, on-demand, server-side translation with glot's LLM key).
//! Off unless STRIPE_SECRET_KEY is set: a self-hosted server is never limited, and the CLI / local MCP never
//! reach this code. See docs/billing.md.
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::{ApiResult, AppError, AppJson, AppState, auth::AuthUser, projects::ProjectRow};

/// Pinned so payload shapes never follow the account's default version.
pub const API_VERSION: &str = "2026-09-30.endive";
/// Webhook timestamps older than this are refused (replay window, same as Stripe's libraries).
const TOLERANCE_SECS: i64 = 300;

pub const CDN: &str = "cdn_requests";
pub const INSIGHTS: &str = "insights_events";
pub const MCP: &str = "mcp_calls";
pub const ONDEMAND: &str = "ondemand_jobs";
pub const LLM: &str = "llm_strings";
pub const REVIEW: &str = "review_annotations";

/// Monthly limits of a plan; `null` = unlimited. Overridable with `GLOT_PLAN_LIMITS` (JSON, per plan).
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Limits {
    pub projects: Option<i64>,
    pub seats: Option<i64>,
    pub cdn_requests: Option<i64>,
    pub insights_events: Option<i64>,
    pub mcp_calls: Option<i64>,
    pub ondemand_jobs: Option<i64>,
    /// Strings translated server-side with glot's LLM key (BYOK is never counted).
    pub llm_strings: Option<i64>,
    /// Segmented experiments live at once (running/paused/personalized), not monthly.
    #[serde(default)]
    pub experiments: Option<i64>,
    /// Targeted segments per experiment (0 = no segmentation).
    #[serde(default)]
    pub xp_cells: Option<i64>,
    /// Live review annotations created per month (review_live).
    #[serde(default)]
    pub review_annotations: Option<i64>,
}

impl Limits {
    fn get(&self, kind: &str) -> Option<i64> {
        match kind {
            "projects" => self.projects,
            CDN => self.cdn_requests,
            INSIGHTS => self.insights_events,
            MCP => self.mcp_calls,
            ONDEMAND => self.ondemand_jobs,
            LLM => self.llm_strings,
            REVIEW => self.review_annotations,
            _ => None,
        }
    }
}

fn label(kind: &str) -> &'static str {
    match kind {
        CDN => "CDN/OTA requests",
        INSIGHTS => "Insights events",
        MCP => "Remote MCP calls",
        ONDEMAND => "On-demand translation jobs",
        LLM => "Server-side translations with glot's LLM key",
        REVIEW => "Live review annotations",
        _ => "Cloud projects",
    }
}

pub fn default_plans() -> BTreeMap<String, Limits> {
    let l = |projects, seats, cdn, insights, mcp, ondemand, llm| Limits {
        projects: Some(projects),
        seats: Some(seats),
        cdn_requests: Some(cdn),
        insights_events: Some(insights),
        mcp_calls: Some(mcp),
        ondemand_jobs: Some(ondemand),
        llm_strings: Some(llm),
        experiments: None,
        xp_cells: None,
        review_annotations: None,
    };
    let xp = |l: Limits, experiments, xp_cells| Limits {
        experiments,
        xp_cells,
        ..l
    };
    let mut plans = BTreeMap::from([
        (
            "free".into(),
            xp(l(1, 1, 50_000, 10_000, 500, 0, 500), Some(1), Some(0)),
        ),
        (
            "pro".into(),
            xp(
                l(5, 3, 1_000_000, 250_000, 20_000, 30, 10_000),
                Some(5),
                Some(2),
            ),
        ),
        (
            "team".into(),
            xp(
                l(25, 15, 10_000_000, 2_000_000, 200_000, 300, 50_000),
                None,
                Some(4),
            ),
        ),
    ]);
    // Live review: team = unlimited.
    for (plan, max) in [("free", 200), ("pro", 10_000)] {
        if let Some(l) = plans.get_mut(plan) {
            l.review_annotations = Some(max);
        }
    }
    plans
}

#[derive(Debug, Clone)]
pub struct Config {
    pub secret_key: String,
    pub webhook_secret: String,
    /// `https://api.stripe.com`, or stripe-mock (`http://localhost:12111`) in tests.
    pub api_base: String,
    pub web_origin: String,
    /// Stripe Tax on Checkout (needs Stripe Tax set up in the dashboard).
    pub automatic_tax: bool,
    /// Billing Meter event name for LLM overage (`glot_llm_strings`); `None` = no usage-based billing.
    pub meter_event: Option<String>,
    pub portal_configuration: Option<String>,
    pub grace_days: i64,
    pub plans: BTreeMap<String, Limits>,
}

impl Config {
    pub fn new(secret_key: &str, webhook_secret: &str, api_base: &str, web_origin: &str) -> Self {
        Self {
            secret_key: secret_key.into(),
            webhook_secret: webhook_secret.into(),
            api_base: api_base.trim_end_matches('/').into(),
            web_origin: web_origin.into(),
            automatic_tax: false,
            meter_event: None,
            portal_configuration: None,
            grace_days: 7,
            plans: default_plans(),
        }
    }

    pub fn from_env(web_origin: &str) -> anyhow::Result<Option<Self>> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let Some(key) = var("STRIPE_SECRET_KEY") else {
            return Ok(None);
        };
        let hook = var("STRIPE_WEBHOOK_SECRET").ok_or_else(|| {
            anyhow::anyhow!("STRIPE_WEBHOOK_SECRET is required when STRIPE_SECRET_KEY is set")
        })?;
        let base = var("STRIPE_API_BASE").unwrap_or_else(|| "https://api.stripe.com".into());
        let mut c = Self::new(&key, &hook, &base, web_origin);
        c.automatic_tax = var("STRIPE_AUTOMATIC_TAX").is_some_and(|v| v == "1" || v == "true");
        c.meter_event = var("STRIPE_METER_EVENT");
        c.portal_configuration = var("STRIPE_PORTAL_CONFIGURATION");
        if let Some(d) = var("GLOT_BILLING_GRACE_DAYS") {
            c.grace_days = d.parse()?;
        }
        if let Some(j) = var("GLOT_PLAN_LIMITS") {
            let over: BTreeMap<String, Limits> =
                serde_json::from_str(&j).map_err(|e| anyhow::anyhow!("GLOT_PLAN_LIMITS: {e}"))?;
            c.plans.extend(over);
        }
        tracing::info!(live = key.contains("_live_"), "billing enabled");
        Ok(Some(c))
    }

    fn limits(&self, plan: &str) -> Limits {
        self.plans
            .get(plan)
            .or_else(|| self.plans.get("free"))
            .cloned()
            .unwrap_or_default()
    }

    fn upgrade_url(&self) -> String {
        format!("{}/app/settings/billing", self.web_origin)
    }
}

/// Plan that applies now. `past_due` keeps the plan until the grace period ends (unset = Stripe still retrying,
/// `invoice.payment_failed` not received yet); `unpaid`, `canceled`, `incomplete*` fall back to Free.
pub fn effective_plan<'a>(
    plan: &'a str,
    status: &str,
    grace_until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> &'a str {
    match status {
        "active" | "trialing" => plan,
        "past_due" if grace_until.is_none_or(|g| g > now) => plan,
        _ => "free",
    }
}

fn title(plan: &str) -> String {
    let mut c = plan.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn quota_error(cfg: &Config, plan: &str, kind: &str, max: i64, hint: &str) -> AppError {
    let up = cfg.upgrade_url();
    if max == 0 {
        return AppError::new(
            StatusCode::PAYMENT_REQUIRED,
            "plan_required",
            format!(
                "{} are not included in the {} plan.{hint} Upgrade: {up}",
                label(kind),
                title(plan)
            ),
        );
    }
    AppError::new(
        StatusCode::PAYMENT_REQUIRED,
        "quota_exceeded",
        format!(
            "{} quota of the {} plan reached ({max} per month).{hint} Upgrade: {up}",
            label(kind),
            title(plan)
        ),
    )
}

// ---------- account state ----------

#[derive(Debug, Default, sqlx::FromRow)]
struct Account {
    stripe_customer_id: Option<String>,
    plan: String,
    status: String,
    stripe_subscription_id: Option<String>,
    base_item_id: Option<String>,
    metered: bool,
    current_period_end: Option<DateTime<Utc>>,
    cancel_at_period_end: bool,
    grace_until: Option<DateTime<Utc>>,
}

impl Account {
    fn effective(&self) -> &str {
        effective_plan(&self.plan, &self.status, self.grace_until, Utc::now())
    }
}

async fn account(db: &sqlx::PgPool, user: Uuid) -> ApiResult<Account> {
    Ok(sqlx::query_as(
        "SELECT stripe_customer_id, plan, status, stripe_subscription_id, base_item_id, metered,
                current_period_end, cancel_at_period_end, grace_until
         FROM billing_accounts WHERE user_id = $1",
    )
    .bind(user)
    .fetch_optional(db)
    .await?
    .unwrap_or(Account {
        plan: "free".into(),
        status: "none".into(),
        ..Default::default()
    }))
}

async fn owner(db: &sqlx::PgPool, project: Uuid) -> ApiResult<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM project_members WHERE project_id = $1 AND role = 'owner' LIMIT 1",
    )
    .bind(project)
    .fetch_optional(db)
    .await?)
}

async fn used(db: &sqlx::PgPool, user: Uuid, kind: &str) -> ApiResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(sum(n), 0)::int8 FROM billing_usage
         WHERE user_id = $1 AND kind = $2 AND month = date_trunc('month', now())::date",
    )
    .bind(user)
    .bind(kind)
    .fetch_one(db)
    .await?)
}

// ---------- enforcement (called by the cloud endpoints) ----------

/// Count one `kind` hit for the project's owner and refuse it (402) past the plan's monthly limit.
/// One round trip: the counter upsert and the plan lookup are one statement.
// ponytail: one DB write per hit, like the insights rate limit; batch in memory if CDN traffic outgrows it.
pub async fn consume(state: &AppState, project: Uuid, kind: &str) -> ApiResult<()> {
    consume_n(state, project, kind, 1).await
}

/// `consume` for `n` units at once (experiment beacon batches): one statement whatever `n`.
pub async fn consume_n(state: &AppState, project: Uuid, kind: &str, n: i64) -> ApiResult<()> {
    let Some(cfg) = &state.billing else {
        return Ok(());
    };
    let row: Option<(i64, Option<String>, Option<String>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "WITH o AS (SELECT user_id FROM project_members WHERE project_id = $1 AND role = 'owner' LIMIT 1),
              u AS (INSERT INTO billing_usage (user_id, month, kind, n)
                    SELECT user_id, date_trunc('month', now())::date, $2, $3 FROM o
                    ON CONFLICT (user_id, month, kind) DO UPDATE SET n = billing_usage.n + excluded.n
                    RETURNING user_id, n)
         SELECT u.n, b.plan, b.status, b.grace_until FROM u LEFT JOIN billing_accounts b ON b.user_id = u.user_id",
    )
    .bind(project)
    .bind(kind)
    .bind(n)
    .fetch_optional(&state.db)
    .await?;
    let Some((n, plan, status, grace)) = row else {
        return Ok(());
    };
    let plan = effective_plan(
        plan.as_deref().unwrap_or("free"),
        status.as_deref().unwrap_or("none"),
        grace,
        Utc::now(),
    );
    match cfg.limits(plan).get(kind) {
        Some(max) if n > max => Err(quota_error(cfg, plan, kind, max, "")),
        _ => Ok(()),
    }
}

/// Before starting a segmented experiment: `live` = experiments live once this one starts, `cells` = its segments.
pub async fn check_experiment(
    state: &AppState,
    project: Uuid,
    live: i64,
    cells: i64,
) -> ApiResult<()> {
    let Some(cfg) = &state.billing else {
        return Ok(());
    };
    let Some(user) = owner(&state.db, project).await? else {
        return Ok(());
    };
    let acct = account(&state.db, user).await?;
    let plan = acct.effective();
    let limits = cfg.limits(plan);
    let refuse = |what: &str, max: i64| {
        let up = cfg.upgrade_url();
        let (code, msg) = if max == 0 {
            (
                "plan_required",
                format!(
                    "{what} are not included in the {} plan. Upgrade: {up}",
                    title(plan)
                ),
            )
        } else {
            (
                "quota_exceeded",
                format!(
                    "{what}: the {} plan allows {max}. Upgrade: {up}",
                    title(plan)
                ),
            )
        };
        Err(AppError::new(StatusCode::PAYMENT_REQUIRED, code, msg))
    };
    match (limits.experiments, limits.xp_cells) {
        (Some(max), _) if live > max => refuse("Live experiments", max),
        (_, Some(max)) if cells > max => refuse("Experiment segments", max),
        _ => Ok(()),
    }
}

/// Before creating a project: the creator becomes its owner and pays for it.
pub async fn check_new_project(state: &AppState, user: Uuid) -> ApiResult<()> {
    let Some(cfg) = &state.billing else {
        return Ok(());
    };
    let acct = account(&state.db, user).await?;
    let plan = acct.effective();
    let Some(max) = cfg.limits(plan).projects else {
        return Ok(());
    };
    let owned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM project_members WHERE user_id = $1 AND role = 'owner'",
    )
    .bind(user)
    .fetch_one(&state.db)
    .await?;
    if owned >= max {
        return Err(AppError::new(
            StatusCode::PAYMENT_REQUIRED,
            "plan_limit",
            format!(
                "The {} plan includes {max} cloud project(s). The CLI and local MCP server stay free and unlimited. Upgrade: {}",
                title(plan),
                cfg.upgrade_url()
            ),
        ));
    }
    Ok(())
}

/// Server jobs without a project key run on glot's own LLM key (provider env var): that is what we bill.
pub fn uses_platform_key(p: &ProjectRow) -> bool {
    p.api_key_enc.is_none()
        && matches!(p.llm.0.provider.as_str(), "anthropic" | "openai")
        && p.llm.0.base_url.is_none()
}

/// Before queueing a translation job on glot's key: refuse once the plan's included strings are used,
/// unless the subscription carries the usage-based price (then the overage is billed, see `after_job`).
// ponytail: checked before the job, so one job can overshoot the allowance by its own size (bounded by llm.budget_usd).
pub async fn check_llm(state: &AppState, p: &ProjectRow) -> ApiResult<()> {
    let Some(cfg) = &state.billing else {
        return Ok(());
    };
    if !uses_platform_key(p) {
        return Ok(());
    }
    let Some(user) = owner(&state.db, p.id).await? else {
        return Ok(());
    };
    let acct = account(&state.db, user).await?;
    let plan = acct.effective();
    if acct.metered && plan != "free" {
        return Ok(());
    }
    match cfg.limits(plan).llm_strings {
        Some(max) if used(&state.db, user, LLM).await? >= max => Err(quota_error(
            cfg,
            plan,
            LLM,
            max,
            " Add your own LLM key (BYOK, never limited) in the project settings, or upgrade.",
        )),
        _ => Ok(()),
    }
}

/// After a job: count the strings it sent to glot's key and report the part above the plan's allowance to the
/// Billing Meter (identifier = job id, so a retried report is not billed twice).
pub async fn after_job(state: &AppState, project: Uuid, job: Uuid, strings: i64) {
    if let Err(e) = report_llm(state, project, job, strings).await {
        tracing::error!(%job, "billing usage report failed: {}", e.message);
    }
}

async fn report_llm(state: &AppState, project: Uuid, job: Uuid, strings: i64) -> ApiResult<()> {
    let Some(cfg) = &state.billing else {
        return Ok(());
    };
    if strings <= 0 {
        return Ok(());
    }
    let p: ProjectRow = sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at
         FROM projects WHERE id = $1",
    )
    .bind(project)
    .fetch_one(&state.db)
    .await?;
    if !uses_platform_key(&p) {
        return Ok(());
    }
    let Some(user) = owner(&state.db, project).await? else {
        return Ok(());
    };
    let n: i64 = sqlx::query_scalar(
        "INSERT INTO billing_usage (user_id, month, kind, n) VALUES ($1, date_trunc('month', now())::date, $2, $3)
         ON CONFLICT (user_id, month, kind) DO UPDATE SET n = billing_usage.n + excluded.n RETURNING n",
    )
    .bind(user)
    .bind(LLM)
    .bind(strings)
    .fetch_one(&state.db)
    .await?;
    let acct = account(&state.db, user).await?;
    let plan = acct.effective();
    let (Some(event), Some(customer), true) = (
        &cfg.meter_event,
        &acct.stripe_customer_id,
        acct.metered && plan != "free",
    ) else {
        return Ok(());
    };
    let included = cfg.limits(plan).llm_strings.unwrap_or(i64::MAX);
    let overage = strings.min(n - included);
    if overage > 0 {
        stripe(
            cfg,
            Method::POST,
            "/v1/billing/meter_events",
            &[
                ("event_name", event.clone()),
                ("payload[stripe_customer_id]", customer.clone()),
                ("payload[value]", overage.to_string()),
                ("identifier", format!("glot-job-{job}")),
            ],
            None,
        )
        .await?;
    }
    Ok(())
}

// ---------- Stripe HTTP ----------

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
});

async fn stripe(
    cfg: &Config,
    method: Method,
    path: &str,
    params: &[(&str, String)],
    idempotency_key: Option<&str>,
) -> ApiResult<Value> {
    let mut req = HTTP
        .request(method.clone(), format!("{}{path}", cfg.api_base))
        .basic_auth(&cfg.secret_key, None::<&str>)
        .header("Stripe-Version", API_VERSION);
    if let Some(k) = idempotency_key {
        req = req.header("Idempotency-Key", k);
    }
    req = if method == Method::GET {
        req.query(params)
    } else {
        req.form(params)
    };
    let res = req
        .send()
        .await
        .map_err(|e| AppError::internal(format!("stripe {path}: {e}")))?;
    let status = res.status();
    let v: Value = res
        .json()
        .await
        .map_err(|e| AppError::internal(format!("stripe {path}: {e}")))?;
    if !status.is_success() {
        tracing::error!(%status, path, error = %v["error"], "stripe request failed");
        return Err(AppError::new(
            StatusCode::BAD_GATEWAY,
            "stripe_error",
            v["error"]["message"]
                .as_str()
                .unwrap_or("payment provider error")
                .to_string(),
        ));
    }
    Ok(v)
}

/// Active prices by lookup key (the setup script names them; no price ids in .env).
async fn prices(cfg: &Config, keys: &[&str]) -> ApiResult<BTreeMap<String, Value>> {
    let mut q: Vec<(&str, String)> = keys
        .iter()
        .map(|k| ("lookup_keys[]", k.to_string()))
        .collect();
    q.push(("active", "true".into()));
    q.push(("limit", "20".into()));
    let v = stripe(cfg, Method::GET, "/v1/prices", &q, None).await?;
    Ok(v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["lookup_key"].as_str()?.to_string(), p.clone())))
        .collect())
}

/// Id of the active price with this lookup key (Stripe filters server-side, so the first match is it).
async fn price_id(cfg: &Config, lookup: &str) -> ApiResult<Option<String>> {
    let q = [
        ("lookup_keys[]", lookup.to_string()),
        ("active", "true".into()),
        ("limit", "1".into()),
    ];
    let v = stripe(cfg, Method::GET, "/v1/prices", &q, None).await?;
    Ok(v["data"][0]["id"].as_str().map(String::from))
}

fn enabled(state: &AppState) -> ApiResult<&Config> {
    state.billing.as_deref().ok_or_else(|| {
        AppError::not_found(
            "billing is not enabled on this server (self-hosted: nothing is limited)",
        )
    })
}

// ---------- webhook ----------

/// `Stripe-Signature: t=…,v1=…[,v1=…]` = HMAC-SHA256(secret, "{t}.{body}"); several v1 during a secret roll.
pub fn verify_signature(
    secret: &str,
    header: &str,
    body: &[u8],
    now: i64,
) -> Result<(), &'static str> {
    let mut t = None;
    let mut sigs = vec![];
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => sigs.push(v),
            _ => {}
        }
    }
    let t = t.ok_or("missing timestamp")?;
    if now - t > TOLERANCE_SECS {
        return Err("timestamp outside the tolerance window");
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| "bad secret")?;
    mac.update(format!("{t}.").as_bytes());
    mac.update(body);
    let ok = sigs
        .iter()
        .filter_map(|s| hex::decode(s).ok())
        .any(|s| mac.clone().verify_slice(&s).is_ok());
    if ok {
        Ok(())
    } else {
        Err("no matching signature")
    }
}

/// Header value for a payload (tests, local replays).
pub fn sign(secret: &str, body: &[u8], t: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(format!("{t}.").as_bytes());
    mac.update(body);
    format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
}

async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let cfg = enabled(&state)?;
    let sig = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    verify_signature(&cfg.webhook_secret, sig, &body, Utc::now().timestamp()).map_err(|e| {
        tracing::warn!("stripe webhook refused: {e}");
        AppError::bad_request(format!("invalid signature: {e}"))
    })?;
    let ev: Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid event: {e}")))?;
    let (Some(id), Some(ty)) = (ev["id"].as_str(), ev["type"].as_str()) else {
        return Err(AppError::bad_request("event without id/type"));
    };
    // Effects and the event id commit together: a crash mid-way leaves no id, so Stripe's retry re-applies it.
    let mut tx = state.db.begin().await?;
    let fresh =
        sqlx::query("INSERT INTO stripe_events (id, type) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(ty)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
    if !fresh {
        return Ok(StatusCode::OK);
    }
    let obj = &ev["data"]["object"];
    let customer = obj["customer"].as_str();
    match ty {
        "checkout.session.completed" => {
            if let (Some(user), Some(cus)) = (
                obj["client_reference_id"]
                    .as_str()
                    .and_then(|u| u.parse::<Uuid>().ok()),
                customer,
            ) {
                sqlx::query(
                    "INSERT INTO billing_accounts (user_id, stripe_customer_id) VALUES ($1, $2)
                     ON CONFLICT (user_id) DO UPDATE SET stripe_customer_id = excluded.stripe_customer_id, updated_at = now()",
                )
                .bind(user)
                .bind(cus)
                .execute(&mut *tx)
                .await?;
            }
        }
        "customer.subscription.created"
        | "customer.subscription.updated"
        | "customer.subscription.deleted" => {
            apply_subscription(
                &mut tx,
                obj,
                ev["created"].as_i64().unwrap_or(0),
                ty.ends_with("deleted"),
            )
            .await?;
        }
        "invoice.paid" => {
            sqlx::query("UPDATE billing_accounts SET grace_until = NULL, updated_at = now() WHERE stripe_customer_id = $1")
                .bind(customer)
                .execute(&mut *tx)
                .await?;
        }
        "invoice.payment_failed" => {
            let days = state.billing.as_ref().map_or(7, |c| c.grace_days);
            sqlx::query(
                "UPDATE billing_accounts SET grace_until = coalesce(grace_until, now() + make_interval(days => $2)), updated_at = now()
                 WHERE stripe_customer_id = $1",
            )
            .bind(customer)
            .bind(days as i32)
            .execute(&mut *tx)
            .await?;
        }
        _ => {}
    }
    tx.commit().await?;
    tracing::info!(event = id, r#type = ty, "stripe event applied");
    Ok(StatusCode::OK)
}

/// Plan of a subscription item price: `metadata.glot_plan`, else the lookup key (`glot_pro_month` → pro).
fn plan_of(price: &Value) -> Option<&'static str> {
    let tag = price["metadata"]["glot_plan"]
        .as_str()
        .or(price["lookup_key"].as_str())?;
    ["team", "pro"]
        .into_iter()
        .find(|p| tag == *p || tag.starts_with(&format!("glot_{p}")))
}

async fn apply_subscription(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sub: &Value,
    created: i64,
    deleted: bool,
) -> ApiResult<()> {
    let customer = sub["customer"].as_str().unwrap_or("");
    let user: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM billing_accounts WHERE stripe_customer_id = $1")
            .bind(customer)
            .fetch_optional(&mut **tx)
            .await?;
    let Some(user) = user.or_else(|| sub["metadata"]["glot_user_id"].as_str()?.parse().ok()) else {
        tracing::warn!(customer, "subscription for an unknown customer, ignored");
        return Ok(());
    };
    let items = sub["items"]["data"].as_array().cloned().unwrap_or_default();
    let base = items.iter().find(|i| plan_of(&i["price"]).is_some());
    let metered = items
        .iter()
        .any(|i| i["price"]["recurring"]["usage_type"] == "metered");
    // Since 2025-03-31.basil the period lives on the items; older payloads have it on the subscription.
    let period_end = base
        .and_then(|i| i["current_period_end"].as_i64())
        .or(sub["current_period_end"].as_i64())
        .and_then(|t| DateTime::from_timestamp(t, 0));
    let (plan, status) = if deleted {
        ("free", "canceled")
    } else {
        (
            base.and_then(|i| plan_of(&i["price"])).unwrap_or("free"),
            sub["status"].as_str().unwrap_or("none"),
        )
    };
    // Ignore stale events (Stripe does not guarantee order) and the deletion of a subscription that was replaced.
    sqlx::query(
        "INSERT INTO billing_accounts AS b (user_id, stripe_customer_id, plan, status, stripe_subscription_id, base_item_id,
                metered, current_period_end, cancel_at_period_end, sub_event_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (user_id) DO UPDATE SET stripe_customer_id = excluded.stripe_customer_id, plan = excluded.plan,
                status = excluded.status, stripe_subscription_id = excluded.stripe_subscription_id,
                base_item_id = excluded.base_item_id, metered = excluded.metered,
                current_period_end = excluded.current_period_end, cancel_at_period_end = excluded.cancel_at_period_end,
                sub_event_at = excluded.sub_event_at, updated_at = now(),
                grace_until = CASE WHEN excluded.status IN ('active', 'trialing') THEN NULL ELSE b.grace_until END
         WHERE (b.sub_event_at IS NULL OR b.sub_event_at <= excluded.sub_event_at)
           AND NOT ($11 AND b.stripe_subscription_id IS DISTINCT FROM excluded.stripe_subscription_id)",
    )
    .bind(user)
    .bind(customer)
    .bind(plan)
    .bind(status)
    .bind(sub["id"].as_str())
    .bind(base.and_then(|i| i["id"].as_str()))
    .bind(metered && !deleted)
    .bind(period_end)
    .bind(sub["cancel_at_period_end"].as_bool().unwrap_or(false))
    .bind(created)
    .bind(deleted)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ---------- dashboard API ----------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/billing", get(summary))
        .route("/api/billing/checkout", post(checkout))
        .route("/api/billing/portal", post(portal))
        .route("/api/billing/invoices", get(invoices))
}

/// Stripe calls the webhook server-to-server; the pricing page reads plans without a session.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/billing/webhook", post(webhook))
        .route("/api/billing/plans", get(plans))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Summary {
    pub enabled: bool,
    /// Plan in force now (free while a failed payment is past its grace period).
    pub plan: String,
    pub subscribed_plan: String,
    pub status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub cancel_at_period_end: bool,
    pub grace_until: Option<DateTime<Utc>>,
    pub metered: bool,
    pub has_customer: bool,
    pub limits: Limits,
    /// Same keys as `limits`, this calendar month (UTC).
    pub usage: BTreeMap<String, i64>,
}

async fn summary(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> ApiResult<Json<Summary>> {
    let mut usage: BTreeMap<String, i64> = sqlx::query_as(
        "SELECT kind, n FROM billing_usage WHERE user_id = $1 AND month = date_trunc('month', now())::date",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let projects: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM project_members WHERE user_id = $1 AND role = 'owner'",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    usage.insert("projects".into(), projects);
    let Some(cfg) = &state.billing else {
        return Ok(Json(Summary {
            enabled: false,
            plan: "self_hosted".into(),
            subscribed_plan: "self_hosted".into(),
            status: "none".into(),
            current_period_end: None,
            cancel_at_period_end: false,
            grace_until: None,
            metered: false,
            has_customer: false,
            limits: Limits::default(),
            usage,
        }));
    };
    let a = account(&state.db, user.id).await?;
    let plan = a.effective().to_string();
    Ok(Json(Summary {
        enabled: true,
        limits: cfg.limits(&plan),
        plan,
        subscribed_plan: a.plan.clone(),
        status: a.status.clone(),
        current_period_end: a.current_period_end,
        cancel_at_period_end: a.cancel_at_period_end,
        grace_until: a.grace_until,
        metered: a.metered,
        has_customer: a.stripe_customer_id.is_some(),
        usage,
    }))
}

async fn ensure_customer(
    state: &AppState,
    cfg: &Config,
    user: &crate::auth::User,
) -> ApiResult<String> {
    if let Some(c) = account(&state.db, user.id).await?.stripe_customer_id {
        return Ok(c);
    }
    let c = stripe(
        cfg,
        Method::POST,
        "/v1/customers",
        &[
            ("email", user.email.clone()),
            ("name", user.name.clone()),
            ("metadata[glot_user_id]", user.id.to_string()),
        ],
        Some(&format!("glot-customer-{}", user.id)),
    )
    .await?;
    let id = c["id"]
        .as_str()
        .ok_or_else(|| AppError::internal("stripe customer without id"))?;
    // A concurrent request may have linked one first: keep the first.
    Ok(sqlx::query_scalar(
        "INSERT INTO billing_accounts (user_id, stripe_customer_id) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET stripe_customer_id = coalesce(billing_accounts.stripe_customer_id, excluded.stripe_customer_id)
         RETURNING stripe_customer_id",
    )
    .bind(user.id)
    .bind(id)
    .fetch_one(&state.db)
    .await?)
}

#[derive(Deserialize)]
pub struct CheckoutBody {
    plan: String,
    #[serde(default = "month")]
    interval: String,
}

fn month() -> String {
    "month".into()
}

/// `{url}` to redirect to: a Checkout Session, or (existing subscription) the billing page after a prorated swap.
async fn checkout(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppJson(b): AppJson<CheckoutBody>,
) -> ApiResult<Json<Value>> {
    let cfg = enabled(&state)?;
    if !matches!(b.plan.as_str(), "pro" | "team")
        || !matches!(b.interval.as_str(), "month" | "year")
    {
        return Err(AppError::bad_request(
            "plan must be pro|team, interval month|year",
        ));
    }
    let lookup = format!("glot_{}_{}", b.plan, b.interval);
    let price = price_id(cfg, &lookup).await?.ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "billing_not_configured",
            format!("no active Stripe price with lookup key {lookup} (run deploy/stripe-setup.sh)"),
        )
    })?;
    let billing_page = cfg.upgrade_url();

    let a = account(&state.db, user.id).await?;
    if matches!(a.status.as_str(), "active" | "trialing" | "past_due")
        && let (Some(sub), Some(item)) = (&a.stripe_subscription_id, &a.base_item_id)
    {
        stripe(
            cfg,
            Method::POST,
            &format!("/v1/subscriptions/{sub}"),
            &[
                ("items[0][id]", item.clone()),
                ("items[0][price]", price),
                ("proration_behavior", "create_prorations".into()),
            ],
            None,
        )
        .await?;
        return Ok(Json(json!({ "url": format!("{billing_page}?changed=1") })));
    }

    let customer = ensure_customer(&state, cfg, &user).await?;
    let mut form = vec![
        ("mode", "subscription".to_string()),
        ("customer", customer),
        ("client_reference_id", user.id.to_string()),
        ("line_items[0][price]", price),
        ("line_items[0][quantity]", "1".into()),
        ("success_url", format!("{billing_page}?checkout=success")),
        ("cancel_url", format!("{billing_page}?checkout=cancel")),
        (
            "subscription_data[metadata][glot_user_id]",
            user.id.to_string(),
        ),
        ("allow_promotion_codes", "true".into()),
    ];
    // ponytail: usage price is monthly, so only monthly plans carry it (yearly plans stop at their allowance);
    // add a yearly-interval meter price if yearly customers ask for overage.
    if cfg.meter_event.is_some()
        && b.interval == "month"
        && let Some(m) = price_id(cfg, "glot_llm_strings").await?
    {
        form.push(("line_items[1][price]", m));
    }
    if cfg.automatic_tax {
        form.extend([
            ("automatic_tax[enabled]", "true".into()),
            ("customer_update[address]", "auto".into()),
            ("customer_update[name]", "auto".into()),
            ("billing_address_collection", "required".into()),
            ("tax_id_collection[enabled]", "true".into()),
        ]);
    }
    let s = stripe(cfg, Method::POST, "/v1/checkout/sessions", &form, None).await?;
    Ok(Json(json!({ "url": s["url"] })))
}

/// Customer Portal: payment method, invoices, cancellation, tax ids.
async fn portal(State(state): State<AppState>, AuthUser(user): AuthUser) -> ApiResult<Json<Value>> {
    let cfg = enabled(&state)?;
    let customer = ensure_customer(&state, cfg, &user).await?;
    let mut form = vec![("customer", customer), ("return_url", cfg.upgrade_url())];
    if let Some(c) = &cfg.portal_configuration {
        form.push(("configuration", c.clone()));
    }
    let s = stripe(
        cfg,
        Method::POST,
        "/v1/billing_portal/sessions",
        &form,
        None,
    )
    .await?;
    Ok(Json(json!({ "url": s["url"] })))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Invoice {
    pub id: String,
    pub number: Option<String>,
    pub status: Option<String>,
    /// Minor units (cents).
    pub total: i64,
    pub currency: String,
    pub created: Option<DateTime<Utc>>,
    pub hosted_invoice_url: Option<String>,
    pub invoice_pdf: Option<String>,
}

async fn invoices(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> ApiResult<Json<Vec<Invoice>>> {
    let cfg = enabled(&state)?;
    let Some(customer) = account(&state.db, user.id).await?.stripe_customer_id else {
        return Ok(Json(vec![]));
    };
    let v = stripe(
        cfg,
        Method::GET,
        "/v1/invoices",
        &[("customer", customer), ("limit", "24".into())],
        None,
    )
    .await?;
    let s = |v: &Value| v.as_str().map(String::from);
    Ok(Json(
        v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| i["status"] != "draft")
            .map(|i| Invoice {
                id: i["id"].as_str().unwrap_or_default().into(),
                number: s(&i["number"]),
                status: s(&i["status"]),
                total: i["total"].as_i64().unwrap_or(0),
                currency: i["currency"].as_str().unwrap_or("eur").into(),
                created: i["created"]
                    .as_i64()
                    .and_then(|t| DateTime::from_timestamp(t, 0)),
                hosted_invoice_url: s(&i["hosted_invoice_url"]),
                invoice_pdf: s(&i["invoice_pdf"]),
            })
            .collect(),
    ))
}

const PRICE_KEYS: [&str; 5] = [
    "glot_pro_month",
    "glot_pro_year",
    "glot_team_month",
    "glot_team_year",
    "glot_llm_strings",
];
static PLANS_CACHE: LazyLock<Mutex<Option<(Instant, Value)>>> = LazyLock::new(|| Mutex::new(None));

/// Public: limits (server config) + prices (Stripe, the single source of truth), cached 10 min.
async fn plans(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let Some(cfg) = &state.billing else {
        return Ok(Json(json!({ "enabled": false, "plans": {}, "prices": {} })));
    };
    let lock = || PLANS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, v)) = lock().as_ref()
        && at.elapsed() < Duration::from_secs(600)
    {
        return Ok(Json(v.clone()));
    }
    let found = prices(cfg, &PRICE_KEYS).await?;
    let prices: BTreeMap<&String, Value> = found
        .iter()
        .map(|(k, p)| {
            (
                k,
                json!({ "unit_amount": p["unit_amount"], "unit_amount_decimal": p["unit_amount_decimal"],
                        "currency": p["currency"], "interval": p["recurring"]["interval"] }),
            )
        })
        .collect();
    let v = json!({ "enabled": true, "plans": cfg.plans, "prices": prices });
    *lock() = Some((Instant::now(), v.clone()));
    Ok(Json(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_roundtrip_and_tolerance() {
        let body = br#"{"id":"evt_1"}"#;
        let h = sign("whsec_x", body, 1_000);
        assert!(verify_signature("whsec_x", &h, body, 1_100).is_ok());
        assert!(verify_signature("whsec_y", &h, body, 1_100).is_err());
        assert!(verify_signature("whsec_x", &h, b"{}", 1_100).is_err());
        assert!(verify_signature("whsec_x", &h, body, 1_000 + TOLERANCE_SECS + 1).is_err());
        // secret roll: any matching v1 is accepted
        let rolled = format!("{},v1=00ff", h);
        assert!(verify_signature("whsec_x", &rolled, body, 1_000).is_ok());
        assert!(verify_signature("whsec_x", "v1=abc", body, 1_000).is_err());
    }

    #[test]
    fn grace_and_plans() {
        let now = Utc::now();
        let past = Some(now - chrono::Duration::days(1));
        let future = Some(now + chrono::Duration::days(1));
        assert_eq!(effective_plan("pro", "active", None, now), "pro");
        assert_eq!(effective_plan("pro", "past_due", future, now), "pro");
        assert_eq!(effective_plan("pro", "past_due", past, now), "free");
        assert_eq!(effective_plan("pro", "unpaid", None, now), "free");
        assert_eq!(effective_plan("team", "canceled", None, now), "free");
        assert_eq!(
            plan_of(&json!({"lookup_key": "glot_team_year"})),
            Some("team")
        );
        assert_eq!(plan_of(&json!({"lookup_key": "glot_llm_strings"})), None);
        assert_eq!(
            plan_of(&json!({"metadata": {"glot_plan": "pro"}})),
            Some("pro")
        );
    }
}
