//! Bidi bugs in RTL strings, found by running the Unicode Bidi Algorithm (`unicode-bidi`) on a
//! sample of the message (`%@`/`{name}` = an LTR value, `%d` = a number) and comparing it with the
//! same message where each text placeholder is isolated (FSI…PDI, what `<bdi>` does). A difference
//! is a real reordering a reader sees, not a style rule.
use std::ops::Range;

use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};

use super::{Check, Seg, Span, segments};
use crate::validate::{Severity, Syntax};

pub const FSI: char = '\u{2068}';
pub const PDI: char = '\u{2069}';
pub const RLM: char = '\u{200F}';
const LTR_SAMPLE: &str = "Abc";
const NUM_SAMPLE: &str = "12";

fn is_isolate_open(c: char) -> bool {
    matches!(c, '\u{2066}' | '\u{2067}' | FSI)
}

fn bdi_tag(markup: &str) -> Option<bool> {
    let t = markup.trim_start_matches('<').trim_end_matches('>').trim();
    match t.split_whitespace().next()? {
        "bdi" => Some(true),
        "/bdi" => Some(false),
        _ => None,
    }
}

/// Already wrapped in FSI/LRI/RLI…PDI.
fn isolated(text: &str, r: &Range<usize>) -> bool {
    text[..r.start]
        .chars()
        .next_back()
        .is_some_and(is_isolate_open)
        && text[r.end..].starts_with(PDI)
}

/// Rendered sample of each plural/select branch (or the whole flat message).
fn samples(text: &str, segs: &[Span], isolate: bool) -> Vec<String> {
    let mut out = vec![String::new()];
    for s in segs {
        let cur = out.last_mut().expect("non-empty");
        match s.seg {
            Seg::Text => cur.push_str(&text[s.range.clone()]),
            Seg::Num => cur.push_str(NUM_SAMPLE),
            Seg::Str if isolate && !isolated(text, &s.range) => {
                cur.push(FSI);
                cur.push_str(LTR_SAMPLE);
                cur.push(PDI);
            }
            Seg::Str => cur.push_str(LTR_SAMPLE),
            Seg::Markup => match bdi_tag(&text[s.range.clone()]) {
                Some(true) => cur.push(FSI),
                Some(false) => cur.push(PDI),
                None => {}
            },
            Seg::Break => out.push(String::new()),
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

fn has_rtl(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL))
}

/// Visual order in an RTL paragraph (`dir="rtl"`), invisible controls removed.
fn visual(s: &str) -> String {
    let info = BidiInfo::new(s, Some(Level::rtl()));
    info.paragraphs
        .iter()
        .map(|p| info.reorder_line(p, p.range.clone()).into_owned())
        .collect::<String>()
        .chars()
        .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .collect()
}

/// What decides the base direction under first-strong rules (iOS/Android natural direction,
/// `dir="auto"`): `Some(Ok(placeholder))` or `Some(Err(latin word))` when it is LTR.
fn first_strong_ltr<'a>(text: &'a str, segs: &[Span]) -> Option<Result<&'a str, String>> {
    let mut depth = 0i32;
    for s in segs {
        let raw = &text[s.range.clone()];
        match s.seg {
            Seg::Break => return None,
            Seg::Num => {}
            Seg::Markup => match bdi_tag(raw) {
                Some(true) => depth += 1,
                Some(false) => depth -= 1,
                None => {}
            },
            Seg::Str if depth > 0 || isolated(text, &s.range) => {}
            Seg::Str => return Some(Ok(raw)),
            Seg::Text => {
                for (i, c) in raw.char_indices() {
                    match c {
                        _ if is_isolate_open(c) => depth += 1,
                        PDI => depth -= 1,
                        _ if depth > 0 => {}
                        _ => match bidi_class(c) {
                            BidiClass::R | BidiClass::AL => return None,
                            BidiClass::L => {
                                let word: String = raw[i..]
                                    .chars()
                                    .take_while(|c| {
                                        !c.is_whitespace()
                                            && !matches!(
                                                bidi_class(*c),
                                                BidiClass::R | BidiClass::AL
                                            )
                                    })
                                    .take(24)
                                    .collect();
                                return Some(Err(word));
                            }
                            _ => {}
                        },
                    }
                }
            }
        }
    }
    None
}

/// `(`/`[` typed in visual order: a closer before any opener. `Some(true)` when swapping every
/// bracket of the literal text balances it.
fn brackets(text: &str, segs: &[Span]) -> Option<bool> {
    let walk = |swap: bool| {
        let mut depth = 0i32;
        let mut reversed = false;
        for c in super::text_chars(text, segs) {
            let c = if swap { mirror(c) } else { c };
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => {
                    depth -= 1;
                    reversed |= depth < 0;
                }
                _ => {}
            }
        }
        (depth, reversed)
    };
    match walk(false) {
        (0, false) => None,
        (_, reversed) => Some(reversed && walk(true) == (0, false)),
    }
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        c => c,
    }
}

pub(crate) fn check(
    text: &str,
    segs: &[Span],
    syntax: Syntax,
) -> Vec<(Check, Severity, String, bool)> {
    let mut out = vec![];
    let raw = samples(text, segs, false);
    if !raw.iter().any(|s| has_rtl(s)) {
        return out; // untranslated / symbols only: `glot check` reports it
    }
    let iso = samples(text, segs, true);
    let flat = !segs.iter().any(|s| s.seg == Seg::Break);
    let spill: Vec<&str> = segs
        .iter()
        .filter(|s| s.seg == Seg::Str && !isolated(text, &s.range))
        .map(|s| &text[s.range.clone()])
        .collect();
    if raw
        .iter()
        .zip(&iso)
        .any(|(r, i)| has_rtl(r) && visual(r) != visual(i))
    {
        out.push((
            Check::Bidi,
            Severity::Error,
            format!(
                "{} not isolated: with an LTR value (Latin name, URL, email, file name) it merges with the neighbouring number/punctuation and the line reorders; wrap it in FSI/PDI (U+2068/U+2069) or <bdi>",
                spill.join(", ")
            ),
            true,
        ));
    }
    if let Some(first) = first_strong_ltr(text, segs) {
        let (what, fixable, how) = match first {
            Ok(p) => (
                format!("the placeholder {p}"),
                true,
                "isolate it with FSI/PDI",
            ),
            Err(w) => (
                format!("the Latin word \"{w}\""),
                flat,
                "start with U+200F RIGHT-TO-LEFT MARK",
            ),
        };
        out.push((
            Check::Bidi,
            // Native UI takes the base direction from the first strong character; the web only with dir="auto".
            if syntax == Syntax::Printf {
                Severity::Error
            } else {
                Severity::Warning
            },
            format!(
                "starts with {what}: where the base direction comes from the first strong character (iOS/Android natural direction, dir=\"auto\") the whole line is laid out LTR; {how}"
            ),
            fixable,
        ));
    }
    if let Some(swappable) = brackets(text, segs) {
        out.push(if swappable {
            (
                Check::Brackets,
                Severity::Error,
                "brackets typed in visual order (\")…(\"): in logical order RTL text opens with \"(\", the renderer mirrors it".into(),
                true,
            )
        } else {
            (
                Check::Brackets,
                Severity::Warning,
                "unbalanced brackets: the bidi algorithm cannot pair them, the lone one mirrors unpredictably".into(),
                false,
            )
        });
    }
    out
}

/// Isolate text placeholders, swap reversed brackets, prepend RLM when a Latin word starts the line.
pub(crate) fn fix(text: &str, syntax: Syntax) -> Option<String> {
    let segs = segments(text, syntax);
    let found = check(text, &segs, syntax);
    let fixable: Vec<_> = found.iter().filter(|f| f.3).collect();
    if fixable.is_empty() {
        return None;
    }
    let mut edits: Vec<(Range<usize>, String)> = vec![];
    if fixable.iter().any(|f| f.0 == Check::Bidi) {
        for s in segs.iter().filter(|s| s.seg == Seg::Str) {
            if !isolated(text, &s.range) {
                edits.push((
                    s.range.clone(),
                    format!("{FSI}{}{PDI}", &text[s.range.clone()]),
                ));
            }
        }
    }
    if fixable.iter().any(|f| f.0 == Check::Brackets) {
        for s in segs.iter().filter(|s| s.seg == Seg::Text) {
            let t = &text[s.range.clone()];
            if t.contains(['(', ')', '[', ']']) {
                edits.push((s.range.clone(), t.chars().map(mirror).collect()));
            }
        }
    }
    let mut out = super::typo::apply(text, edits);
    let segs2 = segments(&out, syntax);
    if matches!(first_strong_ltr(&out, &segs2), Some(Err(_)))
        && !segs2.iter().any(|s| s.seg == Seg::Break)
    {
        out.insert(0, RLM);
    }
    (out != text).then_some(out)
}
