//! glot Edge protection: what is never translated nor sent to glot.
//!
//! - Zones: code (`script style pre code kbd samp var svg math`), form values (`textarea select`, the `value`
//!   of text inputs, `contenteditable`), user content (`[data-glot-user]`), explicit opt-outs (`[translate=no]`,
//!   `.notranslate`, `[data-glot-skip]`) and project selectors ([`Selector`]).
//! - Values inside a segment ([`mask`]): numbers, URLs, code identifiers become `{n}` placeholders kept verbatim;
//!   personal data (emails, phones, cards, IBANs, UUIDs, tokens, long numbers, street addresses) is masked too
//!   and makes the segment non-discoverable: it is translated if a reviewer added it, never reported.
//!
//! Mirrored in `packages/edge/src/segment.ts`; shared fixtures in `crates/core/tests/fixtures/edge`.
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Number,
    Url,
    Code,
    /// A personal-looking value the project allowlisted (`support@acme.com`, the company phone).
    Public,
    Email,
    Phone,
    Card,
    Iban,
    Uuid,
    Token,
    LongNumber,
}

impl ValueKind {
    pub fn is_pii(self) -> bool {
        !matches!(self, Self::Number | Self::Url | Self::Code | Self::Public)
    }
}

/// A `{n}` placeholder: the decoded source value, re-emitted as is in every language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub kind: ValueKind,
    pub value: String,
}

/// HTML whitespace (not NBSP: it is meaningful, `20 %`).
pub fn is_html_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Word separators for value detection: HTML whitespace + NBSP + narrow NBSP.
fn is_sep(c: char) -> bool {
    is_html_ws(c) || c == '\u{a0}' || c == '\u{202f}'
}

/// Collapse HTML whitespace runs to one space and trim.
pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut ws = false;
    for c in s.chars() {
        if is_html_ws(c) {
            ws = true;
        } else {
            if ws && !out.is_empty() {
                out.push(' ');
            }
            ws = false;
            out.push(c);
        }
    }
    out
}

const NAMED: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("hellip", "…"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("euro", "€"),
    ("middot", "·"),
    ("bull", "•"),
    ("times", "×"),
];

/// Decode character references: the named ones above and numeric `&#NN;` / `&#xHH;`. Others stay literal.
// ponytail: small named table; the browser knows ~2000 names, add here if a site uses others in text.
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // `&name;` with an ASCII name of at most 10 chars.
        let semi = rest
            .bytes()
            .skip(1)
            .take(11)
            .position(|c| c == b';')
            .map(|j| j + 1)
            .filter(|&j| {
                j > 1
                    && rest[1..j]
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'#')
            });
        let decoded = semi.and_then(|j| {
            let name = &rest[1..j];
            let code = if let Some(h) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
            {
                (!h.is_empty() && h.len() <= 6 && h.bytes().all(|c| c.is_ascii_hexdigit()))
                    .then(|| u32::from_str_radix(h, 16).ok())
                    .flatten()
            } else if let Some(d) = name.strip_prefix('#') {
                (!d.is_empty() && d.len() <= 7 && d.bytes().all(|c| c.is_ascii_digit()))
                    .then(|| d.parse::<u32>().ok())
                    .flatten()
            } else {
                None
            };
            let c = if name.starts_with('#') {
                code.and_then(char::from_u32).map(String::from)
            } else {
                NAMED
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            };
            c.map(|c| (c, j + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push_str(&c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// Has at least one letter (Unicode `Alphabetic`, as JS `\p{Alphabetic}`).
pub fn has_letter(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

fn is_email(w: &str) -> bool {
    let Some((local, domain)) = w.split_once('@') else {
        return false;
    };
    let tld = domain.rsplit('.').next().unwrap_or("");
    !local.is_empty()
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._%+-".contains(&b))
        && domain.contains('.')
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && tld.len() >= 2
        && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_uuid(w: &str) -> bool {
    w.len() == 36
        && w.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn is_iban(w: &str) -> bool {
    let b = w.as_bytes();
    (15..=34).contains(&b.len())
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[4..]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn is_token(w: &str) -> bool {
    let digit = w.bytes().any(|b| b.is_ascii_digit());
    let alpha = w.bytes().any(|b| b.is_ascii_alphabetic());
    let charset = w
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_-.=+/".contains(&b));
    let hex = w.bytes().all(|b| b.is_ascii_hexdigit());
    digit && alpha && charset && (w.len() >= 20 || (w.len() >= 16 && hex))
}

fn is_code(w: &str) -> bool {
    let b = w.as_bytes();
    w.ends_with("()")
        || w.contains("::")
        || (1..b.len().saturating_sub(1)).any(|i| {
            b[i] == b'_' && b[i - 1].is_ascii_alphanumeric() && b[i + 1].is_ascii_alphanumeric()
        })
}

fn is_url(w: &str) -> bool {
    let l = w.to_ascii_lowercase();
    l.starts_with("http://")
        || l.starts_with("https://")
        || l.starts_with("www.")
        || l.contains("://")
}

fn word_kind(w: &str) -> Option<ValueKind> {
    if is_url(w) {
        Some(ValueKind::Url)
    } else if is_email(w) {
        Some(ValueKind::Email)
    } else if is_uuid(w) {
        Some(ValueKind::Uuid)
    } else if is_iban(w) {
        Some(ValueKind::Iban)
    } else if is_token(w) {
        Some(ValueKind::Token)
    } else if is_code(w) {
        Some(ValueKind::Code)
    } else {
        None
    }
}

fn luhn(digits: &[u8]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            let d = u32::from(d - b'0');
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 { x - 9 } else { x }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

const NUM_SEPS: &[char] = &[' ', '.', '-', '/', ',', ':', '\u{a0}', '\u{202f}'];

/// `1,000,000` / `1 000 000`: a formatted quantity, not an identifier.
fn grouped_by_thousands(run: &str) -> bool {
    let groups: Vec<&str> = run.split(NUM_SEPS).collect();
    groups.len() > 1
        && (1..=3).contains(&groups[0].len())
        && groups[1..].iter().all(|g| g.len() == 3)
}

fn number_kind(run: &str) -> ValueKind {
    let digits: Vec<u8> = run.bytes().filter(u8::is_ascii_digit).collect();
    let n = digits.len();
    if (13..=19).contains(&n) && luhn(&digits) {
        ValueKind::Card
    } else if n >= 9 && !grouped_by_thousands(run.trim_start_matches('+')) {
        if run.starts_with('+') || run.starts_with('0') {
            ValueKind::Phone
        } else {
            ValueKind::LongNumber
        }
    } else {
        ValueKind::Number
    }
}

const STREET: &[&str] = &[
    "rue",
    "avenue",
    "av.",
    "boulevard",
    "bd",
    "chemin",
    "allée",
    "impasse",
    "quai",
    "street",
    "road",
    "lane",
    "drive",
    "calle",
    "avenida",
    "via",
    "strasse",
    "straße",
    "straat",
    "rua",
];

/// `12 rue de la Paix`, `221B Baker Street`: a house number at most 3 words before a street word.
pub fn looks_like_address(text: &str) -> bool {
    let words: Vec<String> = text
        .split(is_sep)
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches(',').to_lowercase())
        .collect();
    let house = |w: &str| {
        let num = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        !num.is_empty()
            && num.len() <= 4
            && w.len() - num.len() <= 3
            && num.bytes().all(|b| b.is_ascii_digit())
    };
    (0..words.len()).any(|j| {
        STREET.contains(&words[j].as_str())
            && words[j.saturating_sub(3)..j].iter().any(|w| house(w))
    })
}

const LEAD_PUNCT: &[char] = &['(', '[', '{', '«', '"', '\'', '“', '‘'];
const TRAIL_PUNCT: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '»', '"', '\'', '”', '’',
];

/// Replace values in `text` with `{n}` (n continues from `values.len()`), pushing them to `values`.
/// Allowlisted values (case-insensitive) are kept as [`ValueKind::Public`].
pub fn mask(text: &str, values: &mut Vec<Value>, allow: &[String]) -> String {
    // (start, end, kind) byte ranges, non-overlapping, in order.
    let mut hits: Vec<(usize, usize, ValueKind)> = Vec::new();
    // 1. Words.
    let mut pos = 0;
    while pos < text.len() {
        let rest = &text[pos..];
        let Some(ws) = rest.find(|c: char| !is_sep(c)) else {
            break;
        };
        let start = pos + ws;
        let end = text[start..].find(is_sep).map_or(text.len(), |e| start + e);
        let word = &text[start..end];
        let core_start = start + (word.len() - word.trim_start_matches(LEAD_PUNCT).len());
        let mut core_end = start + word.trim_end_matches(TRAIL_PUNCT).len();
        if core_start < core_end
            && text[core_start..core_end].contains('(')
            && text[core_end..end].starts_with(')')
        {
            core_end += 1; // `init()` keeps its `)`
        }
        if core_start < core_end {
            // URLs keep a trailing `/` etc.; trailing punctuation never belongs to a value.
            if let Some(k) = word_kind(&text[core_start..core_end]) {
                hits.push((core_start, core_end, k));
            }
        }
        pos = end;
    }
    // 2. Digit runs outside word hits: `+33 6 12 34 56 78`, `4242 4242 4242 4242`, `9.99`, `2026-11-07`.
    let b = text.as_bytes();
    let mut runs = Vec::new();
    let mut p = 0;
    while p < b.len() {
        if !b[p].is_ascii_digit() {
            p += 1;
            continue;
        }
        let mut start = p;
        if start > 0 && b[start - 1] == b'+' && (start == 1 || text[..start - 1].ends_with(is_sep))
        {
            start -= 1;
        }
        let mut end = p + 1;
        loop {
            if end < b.len() && b[end].is_ascii_digit() {
                end += 1;
                continue;
            }
            let next = text[end..].chars().next();
            if let Some(c) = next.filter(|c| NUM_SEPS.contains(c)) {
                let after = end + c.len_utf8();
                if after < b.len() && b[after].is_ascii_digit() {
                    end = after + 1;
                    continue;
                }
            }
            break;
        }
        let before_ok = !text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after_ok = !text[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_');
        if before_ok && after_ok && !hits.iter().any(|h| h.0 < end && start < h.1) {
            runs.push((start, end, number_kind(&text[start..end])));
        }
        p = end;
    }
    hits.extend(runs);
    hits.sort_by_key(|h| h.0);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (s, e, mut k) in hits {
        let v = &text[s..e];
        if k.is_pii() && allow.iter().any(|a| a.eq_ignore_ascii_case(v)) {
            k = ValueKind::Public;
        }
        out.push_str(&text[last..s]);
        out.push_str(&format!("{{{}}}", values.len()));
        values.push(Value {
            kind: k,
            value: v.to_string(),
        });
        last = e;
    }
    out.push_str(&text[last..]);
    out
}

/// Compound CSS selector (`tag`, `.class`, `#id`, `[attr]`, `[attr=value]`, combined: `div.price[data-x]`).
/// No combinators: a project excludes `.user-name`, `#account`, `[data-private]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
}

impl Selector {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() || s.contains([' ', '>', '+', '~', ',']) {
            return None;
        }
        let mut sel = Selector::default();
        let mut rest = s;
        let ident_end = |r: &str| r.find(['.', '#', '[']).unwrap_or(r.len());
        let n = ident_end(rest);
        if n > 0 {
            let t = &rest[..n];
            if t != "*" {
                sel.tag = Some(t.to_ascii_lowercase());
            }
            rest = &rest[n..];
        }
        while let Some(c) = rest.chars().next() {
            rest = &rest[1..];
            match c {
                '.' | '#' => {
                    let n = ident_end(rest);
                    if n == 0 {
                        return None;
                    }
                    let v = rest[..n].to_string();
                    if c == '.' {
                        sel.classes.push(v);
                    } else {
                        sel.id = Some(v);
                    }
                    rest = &rest[n..];
                }
                '[' => {
                    let close = rest.find(']')?;
                    let inner = &rest[..close];
                    rest = &rest[close + 1..];
                    match inner.split_once('=') {
                        Some((k, v)) => sel.attrs.push((
                            k.trim().to_ascii_lowercase(),
                            Some(v.trim().trim_matches(['"', '\'']).to_string()),
                        )),
                        None => sel.attrs.push((inner.trim().to_ascii_lowercase(), None)),
                    }
                }
                _ => return None,
            }
        }
        Some(sel)
    }

    /// `attrs`: lowercase names, decoded values.
    pub fn matches(&self, tag: &str, attrs: &[(String, String)]) -> bool {
        let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        self.tag.as_deref().is_none_or(|t| t == tag)
            && self.id.as_deref().is_none_or(|id| get("id") == Some(id))
            && self
                .classes
                .iter()
                .all(|c| get("class").is_some_and(|cl| cl.split_ascii_whitespace().any(|x| x == c)))
            && self.attrs.iter().all(|(k, v)| match (get(k), v) {
                (Some(a), Some(v)) => a == v,
                (Some(_), None) => true,
                (None, _) => false,
            })
    }
}

/// Why an element's subtree is not segmented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Code, scripts, styles, SVG/MathML: nothing inside nor on it.
    Code,
    /// `textarea`/`select` content is a value; the element's own `placeholder`/`title` are still translated.
    Form,
    /// User content, `translate=no`, `.notranslate`, `[data-glot-skip]`, `contenteditable`, project selectors.
    Opaque,
}

const CODE_TAGS: &[&str] = &[
    "script",
    "style",
    "pre",
    "code",
    "kbd",
    "samp",
    "var",
    "svg",
    "math",
    "xmp",
    "plaintext",
];

pub fn skip(tag: &str, attrs: &[(String, String)], selectors: &[Selector]) -> Option<Skip> {
    let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    if get("translate").is_some_and(|v| v.eq_ignore_ascii_case("no"))
        || get("class").is_some_and(|c| c.split_ascii_whitespace().any(|x| x == "notranslate"))
        || get("data-glot-skip").is_some()
        || get("data-glot-user").is_some()
        || get("contenteditable").is_some_and(|v| !v.eq_ignore_ascii_case("false"))
        || selectors.iter().any(|s| s.matches(tag, attrs))
    {
        Some(Skip::Opaque)
    } else if CODE_TAGS.contains(&tag) {
        Some(Skip::Code)
    } else if matches!(tag, "textarea" | "select") {
        Some(Skip::Form)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> (String, Vec<ValueKind>) {
        let mut v = Vec::new();
        let out = mask(s, &mut v, &["support@acme.com".into()]);
        (out, v.into_iter().map(|x| x.kind).collect())
    }

    #[test]
    fn masks_values_and_pii() {
        use ValueKind::*;
        assert_eq!(m("Save 20 %"), ("Save {0} %".into(), vec![Number]));
        assert_eq!(m("€9.99/mo"), ("€{0}/mo".into(), vec![Number]));
        assert_eq!(
            m("Write to jean.d@gmail.com."),
            ("Write to {0}.".into(), vec![Email])
        );
        assert_eq!(
            m("Mail support@acme.com"),
            ("Mail {0}".into(), vec![Public])
        );
        assert_eq!(
            m("Call +33 6 12 34 56 78 now"),
            ("Call {0} now".into(), vec![Phone])
        );
        assert_eq!(
            m("Card 4242 4242 4242 4242"),
            ("Card {0}".into(), vec![Card])
        );
        assert_eq!(m("1,000,000 users"), ("{0} users".into(), vec![Number]));
        assert_eq!(m("Order 123456789"), ("Order {0}".into(), vec![LongNumber]));
        assert_eq!(
            m("IBAN FR7630006000011234567890189"),
            ("IBAN {0}".into(), vec![Iban])
        );
        assert_eq!(
            m("id 123e4567-e89b-12d3-a456-426614174000"),
            ("id {0}".into(), vec![Uuid])
        );
        assert_eq!(
            m("key sk_live_51Habc123def456ghi"),
            ("key {0}".into(), vec![Token])
        );
        assert_eq!(
            m("call init_app() first"),
            ("call {0} first".into(), vec![Code])
        );
        assert_eq!(m("See https://x.io/a."), ("See {0}.".into(), vec![Url]));
        assert_eq!(
            m("4K video, H264, mp4"),
            ("4K video, H264, mp4".into(), vec![])
        );
        assert_eq!(
            m("\"' (.) at 10:30"),
            ("\"' (.) at {0}".into(), vec![Number])
        );
        assert!(looks_like_address("12 rue de la Paix, Paris"));
        assert!(looks_like_address("221B Baker Street"));
        assert!(!looks_like_address("Save 20 % on 3 plans"));
    }

    #[test]
    fn selectors_and_entities() {
        let a = |v: &[(&str, &str)]| {
            v.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        };
        let s = Selector::parse("div.user[data-x=1]").unwrap();
        assert!(s.matches("div", &a(&[("class", "a user"), ("data-x", "1")])));
        assert!(!s.matches("div", &a(&[("class", "user")])));
        assert!(
            Selector::parse("#account")
                .unwrap()
                .matches("section", &a(&[("id", "account")]))
        );
        assert!(Selector::parse("a b").is_none());
        assert_eq!(
            decode_entities("a &amp; b &#233;&#x41;&nbsp;&bogus; &"),
            "a & b éA\u{a0}&bogus; &"
        );
    }
}
