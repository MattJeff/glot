//! Live review REST handlers. Each handler serves both prefixes (member `/api/projects/{slug}/review`, widget
//! `/api/review/w`): `Ctx` resolves the caller and its access level. Contract: docs/design/live-review-contract.md.
use axum::{
    Json,
    body::Bytes,
    extract::{FromRequestParts, RawPathParams, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use glot_core::{
    measure::{self, Fit},
    validate::{Issue, Syntax, has_errors},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use super::model::{
    self, Access, Actor, Anchor, Annotation, AttachmentMeta, Comment, Ctx, EMOJIS, EVENT_COLS,
    EventRow, Filter, MAX_BODY_CHARS, ReviewEvent, STATUSES, clean, emit, json_error,
};
use crate::{
    ApiResult, AppError, AppJson, AppQuery, AppState,
    auth::sha256_hex,
    keys::{KeyRecord, Translation, Write, find_key, write_translation},
};

pub const MAX_CAPTURE: usize = 1024 * 1024;
const MAX_CAPTURES: i64 = 5;
const SESSION_HOURS: i64 = 12;

/// Path parameters by name (absent on routes without them).
pub struct Ids(HashMap<String, String>);

impl Ids {
    /// For in-process callers (remote MCP tools): `Ids::of([("id", id.to_string())])`.
    pub fn of<const N: usize>(pairs: [(&str, String); N]) -> Self {
        Ids(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Ids {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, s: &S) -> Result<Self, AppError> {
        Ok(Ids(RawPathParams::from_request_parts(parts, s)
            .await
            .map(|p| {
                p.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default()))
    }
}

impl Ids {
    fn str(&self, k: &str) -> ApiResult<String> {
        self.0
            .get(k)
            .cloned()
            .ok_or_else(|| AppError::bad_request(format!("missing {k}")))
    }
    fn uuid(&self, k: &str) -> ApiResult<Uuid> {
        self.str(k)?
            .parse()
            .map_err(|_| AppError::bad_request(format!("{k} must be a uuid")))
    }
}

/// `Option<Option<T>>`: absent = keep, null = clear.
fn some<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

async fn is_member(conn: &mut PgConnection, project: Uuid, user: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM project_members WHERE project_id = $1 AND user_id = $2)",
    )
    .bind(project)
    .bind(user)
    .fetch_one(conn)
    .await?)
}

// ---------- context, members ----------

#[derive(Serialize)]
pub struct Context {
    project: Value,
    access: &'static str,
    me: Actor,
    expires_at: Option<DateTime<Utc>>,
}

pub async fn context(ctx: Ctx) -> Json<Context> {
    let p = &ctx.project;
    Json(Context {
        project: json!({"slug": p.slug, "name": p.name, "source_locale": p.source_locale,
                        "target_locales": p.target_locales}),
        access: ctx.access.as_str(),
        me: ctx.actor.clone(),
        expires_at: ctx.expires_at,
    })
}

pub async fn members(State(state): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT u.id, u.name FROM project_members m JOIN users u ON u.id = m.user_id
         WHERE m.project_id = $1 ORDER BY u.name",
    )
    .bind(ctx.project.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, name)| json!({"user_id": id, "name": name}))
            .collect::<Vec<_>>()
    )))
}

// ---------- settings, links, member sessions (member routes only) ----------

fn member_only(ctx: &Ctx) -> ApiResult<()> {
    if ctx.widget {
        return Err(AppError::not_found("route not found"));
    }
    Ok(())
}

/// `https://app.example.com[:port]`, lowercased, no path.
pub fn parse_origin(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches('/').to_ascii_lowercase();
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))?;
    let ok = !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b));
    ok.then_some(s)
}

async fn origins(state: &AppState, project: Uuid) -> ApiResult<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT origins FROM review_settings WHERE project_id = $1")
            .bind(project)
            .fetch_optional(&state.db)
            .await?
            .unwrap_or_default(),
    )
}

pub async fn get_settings(State(state): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    member_only(&ctx)?;
    Ok(Json(
        json!({ "origins": origins(&state, ctx.project.id).await? }),
    ))
}

#[derive(Deserialize)]
pub struct SettingsBody {
    origins: Vec<String>,
}

pub async fn put_settings(
    State(state): State<AppState>,
    ctx: Ctx,
    AppJson(b): AppJson<SettingsBody>,
) -> ApiResult<Json<Value>> {
    member_only(&ctx)?;
    ctx.need_admin(&state).await?;
    if b.origins.len() > 20 {
        return Err(AppError::bad_request("at most 20 origins"));
    }
    let mut list = vec![];
    for o in &b.origins {
        let o = parse_origin(o).ok_or_else(|| {
            AppError::bad_request(format!(
                "invalid origin '{o}' (expected https://host[:port])"
            ))
        })?;
        if !list.contains(&o) {
            list.push(o);
        }
    }
    sqlx::query(
        "INSERT INTO review_settings (project_id, origins) VALUES ($1, $2)
         ON CONFLICT (project_id) DO UPDATE SET origins = $2",
    )
    .bind(ctx.project.id)
    .bind(&list)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "origins": list })))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ReviewLink {
    id: Uuid,
    label: Option<String>,
    access: String,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    #[sqlx(skip)]
    created_by: Value,
    #[serde(skip)]
    by_id: Option<Uuid>,
    #[serde(skip)]
    by_name: Option<String>,
}

const LINK_SELECT: &str = "SELECT l.id, l.label, l.access, l.created_at, l.expires_at, l.revoked_at, l.last_used_at,
    l.created_by AS by_id, u.name AS by_name FROM review_links l LEFT JOIN users u ON u.id = l.created_by
    WHERE l.project_id = $1 AND l.kind = 'link'";

fn link_view(mut l: ReviewLink) -> ReviewLink {
    l.created_by = json!({"user_id": l.by_id, "name": l.by_name});
    l
}

pub async fn list_links(
    State(state): State<AppState>,
    ctx: Ctx,
) -> ApiResult<Json<Vec<ReviewLink>>> {
    member_only(&ctx)?;
    ctx.need_admin(&state).await?;
    let rows: Vec<ReviewLink> =
        sqlx::query_as(&format!("{LINK_SELECT} ORDER BY l.created_at DESC"))
            .bind(ctx.project.id)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(rows.into_iter().map(link_view).collect()))
}

#[derive(Deserialize)]
pub struct LinkBody {
    label: Option<String>,
    access: String,
    /// absent = 14 days, null = never
    #[serde(default, deserialize_with = "some")]
    expires_in_days: Option<Option<i64>>,
}

fn new_token() -> String {
    format!("glr_{}", hex::encode(rand::random::<[u8; 32]>()))
}

pub async fn create_link(
    State(state): State<AppState>,
    ctx: Ctx,
    AppJson(b): AppJson<LinkBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    member_only(&ctx)?;
    ctx.need_admin(&state).await?;
    if !["read", "comment", "edit"].contains(&b.access.as_str()) {
        return Err(AppError::bad_request("access must be read|comment|edit"));
    }
    let days = b.expires_in_days.unwrap_or(Some(14));
    if days.is_some_and(|d| !(1..=365).contains(&d)) {
        return Err(AppError::bad_request(
            "expires_in_days must be 1-365 or null",
        ));
    }
    let token = new_token();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO review_links (id, project_id, kind, token_hash, access, label, created_by, expires_at)
         VALUES ($1, $2, 'link', $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(ctx.project.id)
    .bind(sha256_hex(&token))
    .bind(&b.access)
    .bind(b.label.map(|l| model::clean_line(&l, 100)).filter(|l| !l.is_empty()))
    .bind(ctx.actor.user_id)
    .bind(days.map(|d| Utc::now() + Duration::days(d)))
    .execute(&state.db)
    .await?;
    let link: ReviewLink = sqlx::query_as(&format!("{LINK_SELECT} AND l.id = $2"))
        .bind(ctx.project.id)
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "token": token, "link": link_view(link) })),
    ))
}

pub async fn revoke_link(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<StatusCode> {
    member_only(&ctx)?;
    ctx.need_admin(&state).await?;
    let n = sqlx::query(
        "UPDATE review_links SET revoked_at = coalesce(revoked_at, now())
         WHERE id = $1 AND project_id = $2 AND kind = 'link'",
    )
    .bind(ids.uuid("id")?)
    .bind(ctx.project.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("link not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Widget token for the calling member (Review page iframe, or the member's own site).
pub async fn session(State(state): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    member_only(&ctx)?;
    let uid = ctx.actor.user_id.ok_or_else(AppError::forbidden)?;
    sqlx::query("DELETE FROM review_links WHERE kind = 'member' AND expires_at < now()")
        .execute(&state.db)
        .await?;
    let token = new_token();
    let expires_at = Utc::now() + Duration::hours(SESSION_HOURS);
    sqlx::query(
        "INSERT INTO review_links (id, project_id, kind, token_hash, access, user_id, created_by, expires_at)
         VALUES ($1, $2, 'member', $3, 'member', $4, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(ctx.project.id)
    .bind(sha256_hex(&token))
    .bind(uid)
    .bind(expires_at)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "token": token, "expires_at": expires_at })))
}

// ---------- annotations ----------

#[derive(Serialize)]
pub struct Page {
    items: Vec<Annotation>,
    total: i64,
}

pub async fn list(
    State(state): State<AppState>,
    ctx: Ctx,
    AppQuery(f): AppQuery<Filter>,
) -> ApiResult<Json<Page>> {
    let (items, total) = model::query(&state, ctx.project.id, None, &f, &ctx.actor).await?;
    Ok(Json(Page { items, total }))
}

pub async fn get_one(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<Json<Annotation>> {
    Ok(Json(
        model::one(&state, ctx.project.id, ids.uuid("id")?, &ctx.actor).await?,
    ))
}

#[derive(Deserialize)]
pub struct CreateBody {
    anchor: Anchor,
    body: String,
    suggestion: Option<String>,
    assignee_id: Option<Uuid>,
    author_name: Option<String>,
}

fn suggestion(s: Option<String>) -> ApiResult<Option<String>> {
    s.filter(|s| !s.trim().is_empty())
        .map(|s| clean(&s, MAX_BODY_CHARS, "suggestion"))
        .transpose()
}

pub async fn create(
    State(state): State<AppState>,
    ctx: Ctx,
    AppJson(b): AppJson<CreateBody>,
) -> ApiResult<(StatusCode, Json<Annotation>)> {
    ctx.need(Access::Comment)?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let body = clean(&b.body, MAX_BODY_CHARS, "body")?;
    let suggestion = suggestion(b.suggestion)?;
    let anchor = b.anchor.normalize()?;
    let key_id = model::resolve_key(&state, &ctx.project, &anchor).await?;
    let mut tx = state.db.begin().await?;
    if let Some(a) = b.assignee_id {
        ctx.need(Access::Edit)?;
        if !is_member(&mut tx, ctx.project.id, a).await? {
            return Err(AppError::bad_request("assignee must be a project member"));
        }
    }
    crate::billing::consume(&state, ctx.project.id, "review_annotations").await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO review_annotations (id, project_id, anchor, url, route, locale, key_id, body, suggestion,
           author_user, author_link, author_name, assignee)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(ctx.project.id)
    .bind(sqlx::types::Json(&anchor))
    .bind(&anchor.url)
    .bind(anchor.route.as_deref().unwrap_or("/"))
    .bind(&anchor.locale)
    .bind(key_id)
    .bind(&body)
    .bind(&suggestion)
    .bind(actor.user_id)
    .bind(actor.link_id)
    .bind(&actor.name)
    .bind(b.assignee_id)
    .execute(&mut *tx)
    .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "annotation.created",
        &actor,
        json!({"route": anchor.route, "locale": anchor.locale}),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(model::one(&state, ctx.project.id, id, &ctx.actor).await?),
    ))
}

#[derive(Deserialize)]
pub struct PatchBody {
    status: Option<String>,
    #[serde(default, deserialize_with = "some")]
    assignee_id: Option<Option<Uuid>>,
    body: Option<String>,
    #[serde(default, deserialize_with = "some")]
    suggestion: Option<Option<String>>,
    anchor: Option<Anchor>,
    author_name: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppJson(b): AppJson<PatchBody>,
) -> ApiResult<Json<Annotation>> {
    ctx.need(Access::Comment)?;
    let id = ids.uuid("id")?;
    let cur = model::one(&state, ctx.project.id, id, &ctx.actor).await?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let author = ctx.is_author(&actor, &cur.author);
    let mut changed = serde_json::Map::new();
    if let Some(s) = &b.status {
        ctx.need(Access::Edit)?;
        if !STATUSES.contains(&s.as_str()) {
            return Err(AppError::bad_request(
                "status must be open|in_progress|resolved|wontfix",
            ));
        }
        changed.insert("status".into(), json!(s));
    }
    let mut tx = state.db.begin().await?;
    if let Some(a) = &b.assignee_id {
        ctx.need(Access::Edit)?;
        if let Some(u) = a
            && !is_member(&mut tx, ctx.project.id, *u).await?
        {
            return Err(AppError::bad_request("assignee must be a project member"));
        }
        changed.insert("assignee_id".into(), json!(a));
    }
    let body = match &b.body {
        Some(x) => {
            if !author {
                return Err(AppError::forbidden());
            }
            changed.insert("body".into(), json!(true));
            Some(clean(x, MAX_BODY_CHARS, "body")?)
        }
        None => None,
    };
    let sugg = match b.suggestion {
        Some(s) => {
            if !author {
                ctx.need(Access::Edit)?;
            }
            let s = suggestion(s)?;
            changed.insert("suggestion".into(), json!(s));
            Some(s)
        }
        None => None,
    };
    let (anchor, key_id) = match b.anchor {
        Some(a) => {
            if !author {
                return Err(AppError::forbidden());
            }
            let a = a.normalize()?;
            let k = model::resolve_key(&state, &ctx.project, &a).await?;
            changed.insert("anchor".into(), json!(true));
            (Some(a), k)
        }
        None => (None, None),
    };
    if changed.is_empty() {
        return Ok(Json(cur));
    }
    sqlx::query(
        "UPDATE review_annotations SET status = coalesce($2, status),
           resolved_at = CASE WHEN $2::text IS NULL THEN resolved_at
                              WHEN $2 IN ('resolved', 'wontfix') THEN coalesce(resolved_at, now()) ELSE NULL END,
           assignee = CASE WHEN $3 THEN $4 ELSE assignee END, body = coalesce($5, body),
           suggestion = CASE WHEN $6 THEN $7 ELSE suggestion END,
           anchor = coalesce($8, anchor), url = coalesce($9, url), route = coalesce($10, route),
           locale = coalesce($11, locale), key_id = CASE WHEN $8::jsonb IS NULL THEN key_id ELSE $12 END,
           updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&b.status)
    .bind(b.assignee_id.is_some())
    .bind(b.assignee_id.flatten())
    .bind(body)
    .bind(sugg.is_some())
    .bind(sugg.flatten())
    .bind(anchor.as_ref().map(sqlx::types::Json))
    .bind(anchor.as_ref().map(|a| a.url.clone()))
    .bind(anchor.as_ref().and_then(|a| a.route.clone()))
    .bind(anchor.as_ref().map(|a| a.locale.clone()))
    .bind(key_id)
    .execute(&mut *tx)
    .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "annotation.updated",
        &actor,
        Value::Object(changed),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        model::one(&state, ctx.project.id, id, &ctx.actor).await?,
    ))
}

pub async fn remove(State(state): State<AppState>, ctx: Ctx, ids: Ids) -> ApiResult<StatusCode> {
    ctx.need(Access::Comment)?;
    let id = ids.uuid("id")?;
    let cur = model::one(&state, ctx.project.id, id, &ctx.actor).await?;
    let actor = ctx.writer(None).unwrap_or_else(|_| ctx.actor.clone());
    if !ctx.is_author(&actor, &cur.author) && !ctx.is_admin(&state).await {
        return Err(AppError::forbidden());
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM review_annotations WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "annotation.deleted",
        &actor,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn history(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<Json<Vec<ReviewEvent>>> {
    let rows: Vec<EventRow> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLS} FROM review_events WHERE project_id = $1 AND annotation_id = $2 ORDER BY id LIMIT 1000"
    ))
    .bind(ctx.project.id)
    .bind(ids.uuid("id")?)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(ReviewEvent::from).collect()))
}

// ---------- comments, reactions ----------

#[derive(Deserialize)]
pub struct CommentBody {
    body: String,
    #[serde(default)]
    mentions: Vec<Uuid>,
    author_name: Option<String>,
}

async fn members_among(
    conn: &mut PgConnection,
    project: Uuid,
    ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM project_members WHERE project_id = $1 AND user_id = ANY($2) ORDER BY user_id",
    )
    .bind(project)
    .bind(ids)
    .fetch_all(conn)
    .await?)
}

/// Locks the annotation (must belong to the project) and bumps its updated_at.
async fn touch(conn: &mut PgConnection, project: Uuid, id: Uuid) -> ApiResult<()> {
    let n = sqlx::query(
        "UPDATE review_annotations SET updated_at = now() WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(project)
    .execute(conn)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("annotation not found"));
    }
    Ok(())
}

async fn find_comment(state: &AppState, ctx: &Ctx, id: Uuid, cid: Uuid) -> ApiResult<Comment> {
    model::comments(state, id, &ctx.actor)
        .await?
        .into_iter()
        .find(|c| c.id == cid)
        .ok_or_else(|| AppError::not_found("comment not found"))
}

pub async fn comment(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppJson(b): AppJson<CommentBody>,
) -> ApiResult<(StatusCode, Json<Comment>)> {
    ctx.need(Access::Comment)?;
    let id = ids.uuid("id")?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let body = clean(&b.body, MAX_BODY_CHARS, "body")?;
    let mut tx = state.db.begin().await?;
    touch(&mut tx, ctx.project.id, id).await?;
    let mentions = members_among(&mut tx, ctx.project.id, &b.mentions).await?;
    let cid = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO review_comments (id, annotation_id, author_user, author_link, author_name, body, mentions)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(cid)
    .bind(id)
    .bind(actor.user_id)
    .bind(actor.link_id)
    .bind(&actor.name)
    .bind(&body)
    .bind(&mentions)
    .execute(&mut *tx)
    .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        Some(cid),
        "comment.created",
        &actor,
        json!({"mentions": mentions}),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(find_comment(&state, &ctx, id, cid).await?),
    ))
}

#[derive(Deserialize)]
pub struct EditCommentBody {
    body: String,
    mentions: Option<Vec<Uuid>>,
    author_name: Option<String>,
}

pub async fn edit_comment(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppJson(b): AppJson<EditCommentBody>,
) -> ApiResult<Json<Comment>> {
    ctx.need(Access::Comment)?;
    let (id, cid) = (ids.uuid("id")?, ids.uuid("cid")?);
    let actor = ctx.writer(b.author_name.as_deref())?;
    let c = find_comment(&state, &ctx, id, cid).await?;
    touch(&mut *state.db.acquire().await?, ctx.project.id, id).await?;
    if !ctx.is_author(&actor, &c.author) {
        return Err(AppError::forbidden());
    }
    let body = clean(&b.body, MAX_BODY_CHARS, "body")?;
    let mut tx = state.db.begin().await?;
    let mentions = match &b.mentions {
        Some(m) => members_among(&mut tx, ctx.project.id, m).await?,
        None => c.mentions.clone(),
    };
    sqlx::query(
        "UPDATE review_comments SET body = $2, mentions = $3, edited_at = now() WHERE id = $1",
    )
    .bind(cid)
    .bind(&body)
    .bind(&mentions)
    .execute(&mut *tx)
    .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        Some(cid),
        "comment.updated",
        &actor,
        json!({"mentions": mentions}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(find_comment(&state, &ctx, id, cid).await?))
}

pub async fn delete_comment(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<StatusCode> {
    ctx.need(Access::Comment)?;
    let (id, cid) = (ids.uuid("id")?, ids.uuid("cid")?);
    let mut tx = state.db.begin().await?;
    touch(&mut tx, ctx.project.id, id).await?;
    let c = find_comment(&state, &ctx, id, cid).await?;
    let actor = ctx.writer(None).unwrap_or_else(|_| ctx.actor.clone());
    if !ctx.is_author(&actor, &c.author) && !ctx.is_admin(&state).await {
        return Err(AppError::forbidden());
    }
    sqlx::query("DELETE FROM review_comments WHERE id = $1")
        .bind(cid)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        Some(cid),
        "comment.deleted",
        &actor,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ReactBody {
    emoji: String,
    comment_id: Option<Uuid>,
    author_name: Option<String>,
}

pub async fn react(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppJson(b): AppJson<ReactBody>,
) -> ApiResult<Json<Value>> {
    ctx.need(Access::Comment)?;
    let id = ids.uuid("id")?;
    if !EMOJIS.contains(&b.emoji.as_str()) {
        return Err(AppError::bad_request(format!(
            "emoji must be one of {}",
            EMOJIS.join("|")
        )));
    }
    let actor = ctx.writer(b.author_name.as_deref())?;
    let mut tx = state.db.begin().await?;
    touch(&mut tx, ctx.project.id, id).await?;
    if let Some(c) = b.comment_id {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM review_comments WHERE id = $1 AND annotation_id = $2)",
        )
        .bind(c)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !ok {
            return Err(AppError::not_found("comment not found"));
        }
    }
    let removed = sqlx::query(
        "DELETE FROM review_reactions WHERE annotation_id = $1 AND comment_id IS NOT DISTINCT FROM $2
           AND actor = $3 AND emoji = $4",
    )
    .bind(id)
    .bind(b.comment_id)
    .bind(actor.key())
    .bind(&b.emoji)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if !removed {
        sqlx::query("INSERT INTO review_reactions (annotation_id, comment_id, actor, emoji) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(b.comment_id)
            .bind(actor.key())
            .bind(&b.emoji)
            .execute(&mut *tx)
            .await?;
    }
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        b.comment_id,
        "reaction.toggled",
        &actor,
        json!({"emoji": b.emoji, "comment_id": b.comment_id, "on": !removed}),
    )
    .await?;
    tx.commit().await?;
    let reactions = match b.comment_id {
        None => {
            model::one(&state, ctx.project.id, id, &actor)
                .await?
                .reactions
        }
        Some(c) => find_comment(&state, &ctx, id, c)
            .await
            .map(|c| c.reactions)?,
    };
    Ok(Json(json!({ "reactions": reactions })))
}

// ---------- captures ----------

/// (content type, width, height) from the magic bytes; dimensions for PNG only.
pub fn sniff(b: &[u8]) -> Option<(&'static str, Option<i32>, Option<i32>)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.len() >= 24 {
        let dim = |i: usize| i32::try_from(u32::from_be_bytes(b[i..i + 4].try_into().ok()?)).ok();
        return Some(("image/png", dim(16), dim(20)));
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(("image/jpeg", None, None));
    }
    if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(("image/webp", None, None));
    }
    None
}

pub async fn attach(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<AttachmentMeta>)> {
    ctx.need(Access::Comment)?;
    let id = ids.uuid("id")?;
    let actor = ctx.writer(None)?;
    if body.len() > MAX_CAPTURE {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "capture must be ≤ 1 MiB",
        ));
    }
    let (ct, w, h) = sniff(&body)
        .ok_or_else(|| AppError::bad_request("capture must be a PNG, JPEG or WebP image"))?;
    let mut tx = state.db.begin().await?;
    touch(&mut tx, ctx.project.id, id).await?;
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM review_attachments WHERE annotation_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if n >= MAX_CAPTURES {
        return Err(AppError::conflict(format!(
            "at most {MAX_CAPTURES} captures per annotation"
        )));
    }
    let aid = Uuid::new_v4();
    let meta: AttachmentMeta = sqlx::query_as(
        "INSERT INTO review_attachments (id, annotation_id, content_type, data, width, height, author_name, author_link, author_user)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id, annotation_id, content_type, octet_length(data)::int8 AS size, width, height, created_at",
    )
    .bind(aid)
    .bind(id)
    .bind(ct)
    .bind(body.as_ref())
    .bind(w)
    .bind(h)
    .bind(&actor.name)
    .bind(actor.link_id)
    .bind(actor.user_id)
    .fetch_one(&mut *tx)
    .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "attachment.created",
        &actor,
        json!({"attachment_id": aid}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(meta)))
}

pub async fn attachment(State(state): State<AppState>, ctx: Ctx, ids: Ids) -> ApiResult<Response> {
    let row: Option<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT f.content_type, f.data FROM review_attachments f JOIN review_annotations a ON a.id = f.annotation_id
         WHERE a.project_id = $1 AND a.id = $2 AND f.id = $3",
    )
    .bind(ctx.project.id)
    .bind(ids.uuid("id")?)
    .bind(ids.uuid("aid")?)
    .fetch_optional(&state.db)
    .await?;
    let (ct, data) = row.ok_or_else(|| AppError::not_found("capture not found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, ct),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".into(),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        ],
        data,
    )
        .into_response())
}

pub async fn delete_attachment(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<StatusCode> {
    ctx.need(Access::Comment)?;
    let (id, aid) = (ids.uuid("id")?, ids.uuid("aid")?);
    let mut tx = state.db.begin().await?;
    touch(&mut tx, ctx.project.id, id).await?;
    let author: Option<(Option<Uuid>, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT author_user, author_link, author_name FROM review_attachments WHERE id = $1 AND annotation_id = $2",
    )
    .bind(aid)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (u, l, name) = author.ok_or_else(|| AppError::not_found("capture not found"))?;
    let actor = ctx.writer(None).unwrap_or_else(|_| ctx.actor.clone());
    let owner = Actor {
        user_id: u,
        link_id: l,
        name,
    };
    if !ctx.is_author(&actor, &owner) && !ctx.is_admin(&state).await {
        return Err(AppError::forbidden());
    }
    sqlx::query("DELETE FROM review_attachments WHERE id = $1")
        .bind(aid)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "attachment.deleted",
        &actor,
        json!({"attachment_id": aid}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- annotation ↔ key ↔ translation ----------

fn require_target(ctx: &Ctx, locale: &str) -> ApiResult<()> {
    if ctx.project.target_locales.iter().any(|l| l == locale) {
        Ok(())
    } else {
        Err(AppError::bad_request(format!(
            "'{locale}' is not a target locale of this project"
        )))
    }
}

pub(super) fn invalid(results: &Value, issues: &[&Issue]) -> Response {
    let msg = issues
        .iter()
        .map(|i| i.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    json_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_translation",
        &msg,
        json!({ "results": results }),
    )
}

#[derive(Deserialize, Default)]
pub struct ApplyBody {
    value: Option<String>,
    author_name: Option<String>,
}

pub async fn apply(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    body: Bytes,
) -> ApiResult<Response> {
    ctx.need(Access::Edit)?;
    let id = ids.uuid("id")?;
    let b: ApplyBody = if body.is_empty() {
        ApplyBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?
    };
    let actor = ctx.writer(b.author_name.as_deref())?;
    let a = model::one(&state, ctx.project.id, id, &ctx.actor).await?;
    let key = a.key.clone().ok_or_else(|| {
        AppError::new(
            StatusCode::CONFLICT,
            "no_key",
            "this annotation has no translation key",
        )
    })?;
    require_target(&ctx, &a.locale)?;
    let value = b
        .value
        .or(a.suggestion.clone())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| AppError::bad_request("no suggestion: send a value"))?;
    let glossary = crate::glossary::load(&state.db, ctx.project.id).await?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, ctx.project.id, &key).await?;
    let issues = crate::keys::check(&k.source, &value, &a.locale, &glossary);
    if has_errors(&issues) {
        let errs: Vec<&Issue> = issues.iter().collect();
        let by_locale = BTreeMap::from([(a.locale.clone(), issues.clone())]);
        return Ok(invalid(&results_json(&by_locale, &BTreeMap::new()), &errs));
    }
    write_translation(
        &mut tx,
        &glossary,
        Write {
            key: &k,
            locale: &a.locale,
            value: &value,
            origin: "human",
            status: "reviewed",
            model: None,
            score: None,
            extra_issues: vec![],
            author: Some(&actor.name),
        },
    )
    .await?;
    sqlx::query(
        "UPDATE review_annotations SET status = 'resolved', resolved_at = coalesce(resolved_at, now()),
           suggestion = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(&value)
    .execute(&mut *tx)
    .await?;
    if let Some(g) = a.feedback_group_id {
        sqlx::query(
            "UPDATE feedback_groups SET status = 'accepted', proposal = $2, previous = coalesce(previous, $3), updated_at = now()
             WHERE id = $1 AND status IN ('open', 'proposed')",
        )
        .bind(g)
        .bind(&value)
        .bind(a.translation.as_ref().map(|t| &t.value))
        .execute(&mut *tx)
        .await?;
    }
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "suggestion.applied",
        &actor,
        json!({"key": key, "locale": a.locale, "value": value}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(model::one(&state, ctx.project.id, id, &ctx.actor).await?).into_response())
}

/// Feedback group (end-user reports) → annotation, anchored on its latest report.
pub async fn from_feedback(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<(StatusCode, Json<Annotation>)> {
    member_only(&ctx)?;
    let gid = ids.uuid("group_id")?;
    let g: Option<(Uuid, String, Option<String>, String)> = sqlx::query_as(
        "SELECT g.key_id, g.locale, g.proposal, k.key FROM feedback_groups g JOIN keys k ON k.id = g.key_id
         WHERE g.id = $1 AND k.project_id = $2",
    )
    .bind(gid)
    .bind(ctx.project.id)
    .fetch_optional(&state.db)
    .await?;
    let (key_id, locale, proposal, key) =
        g.ok_or_else(|| AppError::not_found("feedback group not found"))?;
    let reports: Vec<(String, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT kind, text, suggestion, url, dom FROM feedback_reports WHERE group_id = $1 ORDER BY id DESC LIMIT 20",
    )
    .bind(gid)
    .fetch_all(&state.db)
    .await?;
    let last = reports.first();
    let mut body = format!(
        "Converted from {} end-user feedback report(s):",
        reports.len()
    );
    for (kind, text, sugg, _, _) in reports.iter().take(5) {
        body.push_str(&format!("\n- *{kind}*: {}", model::clean_line(text, 200)));
        if let Some(s) = sugg {
            body.push_str(&format!(" → {}", model::clean_line(s, 200)));
        }
    }
    let anchor = Anchor {
        key: Some(key),
        key_source: Some("manual".into()),
        selector: last
            .and_then(|r| r.4.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "body".into()),
        text: last.map(|r| r.1.clone()).unwrap_or_default(),
        url: last.and_then(|r| r.3.clone()).unwrap_or_else(|| "/".into()),
        route: None,
        locale: locale.clone(),
        viewport: model::Viewport {
            w: 0.0,
            h: 0.0,
            dpr: None,
        },
        rect: None,
        point: None,
        dom: last.and_then(|r| r.4.clone()),
    }
    .normalize()?;
    let suggestion = proposal.or_else(|| last.and_then(|r| r.2.clone()));
    crate::billing::consume(&state, ctx.project.id, "review_annotations").await?;
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO review_annotations (id, project_id, anchor, url, route, locale, key_id, body, suggestion,
           author_user, author_name, feedback_group_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(ctx.project.id)
    .bind(sqlx::types::Json(&anchor))
    .bind(&anchor.url)
    .bind(anchor.route.as_deref().unwrap_or("/"))
    .bind(&locale)
    .bind(key_id)
    .bind(clean(&body, MAX_BODY_CHARS, "body")?)
    .bind(suggestion)
    .bind(ctx.actor.user_id)
    .bind(&ctx.actor.name)
    .bind(gid)
    .execute(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.status == StatusCode::CONFLICT => AppError::conflict("this feedback group is already an annotation"),
        e => e,
    })?;
    emit(
        &mut tx,
        ctx.project.id,
        Some(id),
        None,
        "annotation.created",
        &ctx.actor,
        json!({"route": anchor.route, "locale": locale, "feedback_group_id": gid}),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(model::one(&state, ctx.project.id, id, &ctx.actor).await?),
    ))
}

// ---------- manual text editing (contract §6) ----------

#[derive(Serialize)]
pub struct TextEntry {
    key: String,
    source: String,
    source_locale: String,
    description: Option<String>,
    ui_role: Option<String>,
    values: BTreeMap<String, Translation>,
}

async fn text_entry(conn: &mut PgConnection, ctx: &Ctx, k: &KeyRecord) -> ApiResult<TextEntry> {
    let rows: Vec<(String, String, String, Option<i16>, sqlx::types::Json<Vec<Issue>>, String, Option<String>, DateTime<Utc>)> =
        sqlx::query_as(
            "SELECT locale, value, status, score, issues, origin, model, updated_at FROM translations WHERE key_id = $1",
        )
        .bind(k.id)
        .fetch_all(conn)
        .await?;
    let mut values: BTreeMap<String, Translation> = rows
        .into_iter()
        .map(|(l, value, status, score, issues, origin, model, at)| {
            (
                l,
                Translation {
                    value,
                    status,
                    score: score.map(|s| s.clamp(0, 100) as u8),
                    issues: issues.0,
                    origin,
                    model,
                    updated_at: Some(at),
                },
            )
        })
        .collect();
    for l in &ctx.project.target_locales {
        values.entry(l.clone()).or_insert_with(|| Translation {
            value: String::new(),
            status: "missing".into(),
            score: None,
            issues: vec![],
            origin: "machine".into(),
            model: None,
            updated_at: None,
        });
    }
    Ok(TextEntry {
        key: k.key.clone(),
        source: k.source.clone(),
        source_locale: ctx.project.source_locale.clone(),
        description: k.description.clone(),
        ui_role: k.ui_role.clone(),
        values,
    })
}

pub async fn get_text(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<Json<TextEntry>> {
    let mut conn = state.db.acquire().await?;
    let k = find_key(&mut conn, ctx.project.id, &ids.str("key")?).await?;
    Ok(Json(text_entry(&mut conn, &ctx, &k).await?))
}

/// Validator + glossary + pixel budget for each value; (per-locale results, error issues).
pub(super) fn check_values(
    ctx: &Ctx,
    source: &str,
    values: &BTreeMap<String, String>,
    fit: Option<&Fit>,
    glossary: &glot_core::glossary::Glossary,
) -> ApiResult<(BTreeMap<String, Vec<Issue>>, Vec<Issue>)> {
    if values.is_empty() {
        return Err(AppError::bad_request(
            "values must name at least one locale",
        ));
    }
    let mut out = BTreeMap::new();
    let mut errors = vec![];
    for (loc, v) in values {
        require_target(ctx, loc)?;
        if v.trim().is_empty()
            || v.chars().count() > 10_000
            || v.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(AppError::bad_request(format!(
                "{loc}: value must be 1-10000 chars without control characters (DELETE the translation to clear it)"
            )));
        }
        let mut issues = crate::keys::check(source, v, loc, glossary);
        if let Some(f) = fit
            && let Some(i) = measure::check(v, Syntax::Icu, f)
        {
            issues.push(i);
        }
        if has_errors(&issues) {
            errors.extend(
                issues
                    .iter()
                    .filter(|i| has_errors(std::slice::from_ref(*i)))
                    .cloned(),
            );
        }
        out.insert(loc.clone(), issues);
    }
    Ok((out, errors))
}

pub(super) fn results_json(
    issues: &BTreeMap<String, Vec<Issue>>,
    written: &BTreeMap<String, Translation>,
) -> Value {
    Value::Object(
        issues
            .iter()
            .map(|(l, i)| {
                (
                    l.clone(),
                    json!({"issues": i, "translation": written.get(l)}),
                )
            })
            .collect(),
    )
}

pub(super) async fn write_values(
    tx: &mut PgConnection,
    k: &KeyRecord,
    values: &BTreeMap<String, String>,
    issues: &BTreeMap<String, Vec<Issue>>,
    glossary: &glot_core::glossary::Glossary,
    author: &str,
) -> ApiResult<BTreeMap<String, Translation>> {
    let mut written = BTreeMap::new();
    for (loc, v) in values {
        let t = write_translation(
            &mut *tx,
            glossary,
            Write {
                key: k,
                locale: loc,
                value: v,
                origin: "human",
                status: "reviewed",
                model: None,
                score: None,
                // keeps the pixel-budget issue (too_wide) next to the validator's (deduplicated by code)
                extra_issues: issues[loc].clone(),
                author: Some(author),
            },
        )
        .await?;
        written.insert(loc.clone(), t);
    }
    Ok(written)
}

#[derive(Deserialize)]
pub struct PutTextBody {
    values: BTreeMap<String, String>,
    fit: Option<Fit>,
    #[serde(default)]
    dry_run: bool,
    author_name: Option<String>,
}

pub async fn put_text(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppJson(b): AppJson<PutTextBody>,
) -> ApiResult<Response> {
    ctx.need(Access::Edit)?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let glossary = crate::glossary::load(&state.db, ctx.project.id).await?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, ctx.project.id, &ids.str("key")?).await?;
    let (issues, errors) = check_values(&ctx, &k.source, &b.values, b.fit.as_ref(), &glossary)?;
    if b.dry_run {
        return Ok(
            Json(json!({"results": results_json(&issues, &BTreeMap::new())})).into_response(),
        );
    }
    if !errors.is_empty() {
        return Ok(invalid(
            &results_json(&issues, &BTreeMap::new()),
            &errors.iter().collect::<Vec<_>>(),
        ));
    }
    let written = write_values(&mut tx, &k, &b.values, &issues, &glossary, &actor.name).await?;
    emit(
        &mut tx,
        ctx.project.id,
        None,
        None,
        "text.updated",
        &actor,
        json!({"key": k.key, "locales": b.values.keys().collect::<Vec<_>>()}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"results": results_json(&issues, &written)})).into_response())
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    locale: Option<String>,
}

pub async fn text_history(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppQuery(q): AppQuery<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.db.acquire().await?;
    let k = find_key(&mut conn, ctx.project.id, &ids.str("key")?).await?;
    let rows: Vec<(
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        DateTime<Utc>,
    )> = sqlx::query_as(
        "SELECT id, locale, value, origin, model, author, created_at FROM translation_history
         WHERE key_id = $1 AND ($2::text IS NULL OR locale = $2) ORDER BY id DESC LIMIT 200",
    )
    .bind(k.id)
    .bind(q.locale.filter(|l| !l.is_empty()))
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|(id, locale, value, origin, model, author, created_at)| json!({"id": id, "locale": locale,
            "value": value, "origin": origin, "model": model, "author": author, "created_at": created_at}))
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct RevertBody {
    locale: String,
    history_id: Option<i64>,
    author_name: Option<String>,
}

pub async fn revert_text(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
    AppJson(b): AppJson<RevertBody>,
) -> ApiResult<Response> {
    ctx.need(Access::Edit)?;
    require_target(&ctx, &b.locale)?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let glossary = crate::glossary::load(&state.db, ctx.project.id).await?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, ctx.project.id, &ids.str("key")?).await?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT value FROM translations WHERE key_id = $1 AND locale = $2")
            .bind(k.id)
            .bind(&b.locale)
            .fetch_optional(&mut *tx)
            .await?;
    // Default target: the latest version that differs from the live value (or the latest one when deleted).
    let target: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, value FROM translation_history WHERE key_id = $1 AND locale = $2
           AND ($3::int8 IS NULL OR id = $3) AND ($3::int8 IS NOT NULL OR value IS DISTINCT FROM $4)
         ORDER BY id DESC LIMIT 1",
    )
    .bind(k.id)
    .bind(&b.locale)
    .bind(b.history_id)
    .bind(&current)
    .fetch_optional(&mut *tx)
    .await?;
    let (hid, value) = target.ok_or_else(|| {
        AppError::new(
            StatusCode::CONFLICT,
            "nothing_to_revert",
            "no earlier version to revert to",
        )
    })?;
    let values = BTreeMap::from([(b.locale.clone(), value)]);
    let (issues, errors) = check_values(&ctx, &k.source, &values, None, &glossary)?;
    if !errors.is_empty() {
        return Ok(invalid(
            &results_json(&issues, &BTreeMap::new()),
            &errors.iter().collect::<Vec<_>>(),
        ));
    }
    let mut written = write_values(&mut tx, &k, &values, &issues, &glossary, &actor.name).await?;
    emit(
        &mut tx,
        ctx.project.id,
        None,
        None,
        "text.reverted",
        &actor,
        json!({"key": k.key, "locale": b.locale, "history_id": hid}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"translation": written.remove(&b.locale)})).into_response())
}

pub async fn delete_translation(
    State(state): State<AppState>,
    ctx: Ctx,
    ids: Ids,
) -> ApiResult<StatusCode> {
    ctx.need(Access::Edit)?;
    let locale = ids.str("locale")?;
    require_target(&ctx, &locale)?;
    let actor = ctx.writer(None)?;
    let mut tx = state.db.begin().await?;
    let k = find_key(&mut tx, ctx.project.id, &ids.str("key")?).await?;
    let n = sqlx::query("DELETE FROM translations WHERE key_id = $1 AND locale = $2")
        .bind(k.id)
        .bind(&locale)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("no translation for this locale"));
    }
    emit(
        &mut tx,
        ctx.project.id,
        None,
        None,
        "text.deleted",
        &actor,
        json!({"key": k.key, "locale": locale}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn slug_part(s: &str, words: usize) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(words)
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
        .chars()
        .take(40)
        .collect::<String>()
        .trim_end_matches('_')
        .to_string()
}

/// `/pricing/teams` + "Start free trial" → `pricing.teams.start_free_trial`.
pub fn generate_key(route: &str, text: &str) -> String {
    let scope: Vec<String> = route
        .split('/')
        .map(|s| slug_part(s, 3))
        .filter(|s| !s.is_empty())
        .take(3)
        .collect();
    let scope = if scope.is_empty() {
        "common".to_string()
    } else {
        scope.join(".")
    };
    let name = slug_part(text, 5);
    format!("{scope}.{}", if name.is_empty() { "text" } else { &name })
}

#[derive(Deserialize)]
pub struct CreateTextBody {
    source: String,
    key: Option<String>,
    description: Option<String>,
    anchor: Option<Anchor>,
    #[serde(default)]
    values: BTreeMap<String, String>,
    author_name: Option<String>,
}

pub async fn create_text(
    State(state): State<AppState>,
    ctx: Ctx,
    AppJson(b): AppJson<CreateTextBody>,
) -> ApiResult<Response> {
    ctx.need(Access::Edit)?;
    let actor = ctx.writer(b.author_name.as_deref())?;
    let source = clean(&b.source, 10_000, "source")?;
    let anchor = b.anchor.map(Anchor::normalize).transpose()?;
    let glossary = crate::glossary::load(&state.db, ctx.project.id).await?;
    let (issues, errors) = if b.values.is_empty() {
        (BTreeMap::new(), vec![])
    } else {
        check_values(&ctx, &source, &b.values, None, &glossary)?
    };
    if !errors.is_empty() {
        return Ok(invalid(
            &results_json(&issues, &BTreeMap::new()),
            &errors.iter().collect::<Vec<_>>(),
        ));
    }
    let base = match &b.key {
        Some(k) => {
            crate::validate_key(k)?;
            k.clone()
        }
        None => generate_key(
            anchor
                .as_ref()
                .and_then(|a| a.route.as_deref())
                .unwrap_or("/"),
            &source,
        ),
    };
    let description = b
        .description
        .map(|d| clean(&d, 2000, "description"))
        .transpose()?;
    let mut tx = state.db.begin().await?;
    let mut created = None;
    for i in 1..=50 {
        let key = if i == 1 {
            base.clone()
        } else {
            format!("{base}_{i}")
        };
        created = sqlx::query_scalar::<_, String>(
            "INSERT INTO keys (id, project_id, key, source, description) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (project_id, key) DO NOTHING RETURNING key",
        )
        .bind(Uuid::new_v4())
        .bind(ctx.project.id)
        .bind(&key)
        .bind(&source)
        .bind(&description)
        .fetch_optional(&mut *tx)
        .await?;
        if created.is_some() || b.key.is_some() {
            break;
        }
    }
    let key = created.ok_or_else(|| AppError::conflict(format!("key '{base}' already exists")))?;
    let k = find_key(&mut tx, ctx.project.id, &key).await?;
    if !b.values.is_empty() {
        write_values(&mut tx, &k, &b.values, &issues, &glossary, &actor.name).await?;
    }
    emit(
        &mut tx,
        ctx.project.id,
        None,
        None,
        "text.created",
        &actor,
        json!({"key": key}),
    )
    .await?;
    let entry = text_entry(&mut tx, &ctx, &k).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(entry)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_origins_and_images() {
        assert_eq!(
            generate_key("/pricing/teams", "Start free trial!"),
            "pricing.teams.start_free_trial"
        );
        assert_eq!(generate_key("/", "日本語"), "common.text");
        assert_eq!(
            parse_origin("HTTPS://App.example.com:8443/").as_deref(),
            Some("https://app.example.com:8443")
        );
        assert_eq!(parse_origin("https://a.b/path"), None);
        assert_eq!(parse_origin("javascript:alert(1)"), None);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        assert_eq!(sniff(&png), Some(("image/png", Some(640), Some(480))));
        assert_eq!(sniff(b"<svg onload=alert(1)>"), None);
    }
}
