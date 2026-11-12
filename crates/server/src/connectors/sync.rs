//! Generic sync: pull (paginated, resumable) → `connector_items` + glot keys → `connector_translate` job →
//! push of validated translations (idempotent per locale). One background worker per process.
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, types::Json as DbJson};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use uuid::Uuid;

use super::{Creds, Format, PushItem, SourceItem, oauth, quality, registry, rich};
use crate::{AppState, projects::ProjectRow};

/// Kind of the translation jobs a sync queues (runs `jobs::translate`, no email per job).
pub const JOB_KIND: &str = "connector_translate";
/// Translations are pushed again at most this often when nothing else triggers it (human reviews…).
const PUSH_EVERY: &str = "10 minutes";
/// Platforms without webhooks (or with a missed one) are pulled again, incrementally, this often.
const PULL_EVERY: &str = "24 hours";

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
    /// `translate` = stop after the pull (preview / paused), `push` = translate but do not write.
    pub hold: Option<String>,
    pub last_webhook_at: Option<DateTime<Utc>>,
}

pub const COLS: &str = "id, project_id, kind, name, creds_enc, settings, webhook_secret, status, sync_cursor, sync_since, sync_started_at, dirty, pending_job, last_sync_at, last_push_at, last_error, stats, created_at, hold, last_webhook_at";

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
    sqlx::query(&format!(
        "UPDATE connector_accounts SET status = 'queued', sync_since = last_sync_at
         WHERE status = 'ok' AND hold IS NULL AND pending_job IS NULL AND last_sync_at < now() - interval '{PULL_EVERY}'"
    ))
    .execute(&state.db)
    .await?;
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
           OR (pending_job IS NULL AND hold IS NULL AND (last_push_at IS NULL OR last_push_at < now() - interval '{PUSH_EVERY}')))
         ORDER BY last_push_at NULLS FIRST LIMIT 10"
    ))
    .fetch_all(&state.db)
    .await?;
    for a in due {
        // Held accounts (preview, paused, review before publishing) write nothing on their own.
        let res = match a.hold {
            None => push_ready(state, &a).await.map(|_| ()),
            Some(_) => Ok(()),
        };
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

/// `{kind}:{resource}:{id}:{field}` → `resource` (the content type a merchant can leave out).
fn resource_of(key: &str) -> &str {
    key.split(':').nth(1).unwrap_or_default()
}

/// Content types left out of the sync (`settings.exclude`, chosen in the dashboard from the preview).
fn excluded(settings: &Value) -> Vec<String> {
    settings["exclude"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// Pull every page into `connector_items` and glot keys; queue the translation; push what is ready.
/// With `hold = translate` it stops after the pull: the stats are the preview the merchant approves.
pub async fn run_sync(state: &AppState, a: &AccountRow) -> anyhow::Result<()> {
    let c = registry(&a.kind).ok_or_else(|| anyhow::anyhow!("unknown connector '{}'", a.kind))?;
    let creds = oauth::fresh(state, a).await?;
    let p = project(&state.db, a.project_id).await?;
    let mut cursor = a.sync_cursor.clone();
    let mut pulled = 0usize;
    loop {
        let page = c.pull(&creds, cursor.clone()).await?;
        pulled += page.items.len();
        store_page(state, a, &p, &page.items).await?;
        cursor = page.next;
        // The cursor is the resume point after a crash; `pulled` is the progress the dashboard shows.
        sqlx::query(
            "UPDATE connector_accounts SET sync_cursor = $2, stats = stats || $3 WHERE id = $1",
        )
        .bind(a.id)
        .bind(&cursor)
        .bind(json!({"pulled": pulled}))
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
    let plan = plan(state, a, &p).await?;
    let mut job = None;
    if plan.to_translate > 0 && a.hold.as_deref() != Some("translate") {
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
            keys: Some(plan.keys.clone()),
            // CMS content is not an ICU message: `{{ shop.name }}`, a lone `{` or `<` must not fail a parser.
            // Template code and markup are checked on the rebuilt value before it is written (`quality`).
            syntax: Some(glot_core::validate::Syntax::Plain),
            ..Default::default()
        }))
        .execute(&state.db)
        .await?;
        state.job_wakeup.notify_one();
        job = Some(id);
    }
    sqlx::query(
        "UPDATE connector_accounts SET sync_cursor = NULL, sync_since = NULL, last_sync_at = $2, pending_job = $3,
           stats = stats || $4 WHERE id = $1",
    )
    .bind(a.id)
    .bind(started)
    .bind(job)
    .bind(json!({"items": plan.items, "units": plan.keys.len(), "pulled": pulled, "to_translate": plan.to_translate,
        "by_type": plan.by_type, "resources": plan.resources, "words": plan.words, "cost_usd": plan.cost_usd,
        "locales": p.target_locales}))
    .execute(&state.db)
    .await?;
    if job.is_none() && a.hold.is_none() {
        push_ready(state, a).await?;
        sqlx::query("UPDATE connector_accounts SET last_push_at = now() WHERE id = $1")
            .bind(a.id)
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

/// What a sync is about to do, in a merchant's units: "412 products, 3 languages, about $1.20".
struct Plan {
    /// Translatable fields kept (after `exclude`).
    items: usize,
    /// Fields per content type, excluded types included (the dashboard lists them all to choose from).
    by_type: BTreeMap<String, usize>,
    /// Distinct resources (products, pages…) per content type.
    resources: BTreeMap<String, usize>,
    keys: Vec<String>,
    /// Missing or stale (unit, locale) pairs.
    to_translate: i64,
    /// Source words still to translate, all languages together.
    words: usize,
    cost_usd: f64,
}

async fn plan(state: &AppState, a: &AccountRow, p: &ProjectRow) -> anyhow::Result<Plan> {
    let all = items(&state.db, a.id, &[]).await?;
    let skip = excluded(&a.settings.0);
    let (mut by_type, mut seen) = (BTreeMap::new(), std::collections::BTreeSet::new());
    let mut units: Vec<(String, String)> = vec![];
    let mut kept = 0;
    for i in &all {
        let ty = resource_of(&i.key).to_string();
        *by_type.entry(ty.clone()).or_default() += 1;
        seen.insert((ty.clone(), i.external_id.clone()));
        if !skip.contains(&ty) {
            kept += 1;
            units.extend(rich::units(
                &i.key,
                &format_of(&i.format, &i.source),
                &i.source,
                &p.source_locale,
            ));
        }
    }
    let mut resources = BTreeMap::new();
    for (ty, _) in seen {
        *resources.entry(ty).or_default() += 1;
    }
    let keys: Vec<String> = units.iter().map(|u| u.0.clone()).collect();
    let done: std::collections::HashSet<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT k.key, t.locale FROM keys k JOIN translations t ON t.key_id = k.id
         WHERE k.project_id = $1 AND k.key = ANY($2) AND t.locale = ANY($3) AND t.status <> 'stale'",
    )
    .bind(p.id)
    .bind(&keys)
    .bind(&p.target_locales)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let (mut to_translate, mut words, mut cost_usd) = (0i64, 0usize, 0.0);
    for locale in &p.target_locales {
        let todo: Vec<(String, String)> = units
            .iter()
            .filter(|(k, _)| !done.contains(&(k.clone(), locale.clone())))
            .cloned()
            .collect();
        to_translate += todo.len() as i64;
        words += todo
            .iter()
            .map(|(_, t)| t.split_whitespace().count())
            .sum::<usize>();
        if !todo.is_empty() {
            cost_usd += crate::insights::estimate_cost(p, &todo, locale);
        }
    }
    Ok(Plan {
        items: kept,
        by_type,
        resources,
        keys,
        to_translate,
        words,
        cost_usd,
    })
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
    let skip = excluded(&a.settings.0);
    let mut source_cat = BTreeMap::new();
    let mut notes = BTreeMap::new();
    for it in items {
        crate::validate_key(&it.key).map_err(|e| anyhow::anyhow!(e.message))?;
        let (source, hash) = stored(it);
        // An excluded type is mirrored (it shows in the preview) with an empty hash and no glot key: including
        // it later makes its next pull look changed, which creates the keys.
        let out = skip.iter().any(|t| t == resource_of(&it.key));
        let hash = if out { String::new() } else { hash };
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
        if out || old.get(&(it.external_id.clone(), it.field.clone())) == Some(&hash) {
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
    written: DbJson<BTreeMap<String, String>>,
}

/// The account's items, without the content types in `exclude`.
async fn items(db: &sqlx::PgPool, account: Uuid, exclude: &[String]) -> sqlx::Result<Vec<ItemRow>> {
    sqlx::query_as(
        "SELECT external_id, field, key, format, source, meta, pushed, written FROM connector_items
         WHERE account_id = $1 AND NOT (split_part(key, ':', 2) = ANY($2)) ORDER BY external_id, field",
    )
    .bind(account)
    .bind(exclude)
    .fetch_all(db)
    .await
}

#[derive(Debug, Default, serde::Serialize)]
pub struct PushStats {
    pub pushed: usize,
    pub unchanged: usize,
    /// Items with at least one unit not translated (or stale) yet: not pushed.
    pub waiting: usize,
    /// Translations edited by a human in the platform since glot wrote them (or that existed before glot):
    /// kept as they are, unless `settings.overwrite` is on.
    pub kept: usize,
    pub failed: Vec<(String, String)>,
    /// The run to roll back, when something was written.
    pub run: Option<Uuid>,
}

/// Fingerprint of a value as text: markup and whitespace ignored, so a platform that re-serializes the HTML
/// glot wrote does not look like a human edit.
// ponytail: rich text JSON is compared as a string; a platform that reorders its keys would read as "edited"
// (kept, never lost). Compare text nodes if a connector with `current` on rich text shows it.
fn fingerprint(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut tag = false;
    for ch in value.chars() {
        match ch {
            '<' => tag = true,
            '>' if tag => tag = false,
            c if !tag => text.push(c),
            _ => {}
        }
    }
    crate::auth::sha256_hex(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A translation ready to be written: every unit translated, value different from the last one pushed.
pub struct Pending {
    pub locale: String,
    /// The platform's code for `locale` (`settings.locale_map`).
    pub cms_locale: String,
    pub item: PushItem,
    pub source: String,
    hash: String,
    /// Fingerprint of what glot last wrote for this locale, if it ever did.
    written: Option<String>,
    pub update: bool,
    /// Refused by [`quality::check`]: reported, not written.
    pub blocked: Option<String>,
}

/// Everything `push_ready` would write now, plus the (waiting, unchanged) counts. Reads the database only.
pub async fn pending(
    state: &AppState,
    a: &AccountRow,
) -> anyhow::Result<(Vec<Pending>, usize, usize)> {
    let p = project(&state.db, a.project_id).await?;
    let rows = items(&state.db, a.id, &excluded(&a.settings.0)).await?;
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
    let map = &a.settings.0["locale_map"];
    let (mut out, mut waiting, mut unchanged) = (vec![], 0, 0);
    for locale in &p.target_locales {
        let cms = map.get(locale).and_then(Value::as_str).unwrap_or(locale);
        let lookup = |k: &str| tr.get(&(locale.clone(), k.to_string())).cloned();
        for (r, f) in rows.iter().zip(&formats) {
            let Some(value) = rich::rebuild(&r.key, f, &r.source, &p.source_locale, &lookup) else {
                waiting += 1;
                continue;
            };
            let hash = crate::auth::sha256_hex(&value);
            if r.pushed.0.get(locale) == Some(&hash) {
                unchanged += 1;
                continue;
            }
            let blocked = quality::check(&r.source, &value, *f == Format::Html).err();
            let format = match f {
                Format::RichText(_) => Format::RichText(serde_json::from_str(&value)?),
                f => f.clone(),
            };
            out.push(Pending {
                locale: locale.clone(),
                cms_locale: cms.to_string(),
                item: PushItem {
                    external_id: r.external_id.clone(),
                    field: r.field.clone(),
                    value,
                    format,
                    meta: r.meta.clone(),
                },
                source: r.source.clone(),
                hash,
                written: r.written.0.get(locale).cloned(),
                update: r.pushed.0.contains_key(locale),
                blocked,
            });
        }
    }
    Ok((out, waiting, unchanged))
}

async fn mark_pushed(db: &sqlx::PgPool, account: Uuid, w: &Pending) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE connector_items SET pushed = jsonb_set(pushed, ARRAY[$4::text], to_jsonb($5::text)),
           written = jsonb_set(written, ARRAY[$4::text], to_jsonb($6::text)), pushed_at = now()
         WHERE account_id = $1 AND external_id = $2 AND field = $3",
    )
    .bind(account)
    .bind(&w.item.external_id)
    .bind(&w.item.field)
    .bind(&w.locale)
    .bind(&w.hash)
    .bind(fingerprint(&w.item.value))
    .execute(db)
    .await?;
    Ok(())
}

/// Pushes, per target locale, every item whose units are all translated (`machine` / `reviewed`) and whose
/// rebuilt value differs from the last one pushed. Never overwrites a translation a human edited in the
/// platform (when the connector can read translations back), never writes a value that fails
/// [`quality::check`], and records the previous values of what it writes (see [`rollback`]).
pub async fn push_ready(state: &AppState, a: &AccountRow) -> anyhow::Result<PushStats> {
    let c = registry(&a.kind).ok_or_else(|| anyhow::anyhow!("unknown connector '{}'", a.kind))?;
    let creds = oauth::fresh(state, a).await?;
    let (all, waiting, unchanged) = pending(state, a).await?;
    let overwrite = a.settings.0["overwrite"] == true;
    let mut out = PushStats {
        waiting,
        unchanged,
        ..Default::default()
    };
    let mut errors = vec![];
    let mut by_locale: BTreeMap<String, Vec<Pending>> = BTreeMap::new();
    for w in all {
        match &w.blocked {
            Some(why) => out.failed.push((
                format!("{}:{}/{}", w.locale, w.item.external_id, w.item.field),
                why.clone(),
            )),
            None => by_locale.entry(w.locale.clone()).or_default().push(w),
        }
    }
    for (locale, batch) in &by_locale {
        let cms = &batch[0].cms_locale;
        'chunks: for chunk in batch.chunks(c.push_batch().max(1)) {
            let items: Vec<PushItem> = chunk.iter().map(|w| w.item.clone()).collect();
            let current = match c.current(&creds, cms, &items).await {
                Ok(cur) => cur,
                // Not knowing what is there, nothing is written.
                Err(e) => {
                    errors.push(format!("{locale}: reading current translations: {e:#}"));
                    break 'chunks;
                }
            };
            let mut send: Vec<(&Pending, Option<String>)> = vec![];
            for (i, w) in chunk.iter().enumerate() {
                let there = current.as_ref().and_then(|c| c.get(i).cloned().flatten());
                if let Some(cur) = &there {
                    let f = fingerprint(cur);
                    if f == fingerprint(&w.item.value) {
                        mark_pushed(&state.db, a.id, w).await?;
                        out.unchanged += 1;
                        continue;
                    }
                    // Ours when it is what glot last wrote; rows pushed before fingerprints existed are ours too.
                    let ours = w.written.as_ref().map_or(w.update, |x| *x == f);
                    if !ours && !overwrite {
                        out.kept += 1;
                        continue;
                    }
                }
                send.push((w, there));
            }
            if send.is_empty() {
                continue;
            }
            let items: Vec<PushItem> = send.iter().map(|(w, _)| w.item.clone()).collect();
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
            for (w, previous) in send.iter().filter(|(w, _)| !failed(&w.item)) {
                let run = match out.run {
                    Some(r) => r,
                    None => {
                        let r = Uuid::new_v4();
                        sqlx::query("INSERT INTO connector_runs (id, account_id) VALUES ($1, $2)")
                            .bind(r)
                            .bind(a.id)
                            .execute(&state.db)
                            .await?;
                        out.run = Some(r);
                        r
                    }
                };
                sqlx::query(
                    "INSERT INTO connector_writes (run_id, external_id, field, locale, previous) VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT DO NOTHING",
                )
                .bind(run)
                .bind(&w.item.external_id)
                .bind(&w.item.field)
                .bind(locale)
                .bind(previous)
                .execute(&state.db)
                .await?;
                mark_pushed(&state.db, a.id, w).await?;
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
    out.failed.truncate(100);
    sqlx::query("UPDATE connector_accounts SET stats = stats || $2 WHERE id = $1")
        .bind(a.id)
        .bind(json!({"last_push": &out}))
        .execute(&state.db)
        .await?;
    if let Some(run) = out.run {
        sqlx::query("UPDATE connector_runs SET stats = $2 WHERE id = $1")
            .bind(run)
            .bind(json!({"pushed": out.pushed, "kept": out.kept, "failed": out.failed.len()}))
            .execute(&state.db)
            .await?;
    }
    if !errors.is_empty() {
        anyhow::bail!("{}", errors.join("; "));
    }
    if !out.failed.is_empty() {
        anyhow::bail!(
            "{} item(s) not written, first: {} ({})",
            out.failed.len(),
            out.failed[0].0,
            out.failed[0].1
        );
    }
    Ok(out)
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Rollback {
    /// Previous translations written back.
    pub restored: usize,
    /// Translations glot had created, deleted again.
    pub removed: usize,
    /// Translations glot had created on a platform that cannot delete them (or read them back): left in place.
    pub left: usize,
    /// Edited by a human in the platform since glot wrote them: not touched.
    pub kept: usize,
    pub failed: Vec<(String, String)>,
}

/// Undoes the latest push: every value it wrote goes back to what the platform had before, except those a
/// human edited since (kept). Runs are undone newest first, so a rollback never skips over a later write.
/// Publishing is then put on hold (`hold = push`), or the next automatic push would write the same again.
pub async fn rollback(state: &AppState, a: &AccountRow, run: Uuid) -> anyhow::Result<Rollback> {
    let c = registry(&a.kind).ok_or_else(|| anyhow::anyhow!("unknown connector '{}'", a.kind))?;
    let creds = oauth::fresh(state, a).await?;
    let latest: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM connector_runs WHERE account_id = $1 AND rolled_back_at IS NULL ORDER BY started_at DESC LIMIT 1",
    )
    .bind(a.id)
    .fetch_optional(&state.db)
    .await?;
    anyhow::ensure!(
        latest == Some(run),
        "only the latest sync that is not rolled back yet can be rolled back"
    );
    type Row = (
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        Value,
        Option<String>,
    );
    let writes: Vec<Row> = sqlx::query_as(
        "SELECT w.external_id, w.field, w.locale, w.previous, i.format, i.source, i.meta, i.written->>w.locale
         FROM connector_writes w JOIN connector_runs r ON r.id = w.run_id
         JOIN connector_items i ON i.account_id = r.account_id AND i.external_id = w.external_id AND i.field = w.field
         WHERE w.run_id = $1 AND r.account_id = $2 AND r.rolled_back_at IS NULL ORDER BY w.locale",
    )
    .bind(run)
    .bind(a.id)
    .fetch_all(&state.db)
    .await?;
    anyhow::ensure!(
        !writes.is_empty(),
        "nothing to roll back (already rolled back, or the content is gone)"
    );
    let map = &a.settings.0["locale_map"];
    let mut out = Rollback::default();
    // Per locale: (previous value or None, the item as written back, fingerprint of what glot last wrote).
    type Undo = (bool, PushItem, Option<String>);
    let mut all: BTreeMap<String, Vec<Undo>> = BTreeMap::new();
    for (external_id, field, locale, previous, format, source, meta, written) in writes {
        let item = |value: String| PushItem {
            external_id: external_id.clone(),
            field: field.clone(),
            format: match format_of(&format, &source) {
                Format::RichText(_) => {
                    Format::RichText(serde_json::from_str(&value).unwrap_or(Value::Null))
                }
                f => f,
            },
            value,
            meta: meta.clone(),
        };
        let had = previous.is_some();
        all.entry(locale)
            .or_default()
            .push((had, item(previous.unwrap_or_default()), written));
    }
    for (locale, undo) in &all {
        let cms = map.get(locale).and_then(Value::as_str).unwrap_or(locale);
        let items: Vec<PushItem> = undo.iter().map(|u| u.1.clone()).collect();
        let current = c.current(&creds, cms, &items).await?;
        let (mut restore, mut remove) = (vec![], vec![]);
        for (i, (had, item, written)) in undo.iter().enumerate() {
            let there = current.as_ref().and_then(|c| c.get(i).cloned().flatten());
            match (&there, written) {
                (Some(cur), Some(w)) if fingerprint(cur) != *w => out.kept += 1,
                _ if *had => restore.push(item.clone()),
                // Nothing there any more (or unreadable): nothing to delete for sure only when readable.
                _ if current.is_some() && there.is_none() => {}
                _ => remove.push(item.clone()),
            }
        }
        let (restore, remove) = (&restore, &remove);
        for chunk in restore.chunks(c.push_batch().max(1)) {
            let report = c.push(&creds, cms, chunk).await?;
            out.restored += report.ok;
            out.failed.extend(
                report
                    .failed
                    .iter()
                    .map(|(i, e)| (format!("{locale}:{i}"), e.clone())),
            );
            for it in chunk {
                sqlx::query(
                    "UPDATE connector_items SET pushed = jsonb_set(pushed, ARRAY[$4::text], to_jsonb($5::text)),
                       written = jsonb_set(written, ARRAY[$4::text], to_jsonb($6::text))
                     WHERE account_id = $1 AND external_id = $2 AND field = $3",
                )
                .bind(a.id)
                .bind(&it.external_id)
                .bind(&it.field)
                .bind(locale)
                .bind(crate::auth::sha256_hex(&it.value))
                .bind(fingerprint(&it.value))
                .execute(&state.db)
                .await?;
            }
        }
        if remove.is_empty() {
            continue;
        }
        if c.remove(&creds, cms, remove).await? {
            out.removed += remove.len();
            for it in remove {
                sqlx::query(
                    "UPDATE connector_items SET pushed = pushed - $4::text, written = written - $4::text
                     WHERE account_id = $1 AND external_id = $2 AND field = $3",
                )
                .bind(a.id)
                .bind(&it.external_id)
                .bind(&it.field)
                .bind(locale)
                .execute(&state.db)
                .await?;
            }
        } else {
            out.left += remove.len();
        }
    }
    sqlx::query(
        "UPDATE connector_runs SET rolled_back_at = now(), stats = stats || $2 WHERE id = $1",
    )
    .bind(run)
    .bind(json!({"rollback": &out}))
    .execute(&state.db)
    .await?;
    sqlx::query("UPDATE connector_accounts SET hold = 'push' WHERE id = $1")
        .bind(a.id)
        .execute(&state.db)
        .await?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::fingerprint;

    #[test]
    fn fingerprint_ignores_markup_only() {
        assert_eq!(
            fingerprint("<p>Chemise  en <b>coton</b></p>"),
            fingerprint("<p class=\"x\">Chemise en <strong>coton</strong></p>\n")
        );
        assert_ne!(
            fingerprint("<p>Chemise en coton</p>"),
            fingerprint("<p>Chemise en lin</p>")
        );
    }
}
