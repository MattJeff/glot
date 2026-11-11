//! Helpers shared by the headless CMS connectors (Contentful, Sanity, Strapi).
//!
//! - Markdown fields (Contentful long text, Strapi "Rich text (Markdown)") become a small rich text document
//!   ([`md_doc`]): one text node per line or table cell, with the block syntax (`## `, `- `, `> `, `1. `,
//!   fences, code, separators) kept outside the text, so a translation can never break the structure.
//! - Code inside rich text (code marks, Strapi code blocks) is hidden from the translator ([`protect_code`])
//!   and restored on push ([`restore`]).
use serde_json::{Value, json};

use super::Creds;

/// A connector parameter: from the credentials form, else from the settings.
pub fn param<'a>(creds: &'a Creds, name: &str) -> Option<&'a str> {
    creds.opt(name).or_else(|| {
        creds
            .setting(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
}

pub fn required<'a>(creds: &'a Creds, name: &str) -> anyhow::Result<&'a str> {
    param(creds, name).ok_or_else(|| anyhow::anyhow!("missing '{name}'"))
}

pub fn has_letter(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

/// Whether a string looks like human text (document mode, no explicit `fields`).
pub fn looks_like_text(s: &str) -> bool {
    has_letter(s)
        && !s.contains("://")
        && !s.starts_with('/')
        && !s.starts_with('#')
        && (s.contains(char::is_whitespace) || s.chars().next().is_some_and(char::is_uppercase))
}

const MD: &str = "glot_markdown";
const CODE: &str = "glot:code";

fn text(s: &str) -> Value {
    json!({"type": "text", "text": s})
}
fn raw(s: &str) -> Value {
    json!({"raw": s})
}

/// Length of the block syntax at the start of a Markdown line (`  - [ ] `, `> ## `, `12. `).
fn md_prefix(line: &str) -> usize {
    let b = line.as_bytes();
    let mut i = 0;
    loop {
        let start = i;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let j = i;
        if b.get(i) == Some(&b'>') {
            i += 1;
            if b.get(i) == Some(&b' ') {
                i += 1;
            }
        } else if b.get(i) == Some(&b'#') {
            while b.get(i) == Some(&b'#') {
                i += 1;
            }
            if i - j > 6 || b.get(i) != Some(&b' ') {
                return start;
            }
            i += 1;
        } else if matches!(b.get(i), Some(b'-' | b'*' | b'+')) && b.get(i + 1) == Some(&b' ') {
            i += 2;
            if line[i..].starts_with("[ ] ")
                || line[i..].starts_with("[x] ")
                || line[i..].starts_with("[X] ")
            {
                i += 4;
            }
        } else if b.get(i).is_some_and(u8::is_ascii_digit) {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if !matches!(b.get(i), Some(b'.' | b')')) || b.get(i + 1) != Some(&b' ') {
                return start;
            }
            i += 2;
        } else {
            return j;
        }
    }
}

/// Markdown → rich text document (see the module doc). `md_text(&md_doc(s)) == s` for any `s`.
pub fn md_doc(src: &str) -> Value {
    let mut lines = vec![];
    let mut fence: Option<&str> = None;
    for line in src.split('\n') {
        let t = line.trim_start();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            lines.push(json!([raw(line)]));
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            lines.push(json!([raw(line)]));
            continue;
        }
        if line.starts_with("    ") || line.starts_with('\t') || !has_letter(line) {
            // Indented code, blank lines, separators (`---`, `|---|`).
            lines.push(json!([raw(line)]));
            continue;
        }
        if t.starts_with('|') {
            let mut parts = vec![];
            for (i, cell) in line.split('|').enumerate() {
                if i > 0 {
                    parts.push(raw("|"));
                }
                parts.push(if has_letter(cell) {
                    text(cell)
                } else {
                    raw(cell)
                });
            }
            lines.push(Value::Array(parts));
            continue;
        }
        let p = md_prefix(line);
        let mut parts = vec![];
        if p > 0 {
            parts.push(raw(&line[..p]));
        }
        parts.push(text(&line[p..]));
        lines.push(Value::Array(parts));
    }
    json!({ MD: lines })
}

pub fn is_md(doc: &Value) -> bool {
    doc.get(MD).is_some()
}

/// Rich text document from [`md_doc`] (possibly translated) → Markdown.
pub fn md_text(doc: &Value) -> String {
    let lines = doc
        .get(MD)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    lines
        .iter()
        .map(|parts| {
            parts
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    p.get("raw")
                        .or_else(|| p.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_code_mark(o: &serde_json::Map<String, Value>) -> bool {
    o.get("code") == Some(&Value::Bool(true))
        || o.get("marks").and_then(Value::as_array).is_some_and(|m| {
            m.iter().any(|x| {
                x.as_str() == Some("code") || x.get("type").and_then(Value::as_str) == Some("code")
            })
        })
}

/// Renames the text nodes holding code (code marks, Strapi `code` blocks) so they are not translated.
pub fn protect_code(v: &mut Value) {
    fn walk(v: &mut Value, in_code: bool) {
        match v {
            Value::Object(o) => {
                let code = in_code || o.get("type").and_then(Value::as_str) == Some("code");
                let mark = code || has_code_mark(o);
                for f in ["nodeType", "_type", "type"] {
                    let t = o.get(f).and_then(Value::as_str);
                    if mark && (t == Some("text") || (f == "_type" && t == Some("span"))) {
                        o.insert(f.into(), CODE.into());
                    }
                }
                o.values_mut().for_each(|c| walk(c, code));
            }
            Value::Array(a) => a.iter_mut().for_each(|c| walk(c, in_code)),
            _ => {}
        }
    }
    walk(v, false)
}

/// Undoes [`protect_code`].
pub fn restore(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (f, orig) in [("nodeType", "text"), ("_type", "span"), ("type", "text")] {
                if o.get(f).and_then(Value::as_str) == Some(CODE) {
                    o.insert(f.into(), orig.into());
                }
            }
            o.values_mut().for_each(restore);
        }
        Value::Array(a) => a.iter_mut().for_each(restore),
        _ => {}
    }
}

/// A pushed rich text value back in the CMS's shape: Markdown string, or the document with code restored.
pub fn pushed_value(doc: &Value) -> Value {
    if is_md(doc) {
        return Value::String(md_text(doc));
    }
    let mut d = doc.clone();
    restore(&mut d);
    d
}

/// Header value as text.
pub fn header<'a>(h: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::trim)
}

/// Shared secret sent by the CMS in `X-Glot-Webhook-Secret` or `Authorization: [Bearer] <secret>`.
pub fn shared_secret_ok(h: &axum::http::HeaderMap, secret: &str) -> bool {
    !secret.is_empty()
        && [
            header(h, "x-glot-webhook-secret"),
            header(h, "authorization").map(|a| a.strip_prefix("Bearer ").unwrap_or(a).trim()),
        ]
        .into_iter()
        .flatten()
        .any(|v| super::secret_eq(v, secret))
}

/// A locale → value map: the `name` setting as a JSON object, or the form field `"fr: x, de: y"`.
pub fn pairs(c: &Creds, name: &str) -> std::collections::BTreeMap<String, String> {
    if let Some(m) = c.setting(name).and_then(Value::as_object) {
        return m
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), id_str(v)?)))
            .collect();
    }
    param(c, name)
        .unwrap_or("")
        .split([',', '\n'])
        .filter_map(|p| {
            let (k, v) = p.split_once([':', '='])?;
            let (k, v) = (k.trim(), v.trim());
            (!k.is_empty() && !v.is_empty()).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// An id given as a JSON string or number.
pub fn id_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Percent-encodes a query value (everything but unreserved characters).
pub fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The CMS locale matching a glot `locale`: same code (any case, `_` = `-`), else same language when one
/// side has no region (`fr` ↔ `fr-FR`).
pub fn match_locale<'a>(available: &'a [String], locale: &str) -> Option<&'a str> {
    let norm = |s: &str| s.replace('_', "-").to_ascii_lowercase();
    let base = |s: &str| norm(s).split('-').next().unwrap_or("").to_string();
    let want = norm(locale);
    available
        .iter()
        .find(|a| norm(a) == want)
        .or_else(|| {
            available.iter().find(|a| {
                base(a) == base(locale) && (!norm(a).contains('-') || !want.contains('-'))
            })
        })
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::{Format, rich};

    #[test]
    fn locale_matching() {
        let a: Vec<String> = ["en-us", "fr", "pt_BR"].map(String::from).into();
        assert_eq!(match_locale(&a, "fr-FR"), Some("fr"));
        assert_eq!(match_locale(&a, "pt-BR"), Some("pt_BR"));
        assert_eq!(match_locale(&a, "en"), Some("en-us"));
        assert_eq!(match_locale(&a, "pt-PT"), None);
        assert_eq!(match_locale(&a, "de"), None);
    }
    use std::collections::HashMap;

    const MDS: &str = "# Hello world\n\nSome **bold** text and a [link](https://x.io).\n\n- [ ] Task one\n  1. Nested item\n> ## Quoted title\n\n```js\nconst greeting = 'Hello there';\n```\n\n    indented code line\n| Name | Price |\n|------|------:|\n| Apple | 3 |\n---\n";

    #[test]
    fn markdown_roundtrip_and_units() {
        let d = md_doc(MDS);
        assert_eq!(md_text(&d), MDS);
        let u = rich::units("k", &Format::RichText(d.clone()), "", "en");
        let texts: Vec<_> = u.iter().map(|x| x.1.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Hello world",
                "Some **bold** text and a [link](https://x.io).",
                "Task one",
                "Nested item",
                "Quoted title",
                "Name",
                "Price",
                "Apple"
            ]
        );
        let fr: HashMap<String, String> = u
            .iter()
            .map(|(k, t)| (k.clone(), format!("FR {t}")))
            .collect();
        let out =
            rich::rebuild("k", &Format::RichText(d), "", "en", &|k| fr.get(k).cloned()).unwrap();
        let md = md_text(&serde_json::from_str(&out).unwrap());
        assert!(
            md.starts_with("# FR Hello world\n\nFR Some **bold**"),
            "{md}"
        );
        assert!(
            md.contains("- [ ] FR Task one\n  1. FR Nested item\n> ## FR Quoted title"),
            "{md}"
        );
        assert!(
            md.contains("const greeting = 'Hello there';"),
            "code untouched: {md}"
        );
        assert!(md.contains("    indented code line"), "{md}");
        assert!(
            md.contains("| FR Name | FR Price |\n|------|------:|\n| FR Apple | 3 |"),
            "{md}"
        );
    }

    #[test]
    fn code_is_protected() {
        let mut doc = json!([
            {"_type": "block", "children": [{"_type": "span", "text": "Run ", "marks": []}, {"_type": "span", "text": "npm install", "marks": ["code"]}]},
            {"type": "code", "children": [{"type": "text", "text": "let x = hello;"}]},
            {"type": "paragraph", "children": [{"type": "text", "text": "inline", "code": true}]},
            {"nodeType": "text", "value": "foo()", "marks": [{"type": "code"}]}
        ]);
        let orig = doc.clone();
        protect_code(&mut doc);
        let u = rich::units("k", &Format::RichText(doc.clone()), "", "en");
        assert_eq!(u.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["Run"]);
        assert_eq!(pushed_value(&doc), orig);
    }

    #[test]
    fn shared_secret() {
        let mut h = axum::http::HeaderMap::new();
        assert!(!shared_secret_ok(&h, "s"));
        h.insert("authorization", "Bearer s3".parse().unwrap());
        assert!(
            shared_secret_ok(&h, "s3") && !shared_secret_ok(&h, "s4") && !shared_secret_ok(&h, "")
        );
    }
}
