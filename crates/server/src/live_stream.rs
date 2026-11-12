//! Live stream translation: text that exists only once it is generated or typed (an AI answer, a notification,
//! a chat message) is translated while it streams. The engine is `glot_llm::live_stream`; this module is the
//! HTTP surface, the project's opt-in, what is remembered, and the counters.
//!
//! `POST /api/projects/{slug}/live-stream/translate` (API token or dashboard session) and
//! `POST /live/v1/{slug}/translate` (public key `pk_…`, browsers; needs `browser`) take
//! `{"to": "es", "text": "…", "final": false, "conversation": "c1", "fence": null, "mid": false}` and answer a
//! Server-Sent Events stream: `delta` events (`{"text", "n", "via", "reason"?}`, final, in order, never
//! retracted) then one `done` (`{"consumed", "fence", "mid", "stats"}`).
//!
//! The server keeps no stream state: the caller sends what it has, the server translates the stable part and says
//! how much it consumed (`consumed`, UTF-16 units) and where it stopped (`fence`, `mid`); the caller sends the rest
//! with the next text. Any HTTP client can do that (browsers cannot stream a request body and read the answer at
//! the same time, and a WebSocket would pin a conversation to one replica), one request is one batch, and closing
//! the response cancels the provider calls in flight.
//!
//! Privacy: off until an admin opts in. What goes to the LLM provider: the units to translate with code, links,
//! do-not-translate terms and (unless `mask_pii` is off) emails and digit runs replaced by placeholders, the
//! glossary terms that occur, and the last source units of the conversation (masked the same way). What is
//! stored: a keyed hash and a hit counter per distinct unit, daily counters; the masked translation only with
//! `store`. A conversation lives in memory for 30 minutes (masked text), never on disk.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use glot_llm::live_stream::{Delta, Lookup, Options, Segmenter, Via};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppState,
    auth::AuthUser,
    projects::{self, ProjectRow, Role},
    validate_locale,
};

/// Longest `text` of one request (the caller's unconsumed tail + what arrived since).
const MAX_TEXT: usize = 32_000;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(8);
/// After a provider failure the project's streams pass through for this long instead of waiting on it again.
const BREAKER: Duration = Duration::from_secs(20);
const CONVERSATION_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Settings {
    pub enabled: bool,
    /// Accept the public key from browsers (`/live/v1/…`).
    pub browser: bool,
    pub mask_pii: bool,
    /// Keep masked translations so repeated sentences are instant and free.
    pub store: bool,
    pub model: Option<String>,
    pub daily_chars: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            browser: false,
            mask_pii: true,
            store: false,
            model: None,
            daily_chars: 500_000,
        }
    }
}

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/live-stream";
    Router::new()
        .route(p, get(status).put(put_settings))
        .route(&format!("{p}/translate"), post(translate))
        .route(&format!("{p}/cache"), axum::routing::delete(purge))
}

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/live/v1/{project}/translate", post(translate_public))
        .layer(DefaultBodyLimit::max(4 * MAX_TEXT))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::POST])
                .allow_headers([header::CONTENT_TYPE]),
        )
}

async fn settings(state: &AppState, project: Uuid) -> ApiResult<Settings> {
    Ok(sqlx::query_as(
        "SELECT enabled, browser, mask_pii, store, model, daily_chars FROM live_stream_settings WHERE project_id = $1",
    )
    .bind(project)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or_default())
}

/// Which provider would translate, and whether it can (never contains the key).
fn provider_state(state: &AppState, p: &ProjectRow) -> serde_json::Value {
    let name = p.llm.0.provider.as_str();
    let error = crate::feedback::provider(state, p).err().map(|e| e.message);
    json!({
        "name": name,
        "model": p.llm.0.model,
        // The mock provider answers `⟦es⟧ text`: useful for tests, not a translation.
        "ready": error.is_none() && name != "mock",
        "error": error,
    })
}

async fn status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days: Vec<(chrono::NaiveDate, i64, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT day, requests, chars, units, llm_units, known_units, failed_units, first_ms FROM live_stream_usage
         WHERE project_id = $1 AND day > current_date - 30 ORDER BY day",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let (distinct, repeats, stored): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(hits - 1), 0)::int8, count(target) FROM live_stream_units WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    let terms: i64 =
        sqlx::query_scalar("SELECT count(*) FROM glossary_terms WHERE project_id = $1")
            .bind(p.id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(json!({
        "settings": settings(&state, p.id).await?,
        "public_key": crate::insights::public_key(&state, p.id).await?,
        "provider": provider_state(&state, &p),
        "glossary_terms": terms,
        "units": { "distinct": distinct, "repeats": repeats, "stored": stored },
        "days": days.iter().map(|d| json!({
            "day": d.0, "requests": d.1, "chars": d.2, "units": d.3, "llm_units": d.4,
            "known_units": d.5, "failed_units": d.6,
            // Mean time from request to first translated unit, over requests that translated something.
            "first_ms": d.7,
        })).collect::<Vec<_>>(),
    })))
}

async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Settings>,
) -> ApiResult<Json<Settings>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    if !(0..=50_000_000).contains(&b.daily_chars) {
        return Err(AppError::bad_request("daily_chars must be 0-50000000"));
    }
    let model = b.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    if model.is_some_and(|m| m.len() > 100) {
        return Err(AppError::bad_request("model is too long"));
    }
    sqlx::query(
        "INSERT INTO live_stream_settings (project_id, enabled, browser, mask_pii, store, model, daily_chars)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (project_id) DO UPDATE SET enabled = $2, browser = $3, mask_pii = $4, store = $5, model = $6,
           daily_chars = $7, updated_at = now()",
    )
    .bind(p.id)
    .bind(b.enabled)
    .bind(b.browser)
    .bind(b.mask_pii)
    .bind(b.store)
    .bind(model)
    .bind(b.daily_chars)
    .execute(&state.db)
    .await?;
    // Turning storage off also forgets what was stored.
    if !b.store {
        sqlx::query("UPDATE live_stream_units SET target = NULL WHERE project_id = $1 AND target IS NOT NULL")
            .bind(p.id)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(settings(&state, p.id).await?))
}

/// Forget every unit (hashes, counters and stored translations) and the conversations in memory.
async fn purge(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let n = sqlx::query("DELETE FROM live_stream_units WHERE project_id = $1")
        .bind(p.id)
        .execute(&state.db)
        .await?
        .rows_affected();
    let prefix = format!("{}:", p.id);
    CONVERSATIONS
        .lock()
        .unwrap()
        .retain(|k, _| !k.starts_with(&prefix));
    Ok(Json(json!({ "deleted": n })))
}

#[derive(Debug, Deserialize)]
pub struct Body {
    /// Target locale.
    pub to: String,
    /// Source locale; absent = detected by the model.
    #[serde(default)]
    pub from: Option<String>,
    /// The unconsumed tail of the stream so far.
    pub text: String,
    /// The stream is over: translate the unfinished sentence too.
    #[serde(default, rename = "final")]
    pub last: bool,
    /// Opaque id shared by the turns of one conversation: repeated sentences and wording stay consistent.
    #[serde(default)]
    pub conversation: Option<String>,
    /// `fence` / `mid` of the previous `done` event.
    #[serde(default)]
    pub fence: Option<String>,
    #[serde(default)]
    pub mid: bool,
    #[serde(default)]
    pub key: Option<String>,
}

async fn translate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Body>,
) -> ApiResult<Response> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let s = settings(&state, p.id).await?;
    serve(state, p, s, b).await
}

/// Browsers: any content type (no CORS preflight for `text/plain`), public key in the body or `X-Glot-Key`.
async fn translate_public(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let b: Body = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?;
    let key = b
        .key
        .clone()
        .or_else(|| {
            headers
                .get("x-glot-key")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        })
        .ok_or_else(|| AppError::bad_request("missing public key (`key` or X-Glot-Key)"))?;
    let p = projects::load_public(&state, &slug)
        .await?
        .ok_or_else(|| AppError::not_found("unknown project or public key"))?;
    let known: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM insight_keys WHERE project_id = $1 AND public_key = $2")
            .bind(p.id)
            .bind(&key)
            .fetch_optional(&state.db)
            .await?;
    if known.is_none() {
        return Err(AppError::not_found("unknown project or public key"));
    }
    let s = settings(&state, p.id).await?;
    if s.enabled && !s.browser {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "live_stream_browser_disabled",
            "live stream translation from browsers is off for this project: call it from your server with an API token, or enable \"Browser access\" in the dashboard (Live stream)",
        ));
    }
    serve(state, p, s, b).await
}

async fn serve(state: AppState, p: ProjectRow, s: Settings, b: Body) -> ApiResult<Response> {
    if !s.enabled {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "live_stream_disabled",
            "live stream translation is off for this project: an admin enables it in the dashboard (Live stream), after reading what is sent to the LLM provider",
        ));
    }
    validate_locale(&b.to)?;
    if let Some(f) = &b.from {
        validate_locale(f)?;
    }
    if b.text.len() > MAX_TEXT {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "text_too_long",
            format!("at most {MAX_TEXT} bytes of text per request: send the stream as it arrives"),
        ));
    }
    if b.conversation.as_ref().is_some_and(|c| c.len() > 100)
        || b.fence
            .as_ref()
            .is_some_and(|f| f.len() > 16 || !f.chars().all(|c| c == '`' || c == '~'))
    {
        return Err(AppError::bad_request("invalid `conversation` or `fence`"));
    }
    let today: Option<i64> = sqlx::query_scalar(
        "SELECT chars FROM live_stream_usage WHERE project_id = $1 AND day = current_date",
    )
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?;
    if today.unwrap_or(0) >= s.daily_chars as i64 {
        return Err(AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "live_stream_daily_cap",
            format!(
                "this project's daily live stream cap ({} characters) is reached: raise it in the dashboard (Live stream)",
                s.daily_chars
            ),
        ));
    }
    crate::billing::check_llm(&state, &p).await?;

    let mut seg = Segmenter::resume(b.fence.clone().filter(|f| !f.is_empty()), b.mid);
    let mut pieces = seg.push(&b.text);
    if b.last {
        pieces.extend(seg.finish());
    }
    let n16 = |t: &str| t.encode_utf16().count();
    let consumed = n16(&b.text) - n16(seg.pending());

    // No usable provider: what the project already knows (memory, cache) still applies, the rest passes through.
    let (provider, mut skip): (Arc<dyn glot_llm::provider::Provider>, _) =
        match crate::feedback::provider(&state, &p) {
            Ok(pv) => (Arc::from(pv), None),
            Err(_) => (Arc::new(glot_llm::mock::MockProvider), Some("no_provider")),
        };
    if BROKEN
        .lock()
        .unwrap()
        .get(&p.id)
        .is_some_and(|until| Instant::now() < *until)
    {
        skip = skip.or(Some("provider_down"));
    }
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let model = s.model.clone().unwrap_or_else(|| p.llm.0.model.clone());
    // What a cached translation depends on besides the text.
    let fingerprint = crate::auth::sha256_hex(&format!(
        "{}|{:?}|{:?}|{model}",
        serde_json::to_string(&glossary).unwrap_or_default(),
        p.style,
        b.from
    ));
    let opts = Arc::new(Options {
        source_locale: b.from.clone(),
        target_locale: b.to.clone(),
        model,
        style: p.style.clone(),
        glossary,
        mask_pii: s.mask_pii,
        timeout: PROVIDER_TIMEOUT,
        skip_provider: skip,
    });
    let memory = Arc::new(Memory {
        state: state.clone(),
        project: p.id,
        locale: b.to.clone(),
        salt: format!("{}|{}|{fingerprint}|", hex::encode(state.secret), p.id),
        store: s.store,
        conversation: b
            .conversation
            .as_ref()
            .map(|c| format!("{}:{}:{c}", p.id, b.to)),
        local: Mutex::default(),
    });
    let tally = Tally {
        state,
        project: p.id,
        started: Instant::now(),
        chars: consumed as i64,
        counts: Default::default(),
    };
    // The engine runs in its own task and stops as soon as the response is dropped (the caller left): the
    // provider calls in flight are cancelled, the counters are written by `Tally::drop`.
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(16);
    tokio::spawn(async move {
        let mut deltas = std::pin::pin!(glot_llm::live_stream::run(provider, memory, opts, pieces));
        while let Some(d) = deltas.next().await {
            tally.count(&d);
            let Ok(e) = Event::default().event("delta").json_data(&d) else {
                continue;
            };
            if tx.send(e).await.is_err() {
                return;
            }
        }
        let done = json!({ "consumed": consumed, "fence": seg.fence, "mid": seg.mid, "stats": tally.stats() });
        if let Ok(e) = Event::default().event("done").json_data(done) {
            let _ = tx.send(e).await;
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx).map(Ok::<_, Infallible>)).into_response())
}

// ---------- counters ----------

static BROKEN: LazyLock<Mutex<HashMap<Uuid, Instant>>> = LazyLock::new(Default::default);

/// Per-request counters, written when the response ends: completed or dropped by the caller.
struct Tally {
    state: AppState,
    project: Uuid,
    started: Instant,
    chars: i64,
    /// units, llm, known, failed, first_ms (-1 = nothing translated yet), provider failures
    counts: [AtomicI64; 6],
}

impl Tally {
    fn count(&self, d: &Delta) {
        let add = |i: usize| self.counts[i].fetch_add(1, Ordering::Relaxed);
        match d.via {
            Via::Raw => return,
            Via::Llm => add(1),
            Via::Cache | Via::Tm => add(2),
            Via::Source => add(3),
        };
        if add(0) == 0 {
            self.counts[4].store(self.started.elapsed().as_millis() as i64, Ordering::Relaxed);
        }
        if matches!(d.reason, Some("provider_error" | "provider_timeout")) {
            add(5);
        }
    }
    fn get(&self, i: usize) -> i64 {
        self.counts[i].load(Ordering::Relaxed)
    }
    fn stats(&self) -> serde_json::Value {
        json!({
            "units": self.get(0), "llm": self.get(1), "known": self.get(2), "failed": self.get(3),
            "first_ms": self.get(4), "ms": self.started.elapsed().as_millis() as u64,
        })
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        let (state, project, chars) = (self.state.clone(), self.project, self.chars);
        let c: Vec<i64> = (0..6).map(|i| self.get(i)).collect();
        if c[5] > 0 {
            BROKEN
                .lock()
                .unwrap()
                .insert(project, Instant::now() + BREAKER);
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        rt.spawn(async move {
            let r = sqlx::query(
                "INSERT INTO live_stream_usage AS u (project_id, day, requests, chars, units, llm_units, known_units, failed_units, first_ms)
                 VALUES ($1, current_date, 1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (project_id, day) DO UPDATE SET requests = u.requests + 1, chars = u.chars + excluded.chars,
                   units = u.units + excluded.units, llm_units = u.llm_units + excluded.llm_units,
                   known_units = u.known_units + excluded.known_units, failed_units = u.failed_units + excluded.failed_units,
                   first_ms = u.first_ms + excluded.first_ms",
            )
            .bind(project)
            .bind(chars)
            .bind(c[0])
            .bind(c[1])
            .bind(c[2])
            .bind(c[3])
            .bind(if c[0] > 0 { c[4] } else { 0 })
            .execute(&state.db)
            .await;
            if let Err(e) = r {
                tracing::error!("live stream usage: {e}");
            }
            // Units translated on glot's own key count like catalog strings.
            crate::billing::after_job(&state, project, Uuid::new_v4(), c[1]).await;
        });
    }
}

// ---------- memory ----------

#[derive(Default)]
struct Conversation {
    /// masked source → masked target
    pairs: HashMap<String, String>,
    /// last source units (masked), oldest first: the provider's context
    recent: VecDeque<String>,
    at: Option<Instant>,
}

// ponytail: per-process map (a second replica just has less context and re-translates); move to Postgres/Redis
// with the same TTL when the API runs on several replicas.
static CONVERSATIONS: LazyLock<Mutex<HashMap<String, Conversation>>> =
    LazyLock::new(Default::default);

/// What is already known about a unit, best first: this conversation, the project's translation memory
/// (catalog and glot Edge segments, exact match), the stored live units.
struct Memory {
    state: AppState,
    project: Uuid,
    locale: String,
    salt: String,
    store: bool,
    conversation: Option<String>,
    /// The conversation of a request that named none: this request only.
    local: Mutex<Conversation>,
}

impl Memory {
    fn with<T>(&self, f: impl FnOnce(&mut Conversation) -> T) -> T {
        let Some(key) = &self.conversation else {
            return f(&mut self.local.lock().unwrap());
        };
        let mut all = CONVERSATIONS.lock().unwrap();
        if all.len() > 5000 {
            all.retain(|_, c| c.at.is_some_and(|t| t.elapsed() < CONVERSATION_TTL));
        }
        let c = all.entry(key.clone()).or_default();
        if c.at.is_some_and(|t| t.elapsed() > CONVERSATION_TTL) {
            *c = Conversation::default();
        }
        c.at = Some(Instant::now());
        f(c)
    }
    fn hash(&self, unit: &str) -> String {
        crate::auth::sha256_hex(&format!("{}{unit}", self.salt))
    }
    async fn known(&self, units: &[String]) -> sqlx::Result<HashMap<String, (String, Via)>> {
        let hashes: Vec<String> = units.iter().map(|u| self.hash(u)).collect();
        // Every unit seen is counted (keyed hash only): the dashboard's repeat rate.
        sqlx::query(
            "INSERT INTO live_stream_units AS u (project_id, locale, hash) SELECT $1, $2, h FROM unnest($3::text[]) h
             ON CONFLICT (project_id, locale, hash) DO UPDATE SET hits = u.hits + 1, last_seen = now()",
        )
        .bind(self.project)
        .bind(&self.locale)
        .bind(&hashes)
        .execute(&self.state.db)
        .await?;
        let mut out = HashMap::new();
        let stored: Vec<(String, String)> = sqlx::query_as(
            "SELECT hash, target FROM live_stream_units
             WHERE project_id = $1 AND locale = $2 AND hash = ANY($3) AND target IS NOT NULL",
        )
        .bind(self.project)
        .bind(&self.locale)
        .bind(&hashes)
        .fetch_all(&self.state.db)
        .await?;
        for (u, h) in units.iter().zip(&hashes) {
            if let Some((_, t)) = stored.iter().find(|s| &s.0 == h) {
                out.insert(u.clone(), (t.clone(), Via::Cache));
            }
        }
        // Reviewed and machine translations the project already ships; they win over the live cache.
        let tm: Vec<(String, String)> = sqlx::query_as(
            "SELECT k.source, t.value FROM keys k JOIN translations t ON t.key_id = k.id
             WHERE k.project_id = $1 AND t.locale = $2 AND t.status <> 'stale' AND t.value <> '' AND k.source = ANY($3)
             UNION ALL
             SELECT s.source_text, t.value FROM edge_segments s
             JOIN edge_translations t ON t.project_id = s.project_id AND t.hash = s.hash AND t.context = s.context
             WHERE s.project_id = $1 AND t.locale = $2 AND t.status IN ('machine', 'reviewed') AND t.value <> ''
               AND s.source_text = ANY($3)",
        )
        .bind(self.project)
        .bind(&self.locale)
        .bind(units)
        .fetch_all(&self.state.db)
        .await?;
        for (source, value) in tm.into_iter().rev() {
            out.insert(source, (value, Via::Tm));
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl Lookup for Memory {
    async fn get(&self, units: &[String]) -> Vec<Option<(String, Via)>> {
        let mut out: Vec<Option<(String, Via)>> = self.with(|c| {
            units
                .iter()
                .map(|u| c.pairs.get(u).map(|t| (t.clone(), Via::Cache)))
                .collect()
        });
        let missing: Vec<String> = units
            .iter()
            .zip(&out)
            .filter(|(_, o)| o.is_none())
            .map(|(u, _)| u.clone())
            .collect();
        if !missing.is_empty() {
            match self.known(&missing).await {
                Ok(known) => {
                    for (u, o) in units.iter().zip(out.iter_mut()) {
                        if o.is_none() {
                            *o = known.get(u).cloned();
                        }
                    }
                }
                // The memory is an optimisation: without it the provider translates.
                Err(e) => tracing::error!("live stream memory: {e}"),
            }
        }
        self.with(|c| {
            for (u, o) in units.iter().zip(&out) {
                if let Some((t, _)) = o {
                    c.pairs.insert(u.clone(), t.clone());
                    remember(c, u);
                }
            }
        });
        out
    }

    async fn put(&self, pairs: &[(String, String)]) {
        self.with(|c| {
            for (source, target) in pairs {
                if c.pairs.len() < 500 {
                    c.pairs.insert(source.clone(), target.clone());
                }
                remember(c, source);
            }
        });
        if !self.store || pairs.is_empty() {
            return;
        }
        let (hashes, targets): (Vec<String>, Vec<&str>) = pairs
            .iter()
            .map(|(s, t)| (self.hash(s), t.as_str()))
            .unzip();
        let r = sqlx::query(
            "UPDATE live_stream_units u SET target = x.t FROM unnest($3::text[], $4::text[]) AS x(h, t)
             WHERE u.project_id = $1 AND u.locale = $2 AND u.hash = x.h",
        )
        .bind(self.project)
        .bind(&self.locale)
        .bind(&hashes)
        .bind(&targets)
        .execute(&self.state.db)
        .await;
        if let Err(e) = r {
            tracing::error!("live stream cache: {e}");
        }
    }

    fn context(&self) -> Vec<String> {
        self.with(|c| c.recent.iter().cloned().collect())
    }
}

fn remember(c: &mut Conversation, unit: &str) {
    if c.recent.back().is_some_and(|l| l == unit) {
        return;
    }
    c.recent.push_back(unit.to_string());
    if c.recent.len() > 4 {
        c.recent.pop_front();
    }
}
