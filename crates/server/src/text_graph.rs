//! Text graph (the "universal anchor"): glot manages sentences, not files of keys.
//!
//! A node (`text_anchors`) is a sentence: placeholder-aware normalized source text + context. An edge
//! (`text_occurrences`) is a place where it appears: a catalog key, a CMS field mirrored by a connector, a segment
//! glot Edge saw on a page, or anything an importer declares (emails, design files, videos). Translations of the
//! existing stores are never copied: they are read live from `translations` and `edge_translations`, and written
//! back through the same checks their owners use.
//!
//! Identity: `exact` (the text, whitespace collapsed: glot Edge's segment hash), `norm` (placeholder syntax, quote
//! style and spacing ignored: the anchor), `loose` (case and punctuation ignored) and trigram similarity (both only
//! suggested as related sentences, never merged). A short text on a button or a link is a different sentence from
//! the same word used as plain text ("Save" the action, "Save" the discount).
//!
//! Propagation rules ([`plan`]): a human-reviewed translation is never replaced by a machine one; two reviewed
//! translations that disagree are a conflict a person settles; two machine translations that disagree are an
//! inconsistency nobody settles automatically; every write passes the placeholder / syntax / glossary validation
//! of the place it lands in.
use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use glot_core::{
    edge::tm::{normalize as collapse_ws, text_hash},
    validate::{Severity, Syntax},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, types::Json as DbJson};
use uuid::Uuid;

use crate::{
    ApiResult, AppError, AppJson, AppPath, AppQuery, AppState,
    auth::AuthUser,
    keys,
    projects::{self, ProjectRow, Role},
};

pub fn routes() -> Router<AppState> {
    let p = "/api/projects/{slug}/text-graph";
    Router::new()
        .route(p, get(overview))
        .route(&format!("{p}/search"), get(search))
        .route(&format!("{p}/anchors/{{id}}"), get(anchor))
        .route(&format!("{p}/anchors/{{id}}/unify"), post(unify))
        .route(&format!("{p}/propagate"), post(propagate))
        .route(&format!("{p}/sync"), post(sync_now))
        .route(&format!("{p}/import"), post(import))
        .route(&format!("{p}/export"), get(export))
}

// ------------------------------------------------------------------------------------------- identity

/// Length of the placeholder starting at `b[0]`, 0 when there is none. Recognized: `{name}` `{0}` `{{ name }}`
/// `${name}` `#{name}` `%{name}` `%(name)s` and printf (`%s` `%d` `%@` `%1$s` `%.2f` `%lld`).
/// ICU plural/select blocks (`{n, plural, …}`) and tags are text: two sentences match across placeholder
/// *syntaxes*, not across message formats.
// ponytail: Laravel `:name` is not recognized (too close to ordinary punctuation); add it behind the project
// syntax if Laravel users need cross-format matching.
fn placeholder_len(b: &[u8]) -> usize {
    let ident = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-');
    let braces = |open: usize| -> usize {
        let double = b.get(open + 1) == Some(&b'{');
        let mut i = open + if double { 2 } else { 1 };
        while b.get(i) == Some(&b' ') {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(|c| ident(*c)) {
            i += 1;
        }
        if i == start {
            return 0;
        }
        while b.get(i) == Some(&b' ') {
            i += 1;
        }
        let close: &[u8] = if double { b"}}" } else { b"}" };
        if b[i..].starts_with(close) {
            i + close.len()
        } else {
            0
        }
    };
    match b[0] {
        b'{' => braces(0),
        b'$' | b'#' | b'%' if b.get(1) == Some(&b'{') => braces(1),
        b'%' => {
            let mut i = 1;
            if b.get(1) == Some(&b'(') {
                i = 2;
                while b.get(i).is_some_and(|c| ident(*c)) {
                    i += 1;
                }
                if i == 2 || b.get(i) != Some(&b')') {
                    return 0;
                }
                i += 1;
            } else {
                let digits = |mut i: usize| {
                    while b.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                    i
                };
                let d = digits(i);
                if d > i && b.get(d) == Some(&b'$') {
                    i = d + 1;
                }
                while b
                    .get(i)
                    .is_some_and(|c| matches!(c, b'-' | b'+' | b'0' | b'#'))
                {
                    i += 1;
                }
                i = digits(i);
                if b.get(i) == Some(&b'.') {
                    i = digits(i + 1);
                }
                while b.get(i).is_some_and(|c| matches!(c, b'l' | b'h' | b'z')) {
                    i += 1;
                }
            }
            match b.get(i) {
                Some(c) if b"sdiufxXeEgGc@".contains(c) => i + 1,
                _ => 0,
            }
        }
        _ => 0,
    }
}

enum Tok<'a> {
    Text(&'a str),
    Ph(&'a str),
}

fn tokens(s: &str) -> Vec<Tok<'_>> {
    let b = s.as_bytes();
    let (mut out, mut from, mut i) = (vec![], 0, 0);
    while i < b.len() {
        // `%%` is a literal percent sign.
        if b[i] == b'%' && b.get(i + 1) == Some(&b'%') {
            i += 2;
            continue;
        }
        match placeholder_len(&b[i..]) {
            0 => i += 1,
            n => {
                if from < i {
                    out.push(Tok::Text(&s[from..i]));
                }
                out.push(Tok::Ph(&s[i..i + n]));
                i += n;
                from = i;
            }
        }
    }
    if from < s.len() {
        out.push(Tok::Text(&s[from..]));
    }
    out
}

/// `%s`, `%d`, `%@`: every occurrence is its own argument (a named or numbered one is the same argument each time).
fn positional(raw: &str) -> bool {
    raw.starts_with('%') && !raw.contains(['{', '(', '$'])
}

fn marker(i: usize) -> String {
    format!("\u{27e6}{i}\u{27e7}")
}

/// Normalized text (placeholders as numbered slots, straight quotes, plain spaces, `...`) and the raw placeholder
/// of each slot.
pub fn parse(text: &str) -> (String, Vec<String>) {
    let (mut norm, mut slots) = (String::new(), Vec::<String>::new());
    for t in tokens(text) {
        match t {
            Tok::Ph(raw) => {
                let i = match slots.iter().position(|s| s.as_str() == raw) {
                    Some(i) if !positional(raw) => i,
                    _ => {
                        slots.push(raw.to_string());
                        slots.len() - 1
                    }
                };
                norm.push_str(&marker(i));
            }
            Tok::Text(s) => {
                for c in s.chars() {
                    match c {
                        '\u{2018}' | '\u{2019}' => norm.push('\''),
                        '\u{201c}' | '\u{201d}' => norm.push('"'),
                        '\u{a0}' | '\u{202f}' | '\u{2009}' => norm.push(' '),
                        '\u{2026}' => norm.push_str("..."),
                        c => norm.push(c),
                    }
                }
            }
        }
    }
    (collapse_ws(&norm), slots)
}

/// Case and punctuation ignored: "Save changes." and "save changes" are near-duplicates.
fn loose(norm: &str) -> String {
    let kept: String = norm
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '\u{27e6}' | '\u{27e7}') {
                c
            } else {
                ' '
            }
        })
        .collect();
    collapse_ws(&kept.to_lowercase())
}

/// `value` (a translation of a source with placeholders `slots`) with its placeholders as slot numbers.
/// `None`: it uses a placeholder its source does not have.
fn slotted(value: &str, slots: &[String]) -> Option<Vec<Result<usize, String>>> {
    let mut used = vec![false; slots.len()];
    tokens(value)
        .into_iter()
        .map(|t| match t {
            Tok::Text(s) => Some(Err(s.to_string())),
            Tok::Ph(raw) => {
                let i =
                    (0..slots.len()).find(|&i| slots[i] == raw && !(positional(raw) && used[i]))?;
                used[i] = true;
                Some(Ok(i))
            }
        })
        .collect()
}

/// What two translations are compared by: placeholder syntax and spacing ignored.
fn canon(value: &str, slots: &[String]) -> String {
    match slotted(value, slots) {
        Some(parts) => collapse_ws(
            &parts
                .into_iter()
                .map(|p| p.map_or_else(|s| s, marker))
                .collect::<String>(),
        ),
        None => collapse_ws(value),
    }
}

/// A translation written for a source with placeholders `from`, rewritten for the same sentence with placeholders
/// `to` (`Bonjour {name}` → `Bonjour %s`). `None` when it cannot be done safely.
pub fn retarget(value: &str, from: &[String], to: &[String]) -> Option<String> {
    if from == to {
        return Some(value.to_string());
    }
    if from.len() != to.len() {
        return None;
    }
    Some(
        slotted(value, from)?
            .into_iter()
            .map(|p| p.map_or_else(|s| s, |i| to[i].clone()))
            .collect(),
    )
}

fn is_action(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "button" | "a" | "link" | "menuitem" | "tab" | "cta" | "submit" | "summary"
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub anchor_id: String,
    pub exact_hash: String,
    pub norm_hash: String,
    pub loose_hash: String,
    pub context: String,
}

/// The identity of a text shown with `role` (a tag or UI role, may be empty). `explicit` is a context the caller
/// states (gettext `msgctxt` style): sentences with different explicit contexts are never the same.
pub fn identify(source_locale: &str, text: &str, role: &str, explicit: Option<&str>) -> Identity {
    let (norm, _) = parse(text);
    let context = match explicit.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("x:{c}"),
        // Only short texts are ambiguous enough to split: "Save" (verb, on a button) vs "Save" (noun).
        None if is_action(role) && norm.split_whitespace().count() <= 3 => "action".into(),
        None => String::new(),
    };
    Identity {
        anchor_id: text_hash(source_locale, &format!("{norm}\u{0}{context}")),
        exact_hash: text_hash(source_locale, &collapse_ws(text)),
        norm_hash: text_hash(source_locale, &norm),
        loose_hash: text_hash(source_locale, &loose(&norm)),
        context,
    }
}

/// Dice coefficient over character trigrams of the loose forms, 0..1.
pub fn similarity(a: &str, b: &str) -> f32 {
    let grams = |s: &str| -> HashSet<[char; 3]> {
        let c: Vec<char> = loose(&parse(s).0).chars().collect();
        c.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
    };
    let (a, b) = (grams(a), grams(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    2.0 * a.intersection(&b).count() as f32 / (a.len() + b.len()) as f32
}

// ------------------------------------------------------------------------------------------- analysis

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Occ {
    pub anchor_id: String,
    pub surface: String,
    pub locator: String,
    pub source_text: String,
    pub exact_hash: String,
    pub role: String,
    pub meta: DbJson<Value>,
}

#[derive(Debug, Clone)]
pub struct Tr {
    pub value: String,
    pub status: String,
    pub origin: String,
}

impl Tr {
    /// Stale, pending (awaiting review) and empty translations are not shown anywhere: they count as missing.
    fn live(&self) -> bool {
        matches!(self.status.as_str(), "machine" | "reviewed") && !self.value.is_empty()
    }
}

/// Translations by (surface, locator), then locale.
pub type TrMap = HashMap<(String, String), BTreeMap<String, Tr>>;

struct Cell<'a> {
    occ: &'a Occ,
    slots: Vec<String>,
    tr: Option<&'a Tr>,
    /// A glot Edge segment without its own translation that Edge serves from the catalog key of the same text.
    inherited: bool,
    /// Comparable form of the translation shown here; `None` = nothing shown.
    canon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub value: String,
    pub count: usize,
    pub reviewed: bool,
    /// `surface:locator` of the places showing this translation.
    pub places: Vec<String>,
    #[serde(skip)]
    canon: String,
    #[serde(skip)]
    slots: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Winner {
    pub value: String,
    /// Placeholders of the source `value` was written for.
    pub slots: Vec<String>,
    pub reviewed: bool,
}

pub struct Analysis<'a> {
    cells: Vec<Cell<'a>>,
    pub groups: Vec<Group>,
    /// missing | consistent | partial (one translation, some places without it) | inconsistent | conflict
    pub state: &'static str,
    /// The translation propagation may apply without asking: the only reviewed one, or the only one.
    pub winner: Option<Winner>,
    pub missing: usize,
}

pub fn analyze<'a>(occs: &'a [Occ], trs: &'a TrMap, locale: &str) -> Analysis<'a> {
    let own = |o: &Occ| {
        trs.get(&(o.surface.clone(), o.locator.clone()))
            .and_then(|m| m.get(locale))
    };
    // What glot Edge falls back to (edge::tm::snapshot): the catalog translation of exactly this text.
    let donors: HashMap<&str, &Tr> = occs
        .iter()
        .filter(|o| o.surface == "catalog" || o.surface == "connector")
        .filter(|o| !o.source_text.contains(['{', '<']))
        .filter_map(|o| Some((o.exact_hash.as_str(), own(o).filter(|t| t.live())?)))
        .collect();
    let cells: Vec<Cell> = occs
        .iter()
        .map(|occ| {
            let slots = parse(&occ.source_text).1;
            let mut tr = own(occ);
            let mut inherited = false;
            if occ.surface == "edge" && tr.is_none() {
                tr = donors.get(occ.exact_hash.as_str()).copied();
                inherited = tr.is_some();
            }
            let canon = tr.filter(|t| t.live()).map(|t| canon(&t.value, &slots));
            Cell {
                occ,
                slots,
                tr,
                inherited,
                canon,
            }
        })
        .collect();
    let mut groups: Vec<Group> = vec![];
    for c in &cells {
        let (Some(k), Some(t)) = (&c.canon, c.tr) else {
            continue;
        };
        let i = groups
            .iter()
            .position(|g| g.canon == *k)
            .unwrap_or_else(|| {
                groups.push(Group {
                    value: t.value.clone(),
                    count: 0,
                    reviewed: false,
                    places: vec![],
                    canon: k.clone(),
                    slots: c.slots.clone(),
                });
                groups.len() - 1
            });
        groups[i].count += 1;
        groups[i].reviewed |= t.status == "reviewed";
        groups[i]
            .places
            .push(format!("{}:{}", c.occ.surface, c.occ.locator));
    }
    groups.sort_by_key(|g| (!g.reviewed, std::cmp::Reverse(g.count)));
    let reviewed = groups.iter().filter(|g| g.reviewed).count();
    let missing = cells.iter().filter(|c| c.canon.is_none()).count();
    let state = match (groups.len(), reviewed) {
        (0, _) => "missing",
        (1, _) if missing == 0 => "consistent",
        (1, _) => "partial",
        (_, r) if r >= 2 => "conflict",
        _ => "inconsistent",
    };
    let winner = match (groups.len(), reviewed) {
        (_, 1) | (1, 0) => Some(Winner {
            value: groups[0].value.clone(),
            slots: groups[0].slots.clone(),
            reviewed: groups[0].reviewed,
        }),
        _ => None,
    };
    Analysis {
        cells,
        groups,
        state,
        winner,
        missing,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub anchor_id: String,
    pub surface: String,
    pub locator: String,
    pub locale: String,
    pub value: String,
    /// The translation shown there now (absent = none).
    pub previous: Option<String>,
    /// write | skip
    pub action: &'static str,
    /// Why it is skipped: reviewed_conflict | machine_disagrees | needs_review | placeholders | validation.
    pub reason: Option<&'static str>,
    pub detail: Option<String>,
}

/// What applying `w` to every place of the sentence does. `force` (a person explicitly asked) also replaces
/// reviewed translations that say something else. `check` returns the validation error of a value in a place.
pub fn plan(
    a: &Analysis,
    w: &Winner,
    locale: &str,
    force: bool,
    check: &dyn Fn(&Occ, &str) -> Option<String>,
) -> Vec<Action> {
    let target = canon(&w.value, &w.slots);
    let mut out = vec![];
    for c in &a.cells {
        // Already says so, or follows the catalog key that does / will.
        if c.canon.as_deref() == Some(target.as_str()) || c.inherited {
            continue;
        }
        let value = retarget(&w.value, &w.slots, &c.slots);
        let status = c.tr.map(|t| t.status.as_str());
        let live = c.canon.is_some();
        let (reason, detail) = match &value {
            None => (Some("placeholders"), None),
            Some(_) if live && status == Some("reviewed") && !force => {
                (Some("reviewed_conflict"), None)
            }
            Some(_) if live && !w.reviewed => (Some("machine_disagrees"), None),
            // An Edge segment first seen on a sensitive page waits for a person, not for a machine copy.
            Some(_) if status == Some("pending") && !w.reviewed => (Some("needs_review"), None),
            Some(v) => match check(c.occ, v) {
                Some(e) => (Some("validation"), Some(e)),
                None => (None, None),
            },
        };
        out.push(Action {
            anchor_id: c.occ.anchor_id.clone(),
            surface: c.occ.surface.clone(),
            locator: c.occ.locator.clone(),
            locale: locale.to_string(),
            value: value.unwrap_or_else(|| w.value.clone()),
            previous: c.tr.filter(|_| live).map(|t| t.value.clone()),
            action: if reason.is_none() { "write" } else { "skip" },
            reason,
            detail,
        });
    }
    out
}

// ------------------------------------------------------------------------------------------- storage

const MIRRORED: [&str; 3] = ["catalog", "connector", "edge"];

pub struct NewOcc {
    pub surface: String,
    pub locator: String,
    pub text: String,
    pub role: String,
    pub meta: Value,
    pub context: Option<String>,
    /// glot Edge's own segment hash (what its TM is keyed by); computed from the text otherwise.
    pub exact_hash: Option<String>,
}

async fn upsert(
    conn: &mut PgConnection,
    p: &ProjectRow,
    rows: Vec<NewOcc>,
    seen: DateTime<Utc>,
) -> ApiResult<Vec<Identity>> {
    let mut anchors: BTreeMap<String, (String, String, String, String)> = BTreeMap::new();
    let mut ids = Vec::with_capacity(rows.len());
    let (mut surface, mut locator, mut anchor, mut exact, mut text, mut role, mut meta) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    for r in rows {
        let mut id = identify(&p.source_locale, &r.text, &r.role, r.context.as_deref());
        if let Some(h) = r.exact_hash {
            id.exact_hash = h;
        }
        anchors.entry(id.anchor_id.clone()).or_insert_with(|| {
            (
                id.norm_hash.clone(),
                id.loose_hash.clone(),
                id.context.clone(),
                r.text.clone(),
            )
        });
        surface.push(r.surface);
        locator.push(r.locator);
        anchor.push(id.anchor_id.clone());
        exact.push(id.exact_hash.clone());
        text.push(r.text);
        role.push(r.role);
        meta.push(r.meta);
        ids.push(id);
    }
    let (mut a_id, mut a_norm, mut a_loose, mut a_ctx, mut a_text) =
        (vec![], vec![], vec![], vec![], vec![]);
    for (id, (norm, loose, ctx, text)) in anchors {
        a_id.push(id);
        a_norm.push(norm);
        a_loose.push(loose);
        a_ctx.push(ctx);
        a_text.push(text);
    }
    sqlx::query(
        "INSERT INTO text_anchors (project_id, id, norm_hash, loose_hash, context, source_text)
         SELECT $1::uuid, * FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::text[])
         ON CONFLICT (project_id, id) DO NOTHING",
    )
    .bind(p.id)
    .bind(&a_id)
    .bind(&a_norm)
    .bind(&a_loose)
    .bind(&a_ctx)
    .bind(&a_text)
    .execute(&mut *conn)
    .await?;
    // A place whose source was reworded moves to its new sentence and remembers the one it left.
    sqlx::query(
        "INSERT INTO text_occurrences AS o
            (project_id, surface, locator, anchor_id, exact_hash, source_text, role, meta, seen_at)
         SELECT $1::uuid, x.*, $9::timestamptz
         FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::text[], $8::jsonb[]) AS x
         ON CONFLICT (project_id, surface, locator) DO UPDATE SET
            prev_anchor_id = CASE WHEN o.anchor_id <> EXCLUDED.anchor_id THEN o.anchor_id ELSE o.prev_anchor_id END,
            moved_at = CASE WHEN o.anchor_id <> EXCLUDED.anchor_id THEN EXCLUDED.seen_at ELSE o.moved_at END,
            anchor_id = EXCLUDED.anchor_id, exact_hash = EXCLUDED.exact_hash, source_text = EXCLUDED.source_text,
            role = EXCLUDED.role, meta = EXCLUDED.meta, seen_at = EXCLUDED.seen_at",
    )
    .bind(p.id)
    .bind(&surface)
    .bind(&locator)
    .bind(&anchor)
    .bind(&exact)
    .bind(&text)
    .bind(&role)
    .bind(&meta)
    .bind(seen)
    .execute(&mut *conn)
    .await?;
    Ok(ids)
}

async fn drop_orphans(conn: &mut PgConnection, project: Uuid) -> sqlx::Result<()> {
    sqlx::query(
        "DELETE FROM text_anchors a WHERE a.project_id = $1 AND NOT EXISTS
            (SELECT 1 FROM text_occurrences o WHERE o.project_id = a.project_id AND o.anchor_id = a.id)",
    )
    .bind(project)
    .execute(conn)
    .await?;
    Ok(())
}

/// One writer per project at a time; returns the instant this pass stamps on what it sees.
async fn lock(conn: &mut PgConnection, project: Uuid) -> sqlx::Result<DateTime<Utc>> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 177))")
        .bind(project)
        .execute(&mut *conn)
        .await?;
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(conn)
        .await
}

/// Mirror the places of the stores that already exist (read-only on their tables): catalog keys, CMS fields
/// mirrored by connectors (keys named `{kind}:…`), segments glot Edge and the website tag saw.
// ponytail: full pass, O(keys + segments) per sync, a few hashes per row. Go incremental (keys.updated_at /
// edge_segments.last_seen watermarks) when a project passes ~200k places.
pub async fn sync(db: &PgPool, p: &ProjectRow) -> ApiResult<usize> {
    let mut tx = db.begin().await?;
    let seen = lock(&mut tx, p.id).await?;
    let keys: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT key, source, ui_role FROM keys WHERE project_id = $1 AND source <> ''",
    )
    .bind(p.id)
    .fetch_all(&mut *tx)
    .await?;
    let items: HashMap<String, Value> =
        sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT i.key, a.kind, a.name, i.external_id, i.field FROM connector_items i
         JOIN connector_accounts a ON a.id = i.account_id WHERE a.project_id = $1",
        )
        .bind(p.id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|(key, kind, account, id, field)| {
            (
                key,
                json!({ "kind": kind, "account": account, "external_id": id, "field": field }),
            )
        })
        .collect();
    let segments: Vec<(String, String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT hash, context, source_text, url_sample, hits FROM edge_segments WHERE project_id = $1",
    )
    .bind(p.id)
    .fetch_all(&mut *tx)
    .await?;

    let mut rows = Vec::with_capacity(keys.len() + segments.len());
    for (key, source, ui_role) in keys {
        let connector = crate::connectors::is_key(&key);
        // Rich text fields own `{key}#{hash}` segment keys: same CMS field.
        let item = key.split('#').next().unwrap_or(&key);
        let meta = match items.get(item) {
            Some(m) => m.clone(),
            None if connector => json!({ "kind": key.split(':').next() }),
            None => json!({}),
        };
        rows.push(NewOcc {
            surface: if connector { "connector" } else { "catalog" }.into(),
            locator: key,
            text: source,
            role: ui_role.unwrap_or_default(),
            meta,
            context: None,
            exact_hash: None,
        });
    }
    for (hash, context, text, url, hits) in segments {
        if text.trim().is_empty() {
            continue;
        }
        rows.push(NewOcc {
            surface: "edge".into(),
            locator: format!("{hash}|{context}"),
            role: context.split('|').next().unwrap_or("").to_string(),
            meta: json!({ "url": url, "hits": hits, "context": context }),
            text,
            context: None,
            exact_hash: Some(hash),
        });
    }
    let n = rows.len();
    upsert(&mut tx, p, rows, seen).await?;
    sqlx::query(
        "DELETE FROM text_occurrences WHERE project_id = $1 AND surface = ANY($2) AND seen_at < $3",
    )
    .bind(p.id)
    .bind(&MIRRORED[..])
    .bind(seen)
    .execute(&mut *tx)
    .await?;
    drop_orphans(&mut tx, p.id).await?;
    sqlx::query(
        "INSERT INTO text_graph_state (project_id, synced_at) VALUES ($1, now())
         ON CONFLICT (project_id) DO UPDATE SET synced_at = now()",
    )
    .bind(p.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(n)
}

/// Reads never show a graph older than a minute; writes always sync first.
async fn ensure_fresh(db: &PgPool, p: &ProjectRow) -> ApiResult<()> {
    let fresh: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM text_graph_state
         WHERE project_id = $1 AND synced_at > now() - interval '60 seconds')",
    )
    .bind(p.id)
    .fetch_one(db)
    .await?;
    if !fresh {
        sync(db, p).await?;
    }
    Ok(())
}

/// `$1` project, `$2` anchors (NULL = all), `$3` only sentences that appear in several places.
const SCOPE: &str = "o.project_id = $1 AND ($2::text[] IS NULL OR o.anchor_id = ANY($2))
    AND (NOT $3 OR o.anchor_id IN (SELECT anchor_id FROM text_occurrences WHERE project_id = $1
                                   GROUP BY anchor_id HAVING count(*) > 1))";

pub struct Graph {
    /// Places by sentence.
    pub occs: BTreeMap<String, Vec<Occ>>,
    pub trs: TrMap,
}

// ponytail: loads every shared sentence of the project in memory for the project-wide views; page by anchor
// if a project has more than ~100k shared places.
async fn load(
    db: &PgPool,
    project: Uuid,
    anchors: Option<&[String]>,
    shared: bool,
) -> ApiResult<Graph> {
    let occs: Vec<Occ> = sqlx::query_as(&format!(
        "SELECT o.anchor_id, o.surface, o.locator, o.source_text, o.exact_hash, o.role, o.meta
         FROM text_occurrences o WHERE {SCOPE} ORDER BY o.surface, o.locator"
    ))
    .bind(project)
    .bind(anchors)
    .bind(shared)
    .fetch_all(db)
    .await?;
    // The three stores, read live: nothing is copied, so nothing can drift.
    let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(&format!(
        "SELECT o.surface, o.locator, t.locale, t.value, t.status, t.origin
         FROM text_occurrences o JOIN keys k ON k.project_id = o.project_id AND k.key = o.locator
         JOIN translations t ON t.key_id = k.id
         WHERE o.surface IN ('catalog', 'connector') AND {SCOPE}
         UNION ALL
         SELECT o.surface, o.locator, e.locale, e.value, e.status, e.origin
         FROM text_occurrences o JOIN edge_translations e ON e.project_id = o.project_id
              AND o.locator = e.hash || '|' || e.context
         WHERE o.surface = 'edge' AND {SCOPE}
         UNION ALL
         SELECT o.surface, o.locator, x.locale, x.value, x.status, x.origin
         FROM text_occurrences o JOIN text_translations x USING (project_id, surface, locator)
         WHERE {SCOPE}"
    ))
    .bind(project)
    .bind(anchors)
    .bind(shared)
    .fetch_all(db)
    .await?;
    let mut g = Graph {
        occs: BTreeMap::new(),
        trs: HashMap::new(),
    };
    for o in occs {
        g.occs.entry(o.anchor_id.clone()).or_default().push(o);
    }
    for (surface, locator, locale, value, status, origin) in rows {
        g.trs.entry((surface, locator)).or_default().insert(
            locale,
            Tr {
                value,
                status,
                origin,
            },
        );
    }
    Ok(g)
}

fn syntax_of(p: &ProjectRow, surface: &str) -> Syntax {
    match surface {
        "catalog" => p.syntax,
        "edge" => Syntax::Icu, // edge::tm validates its segments as ICU
        _ => Syntax::Plain,
    }
}

/// Validation errors of `value` in the place `occ` (the same check its own write path runs).
fn errors(
    p: &ProjectRow,
    glossary: &glot_core::glossary::Glossary,
    occ: &Occ,
    value: &str,
    locale: &str,
) -> Option<String> {
    let errs: Vec<String> = keys::check(
        &occ.source_text,
        value,
        locale,
        syntax_of(p, &occ.surface),
        glossary,
    )
    .into_iter()
    .filter(|i| i.severity == Severity::Error)
    .map(|i| i.message)
    .collect();
    (!errs.is_empty()).then(|| errs.join(" "))
}

/// Write the planned translations where they live. A reviewed winner lands reviewed (a person approved this
/// sentence), a machine one lands machine; the catalog history names the text graph as author.
async fn apply(
    conn: &mut PgConnection,
    p: &ProjectRow,
    glossary: &glot_core::glossary::Glossary,
    actions: &[Action],
    sources: &HashMap<(String, String), String>,
    reviewed: bool,
) -> ApiResult<usize> {
    let (status, origin) = if reviewed {
        ("reviewed", "human")
    } else {
        ("machine", "machine")
    };
    let model = (!reviewed).then_some("text-graph");
    let mut n = 0;
    for a in actions.iter().filter(|a| a.action == "write") {
        match a.surface.as_str() {
            "catalog" | "connector" => {
                let key = keys::find_key(&mut *conn, p.id, &a.locator).await?;
                keys::write_translation(
                    &mut *conn,
                    glossary,
                    keys::Write {
                        key: &key,
                        locale: &a.locale,
                        value: &a.value,
                        origin,
                        status,
                        model,
                        score: None,
                        extra_issues: vec![],
                        author: Some("text graph"),
                        syntax: p.syntax,
                    },
                )
                .await?;
            }
            "edge" => {
                let (hash, context) = a.locator.split_once('|').unwrap_or((&a.locator, ""));
                let source = sources
                    .get(&(a.surface.clone(), a.locator.clone()))
                    .map_or("", String::as_str);
                let issues = keys::check(source, &a.value, &a.locale, Syntax::Icu, glossary);
                sqlx::query(
                    "INSERT INTO edge_translations
                        (project_id, hash, context, locale, value, status, origin, model, issues, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
                     ON CONFLICT (project_id, hash, context, locale) DO UPDATE SET value = $5, status = $6,
                        origin = $7, model = $8, score = NULL, issues = $9, updated_at = now()",
                )
                .bind(p.id)
                .bind(hash)
                .bind(context)
                .bind(&a.locale)
                .bind(&a.value)
                .bind(status)
                .bind(origin)
                .bind(model)
                .bind(DbJson(&issues))
                .execute(&mut *conn)
                .await?;
            }
            _ => {
                sqlx::query(
                    "INSERT INTO text_translations (project_id, surface, locator, locale, value, status, origin)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (project_id, surface, locator, locale) DO UPDATE SET value = $5, status = $6,
                        origin = $7, updated_at = now()",
                )
                .bind(p.id)
                .bind(&a.surface)
                .bind(&a.locator)
                .bind(&a.locale)
                .bind(&a.value)
                .bind(status)
                .bind(origin)
                .execute(&mut *conn)
                .await?;
            }
        }
        n += 1;
    }
    Ok(n)
}

fn sources(g: &Graph) -> HashMap<(String, String), String> {
    g.occs
        .values()
        .flatten()
        .map(|o| {
            (
                (o.surface.clone(), o.locator.clone()),
                o.source_text.clone(),
            )
        })
        .collect()
}

// ------------------------------------------------------------------------------------------- API

#[derive(Serialize, sqlx::FromRow)]
pub struct AnchorSummary {
    pub id: String,
    pub text: String,
    pub context: String,
    pub occurrences: i64,
    pub surfaces: Vec<String>,
}

#[derive(Serialize)]
struct Inconsistency {
    anchor_id: String,
    text: String,
    locale: String,
    state: &'static str,
    values: Vec<Group>,
}

/// Everything the project-wide views need: what disagrees, and what automatic propagation would do.
fn survey(
    p: &ProjectRow,
    glossary: &glot_core::glossary::Glossary,
    g: &Graph,
) -> (Vec<Inconsistency>, Vec<Action>) {
    let (mut bad, mut actions) = (vec![], vec![]);
    for (id, occs) in &g.occs {
        for locale in &p.target_locales {
            let a = analyze(occs, &g.trs, locale);
            if let Some(w) = &a.winner {
                actions.extend(plan(&a, w, locale, false, &|o, v| {
                    errors(p, glossary, o, v, locale)
                }));
            }
            if matches!(a.state, "inconsistent" | "conflict") {
                bad.push(Inconsistency {
                    anchor_id: id.clone(),
                    text: occs[0].source_text.clone(),
                    locale: locale.clone(),
                    state: a.state,
                    values: a.groups,
                });
            }
        }
    }
    (bad, actions)
}

async fn overview(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    ensure_fresh(&state.db, &p).await?;
    let anchors: i64 =
        sqlx::query_scalar("SELECT count(*) FROM text_anchors WHERE project_id = $1")
            .bind(p.id)
            .fetch_one(&state.db)
            .await?;
    let surfaces: BTreeMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT surface, count(*) FROM text_occurrences WHERE project_id = $1 GROUP BY surface",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let (shared, cross): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE n > 1), count(*) FILTER (WHERE s > 1)
         FROM (SELECT count(*) AS n, count(DISTINCT surface) AS s FROM text_occurrences
               WHERE project_id = $1 GROUP BY anchor_id) x",
    )
    .bind(p.id)
    .fetch_one(&state.db)
    .await?;
    // Places whose source was reworded while other places still show the old sentence.
    let reworded: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT o.prev_anchor_id, old.source_text, o.anchor_id, new.source_text, count(*),
                (SELECT count(*) FROM text_occurrences r
                 WHERE r.project_id = $1 AND r.anchor_id = o.prev_anchor_id)
         FROM text_occurrences o
         JOIN text_anchors old ON old.project_id = o.project_id AND old.id = o.prev_anchor_id
         JOIN text_anchors new ON new.project_id = o.project_id AND new.id = o.anchor_id
         WHERE o.project_id = $1
         GROUP BY 1, 2, 3, 4 ORDER BY max(o.moved_at) DESC LIMIT 50",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let g = load(&state.db, p.id, None, true).await?;
    let (bad, actions) = survey(&p, &glossary, &g);
    let synced_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT synced_at FROM text_graph_state WHERE project_id = $1")
            .bind(p.id)
            .fetch_optional(&state.db)
            .await?;
    Ok(Json(json!({
        "synced_at": synced_at,
        "anchors": anchors,
        "occurrences": surfaces.values().sum::<i64>(),
        "surfaces": surfaces,
        "shared": shared,
        "cross_surface": cross,
        "inconsistent": bad.iter().filter(|b| b.state == "inconsistent").count(),
        "conflicts": bad.iter().filter(|b| b.state == "conflict").count(),
        "inconsistencies": bad.into_iter().take(100).collect::<Vec<_>>(),
        // What "apply everywhere" would write right now, and what it would leave for a person.
        "fillable": actions.iter().filter(|a| a.action == "write").count(),
        "blocked": actions.iter().filter(|a| a.action == "skip").count(),
        "reworded": reworded.into_iter().map(|(from, from_text, to, to_text, moved, remaining)| json!({
            "from": { "id": from, "text": from_text }, "to": { "id": to, "text": to_text },
            "moved": moved, "remaining": remaining,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<i64>,
}

/// Sentences whose source text, or the address of one of their places (a key, a URL path), contains `q`.
// ponytail: ILIKE scan; a pg_trgm index when a project has more than ~100k sentences.
async fn search(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    ensure_fresh(&state.db, &p).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let q = q.q.as_deref().unwrap_or("").trim().to_string();
    let like = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows: Vec<AnchorSummary> = sqlx::query_as(
        "SELECT a.id, a.source_text AS text, a.context, count(*) AS occurrences,
                array_agg(DISTINCT o.surface) AS surfaces
         FROM text_anchors a JOIN text_occurrences o ON o.project_id = a.project_id AND o.anchor_id = a.id
         WHERE a.project_id = $1 AND ($2 = '' OR a.source_text ILIKE $3 OR EXISTS
            (SELECT 1 FROM text_occurrences x WHERE x.project_id = a.project_id AND x.anchor_id = a.id
             AND (x.locator ILIKE $3 OR x.meta->>'url' ILIKE $3)))
         GROUP BY a.id, a.source_text, a.context
         ORDER BY count(DISTINCT o.surface) DESC, count(*) DESC, a.source_text LIMIT $4",
    )
    .bind(p.id)
    .bind(&q)
    .bind(&like)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "anchors": rows })))
}

async fn anchor(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, String)>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    ensure_fresh(&state.db, &p).await?;
    let (text, context, norm_hash, loose_hash): (String, String, String, String) = sqlx::query_as(
        "SELECT source_text, context, norm_hash, loose_hash FROM text_anchors WHERE project_id = $1 AND id = $2",
    )
    .bind(p.id)
    .bind(&id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("no such sentence in this project (it may have been reworded: search its text)"))?;
    let g = load(&state.db, p.id, Some(std::slice::from_ref(&id)), false).await?;
    let occs = g.occs.get(&id).map(Vec::as_slice).unwrap_or(&[]);
    let glossary = crate::glossary::load(&state.db, p.id).await?;

    let mut per_occ: Vec<BTreeMap<&str, Value>> = vec![BTreeMap::new(); occs.len()];
    let mut locales = vec![];
    for locale in &p.target_locales {
        let a = analyze(occs, &g.trs, locale);
        for (i, c) in a.cells.iter().enumerate() {
            if let Some(t) = c.tr {
                per_occ[i].insert(
                    locale.as_str(),
                    json!({ "value": t.value, "status": t.status, "origin": t.origin, "inherited": c.inherited }),
                );
            }
        }
        let actions = a.winner.as_ref().map_or(vec![], |w| {
            plan(&a, w, locale, false, &|o, v| {
                errors(&p, &glossary, o, v, locale)
            })
        });
        locales.push(json!({
            "locale": locale,
            "state": a.state,
            "values": a.groups,
            "missing": a.missing,
            "winner": a.winner.as_ref().map(|w| &w.value),
            "fillable": actions.iter().filter(|x| x.action == "write").count(),
            "blocked": actions.iter().filter(|x| x.action == "skip").collect::<Vec<_>>(),
        }));
    }

    // Related sentences: same text in another context, same words (case / punctuation), or nearly the same.
    // ponytail: one length-bounded scan per view; an index of trigrams or embeddings when this gets slow.
    let len = text.chars().count() as i32;
    let candidates: Vec<(String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT a.id, a.source_text, a.context, a.norm_hash, a.loose_hash,
                (SELECT count(*) FROM text_occurrences o WHERE o.project_id = a.project_id AND o.anchor_id = a.id)
         FROM text_anchors a WHERE a.project_id = $1 AND a.id <> $2
           AND (a.norm_hash = $3 OR a.loose_hash = $4 OR char_length(a.source_text) BETWEEN $5 AND $6)
         LIMIT 5000",
    )
    .bind(p.id)
    .bind(&id)
    .bind(&norm_hash)
    .bind(&loose_hash)
    .bind(len * 3 / 4)
    .bind(len * 4 / 3 + 1)
    .fetch_all(&state.db)
    .await?;
    let mut related: Vec<(f32, Value)> = candidates
        .into_iter()
        .filter_map(|(rid, rtext, rctx, rnorm, rloose, n)| {
            let (kind, score) = if rnorm == norm_hash {
                ("context", 1.0)
            } else if rloose == loose_hash {
                ("near", 1.0)
            } else {
                let s = similarity(&text, &rtext);
                // Short texts differ by one word and mean something else ("Sign in" / "Sign up").
                if len < 16 || s < 0.8 {
                    return None;
                }
                ("fuzzy", s)
            };
            Some((
                score,
                json!({ "id": rid, "text": rtext, "context": rctx, "kind": kind, "score": score, "occurrences": n }),
            ))
        })
        .collect();
    related.sort_by(|a, b| b.0.total_cmp(&a.0));
    let moved: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT o.surface, o.locator, o.anchor_id, a.source_text FROM text_occurrences o
         JOIN text_anchors a ON a.project_id = o.project_id AND a.id = o.anchor_id
         WHERE o.project_id = $1 AND o.prev_anchor_id = $2 ORDER BY o.moved_at DESC LIMIT 50",
    )
    .bind(p.id)
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "id": id,
        "text": text,
        "context": context,
        "occurrences": occs.iter().zip(per_occ).map(|(o, translations)| json!({
            "surface": o.surface, "locator": o.locator, "text": o.source_text, "role": o.role,
            "meta": o.meta.0, "translations": translations,
        })).collect::<Vec<_>>(),
        "locales": locales,
        "related": related.into_iter().take(10).map(|(_, v)| v).collect::<Vec<_>>(),
        "moved": moved.into_iter().map(|(surface, locator, to, to_text)| json!({
            "surface": surface, "locator": locator, "to": { "id": to, "text": to_text },
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct UnifyBody {
    locale: String,
    /// The translation to show everywhere. Absent = the one propagation would pick on its own.
    value: Option<String>,
    /// Also replace human-reviewed translations that say something else.
    #[serde(default)]
    force: bool,
    #[serde(default)]
    dry_run: bool,
}

fn summary(dry_run: bool, written: usize, actions: Vec<Action>) -> Value {
    json!({
        "dry_run": dry_run,
        "written": written,
        "writes": actions.iter().filter(|a| a.action == "write").count(),
        "skipped": actions.iter().filter(|a| a.action == "skip").count(),
        "actions": actions.into_iter().take(500).collect::<Vec<_>>(),
    })
}

/// One sentence, one locale, one translation in every place it appears.
async fn unify(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath((slug, id)): AppPath<(String, String)>,
    AppJson(b): AppJson<UnifyBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    if !p.target_locales.contains(&b.locale) {
        return Err(AppError::bad_request(format!(
            "'{}' is not a target locale of this project",
            b.locale
        )));
    }
    sync(&state.db, &p).await?;
    let g = load(&state.db, p.id, Some(std::slice::from_ref(&id)), false).await?;
    let occs = g
        .occs
        .get(&id)
        .ok_or_else(|| AppError::not_found("no such sentence in this project"))?;
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let a = analyze(occs, &g.trs, &b.locale);
    let winner = match b.value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        // A person chose it: it lands reviewed. Its placeholders are those of the place it was copied from.
        Some(v) => Winner {
            value: v.to_string(),
            slots: a
                .groups
                .iter()
                .find(|g| g.value == v)
                .map(|g| g.slots.clone())
                .unwrap_or_else(|| parse(&occs[0].source_text).1),
            reviewed: true,
        },
        None => a.winner.clone().ok_or_else(|| {
            AppError::conflict(match a.state {
                "conflict" => "several human-reviewed translations disagree: send the one to keep as `value`",
                "missing" => "this sentence has no translation yet in this locale: send one as `value`",
                _ => "several machine translations disagree and none is reviewed: send the one to keep as `value`",
            })
        })?,
    };
    let actions = plan(&a, &winner, &b.locale, b.force, &|o, v| {
        errors(&p, &glossary, o, v, &b.locale)
    });
    let mut written = 0;
    if !b.dry_run {
        let mut tx = state.db.begin().await?;
        written = apply(
            &mut tx,
            &p,
            &glossary,
            &actions,
            &sources(&g),
            winner.reviewed,
        )
        .await?;
        tx.commit().await?;
    }
    let mut out = summary(b.dry_run, written, actions);
    out["anchor_id"] = json!(id);
    out["locale"] = json!(b.locale);
    out["value"] = json!(winner.value);
    Ok(Json(out))
}

#[derive(Deserialize)]
struct PropagateBody {
    #[serde(default)]
    dry_run: bool,
}

/// Translate once, apply everywhere: for every sentence shown in several places, the translation nobody disputes
/// goes to the places that lack it. Never guesses between two translations.
async fn propagate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<PropagateBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    sync(&state.db, &p).await?;
    let glossary = crate::glossary::load(&state.db, p.id).await?;
    let g = load(&state.db, p.id, None, true).await?;
    let (bad, actions) = survey(&p, &glossary, &g);
    let mut written = 0;
    if !b.dry_run {
        let src = sources(&g);
        // The winner of each (sentence, locale) decides the status its copies land with.
        let mut reviewed: HashSet<(String, String)> = HashSet::new();
        for (id, occs) in &g.occs {
            for l in &p.target_locales {
                if analyze(occs, &g.trs, l).winner.is_some_and(|w| w.reviewed) {
                    reviewed.insert((id.clone(), l.clone()));
                }
            }
        }
        let (human, machine): (Vec<Action>, Vec<Action>) = actions
            .iter()
            .cloned()
            .partition(|a| reviewed.contains(&(a.anchor_id.clone(), a.locale.clone())));
        let mut tx = state.db.begin().await?;
        written += apply(&mut tx, &p, &glossary, &human, &src, true).await?;
        written += apply(&mut tx, &p, &glossary, &machine, &src, false).await?;
        tx.commit().await?;
    }
    let mut out = summary(b.dry_run, written, actions);
    out["unsettled"] = json!(bad.len());
    Ok(Json(out))
}

async fn sync_now(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let n = sync(&state.db, &p).await?;
    Ok(Json(json!({ "occurrences": n })))
}

// ------------------------------------------------------------------------------------------- importer contract

#[derive(Deserialize)]
#[serde(untagged)]
enum ImportedTr {
    Value(String),
    Full {
        value: String,
        #[serde(default)]
        reviewed: bool,
    },
}

#[derive(Deserialize)]
struct ImportItem {
    locator: String,
    text: String,
    #[serde(default)]
    role: String,
    context: Option<String>,
    url: Option<String>,
    #[serde(default)]
    meta: serde_json::Map<String, Value>,
    #[serde(default)]
    translations: BTreeMap<String, ImportedTr>,
}

#[derive(Deserialize)]
struct ImportBody {
    surface: String,
    /// The payload is the whole surface: places of this surface it does not list are removed.
    #[serde(default)]
    replace: bool,
    items: Vec<ImportItem>,
}

const MAX_ITEMS: usize = 5000;

fn validate_surface(s: &str) -> ApiResult<()> {
    let ok = (2..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
        && s.as_bytes()[0].is_ascii_lowercase();
    if !ok {
        return Err(AppError::bad_request(
            "surface must be 2-32 chars of a-z, 0-9, '-' and '_', starting with a letter (e.g. \"email\")",
        ));
    }
    if MIRRORED.contains(&s) {
        return Err(AppError::bad_request(format!(
            "'{s}' is read by glot from its own store: pick another surface name"
        )));
    }
    Ok(())
}

/// The importer contract (documented in web/content/docs/text-graph.md): any system that shows text declares
/// its sentences and where they are; glot answers with the sentence each one is.
async fn import(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppJson(b): AppJson<ImportBody>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Admin).await?;
    validate_surface(&b.surface)?;
    if b.items.len() > MAX_ITEMS {
        return Err(AppError::bad_request(format!(
            "{} items: send at most {MAX_ITEMS} per request (several requests with replace=false)",
            b.items.len()
        )));
    }
    let mut seen_locators = HashSet::new();
    for (i, it) in b.items.iter().enumerate() {
        let bad = |what: &str| AppError::bad_request(format!("items[{i}]: {what}"));
        if it.locator.is_empty()
            || it.locator.len() > 500
            || it.locator.chars().any(char::is_control)
        {
            return Err(bad(
                "locator must be 1-500 characters without control characters",
            ));
        }
        if !seen_locators.insert(&it.locator) {
            return Err(bad(&format!("locator '{}' appears twice", it.locator)));
        }
        if it.text.trim().is_empty() || it.text.len() > 20_000 {
            return Err(bad("text must be 1-20000 bytes and not blank"));
        }
        for locale in it.translations.keys() {
            if !p.target_locales.contains(locale) {
                return Err(bad(&format!(
                    "'{locale}' is not a target locale of this project ({})",
                    p.target_locales.join(", ")
                )));
            }
        }
    }
    sync(&state.db, &p).await?;

    let mut tx = state.db.begin().await?;
    let seen = lock(&mut tx, p.id).await?;
    let locators: Vec<String> = b.items.iter().map(|i| i.locator.clone()).collect();
    let before: HashMap<String, String> = sqlx::query_as::<_, (String, String)>(
        "SELECT locator, source_text FROM text_occurrences
         WHERE project_id = $1 AND surface = $2 AND locator = ANY($3)",
    )
    .bind(p.id)
    .bind(&b.surface)
    .bind(&locators)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    // A reworded text invalidates the translations stored for the old wording.
    let reworded: Vec<String> = b
        .items
        .iter()
        .filter(|i| before.get(&i.locator).is_some_and(|t| *t != i.text))
        .map(|i| i.locator.clone())
        .collect();
    sqlx::query(
        "DELETE FROM text_translations WHERE project_id = $1 AND surface = $2 AND locator = ANY($3)",
    )
    .bind(p.id)
    .bind(&b.surface)
    .bind(&reworded)
    .execute(&mut *tx)
    .await?;

    let mut trs = vec![];
    let rows: Vec<NewOcc> = b
        .items
        .into_iter()
        .map(|it| {
            let mut meta = it.meta;
            if let Some(url) = it.url {
                meta.insert("url".into(), json!(url));
            }
            for (locale, t) in it.translations {
                let (value, reviewed) = match t {
                    ImportedTr::Value(value) => (value, false),
                    ImportedTr::Full { value, reviewed } => (value, reviewed),
                };
                if !value.is_empty() {
                    trs.push((it.locator.clone(), locale, value, reviewed));
                }
            }
            NewOcc {
                surface: b.surface.clone(),
                locator: it.locator,
                text: it.text,
                role: it.role,
                meta: Value::Object(meta),
                context: it.context,
                exact_hash: None,
            }
        })
        .collect();
    let ids = upsert(&mut tx, &p, rows, seen).await?;
    for (locator, locale, value, reviewed) in trs {
        sqlx::query(
            "INSERT INTO text_translations (project_id, surface, locator, locale, value, status, origin)
             VALUES ($1, $2, $3, $4, $5, $6, 'imported')
             ON CONFLICT (project_id, surface, locator, locale) DO UPDATE SET value = $5, status = $6,
                origin = 'imported', updated_at = now()",
        )
        .bind(p.id)
        .bind(&b.surface)
        .bind(&locator)
        .bind(&locale)
        .bind(&value)
        .bind(if reviewed { "reviewed" } else { "machine" })
        .execute(&mut *tx)
        .await?;
    }
    let removed = if b.replace {
        sqlx::query(
            "DELETE FROM text_occurrences WHERE project_id = $1 AND surface = $2 AND seen_at < $3",
        )
        .bind(p.id)
        .bind(&b.surface)
        .bind(seen)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        0
    };
    drop_orphans(&mut tx, p.id).await?;
    // Where else glot already knows each sentence.
    let anchor_ids: Vec<String> = ids.iter().map(|i| i.anchor_id.clone()).collect();
    let elsewhere: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT anchor_id, count(*) FROM text_occurrences
         WHERE project_id = $1 AND anchor_id = ANY($2) AND surface <> $3 GROUP BY anchor_id",
    )
    .bind(p.id)
    .bind(&anchor_ids)
    .bind(&b.surface)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    tx.commit().await?;
    Ok(Json(json!({
        "surface": b.surface,
        "imported": ids.len(),
        "removed": removed,
        "items": locators.iter().zip(&ids).map(|(locator, id)| json!({
            "locator": locator,
            "anchor_id": id.anchor_id,
            "context": id.context,
            "elsewhere": elsewhere.get(&id.anchor_id).copied().unwrap_or(0),
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct ExportQuery {
    surface: String,
    locale: String,
}

/// What a surface should show in `locale`: its own translation, else the one the graph knows for the sentence
/// (`from: "graph"`), rewritten for this place's placeholders. Nothing is written.
async fn export(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<ExportQuery>,
) -> ApiResult<Json<Value>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    if !p.target_locales.contains(&q.locale) {
        return Err(AppError::bad_request(format!(
            "'{}' is not a target locale of this project",
            q.locale
        )));
    }
    sync(&state.db, &p).await?;
    let anchors: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT anchor_id FROM text_occurrences WHERE project_id = $1 AND surface = $2",
    )
    .bind(p.id)
    .bind(&q.surface)
    .fetch_all(&state.db)
    .await?;
    let g = load(&state.db, p.id, Some(&anchors), false).await?;
    let (mut strings, mut missing) = (BTreeMap::new(), vec![]);
    for occs in g.occs.values() {
        let a = analyze(occs, &g.trs, &q.locale);
        for c in a.cells.iter().filter(|c| c.occ.surface == q.surface) {
            let known = match (c.tr.filter(|_| c.canon.is_some()), &a.winner) {
                (Some(t), _) => Some((t.value.clone(), t.status.clone(), "own")),
                (None, Some(w)) => retarget(&w.value, &w.slots, &c.slots).map(|v| {
                    let status = if w.reviewed { "reviewed" } else { "machine" };
                    (v, status.to_string(), "graph")
                }),
                (None, None) => None,
            };
            match known {
                Some((value, status, from)) => {
                    strings.insert(
                        c.occ.locator.clone(),
                        json!({ "value": value, "status": status, "from": from, "anchor_id": c.occ.anchor_id }),
                    );
                }
                None => missing.push(c.occ.locator.clone()),
            }
        }
    }
    Ok(Json(json!({
        "surface": q.surface,
        "locale": q.locale,
        "strings": strings,
        "missing": missing,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_syntaxes_are_one_sentence() {
        let id = |t: &str| identify("en", t, "", None);
        let a = id("Hello {name}, you have {count} messages");
        for same in [
            "Hello {{name}}, you have {{ count }} messages",
            "Hello %s, you have %d messages",
            "Hello %1$s, you have %2$d messages",
            "Hello %{name},  you have %{count} messages",
            "Hello ${name}, you have\u{a0}${count} messages",
        ] {
            assert_eq!(id(same).anchor_id, a.anchor_id, "{same}");
        }
        // Different wording, different arity, ICU plural block: other sentences.
        for other in [
            "Hello {name}, you have {count} message",
            "Hello {name}",
            "Hello {name}, you have {count, plural, one {# message} other {# messages}}",
        ] {
            assert_ne!(id(other).anchor_id, a.anchor_id, "{other}");
        }
        // The exact hash still tells them apart, and equals glot Edge's segment hash.
        assert_ne!(
            id("Hello %s, you have %d messages").exact_hash,
            a.exact_hash
        );
        assert_eq!(id("Save  now").exact_hash, text_hash("en", "Save now"));
        // Typography is not meaning; percent signs are not placeholders.
        assert_eq!(
            id("Don\u{2019}t wait\u{2026}").anchor_id,
            id("Don't wait...").anchor_id
        );
        assert_eq!(parse("100% sure, 50%% off").1, Vec::<String>::new());
    }

    #[test]
    fn near_duplicates_are_linked_not_merged() {
        let id = |t: &str| identify("en", t, "", None);
        let (a, b) = (id("Save changes"), id("save changes."));
        assert_ne!(a.anchor_id, b.anchor_id);
        assert_eq!(a.loose_hash, b.loose_hash);
        assert!(
            similarity(
                "Your order has been shipped today",
                "Your order has been shipped"
            ) > 0.8
        );
        assert!(similarity("Your order has been shipped", "Delete this project forever") < 0.3);
    }

    #[test]
    fn short_action_text_is_its_own_sentence() {
        let verb = identify("en", "Save", "button", None);
        let noun = identify("en", "Save", "td", None);
        assert_ne!(verb.anchor_id, noun.anchor_id);
        assert_eq!(verb.norm_hash, noun.norm_hash);
        assert_eq!(noun.anchor_id, identify("en", "Save", "", None).anchor_id);
        // A full sentence means the same on a button and in a paragraph.
        let long = "Save your changes before leaving";
        assert_eq!(
            identify("en", long, "button", None).anchor_id,
            identify("en", long, "p", None).anchor_id
        );
        assert_ne!(
            identify("en", long, "", Some("legal")).anchor_id,
            identify("en", long, "", None).anchor_id
        );
    }

    #[test]
    fn translations_follow_the_placeholders_of_their_place() {
        let from = parse("Hello {name}, {count} new").1;
        let to = parse("Hello %1$s, %2$d new").1;
        // The translation reorders its arguments: numbered placeholders follow.
        assert_eq!(
            retarget("{count} nouveaux, bonjour {name}", &from, &to).as_deref(),
            Some("%2$d nouveaux, bonjour %1$s")
        );
        assert_eq!(
            canon("Bonjour {name}", &from),
            canon("Bonjour  {name}", &from)
        );
        // Unknown placeholder or different arity: refused, never guessed.
        assert_eq!(retarget("Bonjour {nom}", &from, &to), None);
        assert_eq!(
            retarget("Bonjour {name}", &from, &parse("Hello %s").1),
            None
        );
        // Same placeholders: untouched, whatever the value contains.
        assert_eq!(retarget("x {y}", &from, &from).as_deref(), Some("x {y}"));
    }
}
