//! Two-way translation: what customers write back (ticket, form, chat, email) is read in the team's language
//! and answered in the customer's, with the project's glossary and style, the original always kept.
//!
//! - Core (`inbound`, `outbound`): local language detection (`detect`) → PII masked (`pii`) → the guarded
//!   translation pipeline (`glot_llm::translate`: placeholders enforced, glossary checked, retries) → PII
//!   restored. An outbound reply is also translated back so the agent can check what the customer will read,
//!   and is flagged for review when the two disagree.
//! - Nothing reaches a customer unless the project's mode allows it: `suggest` (glot never sends),
//!   `one_click` (a person sends each reply), `auto` (sent unless flagged).
//! - Channels: signed webhook + REST (any tool), contact-form helper (public, stores nothing by default),
//!   inbound email (parse → translate → forward), Help Scout conversations (`helpscout`).
//!
//! Design: docs/moonshots/two-way.md.
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use glot_core::{
    context::KeyContext,
    glossary::{Glossary, Term},
    validate::{Severity, Syntax},
};
use glot_llm::translate::{Item, Request, Translator};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::types::Json as DbJson;
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::{AuthUser, decrypt, encrypt},
    projects::{self, ProjectRow, Role},
    validate_locale,
};

pub mod detect;
pub mod helpscout;
pub mod pii;

use pii::Masker;

const MAX_TEXT: usize = 20_000;
const MAX_FORM_TEXT: usize = 5_000;
/// Previous messages given to the model for consistency (masked like the message itself).
const THREAD_CONTEXT: i64 = 4;

// ------------------------------------------------------------------------------------------------ core

/// What one translation needs from a project. Built by `engine` for a project; tests build it around
/// their own provider.
pub struct Engine {
    pub translator: Translator,
    pub glossary: Glossary,
    /// The project's source locale: the language the glossary's `term` column is written in.
    pub project_source: String,
    pub style: Option<String>,
}

/// Primary language subtag, lowercased: `pt-BR` → `pt`.
pub fn lang(locale: &str) -> String {
    locale.split(['-', '_']).next().unwrap_or("").to_lowercase()
}

fn same_lang(a: &str, b: &str) -> bool {
    lang(a) == lang(b)
}

/// The project glossary re-pivoted for a `from` → `to` pair that may not involve the project's source
/// locale at all (a Japanese customer, a French team, an English source). Only terms found in `text`.
pub fn pivot_glossary(
    g: &Glossary,
    project_source: &str,
    from: &str,
    to: &str,
    text: &str,
) -> Vec<Term> {
    let form = |t: &Term, l: &str| -> Option<String> {
        if t.do_not_translate || same_lang(l, project_source) {
            return Some(t.term.clone());
        }
        t.translations
            .get(l)
            .or_else(|| {
                t.translations
                    .iter()
                    .find(|(k, _)| same_lang(k, l))
                    .map(|(_, v)| v)
            })
            .cloned()
    };
    let lower = text.to_lowercase();
    g.terms
        .iter()
        .filter_map(|t| {
            let (s, d) = (form(t, from)?, form(t, to)?);
            let found = if t.case_sensitive {
                contains_term(text, &s)
            } else {
                contains_term(&lower, &s.to_lowercase())
            };
            found.then(|| Term {
                term: s,
                description: t.description.clone(),
                do_not_translate: t.do_not_translate,
                case_sensitive: t.case_sensitive,
                translations: [(to.to_string(), d)].into(),
                forbidden: t
                    .forbidden
                    .iter()
                    .filter(|(k, _)| same_lang(k, to))
                    .map(|(_, v)| (to.to_string(), v.clone()))
                    .collect(),
            })
        })
        .collect()
}

/// `needle` in `hay` as a whole word: "clip" is not in "ClipOnIt". Scripts written without spaces (CJK, kana,
/// Thai) have no word boundaries: there, any occurrence counts. ponytail: in those scripts a short term can
/// still match inside a longer word; a segmenter fixes it if it shows up.
pub fn contains_term(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let spaceless = |c: char| matches!(c as u32, 0x0E00..=0x0E7F | 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF);
    let word = |c: char| c.is_alphanumeric() && !spaceless(c);
    hay.match_indices(needle).any(|(i, m)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + m.len()..].chars().next();
        let first = needle.chars().next().is_some_and(spaceless);
        let last = needle.chars().next_back().is_some_and(spaceless);
        (first || !before.is_some_and(word)) && (last || !after.is_some_and(word))
    })
}

const NOTE_COMMON: &str = "This is a message in a customer conversation, not a UI string: there is no length limit, keep the \
line breaks. Placeholders such as {email1}, {phone1} or {ref1} stand for hidden personal data: keep them unchanged.";
const NOTE_IN: &str = "Written by a customer. Translate faithfully for the support team: keep the tone, the urgency, the \
emphasis and the rudeness exactly as they are. Never soften, summarize, explain or add anything.";
const NOTE_OUT: &str = "Written by a support agent, to be read by the customer. Write what a native support agent would \
write, in a natural and polite register; keep every fact, amount, date and commitment; add nothing.";
const NOTE_BACK: &str = "Back-translation used as a safety check: translate as literally as possible, do not improve or \
correct the text.";

struct Pass {
    /// Translation with the placeholders still in.
    masked: String,
    flags: Vec<String>,
}

/// One guarded translation of already-masked text.
async fn pass(
    e: &Engine,
    masked: &str,
    from: &str,
    to: &str,
    note: &str,
    thread: &[(String, String)],
) -> ApiResult<Pass> {
    let req = Request {
        source_locale: from.to_string(),
        target_locale: to.to_string(),
        items: vec![Item {
            key: "message".into(),
            source: masked.to_string(),
            context: KeyContext {
                key: "message".into(),
                description: Some(format!("{NOTE_COMMON} {note}")),
                neighbors: thread.to_vec(),
                ..Default::default()
            },
            previous: None,
        }],
        glossary: pivot_glossary(&e.glossary, &e.project_source, from, to, masked),
        // The placeholders are ICU arguments whatever the project's catalog syntax is.
        syntax: Syntax::Icu,
        style: e.style.clone(),
    };
    let (mut out, _usage) = e.translator.translate(req).await.map_err(|err| {
        AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "llm_error",
            format!("the translation model failed: {err}"),
        )
    })?;
    let t = out.pop().ok_or_else(|| {
        AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "llm_error",
            "the translation model returned nothing for this message",
        )
    })?;
    let mut flags: Vec<String> = t
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| {
            let code = serde_json::to_value(i.code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{:?}", i.code));
            format!("validation:{code}")
        })
        .collect();
    flags.dedup();
    Ok(Pass {
        masked: t.value,
        flags,
    })
}

fn restore(m: &Masker, source: &str, p: &mut Pass) -> String {
    match m.restore(source, &p.masked) {
        Ok(s) => s,
        Err(_) => {
            p.flags.push("pii_lost".into());
            m.restore_lossy(&p.masked)
        }
    }
}

fn mask_thread(m: &mut Masker, thread: &[(String, String)]) -> Vec<(String, String)> {
    thread
        .iter()
        .map(|(who, text)| {
            let short: String = text.chars().take(400).collect();
            (who.clone(), m.mask(&short))
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct Inbound {
    /// The customer's language as resolved (detection, thread, hint); `None` = unknown.
    pub locale: Option<String>,
    pub detect_confidence: f32,
    /// `None`: nothing to translate (already the team's language) or not translatable (see `flags`).
    pub translation: Option<String>,
    pub flags: Vec<String>,
    pub pii_masked: usize,
    /// The detection alone was confident: the thread's language can follow it.
    pub confident: bool,
}

/// Customer → team. `thread_locale` is the language the customer used before in this thread, `hint` what
/// the integrator knows (browser language, CRM field): both only used when detection is not confident.
#[allow(clippy::too_many_arguments)]
pub async fn inbound(
    e: &Engine,
    text: &str,
    team: &str,
    thread_locale: Option<&str>,
    hint: Option<&str>,
    min_confidence: f32,
    thread: &[(String, String)],
) -> ApiResult<Inbound> {
    let mut m = Masker::default();
    let masked = m.mask(text);
    // Detection never sees the hidden data either (an email address or a URL is Latin noise).
    let d = detect::detect(&strip_placeholders(&masked));
    let mut out = Inbound {
        detect_confidence: d.confidence,
        pii_masked: m.count(),
        confident: d.locale.is_some() && d.confidence >= min_confidence,
        ..Default::default()
    };
    out.locale = if out.confident {
        d.locale.clone()
    } else if let Some(l) = thread_locale.or(hint) {
        Some(lang(l))
    } else {
        if d.locale.is_some() {
            out.flags.push("language_uncertain".into());
        }
        d.locale.clone()
    };
    if d.mixed.len() > 1 {
        out.flags.push(format!("mixed:{}", d.mixed.join("+")));
    }
    let Some(from) = out.locale.clone() else {
        out.flags.push("untranslated:no_language".into());
        return Ok(out);
    };
    if same_lang(&from, team) && d.mixed.len() < 2 {
        return Ok(out);
    }
    let note = if d.mixed.len() > 1 {
        format!(
            "{NOTE_IN} The message mixes several languages ({}): translate all of it.",
            d.mixed.join(", ")
        )
    } else {
        NOTE_IN.to_string()
    };
    let ctx = mask_thread(&mut m, thread);
    let mut p = pass(e, &masked, &from, team, &note, &ctx).await?;
    out.translation = Some(restore(&m, &masked, &mut p));
    out.flags.extend(p.flags);
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct Outbound {
    pub translation: String,
    /// The translation translated back into the team's language (`None`: same language, nothing translated).
    pub back_translation: Option<String>,
    /// How much the back-translation agrees with what the agent wrote, 0-1.
    pub confidence: f32,
    pub flags: Vec<String>,
    pub pii_masked: usize,
    pub needs_review: bool,
}

/// Team → customer, then back, so the agent can read what the customer will read.
pub async fn outbound(
    e: &Engine,
    text: &str,
    team: &str,
    customer: &str,
    thread: &[(String, String)],
    review_below: f32,
) -> ApiResult<Outbound> {
    if same_lang(team, customer) {
        return Ok(Outbound {
            translation: text.to_string(),
            back_translation: None,
            confidence: 1.0,
            flags: vec![],
            pii_masked: 0,
            needs_review: false,
        });
    }
    let mut m = Masker::default();
    let masked = m.mask(text);
    let pii_masked = m.count();
    let ctx = mask_thread(&mut m, thread);
    let mut fwd = pass(e, &masked, team, customer, NOTE_OUT, &ctx).await?;
    let translation = restore(&m, &masked, &mut fwd);
    // Back-translate the masked translation: the hidden data does not travel a second time either.
    let back = pass(e, &fwd.masked, customer, team, NOTE_BACK, &[]).await?;
    let confidence = agreement(&masked, &back.masked);
    let mut flags = fwd.flags;
    if confidence < review_below {
        flags.push("low_confidence".into());
    }
    Ok(Outbound {
        translation,
        back_translation: Some(m.restore_lossy(&back.masked)),
        confidence,
        needs_review: !flags.is_empty(),
        flags,
        pii_masked,
    })
}

fn strip_placeholders(masked: &str) -> String {
    let mut out = String::with_capacity(masked.len());
    let mut depth = 0;
    for c in masked.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Agreement of two texts in the same language, 0-1: Dice coefficient on character trigrams (works without
/// word boundaries). ponytail: lexical, so a faithful paraphrase scores lower than it deserves; the threshold
/// is a project setting, and a judge-model score can replace this if false alarms get in the way.
pub fn agreement(a: &str, b: &str) -> f32 {
    let grams = |s: &str| -> std::collections::HashSet<[char; 3]> {
        let c: Vec<char> = s
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        c.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
    };
    let (x, y) = (grams(a), grams(b));
    if x.is_empty() || y.is_empty() {
        let norm = |s: &str| s.trim().to_lowercase();
        return if norm(a) == norm(b) { 1.0 } else { 0.0 };
    }
    2.0 * x.intersection(&y).count() as f32 / (x.len() + y.len()) as f32
}

/// The engine of a project: its model, key, glossary and style. Checks the plan's LLM allowance first.
pub async fn engine(state: &AppState, p: &ProjectRow) -> ApiResult<Engine> {
    crate::billing::check_llm(state, p).await?;
    Ok(Engine {
        translator: Translator::new(
            &crate::feedback::config(p),
            crate::feedback::provider(state, p)?,
        ),
        glossary: crate::glossary::load(&state.db, p.id).await?,
        project_source: p.source_locale.clone(),
        style: p.style.clone(),
    })
}

// ------------------------------------------------------------------------------------------------ settings

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Settings {
    pub team_locale: Option<String>,
    pub mode: String,
    pub review_below: f32,
    pub min_detect_confidence: f32,
    pub retention_days: i32,
    pub webhook_secret_enc: Option<Vec<u8>>,
    pub reply_url: Option<String>,
    pub form_enabled: bool,
    pub form_store: bool,
    pub form_per_day: i32,
    pub forward_to: Option<String>,
    pub helpscout_app_id: Option<String>,
    pub helpscout_app_secret_enc: Option<Vec<u8>>,
    pub helpscout_webhook_secret_enc: Option<Vec<u8>>,
}

impl Settings {
    pub fn team<'a>(&'a self, p: &'a ProjectRow) -> &'a str {
        self.team_locale.as_deref().unwrap_or(&p.source_locale)
    }
}

/// Settings of a project; the defaults (suggest mode, nothing public) when it never saved any.
pub async fn settings(state: &AppState, project: Uuid) -> ApiResult<Settings> {
    sqlx::query("INSERT INTO two_way_settings (project_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(project)
        .execute(&state.db)
        .await?;
    Ok(sqlx::query_as(
        "SELECT team_locale, mode, review_below, min_detect_confidence, retention_days, webhook_secret_enc, reply_url,
                form_enabled, form_store, form_per_day, forward_to, helpscout_app_id, helpscout_app_secret_enc,
                helpscout_webhook_secret_enc
         FROM two_way_settings WHERE project_id = $1",
    )
    .bind(project)
    .fetch_one(&state.db)
    .await?)
}

fn settings_view(p: &ProjectRow, s: &Settings) -> Value {
    json!({
        "team_locale": s.team(p),
        "mode": s.mode,
        "review_below": s.review_below,
        "min_detect_confidence": s.min_detect_confidence,
        "retention_days": s.retention_days,
        "reply_url": s.reply_url,
        "has_webhook_secret": s.webhook_secret_enc.is_some(),
        "form_enabled": s.form_enabled,
        "form_store": s.form_store,
        "form_per_day": s.form_per_day,
        "forward_to": s.forward_to,
        "helpscout": {
            "app_id": s.helpscout_app_id,
            "has_app_secret": s.helpscout_app_secret_enc.is_some(),
            "has_webhook_secret": s.helpscout_webhook_secret_enc.is_some(),
        },
        "paths": {
            "webhook": format!("/api/two-way/{}/webhook", p.slug),
            "email": format!("/api/two-way/{}/email", p.slug),
            "helpscout": format!("/api/two-way/{}/helpscout", p.slug),
            "form": format!("/api/two-way/{}/form", p.slug),
            "form_script": "/api/two-way/form.js",
        },
    })
}

/// Every field is optional: absent = unchanged, `""` clears a text field. Secrets are write-only.
#[derive(Debug, Default, Deserialize)]
pub struct SettingsBody {
    team_locale: Option<String>,
    mode: Option<String>,
    review_below: Option<f32>,
    min_detect_confidence: Option<f32>,
    retention_days: Option<i32>,
    reply_url: Option<String>,
    form_enabled: Option<bool>,
    form_store: Option<bool>,
    form_per_day: Option<i32>,
    forward_to: Option<String>,
    /// `true`: create (or rotate) the webhook secret; it is returned once in the response.
    #[serde(default)]
    generate_webhook_secret: bool,
    helpscout_app_id: Option<String>,
    helpscout_app_secret: Option<String>,
    helpscout_webhook_secret: Option<String>,
}

async fn get_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    Ok(Json(settings_view(&p, &settings(&state, p.id).await?)))
}

async fn put_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<SettingsBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    let mut s = settings(&state, p.id).await?;
    let text = |v: String| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    if let Some(v) = b.team_locale {
        s.team_locale = text(v);
        if let Some(l) = &s.team_locale {
            validate_locale(l)?;
        }
    }
    if let Some(v) = b.mode {
        if !["suggest", "one_click", "auto"].contains(&v.as_str()) {
            return Err(AppError::bad_request(
                "mode must be suggest, one_click or auto",
            ));
        }
        s.mode = v;
    }
    for (name, v, slot) in [
        ("review_below", b.review_below, &mut s.review_below),
        (
            "min_detect_confidence",
            b.min_detect_confidence,
            &mut s.min_detect_confidence,
        ),
    ] {
        if let Some(v) = v {
            if !(0.0..=1.0).contains(&v) {
                return Err(AppError::bad_request(format!(
                    "{name} must be between 0 and 1"
                )));
            }
            *slot = v;
        }
    }
    if let Some(v) = b.retention_days {
        if !(1..=3650).contains(&v) {
            return Err(AppError::bad_request("retention_days must be 1-3650"));
        }
        s.retention_days = v;
    }
    if let Some(v) = b.form_per_day {
        if !(0..=100_000).contains(&v) {
            return Err(AppError::bad_request("form_per_day must be 0-100000"));
        }
        s.form_per_day = v;
    }
    if let Some(v) = b.reply_url {
        s.reply_url = text(v);
        if let Some(u) = &s.reply_url {
            // Same guard as the connectors: http(s), public address only.
            crate::connectors::client_for(u)
                .await
                .map_err(|e| AppError::bad_request(format!("reply_url: {e}")))?;
        }
    }
    if let Some(v) = b.forward_to {
        s.forward_to = text(v);
        if s.forward_to.as_ref().is_some_and(|a| !a.contains('@')) {
            return Err(AppError::bad_request("forward_to must be an email address"));
        }
    }
    s.form_enabled = b.form_enabled.unwrap_or(s.form_enabled);
    s.form_store = b.form_store.unwrap_or(s.form_store);
    let mut created = None;
    if b.generate_webhook_secret {
        let secret = format!("glot_whsec_{}", hex::encode(rand::random::<[u8; 24]>()));
        s.webhook_secret_enc = Some(encrypt(&state.secret, &secret));
        created = Some(secret);
    }
    if let Some(v) = b.helpscout_app_id {
        s.helpscout_app_id = text(v);
    }
    for (v, slot) in [
        (b.helpscout_app_secret, &mut s.helpscout_app_secret_enc),
        (
            b.helpscout_webhook_secret,
            &mut s.helpscout_webhook_secret_enc,
        ),
    ] {
        if let Some(v) = v {
            *slot = text(v).map(|v| encrypt(&state.secret, &v));
        }
    }
    sqlx::query(
        "UPDATE two_way_settings SET team_locale = $2, mode = $3, review_below = $4, min_detect_confidence = $5,
                retention_days = $6, webhook_secret_enc = $7, reply_url = $8, form_enabled = $9, form_store = $10,
                form_per_day = $11, forward_to = $12, helpscout_app_id = $13, helpscout_app_secret_enc = $14,
                helpscout_webhook_secret_enc = $15, updated_at = now()
         WHERE project_id = $1",
    )
    .bind(p.id)
    .bind(&s.team_locale)
    .bind(&s.mode)
    .bind(s.review_below)
    .bind(s.min_detect_confidence)
    .bind(s.retention_days)
    .bind(&s.webhook_secret_enc)
    .bind(&s.reply_url)
    .bind(s.form_enabled)
    .bind(s.form_store)
    .bind(s.form_per_day)
    .bind(&s.forward_to)
    .bind(&s.helpscout_app_id)
    .bind(&s.helpscout_app_secret_enc)
    .bind(&s.helpscout_webhook_secret_enc)
    .execute(&state.db)
    .await?;
    let mut v = settings_view(&p, &s);
    if let Some(secret) = created {
        v["webhook_secret"] = json!(secret);
    }
    Ok(Json(v))
}

fn secret(state: &AppState, enc: &Option<Vec<u8>>, what: &str) -> ApiResult<String> {
    let enc = enc.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::NOT_FOUND,
            "not_configured",
            format!("{what} is not set for this project (Conversations → Settings)"),
        )
    })?;
    decrypt(&state.secret, enc).ok_or_else(|| {
        AppError::internal("cannot decrypt a two-way secret (GLOT_SECRET_KEY changed?)")
    })
}

// ------------------------------------------------------------------------------------------------ storage

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub direction: String,
    pub external_id: Option<String>,
    pub original: String,
    pub original_locale: Option<String>,
    pub translation: Option<String>,
    pub translation_locale: Option<String>,
    pub back_translation: Option<String>,
    pub confidence: Option<f32>,
    pub detect_confidence: Option<f32>,
    pub flags: Vec<String>,
    pub pii_masked: i32,
    pub status: String,
    pub delivered_via: Option<String>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub sent_at: Option<chrono::DateTime<chrono::Utc>>,
}

const MSG: &str = "id, conversation_id, direction, external_id, original, original_locale, translation, translation_locale,
    back_translation, confidence, detect_confidence, flags, pii_masked, status, delivered_via, error, created_at, sent_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Conversation {
    pub id: Uuid,
    pub channel: String,
    pub external_id: String,
    pub customer_locale: Option<String>,
    pub meta: DbJson<Value>,
}

async fn conversation(state: &AppState, project: Uuid, id: Uuid) -> ApiResult<Conversation> {
    sqlx::query_as(
        "SELECT id, channel, external_id, customer_locale, meta FROM two_way_conversations WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(project)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("conversation not found"))
}

/// The last messages of a thread on one side of the language pair: `customer_side` = what the customer
/// wrote and read, else what the team read and wrote.
async fn thread(
    state: &AppState,
    conversation: Uuid,
    customer_side: bool,
) -> ApiResult<Vec<(String, String)>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT direction, original, translation FROM two_way_messages WHERE conversation_id = $1
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(conversation)
    .bind(THREAD_CONTEXT)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .rev()
        .map(|(dir, original, translation)| {
            let inbound = dir == "in";
            let text = if inbound == customer_side {
                original
            } else {
                translation.unwrap_or(original)
            };
            (if inbound { "customer" } else { "agent" }.to_string(), text)
        })
        .collect())
}

async fn count_language(state: &AppState, project: Uuid, channel: &str, locale: Option<&str>) {
    let _ = sqlx::query(
        "INSERT INTO two_way_lang_stats (project_id, day, channel, locale, messages) VALUES ($1, current_date, $2, $3, 1)
         ON CONFLICT (project_id, day, channel, locale) DO UPDATE SET messages = two_way_lang_stats.messages + 1",
    )
    .bind(project)
    .bind(channel)
    .bind(locale.unwrap_or("und"))
    .execute(&state.db)
    .await;
}

fn check_text(text: &str, max: usize) -> ApiResult<()> {
    if text.trim().is_empty() {
        return Err(AppError::bad_request("text is empty"));
    }
    if text.chars().count() > max {
        return Err(AppError::bad_request(format!(
            "text is longer than {max} characters"
        )));
    }
    Ok(())
}

fn hint(h: Option<&str>) -> Option<String> {
    h.map(lang).filter(|l| validate_locale(l).is_ok())
}

/// A message received from a customer, whatever the channel.
#[derive(Debug, Default, Deserialize)]
pub struct Incoming {
    /// The thread's id in the calling tool (ticket id, chat id): any opaque string.
    pub conversation_id: String,
    /// The message's id in the calling tool: a repeated delivery is answered with the first result.
    #[serde(default)]
    pub message_id: Option<String>,
    pub text: String,
    /// What the caller knows about the customer's language; only used when detection is unsure.
    #[serde(default)]
    pub lang_hint: Option<String>,
    #[serde(default)]
    pub customer_name: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(skip)]
    pub meta: Value,
}

/// Store + translate one inbound message. `(message, new)`: `new = false` for a repeated `message_id`.
/// A model failure never loses the message: it is stored untranslated with the reason.
pub async fn receive(
    state: &AppState,
    p: &ProjectRow,
    s: &Settings,
    channel: &str,
    m: Incoming,
) -> ApiResult<(Message, bool)> {
    check_text(&m.text, MAX_TEXT)?;
    if m.conversation_id.trim().is_empty() || m.conversation_id.len() > 200 {
        return Err(AppError::bad_request(
            "conversation_id is required (max 200 chars)",
        ));
    }
    sqlx::query(
        "DELETE FROM two_way_conversations WHERE project_id = $1 AND last_message_at < now() - make_interval(days => $2)",
    )
    .bind(p.id)
    .bind(s.retention_days)
    .execute(&state.db)
    .await?;
    let meta = if m.meta.is_object() {
        m.meta
    } else {
        json!({})
    };
    let (conv, thread_locale): (Uuid, Option<String>) = sqlx::query_as(
        "INSERT INTO two_way_conversations (id, project_id, channel, external_id, customer_name, subject, meta)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (project_id, channel, external_id) DO UPDATE SET
            last_message_at = now(),
            customer_name = coalesce(EXCLUDED.customer_name, two_way_conversations.customer_name),
            subject = coalesce(two_way_conversations.subject, EXCLUDED.subject),
            meta = two_way_conversations.meta || EXCLUDED.meta
         RETURNING id, customer_locale",
    )
    .bind(Uuid::new_v4())
    .bind(p.id)
    .bind(channel)
    .bind(m.conversation_id.trim())
    .bind(&m.customer_name)
    .bind(&m.subject)
    .bind(DbJson(&meta))
    .fetch_one(&state.db)
    .await?;
    if let Some(ext) = &m.message_id
        && let Some(existing) = sqlx::query_as::<_, Message>(&format!(
            "SELECT {MSG} FROM two_way_messages WHERE conversation_id = $1 AND external_id = $2"
        ))
        .bind(conv)
        .bind(ext)
        .fetch_optional(&state.db)
        .await?
    {
        return Ok((existing, false));
    }
    let team = s.team(p).to_string();
    let translated = match engine(state, p).await {
        Ok(e) => {
            let ctx = thread(state, conv, true).await?;
            inbound(
                &e,
                &m.text,
                &team,
                thread_locale.as_deref(),
                hint(m.lang_hint.as_deref()).as_deref(),
                s.min_detect_confidence,
                &ctx,
            )
            .await
        }
        Err(e) => Err(e),
    };
    let (r, error) = match translated {
        Ok(r) => (r, None),
        // Keep the message and say why it is not translated (quota, model down, key revoked…).
        Err(e) => {
            let d = detect::detect(&m.text);
            (
                Inbound {
                    locale: d.locale.or(thread_locale.clone()),
                    detect_confidence: d.confidence,
                    flags: vec![format!("untranslated:{}", e.code)],
                    ..Default::default()
                },
                Some(e.message),
            )
        }
    };
    let msg: Message = sqlx::query_as(&format!(
        "INSERT INTO two_way_messages (id, conversation_id, project_id, direction, external_id, original, original_locale,
            translation, translation_locale, detect_confidence, flags, pii_masked, status, error)
         VALUES ($1, $2, $3, 'in', $4, $5, $6, $7, $8, $9, $10, $11, 'received', $12) RETURNING {MSG}"
    ))
    .bind(Uuid::new_v4())
    .bind(conv)
    .bind(p.id)
    .bind(&m.message_id)
    .bind(&m.text)
    .bind(&r.locale)
    .bind(&r.translation)
    .bind(r.translation.as_ref().map(|_| &team))
    .bind(r.detect_confidence)
    .bind(&r.flags)
    .bind(r.pii_masked as i32)
    .bind(&error)
    .fetch_one(&state.db)
    .await?;
    // The thread follows the customer: a confident detection wins, a guess only fills an empty slot.
    if let Some(l) = &r.locale
        && (r.confident || thread_locale.is_none())
    {
        sqlx::query("UPDATE two_way_conversations SET customer_locale = $2 WHERE id = $1")
            .bind(conv)
            .bind(l)
            .execute(&state.db)
            .await?;
    }
    count_language(state, p.id, channel, r.locale.as_deref()).await;
    Ok((msg, true))
}

/// Translate an agent's reply and store it. It is delivered only when the mode allows: `auto` (unless flagged
/// for review), or `send = true` (an explicit human action) outside `suggest`.
pub async fn reply(
    state: &AppState,
    p: &ProjectRow,
    s: &Settings,
    conv: &Conversation,
    text: &str,
    send: bool,
    external_id: Option<&str>,
) -> ApiResult<Message> {
    check_text(text, MAX_TEXT)?;
    if send && s.mode == "suggest" {
        return Err(mode_forbids());
    }
    if let Some(ext) = external_id
        && let Some(existing) = sqlx::query_as::<_, Message>(&format!(
            "SELECT {MSG} FROM two_way_messages WHERE conversation_id = $1 AND external_id = $2"
        ))
        .bind(conv.id)
        .bind(ext)
        .fetch_optional(&state.db)
        .await?
    {
        return Ok(existing);
    }
    let customer = conv.customer_locale.clone().ok_or_else(|| {
        AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "customer_language_unknown",
            "glot could not tell which language this customer writes in: set the conversation's language first",
        )
    })?;
    let team = s.team(p);
    let e = engine(state, p).await?;
    let ctx = thread(state, conv.id, false).await?;
    let o = outbound(&e, text, team, &customer, &ctx, s.review_below).await?;
    let msg: Message = sqlx::query_as(&format!(
        "INSERT INTO two_way_messages (id, conversation_id, project_id, direction, external_id, original, original_locale,
            translation, translation_locale, back_translation, confidence, flags, pii_masked, status)
         VALUES ($1, $2, $3, 'out', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING {MSG}"
    ))
    .bind(Uuid::new_v4())
    .bind(conv.id)
    .bind(p.id)
    .bind(external_id)
    .bind(text)
    .bind(team)
    .bind(&o.translation)
    .bind(&customer)
    .bind(&o.back_translation)
    .bind(o.confidence)
    .bind(&o.flags)
    .bind(o.pii_masked as i32)
    .bind(if o.needs_review { "needs_review" } else { "draft" })
    .fetch_one(&state.db)
    .await?;
    sqlx::query("UPDATE two_way_conversations SET last_message_at = now() WHERE id = $1")
        .bind(conv.id)
        .execute(&state.db)
        .await?;
    if send || (s.mode == "auto" && !o.needs_review) {
        return deliver(state, p, s, conv, msg).await;
    }
    if conv.channel == "helpscout" && s.mode == "suggest" {
        // Suggest mode in a helpdesk: the translation waits as a draft the agent sends from their own tool.
        return helpscout::deliver(state, s, conv, msg, true).await;
    }
    Ok(msg)
}

fn mode_forbids() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "mode_forbids_send",
        "this project is in suggest mode: glot never sends to customers. Copy the translation, or switch to one-click or auto in Conversations → Settings",
    )
}

/// Record the outcome of a delivery attempt. A failure is kept on the message (shown in the inbox, retryable).
pub(crate) async fn delivered(
    state: &AppState,
    msg: Uuid,
    outcome: Result<(&str, bool), String>,
) -> ApiResult<Message> {
    let (via, sent, error) = match outcome {
        Ok((via, sent)) => (Some(via.to_string()), sent, None),
        Err(e) => (None, false, Some(e)),
    };
    Ok(sqlx::query_as(&format!(
        "UPDATE two_way_messages SET
            status = CASE WHEN $4::text IS NOT NULL THEN 'failed' WHEN $3 THEN 'sent' ELSE status END,
            delivered_via = coalesce($2, delivered_via), error = $4,
            sent_at = CASE WHEN $3 THEN now() ELSE sent_at END
         WHERE id = $1 RETURNING {MSG}"
    ))
    .bind(msg)
    .bind(via)
    .bind(sent)
    .bind(error)
    .fetch_one(&state.db)
    .await?)
}

async fn deliver(
    state: &AppState,
    p: &ProjectRow,
    s: &Settings,
    conv: &Conversation,
    msg: Message,
) -> ApiResult<Message> {
    match conv.channel.as_str() {
        "helpscout" => helpscout::deliver(state, s, conv, msg, false).await,
        "api" => {
            let Some(url) = &s.reply_url else {
                // No delivery URL: the caller reads the approved reply from the API response and sends it.
                return delivered(state, msg.id, Ok(("api", true))).await;
            };
            let body = json!({
                "event": "reply",
                "project": p.slug,
                "conversation_id": conv.external_id,
                "message_id": msg.id,
                "text": msg.translation,
                "locale": msg.translation_locale,
                "original": msg.original,
                "original_locale": msg.original_locale,
            })
            .to_string();
            let outcome = async {
                let key = secret(state, &s.webhook_secret_enc, "the webhook secret")
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                let http = crate::connectors::client_for(url).await?;
                let sig =
                    crate::billing::sign(&key, body.as_bytes(), chrono::Utc::now().timestamp());
                let res = http
                    .post(url)
                    .header("Glot-Signature", sig)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body.clone())
                    .send()
                    .await
                    .map_err(|_| anyhow::anyhow!("reply_url is unreachable"))?;
                anyhow::ensure!(
                    res.status().is_success(),
                    "reply_url answered {}",
                    res.status()
                );
                Ok(())
            }
            .await;
            delivered(
                state,
                msg.id,
                outcome
                    .map(|_| ("webhook", true))
                    .map_err(|e: anyhow::Error| e.to_string()),
            )
            .await
        }
        other => Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_reply_path",
            format!(
                "glot cannot send on the '{other}' channel yet: copy the translation and answer from your own tool"
            ),
        )),
    }
}

// ------------------------------------------------------------------------------------------------ dashboard API

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

async fn list_conversations(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<ListQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let rows: Vec<(
        Uuid,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        i64,
        i64,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT c.id, c.channel, c.external_id, c.customer_locale, c.customer_name, c.subject, c.last_message_at,
                (SELECT count(*) FROM two_way_messages m WHERE m.conversation_id = c.id),
                (SELECT count(*) FROM two_way_messages m WHERE m.conversation_id = c.id AND m.status IN ('needs_review', 'failed')),
                (SELECT left(coalesce(m.translation, m.original), 160) FROM two_way_messages m
                  WHERE m.conversation_id = c.id AND m.direction = 'in' ORDER BY m.created_at DESC LIMIT 1)
         FROM two_way_conversations c WHERE c.project_id = $1 ORDER BY c.last_message_at DESC LIMIT $2",
    )
    .bind(p.id)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&state.db)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "channel": r.1, "external_id": r.2, "customer_locale": r.3, "customer_name": r.4,
                "subject": r.5, "last_message_at": r.6, "messages": r.7, "attention": r.8, "preview": r.9,
            })
        })
        .collect();
    Ok(Json(json!({ "conversations": items })))
}

async fn conversation_view(state: &AppState, p: &ProjectRow, id: Uuid) -> ApiResult<Json<Value>> {
    let c = conversation(state, p.id, id).await?;
    let (name, subject): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT customer_name, subject FROM two_way_conversations WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let messages: Vec<Message> = sqlx::query_as(&format!(
        "SELECT {MSG} FROM two_way_messages WHERE conversation_id = $1 ORDER BY created_at"
    ))
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "id": c.id, "channel": c.channel, "external_id": c.external_id, "customer_locale": c.customer_locale,
        "customer_name": name, "subject": subject, "messages": messages,
    })))
}

async fn get_conversation(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    conversation_view(&state, &p, id).await
}

#[derive(Deserialize)]
struct PatchConversation {
    customer_locale: String,
}

/// Correct the customer's language (detection was unsure): inbound messages are translated again from it.
async fn patch_conversation(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<PatchConversation>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    validate_locale(&b.customer_locale)?;
    let c = conversation(&state, p.id, id).await?;
    sqlx::query("UPDATE two_way_conversations SET customer_locale = $2 WHERE id = $1")
        .bind(c.id)
        .bind(&b.customer_locale)
        .execute(&state.db)
        .await?;
    let s = settings(&state, p.id).await?;
    let e = engine(&state, &p).await?;
    let inbound_msgs: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, original FROM two_way_messages WHERE conversation_id = $1 AND direction = 'in' ORDER BY created_at",
    )
    .bind(c.id)
    .fetch_all(&state.db)
    .await?;
    for (mid, original) in inbound_msgs {
        // The person's choice is the thread language, and detection is not allowed to overrule it.
        let r = inbound(
            &e,
            &original,
            s.team(&p),
            Some(&b.customer_locale),
            None,
            2.0,
            &[],
        )
        .await?;
        sqlx::query(
            "UPDATE two_way_messages SET original_locale = $2, translation = $3, translation_locale = $4, flags = $5,
                    error = NULL WHERE id = $1",
        )
        .bind(mid)
        .bind(&r.locale)
        .bind(&r.translation)
        .bind(r.translation.as_ref().map(|_| s.team(&p)))
        .bind(&r.flags)
        .execute(&state.db)
        .await?;
    }
    conversation_view(&state, &p, id).await
}

#[derive(Deserialize)]
struct ReplyBody {
    text: String,
    #[serde(default)]
    send: bool,
}

async fn post_reply(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
    AppJson(b): AppJson<ReplyBody>,
) -> ApiResult<Json<Message>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let s = settings(&state, p.id).await?;
    let c = conversation(&state, p.id, id).await?;
    Ok(Json(
        reply(&state, &p, &s, &c, &b.text, b.send, None).await?,
    ))
}

#[derive(Deserialize)]
struct PreviewBody {
    text: String,
    conversation_id: Option<Uuid>,
    customer_locale: Option<String>,
}

/// The reply box's live safety check: translation + back-translation + confidence, nothing stored or sent.
async fn preview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PreviewBody>,
) -> ApiResult<Json<Outbound>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    check_text(&b.text, MAX_TEXT)?;
    let s = settings(&state, p.id).await?;
    let (customer, ctx) = match b.conversation_id {
        Some(id) => {
            let c = conversation(&state, p.id, id).await?;
            (
                b.customer_locale.or(c.customer_locale),
                thread(&state, c.id, false).await?,
            )
        }
        None => (b.customer_locale, vec![]),
    };
    let customer = customer
        .ok_or_else(|| AppError::bad_request("customer_locale or conversation_id is required"))?;
    validate_locale(&customer)?;
    let e = engine(&state, &p).await?;
    Ok(Json(
        outbound(&e, &b.text, s.team(&p), &customer, &ctx, s.review_below).await?,
    ))
}

async fn send_message(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, Uuid)>,
) -> ApiResult<Json<Message>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let s = settings(&state, p.id).await?;
    if s.mode == "suggest" {
        return Err(mode_forbids());
    }
    let msg: Message = sqlx::query_as(&format!(
        "SELECT {MSG} FROM two_way_messages WHERE id = $1 AND project_id = $2 AND direction = 'out'"
    ))
    .bind(id)
    .bind(p.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("reply not found"))?;
    if msg.status == "sent" {
        return Err(AppError::conflict("this reply was already sent"));
    }
    let c = conversation(&state, p.id, msg.conversation_id).await?;
    Ok(Json(deliver(&state, &p, &s, &c, msg).await?))
}

async fn rest_inbound(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<Incoming>,
) -> ApiResult<Json<Message>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let s = settings(&state, p.id).await?;
    Ok(Json(receive(&state, &p, &s, "api", b).await?.0))
}

#[derive(Deserialize)]
struct StatsQuery {
    days: Option<i32>,
}

/// Which languages customers write in. `served = false` on a language the project does not ship yet:
/// customers already writing in it is the strongest signal for the next market.
async fn stats(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<StatsQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let days = q.days.unwrap_or(90).clamp(1, 3650);
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT locale, sum(messages)::bigint FROM two_way_lang_stats
         WHERE project_id = $1 AND day > current_date - $2 GROUP BY locale ORDER BY 2 DESC, 1",
    )
    .bind(p.id)
    .bind(days)
    .fetch_all(&state.db)
    .await?;
    let total: i64 = rows.iter().map(|r| r.1).sum();
    let languages: Vec<Value> = rows
        .iter()
        .map(|(l, n)| {
            json!({
                "locale": l,
                "messages": n,
                "share": if total > 0 { *n as f64 / total as f64 } else { 0.0 },
                "served": same_lang(l, &p.source_locale) || p.target_locales.iter().any(|t| same_lang(t, l)),
            })
        })
        .collect();
    Ok(Json(
        json!({ "days": days, "total": total, "languages": languages }),
    ))
}

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/two-way";
    Router::new()
        .route(
            &format!("{p}/settings"),
            get(get_settings).put(put_settings),
        )
        .route(&format!("{p}/conversations"), get(list_conversations))
        .route(
            &format!("{p}/conversations/{{id}}"),
            get(get_conversation).patch(patch_conversation),
        )
        .route(&format!("{p}/conversations/{{id}}/reply"), post(post_reply))
        .route(&format!("{p}/messages/{{id}}/send"), post(send_message))
        .route(&format!("{p}/preview"), post(preview))
        .route(&format!("{p}/inbound"), post(rest_inbound))
        .route(&format!("{p}/stats"), get(stats))
}

// ------------------------------------------------------------------------------------------------ public channels

async fn public_project(state: &AppState, slug: &str) -> ApiResult<(ProjectRow, Settings)> {
    let p = projects::load_public(state, slug)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))?;
    let s = settings(state, p.id).await?;
    Ok((p, s))
}

/// `Glot-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256(secret, "{t}.{raw body}")>`, five minutes of tolerance
/// (the scheme glot already uses for Stripe: `billing::verify_signature`).
fn verify_webhook(
    state: &AppState,
    s: &Settings,
    headers: &HeaderMap,
    body: &[u8],
) -> ApiResult<()> {
    let key = secret(state, &s.webhook_secret_enc, "the webhook secret")?;
    let bad = |why: &str| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            format!("Glot-Signature rejected: {why}"),
        )
    };
    let sig = headers
        .get("glot-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| bad("header missing"))?;
    crate::billing::verify_signature(&key, sig, body, chrono::Utc::now().timestamp()).map_err(bad)
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> ApiResult<T> {
    serde_json::from_slice(body)
        .map_err(|e| AppError::bad_request(format!("invalid JSON body: {e}")))
}

/// Generic channel: any tool posts a customer message, the answer carries its translation.
async fn webhook(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Message>> {
    let (p, s) = public_project(&state, &slug).await?;
    verify_webhook(&state, &s, &headers, &body)?;
    Ok(Json(receive(&state, &p, &s, "api", parse(&body)?).await?.0))
}

#[derive(Deserialize)]
struct EmailBody {
    from: String,
    #[serde(default)]
    subject: Option<String>,
    text: String,
    #[serde(default)]
    message_id: Option<String>,
    /// The mail thread (`References` root or the provider's thread id) when the caller has it.
    #[serde(default)]
    thread_id: Option<String>,
}

/// The new part of an email reply: everything before the quoted previous message.
pub fn strip_quoted(text: &str) -> String {
    let mut out = vec![];
    for line in text.lines() {
        let l = line.trim();
        let attribution = (l.starts_with("On ") && l.ends_with("wrote:"))
            || (l.starts_with("Le ")
                && (l.ends_with("a écrit :") || l.ends_with("a écrit\u{a0}:")))
            || (l.starts_with("Am ") && l.ends_with("schrieb:"))
            || (l.starts_with("El ") && l.ends_with("escribió:"));
        if l.starts_with('>') || attribution || l.starts_with("-----Original Message-----") {
            break;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>")
}

/// Inbound email (posted by an inbound-mail provider or worker, signed like the webhook): parse → translate →
/// forward to the team's address through the transactional outbox. The sender's address is used for the
/// forward only; the conversation is keyed by a hash of it.
async fn email_inbound(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let (p, s) = public_project(&state, &slug).await?;
    verify_webhook(&state, &s, &headers, &body)?;
    let b: EmailBody = parse(&body)?;
    let thread_key = b
        .thread_id
        .clone()
        .unwrap_or_else(|| b.from.trim().to_lowercase());
    let incoming = Incoming {
        conversation_id: crate::auth::sha256_hex(&thread_key)[..32].to_string(),
        message_id: b.message_id.clone(),
        text: strip_quoted(&b.text),
        subject: b.subject.clone(),
        ..Default::default()
    };
    let (msg, new) = receive(&state, &p, &s, "email", incoming).await?;
    let mut forwarded = false;
    if new
        && state.mailer.is_some()
        && let Some(to) = &s.forward_to
    {
        let pair = format!(
            "{} → {}",
            msg.original_locale.as_deref().unwrap_or("?"),
            s.team(&p)
        );
        let subject = format!(
            "[{pair}] {}",
            b.subject.as_deref().unwrap_or("(no subject)")
        );
        let translated = msg.translation.as_deref().unwrap_or(&msg.original);
        let text = format!(
            "From: {}\n\n{translated}\n\n--- Original ({}) ---\n{}\n",
            b.from,
            msg.original_locale.as_deref().unwrap_or("?"),
            msg.original
        );
        let html = format!(
            "<p><b>From:</b> {}</p><p>{}</p><hr><p><b>Original ({}):</b></p><p>{}</p>",
            esc(&b.from),
            esc(translated),
            esc(msg.original_locale.as_deref().unwrap_or("?")),
            esc(&msg.original)
        );
        // The outbox worker delivers it (retries, bodies cleared once sent).
        forwarded = sqlx::query(
            "INSERT INTO email_outbox (id, idem_key, to_email, template, subject, html, text_body)
             VALUES ($1, $2, $3, 'two_way_forward', $4, $5, $6) ON CONFLICT (idem_key) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(format!("two-way:{}", msg.id))
        .bind(to)
        .bind(subject)
        .bind(html)
        .bind(text)
        .execute(&state.db)
        .await?
        .rows_affected()
            == 1;
    }
    Ok(Json(json!({ "message": msg, "forwarded": forwarded })))
}

#[derive(Deserialize)]
struct FormBody {
    text: String,
    #[serde(default)]
    lang_hint: Option<String>,
}

/// Contact-form helper: tags a message with its language and its translation for the site owner.
/// Public (called from the visitor's browser), so: off until the project turns it on, a daily cap on
/// translations, and nothing stored but a per-language counter unless the project opts in (`form_store`).
async fn form(
    State(state): State<AppState>,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<FormBody>,
) -> ApiResult<Json<Value>> {
    let (p, s) = public_project(&state, &slug).await?;
    if !s.form_enabled {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "form_disabled",
            "the contact-form helper is off for this project: turn it on in Conversations → Settings",
        ));
    }
    check_text(&b.text, MAX_FORM_TEXT)?;
    let team = s.team(&p).to_string();
    let (today,): (i64,) = sqlx::query_as(
        "SELECT coalesce(sum(messages), 0)::bigint FROM two_way_lang_stats
         WHERE project_id = $1 AND channel = 'form' AND day = current_date",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    let view = |r: &Inbound, reason: Option<&str>| {
        json!({
            "language": r.locale,
            "confidence": r.detect_confidence,
            "team_locale": team,
            "translation": r.translation,
            "flags": r.flags,
            "reason": reason,
        })
    };
    if today >= s.form_per_day as i64 {
        // Over the cap: detection is local and free, so the language tag is still returned.
        let d = detect::detect(&Masker::default().mask(&b.text));
        let r = Inbound {
            locale: d.locale,
            detect_confidence: d.confidence,
            ..Default::default()
        };
        return Ok(Json(view(&r, Some("daily_cap"))));
    }
    if s.form_store {
        let incoming = Incoming {
            conversation_id: Uuid::new_v4().to_string(),
            text: b.text,
            lang_hint: b.lang_hint,
            ..Default::default()
        };
        let (m, _) = receive(&state, &p, &s, "form", incoming).await?;
        return Ok(Json(json!({
            "language": m.original_locale, "confidence": m.detect_confidence, "team_locale": team,
            "translation": m.translation, "flags": m.flags, "reason": m.error,
        })));
    }
    let e = engine(&state, &p).await?;
    let r = inbound(
        &e,
        &b.text,
        &team,
        None,
        hint(b.lang_hint.as_deref()).as_deref(),
        s.min_detect_confidence,
        &[],
    )
    .await?;
    count_language(&state, p.id, "form", r.locale.as_deref()).await;
    Ok(Json(view(&r, None)))
}

/// Drop-in script: on submit of a `<form data-glot-form>`, adds hidden `glot_language` / `glot_translation`
/// fields, then lets the form go wherever it was going (any form backend). Never blocks a submission.
const FORM_JS: &str = r#"(()=>{const s=document.currentScript,api=new URL(s.src).origin,slug=s.dataset.project;
const set=(f,n,v)=>{if(v==null)return;let i=f.querySelector(`input[name="${n}"]`);if(!i){i=document.createElement("input");i.type="hidden";i.name=n;f.appendChild(i)}i.value=v};
document.addEventListener("submit",async e=>{const f=e.target;
if(!slug||f.dataset.glotDone||!f.matches(s.dataset.form||"form[data-glot-form]"))return;
const t=f.querySelector(s.dataset.field||"textarea");if(!t||!t.value.trim())return;
e.preventDefault();const b=e.submitter;
try{const r=await fetch(`${api}/api/two-way/${encodeURIComponent(slug)}/form`,{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({text:t.value,lang_hint:navigator.language}),signal:AbortSignal.timeout(8000)});
if(r.ok){const d=await r.json();set(f,"glot_language",d.language);set(f,"glot_translation",d.translation)}}catch{}
f.dataset.glotDone="1";f.requestSubmit?f.requestSubmit(b):f.submit()},true)})();"#;

async fn form_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        FORM_JS,
    )
        .into_response()
}

pub fn public_routes() -> Router<AppState> {
    let open = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::POST])
        .allow_headers([header::CONTENT_TYPE]);
    Router::new()
        .route("/api/two-way/{slug}/webhook", post(webhook))
        .route("/api/two-way/{slug}/email", post(email_inbound))
        .route("/api/two-way/{slug}/helpscout", post(helpscout::webhook))
        .route("/api/two-way/form.js", get(form_js))
        .merge(
            Router::new()
                .route("/api/two-way/{slug}/form", post(form))
                .layer(open),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agreement_separates_faithful_from_divergent() {
        let said = "Votre commande arrive demain, nous vous remboursons les frais de port.";
        assert!(
            agreement(
                said,
                "Votre commande arrive demain et nous vous remboursons les frais de port."
            ) > 0.8
        );
        assert!(
            agreement(
                said,
                "Nous ne pouvons rien faire pour vous, contactez le transporteur."
            ) < 0.4
        );
        assert_eq!(agreement("ok", "OK "), 1.0);
        assert_eq!(agreement("ok", "no"), 0.0);
    }

    #[test]
    fn glossary_pivots_between_two_target_locales() {
        let g = Glossary {
            terms: vec![
                Term {
                    term: "workspace".into(),
                    description: None,
                    do_not_translate: false,
                    case_sensitive: false,
                    translations: [
                        ("fr".to_string(), "espace de travail".to_string()),
                        ("ja".to_string(), "ワークスペース".to_string()),
                    ]
                    .into(),
                    forbidden: Default::default(),
                },
                Term {
                    term: "glot Edge".into(),
                    description: None,
                    do_not_translate: true,
                    case_sensitive: true,
                    translations: Default::default(),
                    forbidden: Default::default(),
                },
            ],
        };
        // Japanese customer → French team, English project: neither side is the glossary's own language.
        let t = pivot_glossary(
            &g,
            "en",
            "ja",
            "fr",
            "ワークスペースが開けません。glot Edge のせい？",
        );
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].term, "ワークスペース");
        assert_eq!(t[0].translations["fr"], "espace de travail");
        assert!(t[1].do_not_translate && t[1].term == "glot Edge");
        // And back: French agent → Japanese customer.
        let t = pivot_glossary(&g, "en", "fr", "ja", "Votre Espace de travail est rouvert.");
        assert_eq!(t[0].translations["ja"], "ワークスペース");
        assert!(pivot_glossary(&g, "en", "fr", "ja", "Bonjour").is_empty());
        // Whole words only in spaced scripts; any occurrence where words are not separated.
        assert!(contains_term("les clips sont prêts", "clips"));
        assert!(!contains_term("cliponit est prêt", "clip"));
        assert!(contains_term(
            "ワークスペースが開けません",
            "ワークスペース"
        ));
        assert!(contains_term("le glot edge, enfin", "glot edge"));
    }

    #[test]
    fn quoted_email_history_is_dropped() {
        let mail = "まだ届いていません。\n\nOn Mon, 3 Nov 2026 at 10:02, Support <help@shop.example> wrote:\n> Bonjour,\n> votre colis est parti.";
        assert_eq!(strip_quoted(mail), "まだ届いていません。");
        assert_eq!(strip_quoted("Hello\nworld"), "Hello\nworld");
    }
}
