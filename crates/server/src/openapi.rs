use axum::Json;
use utoipa::{
    OpenApi,
    openapi::{HttpMethod, path::OperationBuilder},
};

use crate::{auth, glossary, jobs, keys, projects, sync};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "glot API",
        description = "AI-first SaaS localization. Errors: {\"error\":{\"code\",\"message\"}}. Auth: glot_session cookie or Bearer token."
    ),
    components(schemas(
        auth::User,
        auth::ApiToken,
        auth::CreatedToken,
        projects::Project,
        projects::LlmView,
        projects::Stats,
        projects::LocaleStats,
        keys::KeyRow,
        keys::KeyPage,
        keys::Translation,
        keys::HistoryEntry,
        keys::IssueRow,
        glossary::TermRow,
        jobs::Job,
        jobs::Progress,
        sync::PushResult,
        sync::Conflict,
        sync::PullResult,
    ))
)]
struct Doc;

// ponytail: routes listed by hand (method, path, summary) instead of #[utoipa::path] on every handler.
const ROUTES: &[(&str, &str, &str)] = &[
    (
        "POST",
        "/api/auth/signup",
        "Sign up {email,password,name} -> User (+cookie)",
    ),
    (
        "POST",
        "/api/auth/login",
        "Log in {email,password} -> User (+cookie)",
    ),
    ("POST", "/api/auth/logout", "Log out"),
    ("GET", "/api/me", "Current user -> User"),
    ("GET", "/api/tokens", "List API tokens -> [ApiToken]"),
    (
        "POST",
        "/api/tokens",
        "Create API token {name} -> CreatedToken",
    ),
    ("DELETE", "/api/tokens/{id}", "Revoke API token"),
    ("GET", "/api/projects", "List projects -> [Project]"),
    (
        "POST",
        "/api/projects",
        "Create project {name,slug,source_locale,target_locales} -> Project",
    ),
    ("GET", "/api/projects/{slug}", "Get project -> Project"),
    (
        "PATCH",
        "/api/projects/{slug}",
        "Update project {name?,source_locale?,target_locales?,llm?,style?,public_cdn?} -> Project; a new source_locale re-roots keys on its translations (old source becomes a reviewed target, dependent translations go stale)",
    ),
    ("DELETE", "/api/projects/{slug}", "Delete project"),
    (
        "GET",
        "/api/projects/{slug}/keys",
        "List keys ?q=&locale=&status=&page=&per_page= -> KeyPage",
    ),
    (
        "POST",
        "/api/projects/{slug}/keys",
        "Create key {key,source,description?} -> KeyRow",
    ),
    (
        "PATCH",
        "/api/projects/{slug}/keys/{key}",
        "Update key {source?,description?} -> KeyRow",
    ),
    ("DELETE", "/api/projects/{slug}/keys/{key}", "Delete key"),
    (
        "PUT",
        "/api/projects/{slug}/keys/{key}/translations/{locale}",
        "Write translation {value} -> Translation",
    ),
    (
        "POST",
        "/api/projects/{slug}/keys/{key}/translations/{locale}/review",
        "Mark reviewed -> Translation",
    ),
    (
        "GET",
        "/api/projects/{slug}/keys/{key}/history",
        "Translation history -> [HistoryEntry]",
    ),
    (
        "GET",
        "/api/projects/{slug}/issues",
        "Open issues -> [IssueRow]",
    ),
    (
        "GET",
        "/api/projects/{slug}/glossary",
        "Glossary -> [TermRow]",
    ),
    (
        "POST",
        "/api/projects/{slug}/glossary",
        "Create term -> TermRow",
    ),
    (
        "PATCH",
        "/api/projects/{slug}/glossary/{id}",
        "Update term -> TermRow",
    ),
    (
        "DELETE",
        "/api/projects/{slug}/glossary/{id}",
        "Delete term",
    ),
    (
        "POST",
        "/api/projects/{slug}/jobs",
        "Start job {kind:\"translate\",locales?,keys?,force?} -> Job",
    ),
    ("GET", "/api/projects/{slug}/jobs", "List jobs -> [Job]"),
    ("GET", "/api/jobs/{id}", "Get job -> Job"),
    (
        "GET",
        "/api/jobs/{id}/events",
        "SSE: event progress (data=Job), event done",
    ),
    (
        "POST",
        "/api/projects/{slug}/sync/push",
        "Push {catalogs,lock,glossary} -> PushResult",
    ),
    (
        "GET",
        "/api/projects/{slug}/sync/pull",
        "Pull -> PullResult",
    ),
    (
        "GET",
        "/cdn/{slug}/{locale}.json",
        "OTA nested JSON catalog (ETag, Cache-Control)",
    ),
    ("GET", "/healthz", "Health check"),
];

pub async fn spec() -> Json<utoipa::openapi::OpenApi> {
    let mut doc = Doc::openapi();
    for (method, path, summary) in ROUTES {
        let method = match *method {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "PATCH" => HttpMethod::Patch,
            _ => HttpMethod::Delete,
        };
        doc.paths.add_path_operation(
            *path,
            vec![method],
            OperationBuilder::new().summary(Some(*summary)).build(),
        );
    }
    Json(doc)
}
