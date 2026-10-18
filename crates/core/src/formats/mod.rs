//! Native (non-JSON) catalog formats: Apple String Catalog, `.strings`/`.stringsdict`, Android
//! `strings.xml`, Flutter ARB, gettext PO and YAML (Rails/Symfony).
//!
//! Reading flattens a file into `key => message`; plurals become an ICU wrapper
//! (`{count, plural, one {%d item} other {%d items}}`) whose branches keep the native text verbatim
//! (`%d`, `%1$s`, `%{count}`), so the validator checks categories and placeholders.
//! Writing **patches the existing file**: untouched entries, comments, attributes and order are
//! kept byte for byte; only changed values are re-rendered, removed keys dropped, new keys appended.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::catalog::{Catalog, Format};
use crate::config::{Config, read_optional};
use crate::lock::Lockfile;
use crate::{Error, Key, Locale, Result};

mod android;
mod arb;
mod po;
mod strings;
mod xcstrings;
mod xml;
mod yaml;

#[cfg(test)]
mod tests;

/// What a native file says beyond the plain values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub entries: BTreeMap<Key, String>,
    /// Translator context: comments, ARB `@key.description`, gettext `#.` / `#:`, xcstrings `comment`.
    pub notes: BTreeMap<Key, String>,
    /// Flagged for review in the file (gettext `fuzzy`, xcstrings `needs_review`).
    pub stale: BTreeSet<Key>,
    /// Source text implied by the key when the file stores none (gettext `msgid`).
    pub implicit: BTreeMap<Key, String>,
}

/// The native format handling `path`, `None` for JSON catalogs. `Auto`/JSON formats resolve by
/// extension so an `.arb` or `.xcstrings` (both JSON) is never rewritten as a plain catalog.
pub fn native(format: Format, path: &Path) -> Option<Format> {
    match format {
        Format::Auto | Format::JsonNested | Format::JsonFlat => {
            match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
                "xcstrings" => Some(Format::Xcstrings),
                "strings" | "stringsdict" => Some(Format::Strings),
                "xml" => Some(Format::Android),
                "arb" => Some(Format::Arb),
                "po" | "pot" => Some(Format::Po),
                "yml" | "yaml" => Some(Format::Yaml),
                _ => None,
            }
        }
        f => Some(f),
    }
}

pub fn parse(format: Format, path: &Path, text: &str, locale: &str) -> Result<Parsed, String> {
    match format {
        Format::Xcstrings => xcstrings::parse(text, locale),
        Format::Strings if is_stringsdict(path) => strings::parse_dict(text),
        Format::Strings => strings::parse(text),
        Format::Android => android::parse(text),
        Format::Arb => arb::parse(text),
        Format::Po => po::parse(text, locale),
        Format::Yaml => yaml::parse(text, locale),
        Format::Auto | Format::JsonNested | Format::JsonFlat => Err("not a native format".into()),
    }
}

/// New file content for `cat`, patched into `existing` (`None` = new file).
pub fn write(format: Format, path: &Path, existing: Option<&str>, cat: &Catalog) -> String {
    match format {
        Format::Xcstrings => xcstrings::write(existing, cat),
        Format::Strings if is_stringsdict(path) => strings::write_dict(existing, cat),
        Format::Strings => strings::write(existing, cat),
        Format::Android => android::write(existing, cat),
        Format::Arb => arb::write(existing, cat),
        Format::Po => po::write(existing, cat),
        Format::Yaml => yaml::write(existing, cat, path),
        Format::Auto | Format::JsonNested | Format::JsonFlat => cat.to_json(format),
    }
}

/// Read a native file (`None` when `path` is JSON or missing).
pub fn read(path: &Path, locale: &str, format: Format) -> Result<Option<Parsed>> {
    let Some(f) = native(format, path) else {
        return Ok(None);
    };
    let Some(text) = read_optional(path)? else {
        return Ok(None);
    };
    parse(f, path, &text, locale)
        .map(Some)
        .map_err(|message| Error::Parse {
            path: path.to_path_buf(),
            message,
        })
}

/// Fold native metadata into a loaded workspace (in memory): implicit source text fills empty
/// source values, file notes become translator notes (lock notes win), and entries flagged
/// `fuzzy`/`needs_review` are planned as stale.
pub fn load_meta(
    config: &Config,
    root: &Path,
    catalogs: &mut BTreeMap<Locale, Catalog>,
    lock: &mut Lockfile,
) -> Result<()> {
    for loc in config.all_locales() {
        for p in config.catalog_paths(root, &loc) {
            let Some(meta) = read(&p, &loc, config.format)? else {
                continue;
            };
            for (k, n) in meta.notes {
                lock.notes.entry(k).or_insert(n);
            }
            let Some(cat) = catalogs.get_mut(&loc) else {
                continue;
            };
            if loc == config.source_locale {
                for (k, v) in meta.implicit {
                    let e = cat.entries.entry(k).or_default();
                    if e.is_empty() {
                        *e = v;
                    }
                }
                continue;
            }
            for k in meta.stale {
                let Some(v) = cat.entries.get(&k).filter(|v| !v.is_empty()) else {
                    continue;
                };
                let entries = lock.entries.entry(loc.clone()).or_default();
                match entries.get_mut(&k) {
                    Some(e) => e.source_hash.clear(),
                    None => {
                        // ponytail: built through serde so new defaulted LockEntry fields need no change here.
                        let e = serde_json::from_value(serde_json::json!({
                            "source_hash": "",
                            "target_hash": crate::hash::hash(v),
                            "origin": {"kind": "imported"},
                        }))
                        .expect("valid lock entry");
                        entries.insert(k, e);
                    }
                }
            }
        }
    }
    Ok(())
}

fn is_stringsdict(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "stringsdict")
}

// ---------------------------------------------------------------- ICU plural helpers

/// `{var, kind, k1 {v1} k2 {v2}}`.
pub(crate) fn icu_choice(var: &str, kind: &str, forms: &[(String, String)]) -> String {
    let body: Vec<String> = forms.iter().map(|(k, v)| format!("{k} {{{v}}}")).collect();
    format!("{{{var}, {kind}, {}}}", body.join(" "))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Choice {
    pub start: usize,
    pub end: usize,
    pub var: String,
    pub kind: String,
    pub forms: Vec<(String, String)>,
}

impl Choice {
    pub fn form(&self, cat: &str) -> Option<&str> {
        self.forms
            .iter()
            .find(|(k, _)| k == cat)
            .map(|(_, v)| v.as_str())
    }
}

/// Top-level `{var, plural|select|selectordinal, ...}` blocks (plain brace matching: native
/// branch text is kept verbatim, ICU quoting does not apply).
pub(crate) fn choices(s: &str) -> Vec<Choice> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{'
            && let Some(end) = matching(s, i)
        {
            if let Some(c) = parse_choice(&s[i + 1..end]) {
                out.push(Choice {
                    start: i,
                    end: end + 1,
                    ..c
                });
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The message when it is exactly one choice.
pub(crate) fn whole_choice(s: &str) -> Option<Choice> {
    choices(s)
        .into_iter()
        .next()
        .filter(|c| c.start == 0 && c.end == s.len())
}

/// The message as one choice, surrounding text folded into every branch:
/// `pre {n, plural, a {x}} post` => `{n, plural, a {pre x post}}` (formats whose plurals are whole
/// messages: Android, gettext, YAML, xcstrings variations).
pub(crate) fn folded_choice(s: &str) -> Option<Choice> {
    let cs = choices(s);
    let [c] = cs.as_slice() else { return None };
    let (pre, post) = (&s[..c.start], &s[c.end..]);
    Some(Choice {
        start: 0,
        end: s.len(),
        forms: c
            .forms
            .iter()
            .map(|(k, v)| (k.clone(), format!("{pre}{v}{post}")))
            .collect(),
        ..c.clone()
    })
}

/// Index of the `}` closing the `{` at `open`.
fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_choice(inner: &str) -> Option<Choice> {
    let (var, rest) = inner.split_once(',')?;
    let (kind, mut rest) = rest.split_once(',')?;
    let (var, kind) = (var.trim(), kind.trim());
    if var.is_empty()
        || var.contains(char::is_whitespace)
        || !matches!(kind, "plural" | "select" | "selectordinal")
    {
        return None;
    }
    let mut forms = vec![];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let open = rest.find('{')?;
        let key = rest[..open].trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return None;
        }
        let close = matching(rest, open)?;
        forms.push((key.to_string(), rest[open + 1..close].to_string()));
        rest = &rest[close + 1..];
    }
    (!forms.is_empty()).then(|| Choice {
        start: 0,
        end: 0,
        var: var.into(),
        kind: kind.into(),
        forms,
    })
}

/// Start of the line holding byte `i`, when only whitespace precedes it on that line.
pub(crate) fn line_start(s: &str, i: usize) -> usize {
    let ls = s[..i].rfind('\n').map_or(0, |n| n + 1);
    if s[ls..i].trim().is_empty() { ls } else { i }
}

/// End of the line after byte `i` (past its `\n`), when only whitespace follows.
pub(crate) fn line_end(s: &str, i: usize) -> usize {
    let le = s[i..].find('\n').map_or(s.len(), |n| i + n + 1);
    if s[i..le].trim().is_empty() { le } else { i }
}
