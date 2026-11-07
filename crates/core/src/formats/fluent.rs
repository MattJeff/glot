//! Project Fluent `.ftl` (fluent-rs / fluent-templates / i18n-embed / dioxus-i18n, Firefox,
//! python-fluent, `@fluent/bundle`).
//!
//! `id = value`, terms `-brand = value`, attributes `    .attr = value` (glot key `id.attr`: Fluent
//! identifiers never contain a dot), multiline patterns (indented continuation lines, dedented like
//! Fluent does). Placeables and select expressions stay Fluent text (`{ $n -> [one] … *[other] … }`),
//! validated by `Syntax::Fluent`. A `#` comment right above a message is its note; `##` / `###`
//! comments, blank lines and order are kept.
//! Writing patches values in place: untouched messages keep their bytes, changed ones are
//! re-indented with 4 spaces (8 for attributes), new messages and attributes are appended.
// ponytail: a line in a translated value starting with `[`, `*` or `.` would need `{"["}`-style
// escaping to stay text; translators never produce that in practice.
use std::collections::{BTreeMap, BTreeSet};

use super::Parsed;
use crate::catalog::Catalog;

/// One value (message value or attribute) in the file.
struct Span {
    key: String,
    /// Right after `=`, and the end of the last value line (before its newline).
    start: usize,
    end: usize,
    value: String,
}

struct Message {
    id: String,
    /// Start of the attached `#` comment (else of the id line), end after the last line's newline.
    start: usize,
    end: usize,
    value: Option<Span>,
    /// Attribute and the byte range of its lines (newline included).
    attrs: Vec<(Span, usize, usize)>,
    note: Option<String>,
}

/// `(start, end)` of every line, `end` before `\r\n` / `\n`.
fn lines(s: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut at = 0;
    for l in s.split_inclusive('\n') {
        let body = l.trim_end_matches('\n').trim_end_matches('\r');
        out.push((at, at + body.len()));
        at += l.len();
    }
    out
}

fn is_id(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|c| c.is_ascii_alphabetic())
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `id =` / `-term =` at column 0: the id and the byte offset right after `=`.
fn id_eq(line: &str) -> Option<(&str, usize)> {
    let eq = line.find('=')?;
    let id = line[..eq].trim_end();
    let bare = id.strip_prefix('-').unwrap_or(id);
    (is_id(bare) && !line.starts_with(char::is_whitespace)).then_some((id, eq + 1))
}

/// `    .attr =` (after indentation).
fn attr_eq(line: &str) -> Option<(&str, usize)> {
    let t = line.trim_start();
    if t.len() == line.len() {
        return None;
    }
    let rest = t.strip_prefix('.')?;
    let eq = rest.find('=')?;
    let name = rest[..eq].trim_end();
    is_id(name).then_some((name, line.len() - rest.len() + eq + 1))
}

/// Brace depth after `line`, starting at `depth` (string literals inside placeables skipped).
fn depth_after(line: &str, mut depth: i32) -> i32 {
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => depth += 1,
            '}' => depth = (depth - 1).max(0),
            '"' if depth > 0 => {
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    depth
}

/// Fluent pattern semantics: the inline first line, then continuation lines without their common
/// indentation; blank lines inside are kept, leading / trailing blanks dropped.
fn dedent(raw: &str) -> String {
    let mut it = raw.split('\n').map(|l| l.trim_end_matches('\r'));
    let first = it.next().unwrap_or_default().trim();
    let rest: Vec<&str> = it.collect();
    let indent = rest
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out: Vec<&str> = vec![];
    if !first.is_empty() {
        out.push(first);
    }
    for l in rest {
        if l.trim().is_empty() {
            if !out.is_empty() {
                out.push("");
            }
        } else {
            out.push(l[indent..].trim_end());
        }
    }
    while out.last() == Some(&"") {
        out.pop();
    }
    out.join("\n")
}

/// Text to put right after `=` for `v`, continuation lines indented with `ind`.
fn render(v: &str, ind: &str) -> String {
    if v.is_empty() {
        return String::new();
    }
    let ls: Vec<&str> = v.split('\n').collect();
    let line = |l: &&str| {
        if l.is_empty() {
            String::new()
        } else {
            format!("{ind}{l}")
        }
    };
    // Inline first line only when the rest is not all indented (Fluent would dedent it away).
    let inline = ls.len() == 1 || ls[1..].iter().any(|l| !l.is_empty() && !l.starts_with(' '));
    let rest: Vec<String> = ls[1..].iter().map(line).collect();
    if inline {
        std::iter::once(format!(" {}", ls[0]))
            .chain(rest)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        std::iter::once(String::new())
            .chain(ls.iter().map(line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn messages(s: &str) -> Vec<Message> {
    let ls = lines(s);
    let text = |i: usize| &s[ls[i].0..ls[i].1];
    let after = |i: usize| ls.get(i + 1).map_or(s.len(), |l| l.0);
    let (mut out, mut i) = (vec![], 0);
    let mut comment: Option<(usize, Vec<String>)> = None;
    while i < ls.len() {
        let line = text(i);
        if line == "#" || line.starts_with("# ") {
            let c = comment.get_or_insert((ls[i].0, vec![]));
            c.1.push(line[1..].trim().to_string());
            i += 1;
            continue;
        }
        let Some((id, eq)) = id_eq(line) else {
            comment = None;
            i += 1;
            continue;
        };
        // The message: indented (or blank) lines after it, and any line inside an open placeable.
        let mut depth = depth_after(&line[eq..], 0);
        let (mut last, mut j) = (i, i + 1);
        while j < ls.len() {
            let l = text(j);
            if depth == 0 && !l.trim().is_empty() && !l.starts_with([' ', '\t']) {
                break;
            }
            depth = depth_after(l, depth);
            if !l.trim().is_empty() {
                last = j;
            }
            j += 1;
        }
        // Attributes start at depth-0 `.name =` lines.
        let (mut parts, mut d) = (vec![(None, i, ls[i].0 + eq)], depth_after(&line[eq..], 0));
        for (k, &(start, _)) in ls.iter().enumerate().take(last + 1).skip(i + 1) {
            let l = text(k);
            if d == 0
                && let Some((name, at)) = attr_eq(l)
            {
                parts.push((Some(name.to_string()), k, start + at));
            }
            d = depth_after(l, d);
        }
        let c = comment.take();
        let mut m = Message {
            id: id.to_string(),
            start: c.as_ref().map_or(ls[i].0, |c| c.0),
            end: after(last),
            value: None,
            attrs: vec![],
            note: c.map(|c| c.1.join("\n")),
        };
        for (n, (name, line_no, start)) in parts.iter().enumerate() {
            let end_line = parts.get(n + 1).map_or(last, |p| p.1 - 1);
            // Trailing blank lines belong to nobody.
            let end_line = (*line_no..=end_line)
                .rev()
                .find(|&k| k == *line_no || !text(k).trim().is_empty())
                .unwrap_or(*line_no);
            let end = ls[end_line].1.max(*start);
            let value = dedent(&s[*start..end]);
            match name {
                None if value.is_empty() => {}
                None => {
                    m.value = Some(Span {
                        key: m.id.clone(),
                        start: *start,
                        end,
                        value,
                    })
                }
                Some(a) => m.attrs.push((
                    Span {
                        key: format!("{}.{a}", m.id),
                        start: *start,
                        end,
                        value,
                    },
                    ls[*line_no].0,
                    after(end_line),
                )),
            }
        }
        out.push(m);
        i = last + 1;
    }
    out
}

pub fn parse(s: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    for m in messages(s) {
        for span in m.value.iter().chain(m.attrs.iter().map(|a| &a.0)) {
            if let Some(n) = &m.note {
                p.notes.insert(span.key.clone(), n.clone());
            }
            p.entries.insert(span.key.clone(), span.value.clone());
        }
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or("");
    let list = messages(s);
    // (start, end, replacement), applied in order.
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let mut known: BTreeSet<&str> = BTreeSet::new();
    let ids: BTreeMap<&str, &Message> = list.iter().map(|m| (m.id.as_str(), m)).collect();
    for m in &list {
        let keys: Vec<&Span> = m.value.iter().chain(m.attrs.iter().map(|a| &a.0)).collect();
        known.extend(keys.iter().map(|k| k.key.as_str()));
        let has_new_attr = cat
            .entries
            .keys()
            .any(|k| k.split_once('.').is_some_and(|(id, _)| id == m.id));
        if !keys.is_empty()
            && !has_new_attr
            && !keys.iter().any(|k| cat.entries.contains_key(&k.key))
        {
            edits.push((m.start, m.end, String::new()));
            continue;
        }
        if let Some(v) = &m.value {
            match cat.entries.get(&v.key) {
                Some(new) if *new == v.value => {}
                Some(new) => edits.push((v.start, v.end, render(new, "    "))),
                None => edits.push((v.start, v.end, String::new())),
            }
        }
        for (a, ls, le) in &m.attrs {
            match cat.entries.get(&a.key) {
                Some(new) if *new == a.value => {}
                Some(new) => edits.push((a.start, a.end, render(new, "        "))),
                None => edits.push((*ls, *le, String::new())),
            }
        }
    }
    // New attributes of existing messages go after their message.
    let mut fresh: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for (k, v) in cat
        .entries
        .iter()
        .filter(|(k, _)| !known.contains(k.as_str()))
    {
        let (id, attr) = k.split_once('.').unwrap_or((k, ""));
        match ids.get(id) {
            Some(m) if !attr.is_empty() => {
                let nl = if s[..m.end].ends_with('\n') { "" } else { "\n" };
                edits.push((
                    m.end,
                    m.end,
                    format!("{nl}    .{attr} ={}\n", render(v, "        ")),
                ));
            }
            _ => fresh.entry(id).or_default().push((attr, v)),
        }
    }
    edits.sort_by_key(|e| (e.0, e.1));
    let (mut out, mut pos) = (String::new(), 0);
    for (a, b, text) in edits {
        if a < pos {
            continue;
        }
        out += &s[pos..a];
        out += &text;
        pos = b;
    }
    out += &s[pos..];
    for (id, parts) in fresh {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let value = parts.iter().find(|(a, _)| a.is_empty()).map(|(_, v)| *v);
        out += &format!(
            "{id} ={}\n",
            value.map_or(String::new(), |v| render(v, "    "))
        );
        for (a, v) in parts.iter().filter(|(a, _)| !a.is_empty()) {
            out += &format!("    .{a} ={}\n", render(v, "        "));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/main.ftl"
    ));

    fn cat(p: &Parsed) -> Catalog {
        Catalog {
            locale: "fr".into(),
            entries: p.entries.clone(),
        }
    }

    #[test]
    fn parse_fixture() {
        let p = parse(FIX).unwrap();
        let e = &p.entries;
        assert_eq!(e["hello-world"], "Bonjour le monde !");
        assert_eq!(
            e["greeting"],
            "Bonjour { $name }, bienvenue sur { -brand-name }."
        );
        assert_eq!(e["-brand-name"], "Cliponit");
        assert_eq!(e["-brand-name.gender"], "masculine");
        assert_eq!(
            e["unread-emails"],
            "{ $count ->\n    [one] Vous avez un e-mail non lu.\n   *[other] Vous avez { $count } e-mails non lus.\n}"
        );
        assert_eq!(e["login-input"], "Valeur prédéfinie");
        assert_eq!(e["login-input.placeholder"], "email@exemple.com");
        assert_eq!(e["login-input.aria-label"], "Champ de connexion");
        assert_eq!(e["multiline"], "Première ligne,\ndeuxième ligne.");
        assert!(e["shared-photos"].starts_with("{ $userName } { $photoCount ->\n  [one]"));
        assert!(
            e["shared-photos"].ends_with("\n}."),
            "{}",
            e["shared-photos"]
        );
        assert_eq!(e["literal"], "Accolade { \"{\" } littérale");
        assert_eq!(e["only-attrs.title"], "Infobulle");
        assert!(!e.contains_key("only-attrs"));
        assert_eq!(p.notes["hello-world"], "Titre de la page d'accueil");
        assert_eq!(p.notes["unread-emails"], "Nombre d'e-mails non lus");
        assert!(!p.notes.contains_key("greeting"));
        assert_eq!(e.len(), 13);
    }

    #[test]
    fn roundtrip_and_edits() {
        let p = parse(FIX).unwrap();
        assert_eq!(write(Some(FIX), &cat(&p)), FIX, "byte for byte");
        let mut c = cat(&p);
        c.entries.insert("hello-world".into(), "Salut !".into());
        c.entries.insert(
            "unread-emails".into(),
            "{ $count ->\n    [one] Un e-mail.\n   *[other] { $count } e-mails.\n}".into(),
        );
        c.entries.insert("multiline".into(), "Une\n  deux".into());
        c.entries.remove("login-input.aria-label");
        c.entries.remove("price");
        c.entries.insert("login-input.title".into(), "Titre".into());
        c.entries.insert("new-msg".into(), "Nouveau { $x }".into());
        c.entries.insert("new-msg.alt".into(), "Alt".into());
        let out = write(Some(FIX), &c);
        assert!(out.contains("hello-world = Salut !\n"), "{out}");
        assert!(out.contains(
            "unread-emails = { $count ->\n        [one] Un e-mail.\n       *[other] { $count } e-mails.\n    }\n"
        ), "{out}");
        assert!(out.contains("multiline =\n    Une\n      deux\n"), "{out}");
        assert!(!out.contains("aria-label"));
        assert!(!out.contains("NUMBER"));
        assert!(
            out.contains("    .placeholder = email@exemple.com\n    .title = Titre\n"),
            "{out}"
        );
        assert!(
            out.ends_with("new-msg = Nouveau { $x }\n    .alt = Alt\n"),
            "{out}"
        );
        assert_eq!(parse(&out).unwrap().entries, c.entries);
        // Untouched messages keep their bytes.
        assert!(out.contains("shared-photos = { $userName } { $photoCount ->\n    [one]"));
    }

    #[test]
    fn new_file_and_crlf() {
        let mut c = Catalog::new("en");
        c.entries.insert("a".into(), "A".into());
        c.entries.insert("b.title".into(), "B".into());
        let w = write(None, &c);
        assert_eq!(w, "a = A\nb =\n    .title = B\n");
        assert_eq!(parse(&w).unwrap().entries, c.entries);
        let crlf = "a = A\r\nb = { $n ->\r\n    [one] x\r\n   *[other] y\r\n}\r\n";
        let p = parse(crlf).unwrap();
        assert_eq!(p.entries["a"], "A");
        assert_eq!(p.entries["b"], "{ $n ->\n    [one] x\n   *[other] y\n}");
        assert_eq!(write(Some(crlf), &cat(&p)), crlf);
    }
}
