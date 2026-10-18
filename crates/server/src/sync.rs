use axum::{Json, extract::State};
use glot_core::{
    context::SourceRef,
    glossary::Term,
    hash::hash,
    lock::{LockEntry, Lockfile, Origin},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    keys::{KeyRecord, Write, set_source, write_translation},
    projects::{self, Role},
    validate_key,
};

type Catalogs = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Deserialize)]
pub struct PushBody {
    #[serde(default)]
    catalogs: Catalogs,
    #[serde(default)]
    lock: Lockfile,
    /// List of terms, same shape as `GET /glossary` (without ids).
    #[serde(default)]
    glossary: Vec<Term>,
    /// Code context from `glot extract`, per source key. Keys absent here keep their stored context.
    #[serde(default)]
    context: HashMap<String, CodeContext>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CodeContext {
    refs: Vec<SourceRef>,
    ui_role: Option<String>,
    component: Option<String>,
    description: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Conflict {
    pub locale: String,
    pub key: String,
    pub server: String,
    pub local: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PushResult {
    pub created: u32,
    pub updated: u32,
    pub conflicts: Vec<Conflict>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PullResult {
    #[schema(value_type = Object)]
    pub catalogs: Catalogs,
    #[schema(value_type = Object)]
    pub lock: Lockfile,
    #[schema(value_type = Vec<Object>)]
    pub glossary: Vec<Term>,
}

async fn key_map(
    conn: &mut sqlx::PgConnection,
    project_id: Uuid,
) -> sqlx::Result<HashMap<String, KeyRecord>> {
    let keys: Vec<KeyRecord> =
        sqlx::query_as("SELECT id, key, source, description, refs, ui_role, component FROM keys WHERE project_id = $1")
            .bind(project_id)
            .fetch_all(conn)
            .await?;
    Ok(keys.into_iter().map(|k| (k.key.clone(), k)).collect())
}

/// Push local catalogs. Source values create/update keys (changed source => stale translations).
/// A target value never overwrites a server translation that is reviewed or human-made: it's reported as a conflict.
pub async fn push(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PushBody>,
) -> ApiResult<Json<PushResult>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if let Some(bad) = b
        .catalogs
        .keys()
        .find(|l| **l != p.source_locale && !p.target_locales.contains(l))
    {
        return Err(AppError::bad_request(format!(
            "'{bad}' is not a locale of this project"
        )));
    }
    let mut out = PushResult {
        created: 0,
        updated: 0,
        conflicts: vec![],
    };
    let mut tx = state.db.begin().await?;
    let mut keys = key_map(&mut tx, p.id).await?;

    if let Some(source) = b.catalogs.get(&p.source_locale) {
        for (key, value) in source {
            validate_key(key)?;
            let ctx = b.context.get(key);
            // A missing local note never erases a description edited on the server.
            let note = b
                .lock
                .notes
                .get(key)
                .or(ctx.and_then(|c| c.description.as_ref()));
            match keys.get(key) {
                None => {
                    let none = CodeContext::default();
                    let c = ctx.unwrap_or(&none);
                    sqlx::query("INSERT INTO keys (id, project_id, key, source, description, refs, ui_role, component) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
                        .bind(Uuid::new_v4())
                        .bind(p.id)
                        .bind(key)
                        .bind(value)
                        .bind(note)
                        .bind(sqlx::types::Json(&c.refs))
                        .bind(&c.ui_role)
                        .bind(&c.component)
                        .execute(&mut *tx)
                        .await?;
                    out.created += 1;
                }
                Some(k) => {
                    if k.source != *value {
                        set_source(&mut tx, k.id, value).await?;
                        out.updated += 1;
                    }
                    if note.is_some() && note != k.description.as_ref() {
                        sqlx::query("UPDATE keys SET description = $2 WHERE id = $1")
                            .bind(k.id)
                            .bind(note)
                            .execute(&mut *tx)
                            .await?;
                    }
                    if let Some(c) = ctx
                        && (c.refs != k.refs.0
                            || c.ui_role != k.ui_role
                            || c.component != k.component)
                    {
                        sqlx::query(
                            "UPDATE keys SET refs = $2, ui_role = $3, component = $4 WHERE id = $1",
                        )
                        .bind(k.id)
                        .bind(sqlx::types::Json(&c.refs))
                        .bind(&c.ui_role)
                        .bind(&c.component)
                        .execute(&mut *tx)
                        .await?;
                    }
                }
            }
        }
        keys = key_map(&mut tx, p.id).await?;
    }

    for term in &b.glossary {
        crate::glossary::upsert(&mut *tx, p.id, term).await?;
    }
    let glossary = crate::glossary::load(&mut *tx, p.id).await?;

    for (locale, entries) in b.catalogs.iter().filter(|(l, _)| **l != p.source_locale) {
        let existing: HashMap<Uuid, (String, String, String)> = sqlx::query_as::<_, (Uuid, String, String, String)>(
            "SELECT t.key_id, t.value, t.status, t.origin FROM translations t JOIN keys k ON k.id = t.key_id
             WHERE k.project_id = $1 AND t.locale = $2",
        )
        .bind(p.id)
        .bind(locale)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|(id, v, s, o)| (id, (v, s, o)))
        .collect();
        for (key, value) in entries {
            // ponytail: target keys absent from the source are ignored (orphans).
            let Some(k) = keys.get(key) else { continue };
            let current = existing.get(&k.id);
            if let Some((server, status, origin)) = current {
                if server == value {
                    continue;
                }
                if status == "reviewed" || origin == "human" {
                    out.conflicts.push(Conflict {
                        locale: locale.clone(),
                        key: key.clone(),
                        server: server.clone(),
                        local: value.clone(),
                    });
                    continue;
                }
            }
            let entry = b.lock.entries.get(locale).and_then(|e| e.get(key));
            let (origin, model) = match entry.map(|e| &e.origin) {
                Some(Origin::Machine { model }) => ("machine", Some(model.as_str())),
                Some(Origin::Human) => ("human", None),
                _ => ("imported", None),
            };
            let status = match entry {
                Some(e) if e.source_hash != hash(&k.source) => "stale",
                Some(e) if e.reviewed || origin == "human" => "reviewed",
                _ => "machine",
            };
            write_translation(
                &mut tx,
                &glossary,
                Write {
                    key: k,
                    locale,
                    value,
                    origin,
                    status,
                    model,
                    score: entry.and_then(|e| e.score),
                    extra_issues: vec![],
                    author: Some(&user.name),
                },
            )
            .await?;
            if current.is_some() {
                out.updated += 1;
            } else {
                out.created += 1;
            }
        }
    }
    tx.commit().await?;
    tracing::info!(project = %p.slug, created = out.created, updated = out.updated, conflicts = out.conflicts.len(), "sync push");
    Ok(Json(out))
}

pub async fn pull(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<PullResult>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let keys: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT key, source, description FROM keys WHERE project_id = $1")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let mut catalogs: Catalogs = BTreeMap::new();
    let mut lock = Lockfile::default();
    let source = catalogs.entry(p.source_locale.clone()).or_default();
    for (key, src, description) in keys {
        source.insert(key.clone(), src);
        if let Some(d) = description {
            lock.notes.insert(key, d);
        }
    }
    for l in &p.target_locales {
        catalogs.entry(l.clone()).or_default();
    }
    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<i16>,
        String,
    )> = sqlx::query_as(
        "SELECT k.key, t.locale, t.value, t.status, t.origin, t.model, t.score, t.source_hash
         FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.project_id = $1",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    for (key, locale, value, status, origin, model, score, source_hash) in rows {
        let origin = match origin.as_str() {
            "machine" => Origin::Machine {
                model: model.unwrap_or_default(),
            },
            "human" => Origin::Human,
            _ => Origin::Imported,
        };
        lock.entries.entry(locale.clone()).or_default().insert(
            key.clone(),
            LockEntry {
                source_hash,
                target_hash: hash(&value),
                origin,
                score: score.map(|s| s.clamp(0, 100) as u8),
                reviewed: status == "reviewed",
                translated_at: None,
                reviewed_at: None,
                reviewed_by: None,
                needs_review: None,
            },
        );
        catalogs.entry(locale).or_default().insert(key, value);
    }
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    Ok(Json(PullResult {
        catalogs,
        lock,
        glossary: glossary.terms,
    }))
}
