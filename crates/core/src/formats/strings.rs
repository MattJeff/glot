//! Apple legacy `Localizable.strings` (`"key" = "value";`, `/* note */`) and `.stringsdict`
//! (plist; `%#@var@` variables become ICU plurals named after the variable).
use std::collections::BTreeMap;

use super::xml::{self, Elem, Node};
use super::{Parsed, choices, icu_choice, line_end, line_start};
use crate::catalog::Catalog;

// ---------------------------------------------------------------- .strings

struct Entry {
    key: String,
    value: String,
    /// Value literal content (inside the quotes).
    val: (usize, usize),
    /// Whole entry incl. its attached comment.
    span: (usize, usize),
    note: Option<String>,
}

fn entries(s: &str) -> Result<Vec<Entry>, String> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    // (comment text, comment start, comment end)
    let mut note: Option<(String, usize, usize)> = None;
    let skip_ws = |i: &mut usize| {
        while *i < b.len() && (b[*i] as char).is_ascii_whitespace() {
            *i += 1;
        }
    };
    loop {
        skip_ws(&mut i);
        if i >= b.len() {
            break;
        }
        if s[i..].starts_with("/*") {
            let e = s[i..].find("*/").ok_or("unclosed /* comment")? + i + 2;
            note = Some((s[i + 2..e - 2].trim().to_string(), i, e));
            i = e;
            continue;
        }
        if s[i..].starts_with("//") {
            let e = s[i..].find('\n').map_or(s.len(), |n| i + n);
            note = Some((s[i + 2..e].trim().to_string(), i, e));
            i = e;
            continue;
        }
        let start = i;
        let (key, _) = token(s, &mut i)?;
        skip_ws(&mut i);
        if b.get(i) != Some(&b'=') {
            return Err(format!("expected `=` after \"{key}\""));
        }
        i += 1;
        skip_ws(&mut i);
        let (value, val) = token(s, &mut i)?;
        skip_ws(&mut i);
        if b.get(i) != Some(&b';') {
            return Err(format!("expected `;` after the value of \"{key}\""));
        }
        i += 1;
        // A comment belongs to the entry when only whitespace separates them.
        let note = note
            .take()
            .filter(|(_, _, e)| s[*e..start].trim().is_empty());
        let from = note.as_ref().map_or(start, |n| n.1);
        out.push(Entry {
            key,
            value,
            val,
            span: (line_start(s, from), line_end(s, i)),
            note: note.map(|n| n.0),
        });
    }
    Ok(out)
}

/// Quoted string or bare word; returns the decoded text and the raw content span.
fn token(s: &str, i: &mut usize) -> Result<(String, (usize, usize)), String> {
    let b = s.as_bytes();
    if b.get(*i) == Some(&b'"') {
        let start = *i + 1;
        let mut j = start;
        while j < b.len() && b[j] != b'"' {
            j += if b[j] == b'\\' { 2 } else { 1 };
        }
        if j >= b.len() {
            return Err("unterminated string".into());
        }
        *i = j + 1;
        Ok((unescape(&s[start..j]), (start, j)))
    } else {
        let start = *i;
        while *i < b.len() && (b[*i].is_ascii_alphanumeric() || matches!(b[*i], b'_' | b'.' | b'-'))
        {
            *i += 1;
        }
        if start == *i {
            return Err(format!("unexpected character at byte {start}"));
        }
        Ok((s[start..*i].to_string(), (start, *i)))
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('U' | 'u') => {
                let hex: String = it.by_ref().take(4).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

pub fn parse(s: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    for e in entries(s)? {
        if let Some(n) = e
            .note
            .filter(|n| !n.is_empty() && n != "No comment provided by engineer.")
        {
            p.notes.insert(e.key.clone(), n);
        }
        p.entries.insert(e.key, e.value);
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or("");
    let Ok(list) = entries(s) else {
        return write(None, cat);
    };
    let (mut out, mut pos) = (String::new(), 0);
    for e in &list {
        match cat.entries.get(&e.key) {
            None => {
                out.push_str(&s[pos..e.span.0]);
                pos = e.span.1;
            }
            Some(v) if *v == e.value => {}
            Some(v) => {
                out.push_str(&s[pos..e.val.0]);
                out.push_str(&escape(v));
                pos = e.val.1;
            }
        }
    }
    out.push_str(&s[pos..]);
    for (k, v) in &cat.entries {
        if list.iter().any(|e| &e.key == k) {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out += &format!("\"{}\" = \"{}\";\n", escape(k), escape(v));
    }
    out
}

// ---------------------------------------------------------------- .stringsdict

const DICT_SKELETON: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n</dict>\n</plist>\n";

/// `(key elem, value elem)` pairs of a plist `<dict>`.
fn pairs(s: &str, dict: &Elem) -> Result<Vec<(String, Elem, Elem)>, String> {
    let elems: Vec<Elem> = xml::children(s, dict.open_end, dict.close)?
        .into_iter()
        .filter_map(|n| match n {
            Node::Elem(e) => Some(e),
            Node::Comment(_) => None,
        })
        .collect();
    Ok(elems
        .chunks(2)
        .filter(|c| c.len() == 2 && c[0].tag == "key")
        .map(|c| (text(s, &c[0]), c[0].clone(), c[1].clone()))
        .collect())
}

fn text(s: &str, e: &Elem) -> String {
    xml::unescape(&s[e.open_end..e.close])
}

fn root_dict(s: &str) -> Result<Elem, String> {
    let plist = s.find("<plist").ok_or("no <plist>")?;
    xml::children(s, plist, s.len())?
        .into_iter()
        .find_map(|n| match n {
            Node::Elem(e) if e.tag == "plist" => Some(e),
            _ => None,
        })
        .and_then(|p| {
            xml::children(s, p.open_end, p.close)
                .ok()?
                .into_iter()
                .find_map(|n| match n {
                    Node::Elem(e) if e.tag == "dict" => Some(e),
                    _ => None,
                })
        })
        .ok_or_else(|| "no root <dict>".into())
}

struct DictEntry {
    key: String,
    value: String,
    /// variable => (`%#@var@` token as written, NSStringFormatValueTypeKey)
    vars: BTreeMap<String, (String, String)>,
    span: (usize, usize),
}

fn dict_entries(s: &str) -> Result<Vec<DictEntry>, String> {
    let root = root_dict(s)?;
    let mut out = vec![];
    for (key, k, v) in pairs(s, &root)? {
        if v.tag != "dict" {
            continue;
        }
        let mut format = String::new();
        let mut vars = BTreeMap::new();
        let mut forms: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for (name, _, val) in pairs(s, &v)? {
            if name == "NSStringLocalizedFormatKey" {
                format = text(s, &val);
            } else if val.tag == "dict" {
                let mut value_type = String::new();
                let mut f = vec![];
                for (cat, _, t) in pairs(s, &val)? {
                    match cat.as_str() {
                        "NSStringFormatValueTypeKey" => value_type = text(s, &t),
                        "NSStringFormatSpecTypeKey" => {}
                        _ => f.push((cat, text(s, &t))),
                    }
                }
                forms.insert(name.clone(), f);
                vars.insert(name, (String::new(), value_type));
            }
        }
        // `%#@var@` / `%1$#@var@` => `{var, plural, ...}`.
        let mut value = format.clone();
        for (name, (token, _)) in vars.iter_mut() {
            if let Some((a, b)) = find_token(&value, name) {
                *token = value[a..b].to_string();
                let icu = icu_choice(name, "plural", &forms[name]);
                value.replace_range(a..b, &icu);
            }
        }
        out.push(DictEntry {
            key,
            value,
            vars,
            span: (line_start(s, k.start), line_end(s, v.end)),
        });
    }
    Ok(out)
}

/// Byte range of `%#@name@` (with optional `N$` position) in `s`.
pub(crate) fn find_token(s: &str, name: &str) -> Option<(usize, usize)> {
    let pat = format!("#@{name}@");
    let at = s.find(&pat)?;
    let pct = s[..at].rfind('%')?;
    s[pct + 1..at]
        .chars()
        .all(|c| c.is_ascii_digit() || c == '$')
        .then_some((pct, at + pat.len()))
}

pub fn parse_dict(s: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    for e in dict_entries(s)? {
        p.entries.insert(e.key, e.value);
    }
    Ok(p)
}

pub fn write_dict(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or(DICT_SKELETON);
    let (Ok(list), Ok(root)) = (dict_entries(s), root_dict(s)) else {
        return write_dict(None, cat);
    };
    let (mut out, mut pos) = (String::new(), 0);
    for e in &list {
        match cat.entries.get(&e.key) {
            Some(v) if *v == e.value => {}
            v => {
                out.push_str(&s[pos..e.span.0]);
                if let Some(v) = v {
                    out += &render_dict(&e.key, v, &e.vars);
                }
                pos = e.span.1;
            }
        }
    }
    let at = line_start(s, root.close);
    out.push_str(&s[pos..at]);
    for (k, v) in &cat.entries {
        if !list.iter().any(|e| &e.key == k) {
            out += &render_dict(k, v, &BTreeMap::new());
        }
    }
    out.push_str(&s[at..]);
    out
}

/// `%#@var@`, positional (`%1$#@var@`) when the message has other positional arguments: the
/// first position they leave free.
fn new_token(v: &str, var: &str) -> String {
    let used: Vec<usize> = v
        .match_indices('%')
        .filter_map(|(i, _)| {
            let digits: String = v[i + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            v[i + 1 + digits.len()..]
                .starts_with('$')
                .then(|| digits.parse().ok())?
        })
        .collect();
    match (1..).find(|n| !used.contains(n)) {
        Some(n) if !used.is_empty() => format!("%{n}$#@{var}@"),
        _ => format!("%#@{var}@"),
    }
}

/// NSStringFormatValueTypeKey from the branch text (`%ld` => `ld`), `d` by default.
fn value_type(forms: &[(String, String)]) -> String {
    forms
        .iter()
        .find_map(|(_, t)| {
            let i = t.find('%')?;
            let spec: String = t[i + 1..]
                .chars()
                .skip_while(|c| c.is_ascii_digit() || *c == '$')
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let end = spec.find(|c: char| !"hlqLzjt".contains(c))?;
            Some(spec[..=end].to_string()).filter(|s| s.ends_with(['d', 'i', 'u', 'f', 'g', 'e']))
        })
        .unwrap_or_else(|| "d".into())
}

/// One top-level `<key>` + `<dict>` in Xcode's tab-indented layout.
fn render_dict(key: &str, v: &str, vars: &BTreeMap<String, (String, String)>) -> String {
    let mut format = v.to_string();
    let mut body = String::new();
    for c in choices(v).into_iter().rev() {
        let (token, value_type) = vars
            .get(&c.var)
            .cloned()
            .filter(|(t, _)| !t.is_empty())
            .unwrap_or_else(|| (new_token(v, &c.var), value_type(&c.forms)));
        format.replace_range(c.start..c.end, &token);
        let mut d = format!(
            "\t\t<key>{}</key>\n\t\t<dict>\n\t\t\t<key>NSStringFormatSpecTypeKey</key>\n\t\t\t<string>NSStringPluralRuleType</string>\n\t\t\t<key>NSStringFormatValueTypeKey</key>\n\t\t\t<string>{}</string>\n",
            xml::escape(&c.var),
            xml::escape(&value_type)
        );
        for (cat, t) in c.forms.iter().filter(|(k, _)| !k.starts_with('=')) {
            d += &format!(
                "\t\t\t<key>{}</key>\n\t\t\t<string>{}</string>\n",
                xml::escape(cat),
                xml::escape(t)
            );
        }
        body = d + "\t\t</dict>\n" + &body;
    }
    format!(
        "\t<key>{}</key>\n\t<dict>\n\t\t<key>NSStringLocalizedFormatKey</key>\n\t\t<string>{}</string>\n{body}\t</dict>\n",
        xml::escape(key),
        xml::escape(&format)
    )
}
