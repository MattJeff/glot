//! Swift / SwiftUI extraction without a Swift toolchain: a literal-aware lexer (`"…"`, `"""…"""`,
//! `#"…"#`, nested `\(…)`, nested comments) and an allowlist of localizing APIs.
//!
//! SwiftUI uses the text as the key: `Text("\(n) countries")` looks up `%lld countries`. The
//! interpolation's specifier comes from its type (literal, `let x: Int`, `.count`, `specifier:`), found
//! in the file or project-wide; unknown types fall back to `%@` with a warning.
use std::collections::HashMap;

use glot_core::context::SourceRef;

use crate::{ExtractReport, Hardcoded, Usage};

#[derive(Debug, Clone, Copy, PartialEq)]
enum K {
    Ident,
    Num(bool),
    Str(usize),
    P(char),
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    k: K,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
enum Piece {
    Text(String),
    Interp(usize, usize),
}

#[derive(Debug, Clone)]
struct Lit {
    pieces: Vec<Piece>,
}

/// Escaped `\n` (kept apart from source newlines while dedenting multiline literals).
const NL: char = '\u{E000}';
/// Line start without a newline (start of a multiline literal, `\` line continuation).
const LS: char = '\u{E001}';

struct Lexer<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    toks: Vec<Tok>,
    lits: Vec<Lit>,
}

/// Tokens of `s[start..end]`; `Err(offset)` on an unterminated literal or comment.
fn lex(s: &str, start: usize, end: usize) -> Result<(Vec<Tok>, Vec<Lit>), usize> {
    let mut l = Lexer {
        s,
        b: s.as_bytes(),
        pos: start,
        toks: vec![],
        lits: vec![],
    };
    l.run(end, false)?;
    Ok((l.toks, l.lits))
}

impl Lexer<'_> {
    fn at(&self, i: usize) -> u8 {
        self.b.get(i).copied().unwrap_or(0)
    }

    /// Lex until `end`, or (`interp`) until the `)` closing an interpolation, whose offset is returned.
    fn run(&mut self, end: usize, interp: bool) -> Result<usize, usize> {
        let mut depth = 0usize;
        while self.pos < end {
            let start = self.pos;
            let c = self.b[start];
            match c {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                b'/' if self.at(start + 1) == b'/' => {
                    self.pos = self.s[start..end].find('\n').map_or(end, |i| start + i);
                }
                b'/' if self.at(start + 1) == b'*' => {
                    let mut d = 0;
                    loop {
                        if self.pos + 1 >= end {
                            return Err(start);
                        }
                        match (self.b[self.pos], self.b[self.pos + 1]) {
                            (b'/', b'*') => (d, self.pos) = (d + 1, self.pos + 2),
                            (b'*', b'/') => {
                                (d, self.pos) = (d - 1, self.pos + 2);
                                if d == 0 {
                                    break;
                                }
                            }
                            _ => self.pos += 1,
                        }
                    }
                }
                b'"' => self.string(0, end)?,
                b'#' if {
                    let n = self.b[start..end]
                        .iter()
                        .take_while(|&&c| c == b'#')
                        .count();
                    self.at(start + n) == b'"'
                } =>
                {
                    let n = self.b[start..end]
                        .iter()
                        .take_while(|&&c| c == b'#')
                        .count();
                    self.pos += n;
                    self.string(n, end)?;
                }
                b'`' => {
                    let close = self.s[start + 1..end].find('`').ok_or(start)?;
                    self.pos = start + close + 2;
                    self.push(K::Ident, start + 1, start + 1 + close);
                }
                c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80 => {
                    while self.pos < end && {
                        let c = self.b[self.pos];
                        c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
                    } {
                        self.pos += 1;
                    }
                    self.push(K::Ident, start, self.pos);
                }
                c if c.is_ascii_digit() => {
                    while self.pos < end
                        && (self.b[self.pos].is_ascii_alphanumeric()
                            || self.b[self.pos] == b'_'
                            || (self.b[self.pos] == b'.' && self.at(self.pos + 1).is_ascii_digit()))
                    {
                        self.pos += 1;
                    }
                    let t = &self.s[start..self.pos];
                    let float = t.contains('.') || (!t.starts_with("0x") && t.contains(['e', 'E']));
                    self.push(K::Num(float), start, self.pos);
                }
                _ => {
                    if interp && c == b'(' {
                        depth += 1;
                    }
                    if interp && c == b')' {
                        if depth == 0 {
                            return Ok(start);
                        }
                        depth -= 1;
                    }
                    self.pos += 1;
                    self.push(K::P(c as char), start, self.pos);
                }
            }
        }
        if interp { Err(self.pos) } else { Ok(self.pos) }
    }

    fn push(&mut self, k: K, start: usize, end: usize) {
        self.toks.push(Tok { k, start, end });
    }

    /// A string literal; `self.pos` is on its first `"`, after `n` leading `#`.
    fn string(&mut self, n: usize, end: usize) -> Result<(), usize> {
        let start = self.pos - n;
        let multi = self.b[self.pos..].starts_with(b"\"\"\"");
        let mut close = if multi {
            b"\"\"\"".to_vec()
        } else {
            b"\"".to_vec()
        };
        close.extend(std::iter::repeat_n(b'#', n));
        let hashes = vec![b'#'; n];
        self.pos += if multi { 3 } else { 1 };
        let (mut pieces, mut text) = (vec![], String::new());
        if multi {
            let nl = self.s[self.pos..end].find('\n').ok_or(start)?;
            self.pos += nl + 1;
            text.push(LS);
        }
        loop {
            if self.pos >= end {
                return Err(start);
            }
            if self.b[self.pos..].starts_with(&close) {
                self.pos += close.len();
                break;
            }
            let c = self.b[self.pos];
            if c == b'\n' && !multi {
                return Err(start);
            }
            if c == b'\\' && self.b[self.pos + 1..].starts_with(&hashes) {
                self.pos += 1 + n;
                let e = self.at(self.pos);
                self.pos += 1;
                match e {
                    b'(' => {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                        let (nt, nl) = (self.toks.len(), self.lits.len());
                        let from = self.pos;
                        let to = self.run(end, true)?;
                        self.toks.truncate(nt);
                        self.lits.truncate(nl);
                        pieces.push(Piece::Interp(from, to));
                        self.pos = to + 1;
                    }
                    b'n' => text.push(NL),
                    b't' => text.push('\t'),
                    b'r' => text.push('\r'),
                    b'0' => text.push('\0'),
                    b'\\' | b'"' | b'\'' => text.push(e as char),
                    b'u' if self.at(self.pos) == b'{' => {
                        let close = self.s[self.pos..end].find('}').ok_or(start)?;
                        let hex = &self.s[self.pos + 1..self.pos + close];
                        let ch = u32::from_str_radix(hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or(start)?;
                        text.push(ch);
                        self.pos += close + 1;
                    }
                    b' ' | b'\t' | b'\r' | b'\n' if multi => {
                        self.pos -= 1;
                        while matches!(self.at(self.pos), b' ' | b'\t' | b'\r') {
                            self.pos += 1;
                        }
                        if self.at(self.pos) != b'\n' {
                            return Err(start);
                        }
                        self.pos += 1;
                        text.push(LS);
                    }
                    _ => return Err(start),
                }
                continue;
            }
            let ch = self.s[self.pos..].chars().next().ok_or(start)?;
            text.push(ch);
            self.pos += ch.len_utf8();
        }
        pieces.push(Piece::Text(text));
        if multi {
            dedent(&mut pieces);
        }
        for p in &mut pieces {
            if let Piece::Text(t) = p {
                *t = t.replace(LS, "").replace(NL, "\n");
            }
        }
        self.lits.push(Lit { pieces });
        let idx = self.lits.len() - 1;
        self.push(K::Str(idx), start, self.pos);
        Ok(())
    }
}

/// Multiline literal: strip the closing delimiter's indentation from every line, and the last newline.
fn dedent(pieces: &mut [Piece]) {
    let Some(Piece::Text(last)) = pieces.last_mut() else {
        return;
    };
    let cut = last
        .char_indices()
        .rfind(|&(_, c)| c == '\n' || c == LS)
        .map_or(0, |(i, c)| i + c.len_utf8());
    let indent = last[cut..].to_string();
    if !indent.chars().all(|c| c == ' ' || c == '\t') {
        return;
    }
    last.truncate(cut);
    if last.ends_with('\n') {
        last.pop();
    }
    for p in pieces.iter_mut() {
        if let Piece::Text(t) = p {
            let mut out = String::new();
            let mut rest = t.as_str();
            while let Some(i) = rest.find(['\n', LS]) {
                let ch = rest[i..].chars().next().unwrap();
                out.push_str(&rest[..i + ch.len_utf8()]);
                rest = &rest[i + ch.len_utf8()..];
                rest = rest.strip_prefix(indent.as_str()).unwrap_or(rest);
            }
            out.push_str(rest);
            *t = out;
        }
    }
}

/// One string found in Swift code.
#[derive(Debug, Clone, PartialEq)]
pub struct SwiftString {
    /// Catalog key (SwiftUI-style: interpolations become `%@`, `%lld`…).
    pub key: String,
    /// Default value when the code gives one (SwiftGen `fallback:`).
    pub value: Option<String>,
    /// `tableName:` / `table:` / SwiftGen table; `None` = `Localizable`.
    pub table: Option<String>,
    /// `comment:` argument (note for translators).
    pub comment: Option<String>,
    /// API it was passed to (`Text`, `navigationTitle`, `setTitle`, `.text =`…).
    pub api: String,
    pub line: u32,
    /// Byte span of the whole literal (quotes and `#` included).
    pub start: usize,
    pub end: usize,
    /// Already localized at runtime (SwiftUI key, `NSLocalizedString`…); `false` = plain `String`
    /// shown to the user (UIKit), to wrap in `String(localized:)`.
    pub localized: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SwiftScan {
    pub strings: Vec<SwiftString>,
    /// Lines of calls whose key is not static (`NSLocalizedString(name)`, `L10n.tr("\(x)")`).
    pub dynamic: Vec<u32>,
    /// Static start of those keys (`"errors.\(code)"` -> `errors.`).
    pub dynamic_prefixes: Vec<String>,
    pub warnings: Vec<(u32, String)>,
    /// Lexer error (unterminated literal): nothing was extracted.
    pub error: Option<String>,
}

/// Views whose first unlabeled argument is a `LocalizedStringKey`.
const VIEWS: &[&str] = &[
    "Text",
    "Button",
    "Label",
    "Toggle",
    "Picker",
    "TextField",
    "SecureField",
    "Section",
    "Menu",
    "Link",
    "NavigationLink",
    "ContentUnavailableView",
    "Stepper",
    "DatePicker",
    "ColorPicker",
    "ProgressView",
    "LabeledContent",
    "GroupBox",
    "DisclosureGroup",
    "Tab",
    "ShareLink",
    "PasteButton",
    "LocalizedStringKey",
    "LocalizedStringResource",
];
/// View modifiers whose first unlabeled argument is a `LocalizedStringKey`.
const MODIFIERS: &[&str] = &[
    "navigationTitle",
    "navigationSubtitle",
    "alert",
    "confirmationDialog",
    "help",
    "accessibilityLabel",
    "accessibilityHint",
    "accessibilityValue",
    "badge",
];
/// UIKit: `(api, labels)`; `""` = first unlabeled argument. Plain `String`, not localized.
const UIKIT_CALLS: &[(&str, &[&str])] = &[
    ("setTitle", &[""]),
    ("UIAlertController", &["title", "message"]),
    ("UIAlertAction", &["title"]),
    ("UIBarButtonItem", &["title"]),
    ("UIAction", &["title"]),
    ("UIMenu", &["title"]),
    ("UIContextualAction", &["title"]),
];
/// `x.text = "…"` and friends.
const UIKIT_PROPS: &[&str] = &[
    "text",
    "title",
    "placeholder",
    "prompt",
    "message",
    "accessibilityLabel",
    "accessibilityHint",
];

#[derive(Default)]
struct Frame {
    open: char,
    callee: Option<(String, String, bool)>, // (dotted chain, last name, member call `.x(`)
    arg: usize,
    label: Option<String>,
    lits: Vec<(usize, Option<String>, Tok)>, // (arg index, label, literal token)
    ident0: Option<String>,
}

/// Type names of `let/var x: T`, `x = 1`, params `x: T`, across a set of sources. A name declared
/// with two different types is dropped (ambiguous).
pub fn project_types<'a>(sources: impl IntoIterator<Item = &'a str>) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut bad = std::collections::HashSet::new();
    for s in sources {
        for (k, v) in file_types(s) {
            match out.get(&k) {
                Some(t) if *t != v => {
                    bad.insert(k);
                }
                _ => {
                    out.insert(k, v);
                }
            }
        }
    }
    out.retain(|k, _| !bad.contains(k));
    out
}

fn file_types(src: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let Ok((toks, _)) = lex(src, 0, src.len()) else {
        return out;
    };
    let txt = |t: &Tok| &src[t.start..t.end];
    for i in 0..toks.len() {
        let w = |j: usize| toks.get(i + j);
        if w(0).map(|t| t.k) != Some(K::Ident) || w(1).map(|t| t.k) == Some(K::P('.')) {
            continue;
        }
        let name = txt(&toks[i]).to_string();
        match (w(1).map(|t| t.k), w(2)) {
            (Some(K::P(':')), Some(t)) if t.k == K::Ident => {
                let ty = txt(t);
                let optional = w(3).is_some_and(|t| matches!(t.k, K::P('?') | K::P('!')));
                if ty.starts_with(char::is_uppercase) {
                    out.push((name, if optional { "?" } else { ty }.to_string()));
                }
            }
            // `let x = …`: typed by a literal or `Type(…)`; anything else is unknown here ("?"),
            // so a same-named declaration elsewhere is not trusted.
            (Some(K::P('=')), Some(t)) if i > 0 && matches!(txt(&toks[i - 1]), "let" | "var") => {
                let simple = w(3)
                    .is_none_or(|n| !matches!(n.k, K::P('.' | '+' | '-' | '*' | '/' | '?' | '[')));
                let ty = match t.k {
                    K::Num(true) if simple => "Double",
                    K::Num(false) if simple => "Int",
                    K::Str(_) if simple => "String",
                    K::Ident
                        if txt(t).starts_with(char::is_uppercase)
                            && w(3).is_some_and(|n| n.k == K::P('(')) =>
                    {
                        txt(t)
                    }
                    _ => "?",
                };
                out.push((name, ty.to_string()));
            }
            _ => {}
        }
    }
    out
}

fn type_spec(ty: &str) -> &'static str {
    match ty {
        "Int" | "Int8" | "Int16" | "Int32" | "Int64" => "%lld",
        "UInt" | "UInt8" | "UInt16" | "UInt32" | "UInt64" => "%llu",
        "Double" | "CGFloat" | "TimeInterval" => "%lf",
        "Float" => "%f",
        _ => "%@",
    }
}

/// Scan one Swift file. `fns`: extra key functions (`L10n.tr`), `types`: project-wide declarations.
pub fn scan(path: &str, src: &str, fns: &[String], types: &HashMap<String, String>) -> SwiftScan {
    let out = SwiftScan::default();
    if is_test_or_preview(path) {
        return out;
    }
    let mut types = types.clone();
    types.extend(project_types([src]));
    let mut line_starts = vec![0];
    line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
    let mut a = An {
        src,
        line_starts,
        fns,
        types,
        out,
    };
    if let Err(at) = a.walk(0, src.len()) {
        a.out = SwiftScan {
            error: Some(format!(
                "unterminated literal or comment at line {}",
                a.line(at)
            )),
            ..Default::default()
        };
    }
    a.out
}

fn is_test_or_preview(path: &str) -> bool {
    [
        "Tests/",
        "Tests.swift",
        "Preview Content/",
        "Previews.swift",
    ]
    .iter()
    .any(|p| path.contains(p))
}

struct An<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
    fns: &'a [String],
    types: HashMap<String, String>,
    out: SwiftScan,
}

impl An<'_> {
    fn line(&self, pos: usize) -> u32 {
        self.line_starts.partition_point(|&s| s <= pos) as u32
    }

    fn walk(&mut self, start: usize, end: usize) -> Result<(), usize> {
        let (toks, lits) = lex(self.src, start, end)?;
        let src = self.src;
        let txt = |t: &Tok| &src[t.start..t.end];
        let is = |i: usize, k: K| toks.get(i).is_some_and(|t| t.k == k);
        let mut stack: Vec<Frame> = vec![];
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i];
            // `#Preview { … }`, `struct X_Previews: PreviewProvider { … }`: skip the block.
            if (t.k == K::P('#') && toks.get(i + 1).is_some_and(|n| txt(n) == "Preview"))
                || (t.k == K::Ident && txt(&t) == "PreviewProvider")
            {
                let Some(open) = (i..toks.len()).find(|&j| is(j, K::P('{'))) else {
                    break;
                };
                let mut d = 0;
                i = open;
                while i < toks.len() {
                    match toks[i].k {
                        K::P('{') => d += 1,
                        K::P('}') => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            match t.k {
                K::P('(') => {
                    let mut j = i;
                    let mut chain = vec![];
                    while j > 0 && toks[j - 1].k == K::Ident {
                        chain.push(txt(&toks[j - 1]));
                        j -= 1;
                        if j > 0 && toks[j - 1].k == K::P('.') {
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                    let member = j < i && toks.get(j).is_some_and(|t| t.k == K::P('.'));
                    chain.reverse();
                    let callee = (!chain.is_empty())
                        .then(|| (chain.join("."), chain.last().unwrap().to_string(), member));
                    stack.push(Frame {
                        open: '(',
                        callee,
                        ..Default::default()
                    });
                }
                K::P(c @ ('[' | '{')) => stack.push(Frame {
                    open: c,
                    ..Default::default()
                }),
                K::P(')' | ']' | '}') => {
                    if let Some(f) = stack.pop()
                        && f.open == '('
                    {
                        self.call(&f, &lits);
                    }
                }
                K::P(',') => {
                    if let Some(f) = stack.last_mut() {
                        f.arg += 1;
                        f.label = None;
                    }
                }
                K::P(':') => {
                    if i >= 2
                        && toks[i - 1].k == K::Ident
                        && matches!(toks[i - 2].k, K::P('(') | K::P(','))
                        && let Some(f) = stack.last_mut()
                        && f.open == '('
                    {
                        f.label = Some(txt(&toks[i - 1]).to_string());
                    }
                }
                K::Ident => {
                    if i > 0
                        && is(i - 1, K::P('('))
                        && (is(i + 1, K::P(',')))
                        && let Some(f) = stack.last_mut()
                        && f.arg == 0
                    {
                        f.ident0 = Some(txt(&t).to_string());
                    }
                }
                K::Str(idx) => {
                    for p in &lits[idx].pieces {
                        if let Piece::Interp(a, b) = *p {
                            self.walk(a, b)?;
                        }
                    }
                    let prev = |n: usize| (i >= n).then(|| toks[i - n].k);
                    let next = toks.get(i + 1).map(|t| t.k);
                    let after_arg = matches!(prev(1), Some(K::P('(') | K::P(',') | K::P(':')))
                        || (prev(1) == Some(K::P('?')) && prev(2) != Some(K::P('?')));
                    let ends_arg = matches!(next, Some(K::P(')') | K::P(',') | K::P(':')));
                    if after_arg
                        && ends_arg
                        && let Some(f) = stack.last_mut()
                        && f.open == '('
                    {
                        f.lits.push((f.arg, f.label.clone(), t));
                    } else if prev(1) == Some(K::P('='))
                        && prev(2) == Some(K::Ident)
                        && prev(3) == Some(K::P('.'))
                        && UIKIT_PROPS.contains(&txt(&toks[i - 2]))
                        && !matches!(next, Some(K::P('+') | K::P('.')))
                    {
                        let api = format!(".{} =", txt(&toks[i - 2]));
                        self.emit(&lits[idx], t, api, false, true, None, None);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Ok(())
    }

    fn call(&mut self, f: &Frame, lits: &[Lit]) {
        let Some((chain, name, member)) = &f.callee else {
            return;
        };
        let lit = |label: Option<&str>| {
            f.lits
                .iter()
                .find(|(a, l, _)| l.as_deref() == label && (label.is_some() || *a == 0))
                .map(|x| x.2)
        };
        let plain = |label: &str| {
            lit(Some(label)).and_then(|t| match t.k {
                K::Str(i) => plain(&lits[i]),
                _ => None,
            })
        };
        let comment = plain("comment");
        let n = name.as_str();
        let swiftui = (!member && VIEWS.contains(&n)) || (*member && MODIFIERS.contains(&n));
        if swiftui || (*member && n == "searchable") {
            let label = (n == "searchable").then_some("prompt");
            let table = plain("tableName").or_else(|| plain("table"));
            // Every literal of the key argument (`Text(on ? "On" : "Off")` has two).
            let keys: Vec<Tok> = f
                .lits
                .iter()
                .filter(|(a, l, _)| l.as_deref() == label && (label.is_some() || *a == 0))
                .map(|x| x.2)
                .collect();
            for t in keys {
                self.emit_tok(
                    lits,
                    t,
                    n.to_string(),
                    true,
                    true,
                    table.clone(),
                    comment.clone(),
                );
            }
        } else if n == "NSLocalizedString" {
            if let Some(t) = lit(None) {
                self.emit_tok(
                    lits,
                    t,
                    n.to_string(),
                    true,
                    false,
                    plain("tableName"),
                    comment,
                );
            } else {
                self.out.dynamic.push(self.line_of_frame(f));
            }
        } else if matches!(n, "String" | "AttributedString") && !member {
            if let Some(t) = lit(Some("localized")) {
                self.emit_tok(
                    lits,
                    t,
                    format!("{n}(localized:)"),
                    true,
                    true,
                    plain("table"),
                    comment,
                );
            }
        } else if n == "tr"
            || self
                .fns
                .iter()
                .any(|g| chain == g || chain.ends_with(&format!(".{g}")))
        {
            let unl: Vec<Tok> = f
                .lits
                .iter()
                .filter(|x| x.1.is_none())
                .map(|x| x.2)
                .collect();
            let at = |a: usize| {
                f.lits
                    .iter()
                    .find(|x| x.1.is_none() && x.0 == a)
                    .map(|x| x.2)
            };
            let (table, key) = match (at(0), at(1)) {
                (Some(t), Some(k)) => (self.plain_tok(lits, t), Some(k)),
                (None, Some(k)) => (
                    f.ident0
                        .as_ref()
                        .and_then(|id| self.const_before(id, k.start)),
                    Some(k),
                ),
                (Some(k), None) => (None, Some(k)),
                _ => (None, unl.first().copied()),
            };
            match key {
                Some(k) => {
                    let before = self.out.strings.len();
                    self.emit_tok(lits, k, chain.clone(), true, false, table, comment);
                    if let Some(s) = self.out.strings.get_mut(before) {
                        s.value = plain("fallback");
                    }
                }
                None => self.out.dynamic.push(self.line_of_frame(f)),
            }
        } else if let Some((_, labels)) = UIKIT_CALLS.iter().find(|(c, _)| *c == n) {
            for l in *labels {
                if let Some(t) = lit((!l.is_empty()).then_some(*l)) {
                    self.emit_tok(lits, t, n.to_string(), false, true, None, None);
                }
            }
        }
    }

    fn line_of_frame(&self, f: &Frame) -> u32 {
        f.lits.first().map_or(0, |x| self.line(x.2.start))
    }

    /// Value of the nearest `let NAME = "…"` before `pos` (SwiftGen-style `tr(table, "key")`).
    fn const_before(&self, name: &str, pos: usize) -> Option<String> {
        let (toks, lits) = lex(self.src, 0, pos).ok()?;
        toks.windows(3).rev().find_map(|w| {
            (self.src[w[0].start..w[0].end] == *name && w[1].k == K::P('='))
                .then_some(w[2])
                .and_then(|t| match t.k {
                    K::Str(i) => self.plain(&lits[i]),
                    _ => None,
                })
        })
    }

    fn plain_tok(&self, lits: &[Lit], t: Tok) -> Option<String> {
        match t.k {
            K::Str(i) => self.plain(&lits[i]),
            _ => None,
        }
    }

    fn plain(&self, lit: &Lit) -> Option<String> {
        plain(lit)
    }
}

/// Text of a literal without interpolation.
fn plain(lit: &Lit) -> Option<String> {
    {
        let mut s = String::new();
        for p in &lit.pieces {
            match p {
                Piece::Text(t) => s.push_str(t),
                Piece::Interp(..) => return None,
            }
        }
        Some(s)
    }
}

impl An<'_> {
    #[allow(clippy::too_many_arguments)]
    fn emit_tok(
        &mut self,
        lits: &[Lit],
        t: Tok,
        api: String,
        localized: bool,
        swiftui_key: bool,
        table: Option<String>,
        comment: Option<String>,
    ) {
        if let K::Str(i) = t.k {
            let lit = lits[i].clone();
            if !swiftui_key && self.plain(&lit).is_none() {
                // `NSLocalizedString("\(x)")`: the key is built at runtime.
                self.out.dynamic.push(self.line(t.start));
                // `tr("bugReport.\(kind)")`: keys under `bugReport.` count as used.
                if let Some(Piece::Text(p)) = lit.pieces.first()
                    && !p.is_empty()
                {
                    self.out.dynamic_prefixes.push(p.clone());
                }
                return;
            }
            self.emit(&lit, t, api, localized, swiftui_key, table, comment);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        lit: &Lit,
        t: Tok,
        api: String,
        localized: bool,
        swiftui_key: bool,
        table: Option<String>,
        comment: Option<String>,
    ) {
        let line = self.line(t.start);
        let interp = lit.pieces.iter().any(|p| matches!(p, Piece::Interp(..)));
        let mut key = String::new();
        for p in &lit.pieces {
            match *p {
                Piece::Text(ref s) if interp => key.push_str(&s.replace('%', "%%")),
                Piece::Text(ref s) => key.push_str(s),
                Piece::Interp(a, b) => {
                    let (spec, known) = self.spec(a, b);
                    if !known {
                        let expr = self.src[a..b].trim().to_string();
                        self.out.warnings.push((
                            line,
                            format!("type of \\({expr}) unknown, assumed %@ (check with Xcode's Export Localizations)"),
                        ));
                    }
                    key.push_str(&spec);
                }
            }
        }
        // Keys made only of placeholders / punctuation (`"\(n)"`, `"•"`) are not copy.
        let letters = key
            .replace("%lld", "")
            .replace("%llu", "")
            .replace("%lf", "")
            .replace("%@", "");
        if swiftui_key && !letters.chars().any(char::is_alphabetic) {
            return;
        }
        if !localized && (key.contains("://") || !crate::i18nize::looks_like_copy(&letters)) {
            return;
        }
        self.out.strings.push(SwiftString {
            key,
            value: None,
            table,
            comment,
            api,
            line,
            start: t.start,
            end: t.end,
            localized,
        });
    }

    /// Format specifier of `\(a..b)` and whether its type was known.
    fn spec(&self, a: usize, b: usize) -> (String, bool) {
        let Ok((toks, lits)) = lex(self.src, a, b) else {
            return ("%@".into(), false);
        };
        let txt = |t: &Tok| &self.src[t.start..t.end];
        if let Some(w) = toks
            .windows(3)
            .find(|w| txt(&w[0]) == "specifier" && w[1].k == K::P(':'))
            && let K::Str(i) = w[2].k
            && let Some(s) = self.plain(&lits[i])
        {
            return (s, true);
        }
        let mut depth = 0i32;
        for t in &toks {
            match t.k {
                K::P('(' | '[' | '{') => depth += 1,
                K::P(')' | ']' | '}') => depth -= 1,
                // `\(date, format: …)`, `\(value, style: …)`: formatted to a String.
                K::P(',') if depth == 0 => return ("%@".into(), true),
                _ => {}
            }
        }
        // Operands: literals and `a.b.c` chains (last name), joined by arithmetic.
        let mut types = vec![];
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i];
            match t.k {
                K::Num(f) => types.push(Some(if f { "Double" } else { "Int" }.to_string())),
                K::Str(_) => types.push(Some("String".into())),
                K::Ident => {
                    let mut j = i;
                    while toks.get(j + 1).is_some_and(|t| t.k == K::P('.'))
                        && toks.get(j + 2).is_some_and(|t| t.k == K::Ident)
                    {
                        j += 2;
                    }
                    let last = txt(&toks[j]);
                    let call = toks.get(j + 1).is_some_and(|t| t.k == K::P('('));
                    let ty = if call {
                        let close = (j + 1..toks.len()).scan(0, |d, k| {
                            match toks[k].k {
                                K::P('(') => *d += 1,
                                K::P(')') => *d -= 1,
                                _ => {}
                            }
                            Some((k, *d))
                        });
                        let end = close.filter(|&(_, d)| d == 0).map(|(k, _)| k).next();
                        let ty = match last {
                            "formatted" | "String" | "description" | "tr" => {
                                Some("String".to_string())
                            }
                            l if l.starts_with(char::is_uppercase) => Some(l.to_string()),
                            _ => None,
                        };
                        j = end.unwrap_or(toks.len());
                        ty
                    } else if matches!(last, "count" | "startIndex" | "endIndex") && j > i {
                        Some("Int".into())
                    } else if matches!(
                        last,
                        "description"
                            | "localizedDescription"
                            | "capitalized"
                            | "uppercased"
                            | "lowercased"
                    ) && j > i
                    {
                        Some("String".into())
                    } else {
                        self.types.get(last).filter(|t| *t != "?").cloned()
                    };
                    types.push(ty);
                    i = j;
                }
                K::P('+' | '-' | '*' | '/' | '%' | '(' | ')') => {}
                _ => return ("%@".into(), false),
            }
            i += 1;
        }
        let first = types.first().cloned().flatten();
        match first {
            Some(ty)
                if types.iter().all(|t| {
                    t.as_deref() == Some(ty.as_str())
                        || (t.as_deref() == Some("Int") && ty != "String")
                }) =>
            {
                (type_spec(&ty).into(), true)
            }
            _ => ("%@".into(), false),
        }
    }
}

/// `scan` mapped to an `ExtractReport`: localized strings are key usages, plain UIKit strings are
/// hardcoded copy, type guesses are warnings.
pub fn report(
    path: &str,
    src: &str,
    fns: &[String],
    types: &HashMap<String, String>,
) -> ExtractReport {
    let s = scan(path, src, fns, types);
    let at = |line| SourceRef {
        path: path.to_string(),
        line,
    };
    let mut r = ExtractReport {
        files_scanned: 1,
        ..Default::default()
    };
    for x in s.strings {
        let role = match x.api.as_str() {
            "Button" | "UIAlertAction" | "setTitle" | "UIBarButtonItem" | "UIAction" => "button",
            "TextField" | "SecureField" | ".placeholder =" => "placeholder",
            "navigationTitle" | "Section" | "alert" | "confirmationDialog" => "title",
            "accessibilityLabel" | "accessibilityHint" => "aria",
            _ => "text",
        };
        if x.localized {
            r.usages.entry(x.key).or_default().push(Usage {
                at: at(x.line),
                ui_role: Some(role.into()),
                component: Some(x.api),
            });
        } else {
            r.hardcoded.push(Hardcoded {
                at: at(x.line),
                text: x.key.clone(),
                ui_role: Some(role.into()),
                component: Some(x.api),
                suggested_key: x.key,
                reason: Some("plain String: wrap in String(localized:)".into()),
            });
        }
    }
    r.dynamic.extend(s.dynamic.into_iter().map(at));
    r.dynamic_prefixes.extend(s.dynamic_prefixes);
    r.warnings.extend(
        s.warnings
            .into_iter()
            .map(|(l, w)| format!("{path}:{l}: {w}")),
    );
    if let Some(e) = s.error {
        r.warnings.push(format!("{path}: {e}, file skipped"));
    }
    r
}

/// Wrap the plain (non-localized) strings of `scan` in `String(localized: …)`. `None` if the
/// result no longer lexes (never write a file glot cannot read back).
pub fn rewrite(src: &str, strings: &[&SwiftString]) -> Option<String> {
    let mut out = src.to_string();
    let mut spans: Vec<(usize, usize)> = strings
        .iter()
        .filter(|s| !s.localized)
        .map(|s| (s.start, s.end))
        .collect();
    spans.sort();
    spans.dedup();
    for (a, b) in spans.into_iter().rev() {
        out.replace_range(a..b, &format!("String(localized: {})", &src[a..b]));
    }
    lex(&out, 0, out.len()).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = r##"import SwiftUI

struct Country { let name: String; let visaFree: Int }

struct HomeView: View {
    @State private var count = 3
    let score: Double
    var title: String { "unused" }
    var body: some View {
        NavigationStack {
            VStack {
                Text("Welcome back")
                Text("\(count) countries")
                Text("Score \(score, specifier: "%.1f")")
                Text("\(country.visaFree) visa-free, \(country.name)")
                Text("Rank \(mystery)")
                Text("100% sure \(count)")
                Text(verbatim: "debug-only")
                Text("\(count)")
                Text("Héllo \u{2192} \"quoted\"")
                Text(L10n.tr("Plan ahead"))
                Text(isOn ? "On" : "Off")
                Label("Settings", systemImage: "gear")
                Image("logo")
                Image(systemName: "star")
                Button("Save", role: .destructive) { save() }
                TextField("Email address", text: $email)
                Text("Hello", tableName: "Auth", comment: "greeting")
                Text("""
                    Multi
                      line \
                    joined
                    """)
                Text(#"Raw \n "quoted" \#(count) items"#)
                // Text("commented out")
                /* Text("block /* nested */ comment") */
            }
            .navigationTitle("Home")
            .alert("Delete trip?", isPresented: $show) {}
            .help("Shows help")
            .searchable(text: $q, prompt: "Search places")
        }
        .onAppear {
            print("Appeared")
            UserDefaults.standard.set(true, forKey: "didOnboard")
            label.text = "Plain UIKit text"
            button.setTitle("Tap me", for: .normal)
            let alert = UIAlertController(title: "Oops", message: "Something went wrong", preferredStyle: .alert)
            _ = NSLocalizedString("legacy.key", tableName: "Errors", comment: "legacy")
            _ = NSLocalizedString(dynamicKey, comment: "")
            _ = String(localized: "Trips: \(count)")
            _ = URL(string: "https://example.com")
        }
    }
}

enum L10n {
    enum Login {
        private static let table = "Auth"
        static var title: String { tr(table, "login.title", fallback: "Sign in") }
    }
}

#Preview {
    Text("Preview only")
}

struct HomeView_Previews: PreviewProvider {
    static var previews: some View { Text("Old preview") }
}
"##;

    fn keys(s: &SwiftScan) -> Vec<(&str, bool)> {
        s.strings
            .iter()
            .map(|x| (x.key.as_str(), x.localized))
            .collect()
    }

    #[test]
    fn swiftui_keys_and_exclusions() {
        let fns = vec!["L10n.tr".to_string()];
        let s = scan("App/HomeView.swift", APP, &fns, &HashMap::new());
        assert_eq!(s.error, None);
        let k = keys(&s);
        for want in [
            ("Welcome back", true),
            ("%lld countries", true),
            ("Score %.1f", true),
            ("%lld visa-free, %@", true),
            ("Rank %@", true),
            ("100%% sure %lld", true),
            ("Héllo → \"quoted\"", true),
            ("Plan ahead", true),
            ("On", true),
            ("Off", true),
            ("Settings", true),
            ("Save", true),
            ("Email address", true),
            ("Hello", true),
            ("Multi\n  line joined", true),
            ("Raw \\n \"quoted\" %lld items", true),
            ("Home", true),
            ("Delete trip?", true),
            ("Shows help", true),
            ("Search places", true),
            ("Plain UIKit text", false),
            ("Tap me", false),
            ("Oops", false),
            ("Something went wrong", false),
            ("legacy.key", true),
            ("Trips: %lld", true),
            ("login.title", true),
        ] {
            assert!(k.contains(&want), "missing {want:?} in {k:?}");
        }
        for never in [
            "debug-only",
            "%lld",
            "logo",
            "star",
            "gear",
            "Appeared",
            "didOnboard",
            "https://example.com",
            "commented out",
            "Preview only",
            "Old preview",
            "unused",
            "block /* nested */ comment",
        ] {
            assert!(!k.iter().any(|(x, _)| *x == never), "{never:?} extracted");
        }
        let get = |key: &str| s.strings.iter().find(|x| x.key == key).unwrap();
        assert_eq!(get("Hello").table.as_deref(), Some("Auth"));
        assert_eq!(get("Hello").comment.as_deref(), Some("greeting"));
        assert_eq!(get("legacy.key").table.as_deref(), Some("Errors"));
        assert_eq!(get("login.title").table.as_deref(), Some("Auth"));
        assert_eq!(get("login.title").value.as_deref(), Some("Sign in"));
        assert_eq!(get("Welcome back").line, 12);
        assert_eq!(s.dynamic.len(), 1);
        assert_eq!(s.warnings.len(), 1, "{:?}", s.warnings); // `mystery`
        assert!(s.warnings[0].1.contains("mystery"));
    }

    #[test]
    fn project_types_cross_file() {
        let model = "struct Entry { let rank: Int\n let ratio: Double\n let score: Int }\nstruct B { let score: Double }\nlet left = 3\nfunc f() { let left = q.left }";
        let view = r##"Text("#\(entry.rank) at \(entry.ratio) \(entry.score) \(left)")"##;
        let types = project_types([model]);
        let s = scan("V.swift", view, &[], &types);
        assert_eq!(keys(&s), [("#%lld at %lf %@ %@", true)]);
        assert_eq!(s.warnings.len(), 2); // `score` and `left` are ambiguous
    }

    #[test]
    fn tests_and_errors() {
        assert!(
            scan("AppTests/FooTests.swift", APP, &[], &HashMap::new())
                .strings
                .is_empty()
        );
        let bad = scan("x.swift", "Text(\"unterminated)\n", &[], &HashMap::new());
        assert!(bad.error.is_some() && bad.strings.is_empty());
        // `\(` nested in an interpolation with its own literal.
        let s = scan(
            "x.swift",
            r#"Text("Hi \(L10n.tr("Nested key"))")"#,
            &["L10n.tr".into()],
            &HashMap::new(),
        );
        assert!(keys(&s).contains(&("Nested key", true)), "{:?}", keys(&s));
        assert!(keys(&s).contains(&("Hi %@", true)));
        let s = scan(
            "x.swift",
            r#"L10n.tr("errors.\(code)")"#,
            &[],
            &HashMap::new(),
        );
        assert_eq!(s.dynamic_prefixes, ["errors."]);
    }

    #[test]
    fn rewrite_keeps_file_lexable() {
        let src = "label.text = \"Hello \\(name)\"\nlet x = \"id\"\n";
        let s = scan("x.swift", src, &[], &HashMap::new());
        let out = rewrite(src, &s.strings.iter().collect::<Vec<_>>()).unwrap();
        assert_eq!(
            out,
            "label.text = String(localized: \"Hello \\(name)\")\nlet x = \"id\"\n"
        );
        let again = scan("x.swift", &out, &[], &HashMap::new());
        assert_eq!(keys(&again), [("Hello %@", true)]);
    }
}
