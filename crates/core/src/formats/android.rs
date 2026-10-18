//! Android `res/values*/strings.xml`: `<string>`, `<plurals>` (ICU plural on `count`),
//! `<string-array>` (`name[0]`, `name[1]`…). `translatable="false"` entries are left alone.
//! Values keep markup and `&lt;` as written, other entities are decoded; Android backslash escapes (`\'`, `\"`, `\n`,
//! `\@`, `\uXXXX`) are decoded, CDATA content is taken raw.
use std::collections::{BTreeMap, BTreeSet};

use super::xml::{self, Elem, Node};
use super::{Parsed, folded_choice, icu_choice, line_end, line_start};
use crate::catalog::Catalog;

const SKELETON: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n</resources>\n";

enum Kind {
    String,
    Plurals,
    Array,
}

struct Entry {
    kind: Kind,
    name: String,
    elem: Elem,
    /// `<item>`s of plurals / arrays.
    items: Vec<Elem>,
    note: Option<String>,
}

fn root(s: &str) -> Result<(usize, usize), String> {
    let open = s.find("<resources").ok_or("no <resources> element")?;
    let start = open + s[open..].find('>').ok_or("unclosed <resources>")? + 1;
    let end = s.rfind("</resources>").ok_or("no </resources>")?;
    Ok((start, end))
}

fn entries(s: &str) -> Result<Vec<Entry>, String> {
    let (from, to) = root(s)?;
    let mut out = vec![];
    let mut note = None;
    for node in xml::children(s, from, to)? {
        let e = match node {
            Node::Comment(c) => {
                note = Some(c);
                continue;
            }
            Node::Elem(e) => e,
        };
        let note = note.take();
        let kind = match e.tag.as_str() {
            "string" => Kind::String,
            "plurals" => Kind::Plurals,
            "string-array" => Kind::Array,
            _ => continue,
        };
        let Some(name) = e.attr("name").map(str::to_string) else {
            continue;
        };
        if e.attr("translatable") == Some("false") {
            continue;
        }
        let items = match kind {
            Kind::String => vec![],
            _ => xml::children(s, e.open_end, e.close)?
                .into_iter()
                .filter_map(|n| match n {
                    Node::Elem(i) if i.tag == "item" => Some(i),
                    _ => None,
                })
                .collect(),
        };
        out.push(Entry {
            kind,
            name,
            elem: e,
            items,
            note,
        });
    }
    Ok(out)
}

fn inner(s: &str, e: &Elem) -> String {
    decode(&s[e.open_end..e.close])
}

fn plural_value(s: &str, e: &Entry) -> String {
    let forms: Vec<(String, String)> = e
        .items
        .iter()
        .map(|i| {
            (
                i.attr("quantity").unwrap_or("other").to_string(),
                inner(s, i),
            )
        })
        .collect();
    icu_choice("count", "plural", &forms)
}

pub fn parse(s: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    for e in entries(s)? {
        if let Some(n) = &e.note {
            p.notes.insert(e.name.clone(), n.clone());
        }
        match e.kind {
            Kind::String => {
                p.entries.insert(e.name.clone(), inner(s, &e.elem));
            }
            Kind::Plurals => {
                p.entries.insert(e.name.clone(), plural_value(s, &e));
            }
            Kind::Array => {
                for (i, it) in e.items.iter().enumerate() {
                    p.entries.insert(format!("{}[{i}]", e.name), inner(s, it));
                }
            }
        }
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or(SKELETON);
    let Ok(list) = entries(s) else {
        // ponytail: unparseable existing file => regenerate (parse errors surface on read first).
        return write(None, cat);
    };
    let indent = list
        .first()
        .map(|e| s[line_start(s, e.elem.start)..e.elem.start].to_string())
        .filter(|i| i.trim().is_empty() && !i.is_empty())
        .unwrap_or_else(|| "    ".into());
    let arrays = group_arrays(cat);
    let (mut out, mut pos, mut done) = (String::new(), 0, BTreeSet::new());
    let replace = |out: &mut String, pos: &mut usize, from: usize, to: usize, with: &str| {
        out.push_str(&s[*pos..from]);
        out.push_str(with);
        *pos = to;
    };
    for e in &list {
        let (from, to) = (line_start(s, e.elem.start), line_end(s, e.elem.end));
        match e.kind {
            Kind::String => {
                done.insert(e.name.clone());
                match cat.entries.get(&e.name) {
                    None => replace(&mut out, &mut pos, from, to, ""),
                    Some(v) if *v == inner(s, &e.elem) => {}
                    Some(v) if folded_choice(v).is_some() => {
                        let el = render_plurals(&e.name, v, &indent, &indent);
                        replace(&mut out, &mut pos, from, to, &el)
                    }
                    Some(v) => {
                        let raw = &s[e.elem.open_end..e.elem.close];
                        let enc = encode(v, raw.trim_start().starts_with("<![CDATA["));
                        replace(&mut out, &mut pos, e.elem.open_end, e.elem.close, &enc)
                    }
                }
            }
            Kind::Plurals => {
                done.insert(e.name.clone());
                match cat.entries.get(&e.name) {
                    None => replace(&mut out, &mut pos, from, to, ""),
                    Some(v) if *v == plural_value(s, e) => {}
                    Some(v) => {
                        let ind = s[from..e.elem.start].to_string();
                        let item_ind = e
                            .items
                            .first()
                            .map(|i| s[line_start(s, i.start)..i.start].to_string())
                            .filter(|i| i.trim().is_empty() && !i.is_empty())
                            .unwrap_or_else(|| format!("{ind}{indent}"));
                        let head = &s[e.elem.start..e.elem.open_end];
                        let el = render_items(head, "plurals", &plural_items(v), &ind, &item_ind);
                        replace(&mut out, &mut pos, from, to, &el)
                    }
                }
            }
            Kind::Array => {
                done.insert(format!("{}[", e.name));
                let current: Vec<String> = e.items.iter().map(|i| inner(s, i)).collect();
                match arrays.get(&e.name) {
                    None => replace(&mut out, &mut pos, from, to, ""),
                    Some(v) if *v == current => {}
                    Some(v) => {
                        let ind = s[from..e.elem.start].to_string();
                        let items: Vec<(String, String)> =
                            v.iter().map(|x| (String::new(), x.clone())).collect();
                        let head = &s[e.elem.start..e.elem.open_end];
                        let el = render_items(
                            head,
                            "string-array",
                            &items,
                            &ind,
                            &format!("{ind}{indent}"),
                        );
                        replace(&mut out, &mut pos, from, to, &el)
                    }
                }
            }
        }
    }
    // New keys go before `</resources>`.
    let end = root(s).map_or(s.len(), |r| r.1);
    let at = line_start(s, end);
    out.push_str(&s[pos..at]);
    for (k, v) in &cat.entries {
        if done.contains(k) {
            continue;
        }
        if let Some((name, _)) = array_key(k) {
            if done.insert(format!("{name}[")) {
                let items: Vec<(String, String)> = arrays[name]
                    .iter()
                    .map(|x| (String::new(), x.clone()))
                    .collect();
                let head = format!("<string-array name=\"{name}\">");
                out += &render_items(&head, "string-array", &items, &indent, &indent.repeat(2));
            }
        } else if folded_choice(v).is_some() {
            out += &render_plurals(k, v, &indent, &indent);
        } else {
            out += &format!(
                "{indent}<string name=\"{k}\">{}</string>\n",
                encode(v, false)
            );
        }
    }
    if at < end && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&s[at..]);
    out
}

fn array_key(k: &str) -> Option<(&str, usize)> {
    let (name, rest) = k.split_once('[')?;
    Some((name, rest.strip_suffix(']')?.parse().ok()?))
}

fn group_arrays(cat: &Catalog) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, BTreeMap<usize, String>> = BTreeMap::new();
    for (k, v) in &cat.entries {
        if let Some((name, i)) = array_key(k) {
            m.entry(name.into()).or_default().insert(i, v.clone());
        }
    }
    m.into_iter()
        .map(|(k, v)| (k, v.into_values().collect()))
        .collect()
}

fn plural_items(v: &str) -> Vec<(String, String)> {
    match folded_choice(v) {
        Some(c) => c
            .forms
            .into_iter()
            .filter(|(k, _)| !k.starts_with('='))
            .map(|(k, t)| (format!(" quantity=\"{k}\""), t))
            .collect(),
        None => vec![(" quantity=\"other\"".into(), v.into())],
    }
}

fn render_plurals(name: &str, v: &str, ind: &str, step: &str) -> String {
    let head = format!("<plurals name=\"{name}\">");
    render_items(
        &head,
        "plurals",
        &plural_items(v),
        ind,
        &format!("{ind}{step}"),
    )
}

/// `head` + one `<item{attrs}>` per line + `</tag>`, each on its own indented line.
fn render_items(
    head: &str,
    tag: &str,
    items: &[(String, String)],
    ind: &str,
    item_ind: &str,
) -> String {
    let mut out = format!("{ind}{head}\n");
    for (attrs, v) in items {
        out += &format!("{item_ind}<item{attrs}>{}</item>\n", encode(v, false));
    }
    out + &format!("{ind}</{tag}>\n")
}

/// Android escapes => text. Entities and markup are kept as written.
fn decode(raw: &str) -> String {
    let t = raw.trim();
    if let Some(c) = t
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
    {
        return c.to_string();
    }
    // Entities are decoded except `&lt;`, so `<b>` (markup) and `&lt;` (text) stay distinct.
    let raw = raw
        .split("&lt;")
        .map(xml::unescape)
        .collect::<Vec<_>>()
        .join("&lt;");
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = it.by_ref().take(4).collect();
                    out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                }
                Some(c) => out.push(c),
                None => {}
            },
            // Unescaped double quotes only delimit whitespace-preserving runs.
            '"' => {}
            c => out.push(c),
        }
    }
    out
}

fn encode(v: &str, cdata: bool) -> String {
    if cdata {
        return format!("<![CDATA[{v}]]>");
    }
    let mut out = String::with_capacity(v.len() + 8);
    for (i, c) in v.char_indices() {
        match c {
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\t' => out += "\\t",
            '\'' => out += "\\'",
            '"' => out += "\\\"",
            '@' | '?' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            '&' if !v[i..].starts_with("&lt;") => out += "&amp;",
            c => out.push(c),
        }
    }
    out
}
