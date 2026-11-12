//! Render protocol: one endpoint that localizes any piece of content for a recipient.
//!
//! `POST /v1/render` (and `/v1/render/batch`, up to 50 items) with a project render key
//! (`Authorization: Bearer glot_rk_…`): `{locale, type, content, variables?}` → the same shape, localized.
//! Types: `text`, `html`, `markdown`, `template` (ICU MessageFormat), `email` (`{subject, html, text}`) and
//! `json` (+ `fields`: which paths are text).
//!
//! What happens to a request:
//! 1. The content is cut into units ([`content`]): paragraphs, HTML segments (glot Edge's segmenter), Markdown
//!    segments (`glot docs`' parser), ICU messages. Variables, numbers, URLs and emails are placeholders in a unit.
//! 2. Each unit is looked up by hash in the project's render memory, then in what the project already has:
//!    catalog translations of the same source text, glot Edge's translation memory. Known = no model call.
//! 3. Unknown units are translated with the project's model, glossary and style through the same guarded
//!    translator as jobs, validated again here (placeholders, tags), and remembered.
//! 4. The document is put back together and checked (same tags, same links). If anything is missing or broken
//!    the whole document is returned in the source language with `status: "fallback"` and the reason: a caller
//!    can always send what it gets back.
//! 5. Variables are applied last ([`vars`]), formatted for the locale. They exist in memory for the duration of
//!    the request: never sent to a model, never stored, never logged.
//!
//! Metering: `billing::RENDER` (documents per month, plan quota), the existing `billing::LLM` strings quota for
//! units translated with glot's key, and `render_usage` (per day and key) for the dashboard.
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use glot_core::{
    context::KeyContext,
    edge::segment::SegmentOpts,
    glossary::Glossary,
    validate::{Severity, Syntax, has_errors},
};
use glot_llm::translate::{Item as LlmItem, Request};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::{AuthUser, sha256_hex},
    projects::{self, ProjectRow, Role},
    validate_locale,
};

mod content;
mod vars;

use content::{Kind, Unit};

pub const KEY_PREFIX: &str = "glot_rk_";
/// May translate content that is not in memory yet (spends the project's LLM budget).
pub const SCOPE_RENDER: &str = "render";
/// Only serves what is already in memory: a key that can never spend.
pub const SCOPE_CACHED: &str = "render:cached";
const MAX_ITEMS: usize = 50;
const MAX_BYTES: usize = 512 * 1024;
const MAX_UNITS: usize = 2_000;
const DEFAULT_TIMEOUT_MS: u64 = 20_000;

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/render", post(render_one))
        .route("/v1/render/batch", post(render_batch))
}

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/render";
    Router::new()
        .route(p, post(playground))
        .route(&format!("{p}/keys"), get(list_keys).post(create_key))
        .route(&format!("{p}/keys/{{id}}"), delete(delete_key))
        .route(&format!("{p}/usage"), get(usage))
}

// ---------- request ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    /// Echoed back (batch correlation).
    id: Option<String>,
    /// The recipient's locale (BCP 47; `fr_FR` is accepted).
    locale: String,
    #[serde(rename = "type", default = "default_type")]
    kind: String,
    content: Value,
    /// `json` only: the text fields.
    fields: Option<Value>,
    variables: Option<vars::Vars>,
    /// What this content is ("password reset email"): guides the wording of new translations. Not personal data.
    context: Option<String>,
    /// Accept a document whose untranslatable units stay in the source language (default: all or nothing).
    #[serde(default)]
    allow_partial: bool,
    /// Never call a model for this item: unknown content comes back in the source language at once.
    #[serde(default)]
    cache_only: bool,
}

fn default_type() -> String {
    "text".into()
}

#[derive(Deserialize)]
struct One {
    #[serde(flatten)]
    item: Item,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct Batch {
    items: Vec<Item>,
    timeout_ms: Option<u64>,
}

struct Access {
    /// Nil = the dashboard playground.
    key_id: Uuid,
    translate: bool,
    monthly_limit: Option<i64>,
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes) -> ApiResult<T> {
    if body.len() > MAX_BYTES {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!(
                "the request is larger than {} KB: split it into several calls",
                MAX_BYTES / 1024
            ),
        ));
    }
    serde_json::from_slice(body).map_err(|e| {
        AppError::bad_request(format!(
            "invalid request body: {e}. Expected {{\"locale\": \"fr\", \"type\": \"html\", \"content\": \"…\"}}"
        ))
    })
}

/// The project and rights behind `Authorization: Bearer glot_rk_…`.
async fn key_access(state: &AppState, headers: &HeaderMap) -> ApiResult<(ProjectRow, Access)> {
    let unauthorized = |m: &str| AppError::new(StatusCode::UNAUTHORIZED, "unauthorized", m);
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .ok_or_else(|| {
            unauthorized("send your project's render key: `Authorization: Bearer glot_rk_…` (Project → Render → Keys)")
        })?;
    if !token.starts_with(KEY_PREFIX) {
        return Err(unauthorized(
            "this is not a render key: they start with glot_rk_ and are created in Project → Render → Keys \
             (account API tokens do not work here)",
        ));
    }
    let row: Option<(Uuid, Uuid, Vec<String>, Option<i64>)> = sqlx::query_as(
        "UPDATE render_keys SET last_used_at = now() WHERE key_hash = $1
         RETURNING id, project_id, scopes, monthly_limit",
    )
    .bind(sha256_hex(token))
    .fetch_optional(&state.db)
    .await?;
    let (key_id, project, scopes, monthly_limit) =
        row.ok_or_else(|| unauthorized("unknown or revoked render key"))?;
    let p: ProjectRow = sqlx::query_as(
        "SELECT id, slug, name, source_locale, target_locales, style, public_cdn, llm, api_key_enc, created_at, syntax
         FROM projects WHERE id = $1",
    )
    .bind(project)
    .fetch_one(&state.db)
    .await?;
    Ok((
        p,
        Access {
            key_id,
            translate: scopes.iter().any(|s| s == SCOPE_RENDER),
            monthly_limit,
        },
    ))
}

fn idempotency(headers: &HeaderMap, body: &Bytes) -> ApiResult<Option<(String, String)>> {
    let Some(k) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let k = k.to_str().unwrap_or("").trim();
    if k.is_empty() || k.len() > 255 {
        return Err(AppError::bad_request(
            "Idempotency-Key must be 1-255 visible ASCII characters",
        ));
    }
    use sha2::Digest;
    Ok(Some((
        k.to_string(),
        hex::encode(sha2::Sha256::digest(body)),
    )))
}

async fn render_one(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let (p, access) = key_access(&state, &headers).await?;
    let One { item, timeout_ms } = parse(&body)?;
    let idem = idempotency(&headers, &body)?;
    let out = run(&state, &p, &access, vec![item], timeout_ms, idem).await?;
    let item = out.items.into_iter().next().unwrap_or(Value::Null);
    if let Some(m) = item["error"]["message"].as_str() {
        return Err(AppError::bad_request(m));
    }
    let cache = item["cache"].as_str().unwrap_or("none").to_string();
    let mut res = Json(item).into_response();
    if let Ok(v) = HeaderValue::from_str(&cache) {
        res.headers_mut().insert("x-glot-cache", v);
    }
    Ok(replayed(res, out.replayed))
}

async fn render_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let (p, access) = key_access(&state, &headers).await?;
    let b: Batch = parse(&body)?;
    let idem = idempotency(&headers, &body)?;
    let out = run(&state, &p, &access, b.items, b.timeout_ms, idem).await?;
    Ok(replayed(
        Json(json!({ "items": out.items })).into_response(),
        out.replayed,
    ))
}

fn replayed(mut res: Response, yes: bool) -> Response {
    if yes {
        res.headers_mut()
            .insert("idempotent-replayed", HeaderValue::from_static("true"));
    }
    res
}

/// Dashboard playground: the same pipeline with the session of a project member, metered like any call.
async fn playground(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let b: Batch = parse(&body)?;
    let access = Access {
        key_id: Uuid::nil(),
        translate: true,
        monthly_limit: None,
    };
    let out = run(&state, &p, &access, b.items, b.timeout_ms, None).await?;
    Ok(Json(json!({ "items": out.items })))
}

// ---------- pipeline ----------

struct Outcome {
    items: Vec<Value>,
    replayed: bool,
}

struct Doc {
    item: Item,
    locale: String,
    plan: Result<content::Plan, String>,
}

/// One unit to resolve: (unit, what it is for the translator, may a model be called).
type Todo = (Unit, String, bool);

#[derive(Default)]
struct Resolved {
    values: HashMap<String, String>,
    /// Hashes answered without a model call.
    cached: HashSet<String>,
    /// Why a unit has no translation.
    reasons: HashMap<String, String>,
}

async fn run(
    state: &AppState,
    p: &ProjectRow,
    access: &Access,
    items: Vec<Item>,
    timeout_ms: Option<u64>,
    idem: Option<(String, String)>,
) -> ApiResult<Outcome> {
    if items.is_empty() || items.len() > MAX_ITEMS {
        return Err(AppError::bad_request(format!(
            "`items` must hold 1 to {MAX_ITEMS} items"
        )));
    }
    if let Some((key, hash)) = &idem {
        let row: Option<(String, Value)> = sqlx::query_as(
            "SELECT request_hash, response FROM render_idempotency
             WHERE project_id = $1 AND key = $2 AND created_at > now() - interval '24 hours'",
        )
        .bind(p.id)
        .bind(key)
        .fetch_optional(&state.db)
        .await?;
        if let Some((h, stored)) = row {
            if h != *hash {
                return Err(AppError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "idempotency_key_reused",
                    "this Idempotency-Key was used with a different request body in the last 24 hours: \
                     use a new key for a new request",
                ));
            }
            // Same body, so the same variables: they are applied again to the stored templates.
            let items = stored
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .zip(&items)
                .map(|(pre, item)| finish(pre, item.variables.as_ref()))
                .collect();
            return Ok(Outcome {
                items,
                replayed: true,
            });
        }
    }

    // Quotas before any work. A replay above costs nothing.
    let n = items.len() as i64;
    if let Some(max) = access.monthly_limit {
        // ponytail: read then write, so concurrent calls can overshoot by a few requests; make it one
        // conditional UPDATE if a hard cap is ever sold.
        let used: i64 = sqlx::query_scalar(
            "SELECT coalesce(sum(requests), 0)::int8 FROM render_usage
             WHERE project_id = $1 AND key_id = $2 AND day >= date_trunc('month', now())::date",
        )
        .bind(p.id)
        .bind(access.key_id)
        .fetch_one(&state.db)
        .await?;
        if used + n > max {
            return Err(AppError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "key_quota_exceeded",
                format!(
                    "this render key reached its monthly limit ({used} of {max} documents used, {n} asked). \
                     Raise the limit in Project → Render → Keys, or wait until the 1st of the month (UTC)."
                ),
            ));
        }
    }
    crate::billing::consume_n(state, p.id, crate::billing::RENDER, n).await?;

    let glossary = Arc::new(crate::glossary::load(&state.db, p.id).await?);
    let mut opts = SegmentOpts::new(&p.source_locale);
    opts.dnt = glossary
        .terms
        .iter()
        .filter(|t| t.do_not_translate)
        .map(|t| t.term.clone())
        .collect();

    let docs: Vec<Doc> = items.into_iter().map(|item| prepare(item, &opts)).collect();

    // Units to resolve, per locale, once each.
    let mut todo: HashMap<String, Vec<Todo>> = HashMap::new();
    for d in &docs {
        let Ok(plan) = &d.plan else { continue };
        if same_locale(&d.locale, &p.source_locale) {
            continue;
        }
        let list = todo.entry(d.locale.clone()).or_default();
        let llm = access.translate && !d.item.cache_only;
        for leaf in &plan.leaves {
            for u in &leaf.units {
                match list.iter_mut().find(|t| t.0.hash == u.hash) {
                    Some(t) => t.2 |= llm,
                    None => list.push((
                        u.clone(),
                        describe(leaf.kind, d.item.context.as_deref()),
                        llm,
                    )),
                }
            }
        }
    }
    let timeout =
        Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).clamp(100, 120_000));
    let mut set = tokio::task::JoinSet::new();
    for (locale, list) in todo {
        let (state, p, glossary, key) = (state.clone(), p.clone(), glossary.clone(), access.key_id);
        let translate = access.translate;
        set.spawn(async move {
            let r = resolve(
                &state, &p, &glossary, &locale, list, timeout, key, translate,
            )
            .await;
            (locale, r)
        });
    }
    let mut resolved: HashMap<String, Resolved> = HashMap::new();
    while let Some(r) = set.join_next().await {
        let (locale, r) = r.map_err(AppError::internal)?;
        resolved.insert(locale, r?);
    }

    let empty = Resolved::default();
    let (mut units, mut cached, mut fallbacks) = (0i64, 0i64, 0i64);
    let mut pre = vec![];
    for d in &docs {
        let r = resolved.get(&d.locale).unwrap_or(&empty);
        let v = build(d, p, &opts, r);
        units += v["units"]["total"].as_i64().unwrap_or(0);
        cached += v["units"]["cached"].as_i64().unwrap_or(0);
        fallbacks += i64::from(v["status"] == "fallback");
        pre.push(v);
    }
    bump(
        &state.db,
        p.id,
        access.key_id,
        [n, units, cached, 0, fallbacks],
        0.0,
    )
    .await;

    // Only complete answers are replayed: a fallback retried with the same key gets a new chance.
    if let Some((key, hash)) = &idem
        && pre
            .iter()
            .all(|v| matches!(v["status"].as_str(), Some("translated" | "source")))
    {
        sqlx::query("DELETE FROM render_idempotency WHERE project_id = $1 AND created_at < now() - interval '24 hours'")
            .bind(p.id)
            .execute(&state.db)
            .await?;
        sqlx::query(
            "INSERT INTO render_idempotency (project_id, key, request_hash, response) VALUES ($1, $2, $3, $4)
             ON CONFLICT (project_id, key) DO UPDATE SET request_hash = excluded.request_hash,
                response = excluded.response, created_at = now()",
        )
        .bind(p.id)
        .bind(key)
        .bind(hash)
        .bind(Value::Array(pre.clone()))
        .execute(&state.db)
        .await?;
    }
    let items = pre
        .into_iter()
        .zip(&docs)
        .map(|(v, d)| finish(v, d.item.variables.as_ref()))
        .collect();
    Ok(Outcome {
        items,
        replayed: false,
    })
}

fn same_locale(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(&b.replace('_', "-"))
}

fn describe(kind: Kind, context: Option<&str>) -> String {
    let what = match kind {
        Kind::Text => {
            "A paragraph of a plain-text message sent to a customer (email, notification, document)."
        }
        Kind::Html => {
            "A piece of an HTML message sent to a customer (email, page, document). The numbered tags \
                       <0>…</0> stand for markup."
        }
        Kind::Markdown => "A piece of a Markdown document.",
        Kind::Template => {
            "A message template sent to a customer (notification, email subject, SMS)."
        }
    };
    let mut s = format!(
        "{what} Placeholders such as {{0}} and {{v0}} stand for values filled in later: keep each one exactly. \
         Write full natural sentences, not UI labels."
    );
    if let Some(c) = context.map(str::trim).filter(|c| !c.is_empty()) {
        s.push_str(&format!(
            " What this content is: {}",
            c.chars().take(300).collect::<String>()
        ));
    }
    s
}

/// Validate one item and cut it into units.
fn prepare(item: Item, opts: &SegmentOpts) -> Doc {
    let locale = item.locale.trim().replace('_', "-");
    let plan = (|| {
        validate_locale(&locale).map_err(|e| {
            format!(
                "{}: `locale` is the recipient's BCP 47 locale, e.g. \"fr\" or \"pt-BR\"",
                e.message
            )
        })?;
        let plan = content::plan(&item.kind, &item.content, item.fields.as_ref(), opts)?;
        let n: usize = plan.leaves.iter().map(|l| l.units.len()).sum();
        if n > MAX_UNITS {
            return Err(format!(
                "this document has {n} translatable units (max {MAX_UNITS}): split it into several items"
            ));
        }
        if let Some(v) = &item.variables {
            vars::validate(v, &locale)?;
            // A template must format with these variables before anything is spent on it.
            for l in plan.leaves.iter().filter(|l| l.kind == Kind::Template) {
                vars::icu(&l.source, v, &locale)?;
            }
        }
        Ok(plan)
    })();
    Doc { item, locale, plan }
}

/// One item's answer before variables: the localized templates, the counters and (private) where the leaves are.
fn build(d: &Doc, p: &ProjectRow, opts: &SegmentOpts, r: &Resolved) -> Value {
    let plan = match &d.plan {
        Ok(p) => p,
        Err(e) => {
            return json!({ "id": d.item.id, "error": { "code": "validation_error", "message": e } });
        }
    };
    let source = same_locale(&d.locale, &p.source_locale);
    let mut out = d.item.content.clone();
    let (mut total, mut cached, mut missing) = (0, 0, 0);
    let mut reason: Option<String> = None;
    let mut broken = false;
    for leaf in &plan.leaves {
        total += leaf.units.len();
        if source {
            continue;
        }
        cached += leaf
            .units
            .iter()
            .filter(|u| r.cached.contains(&u.hash))
            .count();
        let a = content::assemble(leaf, opts, &r.values, &d.locale);
        missing += a.missing;
        if a.broken {
            broken = true;
            reason = Some(
                "markup_integrity: the translated HTML did not keep the same tags and links".into(),
            );
        } else if leaf.kind == Kind::Template
            && let Some(v) = &d.item.variables
            && let Err(e) = vars::icu(&a.text, v, &d.locale)
        {
            broken = true;
            reason = Some(format!(
                "validation: the translated template does not format ({e})"
            ));
        }
        if reason.is_none() {
            reason = leaf
                .units
                .iter()
                .find_map(|u| r.reasons.get(&u.hash).cloned());
        }
        if let Some(slot) = out.pointer_mut(&leaf.pointer) {
            *slot = Value::String(a.text);
        }
    }
    let status = if source {
        "source"
    } else if missing == 0 && !broken {
        "translated"
    } else if d.item.allow_partial && !broken && missing < total {
        "partial"
    } else {
        "fallback"
    };
    if status == "fallback" {
        out = d.item.content.clone();
        missing = total;
        cached = 0;
    }
    let cache = match (source || total == 0, cached) {
        (true, _) => "none",
        (_, c) if c == total => "hit",
        (_, 0) => "miss",
        _ => "partial",
    };
    let mut v = json!({
        "id": d.item.id,
        "locale": d.locale,
        "type": d.item.kind,
        "status": status,
        "content": out,
        "cache": cache,
        "units": {
            "total": total,
            "cached": cached,
            "translated": if source { 0 } else { total - cached - missing },
            "fallback": if source { 0 } else { missing },
        },
        "warnings": plan.warnings,
        "_leaves": plan.leaves.iter().map(|l| json!([l.pointer, kind_str(l.kind)])).collect::<Vec<_>>(),
    });
    if matches!(status, "fallback" | "partial") {
        v["fallback_reason"] =
            json!(reason.unwrap_or_else(|| "unknown: some units have no translation".into()));
    }
    v
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Text => "text",
        Kind::Html => "html",
        Kind::Markdown => "markdown",
        Kind::Template => "template",
    }
}

/// Apply the request's variables to a built answer and drop its private fields.
fn finish(mut v: Value, variables: Option<&vars::Vars>) -> Value {
    let Some(o) = v.as_object_mut() else { return v };
    let leaves = o.remove("_leaves");
    let (Some(vars), Some(leaves)) = (variables, leaves) else {
        return v;
    };
    let locale = v["locale"].as_str().unwrap_or("en").to_string();
    for l in leaves.as_array().into_iter().flatten() {
        let (Some(ptr), Some(kind)) = (l[0].as_str(), l[1].as_str()) else {
            continue;
        };
        let Some(slot) = v["content"].pointer_mut(ptr) else {
            continue;
        };
        let Some(s) = slot.as_str() else { continue };
        let filled = match kind {
            "html" => vars::mustache(s, vars, &locale, vars::escape_html),
            // Validated in `prepare` (source) and `build` (translation): an error here leaves the template.
            "template" => vars::icu(s, vars, &locale).unwrap_or_else(|_| s.to_string()),
            _ => vars::mustache(s, vars, &locale, str::to_string),
        };
        *slot = Value::String(filled);
    }
    v
}

/// Second check of a translation, whatever produced it (model, catalog, Edge memory).
fn verify(u: &Unit, value: &str, locale: &str, glossary: &Glossary) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("empty translation".into());
    }
    let issues = crate::keys::check(&u.text, value, locale, u.syntax, glossary);
    if let Some(i) = issues.iter().find(|i| i.severity == Severity::Error) {
        return Err(i.message.clone());
    }
    if u.syntax == Syntax::Plain {
        content::placeholders_kept(&u.text, value)?;
    }
    Ok(())
}

async fn remember(
    db: &sqlx::PgPool,
    project: Uuid,
    locale: &str,
    rows: &[(&Unit, &str, &str, Option<&str>)],
) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let col = |f: fn(&(&Unit, &str, &str, Option<&str>)) -> String| {
        rows.iter().map(f).collect::<Vec<_>>()
    };
    sqlx::query(
        "INSERT INTO render_tm (project_id, locale, hash, source, value, origin, model)
         SELECT $1, $2, h, s, v, o, nullif(m, '')
         FROM unnest($3::text[], $4::text[], $5::text[], $6::text[], $7::text[]) AS x(h, s, v, o, m)
         ON CONFLICT DO NOTHING",
    )
    .bind(project)
    .bind(locale)
    .bind(col(|r| r.0.hash.clone()))
    .bind(col(|r| r.0.text.clone()))
    .bind(col(|r| r.1.to_string()))
    .bind(col(|r| r.2.to_string()))
    .bind(col(|r| r.3.unwrap_or("").to_string()))
    .execute(db)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn resolve(
    state: &AppState,
    p: &ProjectRow,
    glossary: &Arc<Glossary>,
    locale: &str,
    todo: Vec<Todo>,
    timeout: Duration,
    key_id: Uuid,
    key_translates: bool,
) -> ApiResult<Resolved> {
    let mut r = Resolved::default();
    let hashes: Vec<String> = todo.iter().map(|t| t.0.hash.clone()).collect();
    // ponytail: one UPDATE per request keeps the hit counters; batch them in memory if this ever shows in a profile.
    let known: Vec<(String, String)> = sqlx::query_as(
        "UPDATE render_tm SET hits = hits + 1, last_hit_at = now()
         WHERE project_id = $1 AND locale = $2 AND hash = ANY($3) RETURNING hash, value",
    )
    .bind(p.id)
    .bind(locale)
    .bind(&hashes)
    .fetch_all(&state.db)
    .await?;
    for (h, v) in known {
        r.cached.insert(h.clone());
        r.values.insert(h, v);
    }
    let missing: Vec<&Todo> = todo
        .iter()
        .filter(|t| !r.values.contains_key(&t.0.hash))
        .collect();
    if missing.is_empty() {
        return Ok(r);
    }

    // What the project already translated elsewhere: the catalog (same source text), then glot Edge's memory.
    let texts: Vec<String> = missing.iter().map(|t| t.0.text.clone()).collect();
    let catalog: HashMap<String, String> = sqlx::query_as(
        "SELECT DISTINCT ON (k.source) k.source, t.value FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND t.locale = $2 AND t.status <> 'stale' AND t.value <> '' AND k.source = ANY($3)
         ORDER BY k.source, (t.status = 'reviewed') DESC, k.key",
    )
    .bind(p.id)
    .bind(locale)
    .bind(&texts)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let edge: HashMap<String, String> = sqlx::query_as(
        "SELECT DISTINCT ON (hash) hash, value FROM edge_translations
         WHERE project_id = $1 AND locale = $2 AND hash = ANY($3) AND status IN ('machine', 'reviewed')
         ORDER BY hash, (status = 'reviewed') DESC, updated_at DESC",
    )
    .bind(p.id)
    .bind(locale)
    .bind(missing.iter().map(|t| t.0.hash.clone()).collect::<Vec<_>>())
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let mut reused: Vec<(&Unit, &str, &str, Option<&str>)> = vec![];
    for (u, _, _) in missing.iter().map(|t| &**t) {
        // A catalog value is written in the project's syntax: reused as an ICU template only when the project is
        // ICU, and as plain text only when the source has no syntax at all (the rule of the Edge memory).
        let catalog_ok = match u.syntax {
            Syntax::Plain => !u.text.contains(['{', '<']),
            _ => p.syntax == Syntax::Icu,
        };
        let hit = catalog
            .get(&u.text)
            .filter(|_| catalog_ok)
            .map(|v| (v, "catalog"))
            .or_else(|| edge.get(&u.hash).map(|v| (v, "edge")))
            .filter(|(v, _)| verify(u, v, locale, glossary).is_ok());
        if let Some((v, origin)) = hit {
            reused.push((u, v.as_str(), origin, None));
        }
    }
    remember(&state.db, p.id, locale, &reused).await?;
    for (u, v, _, _) in reused {
        r.cached.insert(u.hash.clone());
        r.values.insert(u.hash.clone(), v.to_string());
    }

    let (new, skipped): (Vec<&Todo>, Vec<&Todo>) = missing
        .into_iter()
        .filter(|t| !r.values.contains_key(&t.0.hash))
        .partition(|t| t.2);
    for t in skipped {
        let why = if key_translates {
            "not_cached: this content is not in memory yet and the request asked for `cache_only`"
        } else {
            "not_cached: this content is not in memory yet and this key has the `render:cached` scope only \
             (it never calls a model). Render it once with a `render` key."
        };
        r.reasons.insert(t.0.hash.clone(), why.into());
    }
    if new.is_empty() {
        return Ok(r);
    }
    let fail = |r: &mut Resolved, new: &[&Todo], why: String| {
        for t in new {
            r.reasons.insert(t.0.hash.clone(), why.clone());
        }
    };
    if let Err(e) = crate::billing::check_llm(state, p).await {
        fail(&mut r, &new, format!("llm_unavailable: {}", e.message));
        return Ok(r);
    }
    // Its own task: a caller that stops waiting (timeout, closed connection) does not lose the translation,
    // it lands in memory for the next identical request.
    // ponytail: two identical first requests at the same instant both translate (the second write is ignored);
    // take a Postgres advisory lock per (project, locale) here if duplicate spend ever matters.
    let units: Vec<(Unit, String)> = new.iter().map(|t| (t.0.clone(), t.1.clone())).collect();
    let task = tokio::spawn(translate(
        state.clone(),
        p.id,
        locale.to_string(),
        units,
        key_id,
    ));
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(done)) => {
            for (hash, res) in done {
                match res {
                    Ok(v) => {
                        r.values.insert(hash, v);
                    }
                    Err(why) => {
                        r.reasons.insert(hash, why);
                    }
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!(project = %p.slug, "render translation task failed: {e}");
            fail(&mut r, &new, "internal: the translation task failed".into());
        }
        Err(_) => fail(
            &mut r,
            &new,
            format!(
                "timeout: the translation took more than {} ms. It continues in the background: the next \
                 identical request is served from memory.",
                timeout.as_millis()
            ),
        ),
    }
    Ok(r)
}

/// Translate unknown units with the project's model, glossary and style; keep the valid ones.
/// Only `unit.text` (masked templates) and the caller's `context` reach the provider.
async fn translate(
    state: AppState,
    project: Uuid,
    locale: String,
    units: Vec<(Unit, String)>,
    key_id: Uuid,
) -> HashMap<String, Result<String, String>> {
    let mut out: HashMap<String, Result<String, String>> = HashMap::new();
    let all = |why: String| -> HashMap<String, Result<String, String>> {
        units
            .iter()
            .map(|(u, _)| (u.hash.clone(), Err(why.clone())))
            .collect()
    };
    let (p, _, translator, glossary) = match crate::jobs::setup(&state, project, None).await {
        Ok(s) => s,
        Err(e) => return all(format!("llm_setup: {}", short(&e.to_string()))),
    };
    let mut cost = 0.0;
    let mut models: HashMap<String, String> = HashMap::new();
    for syntax in [Syntax::Plain, Syntax::Icu] {
        let batch: Vec<&(Unit, String)> =
            units.iter().filter(|(u, _)| u.syntax == syntax).collect();
        if batch.is_empty() {
            continue;
        }
        let mut terms: Vec<glot_core::glossary::Term> = vec![];
        for (u, _) in &batch {
            for t in glossary.matches(&u.text) {
                if !terms.iter().any(|g| g.term == t.term) {
                    terms.push(t.clone());
                }
            }
        }
        let req = Request {
            source_locale: p.source_locale.clone(),
            target_locale: locale.clone(),
            items: batch
                .iter()
                .enumerate()
                .map(|(i, (u, desc))| LlmItem {
                    key: format!("u{i}"),
                    source: u.text.clone(),
                    context: KeyContext {
                        key: format!("u{i}"),
                        description: Some(desc.clone()),
                        ui_role: Some(u.role.into()),
                        ..Default::default()
                    },
                    previous: None,
                })
                .collect(),
            glossary: terms,
            syntax,
            style: p.style.clone(),
        };
        match translator.translate(req).await {
            Err(e) => {
                for (u, _) in &batch {
                    out.insert(
                        u.hash.clone(),
                        Err(format!("provider_error: {}", short(&e.to_string()))),
                    );
                }
            }
            Ok((results, usage)) => {
                cost += usage.cost_usd;
                for r in results {
                    let Some((u, _)) = r
                        .key
                        .strip_prefix('u')
                        .and_then(|i| i.parse::<usize>().ok())
                        .and_then(|i| batch.get(i))
                    else {
                        continue;
                    };
                    let checked = match r.issues.iter().find(|i| i.severity == Severity::Error) {
                        Some(i) => Err(i.message.clone()),
                        None if has_errors(&r.issues) => Err("rejected".into()),
                        None => verify(u, &r.value, &locale, &glossary),
                    };
                    models.insert(u.hash.clone(), r.model);
                    out.insert(
                        u.hash.clone(),
                        checked.map(|()| r.value).map_err(|e| {
                            format!(
                                "validation: the model's translation was refused ({})",
                                short(&e)
                            )
                        }),
                    );
                }
                for (u, _) in &batch {
                    out.entry(u.hash.clone()).or_insert_with(|| {
                        Err("validation: the model returned nothing for this unit".into())
                    });
                }
            }
        }
    }
    let ok: Vec<(&Unit, &str, &str, Option<&str>)> = units
        .iter()
        .filter_map(|(u, _)| {
            let v = out.get(&u.hash)?.as_ref().ok()?;
            Some((
                u,
                v.as_str(),
                "machine",
                models.get(&u.hash).map(String::as_str),
            ))
        })
        .collect();
    if let Err(e) = remember(&state.db, project, &locale, &ok).await {
        tracing::error!("render memory not written: {e}");
    }
    let n = ok.len() as i64;
    bump(&state.db, project, key_id, [0, 0, 0, n, 0], cost).await;
    crate::billing::after_job(&state, project, Uuid::new_v4(), n).await;
    out
}

fn short(s: &str) -> String {
    s.chars().take(300).collect()
}

/// Add to today's counters: [requests, units, cached, translated, fallbacks]. Never fails the request.
async fn bump(db: &sqlx::PgPool, project: Uuid, key_id: Uuid, n: [i64; 5], cost: f64) {
    let r = sqlx::query(
        "INSERT INTO render_usage (project_id, day, key_id, requests, units, cached, translated, fallbacks, cost_usd)
         VALUES ($1, current_date, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (project_id, day, key_id) DO UPDATE SET
            requests = render_usage.requests + excluded.requests, units = render_usage.units + excluded.units,
            cached = render_usage.cached + excluded.cached, translated = render_usage.translated + excluded.translated,
            fallbacks = render_usage.fallbacks + excluded.fallbacks, cost_usd = render_usage.cost_usd + excluded.cost_usd",
    )
    .bind(project)
    .bind(key_id)
    .bind(n[0])
    .bind(n[1])
    .bind(n[2])
    .bind(n[3])
    .bind(n[4])
    .bind(cost)
    .execute(db)
    .await;
    if let Err(e) = r {
        tracing::error!("render usage not counted: {e}");
    }
}

// ---------- keys ----------

#[derive(Serialize, sqlx::FromRow)]
struct KeyView {
    id: Uuid,
    name: String,
    /// First characters of the key, to tell keys apart.
    prefix: String,
    scopes: Vec<String>,
    monthly_limit: Option<i64>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    /// Documents rendered with this key this calendar month.
    used_this_month: i64,
}

async fn list_keys(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<KeyView>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(
        sqlx::query_as(
            "SELECT k.id, k.name, k.prefix, k.scopes, k.monthly_limit, k.created_at, k.last_used_at,
                    coalesce((SELECT sum(u.requests) FROM render_usage u
                              WHERE u.project_id = k.project_id AND u.key_id = k.id
                                AND u.day >= date_trunc('month', now())::date), 0)::int8 AS used_this_month
             FROM render_keys k WHERE k.project_id = $1 ORDER BY k.created_at",
        )
        .bind(p.id)
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct CreateKey {
    name: String,
    scopes: Option<Vec<String>>,
    monthly_limit: Option<i64>,
}

async fn create_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<CreateKey>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let name = b.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::bad_request("name is required (max 100 chars)"));
    }
    let scopes = b.scopes.unwrap_or_else(|| vec![SCOPE_RENDER.into()]);
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|s| s != SCOPE_RENDER && s != SCOPE_CACHED)
    {
        return Err(AppError::bad_request(format!(
            "scopes: \"{SCOPE_RENDER}\" (may translate new content) or \"{SCOPE_CACHED}\" (memory only)"
        )));
    }
    if b.monthly_limit.is_some_and(|l| l < 0) {
        return Err(AppError::bad_request("monthly_limit must be >= 0"));
    }
    let key = format!("{KEY_PREFIX}{}", hex::encode(rand::random::<[u8; 24]>()));
    let prefix = key[..KEY_PREFIX.len() + 6].to_string();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO render_keys (id, project_id, name, prefix, key_hash, scopes, monthly_limit)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(p.id)
    .bind(name)
    .bind(&prefix)
    .bind(sha256_hex(&key))
    .bind(&scopes)
    .bind(b.monthly_limit)
    .execute(&state.db)
    .await?;
    // The only time the key is shown.
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": name, "prefix": prefix, "scopes": scopes,
                     "monthly_limit": b.monthly_limit, "key": key }),
        ),
    ))
}

async fn delete_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let n = sqlx::query("DELETE FROM render_keys WHERE project_id = $1 AND id = $2")
        .bind(p.id)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("key not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- usage ----------

#[derive(Serialize, sqlx::FromRow, Default)]
struct Counters {
    requests: i64,
    units: i64,
    cached: i64,
    translated: i64,
    fallbacks: i64,
    cost_usd: f64,
}

#[derive(Serialize, sqlx::FromRow)]
struct Day {
    day: NaiveDate,
    #[sqlx(flatten)]
    #[serde(flatten)]
    n: Counters,
}

#[derive(Serialize)]
struct Usage {
    days: Vec<Day>,
    totals: Counters,
    /// Share of units answered from memory over the period; `null` before the first unit.
    hit_rate: Option<f64>,
    /// Units remembered, all locales.
    memory_entries: i64,
    memory_locales: i64,
}

#[derive(Deserialize)]
struct UsageQuery {
    days: Option<i32>,
}

async fn usage(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<UsageQuery>,
) -> ApiResult<Json<Usage>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days: Vec<Day> = sqlx::query_as(
        "SELECT day, sum(requests)::int8 AS requests, sum(units)::int8 AS units, sum(cached)::int8 AS cached,
                sum(translated)::int8 AS translated, sum(fallbacks)::int8 AS fallbacks, sum(cost_usd)::float8 AS cost_usd
         FROM render_usage WHERE project_id = $1 AND day > current_date - $2::int GROUP BY day ORDER BY day",
    )
    .bind(p.id)
    .bind(q.days.unwrap_or(30).clamp(1, 365))
    .fetch_all(&state.db)
    .await?;
    let mut totals = Counters::default();
    for d in &days {
        totals.requests += d.n.requests;
        totals.units += d.n.units;
        totals.cached += d.n.cached;
        totals.translated += d.n.translated;
        totals.fallbacks += d.n.fallbacks;
        totals.cost_usd += d.n.cost_usd;
    }
    let (memory_entries, memory_locales): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT locale) FROM render_tm WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(Usage {
        hit_rate: (totals.units > 0).then(|| totals.cached as f64 / totals.units as f64),
        days,
        totals,
        memory_entries,
        memory_locales,
    }))
}
