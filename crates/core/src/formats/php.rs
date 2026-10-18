//! Laravel `lang/{locale}/*.php`: `<?php return ['key' => '…', 'nested' => [...]];`. Keys are
//! prefixed with the file name as Laravel resolves them (`lang/fr/auth.php` `failed` => `auth.failed`).
//!
//! A restricted scanner, not a PHP parser: only literal arrays (`[...]` / `array(...)`) of quoted
//! strings are accepted. Anything dynamic (concatenation, `"$var"`, function calls, constants,
//! heredoc) is refused with its line number rather than half-understood and rewritten.
//! Writing patches strings in place (same quote style), drops removed keys and appends new keys to
//! their parent array.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::Parsed;
use crate::catalog::Catalog;

struct Arr {
    /// The closing `]` / `)`.
    close: usize,
    items: Vec<Item>,
}

struct Item {
    key: String,
    /// Key start and value end.
    start: usize,
    end: usize,
    comma: bool,
    val: Val,
    note: Option<String>,
}

enum Val {
    Str {
        text: String,
        q: u8,
        span: (usize, usize),
    },
    Arr(Arr),
}

struct Scan<'a> {
    s: &'a [u8],
}

type R<T> = Result<T, String>;

impl Scan<'_> {
    fn err<T>(&self, i: usize, msg: &str) -> R<T> {
        let line = self.s[..i.min(self.s.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count()
            + 1;
        Err(format!(
            "line {line}: {msg} (glot only edits `return [...]` files of literal strings and arrays)"
        ))
    }

    /// Skip whitespace and comments; the last comment seen is returned as a note.
    fn ws(&self, mut i: usize) -> (usize, Option<String>) {
        let s = self.s;
        let mut note = None;
        while i < s.len() {
            match s[i] {
                b' ' | b'\t' | b'\n' | b'\r' => i += 1,
                b'#' | b'/' if s[i] == b'#' || s.get(i + 1) == Some(&b'/') => {
                    let st = i;
                    while i < s.len() && s[i] != b'\n' {
                        i += 1;
                    }
                    let t = String::from_utf8_lossy(&s[st..i]);
                    note = Some(t.trim_start_matches(['#', '/']).trim().to_string());
                }
                b'/' if s.get(i + 1) == Some(&b'*') => {
                    let st = i + 2;
                    i += 2;
                    while i + 1 < s.len() && !(s[i] == b'*' && s[i + 1] == b'/') {
                        i += 1;
                    }
                    let t = String::from_utf8_lossy(&s[st..i.min(s.len())]);
                    note = Some(
                        t.lines()
                            .map(|l| l.trim().trim_start_matches('*').trim())
                            .filter(|l| !l.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    i = (i + 2).min(s.len());
                }
                _ => break,
            }
        }
        (i, note.filter(|n| !n.is_empty()))
    }

    fn skip(&self, i: usize) -> usize {
        self.ws(i).0
    }

    /// Quoted string at `i`: (text, end after the closing quote).
    fn string(&self, mut i: usize) -> R<(String, usize)> {
        let s = self.s;
        let (q, start) = (s[i], i);
        i += 1;
        let mut out: Vec<u8> = vec![];
        while i < s.len() && s[i] != q {
            let c = s[i];
            if c == b'\\' && i + 1 < s.len() {
                let n = s[i + 1];
                i += 2;
                match (q, n) {
                    (b'\'', b'\'' | b'\\') => out.push(n),
                    (b'\'', _) => out.extend([b'\\', n]),
                    (_, b'n') => out.push(b'\n'),
                    (_, b't') => out.push(b'\t'),
                    (_, b'r') => out.push(b'\r'),
                    (_, b'v') => out.push(0x0B),
                    (_, b'f') => out.push(0x0C),
                    (_, b'e') => out.push(0x1B),
                    (_, b'\\' | b'$' | b'"') => out.push(n),
                    (_, b'u') if s.get(i) == Some(&b'{') => {
                        let end = s[i..].iter().position(|&c| c == b'}').map(|p| i + p);
                        let c = end
                            .and_then(|e| std::str::from_utf8(&s[i + 1..e]).ok())
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .and_then(char::from_u32);
                        let (Some(c), Some(e)) = (c, end) else {
                            return self.err(start, "invalid `\\u{…}` escape");
                        };
                        out.extend(c.to_string().bytes());
                        i = e + 1;
                    }
                    _ => out.extend([b'\\', n]),
                }
                continue;
            }
            if q == b'"'
                && c == b'$'
                && s.get(i + 1)
                    .is_some_and(|&n| n == b'{' || n == b'_' || n.is_ascii_alphabetic())
            {
                return self.err(i, "variable interpolation in a double-quoted string");
            }
            if q == b'"' && c == b'{' && s.get(i + 1) == Some(&b'$') {
                return self.err(i, "variable interpolation in a double-quoted string");
            }
            out.push(c);
            i += 1;
        }
        if i >= s.len() {
            return self.err(start, "unterminated string");
        }
        Ok((String::from_utf8_lossy(&out).into_owned(), i + 1))
    }

    /// Array opening at `i` (`[` or `array(`): index of the bracket, closing byte.
    fn array_open(&self, i: usize) -> Option<(usize, u8)> {
        match self.s.get(i)? {
            b'[' => Some((i, b']')),
            _ if self.s[i..].len() >= 5 && self.s[i..i + 5].eq_ignore_ascii_case(b"array") => {
                let j = self.skip(i + 5);
                (self.s.get(j) == Some(&b'(')).then_some((j, b')'))
            }
            _ => None,
        }
    }

    fn array(&self, open: usize, close: u8) -> R<Arr> {
        let s = self.s;
        let mut items = vec![];
        let mut i = open + 1;
        loop {
            let (j, note) = self.ws(i);
            i = j;
            if i >= s.len() {
                return self.err(open, "unclosed array");
            }
            if s[i] == close {
                return Ok(Arr { close: i, items });
            }
            let start = i;
            let key = match s[i] {
                b'\'' | b'"' => {
                    let (k, e) = self.string(i)?;
                    i = e;
                    k
                }
                c if c.is_ascii_digit() => {
                    while i < s.len() && s[i].is_ascii_digit() {
                        i += 1;
                    }
                    String::from_utf8_lossy(&s[start..i]).into_owned()
                }
                _ => return self.err(i, "array value without a quoted key"),
            };
            i = self.skip(i);
            if !s[i..].starts_with(b"=>") {
                return self.err(i, "expected `=>` after the key");
            }
            let v = self.skip(i + 2);
            let (val, end) = match s.get(v) {
                Some(b'\'' | b'"') => {
                    let (text, e) = self.string(v)?;
                    (
                        Val::Str {
                            text,
                            q: s[v],
                            span: (v, e),
                        },
                        e,
                    )
                }
                Some(_) if self.array_open(v).is_some() => {
                    let (o, c) = self.array_open(v).unwrap();
                    let a = self.array(o, c)?;
                    let e = a.close + 1;
                    (Val::Arr(a), e)
                }
                _ => {
                    return self.err(
                        v,
                        "dynamic value (only quoted strings and arrays are supported)",
                    );
                }
            };
            i = self.skip(end);
            let comma = s.get(i) == Some(&b',');
            if comma {
                i += 1;
            } else if s.get(i) != Some(&close) {
                return self.err(
                    i,
                    "dynamic expression after the value (concatenation, operator or call)",
                );
            }
            items.push(Item {
                key,
                start,
                end,
                comma,
                val,
                note,
            });
        }
    }

    /// `<?php [declare(...);] return [` => the array.
    fn root(&self) -> R<Arr> {
        let s = self.s;
        let mut i = self.skip(0);
        if !s[i..].starts_with(b"<?php") {
            return self.err(i, "missing `<?php`");
        }
        i = self.skip(i + 5);
        if s[i..].starts_with(b"declare") {
            match s[i..].iter().position(|&c| c == b';') {
                Some(p) => i = self.skip(i + p + 1),
                None => return self.err(i, "unterminated declare"),
            }
        }
        if !s[i..].starts_with(b"return") {
            return self.err(i, "expected `return [`");
        }
        i = self.skip(i + 6);
        let Some((open, close)) = self.array_open(i) else {
            return self.err(i, "`return` must be followed by a literal array");
        };
        let a = self.array(open, close)?;
        let mut j = self.skip(a.close + 1);
        if s.get(j) != Some(&b';') {
            return self.err(j, "expected `;` after the array");
        }
        j = self.skip(j + 1);
        if s[j..].starts_with(b"?>") {
            j = self.skip(j + 2);
        }
        if j < s.len() {
            return self.err(j, "code after `return [...];`");
        }
        Ok(a)
    }
}

/// `auth.` for `lang/fr/auth.php`.
fn prefix(path: &Path) -> String {
    path.file_stem()
        .map_or(String::new(), |s| format!("{}.", s.to_string_lossy()))
}

fn leaves<'a>(a: &'a Arr, pre: &str, out: &mut Vec<(String, &'a Item)>) {
    for it in &a.items {
        let key = format!("{pre}{}", it.key);
        match &it.val {
            Val::Arr(inner) => leaves(inner, &format!("{key}."), out),
            Val::Str { .. } => out.push((key, it)),
        }
    }
}

pub fn parse(s: &str, path: &Path) -> Result<Parsed, String> {
    let root = Scan { s: s.as_bytes() }.root()?;
    let mut ls = vec![];
    leaves(&root, &prefix(path), &mut ls);
    let mut p = Parsed::default();
    for (k, it) in ls {
        if let Val::Str { text, .. } = &it.val {
            if let Some(n) = &it.note {
                p.notes.insert(k.clone(), n.clone());
            }
            p.entries.insert(k, text.clone());
        }
    }
    Ok(p)
}

fn quote(v: &str, q: u8) -> String {
    let q = q as char;
    let mut out = String::from(q);
    for c in v.chars() {
        match c {
            '\\' => out += "\\\\",
            '$' if q == '"' => out += "\\$",
            '\n' if q == '"' => out += "\\n",
            '\t' if q == '"' => out += "\\t",
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

#[derive(Default)]
struct Tree(BTreeMap<String, Node>);
enum Node {
    Leaf(String),
    Branch(Tree),
}

impl Tree {
    fn insert(&mut self, segs: &[&str], v: &str) {
        match segs {
            [] => {}
            [k] => {
                self.0.insert(k.to_string(), Node::Leaf(v.into()));
            }
            [k, rest @ ..] => {
                let n = self
                    .0
                    .entry(k.to_string())
                    .or_insert_with(|| Node::Branch(Tree::default()));
                if let Node::Branch(t) = n {
                    t.insert(rest, v);
                }
            }
        }
    }

    fn render(&self, out: &mut String, indent: &str, step: &str, q: u8) {
        for (k, n) in &self.0 {
            match n {
                Node::Leaf(v) => *out += &format!("{indent}{} => {},\n", quote(k, q), quote(v, q)),
                Node::Branch(t) => {
                    *out += &format!("{indent}{} => [\n", quote(k, q));
                    t.render(out, &format!("{indent}{step}"), step, q);
                    *out += &format!("{indent}],\n");
                }
            }
        }
    }
}

fn line_start(s: &str, i: usize) -> usize {
    s[..i].rfind('\n').map_or(0, |p| p + 1)
}

fn indent_of(s: &str, i: usize) -> &str {
    let line = &s[line_start(s, i)..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Deepest existing array for `segs` and how many segments it consumed.
fn target<'a>(a: &'a Arr, segs: &[&str]) -> (&'a Arr, usize) {
    if segs.len() > 1
        && let Some(Item {
            val: Val::Arr(inner),
            ..
        }) = a.items.iter().find(|i| i.key == segs[0])
    {
        let (t, n) = target(inner, &segs[1..]);
        return (t, n + 1);
    }
    (a, 0)
}

pub fn write(existing: Option<&str>, cat: &Catalog, path: &Path) -> String {
    let pre = prefix(path);
    // ponytail: a key outside this file's prefix (routed here as the first file) is kept whole.
    let own = |k: &str| k.strip_prefix(pre.as_str()).unwrap_or(k).to_string();
    let fresh = || {
        let mut t = Tree::default();
        for (k, v) in &cat.entries {
            let k = own(k);
            t.insert(&k.split('.').collect::<Vec<_>>(), v);
        }
        let mut out = String::from("<?php\n\nreturn [\n");
        t.render(&mut out, "    ", "    ", b'\'');
        out + "];\n"
    };
    let Some(s) = existing else { return fresh() };
    let Ok(root) = (Scan { s: s.as_bytes() }).root() else {
        // ponytail: unparseable files are refused on read first; never reached in practice.
        return s.to_string();
    };
    let entries: BTreeMap<String, &String> = cat.entries.iter().map(|(k, v)| (own(k), v)).collect();
    let mut ls = vec![];
    leaves(&root, "", &mut ls);
    let q = ls
        .iter()
        .filter_map(|(_, it)| match it.val {
            Val::Str { q, .. } => Some(q),
            _ => None,
        })
        .fold(0i32, |n, q| n + if q == b'"' { 1 } else { -1 });
    let q = if q > 0 { b'"' } else { b'\'' };
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let (mut seen, mut dropped) = (BTreeSet::new(), BTreeSet::new());
    for (k, it) in &ls {
        seen.insert(k.clone());
        let Val::Str { text, q, span } = &it.val else {
            continue;
        };
        match entries.get(k) {
            Some(v) if *v != text => edits.push((span.0, span.1, quote(v, *q))),
            Some(_) => {}
            None => {
                dropped.insert(it.start);
                let b = s.as_bytes();
                let ls0 = line_start(s, it.start);
                let mut e = it.end;
                while e < b.len() && matches!(b[e], b' ' | b'\t') {
                    e += 1;
                }
                if b.get(e) == Some(&b',') {
                    e += 1;
                }
                let own_line = s[ls0..it.start].trim().is_empty();
                let nl = s[e..].find('\n').map(|n| e + n);
                let (a, z) = match nl {
                    Some(n) if own_line && s[e..n].trim().is_empty() => (ls0, n + 1),
                    _ => (it.start, e),
                };
                edits.push((a, z, String::new()));
            }
        }
    }
    let mut groups: BTreeMap<usize, (&Arr, Tree)> = BTreeMap::new();
    for (k, v) in entries.iter().filter(|(k, _)| !seen.contains(*k)) {
        let segs: Vec<&str> = k.split('.').collect();
        let (a, depth) = target(&root, &segs);
        groups
            .entry(a.close)
            .or_insert_with(|| (a, Tree::default()))
            .1
            .insert(&segs[depth..], v);
    }
    for (a, tree) in groups.values() {
        let close_indent = indent_of(s, a.close);
        let alone = s[line_start(s, a.close)..a.close].trim().is_empty();
        let indent = match a.items.first() {
            Some(it) if s[line_start(s, it.start)..it.start].trim().is_empty() => {
                indent_of(s, it.start).to_string()
            }
            _ => format!("{close_indent}    "),
        };
        let step = indent
            .strip_prefix(close_indent)
            .filter(|x| !x.is_empty())
            .unwrap_or("    ")
            .to_string();
        if let Some(last) = a.items.iter().rev().find(|it| !dropped.contains(&it.start))
            && !last.comma
        {
            edits.push((last.end, last.end, ",".into()));
        }
        let mut body = String::new();
        tree.render(&mut body, &indent, &step, q);
        if alone {
            let at = line_start(s, a.close);
            edits.push((at, at, body));
        } else {
            edits.push((a.close, a.close, format!("\n{body}{close_indent}")));
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = s.to_string();
    for (a, b, t) in edits {
        out.replace_range(a..b, &t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/auth.php"
    ));

    fn path() -> &'static Path {
        Path::new("lang/fr/auth.php")
    }

    #[test]
    fn parse_roundtrip_edit() {
        let p = parse(FIX, path()).unwrap();
        let e = &p.entries;
        assert_eq!(e["auth.failed"], "Ces identifiants ne correspondent pas.");
        assert_eq!(
            e["auth.throttle"],
            "Trop de tentatives. Réessayez dans :seconds secondes."
        );
        assert_eq!(e["auth.quote"], "L'accès est refusé \\o/");
        assert_eq!(e["auth.double"], "Ligne\n\"citée\" $");
        assert_eq!(e["auth.nested.title"], "Connexion");
        assert_eq!(e["auth.nested.deep.x"], "X");
        assert_eq!(e["auth.legacy.ok"], "OK");
        assert_eq!(e["auth.apples"], "{0} Aucune|[1,*] :count pommes");
        assert_eq!(p.notes["auth.failed"], "Message d'échec de connexion");
        let mut c = Catalog {
            locale: "fr".into(),
            entries: p.entries.clone(),
        };
        assert_eq!(write(Some(FIX), &c, path()), FIX, "byte for byte");
        c.entries
            .insert("auth.failed".into(), "Identifiants 'invalides'".into());
        c.entries.remove("auth.legacy.ok");
        c.entries
            .insert("auth.nested.subtitle".into(), "Bienvenue".into());
        c.entries.insert("auth.fresh.a".into(), "A".into());
        let out = write(Some(FIX), &c, path());
        assert!(
            out.contains("'failed' => 'Identifiants \\'invalides\\'',"),
            "{out}"
        );
        assert!(
            out.contains("        'subtitle' => 'Bienvenue',\n    ],"),
            "{out}"
        );
        assert!(
            out.contains("    'fresh' => [\n        'a' => 'A',\n    ],\n];"),
            "{out}"
        );
        assert_eq!(parse(&out, path()).unwrap().entries, c.entries);
        let fresh = write(None, &c, path());
        assert!(fresh.starts_with("<?php\n\nreturn [\n"));
        assert_eq!(parse(&fresh, path()).unwrap().entries, c.entries);
    }

    #[test]
    fn refuses_dynamic_php() {
        for (src, what) in [
            ("<?php return ['a' => 'x' . 'y'];", "concatenation"),
            ("<?php return ['a' => \"hi $name\"];", "interpolation"),
            ("<?php return ['a' => trans('b')];", "dynamic value"),
            ("<?php return ['a' => FOO];", "dynamic value"),
            ("<?php $x = 1; return [];", "expected `return"),
            ("<?php return ['a', 'b'];", "expected `=>`"),
            ("<?php return require 'x.php';", "literal array"),
            ("<?php return ['a' => <<<EOT\nx\nEOT];", "dynamic value"),
        ] {
            let e = parse(src, path())
                .err()
                .unwrap_or_else(|| panic!("accepted: {src}"));
            assert!(e.contains(what), "{src}: {e}");
            assert!(e.starts_with("line "), "{e}");
        }
    }
}
