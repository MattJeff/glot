//! Typography that breaks or reads wrong once rendered: a line break inside "10 €" or before "!"
//! in French, straight/foreign quotes, a decimal point copied from the source, digit systems mixed
//! within one locale (Latin 0-9 next to Arabic-Indic ٠-٩).
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::{Check, Finding, Seg, Span, lang, segments};
use crate::validate::{Severity, Syntax};

const NBSP: &str = "\u{00A0}";
const NNBSP: &str = "\u{202F}";

/// Apply non-overlapping edits (byte range → replacement; empty range = insertion).
pub(crate) fn apply(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|e| (e.0.start, e.0.end));
    let mut out = String::with_capacity(text.len() + 16);
    let mut at = 0;
    for (r, s) in edits {
        if r.start < at {
            continue; // overlapping: keep the first
        }
        out.push_str(&text[at..r.start]);
        out.push_str(&s);
        at = r.end;
    }
    out.push_str(&text[at..]);
    out
}

/// One character in reading order with where it comes from: literal text, or a stand-in for a
/// placeholder (`0` for a number, `a` for text) so "{count} €" is seen as a number before "€".
struct Ch {
    at: usize,
    c: char,
    text: bool,
}

fn chars(text: &str, segs: &[Span]) -> Vec<Ch> {
    let mut v = vec![];
    for s in segs {
        match s.seg {
            Seg::Text => v.extend(text[s.range.clone()].char_indices().map(|(i, c)| Ch {
                at: s.range.start + i,
                c,
                text: true,
            })),
            Seg::Num | Seg::Str => v.push(Ch {
                at: s.range.start,
                c: if s.seg == Seg::Num { '0' } else { 'a' },
                text: false,
            }),
            // A branch boundary or a tag is not a neighbour.
            Seg::Break | Seg::Markup => v.push(Ch {
                at: s.range.start,
                c: '\u{1}',
                text: false,
            }),
        }
    }
    v
}

fn quotes(lang: &str, locale: &str) -> Option<(char, char)> {
    let hant = ["zh-Hant", "zh-TW", "zh-HK", "zh_TW", "zh_HK"]
        .iter()
        .any(|p| locale.starts_with(p));
    Some(match lang {
        "fr" | "ru" | "uk" | "be" | "it" | "es" | "ca" | "nb" | "no" | "el" | "fa" | "hy" => {
            ('«', '»')
        }
        "de" | "cs" | "sk" | "lt" | "bg" | "is" | "da" => ('„', '“'),
        "pl" | "hu" | "ro" | "hr" => ('„', '”'),
        "ja" => ('「', '」'),
        "zh" if hant => ('「', '」'),
        "zh" | "ko" | "nl" | "pt" | "tr" | "vi" | "th" | "hi" | "id" => ('“', '”'),
        "sv" | "fi" => ('”', '”'),
        _ => return None,
    })
}

/// Locales writing 4,5 (comma decimal) vs 4.5.
fn comma_decimal(lang: &str) -> Option<bool> {
    match lang {
        "fr" | "de" | "es" | "it" | "pt" | "ru" | "uk" | "pl" | "nl" | "tr" | "id" | "vi"
        | "cs" | "sk" | "sv" | "da" | "nb" | "no" | "fi" | "ro" | "hu" | "el" | "bg" | "hr"
        | "sl" | "lt" | "lv" | "et" | "ca" => Some(true),
        "en" | "ja" | "zh" | "ko" | "th" | "hi" | "he" | "ms" | "fil" | "tl" => Some(false),
        _ => None,
    }
}

/// Locales grouping thousands with a space: "10 000" must not break across lines.
fn space_grouping(lang: &str) -> bool {
    matches!(
        lang,
        "fr" | "ru"
            | "uk"
            | "pl"
            | "cs"
            | "sk"
            | "sv"
            | "nb"
            | "no"
            | "fi"
            | "hu"
            | "bg"
            | "et"
            | "lv"
            | "lt"
    )
}

/// Every fix with its check and reason.
fn edits(
    locale: &str,
    source: Option<&str>,
    text: &str,
    segs: &[Span],
) -> Vec<(Range<usize>, String, Check, &'static str)> {
    let lang = lang(locale);
    let v = chars(text, segs);
    let mut out = vec![];
    let at = |k: usize| v.get(k).map(|c| c.c);
    let ws_or_end = |k: usize| at(k).is_none_or(|c| c.is_whitespace() || "»”\"')".contains(c));
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || "”\")]".contains(c));
    let fr = lang == "fr";
    let fr_ca = fr && locale[2..].to_ascii_lowercase().contains("ca");
    for k in 0..v.len() {
        let Ch {
            at: pos,
            c,
            text: lit,
        } = v[k];
        if !lit {
            continue;
        }
        let prev = k.checked_sub(1).map(|j| &v[j]);
        let prev_space = prev.filter(|p| p.text && p.c == ' ');
        let here = pos..pos + c.len_utf8();
        if fr && (";!?".contains(c) && !fr_ca || c == ':') {
            let space = if c == ':' { NBSP } else { NNBSP };
            let after = ws_or_end(k + 1) || at(k + 1).is_some_and(|n| ";!?".contains(n));
            if let Some(p) = prev_space {
                out.push((
                    p.at..p.at + 1,
                    space.into(),
                    Check::Nbsp,
                    "no-break space before ; : ! ?",
                ));
            } else if word(prev.map(|p| p.c))
                && after
                && !(c == ':' && prev.is_some_and(|p| p.c.is_ascii_digit()))
            {
                out.push((
                    pos..pos,
                    space.into(),
                    Check::Nbsp,
                    "missing no-break space before ; : ! ?",
                ));
            }
        }
        if fr && c == '«' {
            match at(k + 1) {
                Some(' ') => out.push((
                    v[k + 1].at..v[k + 1].at + 1,
                    NNBSP.into(),
                    Check::Nbsp,
                    "no-break space inside « »",
                )),
                Some(n) if n.is_alphanumeric() => out.push((
                    here.end..here.end,
                    NNBSP.into(),
                    Check::Nbsp,
                    "no-break space inside « »",
                )),
                _ => {}
            }
        }
        if fr && c == '»' {
            if let Some(p) = prev_space {
                out.push((
                    p.at..p.at + 1,
                    NNBSP.into(),
                    Check::Nbsp,
                    "no-break space inside « »",
                ));
            } else if word(prev.map(|p| p.c)) {
                out.push((
                    pos..pos,
                    NNBSP.into(),
                    Check::Nbsp,
                    "no-break space inside « »",
                ));
            }
        }
        if c == ' ' && prev.is_some_and(|p| p.c.is_ascii_digit()) {
            // "10 €", "{count} %": the unit must not wrap to the next line.
            if at(k + 1).is_some_and(|n| "%‰€£¥₽₺₹₩₫₪₴".contains(n)) {
                out.push((
                    here.clone(),
                    if fr { NNBSP } else { NBSP }.into(),
                    Check::Nbsp,
                    "no-break space between a number and its unit",
                ));
            } else if space_grouping(lang)
                && (1..=3).all(|j| at(k + j).is_some_and(|c| c.is_ascii_digit()))
                && !at(k + 4).is_some_and(|c| c.is_ascii_digit())
                && (1..=4).any(|j| {
                    !k.checked_sub(j)
                        .and_then(at)
                        .is_some_and(|c| c.is_ascii_digit())
                })
            {
                out.push((
                    here.clone(),
                    NNBSP.into(),
                    Check::Nbsp,
                    "no-break space as thousands separator (10 000)",
                ));
            }
        }
        // 4.5 copied from the source into a 4,5 locale (and the reverse).
        if let (Some(comma), Some(src)) = (comma_decimal(lang), source)
            && c == if comma { '.' } else { ',' }
        {
            let digits_before: String = (0..k)
                .rev()
                .map(|j| v[j].c)
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let digits_after: String = v[k + 1..]
                .iter()
                .map(|c| c.c)
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let before_num = k.checked_sub(digits_before.len() + 1).and_then(at);
            if !digits_before.is_empty()
                && (1..=2).contains(&digits_after.len())
                && !at(k + 1 + digits_after.len()).is_some_and(|n| n == c || n.is_alphanumeric())
                && !before_num.is_some_and(|b| b.is_alphabetic() || b == c)
                && v[k - digits_before.len()..=k + digits_after.len()]
                    .iter()
                    .all(|c| c.text)
            {
                let num: String = digits_before
                    .chars()
                    .rev()
                    .chain([c])
                    .chain(digits_after.chars())
                    .collect();
                // "Version 1.0", "iOS 17.2": not a decimal number.
                let mut word: Vec<char> = v[..k - digits_before.len()]
                    .iter()
                    .rev()
                    .skip_while(|c| c.c == ' ')
                    .take_while(|c| c.c.is_alphabetic())
                    .map(|c| c.c.to_ascii_lowercase())
                    .collect();
                word.reverse();
                let word: String = word.into_iter().collect();
                let version = matches!(word.as_str(), "v" | "ios" | "android")
                    || ["versi", "versão", "バージョン", "버전"]
                        .iter()
                        .any(|w| word.contains(w));
                if src.contains(&num) && !version {
                    out.push((
                        here.clone(),
                        if comma { "," } else { "." }.into(),
                        Check::Separator,
                        "decimal separator of the source locale",
                    ));
                }
            }
        }
    }
    // Quotes: straight "…" pairs, or English “…” where the locale uses another pair.
    if let Some((open, close)) = quotes(lang, locale) {
        for (o, cl) in [('"', '"'), ('“', '”')] {
            if (o, cl) == (open, close) || (o == '“' && open == '“') {
                continue;
            }
            let idx: Vec<usize> = (0..v.len())
                .filter(|&k| v[k].text && (v[k].c == o || v[k].c == cl))
                .collect();
            if idx.len() < 2 || idx.len() % 2 == 1 {
                continue;
            }
            for (n, &k) in idx.iter().enumerate() {
                let q = v[k].c;
                if o != cl && q != if n % 2 == 0 { o } else { cl } {
                    break;
                }
                let rep = match (n % 2 == 0, lang) {
                    (true, "fr") => format!("{open}{NNBSP}"),
                    (false, "fr") => format!("{NNBSP}{close}"),
                    (true, _) => open.to_string(),
                    (false, _) => close.to_string(),
                };
                out.push((
                    v[k].at..v[k].at + q.len_utf8(),
                    rep,
                    Check::Quotes,
                    "quotation marks of the locale",
                ));
            }
        }
    }
    out
}

pub(crate) fn check(
    locale: &str,
    source: Option<&str>,
    text: &str,
    segs: &[Span],
    _syntax: Syntax,
) -> Vec<(Check, String)> {
    let mut by: BTreeMap<Check, BTreeSet<&'static str>> = BTreeMap::new();
    let mut n: BTreeMap<Check, usize> = BTreeMap::new();
    for (_, _, check, why) in edits(locale, source, text, segs) {
        by.entry(check).or_default().insert(why);
        *n.entry(check).or_default() += 1;
    }
    by.into_iter()
        .map(|(check, why)| {
            let why: Vec<&str> = why.into_iter().collect();
            (check, format!("{}× {}", n[&check], why.join(", ")))
        })
        .collect()
}

pub(crate) fn fix(
    locale: &str,
    source: Option<&str>,
    text: &str,
    syntax: Syntax,
) -> Option<String> {
    let segs = segments(text, syntax);
    let e: Vec<_> = edits(locale, source, text, &segs)
        .into_iter()
        .map(|(r, s, _, _)| (r, s))
        .collect();
    let out = apply(text, e);
    (out != text).then_some(out)
}

// ---------------------------------------------------------------- digits, across a locale

fn system(c: char) -> Option<&'static str> {
    Some(match c {
        '0'..='9' => "Latin 0-9",
        '\u{0660}'..='\u{0669}' => "Arabic-Indic ٠-٩",
        '\u{06F0}'..='\u{06F9}' => "Persian ۰-۹",
        '\u{0966}'..='\u{096F}' => "Devanagari ०-९",
        '\u{09E6}'..='\u{09EF}' => "Bengali ০-৯",
        '\u{0E50}'..='\u{0E59}' => "Thai ๐-๙",
        _ => return None,
    })
}

/// Digit systems of the literal text; Latin digits glued to Latin letters ("4G", "H1B") don't count.
fn systems(text: &str, segs: &[Span]) -> BTreeSet<&'static str> {
    let v = chars(text, segs);
    (0..v.len())
        .filter(|&k| v[k].text)
        .filter_map(|k| {
            let s = system(v[k].c)?;
            let latin_word = |c: Option<&Ch>| c.is_some_and(|c| c.c.is_ascii_alphabetic());
            (s != "Latin 0-9"
                || !(latin_word(k.checked_sub(1).map(|j| &v[j])) || latin_word(v.get(k + 1))))
            .then_some(s)
        })
        .collect()
}

/// Strings whose digits disagree with the rest of the locale (or mix two systems).
pub fn check_digits<'a>(
    locale: &str,
    entries: impl IntoIterator<Item = (&'a String, &'a String)>,
    syntax: Syntax,
) -> Vec<Finding> {
    let per: Vec<(&String, BTreeSet<&str>)> = entries
        .into_iter()
        .map(|(k, t)| (k, systems(t, &segments(t, syntax))))
        .filter(|(_, s)| !s.is_empty())
        .collect();
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, s) in &per {
        for x in s {
            *count.entry(x).or_default() += 1;
        }
    }
    if count.len() < 2 {
        return vec![];
    }
    let (major, m) = count
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(k, n)| (*k, *n))
        .unwrap_or_default();
    per.iter()
        .filter(|(_, s)| s.len() > 1 || !s.contains(major))
        .map(|(key, s)| Finding {
            locale: locale.into(),
            key: (*key).clone(),
            check: Check::Digits,
            severity: Severity::Warning,
            message: if s.len() > 1 {
                format!(
                    "mixes {} in one string",
                    s.iter().copied().collect::<Vec<_>>().join(" and ")
                )
            } else {
                format!(
                    "uses {} while {m} of {} {locale} strings with digits use {major}",
                    s.iter().next().copied().unwrap_or_default(),
                    per.len()
                )
            },
            fixable: false,
        })
        .collect()
}
