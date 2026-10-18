use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::plural::{PluralCategory, categories};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Syntax {
    /// ICU MessageFormat 1 (next-intl, FormatJS, Lingui): `{name}`, `{n, plural, one {#} other {#}}`, `<b>rich</b>`.
    #[default]
    Icu,
    /// i18next: `{{name}}`, `$t(key)`, `_one`/`_other` suffixed keys, `<1>tags</1>`.
    I18next,
    /// Unicode MessageFormat 2.0.
    Mf2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    Empty,
    Untranslated,
    PlaceholderMissing,
    PlaceholderExtra,
    SyntaxError,
    PluralCategoryMissing,
    PluralCategoryExtra,
    /// ICU keyword (plural/select/other/one...) was translated.
    KeywordTranslated,
    TagMismatch,
    TooLong,
    WhitespaceMismatch,
    GlossaryViolation,
    DoNotTranslateViolation,
    LowScore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub code: IssueCode,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct Constraints {
    pub max_length: Option<usize>,
    /// target visible length must be <= ratio * source visible length (warning).
    pub max_length_ratio: Option<f32>,
}

/// Placeholder / argument names used by a message (`{name}`, `{{name}}`, `{$name}`), including those inside plurals/selects.
pub fn placeholders(text: &str, syntax: Syntax) -> Result<BTreeSet<String>, Issue> {
    parse(text, syntax)
        .map(|n| arg_names(&n))
        .map_err(|e| syntax_issue(&e, syntax))
}

fn arg_names(nodes: &[Node]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    walk(nodes, &mut |n| match n {
        Node::Arg { name, .. } | Node::Choice(Choice { name, .. }) => {
            names.insert(name.clone());
        }
        _ => {}
    });
    names
}

/// Validate a translation against its source. Errors must block; warnings are surfaced.
/// Checks: empty, syntax, placeholders, plural categories required by `target_locale`,
/// translated keywords, tags, length, leading/trailing whitespace, untranslated (identical, warning).
pub fn validate(
    source: &str,
    target: &str,
    target_locale: &str,
    syntax: Syntax,
    constraints: &Constraints,
) -> Vec<Issue> {
    let mut out = Vec::new();
    if target.trim().is_empty() {
        if !source.trim().is_empty() {
            out.push(error(
                IssueCode::Empty,
                "The translation is empty; translate the source text.".into(),
            ));
        }
        return out;
    }
    let tgt = match parse(target, syntax) {
        Ok(n) => n,
        Err(e) => return vec![syntax_issue(&e, syntax)],
    };
    // A broken source can't be compared structurally; still run the plain-text checks.
    let src = parse(source, syntax).ok();
    if let Some(src) = &src {
        Checker {
            syntax,
            locale: target_locale,
            src,
            tgt: &tgt,
            out: &mut out,
        }
        .run();
    }
    let src_nodes = src.unwrap_or_else(|| vec![Node::Text(source.into())]);
    let (src_len, tgt_len) = (visible_len(&src_nodes), visible_len(&tgt));
    if let Some(max) = constraints.max_length
        && tgt_len > max
    {
        out.push(error(
            IssueCode::TooLong,
            format!("Too long: {tgt_len} visible characters, the limit is {max}. Use a shorter wording (placeholders and markup don't count)."),
        ));
    }
    if let Some(ratio) = constraints.max_length_ratio
        && src_len > 0
        // +3 chars of slack: short labels ("Save" -> "Enregistrer") legitimately grow a lot.
        && tgt_len as f32 > ratio * src_len as f32 + 3.0
    {
        out.push(warning(
            IssueCode::TooLong,
            format!("The translation has {tgt_len} visible characters vs {src_len} in the source (over {ratio}x); a shorter wording is preferred to avoid UI overflow."),
        ));
    }
    let lead = |s: &str| s[..s.len() - s.trim_start().len()].to_string();
    let trail = |s: &str| s[s.trim_end().len()..].to_string();
    for (side, a, b) in [
        ("starts", lead(source), lead(target)),
        ("ends", trail(source), trail(target)),
    ] {
        if a != b {
            out.push(warning(
                IssueCode::WhitespaceMismatch,
                format!("Whitespace mismatch: the source {side} with {a:?} but the translation {side} with {b:?}; keep the same leading/trailing whitespace."),
            ));
        }
    }
    if source == target && visible_text(&src_nodes).chars().any(char::is_alphabetic) {
        out.push(warning(
            IssueCode::Untranslated,
            "The translation is identical to the source; translate it, unless it is a name or term that must stay as is.".into(),
        ));
    }
    out
}

pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Error)
}

fn error(code: IssueCode, message: String) -> Issue {
    Issue {
        code,
        severity: Severity::Error,
        message,
    }
}

fn warning(code: IssueCode, message: String) -> Issue {
    Issue {
        code,
        severity: Severity::Warning,
        message,
    }
}

fn syntax_issue(e: &str, syntax: Syntax) -> Issue {
    let name = match syntax {
        Syntax::Icu => "ICU MessageFormat",
        Syntax::I18next => "i18next",
        Syntax::Mf2 => "MessageFormat 2",
    };
    error(
        IssueCode::SyntaxError,
        format!("Invalid {name} syntax: {e}."),
    )
}

// ---------------------------------------------------------------- AST

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Text(String),
    /// `{name}` / `{name, number, percent}` / `{{name, fmt}}` / `{$name :number}`.
    Arg {
        name: String,
        format: Option<String>,
    },
    Choice(Choice),
    Pound,
    Open(String),
    Close(String),
    /// Self-closing tag `<br/>`, `{#img/}`.
    Void(String),
    /// i18next nesting `$t(key)`.
    Ref(String),
}

#[derive(Debug, Clone, PartialEq)]
struct Choice {
    name: String,
    /// As written: `plural`, `pluriel`, MF2 function `number`.
    keyword: String,
    kind: Kind,
    options: Vec<(String, Vec<Node>)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Plural,
    Ordinal,
    Select,
    /// ICU argument with options but an unknown type (translated keyword).
    Unknown,
}

fn walk<'a>(nodes: &'a [Node], f: &mut impl FnMut(&'a Node)) {
    for n in nodes {
        f(n);
        if let Node::Choice(c) = n {
            for (_, body) in &c.options {
                walk(body, f);
            }
        }
    }
}

/// Length of the rendered text, ignoring syntax; for choices, the longest branch.
fn visible_len(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Text(t) => t.chars().count(),
            Node::Choice(c) => c
                .options
                .iter()
                .map(|(_, b)| visible_len(b))
                .max()
                .unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn visible_text(nodes: &[Node]) -> String {
    let mut s = String::new();
    walk(nodes, &mut |n| {
        if let Node::Text(t) = n {
            s.push_str(t);
        }
    });
    s
}

fn parse(text: &str, syntax: Syntax) -> Result<Vec<Node>, String> {
    let mut p = Parser {
        s: text.chars().collect(),
        i: 0,
        locals: BTreeMap::new(),
    };
    let nodes = match syntax {
        Syntax::Icu => p.icu_seq(0, false)?,
        Syntax::I18next => p.i18n_seq()?,
        Syntax::Mf2 => p.mf2()?,
    };
    check_tags(&nodes)?;
    Ok(nodes)
}

/// Tags must be balanced and properly nested within each message / branch.
fn check_tags(nodes: &[Node]) -> Result<(), String> {
    let mut stack: Vec<&str> = Vec::new();
    for n in nodes {
        match n {
            Node::Open(t) => stack.push(t),
            Node::Close(t) => match stack.pop() {
                Some(open) if open == t => {}
                Some(open) => {
                    return Err(format!(
                        "tag <{open}> is closed by </{t}>; close tags in the reverse order they were opened"
                    ));
                }
                None => return Err(format!("closing tag </{t}> has no matching <{t}>")),
            },
            Node::Choice(c) => c.options.iter().try_for_each(|(_, b)| check_tags(b))?,
            _ => {}
        }
    }
    match stack.pop() {
        Some(open) => Err(format!("tag <{open}> is never closed; add </{open}>")),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------- parsers

struct Parser {
    s: Vec<char>,
    i: usize,
    /// MF2 `.local` variables => the external variable they derive from, and their function.
    locals: BTreeMap<String, (Option<String>, Option<String>)>,
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.s.get(self.i + n).copied()
    }

    fn starts(&self, pat: &str) -> bool {
        pat.chars()
            .enumerate()
            .all(|(k, c)| self.peek_at(k) == Some(c))
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        self.i += c.is_some() as usize;
        c
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn take_while(&mut self, f: impl Fn(char) -> bool) -> String {
        let start = self.i;
        while self.peek().is_some_and(&f) {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }

    fn err<T>(&self, msg: impl std::fmt::Display) -> Result<T, String> {
        let from = self.i.saturating_sub(15);
        let near: String = self.s[from..(self.i + 15).min(self.s.len())]
            .iter()
            .collect();
        Err(format!("{msg} (at character {}, near `{near}`)", self.i))
    }

    fn expect(&mut self, c: char, what: &str) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(what)
        }
    }

    /// HTML-like tag at the cursor: `<b>`, `</b>`, `<br/>`, `<0>`. Returns the node and its length.
    fn tag(&self) -> Option<(Node, usize)> {
        let mut j = self.i + 1;
        let close = self.s.get(j) == Some(&'/');
        j += close as usize;
        let start = j;
        while self
            .s
            .get(j)
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            j += 1;
        }
        if j == start {
            return None;
        }
        let name: String = self.s[start..j].iter().collect();
        while self.s.get(j) == Some(&' ') {
            j += 1;
        }
        match (self.s.get(j), self.s.get(j + 1)) {
            (Some('/'), Some('>')) if !close => Some((Node::Void(name), j + 2 - self.i)),
            (Some('>'), _) if close => Some((Node::Close(name), j + 1 - self.i)),
            (Some('>'), _) => Some((Node::Open(name), j + 1 - self.i)),
            _ => None,
        }
    }

    fn push_tag_or_text(&mut self, out: &mut Vec<Node>, text: &mut String) {
        match self.tag() {
            Some((node, len)) => {
                flush(out, text);
                out.push(node);
                self.i += len;
            }
            None => {
                text.push('<');
                self.i += 1;
            }
        }
    }

    // ---- ICU MessageFormat 1 (FormatJS flavour: tags, DOUBLE_OPTIONAL apostrophes)

    fn icu_seq(&mut self, depth: usize, plural: bool) -> Result<Vec<Node>, String> {
        let (mut out, mut text) = (Vec::new(), String::new());
        while let Some(c) = self.peek() {
            match c {
                '{' => {
                    flush(&mut out, &mut text);
                    out.push(self.icu_arg(depth, plural)?);
                }
                '}' if depth == 0 => {
                    return self.err("unmatched `}`; a literal brace must be quoted as '}'");
                }
                '}' => break,
                '#' if plural => {
                    flush(&mut out, &mut text);
                    out.push(Node::Pound);
                    self.i += 1;
                }
                '<' => self.push_tag_or_text(&mut out, &mut text),
                '\'' => self.icu_quote(&mut text, plural),
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut out, &mut text);
        Ok(out)
    }

    fn icu_quote(&mut self, text: &mut String, plural: bool) {
        self.i += 1;
        match self.peek() {
            Some('\'') => {
                text.push('\'');
                self.i += 1;
            }
            Some(c) if matches!(c, '{' | '}' | '<' | '>') || (plural && c == '#') => {
                while let Some(c) = self.bump() {
                    if c != '\'' {
                        text.push(c);
                    } else if self.peek() == Some('\'') {
                        text.push('\'');
                        self.i += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => text.push('\''),
        }
    }

    fn icu_arg(&mut self, depth: usize, plural: bool) -> Result<Node, String> {
        self.i += 1;
        self.ws();
        let name = self
            .take_while(|c| !c.is_whitespace() && !matches!(c, ',' | '{' | '}' | '\'' | '#' | '<'));
        if name.is_empty() {
            return self.err(
                "empty or invalid argument: `{` must be followed by an argument name like `{name}`",
            );
        }
        self.ws();
        match self.bump() {
            Some('}') => return Ok(Node::Arg { name, format: None }),
            Some(',') => {}
            None => return self.err(format!("unclosed argument `{{{name}`; add `}}`")),
            Some(c) => {
                return self.err(format!(
                    "unexpected `{c}` after argument name `{name}`; expected `,` or `}}`"
                ));
            }
        }
        self.ws();
        let ty = self.take_while(char::is_alphanumeric);
        if ty.is_empty() {
            return self.err(format!("missing argument type after `{{{name},`"));
        }
        self.ws();
        match self.peek() {
            Some('}') => {
                self.i += 1;
                return Ok(Node::Arg {
                    name,
                    format: Some(ty),
                });
            }
            Some(',') => self.i += 1,
            _ => return self.err(format!("expected `,` or `}}` after `{{{name}, {ty}`")),
        }
        let kind = match ty.as_str() {
            "plural" => Kind::Plural,
            "selectordinal" => Kind::Ordinal,
            "select" => Kind::Select,
            "number" | "date" | "time" | "spellout" | "ordinal" | "duration" => {
                return self.icu_style(name, ty);
            }
            _ => Kind::Unknown,
        };
        let save = self.i;
        match self.icu_options(name.clone(), ty.clone(), kind, depth, plural) {
            Err(_) if kind == Kind::Unknown => {
                self.i = save;
                self.icu_style(name, ty)
            }
            r => r,
        }
    }

    fn icu_style(&mut self, name: String, ty: String) -> Result<Node, String> {
        let start = self.i;
        let mut depth = 0;
        loop {
            match self.bump() {
                None => {
                    return self.err(format!("unclosed argument `{{{name}, {ty}, …`; add `}}`"));
                }
                Some('{') => depth += 1,
                Some('}') if depth == 0 => break,
                Some('}') => depth -= 1,
                _ => {}
            }
        }
        let style: String = self.s[start..self.i - 1].iter().collect();
        Ok(Node::Arg {
            name,
            format: Some(format!("{ty}, {}", style.trim())),
        })
    }

    fn icu_options(
        &mut self,
        name: String,
        keyword: String,
        kind: Kind,
        depth: usize,
        plural: bool,
    ) -> Result<Node, String> {
        self.ws();
        if kind != Kind::Select && self.starts("offset:") {
            self.i += 7;
            self.ws();
            if self.take_while(|c| c.is_ascii_digit()).is_empty() {
                return self.err("`offset:` must be followed by a number");
            }
        }
        let mut options: Vec<(String, Vec<Node>)> = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                Some('}') => {
                    self.i += 1;
                    break;
                }
                None => {
                    return self.err(format!(
                        "unclosed `{{{name}, {keyword}, …`; add the final `}}`"
                    ));
                }
                _ => {}
            }
            let sel = self.take_while(|c| !c.is_whitespace() && !matches!(c, '{' | '}'));
            if sel.is_empty() {
                return self.err(format!(
                    "expected an option selector in `{{{name}, {keyword}, …}}`"
                ));
            }
            self.ws();
            self.expect(
                '{',
                &format!("expected `{{` after selector `{sel}` in `{{{name}, {keyword}, …}}`"),
            )?;
            let body = self.icu_seq(depth + 1, plural || kind != Kind::Select)?;
            self.expect(
                '}',
                &format!("unclosed option `{sel} {{…` in `{{{name}, {keyword}, …}}`; add `}}`"),
            )?;
            if options.iter().any(|(s, _)| *s == sel) {
                return self.err(format!(
                    "duplicate selector `{sel}` in `{{{name}, {keyword}, …}}`"
                ));
            }
            options.push((sel, body));
        }
        if options.is_empty() {
            return self.err(format!("`{{{name}, {keyword}, …}}` has no options"));
        }
        Ok(Node::Choice(Choice {
            name,
            keyword,
            kind,
            options,
        }))
    }

    // ---- i18next

    fn i18n_seq(&mut self) -> Result<Vec<Node>, String> {
        let (mut out, mut text) = (Vec::new(), String::new());
        while let Some(c) = self.peek() {
            if self.starts("{{") {
                flush(&mut out, &mut text);
                self.i += 2;
                let start = self.i;
                while !self.starts("}}") {
                    if self.bump().is_none() {
                        self.i = start;
                        return self.err("unclosed `{{`; interpolations look like `{{name}}`");
                    }
                }
                let inner: String = self.s[start..self.i].iter().collect();
                self.i += 2;
                let inner = inner.trim().trim_start_matches('-').trim();
                let (name, format) = match inner.split_once(',') {
                    Some((n, f)) => (n.trim(), Some(f.trim().to_string())),
                    None => (inner, None),
                };
                if name.is_empty() {
                    return self.err("empty interpolation `{{}}`");
                }
                out.push(Node::Arg {
                    name: name.into(),
                    format,
                });
            } else if self.starts("$t(") {
                flush(&mut out, &mut text);
                self.i += 3;
                let start = self.i;
                let mut depth = 0;
                loop {
                    match self.bump() {
                        None => return self.err("unclosed `$t(`; add `)`"),
                        Some('(') => depth += 1,
                        Some(')') if depth == 0 => break,
                        Some(')') => depth -= 1,
                        _ => {}
                    }
                }
                let inner: String = self.s[start..self.i - 1].iter().collect();
                let key = inner
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .trim_matches(['"', '\'']);
                out.push(Node::Ref(key.into()));
            } else if c == '<' {
                self.push_tag_or_text(&mut out, &mut text);
            } else {
                text.push(c);
                self.i += 1;
            }
        }
        flush(&mut out, &mut text);
        Ok(out)
    }

    // ---- MessageFormat 2

    fn mf2(&mut self) -> Result<Vec<Node>, String> {
        self.ws();
        if !(self.peek() == Some('.') || self.starts("{{")) {
            self.i = 0;
            return self.mf2_pattern(false);
        }
        let mut out = Vec::new();
        // var => function, from `.input`.
        let mut fns: BTreeMap<String, Option<String>> = BTreeMap::new();
        loop {
            self.ws();
            if self.starts(".input") {
                self.i += 6;
                self.ws();
                if self.peek() != Some('{') {
                    return self
                        .err("`.input` must be followed by an expression like `{$count :number}`");
                }
                let e = self.mf2_expr()?;
                let Some(var) = e.var else {
                    return self.err("`.input` expression must start with a `$variable`");
                };
                fns.insert(var.clone(), e.func.clone());
                out.push(Node::Arg {
                    name: var,
                    format: e.func,
                });
                out.extend(
                    e.vars
                        .into_iter()
                        .map(|name| Node::Arg { name, format: None }),
                );
            } else if self.starts(".local") {
                self.i += 6;
                self.ws();
                self.expect('$', "`.local` must be followed by `$name = {…}`")?;
                let local = self.take_while(is_name_char);
                self.ws();
                self.expect('=', &format!("expected `=` after `.local ${local}`"))?;
                self.ws();
                if self.peek() != Some('{') {
                    return self.err(format!(
                        "expected an expression `{{…}}` after `.local ${local} =`"
                    ));
                }
                let e = self.mf2_expr()?;
                let func = e
                    .func
                    .clone()
                    .or_else(|| e.var.as_ref().and_then(|v| fns.get(v).cloned().flatten()));
                if let Some(v) = &e.var {
                    out.push(Node::Arg {
                        name: v.clone(),
                        format: e.func.clone(),
                    });
                }
                out.extend(
                    e.vars
                        .into_iter()
                        .map(|name| Node::Arg { name, format: None }),
                );
                self.locals.insert(local, (e.var, func));
            } else if self.starts(".match") {
                self.i += 6;
                out.extend(self.mf2_match(&fns)?);
                break;
            } else if self.starts("{{") {
                self.i += 2;
                out.extend(self.mf2_pattern(true)?);
                self.i += 2;
                break;
            } else if self.peek() == Some('.') {
                let kw = self.take_while(|c| !c.is_whitespace());
                return self.err(format!(
                    "unknown statement `{kw}`; expected `.input`, `.local` or `.match`"
                ));
            } else {
                return self
                    .err("a complex message must end with a quoted pattern `{{…}}` or a `.match`");
            }
        }
        self.ws();
        if self.peek().is_some() {
            return self.err("unexpected content after the end of the message");
        }
        Ok(out)
    }

    fn mf2_match(&mut self, fns: &BTreeMap<String, Option<String>>) -> Result<Vec<Node>, String> {
        // Selectors: `$var` (final spec) or `{$var :fn}` (older drafts).
        let mut sels: Vec<(String, Option<String>, bool)> = Vec::new();
        let mut extra = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                Some('$') => {
                    self.i += 1;
                    let v = self.take_while(is_name_char);
                    let (name, func) = match self.locals.get(&v) {
                        Some((src, f)) => (src.clone().unwrap_or(v), f.clone()),
                        None => (v.clone(), fns.get(&v).cloned().flatten()),
                    };
                    sels.push((name, func, false));
                }
                Some('{') if !self.starts("{{") => {
                    let start = self.i;
                    let e = self.mf2_expr()?;
                    let raw: String = self.s[start..self.i].iter().collect();
                    let Some(v) = e.var else {
                        return self.err("a `.match` selector must be a variable");
                    };
                    extra.push(Node::Arg {
                        name: v.clone(),
                        format: e.func.clone(),
                    });
                    sels.push((v, e.func, raw.contains("select=ordinal")));
                }
                _ => break,
            }
        }
        if sels.is_empty() {
            return self.err("`.match` needs at least one selector like `.match $count`");
        }
        let mut variants: Vec<(Vec<String>, Vec<Node>)> = Vec::new();
        loop {
            self.ws();
            if self.peek().is_none() {
                break;
            }
            let mut keys = Vec::new();
            while !self.starts("{{") {
                match self.peek() {
                    Some('*') => {
                        self.i += 1;
                        keys.push("*".to_string());
                    }
                    Some('|') => keys.push(self.mf2_quoted()?),
                    Some(c) if is_name_char(c) => keys.push(self.take_while(is_name_char)),
                    None => return self.err("expected a variant pattern `{{…}}` after the keys"),
                    Some(c) => return self.err(format!("unexpected `{c}` in variant keys")),
                }
                self.ws();
            }
            if keys.len() != sels.len() {
                return self.err(format!(
                    "variant has {} key(s) but `.match` has {} selector(s)",
                    keys.len(),
                    sels.len()
                ));
            }
            self.i += 2;
            let pat = self.mf2_pattern(true)?;
            self.i += 2;
            variants.push((keys, pat));
        }
        if variants.is_empty() {
            return self.err("`.match` has no variants; add at least `* {{…}}`");
        }
        // One choice per selector; only the first carries the patterns (lengths/placeholders counted once).
        let mut choices: Vec<Node> = sels
            .into_iter()
            .enumerate()
            .map(|(col, (name, func, ordinal))| {
                let keyword = func.clone().unwrap_or_default();
                let numeric = matches!(
                    func.as_deref(),
                    Some("number" | "integer" | "plural" | "ordinal")
                );
                let kind = match () {
                    _ if ordinal || func.as_deref() == Some("ordinal") => Kind::Ordinal,
                    _ if numeric => Kind::Plural,
                    _ => Kind::Select,
                };
                let mut options = variants
                    .iter()
                    .map(|(keys, pat)| {
                        let k = if keys[col] == "*" {
                            "other".to_string()
                        } else {
                            keys[col].clone()
                        };
                        (k, if col == 0 { pat.clone() } else { Vec::new() })
                    })
                    .collect::<Vec<_>>();
                // Multi-selector messages repeat keys per column.
                let mut seen = BTreeSet::new();
                if col > 0 {
                    options.retain(|(k, _)| seen.insert(k.clone()));
                }
                Node::Choice(Choice {
                    name,
                    keyword,
                    kind,
                    options,
                })
            })
            .collect();
        choices.extend(extra);
        Ok(choices)
    }

    fn mf2_quoted(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return self.err("unclosed quoted literal `|…`; add `|`"),
                Some('|') => return Ok(s),
                Some('\\') => s.extend(self.bump()),
                Some(c) => s.push(c),
            }
        }
    }

    fn mf2_pattern(&mut self, quoted: bool) -> Result<Vec<Node>, String> {
        let (mut out, mut text) = (Vec::new(), String::new());
        loop {
            match self.peek() {
                None if quoted => return self.err("unclosed quoted pattern `{{`; add `}}`"),
                None => break,
                Some('}') if quoted && self.starts("}}") => break,
                Some('}') => return self.err("unescaped `}`; write `\\}` for a literal brace"),
                Some('\\') => {
                    self.i += 1;
                    match self.bump() {
                        Some(c @ ('\\' | '{' | '}' | '|')) => text.push(c),
                        _ => {
                            self.i -= 1;
                            return self.err(
                                "invalid escape; only `\\\\`, `\\{`, `\\}` and `\\|` are allowed",
                            );
                        }
                    }
                }
                Some('{') => {
                    flush(&mut out, &mut text);
                    let e = self.mf2_expr()?;
                    if let Some(m) = e.markup {
                        out.push(m);
                    }
                    if let Some(l) = e.literal {
                        out.push(Node::Text(l));
                    }
                    if let Some(v) = e.var.filter(|v| !self.locals.contains_key(v)) {
                        out.push(Node::Arg {
                            name: v,
                            format: e.func,
                        });
                    }
                    out.extend(
                        e.vars
                            .into_iter()
                            .filter(|v| !self.locals.contains_key(v))
                            .map(|name| Node::Arg { name, format: None }),
                    );
                }
                Some(c) => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut out, &mut text);
        Ok(out)
    }

    /// `{$var :fn opt=$x @attr}`, `{|literal|}`, `{:fn}`, `{#tag}`, `{/tag}`, `{#tag/}`.
    fn mf2_expr(&mut self) -> Result<Expr, String> {
        self.i += 1;
        self.ws();
        let mut e = Expr::default();
        let markup = match self.peek() {
            Some(c @ ('#' | '/')) => {
                self.i += 1;
                let name = self.take_while(is_name_char);
                if name.is_empty() {
                    return self.err(format!("markup `{{{c}` needs a name like `{{{c}b}}`"));
                }
                Some((c, name))
            }
            Some('$') => {
                self.i += 1;
                let v = self.take_while(is_name_char);
                if v.is_empty() {
                    return self.err("`$` must be followed by a variable name");
                }
                e.var = Some(v);
                None
            }
            Some('|') => {
                e.literal = Some(self.mf2_quoted()?);
                None
            }
            Some(c) if is_name_char(c) => {
                e.literal = Some(self.take_while(is_name_char));
                None
            }
            _ => None,
        };
        let mut self_closing = false;
        loop {
            self.ws();
            match self.peek() {
                Some('}') => {
                    self.i += 1;
                    break;
                }
                None => return self.err("unclosed placeholder `{`; add `}`"),
                Some(':') => {
                    self.i += 1;
                    let f = self.take_while(|c| is_name_char(c) || c == ':');
                    if f.is_empty() {
                        return self.err("`:` must be followed by a function name like `:number`");
                    }
                    e.func.get_or_insert(f);
                }
                Some('$') => {
                    self.i += 1;
                    e.vars.push(self.take_while(is_name_char));
                }
                Some('|') => {
                    self.mf2_quoted()?;
                }
                Some('/') if markup.is_some() => {
                    self.i += 1;
                    self_closing = true;
                }
                Some('=' | '@') => self.i += 1,
                Some(c) if is_name_char(c) => {
                    self.take_while(is_name_char);
                }
                Some(c) => return self.err(format!("unexpected `{c}` inside a placeholder")),
            }
        }
        e.markup = markup.map(|(c, name)| match (c, self_closing) {
            ('/', _) => Node::Close(name),
            (_, true) => Node::Void(name),
            _ => Node::Open(name),
        });
        Ok(e)
    }
}

#[derive(Default)]
struct Expr {
    var: Option<String>,
    func: Option<String>,
    /// Variables used as option values.
    vars: Vec<String>,
    literal: Option<String>,
    markup: Option<Node>,
}

fn flush(out: &mut Vec<Node>, text: &mut String) {
    if !text.is_empty() {
        out.push(Node::Text(std::mem::take(text)));
    }
}

// ---------------------------------------------------------------- structural checks

struct Checker<'a> {
    syntax: Syntax,
    locale: &'a str,
    src: &'a [Node],
    tgt: &'a [Node],
    out: &'a mut Vec<Issue>,
}

const CATEGORIES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

impl Checker<'_> {
    fn ph(&self, name: &str) -> String {
        match self.syntax {
            Syntax::Icu => format!("{{{name}}}"),
            Syntax::I18next => format!("{{{{{name}}}}}"),
            Syntax::Mf2 => format!("{{${name}}}"),
        }
    }

    fn run(&mut self) {
        self.placeholders();
        self.refs();
        self.tags();
        self.formats();
        self.choices();
    }

    fn placeholders(&mut self) {
        let (src, tgt) = (arg_names(self.src), arg_names(self.tgt));
        let list = src
            .iter()
            .map(|n| self.ph(n))
            .collect::<Vec<_>>()
            .join(", ");
        for n in src.difference(&tgt) {
            let ph = self.ph(n);
            self.out.push(error(
                IssueCode::PlaceholderMissing,
                format!("Placeholder `{ph}` from the source is missing; keep it exactly as `{ph}` (never translate placeholder names)."),
            ));
        }
        for n in tgt.difference(&src) {
            let ph = self.ph(n);
            let hint = if list.is_empty() {
                "the source has no placeholders".to_string()
            } else {
                format!("the source only has {list}")
            };
            self.out.push(error(
                IssueCode::PlaceholderExtra,
                format!("Placeholder `{ph}` does not exist in the source; remove it or fix its name ({hint})."),
            ));
        }
    }

    fn refs(&mut self) {
        let refs = |nodes| {
            let mut v = Vec::new();
            walk(nodes, &mut |n| {
                if let Node::Ref(k) = n {
                    v.push(k.clone());
                }
            });
            v.sort();
            v
        };
        let (src, tgt) = (refs(self.src), refs(self.tgt));
        if src != tgt {
            let fmt = |v: &[String]| {
                v.iter()
                    .map(|k| format!("$t({k})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            self.out.push(error(
                IssueCode::PlaceholderMissing,
                format!("Nested translations differ: the source uses [{}] but the translation uses [{}]; keep `$t(...)` references unchanged.", fmt(&src), fmt(&tgt)),
            ));
        }
    }

    fn tags(&mut self) {
        let syntax = self.syntax;
        let tags = |nodes| {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            walk(nodes, &mut |n| match n {
                Node::Open(t) => *m.entry(open_tag(syntax, t, false)).or_default() += 1,
                Node::Void(t) => *m.entry(open_tag(syntax, t, true)).or_default() += 1,
                _ => {}
            });
            m
        };
        let (src, tgt) = (tags(self.src), tags(self.tgt));
        if src == tgt {
            return;
        }
        let diff = |a: &BTreeMap<String, usize>, b: &BTreeMap<String, usize>| {
            a.iter()
                .filter(|(t, n)| b.get(*t).unwrap_or(&0) < n)
                .map(|(t, _)| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (missing, extra) = (diff(&src, &tgt), diff(&tgt, &src));
        let mut msg = "Markup tags differ from the source:".to_string();
        if !missing.is_empty() {
            msg += &format!(" missing {missing};");
        }
        if !extra.is_empty() {
            msg += &format!(" unexpected {extra};");
        }
        msg += " keep exactly the same tags (translate only the text between them).";
        self.out.push(error(IssueCode::TagMismatch, msg));
    }

    fn formats(&mut self) {
        let mut src_fmt: BTreeMap<&str, &str> = BTreeMap::new();
        walk(self.src, &mut |n| {
            if let Node::Arg {
                name,
                format: Some(f),
            } = n
            {
                src_fmt.insert(name, f);
            }
        });
        let mut issues = Vec::new();
        walk(self.tgt, &mut |n| {
            let Node::Arg {
                name,
                format: Some(f),
            } = n
            else {
                return;
            };
            let ty = f.split(',').next().unwrap_or_default().trim();
            let known = [
                "number",
                "date",
                "time",
                "spellout",
                "ordinal",
                "duration",
                "plural",
                "select",
                "selectordinal",
            ];
            let expected = src_fmt.get(name.as_str());
            if self.syntax == Syntax::Icu && !known.contains(&ty) {
                let hint = expected
                    .map(|e| format!("use `{{{name}, {e}}}` as in the source"))
                    .unwrap_or_else(|| format!("valid types: {}", known.join(", ")));
                issues.push(format!("`{{{name}, {f}}}`: `{ty}` is not an ICU argument type (translated keyword?); {hint}."));
            } else if let Some(e) = expected.filter(|e| *e != f) {
                issues.push(format!("The format of `{}` changed from `{e}` to `{f}`; formatting keywords must not be translated, keep `{e}`.", self.ph(name)));
            }
        });
        self.out.extend(
            issues
                .into_iter()
                .map(|m| error(IssueCode::KeywordTranslated, m)),
        );
    }

    fn choices(&mut self) {
        let collect = |nodes| {
            let mut v: Vec<&Choice> = Vec::new();
            walk(nodes, &mut |n| {
                if let Node::Choice(c) = n {
                    v.push(c);
                }
            });
            v
        };
        let (src, tgt) = (collect(self.src), collect(self.tgt));
        let required: Vec<&str> = categories(self.locale)
            .iter()
            .map(PluralCategory::as_str)
            .collect();
        let is_mf2 = self.syntax == Syntax::Mf2;
        let other = if is_mf2 { "`*`" } else { "`other`" };
        let mut issues = Vec::new();

        for c in &tgt {
            let same: Vec<&&Choice> = src.iter().filter(|s| s.name == c.name).collect();
            let shown = if is_mf2 {
                format!("`.match ${}`", c.name)
            } else {
                format!("`{{{}, {}, …}}`", c.name, c.keyword)
            };
            let keys: Vec<&str> = c.options.iter().map(|(k, _)| k.as_str()).collect();
            if c.kind == Kind::Unknown {
                let fix = same
                    .first()
                    .map(|s| format!("`{{{}, {}, …}}`", c.name, s.keyword))
                    .unwrap_or_else(|| "`plural`, `select` or `selectordinal`".into());
                issues.push(error(
                    IssueCode::KeywordTranslated,
                    format!("{shown}: `{}` is a translated ICU keyword; keywords must stay in English, use {fix}.", c.keyword),
                ));
                continue;
            }
            if !keys.contains(&"other") {
                issues.push(error(
                    IssueCode::PluralCategoryMissing,
                    format!("{shown} is missing the mandatory {other} option."),
                ));
            }
            match c.kind {
                Kind::Plural | Kind::Ordinal => {
                    let exact = |k: &str| {
                        k.strip_prefix('=')
                            .or(if is_mf2 { Some(k) } else { None })
                            .is_some_and(|n| n.parse::<f64>().is_ok())
                    };
                    let bad: Vec<&str> = keys
                        .iter()
                        .copied()
                        .filter(|k| !CATEGORIES.contains(k) && !exact(k))
                        .collect();
                    if !bad.is_empty() {
                        let ex = if is_mf2 {
                            "exact numbers like `0`"
                        } else {
                            "exact values like `=0`"
                        };
                        issues.push(error(
                            IssueCode::KeywordTranslated,
                            format!(
                                "{shown}: {} not valid plural selector(s); selectors must stay untranslated CLDR keywords (zero, one, two, few, many, other) or {ex}. {} needs: {}.",
                                bad.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "),
                                self.locale,
                                required.join(", ")
                            ),
                        ));
                    }
                    if c.kind == Kind::Ordinal {
                        continue;
                    }
                    let missing: Vec<&str> = required
                        .iter()
                        .copied()
                        .filter(|r| *r != "other" && !keys.contains(r))
                        .collect();
                    // CLDR `many` in fr/es/it/pt/ca only applies to 1e6+ compact numbers: warn, don't block.
                    let (soft, missing): (Vec<&str>, Vec<&str>) =
                        missing.into_iter().partition(|m| {
                            *m == "many"
                                && matches!(
                                    crate::plural::language(self.locale).as_str(),
                                    "fr" | "es" | "it" | "pt" | "ca" | "vec"
                                )
                        });
                    if !soft.is_empty() {
                        issues.push(warning(
                            IssueCode::PluralCategoryMissing,
                            format!("{shown} has no `many {{…}}` branch; {} only uses it for very large numbers (1 000 000+), add it if such counts can appear.", self.locale),
                        ));
                    }
                    if !missing.is_empty() {
                        issues.push(error(
                            IssueCode::PluralCategoryMissing,
                            format!(
                                "{shown} is missing plural categories required by {}: {}. Add {} (all required: {}).",
                                self.locale,
                                missing.join(", "),
                                missing.iter().map(|m| if is_mf2 { format!("`{m} {{{{…}}}}`") } else { format!("`{m} {{…}}`") }).collect::<Vec<_>>().join(", "),
                                required.join(", ")
                            ),
                        ));
                    }
                    let extra: Vec<&str> = keys
                        .iter()
                        .copied()
                        .filter(|k| CATEGORIES.contains(k) && !required.contains(k))
                        .collect();
                    if !extra.is_empty() {
                        issues.push(warning(
                            IssueCode::PluralCategoryExtra,
                            format!("{shown} has categories {} never uses: {}; they can be removed (required: {}).", self.locale, extra.join(", "), required.join(", ")),
                        ));
                    }
                    let src_exact: BTreeSet<&str> = same
                        .iter()
                        .flat_map(|s| s.options.iter())
                        .map(|(k, _)| k.as_str())
                        .filter(|k| exact(k))
                        .collect();
                    let lost: Vec<&str> = src_exact
                        .into_iter()
                        .filter(|k| !keys.contains(k))
                        .collect();
                    if !lost.is_empty() {
                        issues.push(error(
                            IssueCode::PluralCategoryMissing,
                            format!("{shown} dropped the exact selector(s) {} present in the source; keep them.", lost.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")),
                        ));
                    }
                }
                Kind::Select => {
                    let src_keys: BTreeSet<&str> = same
                        .iter()
                        .flat_map(|s| s.options.iter())
                        .map(|(k, _)| k.as_str())
                        .collect();
                    let bad: Vec<&str> = keys
                        .iter()
                        .copied()
                        .filter(|k| !src_keys.is_empty() && !src_keys.contains(k))
                        .collect();
                    if !bad.is_empty() {
                        issues.push(error(
                            IssueCode::KeywordTranslated,
                            format!(
                                "{shown}: option(s) {} do not exist in the source; select keys must stay untranslated: {}.",
                                bad.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "),
                                src_keys.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")
                            ),
                        ));
                    }
                }
                Kind::Unknown => {}
            }
        }
        // A plural in the source must survive in languages that distinguish more than `other`.
        for s in src.iter().filter(|s| s.kind == Kind::Plural) {
            if required.len() > 1 && !tgt.iter().any(|t| t.name == s.name) {
                issues.push(error(
                    IssueCode::PluralCategoryMissing,
                    format!(
                        "The source pluralizes on `{}` but the translation has no plural for it; {} needs the categories {}.",
                        self.ph(&s.name),
                        self.locale,
                        required.join(", ")
                    ),
                ));
            }
        }
        self.out.extend(issues);
    }
}

fn open_tag(syntax: Syntax, name: &str, void: bool) -> String {
    match (syntax, void) {
        (Syntax::Mf2, false) => format!("{{#{name}}}"),
        (Syntax::Mf2, true) => format!("{{#{name}/}}"),
        (_, false) => format!("<{name}>"),
        (_, true) => format!("<{name}/>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use IssueCode::*;

    fn codes(source: &str, target: &str, locale: &str, syntax: Syntax) -> Vec<IssueCode> {
        validate(source, target, locale, syntax, &Constraints::default())
            .into_iter()
            .map(|i| i.code)
            .collect()
    }
    fn icu(source: &str, target: &str, locale: &str) -> Vec<IssueCode> {
        codes(source, target, locale, Syntax::Icu)
    }

    const ITEMS: &str = "{count, plural, =0 {No items} one {# item} other {# items}}";

    #[test]
    fn simple_ok_and_basic_issues() {
        assert!(icu("Hello {name}!", "Bonjour {name} !", "fr").is_empty());
        assert_eq!(icu("Hello", "  ", "fr"), [Empty]);
        assert_eq!(
            icu("Hello {name}", "Bonjour {nom}", "fr"),
            [PlaceholderMissing, PlaceholderExtra]
        );
        assert_eq!(icu("Hello {name}", "Bonjour {name", "fr"), [SyntaxError]);
        assert_eq!(icu("Hello", "Bonjour }", "fr"), [SyntaxError]);
        assert_eq!(icu("Settings", "Settings", "fr"), [Untranslated]);
        assert!(icu("{count}", "{count}", "fr").is_empty());
        assert_eq!(icu("Name: ", "Nom :", "fr"), [WhitespaceMismatch]);
        let i = validate(
            "Hello",
            "Bonjour",
            "fr",
            Syntax::Icu,
            &Constraints::default(),
        );
        assert!(i.is_empty());
    }

    #[test]
    fn plurals_by_locale() {
        let ar = "{count, plural, =0 {لا عناصر} zero {لا عناصر} one {عنصر واحد} two {عنصران} few {# عناصر} many {# عنصرًا} other {# عنصر}}";
        assert!(
            icu(ITEMS, ar, "ar").is_empty(),
            "{:?}",
            icu(ITEMS, ar, "ar")
        );
        let ar_short = "{count, plural, =0 {لا عناصر} one {عنصر واحد} other {# عنصر}}";
        let i = validate(
            ITEMS,
            ar_short,
            "ar-EG",
            Syntax::Icu,
            &Constraints::default(),
        );
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].code, PluralCategoryMissing);
        assert!(
            i[0].message.contains("zero, two, few, many"),
            "{}",
            i[0].message
        );

        let pl = "{count, plural, =0 {Brak elementów} one {# element} few {# elementy} many {# elementów} other {# elementu}}";
        assert!(icu(ITEMS, pl, "pl").is_empty());
        let pl_missing = "{count, plural, =0 {Brak} one {# element} other {# elementu}}";
        assert_eq!(icu(ITEMS, pl_missing, "pl"), [PluralCategoryMissing]);

        // Japanese: only `other`; `one` is extra (warning), and dropping the plural is fine.
        assert!(
            icu(
                ITEMS,
                "{count, plural, =0 {アイテムなし} other {#件のアイテム}}",
                "ja"
            )
            .is_empty()
        );
        assert_eq!(
            icu(
                ITEMS,
                "{count, plural, =0 {なし} one {#件} other {#件}}",
                "ja"
            ),
            [PluralCategoryExtra]
        );
        assert!(icu("{count} items", "{count}件", "ja").is_empty());
        assert_eq!(
            icu(ITEMS, "{count, plural, one {#件} other {#件}}", "ja"),
            [PluralCategoryExtra, PluralCategoryMissing]
        );

        // French needs `many` per CLDR; dropping the plural entirely is an error.
        let fr = "{count, plural, =0 {Aucun élément} one {# élément} many {# d'éléments} other {# éléments}}";
        assert!(
            icu(ITEMS, fr, "fr").is_empty(),
            "{:?}",
            icu(ITEMS, fr, "fr")
        );
        let fr_no_many = "{count, plural, =0 {Aucun élément} one {# élément} other {# éléments}}";
        let v = validate(
            ITEMS,
            fr_no_many,
            "fr",
            Syntax::Icu,
            &Constraints::default(),
        );
        assert!(
            !has_errors(&v) && v.iter().any(|i| i.code == PluralCategoryMissing),
            "{v:?}"
        );
        assert_eq!(
            icu(ITEMS, "{count} éléments", "fr"),
            [PluralCategoryMissing]
        );
        assert_eq!(
            icu(ITEMS, "{count, plural, =0 {a} one {b} many {c}}", "fr"),
            [PluralCategoryMissing]
        );
    }

    #[test]
    fn nested_plural_select_offset_and_pound() {
        let src = "{gender, select, female {{n, plural, offset:1 =0 {She is alone} one {She and # other} other {She and # others}}} other {{n, plural, =0 {They are alone} one {They and # other} other {They and # others}}}}";
        let tgt = "{gender, select, female {{n, plural, offset:1 =0 {Elle est seule} one {Elle et # autre} many {Elle et # autres} other {Elle et # autres}}} other {{n, plural, =0 {Seuls} one {Eux et # autre} many {Eux et # autres} other {Eux et # autres}}}}";
        assert!(
            icu(src, tgt, "fr").is_empty(),
            "{:?}",
            validate(src, tgt, "fr", Syntax::Icu, &Constraints::default())
        );
        assert_eq!(
            placeholders(src, Syntax::Icu)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["gender", "n"]
        );
        let bad_select = tgt.replace("female {", "femme {");
        assert_eq!(icu(src, &bad_select, "fr"), [KeywordTranslated]);
        let no_other = "{gender, select, female {Elle} male {Il}}";
        assert_eq!(
            icu(
                "{gender, select, female {She} male {He} other {They}}",
                no_other,
                "fr"
            ),
            [PluralCategoryMissing]
        );
    }

    #[test]
    fn keyword_translated() {
        let i = validate(
            ITEMS,
            "{count, pluriel, =0 {Aucun} un {# élément} autre {# éléments}}",
            "fr",
            Syntax::Icu,
            &Constraints::default(),
        );
        assert_eq!(
            i.iter().map(|i| i.code).collect::<Vec<_>>(),
            [KeywordTranslated]
        );
        assert!(
            i[0].message.contains("`pluriel`") && i[0].message.contains("{count, plural, …}"),
            "{}",
            i[0].message
        );

        let i = validate(
            ITEMS,
            "{count, plural, =0 {Aucun} un {# élément} many {#} autre {# éléments}}",
            "fr",
            Syntax::Icu,
            &Constraints::default(),
        );
        let c: Vec<_> = i.iter().map(|i| i.code).collect();
        assert!(
            c.contains(&KeywordTranslated) && c.contains(&PluralCategoryMissing),
            "{c:?}"
        );
        assert!(i.iter().any(|i| i.message.contains("`un`, `autre`")));

        assert_eq!(
            icu("{d, date, short}", "{d, date, court}", "fr"),
            [KeywordTranslated]
        );
        assert_eq!(
            icu("{p, number, percent}", "{p, nombre, percent}", "fr"),
            [KeywordTranslated]
        );
        assert!(icu("{p, number, ::percent}", "{p, number, ::percent}%", "fr").is_empty());
    }

    #[test]
    fn apostrophes() {
        // Lone apostrophes are literal; '' is one apostrophe; '{' quotes syntax.
        assert!(icu("Don't go, {name}", "N'y va pas, {name}", "fr").is_empty());
        assert!(icu("It''s {name}", "C''est {name}", "fr").is_empty());
        assert!(
            icu(
                "Use '{braces}' with {name}",
                "Utilise '{accolades}' avec {name}",
                "fr"
            )
            .is_empty()
        );
        assert_eq!(
            placeholders("'{literal}' {real}", Syntax::Icu)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            placeholders("'{literal} {still literal}", Syntax::Icu)
                .unwrap()
                .len(),
            0
        );
        // In a plural, '#' quotes the pound sign.
        let src = "{n, plural, one {'#'1 item} other {# items}}";
        assert!(placeholders(src, Syntax::Icu).is_ok());
    }

    #[test]
    fn rich_text_tags() {
        let src = "Read the <link>terms</link> and <b>accept</b>.";
        assert!(
            icu(
                src,
                "Lis les <link>conditions</link> et <b>accepte</b>.",
                "fr"
            )
            .is_empty()
        );
        assert!(icu(src, "<b>Accepte</b> les <link>conditions</link>.", "fr").is_empty());
        assert_eq!(
            icu(
                src,
                "Lis les <lien>conditions</lien> et <b>accepte</b>.",
                "fr"
            ),
            [TagMismatch]
        );
        assert_eq!(
            icu(src, "Lis les <link>conditions et <b>accepte</b>.", "fr"),
            [SyntaxError]
        );
        assert_eq!(
            icu(src, "Lis <link>les <b>conditions</link></b>.", "fr"),
            [SyntaxError]
        );
        assert!(icu("a < b and c > d", "a < b et c > d", "fr").is_empty());
        let i = validate(
            "<b>Hi</b> {name}",
            "<b>Salut</b> {name}",
            "fr",
            Syntax::Icu,
            &Constraints {
                max_length: Some(6),
                max_length_ratio: None,
            },
        );
        assert!(
            i.is_empty(),
            "visible length ignores tags and placeholders: {i:?}"
        );
    }

    #[test]
    fn length() {
        let c = Constraints {
            max_length: Some(10),
            max_length_ratio: Some(1.5),
        };
        let i = validate("Save now", "Enregistrer maintenant", "fr", Syntax::Icu, &c);
        assert_eq!(
            i.iter().map(|i| (i.code, i.severity)).collect::<Vec<_>>(),
            [(TooLong, Severity::Error), (TooLong, Severity::Warning)]
        );
        let i = validate(
            ITEMS,
            "{count, plural, =0 {Aucun} one {# élément} many {#} other {# éléments}}",
            "fr",
            Syntax::Icu,
            &c,
        );
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn i18next() {
        let s = Syntax::I18next;
        assert!(
            codes(
                "Hello {{name}}, you have {{count, number}} <1>new</1> $t(common.items)",
                "Salut {{name}}, tu as {{count, number}} <1>nouveaux</1> $t(common.items)",
                "fr",
                s
            )
            .is_empty()
        );
        assert_eq!(
            codes("Hello {{name}}", "Salut {{nom}}", "fr", s),
            [PlaceholderMissing, PlaceholderExtra]
        );
        assert_eq!(
            codes("Hello {{name}}", "Salut {{name}", "fr", s),
            [SyntaxError]
        );
        assert_eq!(
            codes("See $t(a.b)", "Voir $t(a.c)", "fr", s),
            [PlaceholderMissing]
        );
        assert_eq!(
            codes("{{x, number}}", "{{x, nombre}}", "fr", s),
            [KeywordTranslated]
        );
        assert_eq!(
            codes("<0>Hi</0> <br/>", "<0>Salut</0>", "fr", s),
            [TagMismatch]
        );
        assert!(codes("{{- html}} {single}", "{{-html}} {single}", "fr", s).is_empty());
        assert_eq!(
            placeholders("{{a}} {{ b, uppercase }}", s)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn mf2() {
        let s = Syntax::Mf2;
        assert!(codes("Hello {$name}!", "Bonjour {$name} !", "fr", s).is_empty());
        assert_eq!(
            codes("Hello {$name}!", "Bonjour {$nom} !", "fr", s),
            [PlaceholderMissing, PlaceholderExtra]
        );
        assert_eq!(
            codes("Hello {$name}", "Bonjour {$name", "fr", s),
            [SyntaxError]
        );
        assert_eq!(codes("Hello", "Bonjour }", "fr", s), [SyntaxError]);
        assert!(codes("Total {$n :number}", "Total \\{ {$n :number} \\}", "fr", s).is_empty());
        assert_eq!(
            codes("Total {$n :number}", "Total {$n :nombre}", "fr", s),
            [KeywordTranslated]
        );
        assert!(codes("{#b}Bold{/b} {#img/}", "{#b}Gras{/b} {#img/}", "fr", s).is_empty());
        assert_eq!(
            codes("{#b}Bold{/b}", "{#strong}Gras{/strong}", "fr", s),
            [TagMismatch]
        );

        let src = ".input {$count :number}\n.match $count\n0 {{No items}}\none {{{$count} item}}\n* {{{$count} items}}";
        let fr = ".input {$count :number}\n.match $count\n0 {{Aucun}}\none {{{$count} élément}}\nmany {{{$count} d'éléments}}\n* {{{$count} éléments}}";
        assert!(
            codes(src, fr, "fr", s).is_empty(),
            "{:?}",
            validate(src, fr, "fr", s, &Constraints::default())
        );
        let fr_missing = ".input {$count :number}\n.match $count\n0 {{Aucun}}\none {{{$count} élément}}\n* {{{$count} éléments}}";
        assert_eq!(codes(src, fr_missing, "fr", s), [PluralCategoryMissing]);
        let fr_no_star = ".input {$count :number}\n.match $count\n0 {{Aucun}}\none {{a}}\nmany {{b}}\nother {{c}}";
        assert!(codes(src, fr_no_star, "fr", s).is_empty());
        let fr_bad =
            ".input {$count :number}\n.match $count\n0 {{Aucun}}\nun {{a}}\nmany {{b}}\n* {{c}}";
        assert_eq!(
            codes(src, fr_bad, "fr", s),
            [KeywordTranslated, PluralCategoryMissing]
        );
        // Locals are not placeholders.
        let local = ".local $n = {$count :integer}\n{{You have {$n} messages}}";
        assert_eq!(
            placeholders(local, s)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["count"]
        );
        assert_eq!(
            codes(
                ".input {$x :string}\n.match $x\nmale {{He}}\n* {{They}}",
                ".input {$x :string}\n.match $x\nmale {{Il}}",
                "fr",
                s
            ),
            [PluralCategoryMissing]
        );
        assert_eq!(codes("a", ".bogus {{x}}", "fr", s), [SyntaxError]);
    }

    #[test]
    fn messages_are_actionable() {
        let i = validate(
            "Hi {name}",
            "Salut",
            "fr",
            Syntax::Icu,
            &Constraints::default(),
        );
        assert!(i[0].message.contains("`{name}`"), "{}", i[0].message);
        let i = validate("Hi", "Salut {x", "fr", Syntax::Icu, &Constraints::default());
        assert!(
            i[0].message.starts_with("Invalid ICU MessageFormat syntax")
                && i[0].message.contains("at character"),
            "{}",
            i[0].message
        );
        assert!(has_errors(&i));
        // Garbage never panics.
        for t in [
            "{",
            "}",
            "{a,",
            "{a, plural",
            "{a, plural, one",
            "<",
            "</b>",
            "'",
            "{{",
            "$t(",
            ".match",
            "{#",
            "\\",
            "{$",
        ] {
            for syntax in [Syntax::Icu, Syntax::I18next, Syntax::Mf2] {
                let _ = validate("x", t, "fr", syntax, &Constraints::default());
            }
        }
    }
}
