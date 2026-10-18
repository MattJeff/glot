//! YAML i18n catalogs: Rails (`config/locales/en.yml`, root key = locale, stripped) and Symfony
//! (`translations/messages.fr.yaml`, no root). A mapping whose children are only CLDR categories
//! (`one:`/`other:`) is a plural => `{count, plural, one {...} other {...}}`.
//! Line-based subset parser (mappings, plain/quoted/block scalars, comments); anchors, aliases,
//! flow collections and lists are left untouched and not translated.
use std::path::Path;

use super::{Parsed, folded_choice, icu_choice};
use crate::catalog::Catalog;

const CATS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Style {
    Plain,
    Single,
    Double,
    Block,
}

#[derive(Debug, Clone)]
struct Item {
    path: Vec<String>,
    indent: usize,
    line: usize,
    /// Last line (inclusive): block lines for scalars, last descendant for mappings.
    last: usize,
    /// `None` = mapping.
    scalar: Option<(String, Style)>,
    /// Byte columns of the value token in its line (single-line scalars).
    cols: (usize, usize),
    child_indent: Option<usize>,
    note: Option<String>,
}

struct Doc<'a> {
    lines: Vec<&'a str>,
    items: Vec<Item>,
    /// Rails locale root key.
    root: Option<String>,
}

fn same_locale(a: &str, b: &str) -> bool {
    a.replace('_', "-")
        .eq_ignore_ascii_case(&b.replace('_', "-"))
}

fn parse_doc<'a>(s: &'a str, locale: &str) -> Result<Doc<'a>, String> {
    let lines: Vec<&str> = s.split_inclusive('\n').collect();
    let mut items: Vec<Item> = vec![];
    let mut stack: Vec<(usize, String, usize)> = vec![];
    let mut comments: Vec<String> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let content = lines[i].trim_end_matches(['\n', '\r']);
        let t = content.trim_start();
        let indent = content.len() - t.len();
        i += 1;
        if t.is_empty() {
            comments.clear();
            continue;
        }
        if let Some(c) = t.strip_prefix('#') {
            comments.push(c.trim().to_string());
            continue;
        }
        if t == "---" || t == "..." || t.starts_with('%') || t.starts_with('-') {
            // ponytail: lists are not translated; their nested lines are skipped by indentation.
            while i < lines.len() && indent_of(lines[i]).is_none_or(|n| n > indent) {
                i += 1;
            }
            continue;
        }
        let Some((key, after)) = split_key(t) else {
            return Err(format!("line {i}: expected `key: value`"));
        };
        while stack.last().is_some_and(|(n, _, _)| *n >= indent) {
            stack.pop();
        }
        if let Some(&(_, _, parent)) = stack.last() {
            items[parent].child_indent.get_or_insert(indent);
        }
        let mut path: Vec<String> = stack.iter().map(|(_, k, _)| k.clone()).collect();
        path.push(key.clone());
        let note = (!comments.is_empty()).then(|| comments.join("\n"));
        comments.clear();
        let value_col = indent + (t.len() - after.len());
        let rest = after.trim_end();
        let mut item = Item {
            path,
            indent,
            line: i - 1,
            last: i - 1,
            scalar: None,
            cols: (value_col, value_col + rest.len()),
            child_indent: None,
            note,
        };
        match rest.chars().next() {
            None | Some('#') => {
                stack.push((indent, key, items.len()));
                items.push(item);
                continue;
            }
            Some('|' | '>') => {
                let mut body = vec![];
                let mut j = i;
                while j < lines.len() && indent_of(lines[j]).is_none_or(|n| n > indent) {
                    j += 1;
                }
                while j > i && indent_of(lines[j - 1]).is_none() {
                    j -= 1;
                }
                let block_indent = lines[i..j]
                    .iter()
                    .filter_map(|l| indent_of(l))
                    .min()
                    .unwrap_or(0);
                for l in &lines[i..j] {
                    let l = l.trim_end_matches(['\n', '\r']);
                    body.push(l.get(block_indent..).unwrap_or(""));
                }
                let folded = rest.starts_with('>');
                let mut v = if folded {
                    body.join(" ").replace(" \n", "\n")
                } else {
                    body.join("\n")
                };
                if !rest.contains('-') {
                    v.push('\n');
                }
                item.scalar = Some((v, Style::Block));
                item.last = j.max(i) - 1;
                i = j;
            }
            Some('"') => {
                let Some(end) = close_quote(rest, '"') else {
                    continue;
                };
                item.scalar = Some((unescape_double(&rest[1..end]), Style::Double));
                item.cols.1 = value_col + end + 1;
            }
            Some('\'') => {
                let Some(end) = close_quote(rest, '\'') else {
                    continue;
                };
                item.scalar = Some((rest[1..end].replace("''", "'"), Style::Single));
                item.cols.1 = value_col + end + 1;
            }
            Some('&' | '*' | '[' | '{' | '!' | '@' | '`') => continue,
            Some(_) => {
                let v = rest.find(" #").map_or(rest, |c| &rest[..c]).trim_end();
                item.cols.1 = value_col + v.len();
                item.scalar = Some((v.to_string(), Style::Plain));
            }
        }
        items.push(item);
    }
    // A mapping ends with its last descendant.
    for k in 0..items.len() {
        if items[k].scalar.is_none() {
            let p = items[k].path.clone();
            let last = items
                .iter()
                .filter(|o| o.path.len() > p.len() && o.path.starts_with(&p))
                .map(|o| o.last)
                .max();
            if let Some(l) = last {
                items[k].last = items[k].last.max(l);
            }
        }
    }
    let tops: Vec<&Item> = items.iter().filter(|it| it.path.len() == 1).collect();
    let root = match tops.as_slice() {
        [t] if t.scalar.is_none() && same_locale(&t.path[0], locale) => Some(t.path[0].clone()),
        _ => None,
    };
    Ok(Doc { lines, items, root })
}

fn indent_of(line: &str) -> Option<usize> {
    let c = line.trim_end_matches(['\n', '\r']);
    let t = c.trim_start();
    (!t.is_empty()).then(|| c.len() - t.len())
}

/// `key: rest` => (key, rest). Keys may be quoted.
fn split_key(t: &str) -> Option<(String, &str)> {
    let (key, rest) = match t.chars().next()? {
        q @ ('"' | '\'') => {
            let end = close_quote(t, q)?;
            let k = &t[1..end];
            let k = if q == '"' {
                unescape_double(k)
            } else {
                k.replace("''", "'")
            };
            (k, t[end + 1..].trim_start())
        }
        _ => {
            let at = t
                .match_indices(':')
                .map(|(i, _)| i)
                .find(|&i| t[i + 1..].is_empty() || t[i + 1..].starts_with([' ', '\t']))?;
            (t[..at].trim_end().to_string(), &t[at..])
        }
    };
    let rest = rest.strip_prefix(':')?;
    Some((key, rest.trim_start()))
}

/// Index of the closing quote in `s` (which starts with the opening one).
fn close_quote(s: &str, q: char) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() {
        match b[i] as char {
            '\\' if q == '"' => i += 2,
            c if c == q && q == '\'' && b.get(i + 1) == Some(&b'\'') => i += 2,
            c if c == q => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn unescape_double(s: &str) -> String {
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
            Some('0') => out.push('\0'),
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

/// Catalog entries: (key, value, item index, is plural).
fn entries(doc: &Doc) -> Vec<(String, String, usize, bool)> {
    let skip = doc.root.as_ref().map_or(0, |_| 1);
    let mut out = vec![];
    let children = |k: usize| -> Vec<usize> {
        let p = &doc.items[k].path;
        (0..doc.items.len())
            .filter(|&o| doc.items[o].path.len() == p.len() + 1 && doc.items[o].path.starts_with(p))
            .collect()
    };
    let is_plural = |k: usize| {
        let c = children(k);
        !c.is_empty()
            && c.iter().all(|&o| {
                doc.items[o].scalar.is_some()
                    && CATS.contains(&doc.items[o].path.last().unwrap().as_str())
            })
            && c.iter()
                .any(|&o| doc.items[o].path.last().unwrap() == "other")
    };
    let plurals: Vec<usize> = (0..doc.items.len())
        .filter(|&k| {
            doc.items[k].scalar.is_none() && doc.items[k].path.len() > skip && is_plural(k)
        })
        .collect();
    for (k, it) in doc.items.iter().enumerate() {
        if it.path.len() <= skip {
            continue;
        }
        let key = it.path[skip..].join(".");
        if plurals.contains(&k) {
            let forms: Vec<(String, String)> = children(k)
                .into_iter()
                .map(|o| {
                    let c = &doc.items[o];
                    (c.path.last().unwrap().clone(), c.scalar.clone().unwrap().0)
                })
                .collect();
            out.push((key, icu_choice("count", "plural", &forms), k, true));
        } else if let Some((v, _)) = &it.scalar
            && !plurals.iter().any(|&p| {
                it.path.len() == doc.items[p].path.len() + 1
                    && it.path.starts_with(&doc.items[p].path)
            })
        {
            out.push((key, v.clone(), k, false));
        }
    }
    out
}

pub fn parse(s: &str, locale: &str) -> Result<Parsed, String> {
    let doc = parse_doc(s, locale)?;
    let mut p = Parsed::default();
    for (key, v, k, _) in entries(&doc) {
        if let Some(n) = &doc.items[k].note {
            p.notes.insert(key.clone(), n.clone());
        }
        p.entries.insert(key, v);
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog, path: &Path) -> String {
    // New file: Rails (`.yml`) nests under the locale, Symfony (`.yaml`) does not.
    let fresh = if path.extension().is_some_and(|e| e == "yml") {
        format!("{}:\n", cat.locale)
    } else {
        String::new()
    };
    let s = existing.unwrap_or(&fresh);
    let Ok(doc) = parse_doc(s, &cat.locale) else {
        return write(None, cat, path);
    };
    let list = entries(&doc);
    // (first line, last line, replacement), applied bottom-up.
    let mut edits: Vec<(usize, usize, String)> = vec![];
    for (key, current, k, plural) in &list {
        let it = &doc.items[*k];
        match cat.entries.get(key) {
            None => edits.push((it.line, it.last, String::new())),
            Some(v) if v == current => {}
            Some(v) if *plural => {
                let ind = " ".repeat(it.child_indent.unwrap_or(it.indent + 2));
                let style = doc
                    .items
                    .iter()
                    .find(|c| c.line == it.line + 1)
                    .and_then(|c| c.scalar.as_ref().map(|s| s.1))
                    .unwrap_or(Style::Double);
                edits.push((it.line + 1, it.last, plural_lines(v, &ind, style)));
            }
            Some(v) => {
                let line = doc.lines[it.line];
                let style = it.scalar.as_ref().unwrap().1;
                let text = if style == Style::Block || v.contains('\n') {
                    block(&line[..it.cols.0], v, it.indent + 2)
                } else {
                    format!(
                        "{}{}{}",
                        &line[..it.cols.0],
                        scalar(v, style),
                        &line[it.cols.1..]
                    )
                };
                edits.push((it.line, it.last, text));
            }
        }
    }
    let mut lines: Vec<String> = doc.lines.iter().map(|l| l.to_string()).collect();
    if lines.last().is_some_and(|l| !l.ends_with('\n')) {
        lines.last_mut().unwrap().push('\n');
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (a, b, text) in edits {
        lines.splice(a..=b, (!text.is_empty()).then_some(text));
    }
    let mut text: String = lines.concat();
    // New keys: under their deepest existing parent mapping, one at a time.
    let known: std::collections::BTreeSet<&String> = list.iter().map(|e| &e.0).collect();
    for (k, v) in &cat.entries {
        if !known.contains(k) {
            text = insert(&text, &cat.locale, k, v);
        }
    }
    if existing.is_some_and(|e| !e.ends_with('\n')) {
        text.pop();
    }
    text
}

fn insert(text: &str, locale: &str, key: &str, v: &str) -> String {
    let Ok(doc) = parse_doc(text, locale) else {
        return text.to_string();
    };
    let mut segs: Vec<String> = doc.root.iter().cloned().collect();
    segs.extend(key.split('.').map(str::to_string));
    let (n, at, mut indent) = (1..segs.len())
        .rev()
        .find_map(|n| {
            doc.items
                .iter()
                .find(|it| it.scalar.is_none() && it.path == segs[..n])
                .map(|it| (n, it.last + 1, it.child_indent.unwrap_or(it.indent + 2)))
        })
        .unwrap_or((0, doc.lines.len(), 0));
    let mut add = String::new();
    for seg in &segs[n..segs.len() - 1] {
        add += &format!("{}{}:\n", " ".repeat(indent), yaml_key(seg));
        indent += 2;
    }
    let last = yaml_key(segs.last().unwrap());
    let pad = " ".repeat(indent);
    if folded_choice(v).is_some_and(|c| c.kind == "plural") {
        add += &format!("{pad}{last}:\n");
        add += &plural_lines(v, &" ".repeat(indent + 2), Style::Double);
    } else if v.contains('\n') {
        add += &block(&format!("{pad}{last}: "), v, indent + 2);
    } else {
        add += &format!("{pad}{last}: {}\n", scalar(v, Style::Plain));
    }
    let mut lines: Vec<String> = doc.lines.iter().map(|l| l.to_string()).collect();
    if lines.last().is_some_and(|l| !l.ends_with('\n')) {
        lines.last_mut().unwrap().push('\n');
    }
    lines.insert(at.min(lines.len()), add);
    lines.concat()
}

fn plural_lines(v: &str, ind: &str, style: Style) -> String {
    let forms = folded_choice(v)
        .map(|c| c.forms)
        .unwrap_or_else(|| vec![("other".into(), v.into())]);
    forms
        .iter()
        .filter(|(k, _)| CATS.contains(&k.as_str()))
        .map(|(k, t)| format!("{ind}{k}: {}\n", scalar(t, style)))
        .collect()
}

/// `prefix|` block scalar (keeps the original `key: ` prefix).
fn block(prefix: &str, v: &str, indent: usize) -> String {
    let head = if v.ends_with('\n') { "|" } else { "|-" };
    let pad = " ".repeat(indent);
    let mut out = format!(
        "{}{head}\n",
        prefix
            .trim_end_matches(['|', '>', '-', '+', ' '])
            .to_string()
            + " "
    );
    for l in v.strip_suffix('\n').unwrap_or(v).split('\n') {
        out += &if l.is_empty() {
            "\n".to_string()
        } else {
            format!("{pad}{l}\n")
        };
    }
    out
}

fn scalar(v: &str, style: Style) -> String {
    match style {
        Style::Single if !v.contains('\n') => format!("'{}'", v.replace('\'', "''")),
        Style::Plain if plain_ok(v) => v.to_string(),
        _ => format!(
            "\"{}\"",
            v.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
        ),
    }
}

fn plain_ok(v: &str) -> bool {
    !v.is_empty()
        && v.trim() == v
        && !v.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.ends_with(':')
        && !v.contains(['\n', '\t'])
        && !is_special(v)
        && v.parse::<f64>().is_err()
}

fn is_special(v: &str) -> bool {
    matches!(
        v.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "y" | "n" | "null" | "~"
    )
}

fn yaml_key(k: &str) -> String {
    if !k.is_empty()
        && k.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !is_special(k)
    {
        k.to_string()
    } else {
        format!("\"{}\"", k.replace('\\', "\\\\").replace('"', "\\\""))
    }
}
