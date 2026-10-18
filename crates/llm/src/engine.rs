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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocaleStatus {
    pub total: usize,
    pub translated: usize,
    pub missing: usize,
    pub stale: usize,
    pub reviewed: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub source_locale: Locale,
    pub keys: usize,
    pub locales: BTreeMap<Locale, LocaleStatus>,
    pub plan: Plan,
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
    /// Expected cost (first pass + judge + `RETRY_MARGIN`), from the real prompts.
    pub estimated_cost_usd: f64,
    /// Pessimistic bound: `ESTIMATE_MAX_FACTOR` × the expected cost.
    #[serde(default)]
    pub estimated_cost_max_usd: f64,
    #[serde(default)]
    pub estimated_input_tokens: u64,
    #[serde(default)]
    pub estimated_output_tokens: u64,
    /// Keys selected for translation per locale (filled for dry runs too).
    #[serde(default)]
    pub planned: BTreeMap<Locale, Vec<Key>>,
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

/// Upper bound of the cost range: tokenizer variance (~±30 %) + heavy retries/escalation to fallback_model.
// ponytail: flat factor; derive from fallback_model price if users need a tighter bound.
const ESTIMATE_MAX_FACTOR: f64 = 2.0;

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
        let lock = Lockfile::load(&root)?;
        let mut catalogs = BTreeMap::new();
        for loc in config.all_locales() {
            let cat = Catalog::read_all(&config.catalog_paths(&root, &loc), &loc, config.format)?;
            catalogs.insert(loc, cat);
        }
        Ok(Self {
            root,
            config,
            glossary,
            lock,
            catalogs,
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

    /// Lock plan over every target locale.
    pub fn plan(&self) -> Plan {
        let targets: Vec<Catalog> = self
            .config
            .target_locales
            .iter()
            .map(|l| self.target(l))
            .collect();
        self.lock.plan(self.source(), &targets)
    }

    /// Plan + per-locale counts (validation run over every existing target).
    pub fn status(&self) -> Status {
        let plan = self.plan();
        let findings = self.check();
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
                        .filter(|f| f.issue.severity == Severity::Warning)
                        .count(),
                };
                (loc.clone(), s)
            })
            .collect();
        Status {
            source_locale: self.config.source_locale.clone(),
            keys: source.entries.len(),
            locales,
            plan,
        }
    }

    fn validate(
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
                for issue in self.validate(loc, src, value, None) {
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
        report.hardcoded.retain(|h| {
            !self
                .glossary
                .terms
                .iter()
                .any(|t| t.do_not_translate && t.term.eq_ignore_ascii_case(h.text.trim()))
        });
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
        let mut contexts: Option<BTreeMap<Key, KeyContext>> = None;
        for loc in &locales {
            let manual = of(&plan.manual_edits, loc);
            let keys: Vec<Key> = if opts.force {
                source.entries.keys().cloned().collect()
            } else {
                let mut k = of(&plan.missing, loc);
                k.extend(of(&plan.stale, loc));
                k.into_iter().filter(|k| !manual.contains(k)).collect()
            };
            let keys: Vec<Key> = keys
                .into_iter()
                .filter(|k| opts.keys.as_ref().is_none_or(|w| w.contains(k)))
                .filter(|k| source.entries.get(k).is_some_and(|v| !v.is_empty()))
                .collect();
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
                    context: ctx.get(&k).cloned().unwrap_or_else(|| KeyContext {
                        key: k.clone(),
                        description: self.lock.notes.get(&k).cloned(),
                        ..Default::default()
                    }),
                    previous: target.entries.get(&k).filter(|v| !v.is_empty()).cloned(),
                    key: k,
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

        let mut report = TranslateReport::default();
        for req in &jobs {
            report.planned.insert(
                req.target_locale.clone(),
                req.items.iter().map(|i| i.key.clone()).collect(),
            );
            let e = translate::estimate(&self.config, req);
            report.estimated_input_tokens += e.input_tokens;
            report.estimated_output_tokens += e.output_tokens;
            report.estimated_cost_usd += e.cost_usd;
        }
        report.estimated_cost_max_usd = report.estimated_cost_usd * ESTIMATE_MAX_FACTOR;
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

        if !jobs.is_empty() {
            let translator =
                Translator::new(&self.config, provider::from_config(&self.config.llm, None)?);
            for req in jobs {
                let loc = req.target_locale.clone();
                if let Some(b) = budget.filter(|b| report.usage.cost_usd >= *b) {
                    self.lock.save(&self.root)?;
                    return Err(Error::Budget {
                        spent: report.usage.cost_usd,
                        budget: b,
                    });
                }
                let (results, usage) = translator.translate(req).await?;
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
                    self.lock.record(
                        &loc,
                        &t.key,
                        &source.entries[&t.key],
                        &t.value,
                        Origin::Machine {
                            model: t.model.clone(),
                        },
                        t.score,
                    );
                }
                self.lock.save(&self.root)?;
                if !ok.is_empty() {
                    report.written.insert(loc.clone(), ok);
                }
                if !bad.is_empty() {
                    report.rejected.insert(loc, bad);
                }
            }
        }
        self.lock.save(&self.root)?;
        Ok(report)
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
    fn write_entries(&mut self, locale: &str, changes: &[(Key, Option<String>)]) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let paths = self.config.catalog_paths(&self.root, locale);
        if paths.is_empty() {
            return Err(Error::Other("glot.toml `files` is empty".into()));
        }
        let mut files = vec![];
        for p in &paths {
            let fmt = self.file_format(p);
            files.push((p.clone(), Catalog::read(p, locale, fmt)?, fmt, false));
        }
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
        for (path, cat, fmt, changed) in files {
            if changed {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| glot_core::Error::Io {
                        path: dir.to_path_buf(),
                        source: e,
                    })?;
                }
                cat.write(&path, fmt)?;
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
