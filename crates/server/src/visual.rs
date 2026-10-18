//! Visual QA reports (uploaded by `glot-visual-qa --upload`) and locale-matrix preview settings.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

/// Reports kept per project; older ones (and their screenshots) are dropped on upload.
const KEEP: i64 = 20;
const MAX_UPLOAD: usize = 128 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}";
    Router::new()
        .route(
            &format!("{p}/visual-reports"),
            get(list)
                .post(upload)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD)),
        )
        .route(&format!("{p}/visual-reports/latest"), get(latest))
        .route(&format!("{p}/visual-reports/{{id}}"), get(one))
        .route(
            &format!("{p}/visual-reports/{{id}}/files/{{*path}}"),
            get(file),
        )
        .route(&format!("{p}/preview"), get(get_preview).put(put_preview))
}

#[derive(Serialize, Deserialize, Default, utoipa::ToSchema)]
pub struct Summary {
    pub pages: usize,
    pub issues: usize,
    pub locales: Vec<String>,
    pub routes: Vec<String>,
    pub viewports: Vec<String>,
    pub by_kind: BTreeMap<String, usize>,
    pub by_locale: BTreeMap<String, usize>,
    pub pages_by_locale: BTreeMap<String, usize>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct VisualReportMeta {
    pub id: Uuid,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub summary: Summary,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct VisualReport {
    #[serde(flatten)]
    pub meta: VisualReportMeta,
    /// The visual-qa `report.json`, as uploaded. Screenshot paths resolve under `.../files/{path}`.
    #[schema(value_type = Object)]
    pub report: Value,
}

fn summarize(report: &Value) -> Summary {
    let mut s = Summary::default();
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let push = |v: &mut Vec<String>, x: String| {
        if !x.is_empty() && !v.contains(&x) {
            v.push(x)
        }
    };
    for page in report["pages"].as_array().into_iter().flatten() {
        s.pages += 1;
        push(&mut s.locales, str_of(page, "locale"));
        push(&mut s.routes, str_of(page, "route"));
        push(&mut s.viewports, str_of(page, "viewport"));
        *s.pages_by_locale.entry(str_of(page, "locale")).or_default() += 1;
    }
    for issue in report["issues"].as_array().into_iter().flatten() {
        s.issues += 1;
        *s.by_kind.entry(str_of(issue, "kind")).or_default() += 1;
        *s.by_locale.entry(str_of(issue, "locale")).or_default() += 1;
    }
    s
}

/// Standard base64 (padding optional). ponytail: hand-rolled, the server has no base64 dependency.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= (val(c)? as u32) << (18 - 6 * i);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..chunk.len()]);
    }
    Some(out)
}

fn content_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    })
}

fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 255
        && !p.starts_with('/')
        && !p.split('/').any(|s| s.is_empty() || s == "." || s == "..")
        && content_type(p).is_some()
}

#[derive(Deserialize)]
pub struct UploadBody {
    pub report: Value,
    /// `{ "<path as referenced in report.json>": "<base64>" }`
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    pub label: Option<String>,
}

pub async fn upload(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<UploadBody>,
) -> ApiResult<(StatusCode, Json<VisualReportMeta>)> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    if !b.report["pages"].is_array() || !b.report["issues"].is_array() {
        return Err(AppError::bad_request(
            "report must be a visual-qa report.json (pages[], issues[])",
        ));
    }
    let mut files = Vec::with_capacity(b.files.len());
    for (path, data) in &b.files {
        if !valid_path(path) {
            return Err(AppError::bad_request(format!(
                "invalid file path '{path}' (relative .png/.jpg/.webp expected)"
            )));
        }
        let bytes = b64_decode(data)
            .ok_or_else(|| AppError::bad_request(format!("file '{path}' is not valid base64")))?;
        files.push((path.clone(), bytes));
    }
    let summary = summarize(&b.report);
    let id = Uuid::new_v4();
    let label = b.label.filter(|l| !l.trim().is_empty());

    let mut tx = state.db.begin().await?;
    let created_at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO visual_reports (id, project_id, label, report, summary) VALUES ($1, $2, $3, $4, $5) RETURNING created_at",
    )
    .bind(id)
    .bind(p.id)
    .bind(&label)
    .bind(&b.report)
    .bind(DbJson(&summary))
    .fetch_one(&mut *tx)
    .await?;
    for (path, bytes) in files {
        sqlx::query("INSERT INTO visual_files (report_id, path, data) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(path)
            .bind(bytes)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "DELETE FROM visual_reports WHERE project_id = $1 AND id NOT IN
           (SELECT id FROM visual_reports WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2)",
    )
    .bind(p.id)
    .bind(KEEP)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(VisualReportMeta {
            id,
            label,
            created_at,
            summary,
        }),
    ))
}

type MetaRow = (Uuid, Option<String>, DateTime<Utc>, DbJson<Summary>);
fn meta((id, label, created_at, summary): MetaRow) -> VisualReportMeta {
    VisualReportMeta {
        id,
        label,
        created_at,
        summary: summary.0,
    }
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<VisualReportMeta>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<MetaRow> = sqlx::query_as(
        "SELECT id, label, created_at, summary FROM visual_reports WHERE project_id = $1 ORDER BY created_at DESC",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(meta).collect()))
}

async fn fetch(state: &AppState, project: Uuid, id: Option<Uuid>) -> ApiResult<VisualReport> {
    let row: Option<(Uuid, Option<String>, DateTime<Utc>, DbJson<Summary>, Value)> =
        sqlx::query_as(
            "SELECT id, label, created_at, summary, report FROM visual_reports
             WHERE project_id = $1 AND ($2::uuid IS NULL OR id = $2) ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project)
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let (id, label, created_at, summary, report) =
        row.ok_or_else(|| AppError::not_found("visual report not found"))?;
    Ok(VisualReport {
        meta: meta((id, label, created_at, summary)),
        report,
    })
}

pub async fn latest(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<VisualReport>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(fetch(&state, p.id, None).await?))
}

pub async fn one(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<VisualReport>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(fetch(&state, p.id, Some(id)).await?))
}

pub async fn file(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id, path)): AppPath<(String, Uuid, String)>,
) -> ApiResult<Response> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let data: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT f.data FROM visual_files f JOIN visual_reports r ON r.id = f.report_id
         WHERE r.project_id = $1 AND f.report_id = $2 AND f.path = $3",
    )
    .bind(p.id)
    .bind(id)
    .bind(&path)
    .fetch_optional(&state.db)
    .await?;
    let data = data.ok_or_else(|| AppError::not_found("file not found"))?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                content_type(&path).unwrap_or("application/octet-stream"),
            ),
            // A report's files never change once uploaded.
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        data,
    )
        .into_response())
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct PreviewSettings {
    /// Base URL of the user's running app, e.g. `http://localhost:3000`.
    pub base_url: Option<String>,
    /// `{locale}` and `{route}` placeholders; the source locale drops its `/{locale}` segment.
    pub url_pattern: String,
    pub routes: Vec<String>,
}

pub async fn get_preview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<PreviewSettings>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let row: Option<(Option<String>, String, Vec<String>)> = sqlx::query_as(
        "SELECT base_url, url_pattern, routes FROM preview_settings WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    let (base_url, url_pattern, routes) =
        row.unwrap_or((None, "/{locale}{route}".into(), vec!["/".into()]));
    Ok(Json(PreviewSettings {
        base_url,
        url_pattern,
        routes,
    }))
}

pub async fn put_preview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PreviewSettings>,
) -> ApiResult<Json<PreviewSettings>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let base_url = b
        .base_url
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    if let Some(u) = &base_url
        && !(u.starts_with("http://") || u.starts_with("https://"))
    {
        return Err(AppError::bad_request(
            "base_url must start with http:// or https://",
        ));
    }
    if !b.url_pattern.contains("{route}") {
        return Err(AppError::bad_request("url_pattern must contain {route}"));
    }
    let routes: Vec<String> = b
        .routes
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .map(|r| {
            if r.starts_with('/') {
                r.to_string()
            } else {
                format!("/{r}")
            }
        })
        .collect();
    sqlx::query(
        "INSERT INTO preview_settings (project_id, base_url, url_pattern, routes) VALUES ($1, $2, $3, $4)
         ON CONFLICT (project_id) DO UPDATE SET base_url = $2, url_pattern = $3, routes = $4",
    )
    .bind(p.id)
    .bind(&base_url)
    .bind(&b.url_pattern)
    .bind(&routes)
    .execute(&state.db)
    .await?;
    Ok(Json(PreviewSettings {
        base_url,
        url_pattern: b.url_pattern,
        routes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        assert_eq!(b64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(b64_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(b64_decode("aGVsbG8h").unwrap(), b"hello!");
        assert_eq!(b64_decode("aGk=\n").unwrap(), b"hi");
        assert_eq!(b64_decode("").unwrap(), b"");
        assert!(b64_decode("a").is_none());
        assert!(b64_decode("a*==").is_none());
    }

    #[test]
    fn paths_and_summary() {
        assert!(valid_path("screenshots/fr_index_1280x800.png"));
        assert!(!valid_path("../etc/passwd.png"));
        assert!(!valid_path("/abs.png"));
        assert!(!valid_path("report.json"));
        let s = summarize(&serde_json::json!({
            "pages": [{"locale":"fr","route":"/","viewport":"1280x800"},{"locale":"de","route":"/","viewport":"1280x800"}],
            "issues": [{"kind":"overflow","locale":"fr"},{"kind":"overflow","locale":"fr"},{"kind":"tofu","locale":"de"}]
        }));
        assert_eq!((s.pages, s.issues), (2, 3));
        assert_eq!(s.by_kind["overflow"], 2);
        assert_eq!(s.by_locale["fr"], 2);
        assert_eq!(s.locales, ["fr", "de"]);
    }
}
