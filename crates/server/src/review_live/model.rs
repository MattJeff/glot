//! Live review model: who is calling (`Ctx`), anchors, annotations as returned by the API, history events,
//! content cleaning and the safe markdown renderer. Contract: docs/design/live-review-contract.md.
use axum::{
    Json,
    extract::{FromRequestParts, Query, RawPathParams},
    http::{HeaderMap, Method, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, types::Json as DbJson};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppState,
    auth::{authenticate, sha256_hex},
    projects::{self, ProjectRow, Role},
};

pub const STATUSES: &[&str] = &["open", "in_progress", "resolved", "wontfix"];
pub const EMOJIS: &[&str] = &["+1", "-1", "heart", "eyes", "check", "approve"];
pub const MAX_BODY_CHARS: usize = 5000;

// ---------- caller ----------

/// Ordered: each level can do everything the previous one can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    Read,
    Comment,
    Edit,
    Member,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Comment => "comment",
            Access::Edit => "edit",
            Access::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Actor {
    pub user_id: Option<Uuid>,
    pub link_id: Option<Uuid>,
    pub name: String,
}

impl Actor {
    /// Identity used for reactions: one per user, or per (link, name) for reviewers without an account.
    pub fn key(&self) -> String {
        match self.user_id {
            Some(u) => format!("u:{u}"),
            None => format!("l:{}:{}", self.link_id.unwrap_or_default(), self.name),
        }
    }
}

/// Resolved caller of a review route: a project member (`/api/projects/{slug}/review/…`, session or API token)
/// or a widget token (`/api/review/w/…`, `Bearer glr_…` or `?token=`).
pub struct Ctx {
    pub project: ProjectRow,
    /// For a review link, `name` is `?author_name=` (or empty); writes take it from the body (`Ctx::writer`).
    pub actor: Actor,
    pub access: Access,
    pub expires_at: Option<DateTime<Utc>>,
    pub widget: bool,
}

fn err(status: StatusCode, code: &'static str, msg: &str) -> AppError {
    AppError::new(status, code, msg)
}

impl Ctx {
    pub fn need(&self, a: Access) -> ApiResult<()> {
        if self.access >= a {
            Ok(())
        } else {
            Err(AppError::forbidden())
        }
    }

    pub async fn need_admin(&self, state: &AppState) -> ApiResult<()> {
        match self.actor.user_id {
            Some(u) => projects::load(state, u, &self.project.slug, Role::Admin)
                .await
                .map(|_| ()),
            None => Err(AppError::forbidden()),
        }
    }

    pub async fn is_admin(&self, state: &AppState) -> bool {
        self.need_admin(state).await.is_ok()
    }

    /// Author of a write. Review links must name the reviewer (body `author_name`, else `?author_name=`).
    pub fn writer(&self, name: Option<&str>) -> ApiResult<Actor> {
        if self.actor.user_id.is_some() {
            return Ok(self.actor.clone());
        }
        let name = name
            .map(|n| clean_line(n, 80))
            .filter(|n| !n.is_empty())
            .or_else(|| Some(self.actor.name.clone()).filter(|n| !n.is_empty()))
            .ok_or_else(|| {
                AppError::bad_request("author_name is required (1-80 chars) for review links")
            })?;
        Ok(Actor {
            name,
            ..self.actor.clone()
        })
    }

    /// Same user; for links, same link and same reviewer name.
    pub fn is_author(&self, who: &Actor, author: &Actor) -> bool {
        match who.user_id {
            Some(u) => author.user_id == Some(u),
            None => {
                author.link_id.is_some() && author.link_id == who.link_id && author.name == who.name
            }
        }
    }

    async fn extract(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let slug = RawPathParams::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|p| {
                p.iter()
                    .find(|(k, _)| *k == "slug")
                    .map(|(_, v)| v.to_string())
            });
        let query: HashMap<String, String> = Query::try_from_uri(&parts.uri)
            .map(|q| q.0)
            .unwrap_or_default();
        if let Some(slug) = slug {
            let user = authenticate(&parts.headers, state)
                .await?
                .ok_or_else(AppError::unauthorized)?;
            let project = projects::load(state, user.id, &slug, Role::Translator).await?;
            return Ok(Ctx {
                project,
                actor: Actor {
                    user_id: Some(user.id),
                    link_id: None,
                    name: user.name,
                },
                access: Access::Member,
                expires_at: None,
                widget: false,
            });
        }
        let invalid = || {
            err(
                StatusCode::UNAUTHORIZED,
                "invalid_review_token",
                "review token missing, invalid, expired or revoked",
            )
        };
        let token = bearer(&parts.headers)
            .or_else(|| query.get("token").cloned())
            .filter(|t| t.starts_with("glr_"))
            .ok_or_else(invalid)?;
        let row: Option<(Uuid, Uuid, String, String, Option<Uuid>, Option<DateTime<Utc>>, String, Vec<String>)> =
            sqlx::query_as(
                "UPDATE review_links l SET last_used_at = now() FROM projects p
                 WHERE l.token_hash = $1 AND p.id = l.project_id AND l.revoked_at IS NULL
                   AND (l.expires_at IS NULL OR l.expires_at > now())
                 RETURNING l.id, l.project_id, l.kind, l.access, l.user_id, l.expires_at, p.slug,
                   coalesce((SELECT origins FROM review_settings s WHERE s.project_id = l.project_id), '{}')",
            )
            .bind(sha256_hex(&token))
            .fetch_optional(&state.db)
            .await?;
        let (link_id, project_id, kind, access, user_id, expires_at, slug, origins) =
            row.ok_or_else(invalid)?;
        // Strict CORS is enforced by the layer; this binds the browser origin to *this* project's list.
        if let Some(origin) = parts
            .headers
            .get(header::ORIGIN)
            .and_then(|o| o.to_str().ok())
        {
            let origin = origin.trim_end_matches('/');
            if origin != super::web_origin() && !origins.iter().any(|o| o == origin) {
                return Err(err(
                    StatusCode::FORBIDDEN,
                    "origin_not_allowed",
                    "this origin is not declared in the project's review settings",
                ));
            }
        }
        let (project, actor, access) = if kind == "member" {
            let uid = user_id.ok_or_else(invalid)?;
            // No longer a member => the session token dies with the membership.
            let project = projects::load(state, uid, &slug, Role::Translator)
                .await
                .map_err(|_| invalid())?;
            let name: String = sqlx::query_scalar("SELECT name FROM users WHERE id = $1")
                .bind(uid)
                .fetch_one(&state.db)
                .await?;
            let actor = Actor {
                user_id: Some(uid),
                link_id: None,
                name,
            };
            (project, actor, Access::Member)
        } else {
            let project = projects::load_public(state, &slug)
                .await?
                .ok_or_else(invalid)?;
            let actor = Actor {
                user_id: None,
                link_id: Some(link_id),
                name: query
                    .get("author_name")
                    .map(|n| clean_line(n, 80))
                    .unwrap_or_default(),
            };
            let access = match access.as_str() {
                "edit" => Access::Edit,
                "comment" => Access::Comment,
                _ => Access::Read,
            };
            (project, actor, access)
        };
        if parts.method != Method::GET && parts.method != Method::HEAD {
            rate_limit(state, project_id).await?;
        }
        Ok(Ctx {
            project,
            actor,
            access,
            expires_at,
            widget: true,
        })
    }
}

impl FromRequestParts<AppState> for Ctx {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        match Self::extract(parts, state).await {
            Ok(c) => Ok(c),
            Err(e) if e.code == "rate_limited" => {
                let retry = e.message.rsplit(' ').next().unwrap_or("60").to_string();
                Err(([(header::RETRY_AFTER, retry)], e).into_response())
            }
            Err(e) => Err(e.into_response()),
        }
    }
}

fn bearer(h: &HeaderMap) -> Option<String> {
    h.get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_string())
}

static RATE: std::sync::LazyLock<i32> = std::sync::LazyLock::new(|| {
    std::env::var("GLOT_REVIEW_RATE_PER_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(120)
});

/// Widget writes per project and minute, shared across replicas (same scheme as feedback_rate).
async fn rate_limit(state: &AppState, project: Uuid) -> ApiResult<()> {
    let (hits, end): (i32, i64) = sqlx::query_as(
        "INSERT INTO review_rate AS r (project_id, window_start, hits) VALUES ($1, date_trunc('minute', now()), 1)
         ON CONFLICT (project_id) DO UPDATE SET
           hits = CASE WHEN excluded.window_start > r.window_start THEN 1 ELSE r.hits + 1 END,
           window_start = greatest(r.window_start, excluded.window_start)
         RETURNING r.hits, extract(epoch FROM r.window_start + interval '1 minute')::int8",
    )
    .bind(project)
    .fetch_one(&state.db)
    .await?;
    if hits > *RATE {
        let retry = (end - Utc::now().timestamp()).max(1);
        // The message's last word is the Retry-After value (read back by the extractor).
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            &format!("too many review writes for this project; retry after {retry}"),
        ));
    }
    Ok(())
}

// ---------- content ----------

/// Strips control characters (keeps newlines), trims, checks 1..=max chars.
pub fn clean(s: &str, max: usize, what: &str) -> ApiResult<String> {
    let s: String = s
        .replace("\r\n", "\n")
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() || s.chars().count() > max {
        return Err(AppError::bad_request(format!(
            "{what} must be 1-{max} chars"
        )));
    }
    Ok(s)
}

/// One line, no control characters, truncated.
pub fn clean_line(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// `**x**` / `*x*` on already-escaped text; an unmatched delimiter stays literal.
fn wrap(s: &str, delim: &str, tag: &str) -> String {
    let parts: Vec<&str> = s.split(delim).collect();
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i % 2 == 0 {
            out.push_str(p);
        } else if i + 1 == parts.len() {
            out.push_str(&format!("{delim}{p}"));
        } else if !p.is_empty() && !p.starts_with(' ') && !p.ends_with(' ') {
            out.push_str(&format!("<{tag}>{p}</{tag}>"));
        } else {
            out.push_str(&format!("{delim}{p}{delim}"));
        }
    }
    out
}

/// `[text](https://…)` on escaped text: http(s) only, no quotes/spaces/tags in the URL.
fn links(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let found = after.find("](").and_then(|close| {
            let url_start = close + 2;
            let end = after[url_start..].find(')')? + url_start;
            Some((close, url_start, end))
        });
        match found {
            Some((close, us, end)) => {
                let (text, url) = (&after[..close], &after[us..end]);
                let ok = (url.starts_with("https://") || url.starts_with("http://"))
                    && !url.contains(['<', '>', ' ', '"', '\''])
                    && !url.contains("&quot;")
                    && !url.contains("&#39;")
                    && !text.is_empty();
                out.push_str(&rest[..open]);
                if ok {
                    out.push_str(&format!(
                        "<a href=\"{url}\" rel=\"noopener noreferrer nofollow\" target=\"_blank\">{text}</a>"
                    ));
                } else {
                    out.push_str(&rest[open..open + 1 + end + 1]);
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[..open + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Tiny safe markdown → HTML: everything is escaped first, then code, bold, italic, http(s) links, line breaks.
pub fn render(md: &str) -> String {
    md.split('\n')
        .map(|line| {
            let parts: Vec<&str> = line.split('`').collect();
            let mut out = String::new();
            for (i, p) in parts.iter().enumerate() {
                if i % 2 == 1 && i + 1 < parts.len() {
                    out.push_str(&format!("<code>{}</code>", esc(p)));
                } else {
                    let p = if i % 2 == 1 {
                        format!("`{p}")
                    } else {
                        p.to_string()
                    };
                    out.push_str(&links(&wrap(&wrap(&esc(&p), "**", "strong"), "*", "em")));
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

// ---------- anchor ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Viewport {
    pub w: f64,
    pub h: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpr: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Where an annotation points (contract §1). Unknown fields are dropped on the way in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub key_source: Option<String>,
    pub selector: String,
    pub text: String,
    pub url: String,
    #[serde(default)]
    pub route: Option<String>,
    pub locale: String,
    pub viewport: Viewport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom: Option<String>,
}

/// `https://x/fr/pricing/?a#b` + `fr` → (`https://x/fr/pricing`, `/pricing`).
pub fn normalize_url(url: &str, locale: &str) -> (String, String) {
    let url: String = url
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .chars()
        .take(500)
        .collect();
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map(|i| &rest[i..]).unwrap_or("/"),
        None => url.as_str(),
    };
    let mut segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.first().is_some_and(|s| s.eq_ignore_ascii_case(locale)) {
        segs.remove(0);
    }
    (url.clone(), format!("/{}", segs.join("/")))
}

impl Anchor {
    /// Validates sizes and fills `url` (cleaned) and `route` (computed).
    pub fn normalize(mut self) -> ApiResult<Self> {
        crate::validate_locale(&self.locale)?;
        let bad = |m: &str| Err(AppError::bad_request(format!("anchor: {m}")));
        if self.selector.trim().is_empty() || self.selector.len() > 1000 {
            return bad("selector must be 1-1000 chars");
        }
        self.text = clean_line(&self.text, 1000);
        if self.url.trim().is_empty() {
            return bad("url is required");
        }
        if let Some(ks) = &self.key_source
            && !["attribute", "marker", "text", "manual"].contains(&ks.as_str())
        {
            return bad("key_source must be attribute|marker|text|manual");
        }
        if let Some(k) = &self.key
            && crate::validate_key(k).is_err()
        {
            self.key = None;
        }
        self.dom = self.dom.map(|d| clean_line(&d, 300));
        let (url, route) = normalize_url(&self.url, &self.locale);
        self.url = url;
        self.route = Some(route);
        Ok(self)
    }
}

/// Key of an anchor: the given key if it exists, else the rendered text matched against the locale's catalog
/// (the source catalog on source-locale pages).
pub async fn resolve_key(state: &AppState, p: &ProjectRow, a: &Anchor) -> ApiResult<Option<Uuid>> {
    if let Some(k) = &a.key {
        let id: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM keys WHERE project_id = $1 AND key = $2")
                .bind(p.id)
                .bind(k)
                .fetch_optional(&state.db)
                .await?;
        if id.is_some() {
            return Ok(id);
        }
    }
    if a.text.is_empty() {
        return Ok(None);
    }
    // ponytail: linear scan of the locale's catalog, like feedback ingestion; index it at ~100k keys.
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT k.id, k.key, CASE WHEN $3 THEN k.source ELSE t.value END FROM keys k
         LEFT JOIN translations t ON t.key_id = k.id AND t.locale = $2
         WHERE k.project_id = $1 AND (CASE WHEN $3 THEN k.source ELSE t.value END) <> ''",
    )
    .bind(p.id)
    .bind(&a.locale)
    .bind(a.locale == p.source_locale)
    .fetch_all(&state.db)
    .await?;
    let pairs: Vec<(String, String)> = rows.iter().map(|r| (r.1.clone(), r.2.clone())).collect();
    Ok(crate::feedback::match_key(&pairs, &a.text)
        .and_then(|k| rows.iter().find(|r| r.1 == k))
        .map(|r| r.0))
}

// ---------- views ----------

#[derive(Debug, Clone, Serialize)]
pub struct Reaction {
    pub emoji: String,
    pub count: i64,
    pub mine: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AttachmentMeta {
    pub id: Uuid,
    #[serde(skip)]
    pub annotation_id: Uuid,
    pub content_type: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: Uuid,
    pub annotation_id: Uuid,
    pub author: Actor,
    pub body: String,
    pub body_html: String,
    pub mentions: Vec<Uuid>,
    pub reactions: Vec<Reaction>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranslationRef {
    pub value: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Assignee {
    pub user_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Annotation {
    pub id: Uuid,
    pub status: String,
    pub anchor: Value,
    pub route: String,
    pub locale: String,
    pub url: String,
    pub key: Option<String>,
    pub source: Option<String>,
    pub translation: Option<TranslationRef>,
    pub body: String,
    pub body_html: String,
    pub suggestion: Option<String>,
    pub author: Actor,
    pub assignee: Option<Assignee>,
    pub feedback_group_id: Option<Uuid>,
    pub comment_count: i64,
    pub attachments: Vec<AttachmentMeta>,
    pub reactions: Vec<Reaction>,
    pub approvals: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<Comment>>,
}

#[derive(sqlx::FromRow)]
struct AnnRow {
    id: Uuid,
    status: String,
    anchor: DbJson<Value>,
    route: String,
    locale: String,
    url: String,
    key: Option<String>,
    source: Option<String>,
    t_value: Option<String>,
    t_status: Option<String>,
    body: String,
    suggestion: Option<String>,
    author_user: Option<Uuid>,
    author_link: Option<Uuid>,
    author_name: String,
    assignee: Option<Uuid>,
    assignee_name: Option<String>,
    feedback_group_id: Option<Uuid>,
    comment_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    total: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct Filter {
    pub route: Option<String>,
    pub locale: Option<String>,
    /// Comma-separated statuses.
    pub status: Option<String>,
    pub key: Option<String>,
    /// user id or exact author name
    pub author: Option<String>,
    /// user id or "me"
    pub assignee: Option<String>,
    pub q: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

async fn reactions_of(
    state: &AppState,
    ids: &[Uuid],
    me: &str,
) -> ApiResult<Vec<(Uuid, Option<Uuid>, Reaction)>> {
    let rows: Vec<(Uuid, Option<Uuid>, String, i64, bool)> = sqlx::query_as(
        "SELECT annotation_id, comment_id, emoji, count(*)::int8, bool_or(actor = $2) FROM review_reactions
         WHERE annotation_id = ANY($1) GROUP BY 1, 2, 3 ORDER BY 3",
    )
    .bind(ids)
    .bind(me)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(a, c, emoji, count, mine)| (a, c, Reaction { emoji, count, mine }))
        .collect())
}

/// Annotations of a project (`id` = one), with attachments and reactions; returns (items, total).
pub async fn query(
    state: &AppState,
    project: Uuid,
    id: Option<Uuid>,
    f: &Filter,
    me: &Actor,
) -> ApiResult<(Vec<Annotation>, i64)> {
    let nz = |o: &Option<String>| o.clone().filter(|s| !s.trim().is_empty());
    let statuses: Option<Vec<String>> = nz(&f.status).map(|s| {
        s.split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()
    });
    if let Some(ss) = &statuses
        && let Some(bad) = ss.iter().find(|s| !STATUSES.contains(&s.as_str()))
    {
        return Err(AppError::bad_request(format!("unknown status '{bad}'")));
    }
    let assignee = match nz(&f.assignee).as_deref() {
        Some("me") => Some(
            me.user_id
                .ok_or_else(|| AppError::bad_request("assignee=me needs a member"))?,
        ),
        Some(s) => Some(
            s.parse::<Uuid>()
                .map_err(|_| AppError::bad_request("assignee must be a user id or 'me'"))?,
        ),
        None => None,
    };
    let q = nz(&f.q).map(|s| {
        format!(
            "%{}%",
            s.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let per_page = f.per_page.unwrap_or(50).clamp(1, 200);
    let page = f.page.unwrap_or(1).max(1);
    let rows: Vec<AnnRow> = sqlx::query_as(
        "SELECT a.id, a.status, a.anchor, a.route, a.locale, a.url, k.key, k.source,
                t.value AS t_value, t.status AS t_status, a.body, a.suggestion, a.author_user, a.author_link,
                a.author_name, a.assignee, u.name AS assignee_name, a.feedback_group_id,
                (SELECT count(*) FROM review_comments c WHERE c.annotation_id = a.id) AS comment_count,
                a.created_at, a.updated_at, a.resolved_at, count(*) OVER () AS total
         FROM review_annotations a
         LEFT JOIN keys k ON k.id = a.key_id
         LEFT JOIN translations t ON t.key_id = a.key_id AND t.locale = a.locale
         LEFT JOIN users u ON u.id = a.assignee
         WHERE a.project_id = $1 AND ($2::uuid IS NULL OR a.id = $2)
           AND ($3::text IS NULL OR a.route = $3) AND ($4::text IS NULL OR a.locale = $4)
           AND ($5::text[] IS NULL OR a.status = ANY($5)) AND ($6::text IS NULL OR k.key = $6)
           AND ($7::text IS NULL OR a.author_user::text = $7 OR a.author_name = $7)
           AND ($8::uuid IS NULL OR a.assignee = $8)
           AND ($9::text IS NULL OR a.body ILIKE $9 OR a.anchor->>'text' ILIKE $9)
         ORDER BY a.updated_at DESC, a.id LIMIT $10 OFFSET $11",
    )
    .bind(project)
    .bind(id)
    .bind(nz(&f.route))
    .bind(nz(&f.locale))
    .bind(&statuses)
    .bind(nz(&f.key))
    .bind(nz(&f.author))
    .bind(assignee)
    .bind(q)
    .bind(per_page)
    .bind((page - 1) * per_page)
    .fetch_all(&state.db)
    .await?;
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let atts: Vec<AttachmentMeta> = sqlx::query_as(
        "SELECT id, annotation_id, content_type, octet_length(data)::int8 AS size, width, height, created_at
         FROM review_attachments WHERE annotation_id = ANY($1) ORDER BY created_at",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let reacts = reactions_of(state, &ids, &me.key()).await?;
    let items = rows
        .into_iter()
        .map(|r| {
            let reactions: Vec<Reaction> = reacts
                .iter()
                .filter(|(a, c, _)| *a == r.id && c.is_none())
                .map(|x| x.2.clone())
                .collect();
            Annotation {
                id: r.id,
                status: r.status,
                anchor: r.anchor.0,
                route: r.route,
                locale: r.locale,
                url: r.url,
                key: r.key,
                source: r.source,
                translation: r.t_value.map(|value| TranslationRef {
                    value,
                    status: r.t_status.unwrap_or_default(),
                }),
                body_html: render(&r.body),
                body: r.body,
                suggestion: r.suggestion,
                author: Actor {
                    user_id: r.author_user,
                    link_id: r.author_link,
                    name: r.author_name,
                },
                assignee: r.assignee.map(|user_id| Assignee {
                    user_id,
                    name: r.assignee_name.unwrap_or_default(),
                }),
                feedback_group_id: r.feedback_group_id,
                comment_count: r.comment_count,
                attachments: atts
                    .iter()
                    .filter(|a| a.annotation_id == r.id)
                    .cloned()
                    .collect(),
                approvals: reactions
                    .iter()
                    .find(|x| x.emoji == "approve")
                    .map(|x| x.count)
                    .unwrap_or(0),
                reactions,
                created_at: r.created_at,
                updated_at: r.updated_at,
                resolved_at: r.resolved_at,
                comments: None,
            }
        })
        .collect();
    Ok((items, total))
}

pub async fn comments(state: &AppState, annotation: Uuid, me: &Actor) -> ApiResult<Vec<Comment>> {
    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        String,
        String,
        Vec<Uuid>,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(
        "SELECT id, author_user, author_link, author_name, body, mentions, created_at, edited_at
             FROM review_comments WHERE annotation_id = $1 ORDER BY created_at, id",
    )
    .bind(annotation)
    .fetch_all(&state.db)
    .await?;
    let reacts = reactions_of(state, &[annotation], &me.key()).await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, u, l, name, body, mentions, created_at, edited_at)| Comment {
                id,
                annotation_id: annotation,
                author: Actor {
                    user_id: u,
                    link_id: l,
                    name,
                },
                body_html: render(&body),
                body,
                mentions,
                reactions: reacts
                    .iter()
                    .filter(|(_, c, _)| *c == Some(id))
                    .map(|x| x.2.clone())
                    .collect(),
                created_at,
                edited_at,
            },
        )
        .collect())
}

/// One annotation with its thread.
pub async fn one(state: &AppState, project: Uuid, id: Uuid, me: &Actor) -> ApiResult<Annotation> {
    let mut a = query(state, project, Some(id), &Filter::default(), me)
        .await?
        .0
        .pop()
        .ok_or_else(|| AppError::not_found("annotation not found"))?;
    a.comments = Some(comments(state, id, me).await?);
    Ok(a)
}

// ---------- history + realtime ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewEvent {
    pub id: i64,
    pub annotation_id: Option<Uuid>,
    pub comment_id: Option<Uuid>,
    pub kind: String,
    pub actor: Actor,
    pub data: Value,
    pub created_at: DateTime<Utc>,
}

pub const EVENT_COLS: &str =
    "id, annotation_id, comment_id, kind, actor_user, actor_link, actor_name, data, created_at";

#[derive(sqlx::FromRow)]
pub struct EventRow {
    id: i64,
    annotation_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    kind: String,
    actor_user: Option<Uuid>,
    actor_link: Option<Uuid>,
    actor_name: String,
    data: DbJson<Value>,
    created_at: DateTime<Utc>,
}

impl From<EventRow> for ReviewEvent {
    fn from(r: EventRow) -> Self {
        Self {
            id: r.id,
            annotation_id: r.annotation_id,
            comment_id: r.comment_id,
            kind: r.kind,
            actor: Actor {
                user_id: r.actor_user,
                link_id: r.actor_link,
                name: r.actor_name,
            },
            data: r.data.0,
            created_at: r.created_at,
        }
    }
}

/// Appends to the history and NOTIFYs it (delivered on commit). Call inside the writing transaction.
pub async fn emit(
    conn: &mut PgConnection,
    project: Uuid,
    annotation: Option<Uuid>,
    comment: Option<Uuid>,
    kind: &str,
    actor: &Actor,
    data: Value,
) -> ApiResult<()> {
    let row: EventRow = sqlx::query_as(&format!(
        "INSERT INTO review_events (project_id, annotation_id, comment_id, kind, actor_user, actor_link, actor_name, data)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {EVENT_COLS}"
    ))
    .bind(project)
    .bind(annotation)
    .bind(comment)
    .bind(kind)
    .bind(actor.user_id)
    .bind(actor.link_id)
    .bind(&actor.name)
    .bind(DbJson(&data))
    .fetch_one(&mut *conn)
    .await?;
    let mut ev = ReviewEvent::from(row);
    let mut payload = json!({ "project_id": project, "event": ev }).to_string();
    if payload.len() > 7900 {
        // NOTIFY payloads stop at 8000 bytes; clients refetch anyway.
        ev.data = json!({ "truncated": true });
        payload = json!({ "project_id": project, "event": ev }).to_string();
    }
    sqlx::query("SELECT pg_notify('glot_review', $1)")
        .bind(payload)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// For RV4 (AI rephrase): sets (or clears) an annotation's suggestion and records `suggestion_set`.
pub async fn set_suggestion(
    state: &AppState,
    project: Uuid,
    annotation: Uuid,
    value: Option<&str>,
    actor: &Actor,
) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    let n = sqlx::query(
        "UPDATE review_annotations SET suggestion = $3, updated_at = now() WHERE id = $1 AND project_id = $2",
    )
    .bind(annotation)
    .bind(project)
    .bind(value)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("annotation not found"));
    }
    emit(
        &mut tx,
        project,
        Some(annotation),
        None,
        "suggestion_set",
        actor,
        json!({ "suggestion": value }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub fn json_error(status: StatusCode, code: &str, message: &str, details: Value) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message, "details": details } })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_safe() {
        assert_eq!(
            render("<script>alert(1)</script> **b** *i* `<x>`\nl2"),
            "&lt;script&gt;alert(1)&lt;/script&gt; <strong>b</strong> <em>i</em> <code>&lt;x&gt;</code><br>l2"
        );
        assert_eq!(
            render("[ok](https://a.b/c) [js](javascript:alert(1)) [q](https://x\"onmouseover=1)"),
            "<a href=\"https://a.b/c\" rel=\"noopener noreferrer nofollow\" target=\"_blank\">ok</a> [js](javascript:alert(1)) [q](https://x&quot;onmouseover=1)"
        );
        assert_eq!(render("2 * 3 and **open"), "2 * 3 and **open");
    }

    #[test]
    fn urls_and_text() {
        assert_eq!(
            normalize_url("https://app.x/fr/pricing/?t=1#a", "fr"),
            ("https://app.x/fr/pricing/".into(), "/pricing".into())
        );
        assert_eq!(normalize_url("https://app.x/fr", "fr").1, "/");
        assert_eq!(normalize_url("https://app.x", "fr").1, "/");
        assert_eq!(normalize_url("/de/a/b", "fr").1, "/de/a/b");
        assert_eq!(clean(" a\u{0}b\r\nc\t ", 10, "x").unwrap(), "ab\nc");
        assert!(clean("  ", 10, "x").is_err());
        assert_eq!(clean_line("a\n  b\u{7}", 80), "a b");
        assert!(Access::Edit > Access::Comment && Access::Member > Access::Edit);
    }
}
