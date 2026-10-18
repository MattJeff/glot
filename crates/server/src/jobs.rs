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
}

#[derive(Debug, Clone, FromRow)]
struct JobRecord {
    id: Uuid,
    project_id: Uuid,
    kind: String,
    status: String,
    params: DbJson<JobParams>,
    progress_done: i32,
    progress_total: i32,
    cost_usd: f64,
    error: Option<String>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
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
        }
    }
}

impl Job {
    fn finished(&self) -> bool {
        self.status == "done" || self.status == "failed"
    }
}

const COLS: &str = "id, project_id, kind, status, params, progress_done, progress_total, cost_usd, error, created_at, finished_at";

// ---------- handlers ----------

#[derive(Deserialize)]
pub struct CreateBody {
    kind: String,
    #[serde(flatten)]
    params: JobParams,
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<Job>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if b.kind != "translate" {
        return Err(AppError::bad_request("kind must be 'translate'"));
    }
    if let Some(locs) = &b.params.locales
        && let Some(bad) = locs.iter().find(|l| !p.target_locales.contains(l))
    {
        return Err(AppError::bad_request(format!(
            "'{bad}' is not a target locale of this project"
        )));
    }
    let j: JobRecord = sqlx::query_as(&format!(
        "INSERT INTO jobs (id, project_id, kind, status, params) VALUES ($1, $2, $3, 'queued', $4) RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(&b.kind)
    .bind(DbJson(&b.params))
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
        "SELECT {COLS} FROM jobs WHERE project_id = $1 ORDER BY created_at DESC LIMIT 100"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(Job::from).collect()))
}

async fn load_visible(state: &AppState, user: Uuid, id: Uuid) -> ApiResult<Job> {
    let j: JobRecord = sqlx::query_as(&format!(
        "SELECT {COLS} FROM jobs j WHERE id = $1
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

/// Background worker: claims queued jobs one at a time (`FOR UPDATE SKIP LOCKED`, safe with several servers).
pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match claim(state.db.clone()).await {
                Ok(Some(job)) => run(state.clone(), job).await,
                Ok(None) => {
                    let _ =
                        tokio::time::timeout(Duration::from_secs(5), state.job_wakeup.notified())
                            .await;
                }
                Err(e) => {
                    tracing::error!("job claim failed: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    })
}

/// Jobs left `running` by a crashed process go back to the queue.
pub async fn requeue_orphans(db: &sqlx::PgPool) -> sqlx::Result<()> {
    sqlx::query("UPDATE jobs SET status = 'queued', progress_done = 0 WHERE status = 'running'")
        .execute(db)
        .await?;
    Ok(())
}

async fn claim(db: sqlx::PgPool) -> sqlx::Result<Option<JobRecord>> {
    let sql = format!(
        "UPDATE jobs SET status = 'running' WHERE id = (
            SELECT id FROM jobs WHERE status = 'queued' ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1
         ) RETURNING {COLS}"
    );
    sqlx::query_as(&sql).fetch_optional(&db).await
}

async fn save(state: &AppState, j: &JobRecord) {
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
    let result = translate(state, &mut job).await;
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
}

async fn translate(state: &AppState, job: &mut JobRecord) -> anyhow::Result<()> {
    let p: ProjectRow = sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at FROM projects WHERE id = $1",
    )
    .bind(job.project_id)
    .fetch_one(&state.db)
    .await?;
    let params = job.params.0.clone();
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
        syntax: Default::default(),
        llm: LlmConfig {
            provider: llm.provider,
            model: llm.model,
            fallback_model: llm.fallback_model,
            judge_model: llm.judge_model,
            api_key_env: None,
            base_url: llm.base_url,
            budget_usd: llm.budget_usd,
            style: p.style.clone(),
            ..LlmConfig::default()
        },
        qa: Default::default(),
        extract: Default::default(),
        server: None,
    };
    let provider = glot_llm::provider::from_config_with(&config.llm, api_key, &p.llm.0.headers())?;
    let translator = Translator::new(&config, provider);
    let glossary = crate::glossary::load(&state.db, p.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;

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

    // Plan: (locale, key, previous value) for keys with no translation, a stale one, or everything when forced.
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
            .filter(|k| params.keys.as_ref().is_none_or(|ks| ks.contains(&k.key)))
            .filter_map(|k| match existing.get(&k.id) {
                None => Some((k, None)),
                Some((value, status)) if params.force || status == "stale" => {
                    Some((k, Some(value.clone())))
                }
                Some(_) => None,
            })
            .collect();
        plan.push((locale.clone(), todo));
    }
    job.progress_total = plan.iter().map(|(_, items)| items.len() as i32).sum();
    save(state, job).await;

    for (locale, items) in plan {
        for chunk in items.chunks(config.llm.batch_size.max(1)) {
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
                        context: context(k, &all_keys),
                        previous: previous.clone(),
                    })
                    .collect(),
                glossary: glossary_terms,
                syntax: config.syntax,
                style: p.style.clone(),
            };
            let (results, usage) = translator.translate(req).await?;
            job.cost_usd += usage.cost_usd;

            let mut tx = state.db.begin().await?;
            for r in results {
                if has_errors(&r.issues) {
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
                        locale: &locale,
                        value: &r.value,
                        origin: "machine",
                        status: "machine",
                        model: Some(&r.model),
                        score: r.score,
                        extra_issues: low_score,
                        author: None,
                    },
                )
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            }
            tx.commit().await?;
            job.progress_done += chunk.len() as i32;
            save(state, job).await;
            if let Some(budget) = config.llm.budget_usd
                && job.cost_usd > budget
            {
                anyhow::bail!(
                    "budget exceeded: spent ${:.4} of ${budget:.2}",
                    job.cost_usd
                );
            }
        }
    }
    Ok(())
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
