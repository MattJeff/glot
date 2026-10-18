//! AI variants by intention for a draft experiment (contract §4 `POST P/{id}/generate`), the project copy policy
//! (`GET|PUT P/policy`) and segment personalization without a test (`POST P/{id}/personalize`).
//! Generation = `glot_llm::copywriting` (source-language copy, then the guarded translation pipeline per locale)
//! + `glot_core::policy` on every (key, locale). Only fully valid variants become arms; the rest is `rejected`.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use glot_core::{
    context::KeyContext,
    measure::Fit,
    policy::{self, Finding, PolicyConfig},
    validate::Severity,
};
use glot_llm::copywriting::{self, Brief, CopyKey, Intent};
use serde::{Deserialize, Serialize};
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{ANY, Experiment, MAX_ARMS, Predicate, Status, Variant, load, readiness};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    feedback,
    projects::{self, ProjectRow, Role},
};

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/xp";
    Router::new()
        .route(&format!("{p}/policy"), get(get_policy).put(put_policy))
        .route(&format!("{p}/{{id}}/generate"), post(generate))
        .route(&format!("{p}/{{id}}/personalize"), post(personalize))
}

// ---------- policy (project settings) ----------

pub async fn project_policy(state: &AppState, project: Uuid) -> ApiResult<PolicyConfig> {
    let v: DbJson<PolicyConfig> =
        sqlx::query_scalar("SELECT copy_policy FROM projects WHERE id = $1")
            .bind(project)
            .fetch_one(&state.db)
            .await?;
    Ok(v.0)
}

const RULES: [&str; 8] = [
    "fake_urgency",
    "fake_counter",
    "unprovable_claim",
    "superlative",
    "unsourced_number",
    "price_or_legal",
    "confirmshaming",
    "forbidden_term",
];

pub async fn get_policy(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<PolicyConfig>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(project_policy(&state, p.id).await?))
}

/// Same shape as `[policy]` in glot.toml. Admin only: turning a rule `off` is the project's legal call.
pub async fn put_policy(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PolicyConfig>,
) -> ApiResult<Json<PolicyConfig>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let unknown = b
        .rules
        .keys()
        .chain(b.patterns.keys())
        .find(|r| !RULES.contains(&r.as_str()));
    if let Some(r) = unknown {
        return Err(AppError::bad_request(format!(
            "unknown rule '{r}' (one of {})",
            RULES.join(", ")
        )));
    }
    let words = b.forbidden.values().chain(b.patterns.values());
    if b.forbidden.len() > 100
        || words
            .clone()
            .any(|w| w.len() > 200 || w.iter().any(|t| !(1..=100).contains(&t.chars().count())))
    {
        return Err(AppError::bad_request(
            "at most 100 markets × 200 terms of 1-100 characters",
        ));
    }
    sqlx::query("UPDATE projects SET copy_policy = $2 WHERE id = $1")
        .bind(p.id)
        .bind(DbJson(&b))
        .execute(&state.db)
        .await?;
    Ok(Json(b))
}

// ---------- generation ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateBody {
    /// social_proof | direct_benefit | honest_urgency | simplicity | curiosity | authority | reassurance | segment
    /// ("social proof" accepted).
    pub intents: Vec<String>,
    /// Subset of the experiment locales (default: all of them).
    pub locales: Option<Vec<String>>,
    /// Where the group is shown ("pricing page hero"); default: the experiment name.
    pub page: Option<String>,
    /// Who the copy speaks to ("teams in Germany"); default: the experiment's segments.
    pub segment: Option<String>,
}

#[derive(Serialize)]
pub struct Note {
    pub intent: String,
    pub key: String,
    pub locale: String,
    pub value: String,
    pub rule: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Serialize)]
pub struct Rejected {
    pub intent: String,
    pub rationale: String,
    /// Every produced (key, locale) with its validator issues and policy findings; failing ones explain why.
    pub cells: Vec<copywriting::Cell>,
}

#[derive(Serialize)]
pub struct GenerateResult {
    /// New or regenerated arms (draft; `approved = false` when the experiment requires review).
    pub arms: Vec<Variant>,
    pub rejected: Vec<Rejected>,
    /// Policy warnings on kept arms (superlatives…): servable, worth a human look.
    pub warnings: Vec<Note>,
    pub cost_usd: f64,
}

fn rule_text(p: &Predicate) -> String {
    let list = |v: &Vec<String>| v.join(", ");
    match (&p.eq, &p.r#in, &p.not_in) {
        (Some(e), ..) => format!("{} = {e}", p.attr),
        (_, Some(l), _) => format!("{} in {}", p.attr, list(l)),
        (.., Some(l)) => format!("{} not in {}", p.attr, list(l)),
        _ => p.attr.clone(),
    }
}

pub async fn generate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<GenerateBody>,
) -> ApiResult<Json<GenerateResult>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let x = load(&state, p.id, id).await?;
    if x.kind != "copy" || x.status != Status::Draft {
        return Err(AppError::conflict(
            "variants are generated on a draft experiment",
        ));
    }
    let mut intents = vec![];
    for s in &b.intents {
        let i = Intent::parse(s).ok_or_else(|| {
            AppError::bad_request(format!(
                "unknown intent '{s}' (one of {})",
                Intent::ALL.map(Intent::id).join(", ")
            ))
        })?;
        if !intents.contains(&i) {
            intents.push(i);
        }
    }
    // Arm number per intent: regenerating an intent replaces its arm, a new one takes the first free slot.
    let label = |i: Intent| i.id().replace('_', " ");
    let mut slots: BTreeMap<Intent, i32> = BTreeMap::new();
    let mut free = (1..MAX_ARMS).filter(|n| !x.arms.iter().any(|a| a.arm == *n));
    for i in &intents {
        let arm = match x
            .arms
            .iter()
            .find(|a| a.arm > 0 && Intent::parse(&a.intent) == Some(*i))
        {
            Some(a) => a.arm,
            None => free.next().ok_or_else(|| {
                AppError::bad_request(format!("at most {} variants per experiment", MAX_ARMS - 1))
            })?,
        };
        slots.insert(*i, arm);
    }
    if intents.is_empty() || x.keys.is_empty() {
        return Err(AppError::bad_request(
            "give at least one intent, on an experiment with keys",
        ));
    }
    let locales: Vec<String> = match &b.locales {
        Some(l) => x
            .locales
            .iter()
            .filter(|x| l.contains(x))
            .cloned()
            .collect(),
        None => x.locales.clone(),
    };
    if locales.is_empty() {
        return Err(AppError::bad_request(
            "no locale of this experiment selected",
        ));
    }

    let keys: Vec<String> = x.keys.iter().map(|k| k.key.clone()).collect();
    let recs: Vec<(String, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT key, source, description, ui_role, component FROM keys WHERE project_id = $1 AND key = ANY($2)",
    )
    .bind(p.id)
    .bind(&keys)
    .fetch_all(&state.db)
    .await?;
    let official: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT k.key, t.locale, t.value FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND k.key = ANY($2) AND t.value <> ''",
    )
    .bind(p.id)
    .bind(&keys)
    .fetch_all(&state.db)
    .await?;
    let neighbors: Vec<(String, String)> =
        recs.iter().map(|r| (r.0.clone(), r.1.clone())).collect();
    let copy_keys: Vec<CopyKey> = recs
        .iter()
        .map(|(key, source, description, ui_role, component)| {
            let mut control: BTreeMap<String, String> = official
                .iter()
                .filter(|o| &o.0 == key)
                .map(|o| (o.1.clone(), o.2.clone()))
                .collect();
            control.insert(p.source_locale.clone(), source.clone());
            CopyKey {
                key: key.clone(),
                source: source.clone(),
                control,
                context: KeyContext {
                    key: key.clone(),
                    description: description.clone(),
                    ui_role: ui_role.clone(),
                    component: component.clone(),
                    neighbors: neighbors.iter().filter(|n| &n.0 != key).cloned().collect(),
                    fit: x
                        .fits
                        .get(key)
                        .and_then(|f| serde_json::from_value::<Fit>(f.clone()).ok()),
                    ..Default::default()
                },
            }
        })
        .collect();
    if copy_keys.len() != keys.len() {
        return Err(AppError::bad_request(
            "a key of this experiment no longer exists",
        ));
    }
    let segment = b.segment.clone().or_else(|| {
        (!x.cells.is_empty()).then(|| {
            x.cells
                .iter()
                .map(|c| {
                    format!(
                        "{} ({})",
                        c.name,
                        c.rule
                            .all
                            .iter()
                            .map(rule_text)
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" / ")
        })
    });
    let brief = Brief {
        source_locale: p.source_locale.clone(),
        locales: locales.clone(),
        keys: copy_keys,
        intents: intents.clone(),
        page: b.page.clone().or_else(|| Some(x.name.clone())),
        style: p.style.clone(),
        segment,
        facts: x.facts.clone(),
        syntax: Default::default(),
        glossary: crate::glossary::load(&state.db, p.id).await?.terms,
        policy: project_policy(&state, p.id).await?,
    };

    let llm_err = |e: glot_llm::Error| {
        AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", e.to_string())
    };
    let translator = glot_llm::translate::Translator::new(
        &feedback::config(&p),
        feedback::provider(&state, &p)?,
    );
    let provider = feedback::provider(&state, &p)?;
    let (out, usage) =
        copywriting::generate(provider.as_ref(), &p.llm.0.model, &translator, &brief)
            .await
            .map_err(llm_err)?;

    let (mut kept, mut rejected, mut warnings) = (vec![], vec![], vec![]);
    let mut tx = state.db.begin().await?;
    let s: String =
        sqlx::query_scalar("SELECT status FROM xp_experiments WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if s != Status::Draft.as_str() {
        return Err(AppError::conflict(
            "the experiment left draft during generation",
        ));
    }
    for g in out {
        if !g.ok {
            rejected.push(Rejected {
                intent: label(g.intent),
                rationale: g.rationale,
                cells: g.cells,
            });
            continue;
        }
        let arm = slots[&g.intent];
        sqlx::query(
            "INSERT INTO xp_arms (experiment_id, arm, intent, approved) VALUES ($1, $2, $3, $4)
             ON CONFLICT (experiment_id, arm) DO UPDATE SET intent = $3, approved = $4",
        )
        .bind(id)
        .bind(arm)
        .bind(label(g.intent))
        .bind(!x.review)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM xp_values WHERE experiment_id = $1 AND arm = $2 AND locale = ANY($3)",
        )
        .bind(id)
        .bind(arm)
        .bind(&locales)
        .execute(&mut *tx)
        .await?;
        for c in g.cells.iter().filter(|c| locales.contains(&c.locale)) {
            sqlx::query("INSERT INTO xp_values (experiment_id, arm, key, locale, value) VALUES ($1, $2, $3, $4, $5)")
                .bind(id)
                .bind(arm)
                .bind(&c.key)
                .bind(&c.locale)
                .bind(&c.value)
                .execute(&mut *tx)
                .await?;
            warnings.extend(c.findings.iter().map(|f| note(g.intent, c, f)));
        }
        kept.push(arm);
    }
    tx.commit().await?;
    let arms = load(&state, p.id, id)
        .await?
        .arms
        .into_iter()
        .filter(|a| kept.contains(&a.arm))
        .collect();
    Ok(Json(GenerateResult {
        arms,
        rejected,
        warnings,
        cost_usd: usage.cost_usd,
    }))
}

fn note(i: Intent, c: &copywriting::Cell, f: &Finding) -> Note {
    Note {
        intent: i.id().replace('_', " "),
        key: c.key.clone(),
        locale: c.locale.clone(),
        value: c.value.clone(),
        rule: f.rule.clone(),
        severity: f.severity,
        message: f.message.clone(),
    }
}

/// Policy errors of an experiment's arms as readiness problems ("arm 2 (social proof) — home.hero.title [de]: …").
/// Hand-edited texts bypass generation: `readiness` should add these so `start`/`personalize` refuse them.
pub async fn policy_problems(
    state: &AppState,
    p: &ProjectRow,
    x: &Experiment,
) -> ApiResult<Vec<String>> {
    let cfg = project_policy(state, p.id).await?;
    let official: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT k.key, t.locale, t.value FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND k.key = ANY($2)",
    )
    .bind(p.id)
    .bind(x.keys.iter().map(|k| k.key.clone()).collect::<Vec<_>>())
    .fetch_all(&state.db)
    .await?;
    let mut out = vec![];
    for a in x.arms.iter().filter(|a| a.arm > 0 && a.approved) {
        for (locale, m) in &a.values {
            for (key, value) in m {
                let control = official
                    .iter()
                    .find(|o| &o.0 == key && &o.1 == locale)
                    .map(|o| o.2.as_str())
                    .or_else(|| {
                        x.keys
                            .iter()
                            .find(|k| &k.key == key)
                            .map(|k| k.source.as_str())
                    })
                    .unwrap_or("");
                for f in policy::check(value, locale, control, &x.facts, &cfg) {
                    if f.severity == Severity::Error {
                        out.push(format!(
                            "arm {} ({}) — {key} [{locale}]: {}",
                            a.arm, a.intent, f.message
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

// ---------- personalization without a test ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalizeBody {
    pub arm: i32,
    /// Segment id (one of the experiment's cells).
    pub cell: String,
}

/// draft → personalized: the segment gets `arm` at 100 %, every other visitor the official text; nothing is
/// counted (no test). Same checks as `start` (readiness, policy, live keys, plan quota). Ended with `stop`.
pub async fn personalize(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<PersonalizeBody>,
) -> ApiResult<Json<Experiment>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let x = load(&state, p.id, id).await?;
    if !x.cells.iter().any(|c| c.id.to_string() == b.cell) {
        return Err(AppError::bad_request(
            "cell must be one of the experiment's segments",
        ));
    }
    if !x
        .arms
        .iter()
        .any(|a| a.arm == b.arm && a.arm > 0 && a.approved)
    {
        return Err(AppError::bad_request("unknown or unapproved arm"));
    }
    let r = readiness(&state, &p, id).await?;
    let mut problems = r.problems;
    problems.extend(policy_problems(&state, &p, &x).await?);
    if !problems.is_empty() {
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "not_ready",
            format!("not ready: {}", problems.join("; ")),
        ));
    }
    let mut tx = state.db.begin().await?;
    let s: String =
        sqlx::query_scalar("SELECT status FROM xp_experiments WHERE id = $1 AND project_id = $2 AND kind = 'copy' FOR UPDATE")
            .bind(id)
            .bind(p.id)
            .fetch_one(&mut *tx)
            .await?;
    if s != Status::Draft.as_str() {
        return Err(AppError::conflict(format!(
            "cannot personalize a {s} experiment"
        )));
    }
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM xp_experiments WHERE project_id = $1 AND kind = 'copy'
         AND status IN ('running', 'paused', 'personalized') AND id <> $2",
    )
    .bind(p.id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    crate::billing::check_experiment(&state, p.id, live + 1, x.cells.len() as i64).await?;
    // ponytail: same key freeze as api::start; share it if a third caller appears.
    sqlx::query(
        "UPDATE xp_keys x SET live = true, source = k.source
         FROM keys k WHERE k.project_id = x.project_id AND k.key = x.key AND x.experiment_id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.code == "conflict" => {
            AppError::conflict("a key of this experiment is already live elsewhere")
        }
        e => e,
    })?;
    let n = x.arms.iter().map(|a| a.arm).max().unwrap_or(0) as usize + 1;
    let one_hot = |arm: i32| {
        (0..n)
            .map(|i| if i as i32 == arm { 1.0 } else { 0.0 })
            .collect::<Vec<f64>>()
    };
    for cell in x
        .cells
        .iter()
        .map(|c| c.id.to_string())
        .chain([ANY.to_string()])
    {
        let target = cell == b.cell;
        sqlx::query(
            "INSERT INTO xp_cells (experiment_id, cell, weights, winner, personalized, decided_at)
             VALUES ($1, $2, $3, $4, true, now())
             ON CONFLICT (experiment_id, cell) DO UPDATE SET weights = $3, winner = $4, personalized = true,
               decided_at = now()",
        )
        .bind(id)
        .bind(&cell)
        .bind(one_hot(if target { b.arm } else { 0 }))
        .bind(target.then_some(b.arm))
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE xp_experiments SET status = 'personalized', started_at = coalesce(started_at, now()) WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(load(&state, p.id, id).await?))
}
