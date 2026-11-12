//! Typed content → translatable units, and back.
//!
//! A document is a set of leaves (one for `text` / `html` / `markdown` / `template`, three for an email, one per
//! text field of a JSON document). A leaf is cut into units: the paragraph of a plain text, the segment of an
//! HTML document (glot Edge's segmenter: inline markup as `<0>…</0>`, numbers / URLs / emails as `{0}`), the
//! segment of a Markdown document (`glot docs`' parser), or a whole ICU message. A unit is what is hashed,
//! remembered and, when unknown, translated: it never contains a variable value, only its placeholder.
use glot_core::edge::{
    protect,
    segment::{SegmentOpts, segment, translate_html},
    tm::text_hash,
};
use glot_core::{markdown, validate::Syntax};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Html,
    Markdown,
    Template,
}

impl Kind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => Self::Text,
            "html" => Self::Html,
            "markdown" => Self::Markdown,
            "template" => Self::Template,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub hash: String,
    /// Masked text: what is sent for translation and stored.
    pub text: String,
    pub syntax: Syntax,
    pub role: &'static str,
}

#[derive(Debug)]
pub struct Leaf {
    /// JSON pointer of this string in the request's `content` (`""` = the content itself).
    pub pointer: String,
    pub kind: Kind,
    pub source: String,
    pub units: Vec<Unit>,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub leaves: Vec<Leaf>,
    pub warnings: Vec<String>,
}

pub const TYPES: &str = "text, html, markdown, template, email or json";

/// Cut `content` of type `kind` into leaves and units. Errors are messages for the caller (400).
pub fn plan(
    kind: &str,
    content: &Value,
    fields: Option<&Value>,
    opts: &SegmentOpts,
) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut push = |pointer: String, k: Kind, s: &str| -> Result<(), String> {
        plan.leaves.push(leaf(pointer, k, s, opts)?);
        Ok(())
    };
    match kind {
        "email" => {
            let o = content.as_object().ok_or(
                "email content must be an object: {\"subject\": \"…\", \"html\": \"…\", \"text\": \"…\"}",
            )?;
            for (k, kind) in [
                ("subject", Kind::Text),
                ("html", Kind::Html),
                ("text", Kind::Text),
            ] {
                match o.get(k) {
                    Some(Value::String(s)) => push(format!("/{k}"), kind, s)?,
                    Some(Value::Null) | None => {}
                    Some(_) => return Err(format!("email `{k}` must be a string")),
                }
            }
            if plan.leaves.is_empty() {
                return Err("email content needs at least one of `subject`, `html`, `text`".into());
            }
        }
        "json" => {
            let fields = fields.and_then(Value::as_array).filter(|f| !f.is_empty()).ok_or(
                "json content needs `fields`: the paths of its text fields, e.g. [\"title\", \"items.*.name\", \
                 {\"path\": \"body\", \"type\": \"html\"}]. Everything else is returned untouched.",
            )?;
            for f in fields {
                let (path, k) = match f {
                    Value::String(p) => (p.as_str(), Kind::Text),
                    Value::Object(o) => {
                        let p = o
                            .get("path")
                            .and_then(Value::as_str)
                            .ok_or("a `fields` object needs a `path`")?;
                        let t = o.get("type").and_then(Value::as_str).unwrap_or("text");
                        let k = Kind::parse(t).ok_or_else(|| {
                            format!(
                                "field '{p}': unknown type '{t}' (text, html, markdown or template)"
                            )
                        })?;
                        (p, k)
                    }
                    _ => return Err("`fields` entries are paths or {path, type} objects".into()),
                };
                let mut pointers = vec![];
                expand(
                    content,
                    &path.split('.').collect::<Vec<_>>(),
                    String::new(),
                    &mut pointers,
                );
                if pointers.is_empty() {
                    plan.warnings
                        .push(format!("field '{path}' matched no string in the document"));
                }
                for ptr in pointers {
                    if plan.leaves.iter().any(|l| l.pointer == ptr) {
                        continue;
                    }
                    let s = content.pointer(&ptr).and_then(Value::as_str).unwrap_or("");
                    plan.leaves.push(leaf(ptr, k, s, opts)?);
                }
            }
        }
        k => {
            let kind =
                Kind::parse(k).ok_or_else(|| format!("unknown type '{k}': expected {TYPES}"))?;
            let s = content
                .as_str()
                .ok_or_else(|| format!("content must be a string for type '{k}'"))?;
            push(String::new(), kind, s)?;
        }
    }
    Ok(plan)
}

/// Strings of `v` matched by a dotted path (`*` = every item of an array or object), as JSON pointers.
fn expand(v: &Value, path: &[&str], ptr: String, out: &mut Vec<String>) {
    let Some((head, rest)) = path.split_first() else {
        if v.is_string() {
            out.push(ptr);
        }
        return;
    };
    let esc = |k: &str| k.replace('~', "~0").replace('/', "~1");
    match v {
        Value::Object(o) if *head == "*" => {
            for (k, c) in o {
                expand(c, rest, format!("{ptr}/{}", esc(k)), out);
            }
        }
        Value::Array(a) if *head == "*" => {
            for (i, c) in a.iter().enumerate() {
                expand(c, rest, format!("{ptr}/{i}"), out);
            }
        }
        Value::Object(o) => {
            if let Some(c) = o.get(*head) {
                expand(c, rest, format!("{ptr}/{}", esc(head)), out);
            }
        }
        Value::Array(a) => {
            if let Some(c) = head.parse::<usize>().ok().and_then(|i| a.get(i)) {
                expand(c, rest, format!("{ptr}/{head}"), out);
            }
        }
        _ => {}
    }
}

fn leaf(pointer: String, kind: Kind, source: &str, opts: &SegmentOpts) -> Result<Leaf, String> {
    let src = &opts.source_locale;
    let unit = |text: String, syntax, role| Unit {
        hash: text_hash(src, &text),
        text,
        syntax,
        role,
    };
    let units = match kind {
        Kind::Text => paragraphs(source)
            .into_iter()
            .filter(|(is_unit, _)| *is_unit)
            .map(|(_, p)| unit(mask_text(p).0, Syntax::Plain, "text"))
            .collect(),
        Kind::Html => segment(source, opts)
            .iter()
            .filter_map(|seg| {
                let (m, _) = mask_tokens(&seg.text);
                has_words(&m).then(|| unit(m, Syntax::Plain, role(&seg.ctx.role)))
            })
            .collect(),
        Kind::Markdown => markdown::Doc::parse(source)
            .segments()
            .iter()
            .map(|s| unit(s.text.clone(), Syntax::Icu, "text"))
            .collect(),
        Kind::Template => {
            glot_core::validate::placeholders(source, Syntax::Icu).map_err(|i| {
                format!("the template is not valid ICU MessageFormat: {}", i.message)
            })?;
            if source.chars().any(char::is_alphabetic) {
                vec![unit(source.to_string(), Syntax::Icu, "text")]
            } else {
                vec![]
            }
        }
    };
    Ok(Leaf {
        pointer,
        kind,
        source: source.to_string(),
        units,
    })
}

fn role(r: &str) -> &'static str {
    match r {
        "a" => "link",
        "button" | "input" => "button",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "title" => "title",
        "label" => "label",
        _ => "text",
    }
}

pub struct Assembled {
    pub text: String,
    /// Units without a usable translation (left in the source language).
    pub missing: usize,
    /// The translated document did not keep the source's tags and links: the source is returned.
    pub broken: bool,
}

/// The leaf with every unit found in `tr` (hash → translated masked text) put back in place.
pub fn assemble(
    leaf: &Leaf,
    opts: &SegmentOpts,
    tr: &HashMap<String, String>,
    target: &str,
) -> Assembled {
    let src = &opts.source_locale;
    let mut missing = 0;
    let text = match leaf.kind {
        Kind::Text => paragraphs(&leaf.source)
            .into_iter()
            .map(|(is_unit, p)| {
                if !is_unit {
                    return p.to_string();
                }
                let (m, values, toks) = mask_text(p);
                match tr.get(&text_hash(src, &m)) {
                    Some(t) => unmask_tokens(&unmask_values(t, &values), &toks),
                    None => {
                        missing += 1;
                        p.to_string()
                    }
                }
            })
            .collect(),
        Kind::Html => {
            let out = translate_html(&leaf.source, opts, |seg| {
                let (m, toks) = mask_tokens(&seg.text);
                if !has_words(&m) {
                    return None;
                }
                let t = tr.get(&text_hash(src, &m));
                if t.is_none() {
                    missing += 1;
                }
                t.map(|t| unmask_tokens(t, &toks))
            });
            if tag_signature(&out) != tag_signature(&leaf.source) {
                return Assembled {
                    text: leaf.source.clone(),
                    missing: leaf.units.len(),
                    broken: true,
                };
            }
            out
        }
        Kind::Markdown => {
            let doc = markdown::Doc::parse(&leaf.source);
            let found: Vec<Option<String>> = doc
                .segments()
                .iter()
                .map(|s| tr.get(&text_hash(src, &s.text)).cloned())
                .collect();
            missing = found.iter().filter(|f| f.is_none()).count();
            doc.render(
                &|i| found.get(i).cloned().flatten(),
                Some((src.as_str(), target)),
            )
        }
        Kind::Template => match leaf.units.first() {
            Some(u) => tr.get(&u.hash).cloned().unwrap_or_else(|| {
                missing = 1;
                leaf.source.clone()
            }),
            None => leaf.source.clone(),
        },
    };
    Assembled {
        text,
        missing,
        broken: false,
    }
}

/// Plain text as (is a unit, slice) pieces whose concatenation is the text: blank-line separated paragraphs
/// with letters are units, their surrounding whitespace and everything else is kept as is.
fn paragraphs(s: &str) -> Vec<(bool, &str)> {
    let mut out = vec![];
    let mut block: Option<(usize, usize)> = None;
    fn flush<'a>(s: &'a str, block: &mut Option<(usize, usize)>, out: &mut Vec<(bool, &'a str)>) {
        if let Some((a, b)) = block.take() {
            let p = &s[a..b];
            let core = p.trim();
            if has_words(&mask_text(core).0) {
                let lead = p.len() - p.trim_start().len();
                out.push((false, &p[..lead]));
                out.push((true, core));
                out.push((false, &p[lead + core.len()..]));
            } else {
                out.push((false, p));
            }
        }
    }
    let mut at = 0;
    for line in s.split_inclusive('\n') {
        let end = at + line.len();
        if line.trim().is_empty() {
            flush(s, &mut block, &mut out);
            out.push((false, line));
        } else {
            block = Some((block.map_or(at, |b| b.0), end));
        }
        at = end;
    }
    flush(s, &mut block, &mut out);
    out.retain(|(_, p)| !p.is_empty());
    out
}

/// A paragraph masked for translation: values (numbers, URLs, emails, phones…) as `{0}`, template tokens as `{v0}`.
fn mask_text(p: &str) -> (String, Vec<protect::Value>, Vec<String>) {
    // Tokens first: `first_name` inside `{{first_name}}` would otherwise be masked as a code identifier.
    let (m, toks) = mask_tokens(p);
    let mut values = vec![];
    let m = protect::mask(&m, &mut values, &[]);
    (m, values, toks)
}

/// Template-engine tokens (`{{name}}`, `{{{raw}}}`, `{{#if x}}`, `{% tag %}`) → `{v0}`, `{v1}`…: the translator
/// sees a placeholder, never the variable's name, and the validator checks each one came back.
pub fn mask_tokens(s: &str) -> (String, Vec<String>) {
    let (mut out, mut toks) = (String::new(), Vec::<String>::new());
    let mut rest = s;
    while let Some(i) = [rest.find("{{"), rest.find("{%")]
        .into_iter()
        .flatten()
        .min()
    {
        let (open, close) = if rest[i..].starts_with("{{{") {
            (3, "}}}")
        } else if rest[i..].starts_with("{{") {
            (2, "}}")
        } else {
            (2, "%}")
        };
        let Some(j) = rest[i + open..].find(close) else {
            break; // unterminated: plain text
        };
        let end = i + open + j + close.len();
        let tok = &rest[i..end];
        let n = toks.iter().position(|t| t == tok).unwrap_or_else(|| {
            toks.push(tok.to_string());
            toks.len() - 1
        });
        out.push_str(&rest[..i]);
        out.push_str(&format!("{{v{n}}}"));
        rest = &rest[end..];
    }
    out.push_str(rest);
    (out, toks)
}

fn unmask_tokens(t: &str, toks: &[String]) -> String {
    replace_braced(t, |inner| {
        inner
            .strip_prefix('v')
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| toks.get(n))
            .cloned()
    })
}

fn unmask_values(t: &str, values: &[protect::Value]) -> String {
    replace_braced(t, |inner| {
        inner
            .parse::<usize>()
            .ok()
            .and_then(|n| values.get(n))
            .map(|v| v.value.clone())
    })
}

/// Replace each `{inner}` for which `f(inner)` is `Some`.
fn replace_braced(t: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        match rest[i..]
            .find('}')
            .and_then(|j| Some((j, f(&rest[i + 1..i + j])?)))
        {
            Some((j, v)) => {
                out.push_str(&v);
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push('{');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Letters left once placeholders and tags are removed: otherwise there is nothing to translate.
fn has_words(masked: &str) -> bool {
    let mut depth = 0i32;
    masked.chars().any(|c| match c {
        '{' | '<' => {
            depth += 1;
            false
        }
        '}' | '>' => {
            depth = (depth - 1).max(0);
            false
        }
        c => depth == 0 && c.is_alphabetic(),
    })
}

/// Our placeholders in a masked text, in order: `{0}` `{v0}` `<0>` `</0>` `<0/>`.
fn placeholders(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let close = match b[i] {
            b'{' => b'}',
            b'<' => b'>',
            _ => {
                i += 1;
                continue;
            }
        };
        let Some(j) = s[i..].find(close as char) else {
            break;
        };
        let inner = &s[i + 1..i + j];
        let core = inner
            .trim_start_matches('/')
            .trim_end_matches('/')
            .trim_start_matches('v');
        if !core.is_empty() && core.bytes().all(|c| c.is_ascii_digit()) && j <= 8 {
            out.push(s[i..=i + j].to_string());
            i += j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Markup integrity of one unit: the translation carries exactly the source's placeholders (same count),
/// and its tags are well nested.
pub fn placeholders_kept(source: &str, target: &str) -> Result<(), String> {
    let (mut a, b) = (placeholders(source), placeholders(target));
    let mut stack: Vec<&str> = vec![];
    for p in &b {
        if let Some(n) = p.strip_prefix("</") {
            if stack.pop() != Some(n) {
                return Err(format!("tag {p} is closed out of order"));
            }
        } else if p.starts_with('<') && !p.ends_with("/>") {
            stack.push(&p[1..]);
        }
    }
    if let Some(n) = stack.pop() {
        return Err(format!("tag <{n} is never closed"));
    }
    let mut b = b;
    a.sort();
    b.sort();
    if a != b {
        let lost = a.iter().find(|p| !b.contains(p));
        let extra = b.iter().find(|p| !a.contains(p));
        return Err(match (lost, extra) {
            (Some(p), _) => format!("placeholder {p} was lost"),
            (_, Some(p)) => format!("placeholder {p} was invented"),
            _ => "a placeholder is repeated or missing".into(),
        });
    }
    Ok(())
}

/// What a translated HTML document must share with its source: the same tags (names, opening and closing)
/// and the same link and image targets, in any order.
fn tag_signature(html: &str) -> Vec<String> {
    let b = html.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while let Some(rel) = html[i..].find('<') {
        let s = i + rel;
        if html[s..].starts_with("<!--") {
            i = html[s..].find("-->").map_or(html.len(), |e| s + e + 3);
            continue;
        }
        if !b
            .get(s + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'/' || *c == b'!')
        {
            i = s + 1;
            continue;
        }
        let (mut j, mut quote) = (s + 1, 0u8);
        while j < b.len() {
            let c = b[j];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        let tag = &html[s..j.min(html.len())];
        out.push(
            tag[1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '/' | '!' | '-' | ':'))
                .collect::<String>()
                .to_ascii_lowercase(),
        );
        for attr in [" href=", " src="] {
            if let Some(p) = tag.find(attr) {
                let v = &tag[p + attr.len()..];
                let v = match v.as_bytes().first() {
                    Some(q @ (b'"' | b'\'')) => v[1..].split(*q as char).next().unwrap_or(""),
                    _ => v.split_whitespace().next().unwrap_or(""),
                };
                out.push(format!("{attr}{v}"));
            }
        }
        i = j + 1;
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> SegmentOpts {
        SegmentOpts::new("en")
    }

    fn fake(plan: &Plan) -> HashMap<String, String> {
        plan.leaves
            .iter()
            .flat_map(|l| &l.units)
            .map(|u| (u.hash.clone(), format!("[fr] {}", u.text)))
            .collect()
    }

    #[test]
    fn text_keeps_whitespace_tokens_and_values() {
        let src = "Hi {{ first_name }},\n\n  Your order 48213 ships to jane@acme.io.\nTrack it: https://acme.io/t/9f3a\n\n-- \n{{team}}\n";
        let p = plan("text", &Value::String(src.into()), None, &opts()).unwrap();
        let units: Vec<&str> = p.leaves[0].units.iter().map(|u| u.text.as_str()).collect();
        assert_eq!(units.len(), 2, "{units:?}"); // the signature `{{team}}` has no words
        assert_eq!(units[0], "Hi {v0},");
        // The order number, the email and the URL are placeholders: never sent, never part of the hash.
        assert!(
            !units[1].contains("48213")
                && !units[1].contains("jane@")
                && !units[1].contains("https")
        );
        // No translation: byte for byte.
        let a = assemble(&p.leaves[0], &opts(), &HashMap::new(), "fr");
        assert_eq!((a.text.as_str(), a.missing), (src, 2));
        let a = assemble(&p.leaves[0], &opts(), &fake(&p), "fr");
        assert_eq!(a.missing, 0);
        assert!(a.text.starts_with(
            "[fr] Hi {{ first_name }},\n\n  [fr] Your order 48213 ships to jane@acme.io."
        ));
        assert!(
            a.text
                .ends_with("https://acme.io/t/9f3a\n\n-- \n{{team}}\n")
        );
    }

    #[test]
    fn same_template_same_hash_whatever_the_values() {
        let h = |s: &str| {
            plan("text", &Value::String(s.into()), None, &opts())
                .unwrap()
                .leaves[0]
                .units[0]
                .hash
                .clone()
        };
        assert_eq!(h("Order 48213 shipped"), h("Order 77 shipped"));
        assert_eq!(h("Hi {{name}}"), h("Hi {{ first_name }}"));
        assert_ne!(h("Order 48213 shipped"), h("Order 48213 delayed"));
    }

    #[test]
    fn html_unit_checks() {
        assert!(
            placeholders_kept("Read <0>the docs</0>, {v0}", "Lisez <0>la doc</0>, {v0}").is_ok()
        );
        for bad in [
            "Lisez la doc, {v0}",
            "Lisez <0>la doc, {v0}",
            "Lisez </0>la doc<0>, {v0}",
            "Lisez <0>la doc</0>",
            "Lisez <0>la doc</0>, {v0} {v1}",
        ] {
            assert!(
                placeholders_kept("Read <0>the docs</0>, {v0}", bad).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn html_that_loses_a_link_returns_the_source() {
        let l = Leaf {
            pointer: String::new(),
            kind: Kind::Html,
            source: "<p>Hello</p>".into(),
            units: vec![],
        };
        assert_eq!(tag_signature(&l.source), tag_signature("<p>Bonjour</p>"));
        assert_ne!(
            tag_signature("<a href=\"https://a.io/x\">x</a>"),
            tag_signature("<a href=\"https://evil.io\">x</a>")
        );
        assert_ne!(tag_signature("<p><b>x</b></p>"), tag_signature("<p>x</p>"));
    }

    #[test]
    fn json_fields() {
        let doc = serde_json::json!({
            "id": "sku_1", "title": "Blue shirt", "tags": ["new", "sale"],
            "variants": [{"name": "Large", "sku": "L"}, {"name": "Small", "sku": "S"}],
            "body": "<p>Soft <b>cotton</b></p>"
        });
        let fields = serde_json::json!(["title", "variants.*.name", {"path": "body", "type": "html"}, "nope"]);
        let p = plan("json", &doc, Some(&fields), &opts()).unwrap();
        let ptrs: Vec<&str> = p.leaves.iter().map(|l| l.pointer.as_str()).collect();
        assert_eq!(
            ptrs,
            ["/title", "/variants/0/name", "/variants/1/name", "/body"]
        );
        assert_eq!(p.warnings.len(), 1);
        assert!(
            plan("json", &doc, None, &opts())
                .unwrap_err()
                .contains("fields")
        );
    }

    #[test]
    fn broken_template_is_refused_with_the_reason() {
        let e = plan(
            "template",
            &Value::String("{n, plural, one {# file}".into()),
            None,
            &opts(),
        )
        .unwrap_err();
        assert!(e.contains("ICU"), "{e}");
    }
}
