//! Hand-rolled TS/JS dictionaries: `locales/fr.ts` = `export const fr = { nav: { home: "Accueil" } }`
//! (nested or flat `"nav.home"` keys), `export default {...}`, `module.exports = {...}`, and inline
//! i18next resources keyed by locale (`const resources = { en: { translation: {...} }, fr: {...} }`).
//!
//! The catalog is the first object literal assigned at top level (`= {` / `export default {`).
//! String values (quotes, backticks without `${}`) are messages; anything else (functions, arrays,
//! spreads, computed keys) is kept verbatim and never translated. Writing patches the file: changed
//! values are re-quoted in place, removed keys dropped, new keys appended to their parent object.
//! The root object is located with oxc (so `.tsx` contexts with JSX, generics and `as const` parse);
//! inside it a hand scanner keeps byte offsets for patching.
//! ponytail: the hand scanner is not a JS parser: a regex literal holding a quote or brace inside
//! the dictionary object breaks it (never seen in a dictionary file).
use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, Expression, ObjectPropertyKind, Statement};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::Parsed;
use crate::catalog::Catalog;

#[derive(Debug)]
struct Obj {
    close: usize,
    props: Vec<Prop>,
}

#[derive(Debug)]
struct Prop {
    key: String,
    /// Key start and value end (comma excluded).
    start: usize,
    end: usize,
    comma: bool,
    val: Val,
}

#[derive(Debug)]
enum Val {
    Str {
        text: String,
        q: u8,
        span: (usize, usize),
    },
    Obj(Obj),
    Other,
}

struct Scan<'a> {
    s: &'a [u8],
}

impl Scan<'_> {
    fn ws(&self, mut i: usize) -> usize {
        let s = self.s;
        while i < s.len() {
            match s[i] {
                b' ' | b'\t' | b'\n' | b'\r' => i += 1,
                b'/' if s.get(i + 1) == Some(&b'/') => {
                    while i < s.len() && s[i] != b'\n' {
                        i += 1;
                    }
                }
                b'/' if s.get(i + 1) == Some(&b'*') => {
                    i += 2;
                    while i + 1 < s.len() && !(s[i] == b'*' && s[i + 1] == b'/') {
                        i += 1;
                    }
                    i = (i + 2).min(s.len());
                }
                _ => break,
            }
        }
        i
    }

    /// String at `i` (quote char there): (decoded text, end after the closing quote). `None` text
    /// for a template literal with `${}`.
    fn string(&self, mut i: usize) -> (Option<String>, usize) {
        let s = self.s;
        let q = s[i];
        i += 1;
        let mut out: Vec<u8> = vec![];
        let mut plain = true;
        while i < s.len() && s[i] != q {
            match s[i] {
                b'\\' if i + 1 < s.len() => {
                    i += 1;
                    match s[i] {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'0' => out.push(0),
                        b'\n' => {}
                        b'u' | b'x' => {
                            let (hex, len) = if s[i] == b'x' {
                                (
                                    std::str::from_utf8(&s[i + 1..(i + 3).min(s.len())])
                                        .unwrap_or(""),
                                    2,
                                )
                            } else if s.get(i + 1) == Some(&b'{') {
                                let end =
                                    s[i..].iter().position(|&c| c == b'}').map_or(i, |p| i + p);
                                let h = std::str::from_utf8(&s[i + 2..end]).unwrap_or("");
                                (h, end - i - 1)
                            } else {
                                (
                                    std::str::from_utf8(&s[i + 1..(i + 5).min(s.len())])
                                        .unwrap_or(""),
                                    4,
                                )
                            };
                            let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                            out.extend(c.unwrap_or('\u{FFFD}').to_string().bytes());
                            i += len;
                        }
                        c => out.push(c),
                    }
                    i += 1;
                }
                b'$' if q == b'`' && s.get(i + 1) == Some(&b'{') => {
                    plain = false;
                    i = self.balanced(i + 1);
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        let text = plain.then(|| String::from_utf8_lossy(&out).into_owned());
        (text, (i + 1).min(s.len()))
    }

    /// `i` on an opening bracket: index after its match.
    fn balanced(&self, mut i: usize) -> usize {
        let mut depth = 0usize;
        while i < self.s.len() {
            match self.s[i] {
                b'\'' | b'"' | b'`' => {
                    i = self.string(i).1;
                    continue;
                }
                b'/' if matches!(self.s.get(i + 1).copied(), Some(b'/' | b'*')) => {
                    i = self.ws(i);
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        i
    }

    /// End of a value expression: the `,` or closing bracket ending it (exclusive).
    fn value_end(&self, mut i: usize) -> usize {
        while i < self.s.len() {
            match self.s[i] {
                b'\'' | b'"' | b'`' => i = self.string(i).1,
                b'/' if matches!(self.s.get(i + 1).copied(), Some(b'/' | b'*')) => i = self.ws(i),
                b'(' | b'[' | b'{' => i = self.balanced(i),
                b',' | b')' | b']' | b'}' => return i,
                _ => i += 1,
            }
        }
        i
    }

    fn object(&self, open: usize) -> Obj {
        let s = self.s;
        let mut props = vec![];
        let mut i = open + 1;
        loop {
            i = self.ws(i);
            if i >= s.len() || s[i] == b'}' {
                break;
            }
            let start = i;
            let key = match s[i] {
                b'\'' | b'"' => {
                    let (k, end) = self.string(i);
                    i = end;
                    k
                }
                c if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' => {
                    while i < s.len()
                        && (s[i].is_ascii_alphanumeric() || s[i] == b'_' || s[i] == b'$')
                    {
                        i += 1;
                    }
                    Some(String::from_utf8_lossy(&s[start..i]).into_owned())
                }
                _ => None, // spread, computed key
            };
            let j = self.ws(i);
            let mut val = Val::Other;
            let end;
            if key.is_some() && s.get(j) == Some(&b':') {
                let v = self.ws(j + 1);
                let after = match s.get(v).copied() {
                    Some(b'\'' | b'"' | b'`') => {
                        let (text, e) = self.string(v);
                        if let Some(text) = text {
                            val = Val::Str {
                                text,
                                q: s[v],
                                span: (v, e),
                            };
                        }
                        e
                    }
                    Some(b'{') => {
                        let o = self.object(v);
                        let e = o.close + 1;
                        val = Val::Obj(o);
                        e
                    }
                    _ => v,
                };
                let k = self.ws(after);
                if matches!(s.get(k).copied(), Some(b',' | b'}') | None) {
                    end = after;
                } else {
                    // `"a" + b`, `"x" as const`, `t("..")`: not a plain message.
                    val = Val::Other;
                    end = self.value_end(v);
                }
            } else {
                end = self.value_end(i);
            }
            i = self.ws(end);
            let comma = s.get(i) == Some(&b',');
            if comma {
                i += 1;
            }
            if let Some(key) = key {
                props.push(Prop {
                    key,
                    start,
                    end,
                    comma,
                    val,
                });
            }
            if i >= s.len() || !comma && s.get(i) != Some(&b'}') {
                i = self.value_end(i);
                if s.get(i) != Some(&b',') {
                    break;
                }
                i += 1;
            }
        }
        Obj {
            close: i.min(s.len()),
            props,
        }
    }

    /// Opening brace of the catalog: the top-level object keyed by locales (oxc), else the first
    /// `export default {` / `= {` at top level (not a type).
    fn root(&self) -> Option<usize> {
        oxc_root(std::str::from_utf8(self.s).ok()?).or_else(|| self.hand_root())
    }

    fn hand_root(&self) -> Option<usize> {
        let s = self.s;
        let mut i = 0;
        while i < s.len() {
            i = self.ws(i);
            if i >= s.len() {
                break;
            }
            match s[i] {
                b'\'' | b'"' | b'`' => i = self.string(i).1,
                b'(' | b'[' | b'{' => i = self.balanced(i),
                b'=' if !matches!(s.get(i + 1).copied(), Some(b'=' | b'>'))
                    && !matches!(s.get(i.wrapping_sub(1)), Some(b'=' | b'!' | b'<' | b'>')) =>
                {
                    let j = self.ws(i + 1);
                    let line = &s[s[..i]
                        .iter()
                        .rposition(|&c| c == b'\n')
                        .map_or(0, |p| p + 1)..i];
                    let line = String::from_utf8_lossy(line);
                    let line = line.trim_start();
                    if s.get(j) == Some(&b'{')
                        && !line.starts_with("type ")
                        && !line.starts_with("export type ")
                    {
                        return Some(j);
                    }
                    i += 1;
                }
                b'd' if s[i..].starts_with(b"default") && s[..i].ends_with(b"export ") => {
                    let j = self.ws(i + 7);
                    if s.get(j) == Some(&b'{') {
                        return Some(j);
                    }
                    i += 7;
                }
                _ => i += 1,
            }
        }
        None
    }
}

/// Top-level object literals (`const x = {`, `export default {`, `module.exports = {`, through
/// `as const` / `satisfies` / type annotations): the first keyed by 2+ locales, else the first.
/// `None` when the file does not parse as TSX, TS or JSX.
fn oxc_root(text: &str) -> Option<usize> {
    for st in [SourceType::tsx(), SourceType::ts(), SourceType::jsx()] {
        let alloc = Allocator::default();
        let r = Parser::new(&alloc, text, st).parse();
        if r.diagnostics.has_errors() {
            continue;
        }
        let mut objs = vec![];
        let mut push = |e: &Expression| {
            if let Expression::ObjectExpression(o) = e.get_inner_expression() {
                objs.push(o.span.start as usize);
                let keyed = o.properties.len() >= 2
                    && o.properties.iter().all(|p| match p {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            p.key.static_name().is_some_and(|k| locale_like(&k))
                                && matches!(
                                    p.value.get_inner_expression(),
                                    Expression::ObjectExpression(_)
                                )
                        }
                        _ => false,
                    });
                if keyed {
                    return Some(o.span.start as usize);
                }
            }
            None
        };
        for stmt in &r.program.body {
            let found = match stmt {
                Statement::VariableDeclaration(d) => {
                    d.declarations.iter().find_map(|x| push(x.init.as_ref()?))
                }
                Statement::ExportDeclaration(e) => match &e.declaration {
                    Declaration::VariableDeclaration(d) => {
                        d.declarations.iter().find_map(|x| push(x.init.as_ref()?))
                    }
                    _ => None,
                },
                Statement::ExportDefaultDeclaration(e) => {
                    e.declaration.as_expression().and_then(&mut push)
                }
                Statement::ExpressionStatement(e) => match &e.expression {
                    Expression::AssignmentExpression(a) => push(&a.right),
                    _ => None,
                },
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        return objs.first().copied();
    }
    None
}

/// Locales of a dictionary keyed by locale at its root (`{ en: {...}, fr: {...} }`), in file order.
pub fn dictionary_locales(text: &str) -> Vec<String> {
    let sc = Scan { s: text.as_bytes() };
    let Some(open) = sc.root() else { return vec![] };
    let root = sc.object(open);
    match locale_base(&root, "\0") {
        Some(_) => root.props.iter().map(|p| p.key.clone()).collect(),
        None => vec![],
    }
}

/// `'en' | 'fr'` unions and `['en', 'fr']` lists naming exactly the dictionary's locales: edits
/// appending `new`, so `type Language` and the language list follow a new locale.
/// ponytail: string literals found by a byte scan; JSX text with an apostrophe can shift it, which
/// at worst misses a list (a false match needs the exact locale set).
fn locale_lists(s: &str, have: &[String], new: &str) -> Vec<(usize, usize, String)> {
    let sc = Scan { s: s.as_bytes() };
    let b = s.as_bytes();
    let mut lits: Vec<(usize, usize, String)> = vec![];
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if matches!(b.get(i + 1).copied(), Some(b'/' | b'*')) => i = sc.ws(i),
            b'\'' | b'"' | b'`' => {
                let (t, e) = sc.string(i);
                if let Some(t) = t {
                    lits.push((i, e, t));
                }
                i = e.max(i + 1);
            }
            _ => i += 1,
        }
    }
    let norm = |v: &[&str]| -> BTreeSet<String> {
        v.iter()
            .map(|l| l.replace('_', "-").to_ascii_lowercase())
            .collect()
    };
    let want = norm(&have.iter().map(String::as_str).collect::<Vec<_>>());
    let mut out = vec![];
    let mut k = 0;
    while k < lits.len() {
        let mut j = k;
        while j + 1 < lits.len() && matches!(s[lits[j].1..lits[j + 1].0].trim(), "|" | ",") {
            j += 1;
        }
        let group = &lits[k..=j];
        let texts: Vec<&str> = group.iter().map(|l| l.2.as_str()).collect();
        if group.len() >= 2 && texts.iter().all(|t| locale_like(t)) && norm(&texts) == want {
            let (prev, last) = (&group[group.len() - 2], &group[group.len() - 1]);
            let q = b[last.0] as char;
            out.push((last.1, last.1, format!("{}{q}{new}{q}", &s[prev.1..last.0])));
        }
        k = j + 1;
    }
    out
}

fn same_locale(a: &str, b: &str) -> bool {
    a.replace('_', "-")
        .eq_ignore_ascii_case(&b.replace('_', "-"))
}

/// `en`, `fr`, `pt_BR`, `enUS` style keys of a resources object.
fn locale_like(k: &str) -> bool {
    let k = k.replace('_', "-");
    let (l, r) = k.split_once('-').unwrap_or((&k, ""));
    (l.len() == 2 || l.len() == 3)
        && l.bytes().all(|b| b.is_ascii_lowercase())
        && (r.is_empty() || (2..=4).contains(&r.len()))
}

/// The locale's object inside an i18next-style resources root (`{ en: { translation: {...} } }`),
/// plus the namespace wrapper the locales use. `Err(wrapper)`: locale-keyed root, locale absent.
fn locale_base<'o>(root: &'o Obj, locale: &str) -> Option<Result<&'o Obj, Option<String>>> {
    let keyed = !root.props.is_empty()
        && root
            .props
            .iter()
            .all(|p| locale_like(&p.key) && matches!(p.val, Val::Obj(_)));
    if !keyed {
        return None;
    }
    let unwrap = |o: &'o Obj| match o.props.as_slice() {
        [p] if p.key == "translation" => match &p.val {
            Val::Obj(inner) => (inner, Some(p.key.clone())),
            _ => (o, None),
        },
        _ => (o, None),
    };
    let found = root.props.iter().find(|p| same_locale(&p.key, locale));
    Some(match found.map(|p| &p.val) {
        Some(Val::Obj(o)) => Ok(unwrap(o).0),
        _ => Err(root.props.iter().find_map(|p| match &p.val {
            Val::Obj(o) => unwrap(o).1,
            _ => None,
        })),
    })
}

fn leaves<'o>(o: &'o Obj, prefix: &str, out: &mut Vec<(String, &'o Prop)>) {
    for p in &o.props {
        let key = if prefix.is_empty() {
            p.key.clone()
        } else {
            format!("{prefix}.{}", p.key)
        };
        match &p.val {
            Val::Obj(inner) => leaves(inner, &key, out),
            _ => out.push((key, p)),
        }
    }
}

pub fn parse(text: &str, locale: &str) -> Result<Parsed, String> {
    let sc = Scan { s: text.as_bytes() };
    let open = sc
        .root()
        .ok_or("no object literal (`export default {` / `= {`) found")?;
    let root = sc.object(open);
    let base = match locale_base(&root, locale) {
        None => &root,
        Some(Ok(o)) => o,
        Some(Err(_)) => return Ok(Parsed::default()),
    };
    let mut ls = vec![];
    leaves(base, "", &mut ls);
    let entries = ls
        .into_iter()
        .filter_map(|(k, p)| match &p.val {
            Val::Str { text, .. } => Some((k, text.clone())),
            _ => None,
        })
        .collect();
    Ok(Parsed {
        entries,
        ..Default::default()
    })
}

fn quote(v: &str, q: u8) -> String {
    let q = q as char;
    let mut out = String::from(q);
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' if q != '`' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' if q != '`' => out.push_str("\\t"),
            '$' if q == '`' => out.push_str("\\$"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

fn ident(k: &str) -> bool {
    k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[derive(Default)]
struct Tree(BTreeMap<String, Node>);
enum Node {
    Leaf(String),
    Branch(Tree),
}

impl Tree {
    fn insert(&mut self, segs: &[String], v: &str) {
        match segs {
            [] => {}
            [k] => {
                self.0.insert(k.clone(), Node::Leaf(v.into()));
            }
            [k, rest @ ..] => {
                let n = self
                    .0
                    .entry(k.clone())
                    .or_insert_with(|| Node::Branch(Tree::default()));
                if let Node::Branch(t) = n {
                    t.insert(rest, v);
                }
            }
        }
    }

    fn render(&self, out: &mut String, indent: &str, step: &str, st: &Style) {
        for (k, n) in &self.0 {
            let key = if ident(k) && !st.quoted_keys {
                k.clone()
            } else {
                quote(k, st.q)
            };
            match n {
                Node::Leaf(v) => out.push_str(&format!("{indent}{key}: {},\n", quote(v, st.q))),
                Node::Branch(t) => {
                    out.push_str(&format!("{indent}{key}: {{\n"));
                    t.render(out, &format!("{indent}{step}"), step, st);
                    out.push_str(&format!("{indent}}},\n"));
                }
            }
        }
    }
}

impl Tree {
    fn inline(&self, st: &Style) -> String {
        self.0
            .iter()
            .map(|(k, n)| {
                let key = if ident(k) && !st.quoted_keys {
                    k.clone()
                } else {
                    quote(k, st.q)
                };
                match n {
                    Node::Leaf(v) => format!("{key}: {}", quote(v, st.q)),
                    Node::Branch(t) => format!("{key}: {{ {} }}", t.inline(st)),
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

struct Style {
    q: u8,
    quoted_keys: bool,
}

fn line_start(s: &str, i: usize) -> usize {
    s[..i].rfind('\n').map_or(0, |p| p + 1)
}

fn indent_of(s: &str, i: usize) -> &str {
    let ls = line_start(s, i);
    let line = &s[ls..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Edits adding `tree` to `o` (before its closing brace).
fn append(
    s: &str,
    o: &Obj,
    tree: &Tree,
    parent_indent: &str,
    st: &Style,
    dropped: &BTreeSet<usize>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    let close_indent = indent_of(s, o.close);
    let alone = s[line_start(s, o.close)..o.close].trim().is_empty();
    let indent = match o.props.first() {
        Some(p) if s[line_start(s, p.start)..p.start].trim().is_empty() => {
            indent_of(s, p.start).to_string()
        }
        _ if alone => format!("{close_indent}  "),
        _ => format!("{parent_indent}  "),
    };
    let step = indent
        .strip_prefix(close_indent)
        .filter(|x| !x.is_empty())
        .unwrap_or("  ")
        .to_string();
    let last = o.props.iter().rev().find(|p| !dropped.contains(&p.start));
    // One-line object (`en: { 'a': 'A', 'b': 'B' },`): new entries stay on that line.
    if let Some(last) = last.filter(|l| line_start(s, l.start) == line_start(s, o.close)) {
        edits.push((last.end, last.end, format!(", {}", tree.inline(st))));
        return;
    }
    if let Some(last) = last
        && !last.comma
    {
        edits.push((last.end, last.end, ",".into()));
    }
    let mut body = String::new();
    tree.render(&mut body, &indent, &step, st);
    if alone {
        edits.push((line_start(s, o.close), line_start(s, o.close), body));
    } else {
        edits.push((o.close, o.close, format!("\n{body}{close_indent}")));
    }
}

/// Deepest existing object for `segs`: (object, its indent, remaining segments).
fn target<'o>(o: &'o Obj, segs: &[String]) -> (&'o Obj, usize) {
    if segs.len() > 1
        && let Some(Prop {
            val: Val::Obj(inner),
            ..
        }) = o.props.iter().find(|p| p.key == segs[0])
    {
        let (t, n) = target(inner, &segs[1..]);
        return (t, n + 1);
    }
    (o, 0)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let fresh = || {
        let mut out = String::from("export default ");
        out.push_str(&cat.to_json(crate::catalog::Format::JsonNested));
        out.insert(out.len() - 1, ';');
        out
    };
    let Some(s) = existing else { return fresh() };
    let sc = Scan { s: s.as_bytes() };
    let Some(open) = sc.root() else {
        return fresh();
    };
    let root = sc.object(open);
    let mut ls = vec![];
    leaves(&root, "", &mut ls);
    let q = ls
        .iter()
        .filter_map(|(_, p)| match p.val {
            Val::Str { q, .. } if q != b'`' => Some(q),
            _ => None,
        })
        .fold(
            (0, 0),
            |(a, b), q| if q == b'\'' { (a + 1, b) } else { (a, b + 1) },
        );
    let st = Style {
        q: if q.0 > q.1 { b'\'' } else { b'"' },
        quoted_keys: false,
    };
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let base = match locale_base(&root, &cat.locale) {
        None => &root,
        Some(Ok(o)) => o,
        Some(Err(wrapper)) => {
            // New locale in a resources object: `fr: { translation: {...} },`.
            let mut tree = Tree::default();
            // Flat `'nav.home'` keys in the other locales: the new one is flat too.
            let flat = root.props.iter().any(|p| match &p.val {
                Val::Obj(o) => {
                    let o = match (&wrapper, o.props.as_slice()) {
                        (Some(w), [q]) if &q.key == w => match &q.val {
                            Val::Obj(i) => i,
                            _ => o,
                        },
                        _ => o,
                    };
                    o.props.iter().any(|q| q.key.contains('.'))
                }
                _ => false,
            });
            for (k, v) in &cat.entries {
                let key: Vec<String> = if flat {
                    vec![k.clone()]
                } else {
                    k.split('.').map(String::from).collect()
                };
                let segs: Vec<String> = std::iter::once(cat.locale.clone())
                    .chain(wrapper.clone())
                    .chain(key)
                    .collect();
                tree.insert(&segs, v);
            }
            append(s, &root, &tree, "", &st, &BTreeSet::new(), &mut edits);
            let have: Vec<String> = root.props.iter().map(|p| p.key.clone()).collect();
            edits.extend(locale_lists(s, &have, &cat.locale));
            return apply(s, edits);
        }
    };
    let mut ls = vec![];
    leaves(base, "", &mut ls);
    let mut seen = BTreeSet::new();
    let mut dropped = BTreeSet::new();
    for (k, p) in &ls {
        seen.insert(k.clone());
        let Val::Str { text, q, span } = &p.val else {
            continue;
        };
        match cat.entries.get(k) {
            Some(v) if v != text => edits.push((span.0, span.1, quote(v, *q))),
            Some(_) => {}
            None => {
                dropped.insert(p.start);
                let ls0 = line_start(s, p.start);
                let own_line = s[ls0..p.start].trim().is_empty();
                let mut b = p.end;
                let rest = &s.as_bytes()[b..];
                let mut n = rest
                    .iter()
                    .take_while(|c| matches!(c, b' ' | b'\t'))
                    .count();
                if rest.get(n) == Some(&b',') {
                    n += 1;
                    n += rest[n..]
                        .iter()
                        .take_while(|c| matches!(c, b' ' | b'\t'))
                        .count();
                }
                b += n;
                let (a, b) = if own_line && s.as_bytes().get(b) == Some(&b'\n') {
                    (ls0, b + 1)
                } else {
                    (p.start, b)
                };
                edits.push((a, b, String::new()));
            }
        }
    }
    // New keys, grouped by the deepest existing object holding them.
    let mut groups: BTreeMap<usize, (&Obj, Tree)> = BTreeMap::new();
    for (k, v) in cat.entries.iter().filter(|(k, _)| !seen.contains(*k)) {
        let segs: Vec<String> = k.split('.').map(String::from).collect();
        let (o, depth) = target(base, &segs);
        let flat = o.props.iter().any(|p| p.key.contains('.'));
        let rest = if flat {
            vec![segs[depth..].join(".")]
        } else {
            segs[depth..].to_vec()
        };
        groups
            .entry(o.close)
            .or_insert_with(|| (o, Tree::default()))
            .1
            .insert(&rest, v);
    }
    for (o, tree) in groups.values() {
        let quoted = o
            .props
            .iter()
            .any(|p| matches!(s.as_bytes()[p.start], b'"' | b'\''));
        let st = Style {
            quoted_keys: quoted,
            ..st
        };
        append(s, o, tree, indent_of(s, o.close), &st, &dropped, &mut edits);
    }
    apply(s, edits)
}

fn apply(s: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = s.to_string();
    for (a, b, t) in edits {
        out.replace_range(a..b, &t);
    }
    out
}
