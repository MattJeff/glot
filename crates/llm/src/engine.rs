//! Local project operations shared by the CLI and the MCP server.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use glot_core::{
    Key, Locale,
    catalog::{Catalog, Format},
    config::Config,
    context::KeyContext,
    glossary::{GLOSSARY_FILE, Glossary},
    lock::{Lockfile, Origin, Plan},
    validate::{self, Constraints, Issue, Severity},
};
use glot_extract::ExtractReport;

use crate::{
    Error, Result, provider,
    translate::{self, Item, Request, Translated, Translator, Usage},
};

pub struct Workspace {
    pub root: PathBuf,
    pub config: Config,
    pub glossary: Glossary,
    pub lock: Lockfile,
    /// locale -> catalog (source + targets), loaded from disk.
    pub catalogs: BTreeMap<Locale, Catalog>,
    /// key -> base64 PNG crop of where it renders; attached to translation calls (`glot translate --visual`).
    pub visual: BTreeMap<Key, String>,
    /// Progress lines of long operations (`translate --batch` polling).
    pub progress: Option<fn(&str)>,
    /// `translate`: first pass through the Anthropic Message Batches API (-50 %, asynchronous);
    /// retries and escalation stay direct calls.
    pub batch: bool,
}

/// Pending Message Batch of `translate --batch`, kept until its results are written (resume after Ctrl-C).
pub const BATCH_FILE: &str = ".glot/batch.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocaleStatus {
    pub total: usize,
    pub translated: usize,
    pub missing: usize,
    pub stale: usize,
    pub reviewed: usize,
    pub errors: usize,
    pub warnings: usize,
    /// Translations identical to their source (`untranslated` warnings).
    #[serde(default)]
    pub untranslated: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub source_locale: Locale,
    pub keys: usize,
    pub locales: BTreeMap<Locale, LocaleStatus>,
    pub plan: Plan,
    /// Source keys flagged by `validate::lint_source` (code left in the catalog).
    #[serde(default)]
    pub suspicious_keys: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranslateOptions {
    /// None => all target locales.
    pub locales: Option<Vec<Locale>>,
    /// None => everything in the plan (missing + stale).
    pub keys: Option<Vec<Key>>,
    /// Also retranslate up-to-date and manually edited entries.
    pub force: bool,
    /// Compute plan + estimated cost only.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranslateReport {
    pub written: BTreeMap<Locale, Vec<Translated>>,
    /// Items rejected because of remaining errors (not written).
    pub rejected: BTreeMap<Locale, Vec<Translated>>,
    pub usage: Usage,
    /// Expected cost (first pass + judge + probable retries/escalation, prompt cache), from the real
    /// prompts; with `batch`, the first pass at the batch price.
    pub estimated_cost_usd: f64,
    /// Pessimistic bound (see `translate::Estimate::max_cost_usd`).
    #[serde(default)]
    pub estimated_cost_max_usd: f64,
    /// Expected cost with `--batch` (first pass at -50 %).
    #[serde(default)]
    pub estimated_batch_cost_usd: f64,
    #[serde(default)]
    pub estimated_input_tokens: u64,
    #[serde(default)]
    pub estimated_output_tokens: u64,
    /// Keys selected for translation per locale (filled for dry runs too).
    #[serde(default)]
    pub planned: BTreeMap<Locale, Vec<Key>>,
    /// Keys filled from the translation memory (exact match, no LLM call), per locale.
    #[serde(default)]
    pub tm_reused: BTreeMap<Locale, Vec<Key>>,
    /// What the TM hits would have cost through the LLM (dry-run estimate of the same prompts).
    #[serde(default)]
    pub tm_saved: Usage,
    /// Written keys routed to human review (`glot queue`), per locale.
    #[serde(default)]
    pub needs_review: BTreeMap<Locale, Vec<Key>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub locale: Locale,
    pub key: Key,
    pub issue: Issue,
}

/// Result of `propose_string` (MCP): normalized key + duplicates + glossary hits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Proposal {
    pub key: Key,
    /// Existing keys with the same or near-identical source text (normalized compare).
    pub duplicates: Vec<(Key, String)>,
    pub glossary_terms: Vec<String>,
    pub key_exists: bool,
}

impl Workspace {
    /// Find root from `start` (walk up to glot.toml) and load everything.
    pub fn open(start: &Path) -> Result<Self> {
        let root = Config::find_root(start).ok_or_else(|| {
            Error::Other(format!(
                "no glot.toml found in {} or its parents (run `glot init`)",
                start.display()
            ))
        })?;
        let config = Config::load(&root)?;
        let glossary = Glossary::load(&root.join(GLOSSARY_FILE))?;
        let mut lock = Lockfile::load(&root)?;
        let mut catalogs = BTreeMap::new();
        for loc in config.all_locales() {
            let cat = Catalog::load(&config, &root, &loc)?;
            catalogs.insert(loc, cat);
        }
        // Mobile/gettext/YAML catalogs: file notes, fuzzy/needs_review flags, msgid as source.
        glot_core::formats::load_meta(&config, &root, &mut catalogs, &mut lock)?;
        Ok(Self {
            root,
            config,
            glossary,
            lock,
            catalogs,
            visual: BTreeMap::new(),
            progress: None,
            batch: false,
        })
    }

    pub fn reload(&mut self) -> Result<()> {
        *self = Self::open(&self.root)?;
        Ok(())
    }

    pub fn source(&self) -> &Catalog {
        &self.catalogs[&self.config.source_locale]
    }

    fn target(&self, locale: &str) -> Catalog {
        self.catalogs
            .get(locale)
            .cloned()
            .unwrap_or_else(|| Catalog::new(locale))
    }

    /// Lock plan over every target locale, plus existing translations failing validation.
    pub fn plan(&self) -> Plan {
        let targets: Vec<Catalog> = self
            .config
            .target_locales
            .iter()
            .map(|l| self.target(l))
            .collect();
        let mut plan = self.lock.plan(self.source(), &targets);
        self.add_invalid(&mut plan, &self.check());
        plan
    }

    /// Error findings not already queued go to `invalid`, or `invalid_human` when a human owns the value.
    fn add_invalid(&self, plan: &mut Plan, findings: &[Finding]) {
        for f in findings
            .iter()
            .filter(|f| f.issue.severity == Severity::Error)
        {
            let item = (f.locale.clone(), f.key.clone());
            if plan.missing.contains(&item) || plan.stale.contains(&item) {
                continue;
            }
            let human = plan.manual_edits.contains(&item)
                || self
                    .lock
                    .entries
                    .get(&f.locale)
                    .and_then(|e| e.get(&f.key))
                    .is_some_and(|e| e.reviewed || e.origin == Origin::Human);
            let list = if human {
                &mut plan.invalid_human
            } else {
                &mut plan.invalid
            };
            if !list.contains(&item) {
                list.push(item);
            }
        }
    }

    /// Plan + per-locale counts (validation run over every existing target).
    pub fn status(&self) -> Status {
        let targets: Vec<Catalog> = self
            .config
            .target_locales
            .iter()
            .map(|l| self.target(l))
            .collect();
        let mut plan = self.lock.plan(self.source(), &targets);
        let findings = self.check();
        self.add_invalid(&mut plan, &findings);
        let source = self.source();
        let count =
            |list: &[(Locale, Key)], loc: &str| list.iter().filter(|(l, _)| l == loc).count();
        let locales = self
            .config
            .target_locales
            .iter()
            .map(|loc| {
                let target = self.catalogs.get(loc);
                let lock = self.lock.entries.get(loc);
                let in_loc = findings.iter().filter(|f| &f.locale == loc);
                let s = LocaleStatus {
                    total: source.entries.len(),
                    translated: source
                        .entries
                        .keys()
                        .filter(|k| {
                            target
                                .and_then(|t| t.entries.get(*k))
                                .is_some_and(|v| !v.is_empty())
                        })
                        .count(),
                    missing: count(&plan.missing, loc),
                    stale: count(&plan.stale, loc),
                    reviewed: source
                        .entries
                        .keys()
                        .filter(|k| lock.and_then(|l| l.get(*k)).is_some_and(|e| e.reviewed))
                        .count(),
                    errors: in_loc
                        .clone()
                        .filter(|f| f.issue.severity == Severity::Error)
                        .count(),
                    warnings: in_loc
                        .clone()
                        .filter(|f| f.issue.severity == Severity::Warning)
                        .count(),
                    untranslated: in_loc
                        .filter(|f| f.issue.code == glot_core::validate::IssueCode::Untranslated)
                        .count(),
                };
                (loc.clone(), s)
            })
            .collect();
        let suspicious_keys = findings
            .iter()
            .filter(|f| f.issue.code == glot_core::validate::IssueCode::SuspiciousSource)
            .map(|f| &f.key)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        Status {
            source_locale: self.config.source_locale.clone(),
            keys: source.entries.len(),
            locales,
            plan,
            suspicious_keys,
        }
    }

    pub(crate) fn validate(
        &self,
        locale: &str,
        source: &str,
        target: &str,
        max_length: Option<usize>,
    ) -> Vec<Issue> {
        let c = Constraints {
            max_length,
            max_length_ratio: Some(self.config.qa.max_length_ratio),
        };
        let mut issues = validate::validate(source, target, locale, self.config.syntax, &c);
        issues.extend(self.glossary.check(source, target, locale));
        // A source that is just a do-not-translate term (brand, product) is meant to stay identical.
        if self
            .glossary
            .terms
            .iter()
            .any(|t| t.do_not_translate && t.term.eq_ignore_ascii_case(source.trim()))
        {
            issues.retain(|i| i.code != glot_core::validate::IssueCode::Untranslated);
        }
        issues
    }

    /// Validate all targets (+ glossary). Deterministic, no network.
    pub fn check(&self) -> Vec<Finding> {
        let source = self.source();
        let mut out = vec![];
        let layout = glot_core::measure::load_layout(&self.root).unwrap_or_default();
        // Source-only problems (code left in the catalog), once per key rather than per locale.
        for (key, value) in &source.entries {
            for issue in glot_core::validate::lint_source(key, value) {
                out.push(Finding {
                    locale: self.config.source_locale.clone(),
                    key: key.clone(),
                    issue,
                });
            }
        }
        for loc in &self.config.target_locales {
            let Some(target) = self.catalogs.get(loc) else {
                continue;
            };
            for (key, value) in &target.entries {
                let Some(src) = source.entries.get(key) else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                // An identical target that glot produced (the LLM saw the source and kept it) or a human
                // reviewed is a decision, not a miss: don't re-flag it as untranslated.
                let decided = self
                    .lock
                    .entries
                    .get(loc)
                    .and_then(|e| e.get(key))
                    .is_some_and(|e| e.target_hash == glot_core::hash::hash(value));
                let fit = layout.get(key);
                let mut issues = self.validate(loc, src, value, None);
                if let Some(f) = fit {
                    // Measured pixel budget: TooWide replaces the length-ratio warning.
                    issues.retain(|i| {
                        !(i.code == glot_core::validate::IssueCode::TooLong
                            && i.severity == Severity::Warning)
                    });
                    issues.extend(glot_core::measure::check(value, self.config.syntax, f));
                }
                // Routed to human review (`glot queue`): pending until approved.
                if let Some(r) = self
                    .lock
                    .entries
                    .get(loc)
                    .and_then(|e| e.get(key))
                    .and_then(|e| e.needs_review.as_ref())
                {
                    issues.push(Issue {
                        code: glot_core::validate::IssueCode::LowScore,
                        severity: Severity::Warning,
                        message: format!(
                            "pending human review ({r}): glot queue approve {loc} {key}"
                        ),
                    });
                }
                // en_GB / fr_CA copying most of their source is expected, not a miss.
                let lang = |l: &str| {
                    l.split(['-', '_'])
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                };
                let same_lang = lang(loc) == lang(&self.config.source_locale);
                for issue in issues.into_iter().filter(|i| {
                    !((decided || same_lang)
                        && i.code == glot_core::validate::IssueCode::Untranslated)
                }) {
                    out.push(Finding {
                        locale: loc.clone(),
                        key: key.clone(),
                        issue,
                    });
                }
            }
        }
        out
    }

    pub fn extract(&self) -> Result<ExtractReport> {
        let mut report = glot_extract::extract(&self.root, &self.config.extract)
            .map_err(|e| Error::Other(e.to_string()))?;
        // Proper nouns marked do_not_translate in the glossary are not hardcoded-string misses.
        let dnt = |h: &glot_extract::Hardcoded| {
            self.glossary
                .terms
                .iter()
                .any(|t| t.do_not_translate && t.term.eq_ignore_ascii_case(h.text.trim()))
        };
        report.hardcoded.retain(|h| !dnt(h));
        report.config.retain(|h| !dnt(h));
        Ok(report)
    }

    pub fn contexts(&self, report: &ExtractReport) -> BTreeMap<Key, KeyContext> {
        glot_extract::contexts(report, self.source(), &self.lock)
    }

    /// Translate per the plan, write catalogs + glot.lock, record imported/manual entries, enforce budget.
    pub async fn translate(&mut self, opts: TranslateOptions) -> Result<TranslateReport> {
        let plan = self.plan();
        let source = self.source().clone();
        let locales: Vec<Locale> = self
            .config
            .target_locales
            .iter()
            .filter(|l| opts.locales.as_ref().is_none_or(|w| w.contains(l)))
            .cloned()
            .collect();
        if let Some(unknown) = opts
            .locales
            .iter()
            .flatten()
            .find(|l| !self.config.target_locales.contains(l))
        {
            return Err(Error::Other(format!(
                "locale \"{unknown}\" is not in target_locales"
            )));
        }
        let of = |list: &[(Locale, Key)], loc: &str| -> BTreeSet<Key> {
            list.iter()
                .filter(|(l, _)| l == loc)
                .map(|(_, k)| k.clone())
                .collect()
        };

        let mut jobs: Vec<Request> = vec![];
        let mut tm = crate::tm::Tm::load(&self.root).unwrap_or_else(|e| {
            tracing::warn!("ignoring the translation memory: {e}");
            Default::default()
        });
        let mut tm_hits: BTreeMap<Locale, Vec<(Key, String)>> = BTreeMap::new();
        let mut tm_saved = Usage::default();
        let mut contexts: Option<BTreeMap<Key, KeyContext>> = None;
        // Pixel budgets from `glot measure`: the validator rejects too-wide translations before writing.
        let layout = glot_core::measure::load_layout(&self.root).unwrap_or_else(|e| {
            tracing::warn!("ignoring {}: {e}", glot_core::measure::LAYOUT_FILE);
            Default::default()
        });
        for loc in &locales {
            let manual = of(&plan.manual_edits, loc);
            let invalid = of(&plan.invalid, loc);
            let keys: Vec<Key> = if opts.force {
                source.entries.keys().cloned().collect()
            } else {
                let mut k = of(&plan.missing, loc);
                k.extend(of(&plan.stale, loc));
                k.extend(of(&plan.invalid, loc));
                k.into_iter().filter(|k| !manual.contains(k)).collect()
            };
            let keys: Vec<Key> = keys
                .into_iter()
                .filter(|k| opts.keys.as_ref().is_none_or(|w| w.contains(k)))
                .filter(|k| source.entries.get(k).is_some_and(|v| !v.is_empty()))
                .collect();
            // Translation memory: an exact, still-valid match costs nothing (not on --force: fresh output asked).
            let src_loc = &self.config.source_locale;
            let (hits, keys): (Vec<_>, Vec<_>) = keys
                .into_iter()
                .map(|k| {
                    let s = &source.entries[&k];
                    let note = self.lock.notes.get(&k).map(String::as_str);
                    let hit = (!opts.force)
                        .then(|| tm.exact(src_loc, loc, s, note))
                        .flatten()
                        .map(|u| u.target.clone())
                        .filter(|t| !validate::has_errors(&self.validate(loc, s, t, None)));
                    (k, hit)
                })
                .partition(|(_, h)| h.is_some());
            let keys: Vec<Key> = keys.into_iter().map(|(k, _)| k).collect();
            if !hits.is_empty() {
                let hits: Vec<(Key, String)> = hits
                    .into_iter()
                    .map(|(k, h)| (k, h.unwrap_or_default()))
                    .collect();
                let saved = Request {
                    source_locale: src_loc.clone(),
                    target_locale: loc.clone(),
                    items: hits
                        .iter()
                        .map(|(k, _)| Item {
                            key: k.clone(),
                            source: source.entries[k].clone(),
                            context: KeyContext::default(),
                            previous: None,
                        })
                        .collect(),
                    glossary: vec![],
                    syntax: self.config.syntax,
                    style: self.config.llm.style.clone(),
                };
                tm_saved.add(&translate::estimate(&self.config, &saved));
                tm_hits.insert(loc.clone(), hits);
            }
            if keys.is_empty() {
                continue;
            }
            let ctx = contexts.get_or_insert_with(|| match self.extract() {
                Ok(r) => self.contexts(&r),
                Err(e) => {
                    tracing::warn!("code extraction failed, translating without code context: {e}");
                    BTreeMap::new()
                }
            });
            let target = self.target(loc);
            let items: Vec<Item> = keys
                .into_iter()
                .map(|k| Item {
                    source: source.entries[&k].clone(),
                    context: KeyContext {
                        fit: layout.get(&k).cloned(),
                        ..ctx.get(&k).cloned().unwrap_or_else(|| KeyContext {
                            key: k.clone(),
                            description: self.lock.notes.get(&k).cloned(),
                            ..Default::default()
                        })
                    },
                    // A value failing validation is not a draft to edit minimally.
                    previous: target
                        .entries
                        .get(&k)
                        .filter(|v| !v.is_empty() && !invalid.contains(&k))
                        .cloned(),
                    key: k,
                })
                .map(|mut i| {
                    // ponytail: the fuzzy TM match rides in `description` (few-shot example), not a new prompt field.
                    if let Some((u, sim)) = tm.fuzzy(src_loc, loc, &i.source) {
                        let ex = format!(
                            "Similar approved translation (TM {:.0} %), reuse its wording: {:?} => {:?}",
                            sim * 100.0,
                            u.source,
                            u.target
                        );
                        i.context.description = Some(match i.context.description.take() {
                            Some(d) => format!("{d}\n{ex}"),
                            None => ex,
                        });
                    }
                    i
                })
                .collect();
            let mut glossary: Vec<glot_core::glossary::Term> = vec![];
            for i in &items {
                for t in self.glossary.matches(&i.source) {
                    if !glossary.iter().any(|g| g.term == t.term) {
                        glossary.push(t.clone());
                    }
                }
            }
            jobs.push(Request {
                source_locale: self.config.source_locale.clone(),
                target_locale: loc.clone(),
                items,
                glossary,
                syntax: self.config.syntax,
                style: self.config.llm.style.clone(),
            });
        }

        let mut report = TranslateReport {
            tm_reused: tm_hits
                .iter()
                .map(|(l, h)| (l.clone(), h.iter().map(|(k, _)| k.clone()).collect()))
                .collect(),
            tm_saved,
            ..Default::default()
        };
        for req in &jobs {
            report.planned.insert(
                req.target_locale.clone(),
                req.items.iter().map(|i| i.key.clone()).collect(),
            );
            let e = translate::estimate_full(&self.config, req, &self.visual);
            let u = &e.expected;
            report.estimated_input_tokens +=
                u.input_tokens + u.cache_creation_tokens + u.cache_read_tokens;
            report.estimated_output_tokens += u.output_tokens;
            report.estimated_cost_usd += if self.batch {
                e.batch_cost_usd
            } else {
                u.cost_usd
            };
            report.estimated_batch_cost_usd += e.batch_cost_usd;
            report.estimated_cost_max_usd += e.max_cost_usd;
        }
        if opts.dry_run {
            return Ok(report);
        }
        let budget = self.config.llm.budget_usd;
        if let Some(b) = budget.filter(|b| report.estimated_cost_usd > *b) {
            return Err(Error::Budget {
                spent: report.estimated_cost_usd,
                budget: b,
            });
        }

        // Bookkeeping: adopt pre-existing and hand-edited targets, forget orphans.
        for loc in &locales {
            let target = self.target(loc);
            for (origin, list) in [
                (Origin::Imported, &plan.untracked),
                (Origin::Human, &plan.manual_edits),
            ] {
                for k in of(list, loc) {
                    if let (Some(s), Some(t)) = (source.entries.get(&k), target.entries.get(&k)) {
                        self.lock.record(loc, &k, s, t, origin.clone(), None);
                    }
                }
            }
            for k in of(&plan.orphaned, loc) {
                self.lock.remove(loc, &k);
            }
        }

        for (loc, hits) in &tm_hits {
            let changes: Vec<(Key, Option<String>)> = hits
                .iter()
                .map(|(k, v)| (k.clone(), Some(v.clone())))
                .collect();
            self.write_entries(loc, &changes)?;
            for (k, v) in hits {
                let origin = Origin::Machine { model: "tm".into() };
                self.record_routed(
                    &mut report,
                    loc,
                    k,
                    &source.entries[k],
                    v,
                    origin,
                    None,
                    &[],
                );
            }
        }

        if !jobs.is_empty() {
            let fallback = crate::routing::fallback_provider(&self.config.llm)?
                .map(|p| crate::visual::wrap(p, &self.visual));
            let translator = Translator::new(
                &self.config,
                crate::visual::wrap(provider::from_config(&self.config.llm, None)?, &self.visual),
            )
            .with_fallback_provider(fallback);
            let firsts = if self.batch {
                self.run_batch(&translator, &jobs).await?
            } else {
                BTreeMap::new()
            };
            for req in jobs {
                let loc = req.target_locale.clone();
                if let Some(b) = budget.filter(|b| report.usage.cost_usd >= *b) {
                    self.lock.save(&self.root)?;
                    return Err(Error::Budget {
                        spent: report.usage.cost_usd,
                        budget: b,
                    });
                }
                let (results, usage) = translator.translate_from(req, &firsts).await?;
                report.usage.add(&usage);
                let (ok, bad): (Vec<_>, Vec<_>) = results
                    .into_iter()
                    .partition(|t| !validate::has_errors(&t.issues));
                let changes: Vec<(Key, Option<String>)> = ok
                    .iter()
                    .map(|t| (t.key.clone(), Some(t.value.clone())))
                    .collect();
                self.write_entries(&loc, &changes)?;
                for t in &ok {
                    let origin = Origin::Machine {
                        model: t.model.clone(),
                    };
                    let added = self.record_routed(
                        &mut report,
                        &loc,
                        &t.key,
                        &source.entries[&t.key],
                        &t.value,
                        origin,
                        t.score,
                        &t.issues,
                    );
                    tm.units.extend(added);
                }
                if let Err(e) = tm.save(&self.root) {
                    tracing::warn!("{e}");
                }
                self.lock.save(&self.root)?;
                if !ok.is_empty() {
                    report.written.insert(loc.clone(), ok);
                }
                if !bad.is_empty() {
                    report.rejected.insert(loc, bad);
                }
            }
            if self.batch {
                let _ = std::fs::remove_file(self.root.join(BATCH_FILE));
            }
        }
        self.lock.save(&self.root)?;
        Ok(report)
    }

    /// Record a written translation with its quality score; route it to the review queue (glot.lock
    /// `needs_review`) or, when validated, return the TM unit to remember.
    #[allow(clippy::too_many_arguments)]
    fn record_routed(
        &mut self,
        report: &mut TranslateReport,
        loc: &str,
        key: &str,
        source: &str,
        value: &str,
        origin: Origin,
        judge: Option<u8>,
        issues: &[Issue],
    ) -> Option<crate::tm::Unit> {
        let score = crate::routing::score(judge, issues);
        self.lock
            .record(loc, key, source, value, origin, Some(score));
        let note = self.lock.notes.get(key).cloned();
        let reason = crate::routing::route(&self.config.qa, key, note.as_deref(), score);
        if let Some(e) = self.lock.entries.get_mut(loc).and_then(|m| m.get_mut(key)) {
            e.needs_review = reason.clone();
        }
        if reason.is_some() {
            report
                .needs_review
                .entry(loc.to_string())
                .or_default()
                .push(key.to_string());
            return None;
        }
        Some(crate::tm::Unit {
            source_locale: self.config.source_locale.clone(),
            target_locale: loc.to_string(),
            source: source.to_string(),
            target: value.to_string(),
            context: note,
        })
    }

    /// Clear a review-queue entry: reviewed, out of the queue, into the translation memory.
    pub fn approve(&mut self, locale: &str, key: &str) -> Result<()> {
        let value = self
            .catalogs
            .get(locale)
            .and_then(|c| c.entries.get(key))
            .cloned();
        let source = self.source().entries.get(key).cloned();
        let entry = self
            .lock
            .entries
            .get_mut(locale)
            .and_then(|m| m.get_mut(key));
        let (Some(e), Some(value), Some(source)) = (entry, value, source) else {
            return Err(Error::Other(format!("no translation for {locale} {key}")));
        };
        e.needs_review = None;
        e.reviewed = true;
        e.reviewed_at = Some(glot_core::lock::utc_now());
        e.reviewed_by = std::env::var("GLOT_REVIEWER")
            .ok()
            .filter(|r| !r.is_empty());
        self.lock.save(&self.root)?;
        self.remember(locale, key, &source, &value)
    }

    fn remember(&self, locale: &str, key: &str, source: &str, value: &str) -> Result<()> {
        crate::tm::Tm::remember(
            &self.root,
            crate::tm::Unit {
                source_locale: self.config.source_locale.clone(),
                target_locale: locale.to_string(),
                source: source.to_string(),
                target: value.to_string(),
                context: self.lock.notes.get(key).cloned(),
            },
        )
    }

    /// Submit (or resume, from `BATCH_FILE`) the first pass of every job as one Message Batch, poll
    /// until it ends, return custom_id -> completion. Custom ids are content hashes, so a rerun with
    /// the same prompts picks the stored batch (and its fetched results) up instead of paying twice.
    async fn run_batch(
        &self,
        t: &Translator,
        jobs: &[Request],
    ) -> Result<BTreeMap<String, provider::Completion>> {
        #[derive(Serialize, Deserialize)]
        struct Pending {
            id: String,
            ids: BTreeSet<String>,
            #[serde(default)]
            results: Option<BTreeMap<String, provider::Completion>>,
        }
        let path = self.root.join(BATCH_FILE);
        let save = |p: &Pending| -> Result<()> {
            let io = |e: std::io::Error| Error::Other(format!("{}: {e}", path.display()));
            std::fs::create_dir_all(path.parent().unwrap_or(&self.root)).map_err(io)?;
            std::fs::write(&path, serde_json::to_string(p).unwrap_or_default()).map_err(io)
        };
        let say = |m: &str| {
            if let Some(f) = self.progress {
                f(m)
            }
        };
        let mut reqs: BTreeMap<String, provider::CompletionRequest> =
            jobs.iter().flat_map(|r| t.first_pass(r)).collect();
        let pending: Option<Pending> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .filter(|p: &Pending| reqs.keys().all(|id| p.ids.contains(id)));
        if let Some(r) = pending.as_ref().and_then(|p| p.results.clone()) {
            say(&format!(
                "Reusing the results of batch {}",
                pending.as_ref().map_or("", |p| &p.id)
            ));
            return Ok(r);
        }
        let client = provider::batch_client(&self.config.llm, None, &BTreeMap::new())?;
        let mut status = match pending {
            Some(p) => {
                say(&format!("Resuming batch {}", p.id));
                client.status(&p.id).await?
            }
            None => {
                for r in reqs.values_mut() {
                    crate::visual::attach(&self.visual, r);
                }
                let list: Vec<_> = reqs.into_iter().collect();
                let s = client.submit(&list).await?;
                save(&Pending {
                    id: s.id.clone(),
                    ids: list.into_iter().map(|(id, _)| id).collect(),
                    results: None,
                })?;
                say(&format!(
                    "Submitted batch {} (resume with the same command if interrupted)",
                    s.id
                ));
                s
            }
        };
        let mut wait = std::time::Duration::from_secs(1);
        while !status.ended() {
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(std::time::Duration::from_secs(30));
            status = client.status(&status.id).await?;
            let c = |k: &str| status.request_counts.get(k).copied().unwrap_or(0);
            let done = c("succeeded") + c("errored") + c("canceled") + c("expired");
            say(&format!(
                "Batch {}: {done}/{} done",
                status.id,
                done + c("processing")
            ));
        }
        let results = client.results(&status).await?;
        let ids = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Pending>(&s).ok())
            .map(|p| p.ids)
            .unwrap_or_else(|| results.keys().cloned().collect());
        save(&Pending {
            id: status.id.clone(),
            ids,
            results: Some(results.clone()),
        })?;
        Ok(results)
    }

    fn file_format(&self, path: &Path) -> Format {
        if self.config.format != Format::Auto {
            return self.config.format;
        }
        let detect = |p: &Path| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| Catalog::detect_format(&s))
        };
        // New file: mirror the source catalog's layout.
        detect(path)
            .or_else(|| {
                self.config
                    .catalog_paths(&self.root, &self.config.source_locale)
                    .first()
                    .and_then(|p| detect(p))
            })
            .unwrap_or(Format::Auto)
    }

    /// Apply sets (Some) / deletions (None) for one locale: update the file already holding the key,
    /// else the first file of the pattern; keep the in-memory catalog in sync.
    pub(crate) fn write_entries(
        &mut self,
        locale: &str,
        changes: &[(Key, Option<String>)],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let paths = self.config.catalog_files(&self.root, locale);
        if paths.is_empty() {
            return Err(Error::Other("glot.toml `files` is empty".into()));
        }
        let mut files = vec![];
        for (p, layout) in &paths {
            let fmt = self.file_format(p);
            let cat = Catalog::read_with(p, locale, fmt, layout)?;
            files.push((p.clone(), cat, fmt, false, layout.clone()));
        }
        // A key new to this locale goes to the file holding it in the source locale (same pattern
        // index): `.stringsdict` plurals, i18next namespaces.
        let src = &self.config.source_locale;
        let src_files: Vec<Catalog> = if paths.len() > 1 && locale != src {
            self.config
                .catalog_files(&self.root, src)
                .iter()
                .map(|(p, l)| {
                    Catalog::read_with(p, src, self.file_format(p), l).unwrap_or_default()
                })
                .collect()
        } else {
            vec![]
        };
        let mem = self
            .catalogs
            .entry(locale.to_string())
            .or_insert_with(|| Catalog::new(locale));
        for (key, value) in changes {
            match value {
                Some(v) => {
                    let i = files
                        .iter()
                        .position(|f| f.1.entries.contains_key(key))
                        // i18next `{ns}` files: `home:title` lives in `home.json`.
                        .or_else(|| {
                            files
                                .iter()
                                .position(|f| !f.4.ns.is_empty() && key.starts_with(&f.4.ns))
                        })
                        .or_else(|| {
                            src_files
                                .iter()
                                .position(|c| c.entries.contains_key(key))
                                .filter(|&i| i < files.len())
                        })
                        // A new key goes to the file already holding its longest dotted prefix
                        // (`devise.new` => `devise.en.yml`).
                        .or_else(|| {
                            key.rmatch_indices('.').find_map(|(d, _)| {
                                let p = format!("{}.", &key[..d]);
                                files.iter().position(|f| {
                                    f.1.entries
                                        .range(p.clone()..)
                                        .next()
                                        .is_some_and(|(k, _)| k.starts_with(&p))
                                })
                            })
                        })
                        .unwrap_or(0);
                    files[i].1.entries.insert(key.clone(), v.clone());
                    files[i].3 = true;
                    mem.entries.insert(key.clone(), v.clone());
                }
                None => {
                    for f in files.iter_mut() {
                        f.3 |= f.1.entries.remove(key).is_some();
                    }
                    mem.entries.remove(key);
                }
            }
        }
        for (path, cat, fmt, changed, layout) in files {
            if changed {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| glot_core::Error::Io {
                        path: dir.to_path_buf(),
                        source: e,
                    })?;
                }
                cat.write_with(&path, fmt, &layout)?;
            }
        }
        Ok(())
    }

    /// Add/replace a source string (and optional note) then save the source catalog + lock.
    pub fn add_source(&mut self, key: &str, value: &str, note: Option<&str>) -> Result<()> {
        let key = key.trim();
        if key.is_empty() || key.starts_with('.') || key.ends_with('.') || key.contains("..") {
            return Err(Error::Other(format!("invalid key \"{key}\"")));
        }
        let src = self.config.source_locale.clone();
        self.write_entries(&src, &[(key.to_string(), Some(value.to_string()))])?;
        if let Some(n) = note.filter(|n| !n.trim().is_empty()) {
            self.lock.notes.insert(key.to_string(), n.to_string());
        }
        self.lock.save(&self.root)?;
        Ok(())
    }

    /// Set a target value by hand (validated; errors returned, nothing written if errors).
    pub fn set_translation(&mut self, locale: &str, key: &str, value: &str) -> Result<Vec<Issue>> {
        if !self.config.target_locales.iter().any(|l| l == locale) {
            return Err(Error::Other(format!(
                "locale \"{locale}\" is not in target_locales"
            )));
        }
        let source = self.source().entries.get(key).cloned().ok_or_else(|| {
            Error::Other(format!("unknown key \"{key}\" (not in the source catalog)"))
        })?;
        let issues = self.validate(locale, &source, value, None);
        if validate::has_errors(&issues) {
            return Ok(issues);
        }
        self.write_entries(locale, &[(key.to_string(), Some(value.to_string()))])?;
        self.lock
            .record(locale, key, &source, value, Origin::Human, None);
        self.lock.save(&self.root)?;
        self.remember(locale, key, &source, value)?;
        Ok(issues)
    }

    pub fn propose(&self, text: &str, suggested_key: Option<&str>) -> Proposal {
        let key = match suggested_key.map(normalize_key).filter(|k| !k.is_empty()) {
            Some(k) => k,
            None => snake(
                &text
                    .split_whitespace()
                    .take(5)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        };
        let norm = normalize_text(text);
        let source = self.source();
        Proposal {
            key_exists: source.entries.contains_key(&key),
            duplicates: source
                .entries
                .iter()
                .filter(|(_, v)| normalize_text(v) == norm)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            glossary_terms: self
                .glossary
                .matches(text)
                .into_iter()
                .map(|t| t.term.clone())
                .collect(),
            key,
        }
    }

    /// Delete keys from every catalog + lock.
    pub fn remove_keys(&mut self, keys: &[Key]) -> Result<()> {
        let changes: Vec<(Key, Option<String>)> = keys.iter().map(|k| (k.clone(), None)).collect();
        for loc in self.config.all_locales() {
            self.write_entries(&loc, &changes)?;
            for k in keys {
                self.lock.remove(&loc, k);
            }
        }
        for k in keys {
            self.lock.notes.remove(k);
        }
        self.lock.save(&self.root)?;
        Ok(())
    }

    pub fn save_glossary(&self) -> Result<()> {
        Ok(self.glossary.save(&self.root.join(GLOSSARY_FILE))?)
    }
}

/// `Settings.Billing Page.Save!` => `settings.billing_page.save`.
fn normalize_key(key: &str) -> String {
    key.split('.')
        .map(snake)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            // camelCase boundary
            if c.is_uppercase() && prev_lower {
                out.push('_');
            }
            out.extend(c.to_lowercase());
            prev_lower = c.is_lowercase() || c.is_numeric();
        } else {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_string()
}

/// Case/whitespace/punctuation-insensitive form used for duplicate detection.
fn normalize_text(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_text() {
        assert_eq!(
            normalize_key("Settings.Billing Page.saveButton!"),
            "settings.billing_page.save_button"
        );
        assert_eq!(snake("Save changes now"), "save_changes_now");
        assert_eq!(
            normalize_text("  Save   Changes! "),
            normalize_text("save changes")
        );
    }
}
