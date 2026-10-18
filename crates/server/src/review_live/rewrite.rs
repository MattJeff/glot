//! RV4 — "ask for a rewrite" on an annotation (docs/design/live-review-contract.md, route `rephrase`).
//! `POST {M|W}/annotations/{id}/rephrase` → 2-3 proposals in the annotation's language, each declined in every
//! project locale through the glot pipeline (`glot_llm::rewrite`); cached per (key, instruction, context) in
//! `review_rewrites`. `POST …/rephrase/apply {rewrite_id, proposal_id}` writes the validated variants as reviewed
//! translations (history keeps the original).
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use glot_core::{
    config::{Config, LlmConfig},
    measure::Fit,
    validate::Syntax,
};
use glot_llm::rewrite::{Instruction, Preset, Rewrite, RewriteRequest};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::types::Json as DbJson;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use super::{
    api,
    model::{self, Access, Ctx},
};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState, keys::KeyRecord, projects::ProjectRow,
};

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct RephraseBody {
    /// shorter | clearer | formal | informal | warmer | direct | no_jargon | segment | brand_tone
    #[schema(value_type = Option<String>)]
    pub preset: Option<Preset>,
    /// Audience for `segment` ("developers", "first-time buyers"…).
    pub segment: Option<String>,
    /// Free instruction. Default (no preset either): the annotation's own text.
    pub instruction: Option<String>,
    /// 2-3 proposals (default 3).
    pub count: Option<usize>,
    /// Pixel budget measured by the widget on the element (`glot_core::measure::Fit`).
    #[schema(value_type = Option<Object>)]
    pub fit: Option<Fit>,
    /// Ignore the cache.
    #[serde(default)]
    pub refresh: bool,
    /// Reviewer name (review links).
    pub author_name: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Rephrased {
    /// Pass it to `rephrase/apply`.
    pub rewrite_id: Uuid,
    pub cached: bool,
    pub created_at: DateTime<Utc>,
    /// `glot_llm::rewrite::Rewrite`: proposals (variants per locale, back-translations, issues, policy), rejected, usage.
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub rewrite: Rewrite,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ApplyBody {
    pub rewrite_id: Uuid,
    pub proposal_id: usize,
    pub author_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct Ann {
    key_id: Option<Uuid>,
    locale: String,
    url: String,
    route: String,
    body: String,
}

fn llm_err(e: glot_llm::Error) -> AppError {
    AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", e.to_string())
}

fn id_of(ids: &HashMap<String, String>) -> ApiResult<Uuid> {
    ids.get("id")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::bad_request("id must be a uuid"))
}

async fn annotation(state: &AppState, p: &ProjectRow, id: Uuid) -> ApiResult<(Ann, KeyRecord)> {
    let a: Ann = sqlx::query_as(
        "SELECT key_id, locale, url, route, body FROM review_annotations WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("annotation not found"))?;
    let Some(key_id) = a.key_id else {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "no_key",
            "this annotation has no translation key",
        ));
    };
    let k: KeyRecord = sqlx::query_as(
        "SELECT id, key, source, description, refs, ui_role, component FROM keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&state.db)
    .await?;
    if a.locale != p.source_locale && !p.target_locales.contains(&a.locale) {
        return Err(AppError::bad_request(format!(
            "'{}' is not a locale of this project",
            a.locale
        )));
    }
    Ok((a, k))
}

/// The rewrite request of an annotation: current texts, thread, crop, glossary, style.
async fn request(
    state: &AppState,
    p: &ProjectRow,
    id: Uuid,
    b: RephraseBody,
) -> ApiResult<RewriteRequest> {
    let (a, k) = annotation(state, p, id).await?;
    let mut values: BTreeMap<String, String> = sqlx::query_as::<_, (String, String)>(
        "SELECT locale, value FROM translations WHERE key_id = $1 AND value <> ''",
    )
    .bind(k.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .filter(|(l, _)| p.target_locales.contains(l))
    .collect();
    values.insert(p.source_locale.clone(), k.source.clone());
    let comments: Vec<(String, String)> = sqlx::query_as(
        "SELECT author_name, body FROM review_comments WHERE annotation_id = $1 ORDER BY created_at LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let thread = std::iter::once(a.body.clone())
        .chain(
            comments
                .into_iter()
                .map(|(who, body)| format!("{who}: {body}")),
        )
        .collect();
    // Latest PNG capture (the providers take PNG only).
    let crop: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT data FROM review_attachments WHERE annotation_id = $1 AND content_type = 'image/png'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let blank = |s: &Option<String>| s.as_deref().is_none_or(|s| s.trim().is_empty());
    let text = if b.preset.is_none() && blank(&b.instruction) {
        Some(a.body.clone())
    } else {
        b.instruction.filter(|s| !s.trim().is_empty())
    };
    if text.as_ref().is_some_and(|t| t.chars().count() > 2000)
        || b.segment.as_ref().is_some_and(|t| t.chars().count() > 200)
    {
        return Err(AppError::bad_request(
            "instruction: at most 2000 characters, segment: at most 200",
        ));
    }
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    Ok(RewriteRequest {
        key: k.key.clone(),
        source_locale: p.source_locale.clone(),
        locale: a.locale.clone(),
        target_locales: p.target_locales.clone(),
        values,
        instruction: Instruction {
            preset: b.preset,
            segment: b.segment,
            text,
        },
        thread,
        context: glot_core::context::KeyContext {
            key: k.key.clone(),
            description: k.description.clone(),
            refs: k.refs.0.clone(),
            ui_role: k.ui_role.clone(),
            component: k.component.clone(),
            fit: b.fit,
            ..Default::default()
        },
        page: Some(if a.route.is_empty() { a.url } else { a.route }),
        crop: crop.map(|d| base64::engine::general_purpose::STANDARD.encode(d)),
        // ponytail: glossary terms of the key only (the prompt stays small).
        glossary: glossary.matches(&k.source).into_iter().cloned().collect(),
        style: p.style.clone(),
        // ponytail: syntax fixed to ICU like keys::check; project policy = defaults until the server stores `[policy]`.
        syntax: Syntax::Icu,
        policy: Default::default(),
        count: b.count.unwrap_or(3),
    })
}

/// `POST {M|W}/annotations/{id}/rephrase` (access `comment`+). The first valid proposal becomes the annotation's
/// suggestion (`set_suggestion`, SSE `suggestion_set`).
pub async fn rephrase_handler(
    State(state): State<AppState>,
    ctx: Ctx,
    AppPath(ids): AppPath<HashMap<String, String>>,
    AppJson(b): AppJson<RephraseBody>,
) -> ApiResult<Json<Rephrased>> {
    ctx.need(Access::Comment)?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let id = id_of(&ids)?;
    let r = rephrase(&state, &ctx.project, id, b).await?;
    if let Some(best) = r.rewrite.proposals.iter().find(|p| p.valid) {
        model::set_suggestion(&state, ctx.project.id, id, Some(&best.text), &actor).await?;
    }
    Ok(Json(r))
}

/// `POST {M|W}/annotations/{id}/rephrase/apply {rewrite_id, proposal_id}` (access `edit`+): writes the proposal's
/// validated variants of every target locale at once (RV1 `PUT /text` path: validator + glossary, reviewed,
/// history keeps the previous values), resolves the annotation. A source-locale wording is returned, not
/// written (the source lives in the code: `source_text`).
pub async fn apply_handler(
    State(state): State<AppState>,
    ctx: Ctx,
    AppPath(ids): AppPath<HashMap<String, String>>,
    AppJson(b): AppJson<ApplyBody>,
) -> ApiResult<Response> {
    ctx.need(Access::Edit)?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let id = id_of(&ids)?;
    let (a, _) = annotation(&state, &ctx.project, id).await?;
    let (key, values, source_text, text) = proposal_values(&state, &ctx.project, &b).await?;
    let glossary = crate::glossary::load(&state.db, ctx.project.id).await?;
    let mut tx = state.db.begin().await?;
    let k = crate::keys::find_key(&mut tx, ctx.project.id, &key).await?;
    let (issues, errors) = api::check_values(&ctx, &k.source, &values, None, &glossary)?;
    if !errors.is_empty() {
        return Ok(api::invalid(
            &api::results_json(&issues, &BTreeMap::new()),
            &errors.iter().collect::<Vec<_>>(),
        ));
    }
    let written = api::write_values(&mut tx, &k, &values, &issues, &glossary, &actor.name).await?;
    sqlx::query(
        "UPDATE review_annotations SET status = 'resolved', resolved_at = coalesce(resolved_at, now()),
           suggestion = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(&text)
    .execute(&mut *tx)
    .await?;
    model::emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "suggestion.applied",
        &actor,
        json!({"key": key, "locale": a.locale, "value": text, "locales": values.keys().collect::<Vec<_>>(),
               "rewrite_id": b.rewrite_id, "proposal_id": b.proposal_id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "annotation": model::one(&state, ctx.project.id, id, &ctx.actor).await?,
        "results": api::results_json(&issues, &written),
        "source_text": source_text,
    }))
    .into_response())
}

/// Generate (or serve from cache) the proposals of an annotation. Billing: the platform-key quota is checked
/// first and every declined string is counted after; BYOK projects are never limited.
pub async fn rephrase(
    state: &AppState,
    p: &ProjectRow,
    id: Uuid,
    b: RephraseBody,
) -> ApiResult<Rephrased> {
    let refresh = b.refresh;
    let req = request(state, p, id, b).await?;
    let cache_key = req.cache_key();
    if !refresh
        && let Some((rewrite_id, DbJson(rewrite), created_at)) = sqlx::query_as::<_, (Uuid, DbJson<Rewrite>, DateTime<Utc>)>(
            "SELECT id, result, created_at FROM review_rewrites WHERE project_id = $1 AND cache_key = $2",
        )
        .bind(p.id)
        .bind(&cache_key)
        .fetch_optional(&state.db)
        .await?
    {
        return Ok(Rephrased { rewrite_id, cached: true, created_at, rewrite });
    }

    crate::billing::check_llm(state, p).await?;
    let llm = &p.llm.0;
    let api_key = match &p.api_key_enc {
        Some(enc) => Some(crate::auth::decrypt(&state.secret, enc).ok_or_else(|| {
            AppError::internal("cannot decrypt the project API key (GLOT_SECRET_KEY changed?)")
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
            provider: llm.provider.clone(),
            model: llm.model.clone(),
            fallback_model: llm.fallback_model.clone(),
            judge_model: llm.judge_model.clone(),
            api_key_env: None,
            base_url: llm.base_url.clone(),
            budget_usd: llm.budget_usd,
            style: p.style.clone(),
            ..LlmConfig::default()
        },
        qa: Default::default(),
        extract: Default::default(),
        server: None,
        i18next: Default::default(),
    };
    let provider = || {
        glot_llm::provider::from_config_with(&config.llm, api_key.clone(), &llm.headers())
            .map_err(llm_err)
    };
    let translator = glot_llm::translate::Translator::new(&config, provider()?);
    // Writing copy is the judgment-heavy step: the judge model when set (usually the stronger one).
    let model = llm.judge_model.clone().unwrap_or_else(|| llm.model.clone());
    let rewrite = glot_llm::rewrite::rewrite(provider()?.as_ref(), &model, &translator, &req)
        .await
        .map_err(llm_err)?;

    let strings: usize = rewrite.proposals.iter().map(|p| p.variants.len()).sum();
    crate::billing::after_job(state, p.id, Uuid::new_v4(), strings as i64).await;
    let rewrite_id = Uuid::new_v4();
    let created_at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO review_rewrites (id, project_id, cache_key, result, cost_usd) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (project_id, cache_key) DO UPDATE SET id = $1, result = $4, cost_usd = $5, created_at = now()
         RETURNING created_at",
    )
    .bind(rewrite_id)
    .bind(p.id)
    .bind(&cache_key)
    .bind(DbJson(&rewrite))
    .bind(rewrite.usage.cost_usd)
    .fetch_one(&state.db)
    .await?;
    Ok(Rephrased {
        rewrite_id,
        cached: false,
        created_at,
        rewrite,
    })
}

/// A stored proposal: (key, validated target-locale values, source-locale wording, annotation-language text). 409 when the proposal
/// is not valid in every locale (nothing is written), 404 for an unknown rewrite / proposal.
pub async fn proposal_values(
    state: &AppState,
    p: &ProjectRow,
    b: &ApplyBody,
) -> ApiResult<(String, BTreeMap<String, String>, Option<String>, String)> {
    let DbJson(rw): DbJson<Rewrite> =
        sqlx::query_scalar("SELECT result FROM review_rewrites WHERE id = $1 AND project_id = $2")
            .bind(b.rewrite_id)
            .bind(p.id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| {
                AppError::not_found("rewrite not found (it may have been regenerated)")
            })?;
    let prop = rw
        .proposals
        .iter()
        .find(|x| x.id == b.proposal_id)
        .ok_or_else(|| AppError::not_found("proposal not found"))?;
    if !prop.valid {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "invalid_proposal",
            "this proposal does not pass validation in every locale; edit it manually instead",
        ));
    }
    Ok((
        rw.key,
        prop.variants
            .iter()
            .filter(|(l, _)| p.target_locales.contains(l))
            .map(|(l, v)| (l.clone(), v.value.clone()))
            .collect(),
        prop.variants.get(&p.source_locale).map(|v| v.value.clone()),
        prop.text.clone(),
    ))
}
