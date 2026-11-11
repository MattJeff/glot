//! Generic sync: pull (paginated, resumable) → `connector_items` + glot keys → `connector_translate` job →
//! push of validated translations (idempotent per locale). One background worker per process.
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, types::Json as DbJson};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use uuid::Uuid;

use super::{Creds, Format, PushItem, SourceItem, registry, rich};
use crate::{AppState, projects::ProjectRow};

/// Kind of the translation jobs a sync queues (runs `jobs::translate`, no email per job).
pub const JOB_KIND: &str = "connector_translate";
/// Translations are pushed again at most this often when nothing else triggers it (human reviews…).
const PUSH_EVERY: &str = "10 minutes";

#[derive(Debug, Clone, FromRow)]
pub struct AccountRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub name: String,
    pub creds_enc: Vec<u8>,
    pub settings: DbJson<Value>,
    pub webhook_secret: String,
    pub status: String,
    pub sync_cursor: Option<String>,
    pub sync_since: Option<DateTime<Utc>>,
    pub sync_started_at: Option<DateTime<Utc>>,
    pub dirty: bool,
    pub pending_job: Option<Uuid>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_push_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub stats: DbJson<Value>,
    pub created_at: DateTime<Utc>,
}

pub const COLS: &str = "id, project_id, kind, name, creds_enc, settings, webhook_secret, status, sync_cursor, sync_since, sync_started_at, dirty, pending_job, last_sync_at, last_push_at, last_error, stats, created_at";

pub fn creds(state: &AppState, a: &AccountRow) -> anyhow::Result<Creds> {
    let plain = crate::auth::decrypt(&state.secret, &a.creds_enc).ok_or_else(|| {
        anyhow::anyhow!("cannot decrypt the connector credentials (GLOT_SECRET_KEY changed?)")
    })?;
    Ok(Creds {
        secrets: serde_json::from_str(&plain)?,
        settings: a.settings.0.clone(),
        since: a.sync_since,
    })
}

async fn project(db: &sqlx::PgPool, id: Uuid) -> sqlx::Result<ProjectRow> {
    sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at, syntax FROM projects WHERE id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await
}

/// Queues a sync (`since` = incremental). Already queued: kept; running: re-run afterwards.
pub async fn queue(db: &sqlx::PgPool, id: Uuid, since: Option<DateTime<Utc>>) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE connector_accounts SET
           dirty = (status = 'running'),
           sync_since = CASE WHEN status = 'queued' THEN (CASE WHEN sync_since IS NULL OR $2::timestamptz IS NULL THEN NULL ELSE least(sync_since, $2) END)
                             WHEN status = 'running' THEN sync_since ELSE $2 END,
           status = CASE WHEN status = 'running' THEN status ELSE 'queued' END
         WHERE id = $1",
    )
    .bind(id)
    .bind(since)
    .execute(db)
    .await?;
    WAKE.notify_one();
    Ok(())
}

static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub fn spawn_worker(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Syncs interrupted by a restart resume from their saved cursor.
        let _ =
            sqlx::query("UPDATE connector_accounts SET status = 'queued' WHERE status = 'running'")
                .execute(&state.db)
                .await;
        loop {
            if let Err(e) = tick(&state).await {
                tracing::error!("connector worker: {e:#}");
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), WAKE.notified()).await;
        }
    })
}

/// One worker pass: every queued sync, pushes after finished jobs, periodic pushes. Public for tests.
pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    while let Some(a) = sqlx::query_as::<_, AccountRow>(&format!(
        "UPDATE connector_accounts SET status = 'running', sync_started_at = CASE WHEN sync_cursor IS NULL THEN now() ELSE coalesce(sync_started_at, now()) END
         WHERE id = (SELECT id FROM connector_accounts WHERE status = 'queued' ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1)
         RETURNING {COLS}"
    ))
    .fetch_optional(&state.db)
    .await?
    {
        let res = run_sync(state, &a).await;
        if let Err(e) = &res {
            tracing::warn!(account = %a.id, kind = %a.kind, "connector sync failed: {e:#}");
        }
        // Cursor kept on failure: the next sync resumes there.
        sqlx::query(
            "UPDATE connector_accounts SET status = CASE WHEN dirty THEN 'queued' WHEN $2::text IS NULL THEN 'ok' ELSE 'error' END,
               sync_since = CASE WHEN dirty THEN last_sync_at ELSE sync_since END,
               dirty = false, last_error = $2 WHERE id = $1",
        )
        .bind(a.id)
        .bind(res.err().map(|e| format!("{e:#}")))
        .execute(&state.db)
        .await?;
    }
    let due: Vec<AccountRow> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM connector_accounts a WHERE status = 'ok' AND (
           (pending_job IS NOT NULL AND EXISTS (SELECT 1 FROM jobs j WHERE j.id = a.pending_job AND j.status IN ('done', 'failed')))
           OR (pending_job IS NULL AND (last_push_at IS NULL OR last_push_at < now() - interval '{PUSH_EVERY}')))
         ORDER BY last_push_at NULLS FIRST LIMIT 10"
    ))
    .fetch_all(&state.db)
    .await?;
    for a in due {
        let res = push_ready(state, &a).await;
        sqlx::query(
            "UPDATE connector_accounts SET pending_job = NULL, last_push_at = now(), last_error = $2 WHERE id = $1",
        )
        .bind(a.id)
        .bind(res.err().map(|e| format!("push: {e:#}")))
        .execute(&state.db)
        .await?;
    }
    Ok(())
}

/// Stored source string and hash of a pulled item.
fn stored(it: &SourceItem) -> (String, String) {
    let source = match &it.format {
        Format::RichText(doc) => doc.to_string(),
        _ => it.text.clone(),
    };
    let hash = crate::auth::sha256_hex(&format!("{}\0{source}", it.format.name()));
    (source, hash)
}

fn format_of(name: &str, source: &str) -> Format {
    match name {
        "html" => Format::Html,
        "rich" => Format::RichText(serde_json::from_str(source).unwrap_or(Value::Null)),
        _ => Format::Plain,
    }
}

/// Pull every page into `connector_items` and glot keys; queue the translation; push what is ready.
pub async fn run_sync(state: &AppState, a: &AccountRow) -> anyhow::Result<()> {
    let c = registry(&a.kind).ok_or_else(|| anyhow::anyhow!("unknown connector '{}'", a.kind))?;
    let creds = creds(state, a)?;
    let p = project(&state.db, a.project_id).await?;
    let mut cursor = a.sync_cursor.clone();
    let mut pulled = 0usize;
    loop {
        let page = c.pull(&creds, cursor.clone()).await?;
        pulled += page.items.len();
        store_page(state, a, &p, &page.items).await?;
        cursor = page.next;
        sqlx::query("UPDATE connector_accounts SET sync_cursor = $2 WHERE id = $1")
            .bind(a.id)
            .bind(&cursor)
            .execute(&state.db)
            .await?;
        if cursor.is_none() {
            break;
        }
    }
    let started = a.sync_started_at.unwrap_or_else(Utc::now);
    if creds.since.is_none() {
        // Full pull: items gone from the CMS stop being pushed (their glot keys stay).
        sqlx::query("DELETE FROM connector_items WHERE account_id = $1 AND seen_at < $2")
            .bind(a.id)
            .bind(started)
            .execute(&state.db)
            .await?;
    }
    let keys = unit_keys(&state.db, a.id, &p.source_locale).await?;
    let todo: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM keys k CROSS JOIN unnest($3::text[]) AS l(locale)
         LEFT JOIN translations t ON t.key_id = k.id AND t.locale = l.locale
         WHERE k.project_id = $1 AND k.key = ANY($2) AND (t.key_id IS NULL OR t.status = 'stale')",
    )
    .bind(p.id)
    .bind(&keys)
    .bind(&p.target_locales)
    .fetch_one(&state.db)
    .await?;
    let mut job = None;
    if todo > 0 {
        crate::billing::check_llm(state, &p)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO jobs (id, project_id, kind, status, params) VALUES ($1, $2, $3, 'queued', $4)",
        )
        .bind(id)
        .bind(p.id)
        .bind(JOB_KIND)
        .bind(DbJson(crate::jobs::JobParams {
            keys: Some(keys.clone()),
            ..Default::default()
        }))
        .execute(&state.db)
        .await?;
        state.job_wakeup.notify_one();
        job = Some(id);
    }
    let items: i64 =
        sqlx::query_scalar("SELECT count(*) FROM connector_items WHERE account_id = $1")
            .bind(a.id)
            .fetch_one(&state.db)
            .await?;
    sqlx::query(
        "UPDATE connector_accounts SET sync_cursor = NULL, sync_since = NULL, last_sync_at = $2, pending_job = $3,
           stats = stats || $4 WHERE id = $1",
    )
    .bind(a.id)
    .bind(started)
    .bind(job)
    .bind(json!({"items": items, "units": keys.len(), "pulled": pulled, "to_translate": todo}))
    .execute(&state.db)
    .await?;
    if job.is_none() {
        push_ready(state, a).await?;
        sqlx::query("UPDATE connector_accounts SET last_push_at = now() WHERE id = $1")
            .bind(a.id)
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

async fn store_page(
    state: &AppState,
    a: &AccountRow,
    p: &ProjectRow,
    items: &[SourceItem],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = items.iter().map(|i| i.external_id.clone()).collect();
    let fields: Vec<String> = items.iter().map(|i| i.field.clone()).collect();
    let old: HashMap<(String, String), String> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT i.external_id, i.field, i.source_hash FROM connector_items i
         JOIN unnest($2::text[], $3::text[]) AS x(e, f) ON i.external_id = x.e AND i.field = x.f
         WHERE i.account_id = $1",
    )
    .bind(a.id)
    .bind(&ids)
    .bind(&fields)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(e, f, h)| ((e, f), h))
    .collect();
    let mut source_cat = BTreeMap::new();
    let mut notes = BTreeMap::new();
    for it in items {
        crate::validate_key(&it.key).map_err(|e| anyhow::anyhow!(e.message))?;
        let (source, hash) = stored(it);
        sqlx::query(
            "INSERT INTO connector_items (account_id, external_id, field, key, format, source, source_hash, meta, seen_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
             ON CONFLICT (account_id, external_id, field) DO UPDATE SET key = excluded.key, format = excluded.format,
               source = excluded.source, source_hash = excluded.source_hash, meta = excluded.meta, seen_at = now()",
        )
        .bind(a.id)
        .bind(&it.external_id)
        .bind(&it.field)
        .bind(&it.key)
        .bind(it.format.name())
        .bind(&source)
        .bind(&hash)
        .bind(&it.meta)
        .execute(&state.db)
        .await?;
        if old.get(&(it.external_id.clone(), it.field.clone())) == Some(&hash) {
            continue;
        }
        for (k, text) in rich::units(&it.key, &it.format, &source, &p.source_locale) {
            if !it.context.is_empty() {
                notes.insert(k.clone(), it.context.clone());
            }
            source_cat.insert(k, text);
        }
    }
    if source_cat.is_empty() {
        return Ok(());
    }
    let mut body = crate::sync::PushBody::default();
    body.catalogs.insert(p.source_locale.clone(), source_cat);
    body.lock.notes = notes;
    crate::sync::apply_push(state, p, body, &format!("connector:{}", a.kind))
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    Ok(())
}

#[derive(FromRow)]
struct ItemRow {
    external_id: String,
    field: String,
    key: String,
    format: String,
    source: String,
    meta: Value,
    pushed: DbJson<BTreeMap<String, String>>,
}

async fn items(db: &sqlx::PgPool, account: Uuid) -> sqlx::Result<Vec<ItemRow>> {
    sqlx::query_as(
        "SELECT external_id, field, key, format, source, meta, pushed FROM connector_items WHERE account_id = $1 ORDER BY external_id, field",
    )
    .bind(account)
    .fetch_all(db)
    .await
}

async fn unit_keys(
    db: &sqlx::PgPool,
    account: Uuid,
    source_locale: &str,
) -> sqlx::Result<Vec<String>> {
    Ok(items(db, account)
        .await?
        .iter()
        .flat_map(|i| {
            rich::units(
                &i.key,
                &format_of(&i.format, &i.source),
                &i.source,
                source_locale,
            )
        })
        .map(|u| u.0)
        .collect())
}

#[derive(Debug, Default, serde::Serialize)]
pub struct PushStats {
    pub pushed: usize,
    pub unchanged: usize,
    /// Items with at least one unit not translated (or stale) yet: not pushed.
    pub waiting: usize,
    pub failed: Vec<(String, String)>,
}

/// Pushes, per target locale, every item whose units are all translated (`machine` / `reviewed`) and whose
/// rebuilt value differs from the last one pushed.
pub async fn push_ready(state: &AppState, a: &AccountRow) -> anyhow::Result<PushStats> {
    let c = registry(&a.kind).ok_or_else(|| anyhow::anyhow!("unknown connector '{}'", a.kind))?;
    let creds = creds(state, a)?;
    let p = project(&state.db, a.project_id).await?;
    let rows = items(&state.db, a.id).await?;
    let formats: Vec<Format> = rows
        .iter()
        .map(|r| format_of(&r.format, &r.source))
        .collect();
    let keys: Vec<String> = rows
        .iter()
        .zip(&formats)
        .flat_map(|(r, f)| rich::units(&r.key, f, &r.source, &p.source_locale))
        .map(|u| u.0)
        .collect();
    let mut tr: HashMap<(String, String), String> = HashMap::new();
    for (k, l, v) in sqlx::query_as::<_, (String, String, String)>(
        "SELECT k.key, t.locale, t.value FROM keys k JOIN translations t ON t.key_id = k.id
         WHERE k.project_id = $1 AND k.key = ANY($2) AND t.status IN ('machine', 'reviewed')",
    )
    .bind(p.id)
    .bind(&keys)
    .fetch_all(&state.db)
    .await?
    {
        tr.insert((l, k), v);
    }
    let map = a
        .settings
        .0
        .get("locale_map")
        .cloned()
        .unwrap_or(Value::Null);
    let mut out = PushStats::default();
    let mut errors = vec![];
    for locale in &p.target_locales {
        let cms = map.get(locale).and_then(Value::as_str).unwrap_or(locale);
        let lookup = |k: &str| tr.get(&(locale.clone(), k.to_string())).cloned();
        let mut batch: Vec<(PushItem, String)> = vec![];
        for (r, f) in rows.iter().zip(&formats) {
            let Some(value) = rich::rebuild(&r.key, f, &r.source, &p.source_locale, &lookup) else {
                out.waiting += 1;
                continue;
            };
            let h = crate::auth::sha256_hex(&value);
            if r.pushed.0.get(locale) == Some(&h) {
                out.unchanged += 1;
                continue;
            }
            let format = match f {
                Format::RichText(_) => Format::RichText(serde_json::from_str(&value)?),
                f => f.clone(),
            };
            batch.push((
                PushItem {
                    external_id: r.external_id.clone(),
                    field: r.field.clone(),
                    value,
                    format,
                    meta: r.meta.clone(),
                },
                h,
            ));
        }
        for chunk in batch.chunks(c.push_batch().max(1)) {
            let items: Vec<PushItem> = chunk.iter().map(|x| x.0.clone()).collect();
            let report = match c.push(&creds, cms, &items).await {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("{locale}: {e:#}"));
                    break;
                }
            };
            let failed = |it: &PushItem| {
                report.failed.iter().any(|(id, _)| {
                    *id == it.external_id || *id == format!("{}/{}", it.external_id, it.field)
                })
            };
            for (it, h) in chunk.iter().filter(|(it, _)| !failed(it)) {
                sqlx::query(
                    "UPDATE connector_items SET pushed = jsonb_set(pushed, ARRAY[$4::text], to_jsonb($5::text)), pushed_at = now()
                     WHERE account_id = $1 AND external_id = $2 AND field = $3",
                )
                .bind(a.id)
                .bind(&it.external_id)
                .bind(&it.field)
                .bind(locale)
                .bind(h)
                .execute(&state.db)
                .await?;
                out.pushed += 1;
            }
            out.failed.extend(
                report
                    .failed
                    .iter()
                    .map(|(i, e)| (format!("{locale}:{i}"), e.clone())),
            );
        }
    }
    sqlx::query("UPDATE connector_accounts SET stats = stats || $2 WHERE id = $1")
        .bind(a.id)
        .bind(json!({"last_push": &out}))
        .execute(&state.db)
        .await?;
    if !errors.is_empty() {
        anyhow::bail!("{}", errors.join("; "));
    }
    if !out.failed.is_empty() {
        anyhow::bail!(
            "{} item(s) refused, first: {} ({})",
            out.failed.len(),
            out.failed[0].0,
            out.failed[0].1
        );
    }
    Ok(out)
}
