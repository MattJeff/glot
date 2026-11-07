//! Bounded discovery: where glot Edge may learn new segments, and which pages need a review before publishing.
//! Discovery only on allowed public paths (`discover_include` / `discover_exclude` globs), never behind
//! authentication (the reporter says when the visitor has a session), never personal data, capped per day.
//! Segments first seen on a sensitive page (payment, account) are translated as `pending`: served only once reviewed.
use axum::{Json, Router, extract::State, routing::get};
use glot_core::edge::protect;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Settings {
    /// Kill switch: `false` = Edge passes the origin through untouched.
    pub enabled: bool,
    pub auto_translate: bool,
    /// Path globs (`/blog/*`) where discovery is allowed; empty = every public path.
    pub discover_include: Vec<String>,
    pub discover_exclude: Vec<String>,
    /// CSS selectors limiting / excluding extraction (applied by the worker, proxy and patcher).
    pub selectors_include: Vec<String>,
    pub selectors_exclude: Vec<String>,
    /// Critical pages (coverage gate of `locales.json`).
    pub critical_patterns: Vec<String>,
    /// Pages whose new segments wait for a review (`pending`) before being served.
    pub sensitive_patterns: Vec<String>,
    pub critical_threshold: f32,
    /// New segments accepted per rolling 24 h.
    pub daily_cap: i32,
    /// Monthly cap on auto-translation spend; `null` = only the project's `llm.budget_usd` per job.
    pub monthly_budget_usd: Option<f64>,
    /// origin | prerender | auto
    pub rendering: String,
    /// Application paths kept on the origin's domain, one rule per element (`/login`, `proxy /api/`; see
    /// `app_paths`). Missing in a request body = the suggested list.
    #[serde(default = "suggested_app_paths")]
    pub app_paths: Vec<String>,
    /// Action of rules without one: redirect | proxy
    #[serde(default = "redirect")]
    pub app_action: String,
}

fn suggested_app_paths() -> Vec<String> {
    super::app_paths::SUGGESTED
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn redirect() -> String {
    "redirect".into()
}

const COLS: &str = "enabled, auto_translate, discover_include, discover_exclude, selectors_include, selectors_exclude,
    critical_patterns, sensitive_patterns, critical_threshold, daily_cap, monthly_budget_usd, rendering, app_paths, app_action";

impl Settings {
    /// The project's settings; the column defaults when it never saved any.
    pub async fn load<'e>(db: impl sqlx::PgExecutor<'e>, project: Uuid) -> sqlx::Result<Self> {
        sqlx::query_as(&format!(
            "SELECT {COLS} FROM edge_settings WHERE project_id = $1"
        ))
        .bind(project)
        .fetch_optional(db)
        .await
        .map(|s| s.unwrap_or_default())
    }

    /// Discovery allowed on `path` (already query-free).
    pub fn discoverable(&self, path: &str) -> bool {
        !matches_any(&self.discover_exclude, path)
            && (self.discover_include.is_empty() || matches_any(&self.discover_include, path))
    }

    pub fn sensitive(&self, path: &str) -> bool {
        matches_any(&self.sensitive_patterns, path)
    }

    pub fn critical(&self, path: &str) -> bool {
        matches_any(&self.critical_patterns, path)
    }
}

impl Default for Settings {
    /// Same values as the `edge_settings` column defaults (migration 0151).
    fn default() -> Self {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        Settings {
            enabled: true,
            auto_translate: true,
            discover_include: vec![],
            discover_exclude: vec![],
            selectors_include: vec![],
            selectors_exclude: vec![],
            critical_patterns: v(&[
                "/",
                "/pricing*",
                "/signup*",
                "/register*",
                "/checkout*",
                "/pay*",
            ]),
            sensitive_patterns: v(&[
                "/checkout*",
                "/pay*",
                "/payment*",
                "/billing*",
                "/account*",
                "/settings*",
            ]),
            critical_threshold: 0.98,
            daily_cap: 2000,
            monthly_budget_usd: None,
            rendering: "auto".into(),
            app_paths: suggested_app_paths(),
            app_action: redirect(),
        }
    }
}

/// Glob match (`*` any run, `?` one char); a bad pattern matches nothing (patterns are checked on save).
pub fn matches_any(patterns: &[String], path: &str) -> bool {
    patterns
        .iter()
        .any(|p| glob::Pattern::new(p).is_ok_and(|g| g.matches(path)))
}

/// Personal or secret data left in a reported text (emails, phones, cards, IBAN, UUIDs, tokens, long numbers,
/// street addresses). The segmenter masks them client-side and never reports such a segment; this is the
/// server's own check at the trust boundary (same detector, `glot_core::edge::protect`).
pub fn looks_personal(text: &str) -> bool {
    let mut values = vec![];
    protect::mask(text, &mut values, &[]);
    values.iter().any(|v| v.kind.is_pii())
        || protect::looks_like_address(text)
        // Digit groups split by spaces/dashes (`FR76 3000 6000 0112…`, `+33 6 12 34 56 78`) the word detector misses.
        || text
            .split(|c: char| !(c.is_ascii_digit() || matches!(c, ' ' | '-' | '+' | '.' | '\u{a0}')))
            .any(|run| run.chars().filter(char::is_ascii_digit).count() >= 9)
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/projects/{slug}/edge/settings",
        get(get_settings).put(put_settings),
    )
}

async fn get_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(Settings::load(&state.db, p.id).await?))
}

async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(s): AppJson<Settings>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    for g in s
        .discover_include
        .iter()
        .chain(&s.discover_exclude)
        .chain(&s.critical_patterns)
        .chain(&s.sensitive_patterns)
    {
        if !g.starts_with('/') || g.len() > 200 || glob::Pattern::new(g).is_err() {
            return Err(AppError::bad_request(format!(
                "invalid path pattern '{g}' (start with '/', `*` wildcards)"
            )));
        }
    }
    if s.selectors_include
        .iter()
        .chain(&s.selectors_exclude)
        .any(|c| c.trim().is_empty() || c.len() > 200)
    {
        return Err(AppError::bad_request("selectors must be 1-200 chars"));
    }
    if !(0.0..=1.0).contains(&s.critical_threshold) {
        return Err(AppError::bad_request("critical_threshold must be 0-1"));
    }
    if !(0..=100_000).contains(&s.daily_cap) {
        return Err(AppError::bad_request("daily_cap must be 0-100000"));
    }
    if s.monthly_budget_usd
        .is_some_and(|v| !v.is_finite() || v < 0.0)
    {
        return Err(AppError::bad_request("monthly_budget_usd must be >= 0"));
    }
    if !["origin", "prerender", "auto"].contains(&s.rendering.as_str()) {
        return Err(AppError::bad_request(
            "rendering must be origin|prerender|auto",
        ));
    }
    let default = match s.app_action.as_str() {
        "redirect" => super::app_paths::Action::Redirect,
        "proxy" => super::app_paths::Action::Proxy,
        _ => return Err(AppError::bad_request("app_action must be redirect|proxy")),
    };
    if s.app_paths.len() > 200 {
        return Err(AppError::bad_request("at most 200 app path rules"));
    }
    for l in &s.app_paths {
        super::app_paths::parse_line(l, default).map_err(AppError::bad_request)?;
    }
    let app_paths: Vec<String> = s
        .app_paths
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let s = Settings { app_paths, ..s };
    sqlx::query(
        "INSERT INTO edge_settings (project_id, enabled, auto_translate, discover_include, discover_exclude,
            selectors_include, selectors_exclude, critical_patterns, sensitive_patterns, critical_threshold,
            daily_cap, monthly_budget_usd, rendering, app_paths, app_action, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, now())
         ON CONFLICT (project_id) DO UPDATE SET enabled = $2, auto_translate = $3, discover_include = $4,
            discover_exclude = $5, selectors_include = $6, selectors_exclude = $7, critical_patterns = $8,
            sensitive_patterns = $9, critical_threshold = $10, daily_cap = $11, monthly_budget_usd = $12,
            rendering = $13, app_paths = $14, app_action = $15, updated_at = now()",
    )
    .bind(p.id)
    .bind(s.enabled)
    .bind(s.auto_translate)
    .bind(&s.discover_include)
    .bind(&s.discover_exclude)
    .bind(&s.selectors_include)
    .bind(&s.selectors_exclude)
    .bind(&s.critical_patterns)
    .bind(&s.sensitive_patterns)
    .bind(s.critical_threshold)
    .bind(s.daily_cap)
    .bind(s.monthly_budget_usd)
    .bind(&s.rendering)
    .bind(&s.app_paths)
    .bind(&s.app_action)
    .execute(&state.db)
    .await?;
    Ok(Json(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_and_personal_data() {
        let s = Settings {
            discover_exclude: vec!["/admin*".into()],
            ..Default::default()
        };
        assert!(s.discoverable("/pricing"));
        assert!(!s.discoverable("/admin/users"));
        assert!(s.sensitive("/checkout/step-2") && !s.sensitive("/pricing"));
        assert!(s.critical("/") && s.critical("/pricing/annual") && !s.critical("/blog"));
        let only_blog = Settings {
            discover_include: vec!["/blog/*".into()],
            ..Default::default()
        };
        assert!(only_blog.discoverable("/blog/a") && !only_blog.discoverable("/"));

        for pii in [
            "Contact jane.doe@example.com",
            "Call +33 6 12 34 56 78",
            "Card 4111 1111 1111 1111",
            "IBAN FR76 3000 6000 0112 3456 7890 189",
            "Order 123456789012",
            "id 3f2a9c1e-77aa-4b1e-9d2b-0c1d2e3f4a5b",
            // Split so secret scanners don't flag this fake key.
            concat!("token sk_", "live_", "4eC39HqLyjWDarjtT1zdp7dc"),
        ] {
            assert!(looks_personal(pii), "{pii}");
        }
        for copy in [
            "Save 20% today",
            "Plans from $9/month, billed yearly",
            "Join 10,000 creators",
            "Made in 2026, updated 2026-11-07",
            "Write to <0>us</0>",
        ] {
            assert!(!looks_personal(copy), "{copy}");
        }
    }
}
