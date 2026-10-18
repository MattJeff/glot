use axum::{Json, extract::State, http::StatusCode};
use glot_core::glossary::{Glossary, Term};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, types::Json as DbJson};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
    validate_locale,
};

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TermRow {
    pub id: Uuid,
    pub term: String,
    pub description: Option<String>,
    pub do_not_translate: bool,
    pub case_sensitive: bool,
    #[schema(value_type = Object)]
    pub translations: DbJson<BTreeMap<String, String>>,
    #[schema(value_type = Object)]
    pub forbidden: DbJson<BTreeMap<String, Vec<String>>>,
}

impl From<TermRow> for Term {
    fn from(t: TermRow) -> Self {
        Term {
            term: t.term,
            description: t.description,
            do_not_translate: t.do_not_translate,
            case_sensitive: t.case_sensitive,
            translations: t.translations.0,
            forbidden: t.forbidden.0,
        }
    }
}

const COLS: &str =
    "id, term, description, do_not_translate, case_sensitive, translations, forbidden";

async fn rows<'e>(db: impl PgExecutor<'e>, project_id: Uuid) -> ApiResult<Vec<TermRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM glossary_terms WHERE project_id = $1 ORDER BY lower(term)"
    ))
    .bind(project_id)
    .fetch_all(db)
    .await?)
}

pub async fn load<'e>(db: impl PgExecutor<'e>, project_id: Uuid) -> ApiResult<Glossary> {
    Ok(Glossary {
        terms: rows(db, project_id)
            .await?
            .into_iter()
            .map(Term::from)
            .collect(),
    })
}

fn validate_term(t: &Term) -> ApiResult<()> {
    if t.term.trim().is_empty() || t.term.len() > 200 {
        return Err(AppError::bad_request("term is required (max 200 chars)"));
    }
    for loc in t.translations.keys().chain(t.forbidden.keys()) {
        validate_locale(loc)?;
    }
    Ok(())
}

/// Insert or replace by term (case-insensitive).
pub async fn upsert<'e>(db: impl PgExecutor<'e>, project_id: Uuid, t: &Term) -> ApiResult<TermRow> {
    validate_term(t)?;
    Ok(sqlx::query_as(&format!(
        "INSERT INTO glossary_terms (id, project_id, term, description, do_not_translate, case_sensitive, translations, forbidden)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (project_id, lower(term)) DO UPDATE SET term = $3, description = $4, do_not_translate = $5,
            case_sensitive = $6, translations = $7, forbidden = $8
         RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(t.term.trim())
    .bind(&t.description)
    .bind(t.do_not_translate)
    .bind(t.case_sensitive)
    .bind(DbJson(&t.translations))
    .bind(DbJson(&t.forbidden))
    .fetch_one(db)
    .await?)
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<TermRow>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(rows(&state.db, p.id).await?))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(t): AppJson<Term>,
) -> ApiResult<(StatusCode, Json<TermRow>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM glossary_terms WHERE project_id = $1 AND lower(term) = lower($2))")
        .bind(p.id)
        .bind(t.term.trim())
        .fetch_one(&state.db)
        .await?;
    if exists {
        return Err(AppError::conflict(format!(
            "term '{}' already exists",
            t.term
        )));
    }
    Ok((
        StatusCode::CREATED,
        Json(upsert(&state.db, p.id, &t).await?),
    ))
}

#[derive(Deserialize)]
pub struct PatchTerm {
    term: Option<String>,
    description: Option<String>,
    do_not_translate: Option<bool>,
    case_sensitive: Option<bool>,
    translations: Option<BTreeMap<String, String>>,
    forbidden: Option<BTreeMap<String, Vec<String>>>,
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<PatchTerm>,
) -> ApiResult<Json<TermRow>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let row: TermRow = sqlx::query_as(&format!(
        "SELECT {COLS} FROM glossary_terms WHERE id = $1 AND project_id = $2"
    ))
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("term not found"))?;
    let mut t = Term::from(row);
    if let Some(v) = b.term {
        t.term = v;
    }
    if let Some(v) = b.description {
        t.description = Some(v).filter(|d| !d.trim().is_empty());
    }
    if let Some(v) = b.do_not_translate {
        t.do_not_translate = v;
    }
    if let Some(v) = b.case_sensitive {
        t.case_sensitive = v;
    }
    if let Some(v) = b.translations {
        t.translations = v;
    }
    if let Some(v) = b.forbidden {
        t.forbidden = v;
    }
    validate_term(&t)?;
    Ok(Json(
        sqlx::query_as(&format!(
            "UPDATE glossary_terms SET term = $3, description = $4, do_not_translate = $5, case_sensitive = $6,
                translations = $7, forbidden = $8 WHERE id = $1 AND project_id = $2 RETURNING {COLS}"
        ))
        .bind(id)
        .bind(p.id)
        .bind(t.term.trim())
        .bind(&t.description)
        .bind(t.do_not_translate)
        .bind(t.case_sensitive)
        .bind(DbJson(&t.translations))
        .bind(DbJson(&t.forbidden))
        .fetch_one(&state.db)
        .await?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let res = sqlx::query("DELETE FROM glossary_terms WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(p.id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::not_found("term not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}
