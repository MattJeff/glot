//! glot-extract: static analysis of JS/TS/JSX sources (oxc).
//!
//! Supported: next-intl (`useTranslations`/`getTranslations` namespaces, `t.rich/markup/raw/has`),
//! react-i18next/i18next (`useTranslation(ns, {keyPrefix})`, `i18n.t`, `<Trans i18nKey>`),
//! FormatJS (`formatMessage({id})`, `<FormattedMessage id>`, `defineMessages`), vue `$t` / `t` / `v-t` / `<i18n-t keypath>` (script and template).
//!
//! i18next namespaces are kept: `useTranslation("home")` + `t("title")`, `t("home:title")` and
//! `<Trans ns="home" i18nKey="title">` are `home:title`, the key of `title` in a `{ns}` catalog
//! file (`public/locales/{locale}/home.json`). A key without namespace matches any namespace, and a
//! namespaced key also matches a one-file catalog's bare key.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub mod email;
mod foreign;
pub mod i18nize;
mod scan;
pub use scan::looks_like_css_classes;
pub mod svg;
pub mod swift;
#[cfg(test)]
mod tests;

use glot_core::{
    Key,
    catalog::Catalog,
    config::ExtractConfig,
    context::{KeyContext, SourceRef},
    lock::Lockfile,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractReport {
    /// Fully-qualified keys (namespace resolved, e.g. `useTranslations("auth")` + `t("login")` => `auth.login`).
    pub usages: BTreeMap<Key, Vec<Usage>>,
    /// User-visible string literals not wrapped in a translation call.
    pub hardcoded: Vec<Hardcoded>,
    /// Calls whose key is not statically known (`t(variable)`, template literals).
    pub dynamic: Vec<SourceRef>,
    /// Static prefixes of dynamic keys (`t(`errors.${code}`)` => `errors.`); keys under them count as used.
    #[serde(default)]
    pub dynamic_prefixes: BTreeSet<Key>,
    /// UI copy in object literals (config arrays, `metadata`, props objects), with a `reason`.
    #[serde(default)]
    pub config: Vec<Hardcoded>,
    /// Non-fatal notes (`path:line: …`), e.g. Swift interpolations of unknown type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub files_scanned: usize,
}

impl ExtractReport {
    pub fn merge(&mut self, other: ExtractReport) {
        for (k, v) in other.usages {
            self.usages.entry(k).or_default().extend(v);
        }
        self.hardcoded.extend(other.hardcoded);
        self.config.extend(other.config);
        self.dynamic.extend(other.dynamic);
        self.dynamic_prefixes.extend(other.dynamic_prefixes);
        self.warnings.extend(other.warnings);
        self.files_scanned += other.files_scanned;
    }

    /// `stripped`: usage keys without their i18next namespace.
    fn is_used(&self, key: &str, stripped: &BTreeSet<&str>) -> bool {
        // `home:title` (namespaced catalog) is used by `t("home:title")` or by a bare
        // `t("title")` (default namespace unknown); `title` (one file) by `t("home:title")` too.
        let (ns, bare) = scan::split_ns(key);
        let has = |k: &str| self.usages.contains_key(k) || (ns.is_none() && stripped.contains(k));
        let hit = |k: &str| {
            let plural_base = k
                .rsplit_once('_')
                .filter(|(_, s)| PLURAL_SUFFIXES.contains(s))
                .map(|(b, _)| b);
            has(k)
                || plural_base.is_some_and(has)
                // `t.raw("list")` uses the whole subtree.
                || k.match_indices('.').any(|(i, _)| has(&k[..i]))
        };
        hit(key)
            || (ns.is_some() && hit(bare))
            || self
                .dynamic_prefixes
                .iter()
                .any(|p| key.starts_with(p.as_str()) || bare.starts_with(scan::strip_ns(p)))
    }
}

const PLURAL_SUFFIXES: &[&str] = &["zero", "one", "two", "few", "many", "other"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub at: SourceRef,
    pub ui_role: Option<String>,
    pub component: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hardcoded {
    pub at: SourceRef,
    pub text: String,
    /// button | heading | label | placeholder | aria | title | link | text
    pub ui_role: Option<String>,
    pub component: Option<String>,
    /// e.g. `settings.billing.save_button`, derived from file path, component and text.
    pub suggested_key: Key,
    /// Why it is still hardcoded (config/data entries only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error on {0}: {1}")]
    Io(std::path::PathBuf, std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Scan `root` with include/exclude globs from `cfg`.
pub fn extract(root: &Path, cfg: &ExtractConfig) -> Result<ExtractReport, Error> {
    let include = patterns(&cfg.include)?;
    let exclude = patterns(&cfg.exclude)?;
    let opts = glob::MatchOptions {
        require_literal_separator: true,
        ..Default::default()
    };
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    let mut report = ExtractReport::default();
    let mut swift = vec![];
    for rel in files {
        let matches = |ps: &[glob::Pattern]| ps.iter().any(|p| p.matches_with(&rel, opts));
        if !matches(&include) || matches(&exclude) {
            continue;
        }
        let path = root.join(&rel);
        let src = std::fs::read_to_string(&path).map_err(|e| Error::Io(path, e))?;
        if rel.ends_with(".swift") {
            swift.push((rel, src));
            continue;
        }
        report.merge(extract_source(&rel, &src, cfg));
    }
    // Swift: interpolation types are looked up project-wide (models live in other files).
    let types = swift::project_types(swift.iter().map(|(_, s)| s.as_str()));
    for (rel, src) in &swift {
        report.merge(swift::report(rel, src, &cfg.functions, &types));
    }
    Ok(report)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.into(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(dir.into(), e))?;
        let path = entry.path();
        let name = entry.file_name();
        let ft = entry.file_type().map_err(|e| Error::Io(path.clone(), e))?;
        if ft.is_dir() {
            if !matches!(
                name.to_str(),
                Some(
                    "node_modules"
                        | ".next"
                        | "dist"
                        | ".git"
                        | "Pods"
                        | "DerivedData"
                        | ".build"
                        | "Carthage"
                )
            ) {
                walk(root, &path, out)?;
            }
        } else if ft.is_file()
            && let Ok(rel) = path.strip_prefix(root)
            && let Some(rel) = rel.to_str()
        {
            out.push(rel.replace('\\', "/"));
        }
    }
    Ok(())
}

fn patterns(globs: &[String]) -> Result<Vec<glob::Pattern>, Error> {
    globs
        .iter()
        .flat_map(|g| expand_braces(g))
        .map(|g| {
            glob::Pattern::new(&g).map_err(|e| Error::Other(format!("invalid glob {g:?}: {e}")))
        })
        .collect()
}

/// `src/**/*.{ts,tsx}` -> `src/**/*.ts`, `src/**/*.tsx` (the `glob` crate has no brace support).
// ponytail: no nested braces, add a real parser if someone needs `{a,{b,c}}`.
fn expand_braces(g: &str) -> Vec<String> {
    let Some(open) = g.find('{') else {
        return vec![g.to_string()];
    };
    let Some(close) = g[open..].find('}').map(|c| open + c) else {
        return vec![g.to_string()];
    };
    g[open + 1..close]
        .split(',')
        .flat_map(|alt| expand_braces(&format!("{}{alt}{}", &g[..open], &g[close + 1..])))
        .collect()
}

/// Scan a single in-memory source (used by tests and MCP `get_context` on unsaved code).
pub fn extract_source(path: &str, source: &str, cfg: &ExtractConfig) -> ExtractReport {
    // Rails / Django / Laravel / Symfony / Angular templates: literal patterns, not oxc.
    if foreign::handles(path) {
        return foreign::scan(path, source);
    }
    let mut report = scan::scan(path, source, &cfg.functions);
    if path.ends_with(".ts") {
        report.merge(foreign::angular_services(path, source));
    }
    if path.ends_with(".vue") {
        report.merge(foreign::vue_template(path, source));
    }
    report
}

/// Build per-key context from usages, lock notes and sibling keys of the source catalog.
pub fn contexts(
    report: &ExtractReport,
    source: &Catalog,
    lock: &Lockfile,
) -> BTreeMap<Key, KeyContext> {
    let keys: BTreeSet<&Key> = report.usages.keys().chain(source.entries.keys()).collect();
    keys.into_iter()
        .map(|key| {
            let usages = report
                .usages
                .get(key)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let parent = parent_of(key);
            // ponytail: linear scan per key at the root level; fine for catalogs of a few thousand keys.
            let neighbors = source
                .entries
                .iter()
                .filter(|(k, _)| *k != key && parent_of(k) == parent)
                .take(8)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let ctx = KeyContext {
                key: key.clone(),
                description: lock.notes.get(key).cloned(),
                refs: usages.iter().map(|u| u.at.clone()).collect(),
                ui_role: usages.iter().find_map(|u| u.ui_role.clone()),
                component: usages.iter().find_map(|u| u.component.clone()),
                neighbors,
                ..Default::default()
            };
            (key.clone(), ctx)
        })
        .collect()
}

fn parent_of(key: &str) -> &str {
    key.rsplit_once('.').map_or("", |(p, _)| p)
}

/// Keys in `source` never referenced (ignores keys under a prefix used dynamically).
pub fn unused_keys(report: &ExtractReport, source: &Catalog) -> Vec<Key> {
    let stripped: BTreeSet<&str> = report.usages.keys().map(|u| scan::strip_ns(u)).collect();
    source
        .entries
        .keys()
        .filter(|k| !report.is_used(k, &stripped))
        .cloned()
        .collect()
}

/// Keys referenced in code but absent from `source`.
pub fn missing_keys(report: &ExtractReport, source: &Catalog) -> Vec<Key> {
    let exact: BTreeSet<&str> = source.entries.keys().map(String::as_str).collect();
    let bare: BTreeSet<&str> = source.entries.keys().map(|k| scan::strip_ns(k)).collect();
    let has = |keys: &BTreeSet<&str>, k: &str| {
        let sub = format!("{k}.");
        keys.contains(k)
            || PLURAL_SUFFIXES
                .iter()
                .any(|s| keys.contains(format!("{k}_{s}").as_str()))
            || keys
                .range(sub.as_str()..)
                .next()
                .is_some_and(|c| c.starts_with(&sub))
    };
    report
        .usages
        .keys()
        .filter(|k| match scan::split_ns(k) {
            // `t("home:title")`: that namespace, or a one-file catalog's `title`.
            (Some(_), b) => !has(&exact, k) && !has(&exact, b),
            // `t("title")`: any namespace.
            (None, _) => !has(&exact, k) && !has(&bare, k),
        })
        .cloned()
        .collect()
}
