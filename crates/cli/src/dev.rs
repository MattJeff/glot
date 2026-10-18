//! `glot dev`: local server for the in-app overlay (`@glot/overlay`).
//! Localhost only, random bearer token, CORS limited to the app origin(s).
//! Writes go through `Workspace` (validated) → catalog files change → the app's HMR reloads.
#![allow(clippy::result_large_err)] // axum Response as error type is idiomatic here
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, broadcast};
use tower_http::cors::{AllowOrigin, CorsLayer};

use glot_core::validate::{self, Constraints};
use glot_llm::engine::{TranslateOptions, Workspace};
use glot_llm::provider::{self, CompletionRequest};

use crate::style::*;

#[derive(clap::Args)]
pub struct DevArgs {
    #[arg(long, default_value_t = 4317)]
    pub port: u16,
    /// Allowed app origin(s), e.g. http://localhost:3000. Default: any http://localhost:* / 127.0.0.1:*.
    #[arg(long)]
    pub origin: Vec<String>,
    /// Fixed token (else GLOT_DEV_TOKEN, else random).
    #[arg(long)]
    pub token: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    ws: Arc<Mutex<Workspace>>,
    token: Arc<String>,
    events: broadcast::Sender<String>,
}

/// The overlay bundle, so `<script src="http://localhost:4317/overlay.js">` works without npm.
const OVERLAY_JS: &str = include_str!("../../../packages/overlay/dist/overlay.global.js");

pub async fn run(args: DevArgs) -> Result<()> {
    let ws = Workspace::open(&std::env::current_dir()?)?;
    let token = args
        .token
        .or_else(|| std::env::var("GLOT_DEV_TOKEN").ok())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port)).await?;
    let addr = listener.local_addr()?;
    anstream::println!(
        "{GREEN}glot dev{GREEN:#} on {BOLD}http://{addr}{BOLD:#}  ({} keys, locales {})",
        ws.source().entries.len(),
        ws.config.all_locales().join(", ")
    );
    anstream::println!("  token  {BOLD}{token}{BOLD:#}");
    anstream::println!(
        "  {DIM}open your app with ?glot_token={token} once (stored in localStorage), or paste it in the overlay{DIM:#}"
    );
    anstream::println!("  {DIM}Alt+click any text, or Alt+Shift+G for inspect mode{DIM:#}");
    let state = new_state(ws, token);
    spawn_watcher(state.clone());
    axum::serve(listener, router(state, &args.origin)).await?;
    Ok(())
}

pub fn new_state(ws: Workspace, token: String) -> AppState {
    AppState {
        ws: Arc::new(Mutex::new(ws)),
        token: Arc::new(token),
        events: broadcast::channel(64).0,
    }
}

pub fn router(state: AppState, origins: &[String]) -> Router {
    let allow = if origins.is_empty() {
        AllowOrigin::predicate(|o: &HeaderValue, _| is_local_origin(o.to_str().unwrap_or("")))
    } else {
        AllowOrigin::list(origins.iter().filter_map(|o| o.parse().ok()))
    };
    let cors = CorsLayer::new()
        .allow_origin(allow)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    let api = Router::new()
        .route("/api/state", get(api_state))
        .route("/api/lookup", post(api_lookup))
        .route("/api/key", get(api_key))
        .route("/api/set", post(api_set))
        .route("/api/suggest", post(api_suggest))
        .route("/api/translate", post(api_translate))
        .route("/api/propose", post(api_propose))
        .route("/api/extract", post(api_extract))
        .route("/api/events", get(api_events))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route(
            "/overlay.js",
            get(|| async { ([(header::CONTENT_TYPE, "text/javascript")], OVERLAY_JS) }),
        )
        .merge(api)
        .layer(cors)
        .with_state(state)
}

fn is_local_origin(o: &str) -> bool {
    let host = o
        .strip_prefix("http://")
        .or_else(|| o.strip_prefix("https://"))
        .unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    // EventSource cannot send headers: accept ?token= too.
    let query = req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|p| p.strip_prefix("token="))
            .map(str::to_string)
    });
    match bearer.or(query) {
        Some(t) if ct_eq(t.as_bytes(), s.token.as_bytes()) => next.run(req).await,
        _ => err(StatusCode::UNAUTHORIZED, "invalid or missing token"),
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn err(code: StatusCode, msg: impl ToString) -> Response {
    (
        code,
        Json(json!({"error": {"code": code.as_u16(), "message": msg.to_string()}})),
    )
        .into_response()
}

type ApiResult = Result<Json<Value>, Response>;

fn bad(e: impl ToString) -> Response {
    err(StatusCode::BAD_REQUEST, e)
}

async fn api_state(State(s): State<AppState>) -> ApiResult {
    let ws = s.ws.lock().await;
    Ok(Json(json!({
        "source_locale": ws.config.source_locale,
        "target_locales": ws.config.target_locales,
        "keys": ws.source().entries.len(),
        "provider": ws.config.llm.provider,
    })))
}

// ---------- lookup: displayed text -> keys ----------

#[derive(Deserialize)]
struct LookupReq {
    text: String,
}

async fn api_lookup(State(s): State<AppState>, Json(r): Json<LookupReq>) -> ApiResult {
    let ws = s.ws.lock().await;
    Ok(Json(json!({ "matches": lookup(&ws, &r.text) })))
}

/// Matches over every locale (the app may be showing any of them), best first, one per key.
// ponytail: linear scan per click (~1 ms per 10k entries); build an inverted index if catalogs get huge.
pub fn lookup(ws: &Workspace, text: &str) -> Vec<Value> {
    let text = norm(text);
    if text.is_empty() {
        return vec![];
    }
    let mut best: BTreeMap<String, (u8, String, String)> = BTreeMap::new();
    for (loc, cat) in &ws.catalogs {
        for (key, value) in &cat.entries {
            let score = match_score(&text, value);
            if score > 0 && best.get(key).is_none_or(|b| b.0 < score) {
                best.insert(key.clone(), (score, loc.clone(), value.clone()));
            }
        }
    }
    let mut out: Vec<_> = best.into_iter().collect();
    out.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.len().cmp(&b.0.len())));
    // A full match makes partial (substring) hits noise.
    let top = out.first().map_or(0, |m| m.1.0);
    out.retain(|m| top < 90 || m.1.0 >= 90);
    out.into_iter()
        .take(10)
        .map(|(key, (score, locale, value))| json!({"key": key, "locale": locale, "value": value, "score": score}))
        .collect()
}

/// 100 exact, 90 placeholder pattern, 50 partial (substring either way), 0 none.
fn match_score(text: &str, value: &str) -> u8 {
    let segs = segments(value);
    let literal: usize = segs.iter().map(|s| s.trim().chars().count()).sum();
    if segs.len() == 1 {
        let v = &segs[0];
        if v == text {
            return 100;
        }
        let short = v.chars().count().min(text.chars().count());
        if short >= 4 && (text.contains(v.as_str()) || v.contains(text)) {
            return 50;
        }
        return 0;
    }
    if literal < 2 {
        return 0; // "{count}" alone would match anything
    }
    if glob(text, &segs) { 90 } else { 0 }
}

/// Literal pieces of a message, split at `{placeholders}` (ICU/i18next, nested), rich tags removed, normalized.
fn segments(value: &str) -> Vec<String> {
    let mut segs = vec![String::new()];
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut depth = 1;
                for c in chars.by_ref() {
                    depth += match c {
                        '{' => 1,
                        '}' => -1,
                        _ => 0,
                    };
                    if depth == 0 {
                        break;
                    }
                }
                segs.push(String::new());
            }
            '<' if chars
                .peek()
                .is_some_and(|n| n.is_ascii_alphabetic() || *n == '/') =>
            {
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
            }
            c => segs.last_mut().unwrap().push(c),
        }
    }
    let n = segs.len();
    segs.iter()
        .enumerate()
        .map(|(i, s)| {
            let s = collapse(s);
            let s = if i == 0 { s.trim_start().into() } else { s };
            if i == n - 1 { s.trim_end().into() } else { s }
        })
        .collect()
}

fn glob(text: &str, segs: &[String]) -> bool {
    let (first, last) = (&segs[0], &segs[segs.len() - 1]);
    if !text.starts_with(first.as_str()) || !text.ends_with(last.as_str()) {
        return false;
    }
    let mut pos = first.len();
    for s in &segs[1..segs.len() - 1] {
        match text[pos..].find(s.as_str()) {
            Some(i) => pos += i + s.len(),
            None => return false,
        }
    }
    text.len() >= pos + last.len()
}

fn collapse(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.extend(c.to_lowercase());
        }
    }
    if space {
        out.push(' ');
    }
    out
}

fn norm(s: &str) -> String {
    collapse(s).trim().to_string()
}

// ---------- key details ----------

#[derive(Deserialize)]
struct KeyQ {
    key: String,
}

async fn api_key(State(s): State<AppState>, Query(q): Query<KeyQ>) -> ApiResult {
    let ws = s.ws.lock().await;
    key_info(&ws, &q.key).map(Json)
}

fn key_info(ws: &Workspace, key: &str) -> Result<Value, Response> {
    if !ws.source().entries.contains_key(key) {
        return Err(err(StatusCode::NOT_FOUND, format!("unknown key \"{key}\"")));
    }
    let values: BTreeMap<_, _> = ws
        .config
        .all_locales()
        .into_iter()
        .map(|l| {
            let v = ws
                .catalogs
                .get(&l)
                .and_then(|c| c.entries.get(key))
                .cloned();
            (l, v)
        })
        .collect();
    let mut issues: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for f in ws.check().into_iter().filter(|f| f.key == key) {
        issues
            .entry(f.locale)
            .or_default()
            .push(serde_json::to_value(f.issue).unwrap_or_default());
    }
    let lock: BTreeMap<_, _> = ws
        .lock
        .entries
        .iter()
        .filter_map(|(l, e)| Some((l.clone(), e.get(key)?.clone())))
        .collect();
    let ctx = ws.extract().ok().and_then(|r| ws.contexts(&r).remove(key));
    Ok(json!({
        "key": key,
        "source_locale": ws.config.source_locale,
        "values": values,
        "issues": issues,
        "lock": lock,
        "note": ws.lock.notes.get(key),
        "refs": ctx.as_ref().map(|c| c.refs.clone()).unwrap_or_default(),
        "ui_role": ctx.as_ref().and_then(|c| c.ui_role.clone()),
        "component": ctx.as_ref().and_then(|c| c.component.clone()),
    }))
}

// ---------- writes ----------

#[derive(Deserialize)]
struct SetReq {
    key: String,
    locale: String,
    value: String,
}

async fn api_set(State(s): State<AppState>, Json(r): Json<SetReq>) -> ApiResult {
    let mut ws = s.ws.lock().await;
    if r.locale == ws.config.source_locale {
        if !ws.source().entries.contains_key(&r.key) {
            return Err(bad(format!("unknown key \"{}\"", r.key)));
        }
        ws.add_source(&r.key, &r.value, None).map_err(bad)?;
        return Ok(Json(json!({"ok": true, "issues": []})));
    }
    let issues = ws
        .set_translation(&r.locale, &r.key, &r.value)
        .map_err(bad)?;
    Ok(Json(
        json!({"ok": !validate::has_errors(&issues), "issues": issues}),
    ))
}

#[derive(Deserialize)]
struct TranslateReq {
    key: String,
    #[serde(default)]
    force: bool,
}

async fn api_translate(State(s): State<AppState>, Json(r): Json<TranslateReq>) -> ApiResult {
    let mut ws = s.ws.lock().await;
    let report = ws
        .translate(TranslateOptions {
            keys: Some(vec![r.key]),
            force: r.force,
            ..Default::default()
        })
        .await
        .map_err(bad)?;
    Ok(Json(serde_json::to_value(report).unwrap_or_default()))
}

#[derive(Deserialize)]
struct ProposeReq {
    text: String,
}

/// Hardcoded text: suggested key, duplicates, and where it sits in the code.
async fn api_propose(State(s): State<AppState>, Json(r): Json<ProposeReq>) -> ApiResult {
    let ws = s.ws.lock().await;
    let mut p = serde_json::to_value(ws.propose(&r.text, None)).unwrap_or_default();
    let text = norm(&r.text);
    if let Ok(report) = ws.extract() {
        let hits: Vec<_> = report
            .hardcoded
            .iter()
            .filter(|h| norm(&h.text) == text)
            .collect();
        if let Some(h) = hits.first().filter(|h| !h.suggested_key.is_empty()) {
            p["key"] = json!(h.suggested_key);
            p["key_exists"] = json!(ws.source().entries.contains_key(&h.suggested_key));
        }
        p["refs"] = json!(hits.iter().map(|h| &h.at).collect::<Vec<_>>());
    }
    Ok(Json(p))
}

#[derive(Deserialize)]
struct ExtractReq {
    key: String,
    value: String,
    note: Option<String>,
}

async fn api_extract(State(s): State<AppState>, Json(r): Json<ExtractReq>) -> ApiResult {
    let mut ws = s.ws.lock().await;
    if ws.source().entries.contains_key(r.key.trim()) {
        return Err(err(
            StatusCode::CONFLICT,
            format!("key \"{}\" already exists", r.key),
        ));
    }
    ws.add_source(&r.key, &r.value, r.note.as_deref())
        .map_err(bad)?;
    Ok(Json(json!({"ok": true, "key": r.key.trim()})))
}

// ---------- AI suggestions ----------

#[derive(Deserialize)]
struct SuggestReq {
    key: Option<String>,
    text: Option<String>,
    locale: String,
    /// variants | shorten | formal | casual | free text
    #[serde(default)]
    instruction: String,
}

async fn api_suggest(State(s): State<AppState>, Json(r): Json<SuggestReq>) -> ApiResult {
    // Gather under the lock, call the LLM without it.
    let (cfg, source, current, note, src_locale, glossary) = {
        let ws = s.ws.lock().await;
        let get = |l: &str, k: &str| ws.catalogs.get(l).and_then(|c| c.entries.get(k)).cloned();
        let source = r
            .key
            .as_deref()
            .and_then(|k| get(&ws.config.source_locale, k));
        let current = r
            .key
            .as_deref()
            .and_then(|k| get(&r.locale, k))
            .or_else(|| r.text.clone())
            .or_else(|| source.clone())
            .ok_or_else(|| bad("key or text required"))?;
        (
            ws.config.clone(),
            source,
            current,
            r.key.as_deref().and_then(|k| ws.lock.notes.get(k).cloned()),
            ws.config.source_locale.clone(),
            ws.glossary.clone(),
        )
    };
    let task = match r.instruction.as_str() {
        "" | "variants" => "Propose better alternative wordings.",
        "shorten" => "Make it noticeably shorter (it may overflow its UI container), same meaning.",
        "formal" => "Make the tone more formal.",
        "casual" => "Make the tone more casual and friendly.",
        other => other,
    };
    let payload = json!({
        "task": "suggest",
        "instruction": task,
        "source_locale": src_locale,
        "target_locale": r.locale,
        "style": cfg.llm.style,
        "note": note,
        "items": [{"key": "current", "source": current}],
        "original_source": source,
    });
    let system = format!(
        "You are a UI copywriter and localizer. Rewrite the UI string in locale \"{}\". {task} \
         Keep every placeholder ({{name}}, ICU plural/select blocks, <tags>) exactly. \
         Answer with JSON only: {{\"suggestions\": [\"...\", \"...\", \"...\"]}} (3 options).",
        r.locale
    );
    let user = format!(
        "<payload>\n{}\n</payload>",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    let p = provider::from_config(&cfg.llm, None).map_err(bad)?;
    let out = p
        .complete(CompletionRequest {
            model: cfg.llm.model.clone(),
            system,
            user,
            max_tokens: 600,
            json: true,
            images: vec![],
        })
        .await
        .map_err(bad)?;
    let v: Value = serde_json::from_str(json_slice(&out.text)).unwrap_or_default();
    // Real models answer {"suggestions":[..]}; the mock provider answers {"translations":{..}}.
    let mut texts: Vec<String> = v["suggestions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .or_else(|| {
            v["translations"].as_object().map(|o| {
                o.values()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default();
    texts.dedup();
    // Deterministic guard: placeholders/glossary checked against the source; errors dropped.
    let c = Constraints {
        max_length: None,
        max_length_ratio: Some(cfg.qa.max_length_ratio),
    };
    let checked: Vec<Value> = texts
        .into_iter()
        .filter_map(|t| {
            let reference = source.as_deref().unwrap_or(&current);
            let mut issues = if r.locale == src_locale {
                validate::validate(
                    reference,
                    &t,
                    &r.locale,
                    cfg.syntax,
                    &Constraints::default(),
                )
                .into_iter()
                .filter(|i| i.code != validate::IssueCode::Untranslated)
                .collect()
            } else {
                validate::validate(reference, &t, &r.locale, cfg.syntax, &c)
            };
            if r.locale != src_locale {
                issues.extend(glossary.check(reference, &t, &r.locale));
            }
            (!validate::has_errors(&issues)).then(|| json!({"value": t, "issues": issues}))
        })
        .collect();
    Ok(Json(json!({"suggestions": checked, "model": out.model})))
}

fn json_slice(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => s,
    }
}

// ---------- events ----------

async fn api_events(
    State(s): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = s.events.subscribe();
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(msg) => return Some((Ok(Event::default().event("change").data(msg)), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn watched(ws: &Workspace) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ws
        .config
        .all_locales()
        .iter()
        .flat_map(|l| ws.config.catalog_paths(&ws.root, l))
        .collect();
    for f in ["glot.toml", "glot.lock", "glossary.toml"] {
        v.push(ws.root.join(f));
    }
    v
}

fn stamp(paths: &[PathBuf]) -> Vec<Option<SystemTime>> {
    paths
        .iter()
        .map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .collect()
}

/// Poll mtimes, reload the workspace and broadcast on change (our own writes included).
// ponytail: 500 ms mtime polling instead of the `notify` crate; switch if it ever matters.
pub fn spawn_watcher(s: AppState) {
    tokio::spawn(async move {
        let mut paths = watched(&*s.ws.lock().await);
        let mut last = stamp(&paths);
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let now = stamp(&paths);
            if now == last {
                continue;
            }
            let mut ws = s.ws.lock().await;
            let msg = match ws.reload() {
                Ok(()) => json!({"ok": true}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            };
            paths = watched(&ws);
            last = stamp(&paths);
            let _ = s.events.send(msg.to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        assert_eq!(match_score("save changes", "Save changes"), 100);
        assert_eq!(match_score("hello ana!", "Hello {name}!"), 90);
        assert_eq!(match_score("hello ana", "Hello {name}!"), 0);
        assert_eq!(
            match_score(
                "you have 3 items in your cart",
                "You have {count, plural, one {# item} other {# items}} in your cart"
            ),
            90
        );
        assert_eq!(
            match_score("read the docs now", "Read the <b>docs</b> now"),
            100
        );
        assert_eq!(match_score("anything", "{count}"), 0);
        assert_eq!(match_score("welcome back, ana — 3 new", "Welcome back"), 50);
        assert!(is_local_origin("http://localhost:4000"));
        assert!(!is_local_origin("https://evil.com"));
        assert!(!is_local_origin("http://localhost.evil.com:80"));
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let w = |p: &str, s: &str| {
            let p = dir.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        };
        w(
            "glot.toml",
            "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
        );
        w(
            "messages/en.json",
            r#"{"home":{"title":"Welcome back","greet":"Hello {name}!"}}"#,
        );
        w("messages/fr.json", r#"{"home":{"title":"Bon retour"}}"#);
        dir
    }

    async fn serve(dir: &tempfile::TempDir) -> (String, AppState) {
        let state = new_state(Workspace::open(dir.path()).unwrap(), "secret".into());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let app = router(state.clone(), &[]);
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (url, state)
    }

    #[tokio::test]
    async fn server_end_to_end() {
        let dir = project();
        let (url, _) = serve(&dir).await;
        let c = reqwest::Client::new();
        let post = |path: &str, body: Value| {
            c.post(format!("{url}{path}"))
                .bearer_auth("secret")
                .json(&body)
                .send()
        };

        // auth
        let r = c
            .post(format!("{url}/api/lookup"))
            .json(&json!({"text": "x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        let r = c
            .post(format!("{url}/api/lookup"))
            .bearer_auth("wrong")
            .json(&json!({"text": "x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);

        // lookup in source and target locales, placeholder tolerant
        let v: Value = post("/api/lookup", json!({"text": "Hello Ana!"}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["matches"][0]["key"], "home.greet");
        let v: Value = post("/api/lookup", json!({"text": " bon  RETOUR "}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["matches"][0]["key"], "home.title");
        assert_eq!(v["matches"][0]["locale"], "fr");

        // validated write: missing placeholder rejected, nothing written
        let v: Value = post(
            "/api/set",
            json!({"key": "home.greet", "locale": "fr", "value": "Bonjour !"}),
        )
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(v["ok"], false);
        let fr = std::fs::read_to_string(dir.path().join("messages/fr.json")).unwrap();
        assert!(!fr.contains("Bonjour"));
        // valid write lands in the file
        let v: Value = post(
            "/api/set",
            json!({"key": "home.greet", "locale": "fr", "value": "Bonjour {name} !"}),
        )
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(v["ok"], true);
        let fr = std::fs::read_to_string(dir.path().join("messages/fr.json")).unwrap();
        assert!(fr.contains("Bonjour {name} !"));

        // key details
        let v: Value = c
            .get(format!("{url}/api/key?key=home.greet"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["values"]["fr"], "Bonjour {name} !");
        assert_eq!(v["values"]["en"], "Hello {name}!");

        // extract hardcoded text to a key
        let p: Value = post("/api/propose", json!({"text": "Start free trial"}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(p["key"], "start_free_trial");
        let r = post(
            "/api/extract",
            json!({"key": "home.cta", "value": "Start free trial"}),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), 200);
        let r = post("/api/extract", json!({"key": "home.cta", "value": "dup"}))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);

        // translate everywhere (mock) + AI suggestions (mock, validated)
        let v: Value = post("/api/translate", json!({"key": "home.cta"}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(v["written"]["fr"].is_array(), "{v}");
        let fr = std::fs::read_to_string(dir.path().join("messages/fr.json")).unwrap();
        assert!(fr.contains("⟦fr⟧ Start free trial"));
        let v: Value = post(
            "/api/suggest",
            json!({"key": "home.greet", "locale": "fr", "instruction": "shorten"}),
        )
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert!(
            v["suggestions"][0]["value"]
                .as_str()
                .unwrap()
                .contains("{name}")
        );
    }

    #[tokio::test]
    async fn watcher_broadcasts_external_changes() {
        let dir = project();
        let (_, state) = serve(&dir).await;
        let mut rx = state.events.subscribe();
        spawn_watcher(state.clone());
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::write(
            dir.path().join("messages/fr.json"),
            r#"{"home":{"title":"Re-bonjour"}}"#,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let ws = state.ws.lock().await;
        assert_eq!(ws.catalogs["fr"].entries["home.title"], "Re-bonjour");
    }
}
