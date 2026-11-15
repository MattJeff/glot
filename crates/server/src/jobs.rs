use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use chrono::{DateTime, Utc};
use glot_core::{
    config::{Config, LlmConfig},
    context::KeyContext,
    glossary::Term,
    validate::{IssueCode, has_errors},
};
use glot_llm::translate::{Item, Request, Translator};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, types::Json as DbJson};
use std::{convert::Infallible, time::Duration};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    keys::{KeyRecord, Write, write_translation},
    projects::{self, ProjectRow, Role},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobParams {
    #[serde(default)]
    pub locales: Option<Vec<String>>,
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub force: bool,
    /// Cap for this job, on top of the project budget (the stricter wins), e.g. a recommendation's estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// Message syntax of this job's keys when it is not the project's: CMS content (connectors) is plain text
    /// with `{0}` placeholders, never ICU, whatever the project's catalogs use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax: Option<glot_core::validate::Syntax>,
}

#[derive(Debug, Clone, FromRow)]
pub struct JobRecord {
    pub(crate) id: Uuid,
    pub(crate) project_id: Uuid,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) params: DbJson<JobParams>,
    pub(crate) progress_done: i32,
    pub(crate) progress_total: i32,
    pub(crate) cost_usd: f64,
    pub(crate) error: Option<String>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    /// Only selected with [`QPOS`]; absent from `RETURNING {COLS}`.
    #[sqlx(default)]
    queue_position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Progress {
    pub done: i32,
    pub total: i32,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    /// queued | running | done | failed
    pub status: String,
    pub progress: Progress,
    pub cost_usd: f64,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// 1-based place among queued jobs, in claim order; only while `queued`.
    pub queue_position: Option<i64>,
}

impl From<&JobRecord> for Job {
    fn from(j: &JobRecord) -> Self {
        Job {
            id: j.id,
            kind: j.kind.clone(),
            status: j.status.clone(),
            progress: Progress {
                done: j.progress_done,
                total: j.progress_total,
            },
            cost_usd: j.cost_usd,
            error: j.error.clone(),
            created_at: j.created_at,
            finished_at: j.finished_at,
            queue_position: j.queue_position,
        }
    }
}

impl Job {
    fn finished(&self) -> bool {
        self.status == "done" || self.status == "failed"
    }
}

pub(crate) const COLS: &str = "id, project_id, kind, status, params, progress_done, progress_total, cost_usd, error, created_at, finished_at";

/// Claim order: short/urgent kinds (edge, connectors, GitHub) before long catalog `translate`, then FIFO.
/// `$t` is the table alias; [`QPOS`] counts with the same key, so a position matches what `claim` picks next.
macro_rules! claim_key {
    ($t:literal) => {
        concat!(
            "((",
            $t,
            ".kind = 'translate'), ",
            $t,
            ".created_at, ",
            $t,
            ".id)"
        )
    };
}

/// `queue_position` column for a query over `jobs j` (one correlated count on the partial `jobs_queued` index).
/// ponytail: ignores the per-project fairness skip in `claim`, so it is the order, not an ETA.
pub(crate) const QPOS: &str = concat!(
    "CASE WHEN j.status = 'queued' THEN (SELECT count(*) + 1 FROM jobs o WHERE o.status = 'queued' AND ",
    claim_key!("o"),
    " < ",
    claim_key!("j"),
    ") END AS queue_position"
);

// ---------- handlers ----------

#[derive(Deserialize)]
pub struct CreateBody {
    kind: String,
    #[serde(flatten)]
    params: JobParams,
}

pub async fn create(
    State(state): State<AppState>,
    crate::auth::VerifiedUser(user): crate::auth::VerifiedUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<Job>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if b.kind != "translate" {
        return Err(AppError::bad_request("kind must be 'translate'"));
    }
    crate::billing::check_llm(&state, &p).await?;
    if b.params
        .budget_usd
        .is_some_and(|x| !(x.is_finite() && x > 0.0))
    {
        return Err(AppError::bad_request("budget_usd must be > 0"));
    }
    if let Some(locs) = &b.params.locales
        && let Some(bad) = locs.iter().find(|l| !p.target_locales.contains(l))
    {
        return Err(AppError::bad_request(format!(
            "'{bad}' is not a target locale of this project"
        )));
    }
    // The CTE's row is not visible to QPOS's count, which is right: it counts the jobs ahead of this one.
    let j: JobRecord = sqlx::query_as(&format!(
        "WITH j AS (INSERT INTO jobs (id, project_id, kind, status, params, created_by) VALUES ($1, $2, $3, 'queued', $4, $5) RETURNING {COLS})
         SELECT {COLS}, {QPOS} FROM j"
    ))
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(&b.kind)
    .bind(DbJson(&b.params))
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    state.job_wakeup.notify_one();
    Ok((StatusCode::CREATED, Json((&j).into())))
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<Job>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<JobRecord> = sqlx::query_as(&format!(
        "SELECT {COLS}, {QPOS} FROM jobs j WHERE project_id = $1 ORDER BY created_at DESC LIMIT 100"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(Job::from).collect()))
}

async fn load_visible(state: &AppState, user: Uuid, id: Uuid) -> ApiResult<Job> {
    let j: JobRecord = sqlx::query_as(&format!(
        "SELECT {COLS}, {QPOS} FROM jobs j WHERE id = $1
         AND EXISTS (SELECT 1 FROM project_members m WHERE m.project_id = j.project_id AND m.user_id = $2)"
    ))
    .bind(id)
    .bind(user)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("job not found"))?;
    Ok((&j).into())
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(id): AppPath<Uuid>,
) -> ApiResult<Json<Job>> {
    Ok(Json(load_visible(&state, user.id, id).await?))
}

/// SSE: `progress` events (data = Job) until the job finishes, then a final `done` event (data = Job).
pub async fn events(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(id): AppPath<Uuid>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    // Subscribe before reading the current state so no update is lost in between.
    let mut rx = state.job_events.subscribe();
    let current = load_visible(&state, user.id, id).await?;
    let (tx, out) = tokio::sync::mpsc::channel::<Event>(16);
    tokio::spawn(async move {
        let send = |name: &str, job: &Job| {
            tx.send(
                Event::default()
                    .event(name)
                    .json_data(job)
                    .expect("job json"),
            )
        };
        if send("progress", &current).await.is_err() {
            return;
        }
        let mut last = current;
        while !last.finished() {
            match rx.recv().await {
                Ok(job) if job.id == id => {
                    if send("progress", &job).await.is_err() {
                        return;
                    }
                    last = job;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
        let _ = send("done", &last).await;
    });
    Ok(Sse::new(ReceiverStream::new(out).map(Ok)).keep_alive(KeepAlive::default()))
}

// ---------- worker ----------

/// Background workers (`GLOT_JOB_WORKERS`, default 4), each claiming one job at a time; safe with several
/// servers. With more than one worker, worker 0 is a fast lane that never takes a long job (catalog `translate`,
/// `github_*`), so edge and connector jobs always find a worker even while hour-long jobs fill the others.
/// The handle owns every worker: aborting it stops them all.
pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    let mut set = tokio::task::JoinSet::new();
    let n = std::env::var("GLOT_JOB_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4)
        .max(1);
    for i in 0..n {
        let state = state.clone();
        let fast_lane = n > 1 && i == 0;
        set.spawn(async move {
            loop {
                match claim(&state.db, fast_lane).await {
                    Ok(Some(job)) => {
                        // Another job may be claimable now (e.g. the next one of the queue): wake a sibling.
                        state.job_wakeup.notify_one();
                        run(state.clone(), job).await
                    }
                    Ok(None) => {
                        let _ = tokio::time::timeout(
                            Duration::from_secs(5),
                            state.job_wakeup.notified(),
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::error!("job claim failed: {e}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }
    tokio::spawn(async move { while set.join_next().await.is_some() {} })
}

/// Jobs left `running` by a crashed process go back to the queue.
pub async fn requeue_orphans(db: &sqlx::PgPool) -> sqlx::Result<()> {
    sqlx::query("UPDATE jobs SET status = 'queued', progress_done = 0 WHERE status = 'running'")
        .execute(db)
        .await?;
    // An Edge pre-warm runs in this process: a restart (deploy) kills it, and a row left `running` blocks the next
    // one for 2 hours and shows a frozen run (cliponit's stopped at a deploy and nothing restarted it).
    sqlx::query(
        "UPDATE edge_prewarm SET status = 'failed', error = 'interrupted by a server restart', finished_at = now()
         WHERE status = 'running'",
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Claims the next job: priority + FIFO ([`claim_key`]), skipping projects that already run a job of the same
/// family (`github_*` is one family), so one project never takes every worker. Long `translate` jobs still run
/// whenever nothing else is claimable, so they cannot starve. `fast_lane` refuses long kinds.
/// Claims are serialized by a transaction advisory lock: the "already running" check is then exact across
/// workers and servers (two claims cannot both see a project idle); a claim is one indexed UPDATE.
pub async fn claim(db: &sqlx::PgPool, fast_lane: bool) -> sqlx::Result<Option<JobRecord>> {
    const FAMILY: &str = "CASE WHEN kind LIKE 'github\\_%' THEN 'github' ELSE kind END";
    let lane = if fast_lane {
        "AND q.kind <> 'translate' AND q.kind NOT LIKE 'github\\_%'"
    } else {
        ""
    };
    let fq = FAMILY.replace("kind", "q.kind");
    let fr = FAMILY.replace("kind", "r.kind");
    let key = claim_key!("q");
    let sql = |fair: &str| {
        format!(
            "UPDATE jobs SET status = 'running' WHERE id = (
                SELECT q.id FROM jobs q WHERE q.status = 'queued' {lane} {fair}
                ORDER BY {key} FOR UPDATE SKIP LOCKED LIMIT 1
             ) RETURNING {COLS}"
        )
    };
    let fair = format!(
        "AND NOT EXISTS (SELECT 1 FROM jobs r WHERE r.status = 'running' AND r.project_id = q.project_id AND {fr} = {fq})"
    );
    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7426_1001)")
        .execute(&mut *tx)
        .await?;
    let mut job = sqlx::query_as(&sql(&fair)).fetch_optional(&mut *tx).await?;
    // Work-conserving for Edge: when every other project is served, an idle worker takes the busy project's
    // next `edge_translate` (one per locale, `tm::queue`, so they never touch the same rows). cliponit's 31
    // locales ran one at a time while three workers idled. GitHub jobs on one repo still wait for each other.
    if job.is_none() {
        job = sqlx::query_as(&sql("AND q.kind = 'edge_translate'"))
            .fetch_optional(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(job)
}

pub(crate) async fn save(state: &AppState, j: &JobRecord) {
    let res = sqlx::query(
        "UPDATE jobs SET status = $2, progress_done = $3, progress_total = $4, cost_usd = $5, error = $6, finished_at = $7 WHERE id = $1",
    )
    .bind(j.id)
    .bind(&j.status)
    .bind(j.progress_done)
    .bind(j.progress_total)
    .bind(j.cost_usd)
    .bind(&j.error)
    .bind(j.finished_at)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::error!(job = %j.id, "saving job failed: {e}");
    }
    let _ = state.job_events.send(j.into());
}

async fn run(state: AppState, mut job: JobRecord) {
    let state = &state;
    tracing::info!(job = %job.id, "job started");
    save(state, &job).await;
    // GitHub jobs: progress counts steps, and the LLM runs in the `glot` subprocess with the project's own key.
    let github = job.kind.starts_with("github_");
    let edge = job.kind == crate::edge::tm::JOB_KIND;
    let result = if github {
        // Every subprocess has its own timeout, but one wedged step must not hold a worker forever: a hard ceiling on the whole job, on top of the translation time
        // (which stops by itself and saves as it goes, see `github_app::translate_secs`).
        let ceiling = Duration::from_secs(
            std::env::var("GLOT_GITHUB_JOB_MAX_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2700)
                + crate::github_app::translate_secs(),
        );
        match tokio::time::timeout(ceiling, crate::github_app::run_job(state, &mut job)).await {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!(
                "job timed out after {}s",
                ceiling.as_secs()
            )),
        }
    } else if edge {
        crate::edge::tm::run_job(state, &mut job).await
    } else {
        translate(state, &mut job).await
    };
    if !github {
        crate::billing::after_job(state, job.project_id, job.id, job.progress_done as i64).await;
    }
    job.finished_at = Some(Utc::now());
    match result {
        Ok(()) => job.status = "done".into(),
        Err(e) => {
            tracing::warn!(job = %job.id, "job failed: {e:#}");
            job.status = "failed".into();
            job.error = Some(format!("{e:#}"));
        }
    }
    save(state, &job).await;
    tracing::info!(job = %job.id, status = %job.status, cost_usd = job.cost_usd, "job finished");
    // New translations may have made a language ready: check that the site really shows it.
    if !github && job.status == "done" {
        crate::live_check::after_job(state, job.project_id);
    }
    // Edge and CMS connector jobs run in the background: no email per job.
    if github || edge || job.kind == crate::connectors::sync::JOB_KIND {
        return;
    }
    crate::email::job_finished(
        state,
        job.id,
        job.status == "done",
        job.progress_done,
        job.progress_total,
        job.cost_usd,
        job.error.as_deref(),
    )
    .await;
}

/// What a translation job needs: the project, its config (budget = project's, capped by `budget`), the
/// translator on the project's provider/key and the glossary. Shared with `edge::tm::run_job`.
pub(crate) async fn setup(
    state: &AppState,
    project_id: Uuid,
    budget: Option<f64>,
) -> anyhow::Result<(
    ProjectRow,
    Config,
    Translator,
    glot_core::glossary::Glossary,
)> {
    let p: ProjectRow = sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at, syntax FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_one(&state.db)
    .await?;
    let llm = p.llm.0.clone();
    let api_key = match &p.api_key_enc {
        Some(enc) => Some(crate::auth::decrypt(&state.secret, enc).ok_or_else(|| {
            anyhow::anyhow!("cannot decrypt the project API key (GLOT_SECRET_KEY changed?)")
        })?),
        None => None,
    };
    let config = Config {
        source_locale: p.source_locale.clone(),
        target_locales: p.target_locales.clone(),
        files: vec![],
        format: Default::default(),
        syntax: p.syntax,
        llm: LlmConfig {
            provider: llm.provider,
            model: llm.model,
            fallback_model: llm.fallback_model,
            judge_model: llm.judge_model,
            api_key_env: None,
            base_url: llm.base_url,
            budget_usd: match (llm.budget_usd, budget) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
            style: p.style.clone(),
            ..LlmConfig::default()
        },
        qa: Default::default(),
        extract: Default::default(),
        server: None,
        i18next: Default::default(),
    };
    let provider = glot_llm::provider::from_config_with(&config.llm, api_key, &p.llm.0.headers())?;
    let translator = Translator::new(&config, provider);
    let glossary = crate::glossary::load(&state.db, p.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    Ok((p, config, translator, glossary))
}

async fn translate(state: &AppState, job: &mut JobRecord) -> anyhow::Result<()> {
    let params = job.params.0.clone();
    translate_with(state, job, &params, Limits::default())
        .await
        .map(|_| ())
}

/// How a translation run started by another job (GitHub) differs from the dashboard one.
#[derive(Default)]
pub(crate) struct Limits {
    /// Refuse before any call when the estimate exceeds the budget; says where that budget is set.
    pub(crate) budget_source: Option<String>,
    /// No batch starts after this instant (batches in flight are still written): the rest is
    /// left for the next run, which starts from what is saved.
    pub(crate) deadline: Option<std::time::Instant>,
    /// This run is step `.0` of `.1` of its job: progress = (step·n + done) / (steps·n).
    pub(crate) step: Option<(i32, i32)>,
}

#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) planned: usize,
    pub(crate) translated: usize,
    /// Strings not attempted (deadline).
    pub(crate) remaining: usize,
    /// Placeholder values (`⟦fr⟧ …`) a real provider answered with: refused, not written.
    pub(crate) placeholders: usize,
}

/// Translates what is missing or stale in the database (resumable: each batch is committed as soon
/// as it is back, so an interrupted run loses at most the batches in flight).
pub(crate) async fn translate_with(
    state: &AppState,
    job: &mut JobRecord,
    params: &JobParams,
    limits: Limits,
) -> anyhow::Result<Outcome> {
    let (p, config, translator, glossary) = setup(state, job.project_id, params.budget_usd).await?;
    let syntax = params.syntax.unwrap_or(p.syntax);
    let mock = config.llm.provider == "mock";

    let all_keys: Vec<KeyRecord> = sqlx::query_as(
        "SELECT id, key, source, description, refs, ui_role, component FROM keys WHERE project_id = $1 ORDER BY key",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let locales = params
        .locales
        .clone()
        .unwrap_or_else(|| p.target_locales.clone());
    let wanted: Option<std::collections::HashSet<&String>> =
        params.keys.as_ref().map(|ks| ks.iter().collect());

    // Plan: (locale, key, previous value) for keys with no translation, a stale one, or everything when forced.
    // A placeholder of the offline mock is missing for a real provider.
    let mut plan: Vec<(String, Vec<(&KeyRecord, Option<String>)>)> = vec![];
    for locale in &locales {
        let existing: std::collections::HashMap<Uuid, (String, String)> = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT t.key_id, t.value, t.status FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.project_id = $1 AND t.locale = $2",
        )
        .bind(p.id)
        .bind(locale)
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|(id, value, status)| (id, (value, status)))
        .collect();
        let todo: Vec<_> = all_keys
            .iter()
            .filter(|k| wanted.as_ref().is_none_or(|ks| ks.contains(&k.key)))
            .filter_map(|k| match existing.get(&k.id) {
                None => Some((k, None)),
                Some((value, _)) if !mock && crate::github_app::has_mock_marker(value) => {
                    Some((k, None))
                }
                Some((value, status)) if params.force || status == "stale" => {
                    Some((k, Some(value.clone())))
                }
                Some(_) => None,
            })
            .collect();
        plan.push((locale.clone(), todo));
    }
    let n: usize = plan.iter().map(|(_, items)| items.len()).sum();
    let mut out = Outcome {
        planned: n,
        ..Default::default()
    };
    let (step, steps) = limits.step.unwrap_or((0, 1));
    let progress = |done: usize| (step * n as i32 + done as i32, steps * n as i32);
    if n > 0 {
        (job.progress_done, job.progress_total) = progress(0);
        save(state, job).await;
    }

    // Requests: CONCURRENCY batches each (run together by the translator), code context computed once per key.
    let mut contexts: std::collections::HashMap<Uuid, KeyContext> = Default::default();
    let mut chunks: Vec<(&str, &[(&KeyRecord, Option<String>)], Request)> = vec![];
    let size = config.llm.batch_size.max(1) * glot_llm::translate::CONCURRENCY;
    for (locale, items) in &plan {
        for chunk in items.chunks(size) {
            let mut glossary_terms: Vec<Term> = vec![];
            for (k, _) in chunk {
                for t in glossary.matches(&k.source) {
                    if !glossary_terms.iter().any(|g| g.term == t.term) {
                        glossary_terms.push(t.clone());
                    }
                }
            }
            let req = Request {
                source_locale: p.source_locale.clone(),
                target_locale: locale.clone(),
                items: chunk
                    .iter()
                    .map(|(k, previous)| Item {
                        key: k.key.clone(),
                        source: k.source.clone(),
                        context: contexts
                            .entry(k.id)
                            .or_insert_with(|| context(k, &all_keys))
                            .clone(),
                        previous: previous.clone(),
                    })
                    .collect(),
                glossary: glossary_terms,
                syntax,
                style: p.style.clone(),
            };
            chunks.push((locale, chunk, req));
        }
    }
    let budget = config.llm.budget_usd;
    if let (Some(b), Some(source)) = (budget, &limits.budget_source) {
        let est: f64 = chunks
            .iter()
            .map(|(_, _, r)| glot_llm::translate::estimate(&config, r).cost_usd)
            .sum();
        if est > b {
            anyhow::bail!(
                "budget: estimated ${est:.2} to translate {n} strings exceeds the budget ${b:.2} {source}"
            );
        }
    }

    // A few requests at a time across locales; each one is written as soon as it is back.
    // ponytail: GLOT_TRANSLATE_PARALLEL × CONCURRENCY calls in flight, lower it on low provider tiers.
    let parallel = std::env::var("GLOT_TRANSLATE_PARALLEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4usize)
        .max(1);
    // A failed request (or the budget reached) starts nothing new, but what is in flight is still
    // written: money already spent is never thrown away.
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut failed: Option<anyhow::Error> = None;
    let (t, stop_ref) = (&translator, &stop);
    let deadline = limits.deadline;
    let mut calls = vec![];
    for (i, (_, _, req)) in chunks.iter().enumerate() {
        let req = req.clone();
        calls.push(async move {
            if stop_ref.load(std::sync::atomic::Ordering::Relaxed)
                || deadline.is_some_and(|d| std::time::Instant::now() >= d)
            {
                return (i, None);
            }
            (i, Some(t.translate(req).await))
        });
    }
    let mut results = futures::StreamExt::buffer_unordered(futures::stream::iter(calls), parallel);
    let mut done = 0usize;
    while let Some((i, res)) = results.next().await {
        let (locale, chunk) = (chunks[i].0, chunks[i].1);
        let Some(res) = res else {
            out.remaining += chunk.len();
            continue;
        };
        let (results, usage) = match res {
            Ok(r) => r,
            Err(e) => {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                failed.get_or_insert(e.into());
                continue;
            }
        };
        job.cost_usd += usage.cost_usd;

        let mut tx = state.db.begin().await?;
        for r in results {
            if has_errors(&r.issues) {
                continue;
            }
            if !mock && crate::github_app::has_mock_marker(&r.value) {
                out.placeholders += 1;
                continue;
            }
            let Some((k, _)) = chunk.iter().find(|(k, _)| k.key == r.key) else {
                continue;
            };
            let low_score = r
                .issues
                .into_iter()
                .filter(|i| i.code == IssueCode::LowScore)
                .collect();
            write_translation(
                &mut tx,
                &glossary,
                Write {
                    key: k,
                    locale,
                    value: &r.value,
                    origin: "machine",
                    status: "machine",
                    model: Some(&r.model),
                    score: r.score,
                    extra_issues: low_score,
                    author: None,
                    syntax,
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
            out.translated += 1;
        }
        tx.commit().await?;
        done += chunk.len();
        (job.progress_done, job.progress_total) = progress(done);
        save(state, job).await;
        if let Some(budget) = budget
            && job.cost_usd > budget
            && failed.is_none()
        {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            failed = Some(anyhow::anyhow!(
                "budget exceeded: spent ${:.4} of ${budget:.2}{}",
                job.cost_usd,
                limits
                    .budget_source
                    .as_ref()
                    .map(|s| format!(" {s}"))
                    .unwrap_or_default()
            ));
        }
    }
    match failed {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

fn context(k: &KeyRecord, all: &[KeyRecord]) -> KeyContext {
    let prefix = k.key.rsplit_once('.').map(|(p, _)| format!("{p}."));
    let neighbors = match &prefix {
        Some(prefix) => all
            .iter()
            .filter(|o| o.id != k.id && o.key.starts_with(prefix.as_str()))
            .take(10)
            .map(|o| (o.key.clone(), o.source.clone()))
            .collect(),
        None => vec![],
    };
    KeyContext {
        key: k.key.clone(),
        description: k.description.clone(),
        refs: k.refs.0.clone(),
        ui_role: k.ui_role.clone(),
        component: k.component.clone(),
        neighbors,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glot_core::context::SourceRef;

    #[test]
    fn context_carries_code_context() {
        let rec = |key: &str| KeyRecord {
            id: Uuid::new_v4(),
            key: key.into(),
            source: "Save".into(),
            description: Some("Saves the form".into()),
            refs: sqlx::types::Json(vec![SourceRef {
                path: "app/page.tsx".into(),
                line: 12,
            }]),
            ui_role: Some("button".into()),
            component: Some("SaveButton".into()),
        };
        let all = [rec("form.save"), rec("form.cancel")];
        let c = context(&all[0], &all);
        assert_eq!(c.ui_role.as_deref(), Some("button"));
        assert_eq!(c.component.as_deref(), Some("SaveButton"));
        assert_eq!(c.description.as_deref(), Some("Saves the form"));
        assert_eq!(c.refs[0].line, 12);
        assert_eq!(c.neighbors, vec![("form.cancel".into(), "Save".into())]);
    }
}
