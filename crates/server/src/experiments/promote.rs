//! Cross-language promotion: a variant that won in one language / segment is proposed for the other languages,
//! and one call launches the test of its translation there against each locale's official translation.
//!
//! - `GET  /api/projects/{slug}/xp/{id}/propagate` (T) → `{proposals: [{experiment, arm, intent, won_in, locales}]}`
//! - `POST /api/projects/{slug}/xp/{id}/propagate` (A) `{arm, locales?, start?}` →
//!   `{experiment, rejected: [{locale, key, value, issues, findings}], started, start_blocked}`
//!
//! The new experiment = same keys, goal and guardrails, all traffic (no segment), arm 0 = official, arm 1 = the
//! winner's text translated from its pivot (source-locale text when it has one) by the guarded pipeline, then
//! re-checked here (validator vs the real source, glossary, pixel budget, copy policy). A locale where any key
//! fails is left out, never served half-translated. "Promote everywhere / for this segment" is `…/promote` (api.rs).
//!
//! Git stays the source of truth: every promotion opens a pull request through the project's GitHub App link
//! ([`open_pr`], Git Data API, no clone): `all` rewrites the winning texts in the catalogs and records them in
//! glot.lock (human, reviewed by "experiment winner · <name>"); `segment` updates `.glot/personalizations.json`
//! (also served by `GET …/xp/personalizations`, written by `glot pull`). The body gives the experiment, segment,
//! lift + 95 % interval, P(best), duration and a link to the results.
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use glot_core::{
    context::KeyContext,
    measure::{self, Fit},
    policy,
    validate::{Syntax, has_errors},
};
use glot_llm::translate::{Item, Request, Translator};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{api, load, model::*};
use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    keys,
    projects::{self, ProjectRow, Role},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/projects/{slug}/xp/{id}/propagate",
            get(proposals).post(propagate),
        )
        .route(
            "/api/projects/{slug}/xp/personalizations",
            get(personalizations),
        )
}

/// Locales of the project the experiment did not cover: where the winner is still untested.
fn untested(p: &ProjectRow, x: &Experiment) -> Vec<String> {
    std::iter::once(&p.source_locale)
        .chain(&p.target_locales)
        .filter(|l| !x.locales.contains(l))
        .cloned()
        .collect()
}

fn cell_name(x: &Experiment, cell: &str) -> String {
    x.cells
        .iter()
        .find(|s| s.id.to_string() == cell)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| {
            if cell == ANY {
                "all traffic".into()
            } else {
                cell.into()
            }
        })
}

/// One proposal per winning arm (≥ 1) that still has untested locales.
pub fn proposals_of(p: &ProjectRow, x: &Experiment) -> Vec<Value> {
    let locales = untested(p, x);
    if locales.is_empty() || x.status == Status::Draft {
        return vec![];
    }
    let mut out: Vec<Value> = vec![];
    for c in &x.state {
        let Some(arm) = c.winner.filter(|w| *w > 0) else {
            continue;
        };
        if out.iter().any(|o| o["arm"] == arm) {
            continue;
        }
        let intent = x
            .arms
            .iter()
            .find(|a| a.arm == arm)
            .map(|a| a.intent.clone())
            .unwrap_or_default();
        let won_in: Vec<String> = x
            .state
            .iter()
            .filter(|s| s.winner == Some(arm))
            .map(|s| cell_name(x, &s.cell))
            .collect();
        out.push(json!({ "experiment": x.name, "id": x.id, "arm": arm, "intent": intent, "won_in": won_in,
                         "tested_locales": x.locales, "locales": locales }));
    }
    out
}

pub async fn proposals(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let x = load(&state, p.id, id).await?;
    Ok(Json(json!({ "proposals": proposals_of(&p, &x) })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropagateBody {
    pub arm: i32,
    /// Default: every project locale the experiment did not cover.
    pub locales: Option<Vec<String>>,
    /// Start it right away (default true); a draft is returned when it cannot start yet.
    pub start: Option<bool>,
}

pub async fn propagate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<PropagateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let x = load(&state, p.id, id).await?;
    let arm = x
        .arms
        .iter()
        .find(|a| a.arm == b.arm && a.arm > 0 && a.approved)
        .ok_or_else(|| AppError::bad_request("unknown, unapproved or official arm"))?;
    // Guardrail: only a declared winner travels (a test, not a guess).
    if !x.state.iter().any(|c| c.winner == Some(b.arm)) {
        return Err(AppError::conflict(format!(
            "arm {} has not won in any segment yet: nothing to propagate",
            b.arm
        )));
    }
    let all: Vec<String> = std::iter::once(p.source_locale.clone())
        .chain(p.target_locales.clone())
        .collect();
    let targets = b.locales.clone().unwrap_or_else(|| untested(&p, &x));
    if let Some(l) = targets.iter().find(|l| !all.contains(l)) {
        return Err(AppError::bad_request(format!(
            "'{l}' is not a locale of this project"
        )));
    }
    if targets.is_empty() {
        return Err(AppError::conflict(
            "every locale of the project was already in this test",
        ));
    }
    let complete = |l: &str| {
        arm.values.get(l).filter(|m| {
            x.keys
                .iter()
                .all(|k| m.get(&k.key).is_some_and(|v| !v.trim().is_empty()))
        })
    };
    // Pivot: the source-locale text when the arm has one, else the first tested locale it is complete in.
    let (pivot, pivot_values) = std::iter::once(&p.source_locale)
        .chain(&x.locales)
        .find_map(|l| complete(l).map(|m| (l.clone(), m.clone())))
        .ok_or_else(|| AppError::conflict("the winning arm has no complete text in any locale"))?;

    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let fits: BTreeMap<String, Fit> = serde_json::from_value(x.fits.clone()).unwrap_or_default();
    let mut conn = state.db.acquire().await?;
    let mut recs = vec![];
    for k in &x.keys {
        recs.push(keys::find_key(&mut conn, p.id, &k.key).await?);
    }
    let mut official: BTreeMap<(String, String), String> = BTreeMap::new();
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT k.key, t.locale, t.value FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND k.key = ANY($2) AND t.value <> ''",
    )
    .bind(p.id)
    .bind(x.keys.iter().map(|k| k.key.clone()).collect::<Vec<_>>())
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    for (k, l, v) in rows {
        official.insert((k, l), v);
    }
    let control = |key: &str, l: &str| -> String {
        if *l == p.source_locale {
            recs.iter()
                .find(|r| r.key == key)
                .map(|r| r.source.clone())
                .unwrap_or_default()
        } else {
            official
                .get(&(key.to_string(), l.to_string()))
                .cloned()
                .unwrap_or_default()
        }
    };

    let translator = Translator::new(
        &crate::feedback::config(&p),
        crate::feedback::provider(&state, &p)?,
    );
    let policy_cfg = super::generate::project_policy(&state, p.id).await?;
    let mut values: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut rejected = vec![];
    for l in &targets {
        let texts: BTreeMap<String, String> = match complete(l) {
            Some(m) => m.clone(),
            None if *l == pivot => pivot_values.clone(),
            None => {
                let req = Request {
                    source_locale: pivot.clone(),
                    target_locale: l.clone(),
                    items: recs
                        .iter()
                        .map(|r| Item {
                            key: r.key.clone(),
                            source: pivot_values[&r.key].clone(),
                            context: KeyContext {
                                key: r.key.clone(),
                                description: Some(format!(
                                    "Winning marketing variant ({}) of `{}`. Keep its persuasion intent and length; add no claim, number, deadline or superlative.{}",
                                    arm.intent,
                                    r.key,
                                    r.description.as_deref().map(|d| format!(" Key note: {d}")).unwrap_or_default()
                                )),
                                ui_role: r.ui_role.clone(),
                                component: r.component.clone(),
                                ..Default::default()
                            },
                            previous: None,
                        })
                        .collect(),
                    glossary: glossary
                        .matches(&pivot_values.values().cloned().collect::<Vec<_>>().join("\n"))
                        .into_iter()
                        .cloned()
                        .collect(),
                    syntax: Syntax::Icu,
                    style: p.style.clone(),
                };
                let (done, _usage) = translator.translate(req).await.map_err(|e| {
                    AppError::new(StatusCode::UNPROCESSABLE_ENTITY, "llm_error", e.to_string())
                })?;
                done.into_iter().map(|t| (t.key, t.value)).collect()
            }
        };
        let mut ok = true;
        for r in &recs {
            let value = texts.get(&r.key).cloned().unwrap_or_default();
            let mut issues = if value.trim().is_empty() {
                vec![]
            } else {
                keys::check(&r.source, &value, l, &glossary)
            };
            issues.extend(
                fits.get(&r.key)
                    .and_then(|f| measure::check(&value, Syntax::Icu, f)),
            );
            let findings = policy::check(&value, l, &control(&r.key, l), &x.facts, &policy_cfg);
            if value.trim().is_empty() || has_errors(&issues) || policy::has_errors(&findings) {
                ok = false;
                rejected.push(json!({ "locale": l, "key": r.key, "value": value, "issues": issues, "findings": findings }));
            }
        }
        if ok {
            values.insert(l.clone(), texts);
        }
    }
    if values.is_empty() {
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_valid_variant",
            format!(
                "no locale passed validation ({} rejected cells)",
                rejected.len()
            ),
        ));
    }

    let locales: Vec<String> = values.keys().cloned().collect();
    let name: String = format!("{} → {} (arm {})", x.name, locales.join(", "), arm.arm)
        .chars()
        .take(100)
        .collect();
    let body = api::NewExperiment {
        name: Some(name),
        keys: Some(x.keys.iter().map(|k| k.key.clone()).collect()),
        locales: Some(locales),
        allocation: Some(if x.allocation == Allocation::Fixed {
            Allocation::Fixed
        } else {
            Allocation::Bandit
        }),
        traffic_pct: Some(x.traffic_pct),
        min_n: Some(x.min_n),
        goal: Some(x.goal.clone()),
        metric: Some(x.metric),
        currency: x.currency.clone(),
        guardrails: Some(x.guardrails.clone()),
        facts: Some(x.facts.clone()),
        fits: Some(fits),
        review: Some(x.review),
        arms: Some(vec![api::ArmInput {
            intent: arm.intent.clone(),
            values,
        }]),
        ..Default::default()
    };
    let (_, Json(created)) = api::create(
        State(state.clone()),
        AuthUser(user.clone()),
        AppPath(slug.clone()),
        AppJson(body),
    )
    .await?;
    let (mut started, mut start_blocked) = (false, Value::Null);
    if b.start.unwrap_or(true) {
        match api::start(
            State(state.clone()),
            AuthUser(user),
            AppPath((slug, created.id)),
        )
        .await
        {
            Ok(r) if r.status().is_success() => started = true,
            Ok(r) => {
                let bytes = axum::body::to_bytes(r.into_body(), 1 << 20)
                    .await
                    .unwrap_or_default();
                start_blocked = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            }
            Err(e) => start_blocked = json!({ "error": { "code": e.code, "message": e.message } }),
        }
    }
    let experiment = load(&state, p.id, created.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "experiment": experiment, "pivot": pivot, "rejected": rejected, "started": started, "start_blocked": start_blocked }),
        ),
    ))
}

// ---------- personalizations (Git record of `segment` promotions) ----------

pub const PERSONALIZATIONS_FILE: &str = ".glot/personalizations.json";

/// Every permanent per-segment personalization of the project: what the edge serves, as a reviewable file.
pub async fn personalizations_of(state: &AppState, project_id: Uuid) -> ApiResult<Value> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM xp_experiments WHERE project_id = $1 AND status = 'personalized' ORDER BY created_at",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await?;
    let mut out = vec![];
    for id in ids {
        let x = load(state, project_id, id).await?;
        for c in x.state.iter().filter(|c| c.personalized) {
            let Some(arm) = c.winner.and_then(|w| x.arms.iter().find(|a| a.arm == w)) else {
                continue;
            };
            let segment = x.cells.iter().find(|s| s.id.to_string() == c.cell);
            out.push(json!({
                "experiment": x.name, "id": x.id, "keys": x.keys.iter().map(|k| &k.key).collect::<Vec<_>>(),
                "segment": segment.map(|s| json!({ "name": s.name, "rule": s.rule })).unwrap_or(json!(ANY)),
                "arm": arm.arm, "intent": arm.intent, "values": arm.values,
            }));
        }
    }
    Ok(json!({ "version": 1, "personalizations": out }))
}

pub async fn personalizations(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(personalizations_of(&state, p.id).await?))
}

// ---------- pull request on promotion ----------

use crate::github_app as app;

/// Promotion summary for the PR body: lift [lo; hi], P(best) of `arm` in the cell (or global), from the
/// latest stats snapshot (XP2 `xp_analyses`).
async fn stats_line(state: &AppState, x: &Experiment, arm: i32, cell: Option<&str>) -> String {
    let a: Option<Value> = sqlx::query_scalar(
        "SELECT analysis FROM xp_analyses WHERE experiment_id = $1 ORDER BY at DESC LIMIT 1",
    )
    .bind(x.id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(a) = a else {
        return "no analysis snapshot recorded".into();
    };
    let name = cell.map(|c| cell_name(x, c));
    let r = a["cells"]
        .as_array()
        .and_then(|cs| {
            cs.iter().find(|r| {
                name.as_deref().is_some_and(|n| r["name"] == n)
                    || cell.is_some_and(|c| r["name"] == c)
            })
        })
        .unwrap_or(&a["global"]);
    let i = a["arm_ids"]
        .as_array()
        .and_then(|ids| ids.iter().position(|v| *v == arm))
        .unwrap_or(arm as usize);
    let ar = &r["arms"][i];
    let pct = |v: &Value| {
        v.as_f64()
            .map_or("n/a".into(), |f| format!("{:+.1} %", f * 100.0))
    };
    format!(
        "lift {} (95 % interval {} to {}), P(best) {}, {} visitors",
        pct(&ar["lift"]),
        pct(&ar["lift_lo"]),
        pct(&ar["lift_hi"]),
        ar["p_best"]
            .as_f64()
            .map_or("n/a".into(), |p| format!("{:.1} %", p * 100.0)),
        r["views"]
    )
}

/// File content at `path` on `branch` (None when absent).
async fn content(
    token: &str,
    api: &str,
    path: &str,
    branch: &str,
) -> anyhow::Result<Option<String>> {
    match app::gh(
        token,
        reqwest::Method::GET,
        &format!("{api}/contents/{path}?ref={branch}"),
        None,
    )
    .await
    {
        Ok(v) => {
            let b64 = v["content"].as_str().unwrap_or("").replace('\n', "");
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)?;
            Ok(Some(String::from_utf8(bytes)?))
        }
        Err(e) if e.to_string().contains("404") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Opens the pull request recording a promotion in the linked repo. `Ok(None)`: the project has no GitHub link.
/// ponytail: JSON catalogs only (nested/flat); other formats get the lock + a note, the catalogs via `glot pull`.
pub async fn open_pr(
    state: &AppState,
    p: &ProjectRow,
    x: &Experiment,
    everywhere: bool,
    arm: i32,
    cell: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let msg = |e: AppError| anyhow::anyhow!(e.message);
    let Some(link) = app::load_link(state, p.id).await.map_err(msg)? else {
        return Ok(None);
    };
    let gh_app = app::require_app(state).await.map_err(msg)?;
    // Installation token narrowed to this repo and to contents + pull requests (same request as RV5).
    let name = link.repo.split('/').nth(1).unwrap_or(&link.repo);
    let token = app::gh(
        &app::app_jwt(&gh_app)?,
        reqwest::Method::POST,
        &format!("{}/app/installations/{}/access_tokens", app::api_base(), link.installation_id),
        Some(json!({ "repositories": [name], "permissions": { "contents": "write", "pull_requests": "write", "metadata": "read" } })),
    )
    .await?["token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("GitHub returned no installation token"))?;
    let api = format!("{}/repos/{}", app::api_base(), link.repo);
    let branch = &link.branch;
    let Some(arm_v) = x.arms.iter().find(|a| a.arm == arm) else {
        anyhow::bail!("unknown arm {arm}")
    };

    let mut files: Vec<(String, String)> = vec![];
    let mut notes = vec![];
    if everywhere && arm > 0 {
        let cfg: glot_core::config::Config = toml::from_str(
            &content(&token, &api, glot_core::config::CONFIG_FILE, branch)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("no glot.toml at the root of {}@{branch}", link.repo)
                })?,
        )?;
        let lock_text = content(&token, &api, glot_core::lock::LOCK_FILE, branch).await?;
        let mut lock: glot_core::lock::Lockfile = lock_text
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        for (locale, values) in arm_v
            .values
            .iter()
            .filter(|(l, _)| p.target_locales.contains(l))
        {
            let paths = cfg.catalog_paths(std::path::Path::new(""), locale);
            let mut cats = vec![];
            for path in &paths {
                let rel = path.to_string_lossy().replace('\\', "/");
                let text = content(&token, &api, &rel, branch)
                    .await?
                    .unwrap_or_else(|| "{}".into());
                let format = glot_core::catalog::Catalog::detect_format(&text);
                if !matches!(
                    format,
                    glot_core::catalog::Format::JsonNested
                        | glot_core::catalog::Format::JsonFlat
                        | glot_core::catalog::Format::Auto
                ) {
                    notes.push(format!(
                        "`{rel}` is not a JSON catalog: run `glot pull` to bring {locale} in"
                    ));
                    continue;
                }
                cats.push((
                    rel,
                    format,
                    glot_core::catalog::Catalog::parse(&text, locale)?,
                ));
            }
            if cats.is_empty() {
                continue;
            }
            for (key, value) in values {
                // The file that already holds the key, else the first one.
                let i = cats
                    .iter()
                    .position(|c| c.2.entries.contains_key(key))
                    .unwrap_or(0);
                cats[i].2.entries.insert(key.clone(), value.clone());
                let source = x
                    .keys
                    .iter()
                    .find(|k| &k.key == key)
                    .map(|k| k.source.as_str())
                    .unwrap_or("");
                lock.record(
                    locale,
                    key,
                    source,
                    value,
                    glot_core::lock::Origin::Human,
                    None,
                );
                if let Some(e) = lock.entries.get_mut(locale).and_then(|m| m.get_mut(key)) {
                    e.reviewed_by = Some(format!("experiment winner · {}", x.name));
                }
            }
            for (rel, format, cat) in cats {
                let f = if format == glot_core::catalog::Format::Auto {
                    glot_core::catalog::Format::JsonNested
                } else {
                    format
                };
                files.push((rel, cat.to_json(f)));
            }
        }
        let mut text = serde_json::to_string_pretty(&lock)?;
        text.push('\n');
        files.push((glot_core::lock::LOCK_FILE.into(), text));
    } else if !everywhere {
        let mut text =
            serde_json::to_string_pretty(&personalizations_of(state, p.id).await.map_err(msg)?)?;
        text.push('\n');
        files.push((PERSONALIZATIONS_FILE.into(), text));
    }
    if files.is_empty() {
        return Ok(None);
    }

    // One commit on a fresh branch through the Git Data API, then the PR.
    let base = app::gh(
        &token,
        reqwest::Method::GET,
        &format!("{api}/git/ref/heads/{branch}"),
        None,
    )
    .await?;
    let base_sha = base["object"]["sha"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let commit = app::gh(
        &token,
        reqwest::Method::GET,
        &format!("{api}/git/commits/{base_sha}"),
        None,
    )
    .await?;
    let tree: Vec<Value> = files
        .iter()
        .map(|(path, c)| json!({ "path": path, "mode": "100644", "type": "blob", "content": c }))
        .collect();
    let t = app::gh(
        &token,
        reqwest::Method::POST,
        &format!("{api}/git/trees"),
        Some(json!({ "base_tree": commit["tree"]["sha"], "tree": tree })),
    )
    .await?;
    let segment = cell.map(|c| cell_name(x, c));
    let title = match &segment {
        Some(s) => format!(
            "glot: personalize \"{}\" for {s} (arm {arm}: {})",
            x.name, arm_v.intent
        ),
        None => format!(
            "glot: promote \"{}\" winner (arm {arm}: {})",
            x.name, arm_v.intent
        ),
    };
    let c = app::gh(
        &token,
        reqwest::Method::POST,
        &format!("{api}/git/commits"),
        Some(json!({ "message": title, "tree": t["sha"], "parents": [base_sha] })),
    )
    .await?;
    let head = format!("glot/xp-{}-{arm}", &x.id.simple().to_string()[..8]);
    app::gh(
        &token,
        reqwest::Method::POST,
        &format!("{api}/git/refs"),
        Some(json!({ "ref": format!("refs/heads/{head}"), "sha": c["sha"] })),
    )
    .await?;
    let days = x
        .started_at
        .map(|s| (chrono::Utc::now() - s).num_hours() as f64 / 24.0);
    let mut body = format!(
        "**Experiment** {} (`{}`) — goal `{}`\n\n**Scope** {}\n\n**Winner** arm {arm} ({})\n\n**Result** {}\n\n**Duration** {}\n\n**Keys** {}\n\n[Results in glot]({}/app/{}/experiments?xp={})\n",
        x.name,
        x.id,
        x.goal,
        segment.as_deref().map_or("everywhere: official translation in every tested locale (glot.lock: experiment winner)".to_string(), |s| format!("segment \"{s}\": served permanently by the edge ({PERSONALIZATIONS_FILE})")),
        arm_v.intent,
        stats_line(state, x, arm, cell).await,
        days.map_or("n/a".into(), |d| format!("{d:.1} days")),
        x.keys.iter().map(|k| format!("`{}`", k.key)).collect::<Vec<_>>().join(", "),
        app::web_origin(),
        p.slug,
        x.id,
    );
    for n in notes {
        body.push_str(&format!("\n> {n}\n"));
    }
    let pr = app::gh(
        &token,
        reqwest::Method::POST,
        &format!("{api}/pulls"),
        Some(json!({ "title": title, "head": head, "base": branch, "body": body })),
    )
    .await?;
    Ok(pr["html_url"].as_str().map(String::from))
}
