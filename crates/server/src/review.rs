//! Back-translation review endpoints (see ARCHITECTURE.md « Review API »). Synchronous LLM call, cached in `translation_reviews`.
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use glot_core::{config::LlmConfig, hash::hash};
use glot_llm::review::{Fidelity, ReviewItem};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
};

const MAX_KEYS: i64 = 200;

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Review {
    pub key: String,
    pub locale: String,
    pub source: String,
    pub target: String,
    pub back_translation: String,
    /// same | nuance | different
    #[schema(value_type = String)]
    pub verdict: Fidelity,
    pub explanation: String,
    pub suggestion: Option<String>,
    pub cached: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ReviewBody {
    locale: String,
    keys: Option<Vec<String>>,
    #[serde(default)]
    refresh: bool,
}

#[derive(Deserialize)]
pub struct LocaleQuery {
    locale: String,
}

pub async fn review_many(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<ReviewBody>,
) -> ApiResult<Json<Vec<Review>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(
        reviews(&state, &p, &b.locale, b.keys, b.refresh).await?,
    ))
}

pub async fn review_one(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, key)): AppPath<(String, String)>,
    AppQuery(q): AppQuery<LocaleQuery>,
) -> ApiResult<Json<Review>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    reviews(&state, &p, &q.locale, Some(vec![key]), false)
        .await?
        .pop()
        .map(Json)
        .ok_or_else(|| AppError::not_found("no translation to review for this key and locale"))
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    key: String,
    source: String,
    description: Option<String>,
    value: String,
    // cached review (LEFT JOIN)
    source_hash: Option<String>,
    target_hash: Option<String>,
    back_translation: Option<String>,
    verdict: Option<String>,
    explanation: Option<String>,
    suggestion: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

async fn reviews(
    state: &AppState,
    p: &ProjectRow,
    locale: &str,
    keys: Option<Vec<String>>,
    refresh: bool,
) -> ApiResult<Vec<Review>> {
    if !p.target_locales.iter().any(|l| l == locale) {
        return Err(AppError::bad_request(format!(
            "'{locale}' is not a target locale of this project"
        )));
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT k.id, k.key, k.source, k.description, t.value,
                r.source_hash, r.target_hash, r.back_translation, r.verdict, r.explanation, r.suggestion, r.created_at
         FROM keys k JOIN translations t ON t.key_id = k.id AND t.locale = $2
         LEFT JOIN translation_reviews r ON r.key_id = k.id AND r.locale = $2
         WHERE k.project_id = $1 AND t.value <> '' AND ($3::text[] IS NULL OR k.key = ANY($3))
         ORDER BY t.updated_at DESC LIMIT $4",
    )
    .bind(p.id)
    .bind(locale)
    .bind(&keys)
    .bind(MAX_KEYS)
    .fetch_all(&state.db)
    .await?;
    let fresh = |r: &Row| {
        !refresh
            && r.source_hash.as_deref() == Some(&hash(&r.source))
            && r.target_hash.as_deref() == Some(&hash(&r.value))
    };
    let todo: Vec<ReviewItem> = rows
        .iter()
        .filter(|r| !fresh(r))
        .map(|r| ReviewItem {
            key: r.key.clone(),
            source: r.source.clone(),
            target: r.value.clone(),
            description: r.description.clone(),
        })
        .collect();
    let mut computed = vec![];
    if !todo.is_empty() {
        let llm = &p.llm.0;
        let cfg = LlmConfig {
            provider: llm.provider.clone(),
            model: llm.model.clone(),
            base_url: llm.base_url.clone(),
            ..LlmConfig::default()
        };
        // ponytail: same key decryption as jobs::translate; share a helper if a third caller appears.
        let api_key = match &p.api_key_enc {
            Some(enc) => Some(crate::auth::decrypt(&state.secret, enc).ok_or_else(|| {
                AppError::internal("cannot decrypt the project API key (GLOT_SECRET_KEY changed?)")
            })?),
            None => None,
        };
        let llm_err = |e: glot_llm::Error| {
            AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", e.to_string())
        };
        let provider = glot_llm::provider::from_config(&cfg, api_key).map_err(llm_err)?;
        let glossary = crate::glossary::load(&state.db, p.id).await?;
        let model = llm.judge_model.clone().unwrap_or_else(|| llm.model.clone());
        let (got, _usage) = glot_llm::review::review(
            provider.as_ref(),
            &model,
            &p.source_locale,
            locale,
            &glossary,
            &todo,
        )
        .await
        .map_err(llm_err)?;
        let mut tx = state.db.begin().await?;
        for r in &got {
            let Some(row) = rows.iter().find(|x| x.key == r.key) else {
                continue;
            };
            sqlx::query(
                "INSERT INTO translation_reviews (key_id, locale, source_hash, target_hash, back_translation, verdict, explanation, suggestion)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (key_id, locale) DO UPDATE SET source_hash = $3, target_hash = $4, back_translation = $5,
                   verdict = $6, explanation = $7, suggestion = $8, created_at = now()",
            )
            .bind(row.id)
            .bind(locale)
            .bind(hash(&row.source))
            .bind(hash(&row.value))
            .bind(&r.back_translation)
            .bind(serde_json::to_value(r.verdict).ok().and_then(|v| v.as_str().map(String::from)))
            .bind(&r.explanation)
            .bind(&r.suggestion)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        computed = got;
    }
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let (back, verdict, explanation, suggestion, cached, at) =
                match computed.iter().find(|c| c.key == r.key) {
                    Some(c) => (
                        c.back_translation.clone(),
                        c.verdict,
                        c.explanation.clone(),
                        c.suggestion.clone(),
                        false,
                        Utc::now(),
                    ),
                    None if fresh(&r) => (
                        r.back_translation.clone()?,
                        serde_json::from_value(serde_json::Value::String(r.verdict.clone()?))
                            .ok()?,
                        r.explanation.clone()?,
                        r.suggestion.clone(),
                        true,
                        r.created_at?,
                    ),
                    None => return None, // the model skipped it
                };
            Some(Review {
                key: r.key,
                locale: locale.to_string(),
                source: r.source,
                target: r.value,
                back_translation: back,
                verdict,
                explanation,
                suggestion,
                cached,
                created_at: at,
            })
        })
        .collect())
}
