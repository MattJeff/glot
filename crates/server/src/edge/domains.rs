//! glot Edge domains (dashboard contract `EdgeDomain` in web/components/edge/api.ts), prefix
//! `/api/projects/{slug}/edge/domains`: list (translator), add / enable-disable / delete / verify (admin).
//!
//! - Verification = DNS: TXT `_glot.{host}` = the domain's `verify_token`, plus for subdomain mode a CNAME to the
//!   edge host (`GLOT_EDGE_CNAME`, default `edge.glotlang.com`). Resolved over DNS-over-HTTPS (JSON API,
//!   `GLOT_DOH_URL`, default Cloudflare then Google, 4 s each): no resolver crate in the tree, and the box's
//!   resolver may cache negative answers. A failed re-check never un-verifies a live domain (a DNS blip must not
//!   take a site down); deleting or disabling the domain does.
//! - Caddy fallback (subdomain mode with the glot server process down): every change rewrites the map file
//!   `GLOT_CADDY_MAP` (`map {host} {glot_origin} {glot_origin_host} {glot_origin_scheme}`, verified subdomain-mode
//!   domains only)
//!   imported by the edge site block of deploy/Caddyfile, then reloads Caddy through its admin API
//!   (`GLOT_CADDY_ADMIN` + the Caddyfile at `GLOT_CADDYFILE`, POST /load). Unset = nothing written.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

use super::ask::clean_host;
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, Role},
    review_live::probe::is_public,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/projects/{slug}/edge/domains", get(list).post(create))
        .route(
            "/api/projects/{slug}/edge/domains/{id}",
            axum::routing::patch(update).delete(remove),
        )
        .route(
            "/api/projects/{slug}/edge/domains/{id}/verify",
            post(verify),
        )
}

fn cname_target() -> String {
    std::env::var("GLOT_EDGE_CNAME").unwrap_or_else(|_| "edge.glotlang.com".into())
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EdgeDomain {
    id: Uuid,
    host: String,
    mode: String,
    locale: String,
    origin_url: String,
    enabled: bool,
    verified_at: Option<DateTime<Utc>>,
    verify_token: String,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    cname_target: Option<String>,
    checks: Option<Value>,
    checked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

const COLS: &str = "id, host, mode, locale, origin_url, enabled, verified_at, verify_token, checks, checked_at, created_at";

fn out(mut d: EdgeDomain) -> EdgeDomain {
    if d.mode == "subdomain" {
        d.cname_target = Some(cname_target());
    }
    d
}

async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Vec<EdgeDomain>>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<EdgeDomain> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM edge_domains WHERE project_id = $1 ORDER BY created_at, host"
    ))
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(out).collect()))
}

#[derive(Deserialize)]
struct NewDomain {
    host: String,
    mode: String,
    locale: String,
    origin_url: String,
}

/// `scheme://host[:port]` of a public http(s) origin (private addresses only with `GLOT_EDGE_ALLOW_PRIVATE=1`).
fn clean_origin(raw: &str) -> Option<Url> {
    let u = Url::parse(raw.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some()
    {
        return None;
    }
    let host = u.host_str()?.trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<std::net::IpAddr>()
        && !is_public(ip)
        && !super::sitemap::allow_private()
    {
        return None;
    }
    if host == "localhost" && !super::sitemap::allow_private() {
        return None;
    }
    Url::parse(&u.origin().ascii_serialization()).ok()
}

async fn create(
    State(state): State<AppState>,
    crate::auth::VerifiedUser(user): crate::auth::VerifiedUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<NewDomain>,
) -> ApiResult<(StatusCode, Json<EdgeDomain>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let host = clean_host(&b.host).ok_or_else(|| AppError::bad_request("invalid host"))?;
    if !["subdomain", "subdirectory", "middleware"].contains(&b.mode.as_str()) {
        return Err(AppError::bad_request(
            "mode must be subdomain|subdirectory|middleware",
        ));
    }
    if b.locale != p.source_locale && !p.target_locales.contains(&b.locale) {
        return Err(AppError::bad_request(format!(
            "locale '{}' is not a locale of this project",
            b.locale
        )));
    }
    let origin = clean_origin(&b.origin_url)
        .ok_or_else(|| AppError::bad_request("origin_url must be a public http(s) URL"))?;
    if b.mode == "subdomain" && origin.host_str() == Some(host.as_str()) {
        return Err(AppError::bad_request(
            "the origin must be the source site, not the translated host itself",
        ));
    }
    // Plan limit: the owner's Edge domains across projects, this one included.
    let owned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM edge_domains d
         JOIN project_members m ON m.project_id = d.project_id AND m.role = 'owner'
         WHERE m.user_id = (SELECT user_id FROM project_members WHERE project_id = $1 AND role = 'owner' LIMIT 1)",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    crate::billing::check_edge_domains(&state, p.id, owned + 1).await?;
    let token = format!("glot-verify-{}", hex::encode(rand::random::<[u8; 16]>()));
    let row: Option<EdgeDomain> = sqlx::query_as(&format!(
        "INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token)
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (host) DO NOTHING RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(&host)
    .bind(&b.mode)
    .bind(&b.locale)
    .bind(origin.as_str().trim_end_matches('/'))
    .bind(&token)
    .fetch_optional(&state.db)
    .await?;
    let d = row.ok_or_else(|| AppError::conflict(format!("{host} is already registered")))?;
    Ok((StatusCode::CREATED, Json(out(d))))
}

#[derive(Deserialize)]
struct Patch {
    enabled: bool,
}

async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<Patch>,
) -> ApiResult<Json<EdgeDomain>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let d: EdgeDomain = sqlx::query_as(&format!(
        "UPDATE edge_domains SET enabled = $3 WHERE id = $1 AND project_id = $2 RETURNING {COLS}"
    ))
    .bind(id)
    .bind(p.id)
    .bind(b.enabled)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("domain not found"))?;
    caddy_sync_bg(&state);
    Ok(Json(out(d)))
}

async fn remove(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let n = sqlx::query("DELETE FROM edge_domains WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(p.id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("domain not found"));
    }
    caddy_sync_bg(&state);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------- DNS

/// Records of `name` / `kind` (TXT, CNAME, MX, A) over DNS-over-HTTPS; Err when no resolver answered.
pub(crate) async fn doh(name: &str, kind: &str) -> Result<Vec<String>, ()> {
    let resolvers = match std::env::var("GLOT_DOH_URL") {
        Ok(u) => vec![u],
        Err(_) => vec![
            "https://cloudflare-dns.com/dns-query".into(),
            "https://dns.google/resolve".into(),
        ],
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ())?;
    let code = match kind {
        "TXT" => 16,
        "MX" => 15,
        "A" => 1,
        _ => 5,
    };
    for r in resolvers {
        let Ok(res) = client
            .get(&r)
            .query(&[("name", name), ("type", kind)])
            .header("accept", "application/dns-json")
            .send()
            .await
        else {
            continue;
        };
        let Ok(v) = res.json::<Value>().await else {
            continue;
        };
        // Status 0 = NOERROR, 3 = NXDOMAIN (an answer: nothing there); anything else, try the next resolver.
        match v["Status"].as_i64() {
            Some(0 | 3) => {}
            _ => continue,
        }
        let answers = v["Answer"].as_array().cloned().unwrap_or_default();
        return Ok(answers
            .iter()
            .filter(|a| a["type"].as_i64() == Some(code))
            .filter_map(|a| a["data"].as_str())
            .map(|d| record_value(d, kind))
            .collect());
    }
    Err(())
}

/// TXT `"a" "b"` → `ab`; CNAME `Edge.Glotlang.com.` → `edge.glotlang.com`.
fn record_value(data: &str, kind: &str) -> String {
    if kind == "TXT" {
        let d = data.trim();
        if d.starts_with('"') {
            return d
                .split('"')
                .enumerate()
                .filter(|(i, _)| i % 2 == 1)
                .map(|(_, s)| s)
                .collect();
        }
        return d.to_string();
    }
    data.trim().trim_end_matches('.').to_ascii_lowercase()
}

async fn verify(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<EdgeDomain>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let (host, mode, token): (String, String, String) = sqlx::query_as(
        "SELECT host, mode, verify_token FROM edge_domains WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("domain not found"))?;
    let unavailable = || {
        AppError::new(
            StatusCode::BAD_GATEWAY,
            "dns_unavailable",
            "DNS lookup failed, retry in a minute",
        )
    };
    let txt = doh(&format!("_glot.{host}"), "TXT")
        .await
        .map_err(|_| unavailable())?;
    let mut checks = json!({
        "txt": { "ok": txt.contains(&token), "expected": token, "found": txt },
    });
    let mut ok = txt.contains(&token);
    if mode == "subdomain" {
        let want = cname_target();
        let found = doh(&host, "CNAME").await.map_err(|_| unavailable())?;
        let c = found.contains(&want);
        ok &= c;
        checks["cname"] = json!({ "ok": c, "expected": want, "found": found });
    }
    let d: EdgeDomain = sqlx::query_as(&format!(
        "UPDATE edge_domains SET checks = $3, checked_at = now(),
            verified_at = CASE WHEN $4 THEN coalesce(verified_at, now()) ELSE verified_at END
         WHERE id = $1 AND project_id = $2 RETURNING {COLS}"
    ))
    .bind(id)
    .bind(p.id)
    .bind(&checks)
    .bind(ok)
    .fetch_one(&state.db)
    .await?;
    if ok {
        caddy_sync_bg(&state);
    }
    Ok(Json(out(d)))
}

// ---------------------------------------------------------------------------------------------- Caddy map

/// The `map` block Caddy imports: verified subdomain-mode hosts → their origin (dial address, Host header,
/// scheme). Every token is checked to be a bare DNS name / address, so nothing from the database can inject
/// Caddyfile syntax.
pub fn caddy_map(rows: &[(String, String)]) -> String {
    let safe = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
    };
    let mut out = String::from(
        "# Written by glot (edge/domains.rs) on every Edge domain change. Do not edit.\n\
         map {host} {glot_origin} {glot_origin_host} {glot_origin_scheme} {\n",
    );
    for (host, origin) in rows {
        let Ok(u) = Url::parse(origin) else { continue };
        let (Some(oh), Some(port)) = (u.host_str(), u.port_or_known_default()) else {
            continue;
        };
        let scheme = u.scheme();
        let dial = format!("{oh}:{port}");
        let host_hdr = match u.port() {
            Some(p) => format!("{oh}:{p}"),
            None => oh.to_string(),
        };
        if matches!(scheme, "http" | "https")
            && clean_host(host).as_deref() == Some(host.as_str())
            && safe(&dial)
            && safe(&host_hdr)
        {
            out.push_str(&format!("\t{host} {dial} {host_hdr} {scheme}\n"));
        }
    }
    out.push_str("\tdefault \"\" \"\" \"\"\n}\n");
    out
}

static CADDY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Rewrites the Caddy map from the database and reloads Caddy when it changed. No-op without `GLOT_CADDY_MAP`.
pub async fn caddy_sync(state: &AppState) -> Result<bool, String> {
    let Ok(path) = std::env::var("GLOT_CADDY_MAP") else {
        return Ok(false);
    };
    let _g = CADDY_LOCK.lock().await;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT host, origin_url FROM edge_domains
         WHERE mode = 'subdomain' AND verified_at IS NOT NULL ORDER BY host",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let body = caddy_map(&rows);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
        return Ok(false);
    }
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, &body)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("write {path}: {e}"))?;
    let Ok(admin) = std::env::var("GLOT_CADDY_ADMIN") else {
        return Ok(true);
    };
    let file = std::env::var("GLOT_CADDYFILE").unwrap_or_else(|_| "/etc/caddy/Caddyfile".into());
    let caddyfile = std::fs::read_to_string(&file).map_err(|e| format!("read {file}: {e}"))?;
    let r = reqwest::Client::new()
        .post(format!("{}/load", admin.trim_end_matches('/')))
        .header("content-type", "text/caddyfile")
        .timeout(Duration::from_secs(10))
        .body(caddyfile)
        .send()
        .await
        .map_err(|e| format!("caddy reload: {e}"))?;
    if !r.status().is_success() {
        return Err(format!(
            "caddy reload: {} {}",
            r.status(),
            r.text().await.unwrap_or_default()
        ));
    }
    Ok(true)
}

fn caddy_sync_bg(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = caddy_sync(&state).await {
            tracing::warn!(target: "glot_edge", "caddy map sync failed: {e}");
        }
    });
}

/// At startup: the map reflects the database even if a change happened while the server was down.
pub fn spawn_caddy_sync(state: AppState) {
    caddy_sync_bg(&state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_only_safe_tokens() {
        let m = caddy_map(&[
            ("es.cliponit.app".into(), "https://cliponit.app".into()),
            ("de.x.io".into(), "https://origin.x.io:8443".into()),
            ("fr.x.io".into(), "http://x.io".into()),
            ("evil}.x.io".into(), "https://x.io".into()),
            ("it.x.io".into(), "https://x.io {\n}".into()),
        ]);
        assert!(
            m.contains("\tes.cliponit.app cliponit.app:443 cliponit.app https\n"),
            "{m}"
        );
        assert!(
            m.contains("\tde.x.io origin.x.io:8443 origin.x.io:8443 https\n"),
            "{m}"
        );
        assert!(m.contains("\tfr.x.io x.io:80 x.io http\n"), "{m}");
        assert!(!m.contains("evil") && !m.contains("it.x.io"), "{m}");
        assert!(m.ends_with("\tdefault \"\" \"\" \"\"\n}\n"));
    }

    #[test]
    fn dns_values() {
        assert_eq!(
            record_value(r#""glot-verify-" "abc""#, "TXT"),
            "glot-verify-abc"
        );
        assert_eq!(
            record_value("Edge.Glotlang.com.", "CNAME"),
            "edge.glotlang.com"
        );
        assert!(
            clean_origin("https://cliponit.app/pricing?x")
                .is_some_and(|u| u.as_str() == "https://cliponit.app/")
        );
        assert!(clean_origin("ftp://x.io").is_none());
        assert!(clean_origin("https://u:p@x.io").is_none());
    }
}
