//! glot-mcp: MCP server (stdio) over a local glot workspace.
//!
//! Not a CRUD wrapper: tools give the coding agent live context (where a key is used,
//! its UI role, neighbors, glossary, issues) and guardrails (duplicate detection,
//! syntax validation, no silent overwrite).
use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    handler::server::{
        router::prompt::PromptRouter, router::tool::ToolRouter, wrapper::Parameters,
    },
    model::*,
    prompt, prompt_handler, prompt_router,
    service::{ElicitationError, RequestContext},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, OwnedMappedMutexGuard, OwnedMutexGuard};

use glot_core::{Locale, glossary::Term, lock::Plan, validate};
use glot_llm::engine::{TranslateOptions, Workspace};

const MAX_ITEMS: usize = 100;

mod review;

/// Serve MCP over stdio for the workspace found from `start`. Logs go to stderr only.
pub async fn serve_stdio(start: PathBuf) -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let server = GlotServer::open(&start)?;
    tracing::info!("glot MCP server ready");
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[derive(Clone)]
pub struct GlotServer {
    // None until a glot.toml is found: the server must start in any repo (plugins, registries
    // probe it) and tell the agent to run `glot init` instead of dying.
    ws: Arc<Mutex<Option<Workspace>>>,
    start: PathBuf,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl GlotServer {
    pub fn open(start: &Path) -> anyhow::Result<Self> {
        let ws = Workspace::open(start).ok();
        if ws.is_none() {
            tracing::warn!(
                "no glot.toml from {}: tools will ask to run `glot init`",
                start.display()
            );
        }
        Ok(Self::build(ws, start.to_path_buf()))
    }

    pub fn new(ws: Workspace) -> Self {
        let start = ws.root.clone();
        Self::build(Some(ws), start)
    }

    fn build(ws: Option<Workspace>, start: PathBuf) -> Self {
        Self {
            ws: Arc::new(Mutex::new(ws)),
            start,
            tool_router: Self::tool_router() + Self::review_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    /// Reload from disk (the coding agent edits files between calls), then run `f`.
    /// Runs in its own task so a panic inside the engine becomes an error, not a dead server.
    async fn with_ws<F, Fut>(&self, f: F) -> Result<Value, String>
    where
        F: FnOnce(OwnedMappedMutexGuard<Option<Workspace>, Workspace>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
        let ws = self.ws.clone();
        let start = self.start.clone();
        let task = tokio::spawn(async move {
            let mut slot = ws.lock_owned().await;
            if slot.is_none() {
                *slot = Some(Workspace::open(&start).map_err(|_| {
                    format!(
                        "No glot project in {}. Run `glot init --yes` (or `npx glot-i18n init --yes`) in the app root, then retry.",
                        start.display()
                    )
                })?);
            }
            let mut g = OwnedMutexGuard::map(slot, |o| o.as_mut().expect("set above"));
            g.reload().map_err(|e| {
                format!(
                    "could not reload the project ({e}); fix glot.toml / catalog JSON and retry"
                )
            })?;
            f(g).await
        });
        match task.await {
            Ok(r) => r,
            Err(e) => Err(format!("internal error in glot engine: {e}")),
        }
    }

    async fn run<F, Fut>(&self, f: F) -> CallToolResult
    where
        F: FnOnce(OwnedMappedMutexGuard<Option<Workspace>, Workspace>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
        match self.with_ws(f).await {
            Ok(v) => CallToolResult::success(vec![ContentBlock::text(pretty(&v))]),
            Err(m) => CallToolResult::error(vec![ContentBlock::text(m)]),
        }
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn plan_summary(p: &Plan) -> Value {
    let s = |v: &Vec<(Locale, String)>| json!({ "count": v.len(), "sample": v.iter().take(20).map(|(l, k)| format!("{l}:{k}")).collect::<Vec<_>>() });
    json!({ "missing": s(&p.missing), "stale": s(&p.stale), "orphaned": s(&p.orphaned),
            "manual_edits": s(&p.manual_edits), "untracked": s(&p.untracked) })
}

fn status_json(ws: &Workspace) -> Value {
    let st = ws.status();
    json!({ "source_locale": st.source_locale, "keys": st.keys, "locales": st.locales, "plan": plan_summary(&st.plan) })
}

fn check_locale(ws: &Workspace, locale: &str) -> Result<(), String> {
    if ws.config.all_locales().iter().any(|l| l == locale) {
        Ok(())
    } else {
        Err(format!(
            "unknown locale `{locale}`; configured locales: {}",
            ws.config.all_locales().join(", ")
        ))
    }
}

fn norm_path(p: &str) -> &str {
    p.trim_start_matches("./")
}

// ---------- tool params ----------

#[derive(Deserialize, JsonSchema)]
pub struct GetContextParams {
    /// Source file path relative to the project root (e.g. `src/app/settings/page.tsx`).
    pub file: Option<String>,
    /// Translation keys to inspect (e.g. `["auth.login.cta"]`).
    pub keys: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProposeParams {
    /// The user-visible source text, in the source locale, with placeholders in the project syntax (e.g. `Hello {name}`).
    pub text: String,
    /// Key you would like to use (dot path, e.g. `settings.billing.save`).
    pub suggested_key: Option<String>,
    /// File where the string will be used, relative to the project root.
    pub file: Option<String>,
    /// UI role: button | heading | label | placeholder | aria | title | link | toast | text.
    pub ui_role: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddStringParams {
    /// Dot-path key (e.g. `settings.billing.save`).
    pub key: String,
    /// Source text in the source locale.
    pub text: String,
    /// Note for translators (where/how the string is shown, max length, tone).
    pub note: Option<String>,
    /// Replace an existing key whose text differs. Default false.
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TranslateParams {
    /// Target locales (default: all configured target locales).
    pub locales: Option<Vec<String>>,
    /// Only these keys (default: the whole delta = missing + stale).
    pub keys: Option<Vec<String>>,
    /// Also retranslate up-to-date and hand-edited entries. Default false.
    pub force: Option<bool>,
    /// Only compute the plan and estimated cost. Default false.
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CheckParams {
    /// Only this locale.
    pub locale: Option<String>,
    /// Skip the first N issues (pagination). Default 0.
    pub offset: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetTranslationParams {
    /// Target locale (e.g. `fr`).
    pub locale: String,
    pub key: String,
    /// The translated text.
    pub value: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindHardcodedParams {
    /// File or directory relative to the project root (default: every file matched by `[extract].include`).
    pub path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GlossaryLookupParams {
    /// Return only terms occurring in this text (default: the whole glossary).
    pub text: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GlossaryUpsertParams {
    /// The source-language term (case-insensitive match on existing terms).
    pub term: String,
    /// Required translations per locale, e.g. `{"fr": "Espace de travail"}`.
    pub translations: Option<std::collections::BTreeMap<String, String>>,
    /// Keep the term verbatim in every locale (brand / product names).
    pub do_not_translate: Option<bool>,
    pub description: Option<String>,
    /// Replace conflicting existing translations without asking. Default false.
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RemoveKeysParams {
    /// Keys to delete from every catalog and from glot.lock.
    pub keys: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct I18nizeFileParams {
    /// Source file relative to the project root (e.g. `app/invoices/page.tsx`).
    pub path: String,
    /// Only return the diff and the keys; write nothing. Default true.
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GlossaryChoice {
    /// true = replace with the new translation(s), false = keep the existing ones.
    pub use_new: bool,
}
rmcp::elicit_safe!(GlossaryChoice);

#[derive(Deserialize, JsonSchema)]
pub struct PathArg {
    /// Source file to internationalize, relative to the project root.
    pub path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct LocaleArg {
    /// Target locale to review (e.g. `fr`).
    pub locale: String,
}

// ---------- tools ----------

#[tool_router(router = tool_router)]
impl GlotServer {
    #[tool(
        description = "Localization overview: number of keys, per-locale counts (translated / missing / stale / reviewed / errors / warnings) and the pending plan (what `translate` would do). Call this first to orient yourself."
    )]
    async fn status(&self) -> CallToolResult {
        self.run(|ws| async move { Ok(status_json(&ws)) }).await
    }

    #[tool(
        description = "Live i18n context for a source file and/or keys. For a file: the translation keys it uses, keys used but missing from the source catalog, and user-visible hardcoded strings not yet internationalized. For every key: where it is used (file:line), UI role, component, translator note, sibling keys, its value in every locale, open validation issues, plus the glossary terms that apply. Use before editing UI text so new strings stay consistent."
    )]
    async fn get_context(&self, Parameters(p): Parameters<GetContextParams>) -> CallToolResult {
        self.run(move |ws| async move {
            if p.file.is_none() && p.keys.is_none() {
                return Err("pass `file` (path relative to the project root) and/or `keys`".into());
            }
            let report = match &p.file {
                Some(f) => {
                    let f = norm_path(f);
                    let src = std::fs::read_to_string(ws.root.join(f))
                        .map_err(|e| format!("cannot read `{f}` under {}: {e}", ws.root.display()))?;
                    glot_extract::extract_source(f, &src, &ws.config.extract)
                }
                None => ws.extract().map_err(|e| format!("extraction failed: {e}"))?,
            };
            let contexts = ws.contexts(&report);
            let source = ws.source();
            let mut keys: Vec<String> = p.keys.clone().unwrap_or_else(|| report.usages.keys().cloned().collect());
            keys.dedup();
            let total_keys = keys.len();
            keys.truncate(MAX_ITEMS);
            let findings = ws.check();
            let mut terms = BTreeSet::new();
            let items: Vec<Value> = keys
                .iter()
                .map(|k| {
                    let values: serde_json::Map<String, Value> = ws
                        .catalogs
                        .iter()
                        .map(|(l, c)| (l.clone(), c.entries.get(k).map_or(Value::Null, |v| json!(v))))
                        .collect();
                    if let Some(s) = source.entries.get(k) {
                        terms.extend(ws.glossary.matches(s).into_iter().cloned().map(|t| t.term));
                    }
                    let issues: Vec<Value> = findings
                        .iter()
                        .filter(|f| &f.key == k)
                        .map(|f| json!({ "locale": f.locale, "code": f.issue.code, "severity": f.issue.severity, "message": f.issue.message }))
                        .collect();
                    json!({ "key": k, "in_source_catalog": source.entries.contains_key(k),
                            "context": contexts.get(k), "values": values, "issues": issues })
                })
                .collect();
            let glossary: Vec<&Term> = ws.glossary.terms.iter().filter(|t| terms.contains(&t.term)).collect();
            let mut out = json!({ "keys": items, "total_keys": total_keys, "glossary": glossary });
            if p.file.is_some() {
                out["missing_from_catalog"] = json!(glot_extract::missing_keys(&report, source));
                out["hardcoded"] = json!(report.hardcoded);
                out["dynamic_keys"] = json!(report.dynamic);
            }
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "Dry-run BEFORE adding any new UI string. Returns the normalized key, existing keys with the same/near-identical text (if any: REUSE that key instead of creating a new one), glossary terms the text contains, whether the key already exists with another text, and placeholder/syntax problems. Creates nothing."
    )]
    async fn propose_string(&self, Parameters(p): Parameters<ProposeParams>) -> CallToolResult {
        self.run(move |ws| async move {
            let prop = ws.propose(&p.text, p.suggested_key.as_deref());
            let existing = ws.source().entries.get(&prop.key).cloned();
            let mut advice = Vec::new();
            if let Some((k, _)) = prop.duplicates.first() {
                advice.push(format!("Reuse `{k}` instead of creating a new key (same text already exists)."));
            }
            if let Some(e) = existing.as_ref().filter(|e| **e != p.text) {
                advice.push(format!("Key `{}` already exists with a different text ({e:?}): pick another key or call add_string with overwrite: true.", prop.key));
            }
            let syntax = validate::placeholders(&p.text, ws.config.syntax);
            if let Err(i) = &syntax {
                advice.push(format!("Fix the source text first: {}", i.message));
            }
            if advice.is_empty() {
                advice.push(format!("OK: call add_string with key `{}`.", prop.key));
            }
            let note = match (&p.ui_role, &p.file) {
                (Some(r), Some(f)) => Some(format!("{r} in {f}")),
                (Some(r), None) => Some(r.clone()),
                (None, Some(f)) => Some(format!("used in {f}")),
                _ => None,
            };
            Ok(json!({
                "key": prop.key, "key_exists": prop.key_exists, "existing_text": existing,
                "duplicates": prop.duplicates.iter().map(|(k, v)| json!({"key": k, "text": v})).collect::<Vec<_>>(),
                "glossary_terms": prop.glossary_terms,
                "placeholders": syntax.as_ref().ok(), "syntax_issue": syntax.err(),
                "suggested_note": note, "advice": advice,
            }))
        })
        .await
    }

    #[tool(
        description = "Add a source string (source locale) with an optional translator note, after propose_string. Validates the message syntax (placeholders, plurals, tags) and refuses to change an existing key with a different text unless overwrite: true. Then replace the literal in the code with the key and call `translate`."
    )]
    async fn add_string(&self, Parameters(p): Parameters<AddStringParams>) -> CallToolResult {
        self.run(move |mut ws| async move {
            let existing = ws.source().entries.get(&p.key).cloned();
            if let Some(e) = existing.as_ref().filter(|e| **e != p.text && !p.overwrite.unwrap_or(false)) {
                return Err(format!("key `{}` already exists with text {e:?}; use another key, reuse it, or pass overwrite: true", p.key));
            }
            let placeholders = validate::placeholders(&p.text, ws.config.syntax)
                .map_err(|i| format!("invalid source text for syntax {:?}: {} (nothing written)", ws.config.syntax, i.message))?;
            let dups: Vec<String> = ws.propose(&p.text, Some(&p.key)).duplicates.into_iter().map(|(k, _)| k).filter(|k| *k != p.key).collect();
            let action = match &existing {
                None => "created",
                Some(e) if *e == p.text => "unchanged",
                Some(_) => "updated",
            };
            ws.add_source(&p.key, &p.text, p.note.as_deref()).map_err(|e| format!("could not write the source catalog: {e}"))?;
            let mut out = json!({ "key": p.key, "action": action, "placeholders": placeholders,
                                  "next": format!("replace the literal in code with `{}`, then call translate {{\"keys\": [\"{}\"]}}", p.key, p.key) });
            if !dups.is_empty() {
                out["warning"] = json!(format!("same text already exists under {dups:?}; consider reusing it and remove_keys this one"));
            }
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "Translate the delta (missing + stale keys) with the configured LLM, guarded by deterministic validation and the glossary; hand-edited translations are never overwritten unless force: true. Returns a compact summary only (counts, rejected keys with reasons, cost) — never whole catalogs. Use dry_run: true to see the plan and estimated cost first."
    )]
    async fn translate(&self, Parameters(p): Parameters<TranslateParams>) -> CallToolResult {
        self.run(move |mut ws| async move {
            if let Some(ls) = &p.locales {
                for l in ls {
                    check_locale(&ws, l)?;
                }
            }
            let dry_run = p.dry_run.unwrap_or(false);
            let opts = TranslateOptions { locales: p.locales, keys: p.keys, force: p.force.unwrap_or(false), dry_run };
            let r = ws.translate(opts).await;
            let r = r.map_err(|e| format!("translation failed: {e}"))?;
            let written: serde_json::Map<String, Value> = r.written.iter().map(|(l, v)| (l.clone(), json!(v.len()))).collect();
            let mut rejected = Vec::new();
            let mut rejected_total = 0;
            for (l, items) in &r.rejected {
                rejected_total += items.len();
                for t in items.iter().take(MAX_ITEMS.saturating_sub(rejected.len())) {
                    let reasons: Vec<&str> = t.issues.iter().map(|i| i.message.as_str()).collect();
                    rejected.push(json!({ "locale": l, "key": t.key, "reasons": reasons }));
                }
            }
            let next = if dry_run {
                "call translate again without dry_run to apply"
            } else if rejected_total > 0 {
                "fix rejected keys by hand with set_translation (or adjust the source / glossary) then call check"
            } else {
                "call check to confirm there are no remaining issues"
            };
            Ok(json!({
                "dry_run": dry_run, "written": written, "rejected_total": rejected_total, "rejected": rejected,
                "cost_usd": r.usage.cost_usd, "estimated_cost_usd": r.estimated_cost_usd,
                "tokens": { "input": r.usage.input_tokens, "output": r.usage.output_tokens },
                "pending": if dry_run { plan_summary(&ws.status().plan) } else { Value::Null },
                "next": next,
            }))
        })
        .await
    }

    #[tool(
        description = "Deterministic QA over every translation (no network): placeholders, ICU/plural syntax, tags, length, glossary and do-not-translate violations. Returns at most 100 issues per call with the total; use offset to page."
    )]
    async fn check(&self, Parameters(p): Parameters<CheckParams>) -> CallToolResult {
        self.run(move |ws| async move {
            if let Some(l) = &p.locale {
                check_locale(&ws, l)?;
            }
            let all: Vec<_> = ws.check().into_iter().filter(|f| p.locale.as_ref().is_none_or(|l| *l == f.locale)).collect();
            let errors = all.iter().filter(|f| f.issue.severity == validate::Severity::Error).count();
            let offset = p.offset.unwrap_or(0);
            let page: Vec<Value> = all
                .iter()
                .skip(offset)
                .take(MAX_ITEMS)
                .map(|f| json!({ "locale": f.locale, "key": f.key, "code": f.issue.code, "severity": f.issue.severity, "message": f.issue.message }))
                .collect();
            let next_offset = (offset + page.len() < all.len()).then_some(offset + page.len());
            Ok(json!({ "total": all.len(), "errors": errors, "warnings": all.len() - errors, "offset": offset, "next_offset": next_offset, "issues": page }))
        })
        .await
    }

    #[tool(
        description = "Set one translation by hand (recorded as human, never overwritten by translate). Validated against the source: on errors nothing is written and the issues are returned."
    )]
    async fn set_translation(
        &self,
        Parameters(p): Parameters<SetTranslationParams>,
    ) -> CallToolResult {
        self.run(move |mut ws| async move {
            check_locale(&ws, &p.locale)?;
            if p.locale == ws.config.source_locale {
                return Err(
                    "this is the source locale: use add_string to change source texts".into(),
                );
            }
            if !ws.source().entries.contains_key(&p.key) {
                return Err(format!(
                    "key `{}` is not in the source catalog; add it with add_string first",
                    p.key
                ));
            }
            let issues = ws
                .set_translation(&p.locale, &p.key, &p.value)
                .map_err(|e| format!("could not write: {e}"))?;
            if validate::has_errors(&issues) {
                return Err(format!(
                    "not written, validation errors: {}",
                    pretty(&json!(issues))
                ));
            }
            Ok(json!({ "written": true, "locale": p.locale, "key": p.key, "issues": issues }))
        })
        .await
    }

    #[tool(
        description = "Find user-visible string literals not wrapped in a translation call, each with file:line, UI role, component and a suggested key. Use it to internationalize code yourself: propose_string -> add_string -> replace the literal with the t() call."
    )]
    async fn find_hardcoded(
        &self,
        Parameters(p): Parameters<FindHardcodedParams>,
    ) -> CallToolResult {
        self.run(move |ws| async move {
            let filter = p
                .path
                .as_deref()
                .map(norm_path)
                .map(|s| s.trim_end_matches('/').to_string());
            let mut hard = match &filter {
                Some(f) if ws.root.join(f).is_file() => {
                    let src = std::fs::read_to_string(ws.root.join(f))
                        .map_err(|e| format!("cannot read `{f}`: {e}"))?;
                    glot_extract::extract_source(f, &src, &ws.config.extract).hardcoded
                }
                _ => {
                    ws.extract()
                        .map_err(|e| format!("extraction failed: {e}"))?
                        .hardcoded
                }
            };
            if let Some(f) = &filter {
                hard.retain(|h| norm_path(&h.at.path).starts_with(f.as_str()));
            }
            let total = hard.len();
            hard.truncate(MAX_ITEMS);
            Ok(json!({ "total": total, "items": hard }))
        })
        .await
    }

    #[tool(
        description = "Glossary terms (required translations, forbidden variants, do-not-translate names). With `text`, only the terms occurring in it."
    )]
    async fn glossary_lookup(
        &self,
        Parameters(p): Parameters<GlossaryLookupParams>,
    ) -> CallToolResult {
        self.run(move |ws| async move {
            let terms: Vec<&Term> = match &p.text {
                Some(t) => ws.glossary.matches(t),
                None => ws.glossary.terms.iter().collect(),
            };
            Ok(json!({ "terms": terms }))
        })
        .await
    }

    #[tool(
        description = "Add or update a glossary term (enforced on every future translation). If the term already has a different translation for a locale, the human is asked which one to keep when the client supports it; otherwise the conflict is returned and nothing is written unless overwrite: true."
    )]
    async fn glossary_upsert(
        &self,
        Parameters(p): Parameters<GlossaryUpsertParams>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let peer = ctx.peer.clone();
        self.run(move |mut ws| async move {
            let existing = ws.glossary.terms.iter().find(|t| t.term.eq_ignore_ascii_case(&p.term)).cloned();
            let mut term = existing.clone().unwrap_or_else(|| Term { term: p.term.clone(), ..Default::default() });
            let new = p.translations.unwrap_or_default();
            let conflicts: Vec<Value> = new
                .iter()
                .filter_map(|(l, v)| term.translations.get(l).filter(|old| *old != v).map(|old| json!({"locale": l, "existing": old, "proposed": v})))
                .collect();
            let mut use_new = p.overwrite.unwrap_or(false) || conflicts.is_empty();
            if !use_new {
                let msg = format!("Glossary term \"{}\" already has other translations: {}. Replace them with the proposed ones?", term.term, json!(conflicts));
                match peer.elicit::<GlossaryChoice>(msg).await {
                    Ok(Some(c)) => use_new = c.use_new,
                    Ok(None) | Err(ElicitationError::UserDeclined) | Err(ElicitationError::UserCancelled) => {
                        return Ok(json!({ "written": false, "conflicts": conflicts, "reason": "the user did not confirm" }));
                    }
                    Err(_) => {
                        return Ok(json!({ "written": false, "conflicts": conflicts,
                            "next": "ask the user which translation to keep, then call glossary_upsert again with overwrite: true (or with only the non-conflicting translations)" }));
                    }
                }
            }
            for (l, v) in new {
                if use_new || !term.translations.contains_key(&l) {
                    term.translations.insert(l, v);
                }
            }
            if let Some(d) = p.do_not_translate {
                term.do_not_translate = d;
            }
            if p.description.is_some() {
                term.description = p.description;
            }
            ws.glossary.upsert(term.clone());
            ws.save_glossary().map_err(|e| format!("could not write glossary.toml: {e}"))?;
            Ok(json!({ "written": true, "action": if existing.is_some() { "updated" } else { "created" }, "term": term,
                       "next": "call check to see existing translations that now violate the glossary" }))
        })
        .await
    }

    #[tool(
        description = "Delete keys from every catalog and glot.lock. Warns if a key is still referenced in code (remove the t() call too)."
    )]
    async fn remove_keys(&self, Parameters(p): Parameters<RemoveKeysParams>) -> CallToolResult {
        self.run(move |mut ws| async move {
            let unknown: Vec<&String> = p.keys.iter().filter(|k| !ws.catalogs.values().any(|c| c.entries.contains_key(*k))).collect();
            let still_used: Vec<Value> = ws
                .extract()
                .map(|r| {
                    p.keys
                        .iter()
                        .filter_map(|k| r.usages.get(k).map(|u| json!({ "key": k, "refs": u.iter().map(|u| format!("{}:{}", u.at.path, u.at.line)).collect::<Vec<_>>() })))
                        .collect()
                })
                .unwrap_or_default();
            let unknown: Vec<String> = unknown.into_iter().cloned().collect();
            ws.remove_keys(&p.keys).map_err(|e| format!("could not remove keys: {e}"))?;
            Ok(json!({ "removed": p.keys.len() - unknown.len(), "not_found": unknown, "still_used_in_code": still_used }))
        })
        .await
    }

    #[tool(
        description = "Internationalize a whole source file in one step: finds hardcoded UI copy (JSX text with {expressions} and inline tags, attributes, concatenations/template literals, plurals), names semantic keys (LLM, one call), and rewrites the code to t(\"key\") with the right hook/import (next-intl or react-i18next). dry_run (default true) returns the unified diff, the keys + notes it would create and the strings it skipped with reasons; apply the diff yourself, or call again with dry_run: false to let glot write the file and the source catalog. Then call translate."
    )]
    async fn i18nize_file(&self, Parameters(p): Parameters<I18nizeFileParams>) -> CallToolResult {
        self.run(move |mut ws| async move {
            use glot_llm::i18nize;
            let lib = i18nize::detect_lib(&ws.root).ok_or("cannot detect next-intl / react-i18next from package.json")?;
            let path = norm_path(&p.path).to_string();
            if !ws.root.join(&path).is_file() {
                return Err(format!("`{path}` is not a file under {}", ws.root.display()));
            }
            let plan = i18nize::plan(&ws, &[path], lib).await.map_err(|e| format!("i18nize failed: {e}"))?;
            let dry_run = p.dry_run.unwrap_or(true);
            if !dry_run {
                i18nize::apply(&mut ws, &plan).map_err(|e| format!("could not apply: {e}"))?;
            }
            let f = &plan.files[0];
            Ok(json!({
                "dry_run": dry_run, "lib": plan.lib, "key_namer": plan.namer, "converted": f.converted,
                "diff": f.diff, "keys": f.keys, "skipped": f.skipped, "error": f.error, "warnings": plan.warnings,
                "next": if dry_run { "apply the diff and add_string each key (or call i18nize_file with dry_run: false), then translate" } else { "call translate" },
            }))
        })
        .await
    }
}

// ---------- prompts ----------

#[prompt_router]
impl GlotServer {
    #[prompt(description = "Internationalize one source file end to end with glot's tools.")]
    async fn internationalize_file(
        &self,
        Parameters(a): Parameters<PathArg>,
    ) -> Vec<PromptMessage> {
        let path = a.path;
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Internationalize `{path}` using the glot MCP tools:\n\
                 1. get_context {{\"file\": \"{path}\"}} to see the keys already used, the glossary and the conventions of neighboring keys.\n\
                 2. find_hardcoded {{\"path\": \"{path}\"}} to list user-visible literals.\n\
                 3. For each literal: propose_string with the text, the suggested_key, file and ui_role. If it reports a duplicate, REUSE that key. Keep placeholders as ICU arguments (`{{name}}`), never concatenate strings.\n\
                 4. add_string for each genuinely new string, with a short note (UI role, constraints).\n\
                 5. Edit `{path}`: replace each literal with the translation call of the project's i18n library (same style as existing usages).\n\
                 6. translate {{\"keys\": [...new keys]}} then check; fix remaining errors with set_translation.\n\
                 Report the keys created, reused and any string you intentionally left hardcoded."
            ),
        )]
    }

    #[prompt(description = "Review the translations of one locale and fix problems.")]
    async fn review_translations(
        &self,
        Parameters(a): Parameters<LocaleArg>,
    ) -> Vec<PromptMessage> {
        let l = a.locale;
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Review the `{l}` translations with the glot MCP tools:\n\
                 1. status, then check {{\"locale\": \"{l}\"}} (page with offset if needed).\n\
                 2. For each issue, get_context {{\"keys\": [...]}} to see where the string is shown, its UI role, neighbors and glossary.\n\
                 3. Fix errors with set_translation (it validates; nothing is written on error). Respect the glossary (glossary_lookup) and the project style.\n\
                 4. If a term is translated inconsistently, fix it once with glossary_upsert.\n\
                 5. Run check again and summarize what changed and what still needs a human."
            ),
        )]
    }
}

// ---------- server handler (+ resources) ----------

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for GlotServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().enable_prompts().build())
            .with_server_info(Implementation::new("glot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "glot manages this project's i18n catalogs (git is the source of truth). Before adding UI text: propose_string \
                 (reuse duplicates), then add_string, replace the literal with the key in code, translate, check. \
                 Use get_context to understand a file's strings. Never edit catalog JSON files by hand for new keys.",
            )
    }

    async fn list_resources(
        &self,
        _r: Option<PaginatedRequestParams>,
        _c: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let r = |uri: &str, name: &str, desc: &str| {
            Resource::new(uri, name)
                .with_description(desc)
                .with_mime_type("application/json")
        };
        Ok(ListResourcesResult::with_all_items(vec![
            r(
                "glot://config",
                "config",
                "Project configuration (glot.toml)",
            ),
            r("glot://glossary", "glossary", "Glossary terms"),
            r(
                "glot://status",
                "status",
                "Per-locale status and pending plan",
            ),
        ]))
    }

    async fn list_resource_templates(
        &self,
        _r: Option<PaginatedRequestParams>,
        _c: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("glot://catalog/{locale}", "catalog")
                .with_description("Flat key -> value catalog of one locale")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        req: ReadResourceRequestParams,
        _c: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = req.uri.clone();
        let res = self
            .with_ws(move |ws| async move {
                let v = match uri.as_str() {
                    "glot://config" => json!(ws.config),
                    "glot://glossary" => json!(ws.glossary),
                    "glot://status" => status_json(&ws),
                    u => match u.strip_prefix("glot://catalog/") {
                        Some(l) => json!(
                            ws.catalogs
                                .get(l)
                                .ok_or_else(|| format!("unknown locale `{l}`"))?
                                .entries
                        ),
                        None => return Err(format!("unknown resource `{u}`")),
                    },
                };
                Ok(v)
            })
            .await;
        match res {
            Ok(v) => Ok(ReadResourceResult::new(vec![
                ResourceContents::text(pretty(&v), req.uri).with_mime_type("application/json"),
            ])
            .into()),
            Err(m) => Err(ErrorData::resource_not_found(m, None)),
        }
    }
}
