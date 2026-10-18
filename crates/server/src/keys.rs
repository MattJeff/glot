use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use glot_core::{
    context::SourceRef,
    glossary::Glossary,
    validate::{Constraints, Issue, Syntax},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, types::Json as DbJson};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
    validate_key,
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KeyRecord {
    pub id: Uuid,
    pub key: String,
    pub source: String,
    pub description: Option<String>,
    pub refs: DbJson<Vec<SourceRef>>,
    pub ui_role: Option<String>,
    pub component: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TranslationRecord {
    key_id: Uuid,
    locale: String,
    value: String,
    status: String,
    score: Option<i16>,
    issues: DbJson<Vec<Issue>>,
    origin: String,
    model: Option<String>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Translation {
    pub value: String,
    /// missing | machine | reviewed | stale
    pub status: String,
    pub score: Option<u8>,
    #[schema(value_type = Vec<Object>)]
    pub issues: Vec<Issue>,
    /// machine | human | imported
    pub origin: String,
    pub model: Option<String>,
    /// null only for status = missing
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<TranslationRecord> for Translation {
    fn from(t: TranslationRecord) -> Self {
        Self {
            value: t.value,
            status: t.status,
            score: t.score.map(|s| s.clamp(0, 100) as u8),
            issues: t.issues.0,
            origin: t.origin,
            model: t.model,
            updated_at: Some(t.updated_at),
        }
    }
}

impl Translation {
    fn missing() -> Self {
        Self {
            value: String::new(),
            status: "missing".into(),
            score: None,
            issues: vec![],
            origin: "machine".into(),
            model: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeyRow {
    pub key: String,
    pub description: Option<String>,
    pub source: String,
    #[schema(value_type = Vec<Object>)]
    pub refs: Vec<SourceRef>,
    /// button | heading | label | placeholder | aria | title | link | toast | text (from `glot push` extraction)
    pub ui_role: Option<String>,
    pub component: Option<String>,
    pub translations: BTreeMap<String, Translation>,
}

const TR_COLS: &str = "key_id, locale, value, status, score, issues, origin, model, updated_at";

pub async fn find_key(
    conn: &mut PgConnection,
    project_id: Uuid,
    key: &str,
) -> ApiResult<KeyRecord> {
    sqlx::query_as(
        "SELECT id, key, source, description, refs, ui_role, component FROM keys WHERE project_id = $1 AND key = $2",
    )
    .bind(project_id)
    .bind(key)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(format!("key '{key}' not found")))
}

async fn rows_for(
    state: &AppState,
    p: &ProjectRow,
    keys: Vec<KeyRecord>,
) -> ApiResult<Vec<KeyRow>> {
    let ids: Vec<Uuid> = keys.iter().map(|k| k.id).collect();
    let trs: Vec<TranslationRecord> = sqlx::query_as(&format!(
        "SELECT {TR_COLS} FROM translations WHERE key_id = ANY($1)"
    ))
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let mut by_key: BTreeMap<Uuid, BTreeMap<String, Translation>> = BTreeMap::new();
    for t in trs {
        by_key
            .entry(t.key_id)
            .or_default()
            .insert(t.locale.clone(), t.into());
    }
    Ok(keys
        .into_iter()
        .map(|k| {
            let mut translations = by_key.remove(&k.id).unwrap_or_default();
            for loc in &p.target_locales {
                translations
                    .entry(loc.clone())
                    .or_insert_with(Translation::missing);
            }
            KeyRow {
                key: k.key,
                description: k.description,
                source: k.source,
                refs: k.refs.0,
                ui_role: k.ui_role,
                component: k.component,
                translations,
            }
        })
        .collect())
}

pub struct Write<'a> {
    pub key: &'a KeyRecord,
    pub locale: &'a str,
    pub value: &'a str,
    /// machine | human | imported
    pub origin: &'a str,
    /// machine | reviewed
    pub status: &'a str,
    pub model: Option<&'a str>,
    pub score: Option<u8>,
    pub extra_issues: Vec<Issue>,
    pub author: Option<&'a str>,
}

/// The single write path for translations: validate + glossary check, store issues, record history.
pub async fn write_translation(
    conn: &mut PgConnection,
    glossary: &Glossary,
    w: Write<'_>,
) -> ApiResult<Translation> {
    let mut issues = check(&w.key.source, w.value, w.locale, glossary);
    for extra in w.extra_issues {
        if !issues.iter().any(|i| i.code == extra.code) {
            issues.push(extra);
        }
    }
    let score = w.score.map(i16::from);
    let t: TranslationRecord = sqlx::query_as(&format!(
        "INSERT INTO translations (key_id, locale, value, status, score, issues, origin, model, source_hash, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
         ON CONFLICT (key_id, locale) DO UPDATE SET value = $3, status = $4, score = $5, issues = $6, origin = $7,
            model = $8, source_hash = $9, updated_at = now()
         RETURNING {TR_COLS}"
    ))
    .bind(w.key.id)
    .bind(w.locale)
    .bind(w.value)
    .bind(w.status)
    .bind(score)
    .bind(DbJson(&issues))
    .bind(w.origin)
    .bind(w.model)
    .bind(glot_core::hash::hash(&w.key.source))
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("INSERT INTO translation_history (key_id, locale, value, origin, model, score, author) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(w.key.id)
        .bind(w.locale)
        .bind(w.value)
        .bind(w.origin)
        .bind(w.model)
        .bind(score)
        .bind(w.author)
        .execute(&mut *conn)
        .await?;
    Ok(t.into())
}

pub fn check(source: &str, target: &str, locale: &str, glossary: &Glossary) -> Vec<Issue> {
    let constraints = Constraints {
        max_length: None,
        max_length_ratio: Some(glot_core::config::QaConfig::default().max_length_ratio),
    };
    // ponytail: syntax fixed to ICU; add a project column if i18next/mf2 projects show up.
    let mut issues =
        glot_core::validate::validate(source, target, locale, Syntax::Icu, &constraints);
    issues.extend(glossary.check(source, target, locale));
    issues
}

fn require_target(p: &ProjectRow, locale: &str) -> ApiResult<()> {
    if p.target_locales.iter().any(|l| l == locale) {
        Ok(())
    } else {
        Err(AppError::bad_request(format!(
            "'{locale}' is not a target locale of this project"
        )))
    }
}

// ---------- handlers ----------

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    locale: Option<String>,
    status: Option<String>,
    page: Option<i64>,
    per_page: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct KeyPage {
    pub items: Vec<KeyRow>,
    pub total: i64,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<ListQuery>,
) -> ApiResult<Json<KeyPage>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(50).clamp(1, 500);
    let status = q.status.filter(|s| !s.is_empty());
    if let Some(s) = &status
        && !["missing", "machine", "reviewed", "stale"].contains(&s.as_str())
    {
        return Err(AppError::bad_request(
            "status must be missing|machine|reviewed|stale",
        ));
    }
    let locales = match q.locale.filter(|l| !l.is_empty()) {
        Some(l) => {
            require_target(&p, &l)?;
            vec![l]
        }
        None => p.target_locales.clone(),
    };
    let pattern = q.q.filter(|s| !s.is_empty()).map(|s| {
        format!(
            "%{}%",
            s.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let filter = "FROM keys k WHERE k.project_id = $1
        AND ($2::text IS NULL OR k.key ILIKE $2 OR k.source ILIKE $2)
        AND ($4::text IS NULL OR CASE WHEN $4 = 'missing'
            THEN EXISTS (SELECT 1 FROM unnest($3::text[]) l WHERE NOT EXISTS
                 (SELECT 1 FROM translations t WHERE t.key_id = k.id AND t.locale = l))
            ELSE EXISTS (SELECT 1 FROM translations t WHERE t.key_id = k.id AND t.locale = ANY($3) AND t.status = $4) END)";
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) {filter}"))
        .bind(p.id)
        .bind(&pattern)
        .bind(&locales)
        .bind(&status)
        .fetch_one(&state.db)
        .await?;
    let keys: Vec<KeyRecord> = sqlx::query_as(&format!(
        "SELECT k.id, k.key, k.source, k.description, k.refs, k.ui_role, k.component {filter} ORDER BY k.key LIMIT $5 OFFSET $6"
    ))
    .bind(p.id)
    .bind(&pattern)
    .bind(&locales)
    .bind(&status)
    .bind(per_page)
    .bind((page - 1) * per_page)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(KeyPage {
        items: rows_for(&state, &p, keys).await?,
        total,
    }))
}

#[derive(Deserialize)]
pub struct CreateBody {
    key: String,
    source: String,
    description: Option<String>,
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<KeyRow>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    validate_key(&b.key)?;
    if b.source.is_empty() {
        return Err(AppError::bad_request("source is required"));
    }
    let k: KeyRecord = sqlx::query_as(
        "INSERT INTO keys (id, project_id, key, source, description) VALUES ($1, $2, $3, $4, $5)
         RETURNING id, key, source, description, refs, ui_role, component",
    )
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(&b.key)
    .bind(&b.source)
    .bind(b.description.filter(|d| !d.trim().is_empty()))
    .fetch_one(&state.db)
    .await?;
    let row = rows_for(&state, &p, vec![k]).await?.remove(0);
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    source: Option<String>,
    description: Option<String>,
}

/// Change a key's source: translations produced from another source become `stale`.
pub async fn set_source(conn: &mut PgConnection, key_id: Uuid, source: &str) -> ApiResult<()> {
    sqlx::query("UPDATE keys SET source = $2 WHERE id = $1")
        .bind(key_id)
        .bind(source)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE translations SET status = 'stale' WHERE key_id = $1 AND source_hash <> $2")
        .bind(key_id)
        .bind(glot_core::hash::hash(source))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, key)): AppPath<(String, String)>,
    AppJson(b): AppJson<UpdateBody>,
) -> ApiResult<Json<KeyRow>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, p.id, &key).await?;
    if let Some(source) = b.source.filter(|s| *s != k.source) {
        if source.is_empty() {
            return Err(AppError::bad_request("source must not be empty"));
        }
        set_source(&mut tx, k.id, &source).await?;
    }
    if let Some(d) = b.description {
        sqlx::query("UPDATE keys SET description = $2 WHERE id = $1")
            .bind(k.id)
            .bind(Some(d).filter(|d| !d.trim().is_empty()))
            .execute(&mut *tx)
            .await?;
    }
    let k = find_key(&mut tx, p.id, &key).await?;
    tx.commit().await?;
    Ok(Json(rows_for(&state, &p, vec![k]).await?.remove(0)))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, key)): AppPath<(String, String)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let res = sqlx::query("DELETE FROM keys WHERE project_id = $1 AND key = $2")
        .bind(p.id)
        .bind(&key)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::not_found(format!("key '{key}' not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PutBody {
    value: String,
}

pub async fn put_translation(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, key, locale)): AppPath<(String, String, String)>,
    AppJson(b): AppJson<PutBody>,
) -> ApiResult<Json<Translation>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    require_target(&p, &locale)?;
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, p.id, &key).await?;
    let t = write_translation(
        &mut tx,
        &glossary,
        Write {
            key: &k,
            locale: &locale,
            value: &b.value,
            origin: "human",
            status: "reviewed",
            model: None,
            score: None,
            extra_issues: vec![],
            author: Some(&user.name),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(t))
}

pub async fn review(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, key, locale)): AppPath<(String, String, String)>,
) -> ApiResult<Json<Translation>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    require_target(&p, &locale)?;
    let mut conn = state.db.acquire().await?;
    let k = find_key(&mut conn, p.id, &key).await?;
    // Reviewing a stale translation confirms it for the current source.
    let t: TranslationRecord = sqlx::query_as(&format!(
        "UPDATE translations SET status = 'reviewed', source_hash = $3, updated_at = now()
         WHERE key_id = $1 AND locale = $2 RETURNING {TR_COLS}"
    ))
    .bind(k.id)
    .bind(&locale)
    .bind(glot_core::hash::hash(&k.source))
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found("no translation to review"))?;
    Ok(Json(t.into()))
}

#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct HistoryEntry {
    pub locale: String,
    pub value: String,
    pub origin: String,
    pub model: Option<String>,
    pub score: Option<i16>,
    pub created_at: DateTime<Utc>,
    pub author: Option<String>,
}

pub async fn history(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, key)): AppPath<(String, String)>,
) -> ApiResult<Json<Vec<HistoryEntry>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let mut conn = state.db.acquire().await?;
    let k = find_key(&mut conn, p.id, &key).await?;
    Ok(Json(
        sqlx::query_as(
            "SELECT locale, value, origin, model, score, created_at, author FROM translation_history
             WHERE key_id = $1 ORDER BY created_at DESC, id DESC LIMIT 500",
        )
        .bind(k.id)
        .fetch_all(&mut *conn)
        .await?,
    ))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct IssueRow {
    pub key: String,
    pub locale: String,
    #[schema(value_type = Object)]
    pub issue: Issue,
}

pub async fn issues(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<IssueRow>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<(String, String, DbJson<Vec<Issue>>)> = sqlx::query_as(
        "SELECT k.key, t.locale, t.issues FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND jsonb_array_length(t.issues) > 0 ORDER BY k.key, t.locale",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .flat_map(|(key, locale, issues)| {
                issues.0.into_iter().map(move |issue| IssueRow {
                    key: key.clone(),
                    locale: locale.clone(),
                    issue,
                })
            })
            .collect(),
    ))
}
