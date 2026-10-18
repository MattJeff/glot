//! Copy experiments ("the translation that converts best"): for one high-stakes key and one locale, the LLM writes
//! 2-3 validated variants of the official translation; `@glot/insights` serves them by Thompson sampling and
//! attributes the goal event to the arm; a variant that wins with P(best) > 95 % (Beta posteriors, min n per arm)
//! is proposed for promotion to the official translation. Only aggregate counters are stored (no visitor id).
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use chrono::{DateTime, Utc};
use glot_core::{
    config::LlmConfig,
    measure::{self, Fit},
    validate::{Issue, Syntax, has_errors},
};
use glot_llm::provider::CompletionRequest;
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    keys,
    projects::{self, ProjectRow, Role},
    validate_locale,
};

/// Posterior probability a winner needs before it is proposed.
pub const WIN_PROB: f64 = 0.95;

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/experiments";
    Router::new()
        .route(p, axum::routing::get(list).post(create))
        .route(&format!("{p}/{{id}}/promote"), post(promote))
        .route(&format!("{p}/{{id}}/stop"), post(stop))
}

// ---------- Bayesian core ----------

pub(crate) fn normal(rng: &mut impl Rng) -> f64 {
    // Box-Muller; 1 - u keeps ln away from 0.
    let (u, v): (f64, f64) = (1.0 - rng.random::<f64>(), rng.random());
    (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
}

/// Gamma(shape, 1), Marsaglia-Tsang.
pub(crate) fn gamma(shape: f64, rng: &mut impl Rng) -> f64 {
    if shape < 1.0 {
        return gamma(shape + 1.0, rng) * rng.random::<f64>().powf(1.0 / shape);
    }
    let d = shape - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    loop {
        let x = normal(rng);
        let v = (1.0 + c * x).powi(3);
        if v <= 0.0 {
            continue;
        }
        let u: f64 = rng.random();
        if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
            return d * v;
        }
    }
}

/// One draw of the Beta(1 + conversions, 1 + misses) posterior of an arm.
fn draw(views: i64, conversions: i64, rng: &mut impl Rng) -> f64 {
    let c = conversions.clamp(0, views.max(0)) as f64;
    let (a, b) = (1.0 + c, 1.0 + views.max(0) as f64 - c);
    if a.min(b) > 50.0 {
        // Both shapes large: Beta ≈ Normal (error far below the Monte-Carlo noise), ~4x cheaper.
        let (n, m) = (a + b, a / (a + b));
        return (m + normal(rng) * (m * (1.0 - m) / (n + 1.0)).sqrt()).clamp(0.0, 1.0);
    }
    let x = gamma(a, rng);
    x / (x + gamma(b, rng))
}

/// Thompson sampling: index of the arm whose posterior draw is highest.
pub fn thompson(arms: &[(i64, i64)], rng: &mut impl Rng) -> usize {
    let mut best = (0, f64::MIN);
    for (i, (v, c)) in arms.iter().enumerate() {
        let x = draw(*v, *c, rng);
        if x > best.1 {
            best = (i, x);
        }
    }
    best.0
}

/// Burn-in, then top-two Thompson sampling. Arms under `min_n` visitors get uniform traffic first, so no arm is
/// judged before it was seen (and the first look is on a fixed sample). Then half the traffic goes to the Thompson
/// leader and half to a challenger: plain Thompson starves losers, their posteriors stay wide and P(best) never
/// clears the bar (best-arm identification, Russo 2016).
pub fn allocate(arms: &[(i64, i64)], min_n: i64, rng: &mut impl Rng) -> usize {
    let young: Vec<usize> = (0..arms.len()).filter(|i| arms[*i].0 < min_n).collect();
    if !young.is_empty() {
        return young[rng.random_range(0..young.len())];
    }
    let leader = thompson(arms, rng);
    if rng.random::<bool>() {
        return leader;
    }
    // Challenger: best fresh posterior draw among the other arms.
    let mut best = (leader, f64::MIN);
    for (i, (v, c)) in arms.iter().enumerate().filter(|(i, _)| *i != leader) {
        let x = draw(*v, *c, rng);
        if x > best.1 {
            best = (i, x);
        }
    }
    best.0
}

/// Monte-Carlo P(arm is best) for each arm, plus P(arm beats arm 0) and the mean relative lift over arm 0.
pub fn posterior(arms: &[(i64, i64)], draws: usize, rng: &mut impl Rng) -> Vec<(f64, f64, f64)> {
    let mut best = vec![0usize; arms.len()];
    let mut beats = vec![0usize; arms.len()];
    let mut lift = vec![0f64; arms.len()];
    let mut x = vec![0f64; arms.len()];
    for _ in 0..draws {
        for (i, (v, c)) in arms.iter().enumerate() {
            x[i] = draw(*v, *c, rng);
        }
        let top = (0..x.len()).fold(0, |b, i| if x[i] > x[b] { i } else { b });
        best[top] += 1;
        for i in 0..x.len() {
            beats[i] += (x[i] > x[0]) as usize;
            lift[i] += x[i] / x[0] - 1.0;
        }
    }
    let n = draws as f64;
    (0..arms.len())
        .map(|i| (best[i] as f64 / n, beats[i] as f64 / n, lift[i] / n))
        .collect()
}

/// Total visitors at which look number `looks` happens: arms·min_n·2^looks. The first look is right after the
/// burn-in (every arm at `min_n`), then each time traffic doubles — a handful of looks, not one per conversion.
pub fn next_look(arms: usize, min_n: i64, looks: i32) -> i64 {
    arms as i64 * min_n * 2i64.saturating_pow(looks.clamp(0, 40) as u32)
}

/// Winner at look `look` (0-based): an arm best with probability > 1 − (1 − WIN_PROB)/(arms·2^(look+1)).
/// The error budget (1 − WIN_PROB = 5 %) is split across arms (any of them could win by luck) and spent over
/// looks (½, ¼, …), so peeking repeatedly stays near 5 % false winners in an A/A test (see the simulation test).
pub fn decide(arms: &[(i64, i64)], min_n: i64, look: i32, rng: &mut impl Rng) -> Option<usize> {
    if arms.len() < 2 || arms.iter().any(|a| a.0 < min_n) {
        return None;
    }
    let threshold = 1.0 - (1.0 - WIN_PROB) / (arms.len() as f64 * 2f64.powi(look.clamp(0, 30) + 1));
    posterior(arms, 4000, rng)
        .iter()
        .position(|p| p.0 > threshold)
}

// ---------- serving (called from insights ingestion) ----------

#[derive(Serialize, Debug, PartialEq)]
pub struct Assigned {
    pub id: Uuid,
    pub key: String,
    pub arm: i32,
    pub value: String,
}

/// A visit draws an arm for every live experiment of the served locale (within its traffic budget); a goal event
/// credits the arms this session was assigned. Returns the new assignments (visits only).
pub async fn on_hit(
    state: &AppState,
    project_id: Uuid,
    served: &str,
    event: &str,
    assigned: &BTreeMap<Uuid, i32>,
) -> ApiResult<Vec<Assigned>> {
    let mut out = vec![];
    if event == "$view" {
        // A changed source text stops serving: variants were validated against the old one.
        let exps: Vec<(Uuid, String, i32, i32, String, Option<i32>)> = sqlx::query_as(
            "SELECT e.id, e.key, e.explore_pct, e.min_n, e.status, e.winner FROM variant_experiments e
             JOIN keys k ON k.project_id = e.project_id AND k.key = e.key AND k.source = e.source
             WHERE e.project_id = $1 AND e.locale = $2 AND e.status IN ('running', 'decided')",
        )
        .bind(project_id)
        .bind(served)
        .fetch_all(&state.db)
        .await?;
        for (id, key, pct, min_n, status, winner) in exps {
            if rand::random::<u32>() % 100 >= pct as u32 {
                continue; // outside the explored traffic: official translation, not counted
            }
            let arms: Vec<(i32, String, i64, i64)> = sqlx::query_as(
                "SELECT arm, value, views, conversions FROM variant_arms WHERE experiment_id = $1 ORDER BY arm",
            )
            .bind(id)
            .fetch_all(&state.db)
            .await?;
            let i = match (status.as_str(), winner) {
                ("decided", Some(w)) => arms.iter().position(|a| a.0 == w).unwrap_or(0),
                _ => {
                    let counts: Vec<(i64, i64)> = arms.iter().map(|a| (a.2, a.3)).collect();
                    allocate(&counts, min_n as i64, &mut rand::rng())
                }
            };
            let Some((arm, value, ..)) = arms.into_iter().nth(i) else {
                continue;
            };
            if status == "running" {
                sqlx::query("UPDATE variant_arms SET views = views + 1 WHERE experiment_id = $1 AND arm = $2")
                    .bind(id)
                    .bind(arm)
                    .execute(&state.db)
                    .await?;
            }
            out.push(Assigned {
                id,
                key,
                arm,
                value,
            });
        }
        return Ok(out);
    }
    // ponytail: at most 20 experiments credited per event (bounded by the 4 kB body anyway).
    for (id, arm) in assigned.iter().take(20) {
        let hit: Option<i64> = sqlx::query_scalar(
            "UPDATE variant_arms a SET conversions = conversions + 1 FROM variant_experiments e
             WHERE e.id = a.experiment_id AND a.experiment_id = $1 AND a.arm = $2 AND e.project_id = $3
               AND e.goal = $4 AND e.status = 'running' RETURNING a.views",
        )
        .bind(id)
        .bind(arm)
        .bind(project_id)
        .bind(event)
        .fetch_optional(&state.db)
        .await?;
        if hit.is_some() {
            evaluate(state, *id).await?;
        }
    }
    Ok(out)
}

/// Stops a running experiment once it has a winner: losers stop being served, the winner awaits promotion.
async fn evaluate(state: &AppState, id: Uuid) -> ApiResult<Option<i32>> {
    let (min_n, looks): (i32, i32) =
        sqlx::query_as("SELECT min_n, looks FROM variant_experiments WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let arms: Vec<(i32, i64, i64)> = sqlx::query_as(
        "SELECT arm, views, conversions FROM variant_arms WHERE experiment_id = $1 ORDER BY arm",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let total: i64 = arms.iter().map(|a| a.1).sum();
    if total < next_look(arms.len(), min_n as i64, looks) {
        return Ok(None);
    }
    // Claim this look atomically: concurrent conversions never spend the same look twice.
    let claimed = sqlx::query("UPDATE variant_experiments SET looks = looks + 1 WHERE id = $1 AND looks = $2 AND status = 'running'")
        .bind(id)
        .bind(looks)
        .execute(&state.db)
        .await?
        .rows_affected()
        == 1;
    let counts: Vec<(i64, i64)> = arms.iter().map(|a| (a.1, a.2)).collect();
    let winner = claimed
        .then(|| decide(&counts, min_n as i64, looks, &mut rand::rng()))
        .flatten();
    let Some(w) = winner.map(|i| arms[i].0) else {
        return Ok(None);
    };
    sqlx::query(
        "UPDATE variant_experiments SET status = 'decided', winner = $2, decided_at = now()
         WHERE id = $1 AND status = 'running'",
    )
    .bind(id)
    .bind(w)
    .execute(&state.db)
    .await?;
    Ok(Some(w))
}

// ---------- generation ----------

#[derive(Deserialize)]
struct Candidate {
    text: String,
    #[serde(default)]
    tone: String,
}

const TONES: [&str; 3] = ["direct", "warm", "urgent"];

fn prompt(
    p: &ProjectRow,
    k: &keys::KeyRecord,
    locale: &str,
    current: &str,
    n: usize,
    fit: Option<&Fit>,
) -> (String, String) {
    let system = format!(
        "You are a senior conversion copywriter and native {locale} localizer. Rewrite a UI string in {locale} \
         so it converts better, keeping exactly the same meaning and promise. Each variant must use a clearly \
         different tone ({}). Keep every placeholder, ICU plural/select and tag exactly as in the source. \
         Never add claims, prices or urgency the source does not make.{}{}\n\
         Answer with JSON only: {{\"variants\": [{{\"text\": \"…\", \"tone\": \"…\"}}]}}",
        TONES[..n].join(", "),
        p.style
            .as_deref()
            .map(|s| format!(" Product style guide: {s}."))
            .unwrap_or_default(),
        fit.map(|f| format!(" Space budget: {}.", measure::hint(f)))
            .unwrap_or_default(),
    );
    let user = serde_json::json!({
        "task": "variants",
        "key": k.key,
        "source_locale": p.source_locale,
        "target_locale": locale,
        "source": k.source,
        "current_translation": current,
        "description": k.description,
        "ui_role": k.ui_role,
        "count": n,
    })
    .to_string();
    (system, user)
}

async fn generate(
    state: &AppState,
    p: &ProjectRow,
    k: &keys::KeyRecord,
    locale: &str,
    current: &str,
    n: usize,
    fit: Option<&Fit>,
) -> ApiResult<Vec<Candidate>> {
    let llm = &p.llm.0;
    if llm.provider == "mock" {
        // Offline/tests: deterministic "same meaning, other tone" rewrites of the official translation.
        return Ok(TONES[..n]
            .iter()
            .map(|t| Candidate {
                text: format!("{current} ({t})"),
                tone: t.to_string(),
            })
            .collect());
    }
    // ponytail: same key decryption as review.rs / jobs.rs; share a helper when it changes.
    let api_key = match &p.api_key_enc {
        Some(enc) => Some(crate::auth::decrypt(&state.secret, enc).ok_or_else(|| {
            AppError::internal("cannot decrypt the project API key (GLOT_SECRET_KEY changed?)")
        })?),
        None => None,
    };
    let llm_err = |e: glot_llm::Error| {
        AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", e.to_string())
    };
    let cfg = LlmConfig {
        provider: llm.provider.clone(),
        model: llm.model.clone(),
        base_url: llm.base_url.clone(),
        ..LlmConfig::default()
    };
    let provider =
        glot_llm::provider::from_config_with(&cfg, api_key, &llm.headers()).map_err(llm_err)?;
    let (system, user) = prompt(p, k, locale, current, n, fit);
    let c = provider
        .complete(CompletionRequest {
            model: llm.model.clone(),
            system,
            user,
            max_tokens: 1024,
            json: true,
            images: vec![],
        })
        .await
        .map_err(llm_err)?;
    let json = c
        .text
        .find('{')
        .zip(c.text.rfind('}'))
        .map(|(a, b)| &c.text[a..=b])
        .unwrap_or("{}");
    #[derive(Deserialize)]
    struct Out {
        variants: Vec<Candidate>,
    }
    serde_json::from_str::<Out>(json)
        .map(|o| o.variants)
        .map_err(|e| {
            AppError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "llm_error",
                format!("unparsable variants: {e}"),
            )
        })
}

// ---------- dashboard ----------

#[derive(Serialize, utoipa::ToSchema)]
pub struct ArmView {
    pub arm: i32,
    pub value: String,
    pub tone: String,
    pub is_control: bool,
    pub views: i64,
    pub conversions: i64,
    pub rate: f64,
    /// Beta posterior parameters (uniform prior): 1 + conversions, 1 + views − conversions.
    pub alpha: f64,
    pub beta: f64,
    pub p_best: f64,
    /// P(this arm converts better than the official translation).
    pub p_beats_control: f64,
    /// Posterior mean of the relative lift over the official translation.
    pub lift: f64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct VariantExperiment {
    pub id: Uuid,
    pub key: String,
    pub locale: String,
    pub source: String,
    /// The key's source changed since generation: variants are no longer served.
    pub source_changed: bool,
    pub goal: String,
    pub explore_pct: i32,
    pub min_n: i32,
    /// running | decided | promoted | stopped
    pub status: String,
    pub winner: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub arms: Vec<ArmView>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Rejected {
    pub text: String,
    pub tone: String,
    #[schema(value_type = Vec<Object>)]
    pub issues: Vec<Issue>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Created {
    #[serde(flatten)]
    pub experiment: VariantExperiment,
    /// Candidates that failed validation (never served).
    pub rejected: Vec<Rejected>,
}

async fn load(
    state: &AppState,
    project_id: Uuid,
    id: Option<Uuid>,
) -> ApiResult<Vec<VariantExperiment>> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        String,
        i32,
        i32,
        String,
        Option<i32>,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        bool,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT e.id, e.key, e.locale, e.source, e.goal, e.explore_pct, e.min_n, e.status, e.winner, e.created_at,
                e.decided_at, coalesce(k.source <> e.source, true)
         FROM variant_experiments e LEFT JOIN keys k ON k.project_id = e.project_id AND k.key = e.key
         WHERE e.project_id = $1 AND ($2::uuid IS NULL OR e.id = $2)
         ORDER BY (e.status IN ('running', 'decided')) DESC, e.created_at DESC LIMIT 50",
    )
    .bind(project_id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let mut out = vec![];
    for (
        id,
        key,
        locale,
        source,
        goal,
        explore_pct,
        min_n,
        status,
        winner,
        created_at,
        decided_at,
        source_changed,
    ) in rows
    {
        let arms: Vec<(i32, String, String, i64, i64)> = sqlx::query_as(
            "SELECT arm, value, tone, views, conversions FROM variant_arms WHERE experiment_id = $1 ORDER BY arm",
        )
        .bind(id)
        .fetch_all(&state.db)
        .await?;
        let counts: Vec<(i64, i64)> = arms.iter().map(|a| (a.3, a.4.min(a.3))).collect();
        // Seeded: the dashboard shows stable numbers between refreshes.
        let post = posterior(
            &counts,
            20_000,
            &mut StdRng::seed_from_u64(id.as_u128() as u64),
        );
        out.push(VariantExperiment {
            arms: arms
                .into_iter()
                .zip(counts)
                .zip(post)
                .map(
                    |(((arm, value, tone, ..), (v, c)), (p_best, p_beats_control, lift))| ArmView {
                        is_control: arm == 0,
                        rate: if v > 0 { c as f64 / v as f64 } else { 0.0 },
                        alpha: 1.0 + c as f64,
                        beta: 1.0 + (v - c) as f64,
                        p_best,
                        p_beats_control,
                        lift,
                        arm,
                        value,
                        tone,
                        views: v,
                        conversions: c,
                    },
                )
                .collect(),
            id,
            key,
            locale,
            source,
            source_changed,
            goal,
            explore_pct,
            min_n,
            status,
            winner,
            created_at,
            decided_at,
        });
    }
    Ok(out)
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<VariantExperiment>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(load(&state, p.id, None).await?))
}

#[derive(Deserialize)]
pub struct NewExperiment {
    key: String,
    locale: String,
    /// 2-3 (default 3).
    variants: Option<usize>,
    /// Share of visitors in the experiment, 1-100 (default 100).
    explore_pct: Option<i32>,
    /// Visitors per arm before a winner can be declared (default 500).
    min_n: Option<i32>,
    goal: Option<String>,
    /// Layout budget from `.glot/layout.json` (`glot measure`): variants that would not fit are rejected.
    fit: Option<Fit>,
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<NewExperiment>,
) -> ApiResult<(StatusCode, Json<Created>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    validate_locale(&b.locale)?;
    if !p.target_locales.contains(&b.locale) {
        return Err(AppError::bad_request(format!(
            "'{}' is not a target locale of this project",
            b.locale
        )));
    }
    let n = b.variants.unwrap_or(3);
    let explore_pct = b.explore_pct.unwrap_or(100);
    let min_n = b.min_n.unwrap_or(500);
    let goal = b.goal.unwrap_or_else(|| "signup".into());
    if !(2..=3).contains(&n)
        || !(1..=100).contains(&explore_pct)
        || !(50..=1_000_000).contains(&min_n)
    {
        return Err(AppError::bad_request(
            "variants must be 2-3, explore_pct 1-100, min_n 50-1000000",
        ));
    }
    if !(1..=40).contains(&goal.len())
        || !goal
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
    {
        return Err(AppError::bad_request("invalid goal event name"));
    }
    let mut conn = state.db.acquire().await?;
    let k = keys::find_key(&mut conn, p.id, &b.key).await?;
    drop(conn);
    if crate::experiments::live_key_owner(&state.db, p.id, &k.key)
        .await?
        .is_some()
    {
        return Err(AppError::conflict(
            "this key is live in a segmented experiment",
        ));
    }
    let current: String = sqlx::query_scalar(
        "SELECT value FROM translations WHERE key_id = $1 AND locale = $2 AND value <> ''",
    )
    .bind(k.id)
    .bind(&b.locale)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        AppError::bad_request(
            "translate this key first: the official translation is the control arm",
        )
    })?;

    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let mut ok: Vec<(String, String)> = vec![];
    let mut rejected = vec![];
    for c in generate(&state, &p, &k, &b.locale, &current, n, b.fit.as_ref()).await? {
        let text = c.text.trim().to_string();
        let mut issues = keys::check(&k.source, &text, &b.locale, &glossary);
        if let Some(i) = b
            .fit
            .as_ref()
            .and_then(|f| measure::check(&text, Syntax::Icu, f))
        {
            issues.push(i);
        }
        let dup = text.is_empty() || text == current || ok.iter().any(|o| o.0 == text);
        if dup || has_errors(&issues) {
            rejected.push(Rejected {
                text,
                tone: c.tone,
                issues,
            });
        } else if ok.len() < n {
            ok.push((text, c.tone));
        }
    }
    if ok.is_empty() {
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_valid_variant",
            format!("no variant passed validation ({} rejected)", rejected.len()),
        ));
    }

    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO variant_experiments (id, project_id, key, locale, source, goal, explore_pct, min_n)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(p.id)
    .bind(&k.key)
    .bind(&b.locale)
    .bind(&k.source)
    .bind(&goal)
    .bind(explore_pct)
    .bind(min_n)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.code == "conflict" => AppError::conflict("an experiment is already live for this key and locale"),
        e => e,
    })?;
    let arms = std::iter::once((current, "official".to_string())).chain(ok);
    for (i, (value, tone)) in arms.enumerate() {
        sqlx::query(
            "INSERT INTO variant_arms (experiment_id, arm, value, tone) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(i as i32)
        .bind(value)
        .bind(tone)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let experiment = load(&state, p.id, Some(id))
        .await?
        .pop()
        .ok_or_else(|| AppError::internal("experiment vanished"))?;
    Ok((
        StatusCode::CREATED,
        Json(Created {
            experiment,
            rejected,
        }),
    ))
}

/// Writes the winning variant as the official translation (same write path as an edit: validated, history,
/// then `glot pull` / CDN / PR like any change) and closes the experiment.
pub async fn promote(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<VariantExperiment>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut winner: Option<(String, String, String, Option<i32>, String)> = sqlx::query_as(
        "SELECT key, locale, source, winner, status FROM variant_experiments
         WHERE id = $1 AND project_id = $2 AND status IN ('running', 'decided')",
    )
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    if let Some(w) = winner.as_mut().filter(|w| w.4 == "running") {
        w.3 = evaluate(&state, id).await?;
    }
    let (key, locale, source, Some(arm), _) =
        winner.ok_or_else(|| AppError::not_found("live experiment not found"))?
    else {
        return Err(AppError::conflict(
            "no winner yet: P(best) must exceed 95 % with the minimum sample",
        ));
    };
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let mut tx = state.db.begin().await?;
    let k = keys::find_key(&mut tx, p.id, &key).await?;
    if k.source != source {
        return Err(AppError::conflict(
            "the source text changed since the variants were generated",
        ));
    }
    if arm != 0 {
        let value: String = sqlx::query_scalar(
            "SELECT value FROM variant_arms WHERE experiment_id = $1 AND arm = $2",
        )
        .bind(id)
        .bind(arm)
        .fetch_one(&mut *tx)
        .await?;
        keys::write_translation(
            &mut tx,
            &glossary,
            keys::Write {
                key: &k,
                locale: &locale,
                value: &value,
                origin: "human",
                status: "reviewed",
                model: Some("experiment"),
                score: None,
                extra_issues: vec![],
                author: Some(&user.name),
            },
        )
        .await?;
    }
    sqlx::query("UPDATE variant_experiments SET status = 'promoted', decided_at = coalesce(decided_at, now()) WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    load(&state, p.id, Some(id))
        .await?
        .pop()
        .map(Json)
        .ok_or_else(|| AppError::internal("experiment vanished"))
}

pub async fn stop(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    sqlx::query(
        "UPDATE variant_experiments SET status = 'stopped', decided_at = coalesce(decided_at, now())
         WHERE id = $1 AND project_id = $2 AND status IN ('running', 'decided')",
    )
    .bind(id)
    .bind(p.id)
    .execute(&state.db)
    .await?
    .rows_affected()
    .eq(&1)
    .then_some(StatusCode::NO_CONTENT)
    .ok_or_else(|| AppError::not_found("live experiment not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulated traffic: Thompson allocation, goal checked as in production (after each conversion).
    /// Returns the declared winner, if any, within `horizon` visitors.
    fn simulate(rates: &[f64], min_n: i64, horizon: usize, rng: &mut StdRng) -> Option<usize> {
        let (mut arms, mut looks) = (vec![(0i64, 0i64); rates.len()], 0);
        for _ in 0..horizon {
            let i = allocate(&arms, min_n, rng);
            arms[i].0 += 1;
            // Same rule as `evaluate`: on a conversion, look only when traffic reached the next checkpoint.
            if rng.random::<f64>() < rates[i] {
                arms[i].1 += 1;
                let total: i64 = arms.iter().map(|a| a.0).sum();
                if total >= next_look(arms.len(), min_n, looks) {
                    looks += 1;
                    if let Some(w) = decide(&arms, min_n, looks - 1, rng) {
                        return Some(w);
                    }
                }
            }
        }
        None
    }

    #[test]
    fn beta_draws_have_the_right_mean() {
        let mut rng = StdRng::seed_from_u64(1);
        let m: f64 = (0..20_000).map(|_| draw(200, 30, &mut rng)).sum::<f64>() / 20_000.0;
        assert!((m - 31.0 / 202.0).abs() < 0.003, "{m}");
    }

    #[test]
    fn posterior_ranks_arms() {
        let mut rng = StdRng::seed_from_u64(2);
        let p = posterior(&[(1000, 50), (1000, 80)], 20_000, &mut rng);
        assert!(
            p[1].0 > 0.99 && p[1].1 > 0.99 && (p[1].2 - 0.6).abs() < 0.1,
            "{p:?}"
        );
        assert!(decide(&[(1000, 50), (1000, 80)], 500, 0, &mut rng) == Some(1));
        assert_eq!(
            decide(&[(400, 20), (400, 60)], 500, 0, &mut rng),
            None,
            "under min n"
        );
        // P(best) ≈ 0.96: short of the third look's 99.7 % threshold.
        assert_eq!(decide(&[(2000, 100), (2000, 125)], 500, 2, &mut rng), None);
        assert_eq!(next_look(3, 500, 0), 1500);
        assert_eq!(next_look(3, 500, 2), 6000);
    }

    #[test]
    fn simulation_right_winner_and_controlled_false_positives() {
        let mut rng = StdRng::seed_from_u64(42);
        // Known rates: 5 % official vs 7.5 % and 5 % variants — the 7.5 % arm must win.
        let runs = 100;
        let got: Vec<Option<usize>> = (0..runs)
            .map(|_| simulate(&[0.05, 0.075, 0.05], 500, 50_000, &mut rng))
            .collect();
        let right = got.iter().filter(|w| **w == Some(1)).count();
        let wrong = got.iter().filter(|w| matches!(w, Some(0 | 2))).count();
        eprintln!(
            "7.5 % vs 5 %: right {right}, wrong {wrong}, undecided {}",
            runs - right - wrong
        );
        assert!(wrong <= runs / 20, "wrong winner {wrong}/{runs}");
        assert!(
            right as f64 / runs as f64 >= 0.9,
            "right winner {right}/{runs}"
        );
        // A/A: all arms equal — declaring any winner is a false positive.
        let fp = (0..runs)
            .filter(|_| simulate(&[0.05, 0.05], 500, 50_000, &mut rng).is_some())
            .count();
        eprintln!("A/A false positives: {fp}/{runs}");
        assert!(
            fp as f64 / runs as f64 <= 0.08,
            "false positives {fp}/{runs}"
        );
    }
}
