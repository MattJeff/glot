//! Translated URL paths (opt-in, `edge_settings.translated_slugs`): `/pricing` is `/precios` on the Spanish site.
//!
//! - One stable mapping per (project, locale, source path) in `edge_slugs`. A slug is written once and never
//!   changes by itself; replacing it (dashboard / API) keeps the old row, so the old URL redirects to the new one.
//! - Where they come from: each path segment is read as text (`my-first-post` → `My first post`) and looked up
//!   in the locale's TM. Unknown texts are reported like any missing segment (role `slug`), so they are
//!   translated by the same job, under the same daily cap, budget, glossary and validators as the page text —
//!   and a nav link "Pricing" already translated gives the slug for free. [`materialize`] then writes the slug.
//! - The proxy (`proxy.rs`) serves `/precios` from the origin's `/pricing`, redirects `/pricing` → `/precios`
//!   (HTML navigations only) and old slugs to the current one; canonical, hreflang, links and sitemaps go
//!   through `glot_core::edge::seo::localized_path`. Switched off: every slug redirects back to its source path.
//!
//! `GET /api/projects/{slug}/edge/slugs` lists them, `PUT` sets one (`{"locale", "source_path", "slug"}`;
//! `slug` = the source path pins "no translated slug").
// ponytail: whole-path mapping, ASCII slugs only (Latin scripts folded; other scripts keep the source segment).
// Per-segment rules and non-Latin slugs are the upgrade if customers ask.
use axum::{Json, Router, extract::State, routing::get};
use glot_core::edge::{
    seo::Slugs,
    tm::{Tm, text_hash},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use super::discover::Settings;
use super::missing;
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
};

const ROLE: &str = "slug";
/// Paths considered per run.
const MAX_PATHS: i64 = 5000;

#[derive(Debug, Default, Clone)]
pub struct Loaded {
    /// locale → source path → current slug (identity rows left out).
    pub current: Slugs,
    /// locale → any slug ever served (current or replaced) → source path.
    pub sources: BTreeMap<String, BTreeMap<String, String>>,
}

pub async fn load(db: &sqlx::PgPool, project: uuid::Uuid) -> sqlx::Result<Loaded> {
    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
        "SELECT locale, slug, source_path, current FROM edge_slugs WHERE project_id = $1",
    )
    .bind(project)
    .fetch_all(db)
    .await?;
    let mut out = Loaded::default();
    for (locale, slug, source, current) in rows {
        if slug == source {
            continue;
        }
        if current {
            out.current
                .entry(locale.clone())
                .or_default()
                .insert(source.clone(), slug.clone());
        }
        out.sources.entry(locale).or_default().insert(slug, source);
    }
    Ok(out)
}

/// `my-first-post` → `My first post`; `None` for what is not prose (ids, files, one or two letters).
pub fn humanize(segment: &str) -> Option<String> {
    let ok = (3..=80).contains(&segment.len())
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && segment.as_bytes()[0].is_ascii_alphabetic()
        && segment.bytes().filter(u8::is_ascii_digit).count() < 4;
    if !ok {
        return None;
    }
    let words: Vec<&str> = segment
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() || words.len() > 12 {
        return None;
    }
    let text = words.join(" ").to_ascii_lowercase();
    let mut c = text.chars();
    let first = c.next()?.to_ascii_uppercase();
    Some(format!("{first}{}", c.as_str()))
}

/// `Précios & más` → `precios-mas`; `None` when nothing ASCII is left (non-Latin scripts keep the source segment).
pub fn slugify(text: &str) -> Option<String> {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        let folded = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ă' | 'ą' => "a",
            'ç' | 'č' | 'ć' => "c",
            'ď' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ě' | 'ę' => "e",
            'ğ' => "g",
            'ì' | 'í' | 'î' | 'ï' | 'ı' => "i",
            'ł' => "l",
            'ñ' | 'ň' | 'ń' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ő' => "o",
            'ř' => "r",
            'ś' | 'š' | 'ş' | 'ș' => "s",
            'ť' | 'ţ' | 'ț' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ů' | 'ű' => "u",
            'ý' | 'ÿ' => "y",
            'ž' | 'ź' | 'ż' => "z",
            'ß' => "ss",
            'œ' => "oe",
            'æ' => "ae",
            c if c.is_ascii_alphanumeric() => {
                out.push(c);
                continue;
            }
            c if c.is_alphanumeric() => return None,
            _ => "-",
        };
        if folded != "-" || !out.ends_with('-') {
            out.push_str(folded);
        }
    }
    let out = out.trim_matches('-');
    let out = &out[..out.len().min(80)];
    (!out.is_empty()).then(|| out.trim_matches('-').to_string())
}

/// The localized path of `path` from the TM: `Ok(path)` when every prose segment is translated (segments that
/// are not prose stay as they are), `Err(texts)` = the texts still waiting for a translation.
fn localized(path: &str, source_locale: &str, tm: &Tm) -> Result<String, Vec<String>> {
    let mut out = String::new();
    let mut waiting = vec![];
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        match humanize(seg) {
            Some(text) => match tm.lookup(&text_hash(source_locale, &text), &format!("{ROLE}|/")) {
                Some(v) => out.push_str(slugify(v).as_deref().unwrap_or(seg)),
                None => waiting.push(text),
            },
            None => out.push_str(seg),
        }
    }
    if path.len() > 1 && path.ends_with('/') {
        out.push('/');
    }
    if waiting.is_empty() {
        Ok(out)
    } else {
        Err(waiting)
    }
}

async fn known_paths(db: &sqlx::PgPool, project: uuid::Uuid) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT path FROM edge_page_segments WHERE project_id = $1 GROUP BY path ORDER BY path LIMIT $2",
    )
    .bind(project)
    .bind(MAX_PATHS)
    .fetch_all(db)
    .await
}

/// Writes the slugs the TM can give now, and reports the texts it cannot translate yet (the usual discovery
/// path: caps, budget, review rules). Returns how many slugs were written. A no-op when the option is off.
pub async fn materialize(state: &AppState, p: &ProjectRow) -> ApiResult<usize> {
    let settings = Settings::load(&state.db, p.id).await?;
    if !settings.translated_slugs || !settings.enabled {
        return Ok(0);
    }
    let paths = known_paths(&state.db, p.id).await?;
    let sources: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    let have = load(&state.db, p.id).await?;
    let pinned: BTreeSet<(String, String)> = sqlx::query_as(
        "SELECT locale, source_path FROM edge_slugs WHERE project_id = $1 AND current",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let mut written = 0;
    for locale in &p.target_locales {
        let tm = super::tm::snapshot(&state.db, p, locale).await?;
        let mut waiting: BTreeSet<String> = BTreeSet::new();
        let (mut slugs, mut srcs) = (vec![], vec![]);
        let mut taken: BTreeSet<String> = have
            .sources
            .get(locale)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        for path in &paths {
            if path == "/" || pinned.contains(&(locale.clone(), path.clone())) {
                continue;
            }
            match localized(path, &p.source_locale, &tm) {
                // Never shadow a real page of the site, never reuse a slug.
                Ok(slug)
                    if slug != *path
                        && !sources.contains(slug.as_str())
                        && taken.insert(slug.clone()) =>
                {
                    slugs.push(slug);
                    srcs.push(path.clone());
                }
                Ok(_) => {}
                Err(texts) => waiting.extend(texts),
            }
        }
        written += sqlx::query(
            "INSERT INTO edge_slugs (project_id, locale, slug, source_path)
             SELECT $1, $2, s, p FROM unnest($3::text[], $4::text[]) AS x(s, p)
             ON CONFLICT DO NOTHING",
        )
        .bind(p.id)
        .bind(locale)
        .bind(&slugs)
        .bind(&srcs)
        .execute(&state.db)
        .await?
        .rows_affected() as usize;
        if !waiting.is_empty() {
            let report = missing::Report {
                key: None,
                locale: locale.clone(),
                auth: false,
                segments: waiting
                    .into_iter()
                    .map(|text| missing::Item {
                        hash: text_hash(&p.source_locale, &text),
                        text,
                        url: "/".into(),
                        role: ROLE.into(),
                    })
                    .collect(),
                pages: vec![],
                mode: None,
            };
            missing::record(state, p, &report).await?;
        }
    }
    Ok(written)
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/projects/{slug}/edge/slugs", get(list).put(put))
}

async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let settings = Settings::load(&state.db, p.id).await?;
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT locale, source_path, slug, origin FROM edge_slugs
         WHERE project_id = $1 AND current ORDER BY source_path, locale",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "enabled": settings.translated_slugs,
        "slugs": rows.into_iter().map(|(locale, source_path, slug, origin)|
            json!({"locale": locale, "source_path": source_path, "slug": slug, "origin": origin})).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct PutBody {
    locale: String,
    source_path: String,
    slug: String,
}

fn valid_path(p: &str) -> bool {
    p.starts_with('/')
        && p.len() <= 200
        && !p.contains("//")
        && !p.starts_with("/__glot")
        && p.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'/')
        })
}

async fn put(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PutBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if !p.target_locales.contains(&b.locale) {
        return Err(AppError::bad_request("not a target locale of this project"));
    }
    let source = b.source_path.trim().to_string();
    let new = b.slug.trim().trim_end_matches('/').to_string();
    if !source.starts_with('/') || source == "/" || source.len() > 2048 {
        return Err(AppError::bad_request(
            "source_path must be a page path other than /",
        ));
    }
    if new != source && !valid_path(&new) {
        return Err(AppError::bad_request(
            "slug must start with / and use a-z, 0-9, - and _ only",
        ));
    }
    let mut tx = state.db.begin().await?;
    let clash: Option<String> = sqlx::query_scalar(
        "SELECT source_path FROM edge_slugs
         WHERE project_id = $1 AND locale = $2 AND slug = $3 AND source_path <> $4 AND current
         UNION ALL
         SELECT path FROM edge_page_segments WHERE project_id = $1 AND path = $3 AND path <> $4 LIMIT 1",
    )
    .bind(p.id)
    .bind(&b.locale)
    .bind(&new)
    .bind(&source)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(other) = clash {
        return Err(AppError::bad_request(format!(
            "'{new}' is already the address of {other}"
        )));
    }
    sqlx::query(
        "UPDATE edge_slugs SET current = false
         WHERE project_id = $1 AND locale = $2 AND source_path = $3 AND current",
    )
    .bind(p.id)
    .bind(&b.locale)
    .bind(&source)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO edge_slugs (project_id, locale, slug, source_path, origin) VALUES ($1, $2, $3, $4, 'human')
         ON CONFLICT (project_id, locale, slug) DO UPDATE SET source_path = $4, current = true, origin = 'human'",
    )
    .bind(p.id)
    .bind(&b.locale)
    .bind(&new)
    .bind(&source)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"locale": b.locale, "source_path": source, "slug": new, "origin": "human"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_from_text() {
        assert_eq!(humanize("my-first_post").as_deref(), Some("My first post"));
        assert_eq!(humanize("pricing").as_deref(), Some("Pricing"));
        for not_prose in ["42", "ab", "3f2a9c1e", "logo.png", "order-20261107", "é"] {
            assert_eq!(humanize(not_prose), None, "{not_prose}");
        }
        assert_eq!(slugify("Précios & más").as_deref(), Some("precios-mas"));
        assert_eq!(slugify("Über uns!").as_deref(), Some("uber-uns"));
        assert_eq!(slugify("料金"), None);
        assert_eq!(slugify("--"), None);

        let h = |t: &str| text_hash("en", t);
        let tm = Tm::from_rows(
            "es",
            1,
            [
                (h("Pricing"), "a|/".to_string(), "Precios".to_string()),
                (h("Blog"), "slug|/".to_string(), "Blog".to_string()),
                (
                    h("My first post"),
                    "slug|/".to_string(),
                    "Mi primer artículo".to_string(),
                ),
            ],
        );
        // The nav link's translation gives the slug; ids and files are kept.
        assert_eq!(localized("/pricing", "en", &tm).unwrap(), "/precios");
        assert_eq!(
            localized("/blog/my-first-post/", "en", &tm).unwrap(),
            "/blog/mi-primer-articulo/"
        );
        assert_eq!(localized("/blog/42", "en", &tm).unwrap(), "/blog/42");
        assert_eq!(localized("/about-us", "en", &tm).unwrap_err(), ["About us"]);
        assert!(
            valid_path("/precios/plan-1") && !valid_path("/Précios") && !valid_path("/__glot/x")
        );
    }
}
