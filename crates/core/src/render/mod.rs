//! Render safety per locale, without a browser (`glot labs render`).
//!
//! - [`fonts`]: does each declared/detected webfont have a glyph for every character of every
//!   translation (cmap), or does the text fall back to a system font?
//! - [`bidi`]: RTL strings whose placeholders or Latin words reorder wrongly (real Unicode Bidi
//!   Algorithm run on a sample, compared with the isolated rendering), reversed brackets.
//! - [`typo`]: digit systems mixed within a locale, decimal separators, non-breaking spaces,
//!   locale quotes.
//!
//! [`check_entry`] reports, [`fix_entry`] returns the fixed text (FSI/PDI isolation, RLM, NBSP,
//! quotes, decimal separator); the caller writes it through the validator.
use serde::Serialize;
use std::ops::Range;

use crate::validate::{Severity, Syntax};

pub mod bidi;
pub mod fonts;
pub mod typo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Character with no glyph anywhere (U+FFFD, private use).
    Tofu,
    Bidi,
    Brackets,
    Digits,
    Separator,
    Nbsp,
    Quotes,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub locale: String,
    pub key: String,
    pub check: Check,
    pub severity: Severity,
    pub message: String,
    /// `--fix` can repair it (the fixed text comes from [`fix_entry`]).
    pub fixable: bool,
}

/// `ar`, `he`, `fa`, `ur`… (also `ar-EG`, `fa_IR`).
pub fn is_rtl(locale: &str) -> bool {
    matches!(
        lang(locale),
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ckb"
    )
}

pub(crate) fn lang(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or(locale)
}

/// Every per-string check. `source`: the source-locale text (decimal separator guard).
pub fn check_entry(
    locale: &str,
    key: &str,
    source: Option<&str>,
    text: &str,
    syntax: Syntax,
) -> Vec<Finding> {
    let segs = segments(text, syntax);
    let mut out = vec![];
    let mut push = |check, severity, message: String, fixable| {
        out.push(Finding {
            locale: locale.into(),
            key: key.into(),
            check,
            severity,
            message,
            fixable,
        })
    };
    for c in text_chars(text, &segs) {
        if c == '\u{FFFD}' {
            push(
                Check::Tofu,
                Severity::Error,
                "contains U+FFFD � (an encoding error): renders as a replacement box everywhere"
                    .into(),
                false,
            );
            break;
        }
        if matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..) {
            push(
                Check::Tofu,
                Severity::Warning,
                format!(
                    "contains private-use U+{:04X}: tofu unless the icon font that defines it is applied",
                    c as u32
                ),
                false,
            );
            break;
        }
    }
    if is_rtl(locale) {
        for (check, severity, message, fixable) in bidi::check(text, &segs, syntax) {
            push(check, severity, message, fixable);
        }
    }
    for (check, message) in typo::check(locale, source, text, &segs, syntax) {
        push(check, Severity::Warning, message, true);
    }
    out
}

/// The text with every fixable finding repaired, `None` when nothing changes.
pub fn fix_entry(locale: &str, source: Option<&str>, text: &str, syntax: Syntax) -> Option<String> {
    let mut t = text.to_string();
    if is_rtl(locale)
        && let Some(f) = bidi::fix(&t, syntax)
    {
        t = f;
    }
    if let Some(f) = typo::fix(locale, source, &t, syntax) {
        t = f;
    }
    (t != text).then_some(t)
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub fonts: Vec<fonts::Family>,
    pub notes: Vec<String>,
    /// Font × locale: characters without a glyph (they render with a fallback font).
    pub glyphs: Vec<fonts::Gap>,
    pub findings: Vec<Finding>,
    pub errors: usize,
    pub warnings: usize,
    pub fixable: usize,
}

/// `[measure] fonts = [...]` of glot.toml (files, globs or family names).
pub fn declared_fonts(root: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(root.join(crate::config::CONFIG_FILE))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| {
            Some(
                t.get("measure")?
                    .get("fonts")?
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
            )
        })
        .unwrap_or_default()
}

/// Every check over the catalogs of `locales` (all when empty). `fonts`: overrides glot.toml.
pub fn report(
    root: &std::path::Path,
    config: &crate::config::Config,
    catalogs: &std::collections::BTreeMap<String, crate::catalog::Catalog>,
    locales: &[String],
    fonts: &[String],
) -> Report {
    let (declared, origin) = if fonts.is_empty() {
        (declared_fonts(root), "glot.toml")
    } else {
        (fonts.to_vec(), "--font")
    };
    let (families, notes) = fonts::discover(root, &declared, origin);
    let wanted = |l: &str| locales.is_empty() || locales.iter().any(|x| x == l);
    let source = catalogs.get(&config.source_locale);
    let syntax = config.syntax;
    let mut findings = vec![];
    for (locale, cat) in catalogs.iter().filter(|(l, _)| wanted(l)) {
        let is_source = *locale == config.source_locale;
        for (key, text) in &cat.entries {
            let src = source
                .filter(|_| !is_source)
                .and_then(|s| s.entries.get(key))
                .map(String::as_str);
            // Untranslated copies and code pasted into the catalog are `glot check`'s findings.
            if src == Some(text.as_str())
                || source
                    .and_then(|s| s.entries.get(key))
                    .is_some_and(|s| !crate::validate::lint_source(key, s).is_empty())
            {
                continue;
            }
            findings.extend(check_entry(locale, key, src, text, syntax));
        }
        findings.extend(typo::check_digits(locale, &cat.entries, syntax));
    }
    findings.sort_by(|a, b| {
        (a.severity != Severity::Error, &a.locale, a.check, &a.key).cmp(&(
            b.severity != Severity::Error,
            &b.locale,
            b.check,
            &b.key,
        ))
    });
    let glyphs = fonts::gaps(
        &families,
        catalogs
            .iter()
            .filter(|(l, _)| wanted(l))
            .map(|(l, c)| (l, &c.entries)),
        syntax,
    );
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    Report {
        errors,
        warnings: findings.len() - errors + glyphs.len(),
        fixable: findings.iter().filter(|f| f.fixable).count(),
        fonts: families,
        notes,
        glyphs,
        findings,
    }
}

// ---------------------------------------------------------------- message segments

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seg {
    Text,
    /// Placeholder whose value is text of unknown direction (`%@`, `{name}`).
    Str,
    /// Number placeholder (`%d`, `{n, number}`, `#`).
    Num,
    /// `<b>`, `</1>`: not rendered.
    Markup,
    /// Plural/select structure: what is on each side never renders next to each other.
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    pub seg: Seg,
    pub range: Range<usize>,
}

/// Characters of the literal text (placeholders and markup excluded).
pub(crate) fn text_chars<'a>(text: &'a str, segs: &'a [Span]) -> impl Iterator<Item = char> + 'a {
    segs.iter()
        .filter(|s| s.seg == Seg::Text)
        .flat_map(move |s| text[s.range.clone()].chars())
}

fn numeric_name(name: &str) -> bool {
    let n = name.trim().trim_start_matches('$').to_ascii_lowercase();
    n == "n"
        || [
            "count", "num", "total", "amount", "qty", "quantity", "percent", "price", "days",
            "hours", "minutes", "seconds", "score",
        ]
        .iter()
        .any(|w| n.contains(w))
}

/// Split a message into literal text, placeholders, markup and plural/select structure.
// ponytail: a scanner, not the validator's parser (that one has no byte positions); ICU
// apostrophe quoting is not interpreted.
pub(crate) fn segments(s: &str, syntax: Syntax) -> Vec<Span> {
    let mut out = vec![];
    scan(s, 0..s.len(), syntax, false, &mut out);
    out
}

fn push(out: &mut Vec<Span>, seg: Seg, range: Range<usize>) {
    if range.is_empty() && seg != Seg::Break {
        return;
    }
    if seg == Seg::Text
        && let Some(last) = out.last_mut()
        && last.seg == Seg::Text
        && last.range.end == range.start
    {
        last.range.end = range.end;
        return;
    }
    out.push(Span { seg, range });
}

/// Byte index of the `}` closing the `{` at `open`.
fn closing(s: &str, open: usize, end: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (j, c) in s[open..end].char_indices() {
        depth += (c == '{') as i32 - (c == '}') as i32;
        if depth == 0 {
            return Some(open + j);
        }
    }
    None
}

fn scan(s: &str, range: Range<usize>, syntax: Syntax, in_plural: bool, out: &mut Vec<Span>) {
    let mut i = range.start;
    while i < range.end {
        let rest = &s[i..range.end];
        let c = rest.chars().next().unwrap_or_default();
        let one = i..i + c.len_utf8();
        // <tag>, </tag>, <1/>
        if c == '<'
            && rest[1..]
                .chars()
                .next()
                .is_some_and(|n| n.is_ascii_alphanumeric() || n == '/')
            && let Some(j) = rest.find('>').filter(|&j| !rest[1..j].contains('<'))
        {
            push(out, Seg::Markup, i..i + j + 1);
            i += j + 1;
            continue;
        }
        if c == '%' && matches!(syntax, Syntax::Printf | Syntax::Rails) {
            if rest.starts_with("%%") {
                push(out, Seg::Text, i..i + 2);
                i += 2;
                continue;
            }
            if let Some(rest2) = rest.strip_prefix("%{").or(rest.strip_prefix("%("))
                && let Some(j) = rest2.find(['}', ')'])
            {
                // `%(name)s`: the conversion letter follows.
                let mut len = 2 + j + 1;
                if rest.starts_with("%(") {
                    len += rest[len..].chars().next().map_or(0, char::len_utf8);
                }
                push(out, Seg::Str, i..i + len);
                i += len;
                continue;
            }
            if syntax == Syntax::Printf
                && let Some((len, num)) = printf_spec(rest)
            {
                push(out, if num { Seg::Num } else { Seg::Str }, i..i + len);
                i += len;
                continue;
            }
        }
        if c == '#' && in_plural {
            push(out, Seg::Num, one);
            i += 1;
            continue;
        }
        if c == '$' && syntax == Syntax::I18next && rest.starts_with("$t(") {
            let j = rest.find(')').map_or(rest.len(), |j| j + 1);
            push(out, Seg::Str, i..i + j);
            i += j;
            continue;
        }
        if c == '{' {
            if rest.starts_with("{{") && syntax == Syntax::I18next {
                let j = rest.find("}}").map_or(rest.len(), |j| j + 2);
                let name = rest[2..j.saturating_sub(2).max(2)]
                    .split(',')
                    .next()
                    .unwrap_or("");
                push(
                    out,
                    if numeric_name(name) {
                        Seg::Num
                    } else {
                        Seg::Str
                    },
                    i..i + j,
                );
                i += j;
                continue;
            }
            if let Some(close) = closing(s, i, range.end) {
                let inner = &s[i + 1..close];
                let mut parts = inner.splitn(3, ',');
                let name = parts.next().unwrap_or("").trim();
                let kind = parts.next().map(|k| k.trim().to_ascii_lowercase());
                match kind.as_deref() {
                    Some(k @ ("plural" | "select" | "selectordinal")) if inner.contains('{') => {
                        push(out, Seg::Break, i..i);
                        let plural = in_plural || k != "select";
                        // Branches start after `name, kind,`.
                        let first = inner.find(',').unwrap_or(0);
                        let second = inner[first + 1..]
                            .find(',')
                            .map_or(first, |k| first + 1 + k);
                        let mut j = i + 1 + second + 1;
                        while j < close {
                            let Some(open) = s[j..close].find('{').map(|k| j + k) else {
                                break;
                            };
                            let Some(end) = closing(s, open, close + 1) else {
                                break;
                            };
                            scan(s, open + 1..end, syntax, plural, out);
                            push(out, Seg::Break, end..end);
                            j = end + 1;
                        }
                        i = close + 1;
                        continue;
                    }
                    _ if !inner.contains('{') && !name.is_empty() => {
                        let num = match kind.as_deref() {
                            Some(k) => {
                                k.starts_with("number")
                                    || k.starts_with(":number")
                                    || ["date", "time", "duration", "spellout", "ordinal"]
                                        .iter()
                                        .any(|w| k.starts_with(w))
                            }
                            None => {
                                numeric_name(name)
                                    || inner.contains(":number")
                                    || inner.contains(":integer")
                            }
                        };
                        push(out, if num { Seg::Num } else { Seg::Str }, i..close + 1);
                        i = close + 1;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        push(out, Seg::Text, one);
        i += c.len_utf8();
    }
}

/// `%@`, `%1$@`, `%-5.2lf`, `%lld`: (byte length, is a number).
fn printf_spec(s: &str) -> Option<(usize, bool)> {
    let b = s.as_bytes();
    let mut j = 1;
    let digits = |j: &mut usize| {
        while *j < b.len() && b[*j].is_ascii_digit() {
            *j += 1;
        }
    };
    let save = j;
    digits(&mut j);
    if j < b.len() && b[j] == b'$' && j > save {
        j += 1;
    } else {
        j = save;
    }
    while j < b.len() && b"-+ #0'".contains(&b[j]) {
        j += 1;
    }
    digits(&mut j);
    if j < b.len() && b[j] == b'.' {
        j += 1;
        digits(&mut j);
    }
    for len in ["hh", "ll", "h", "l", "q", "L", "z", "j", "t"] {
        if s[j..].starts_with(len) {
            j += len.len();
            break;
        }
    }
    let conv = *b.get(j)?;
    let num = b"dDiuUxXoOfFeEgGaA".contains(&conv);
    (num || b"@sScCp".contains(&conv)).then_some((j + 1, num))
}

#[cfg(test)]
mod tests;
