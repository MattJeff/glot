//! Leptos `view!` bodies read as token trees (`proc_macro2` + `syn`), the way rstml reads them:
//! every string literal is placed exactly (text node, attribute of an HTML element, component prop,
//! value of an expression) so the rewrite never has to guess from the surrounding characters.
//!
//! What is rewritten without changing a type:
//! - a text node (`<p>"Hello"</p>`, also next to a `{block}`) => `{move || t("key")}`;
//! - a visible attribute of an HTML element => `attr=move || t("key")`;
//! - the value of a child block or of a visible HTML attribute whose every outcome is a string
//!   literal or already a `String` (`{if ok { "Sent" } else { "Failed" }}`, `match` arms,
//!   `"x".to_string()`): each literal => `t("key")`, so every branch stays one type (`String`);
//! - inline translations: `if en { "Published" } else { "Publié" }` and the app's own pair helpers
//!   (`fn tr(fr: &'static str, en: &'static str)`) => one key, the other languages kept as
//!   translations.
//!
//! Everything else (a literal bound to a variable, passed to a function, a component prop) is
//! reported with the reason and the code to write: its type is used by code glot cannot see.
// ponytail: no type inference. A value is "a string" only when its syntax proves it; the day a
// project needs more, read component prop types from their `#[component] fn` signatures.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, Stmt};

use crate::rust::{Spot, Wrap};
use crate::{Hardcoded, scan};
use glot_core::context::SourceRef;

/// Helpers returning the current-locale string of the table: already `String`.
const STRING_FNS: &[&str] = &["t", "tn", "tc", "tl", "tnm", "tcl", "format"];

#[derive(Default)]
pub(crate) struct Found {
    pub spots: Vec<Spot>,
    /// Extra edits applied with the clean spots: `"".to_string()` on the literals that are not
    /// copy but share a branch tree with one that became `t("key")`.
    pub aux: Vec<(usize, usize, String)>,
}

/// What a pair helper returns: decides where its calls can become `t("key")`.
#[derive(Clone, Copy, PartialEq)]
enum Ret {
    /// `String`: same type as `t()`, replaceable anywhere.
    String,
    /// `&'static str`: only where a `String` is accepted too (rendered values).
    Str,
    /// A closure or anything else: only as the whole value of a block or attribute.
    Other,
}

/// `fn tr(fr: &'static str, en: &'static str)`: locale of each argument.
#[derive(Clone)]
struct Pair {
    locales: [String; 2],
    ret: Ret,
}

struct Cx<'a> {
    path: &'a str,
    src: &'a str,
    source: String,
    targets: Vec<String>,
    pairs: BTreeMap<String, Pair>,
    /// Starts of the expressions already turned into spots.
    seen: BTreeSet<usize>,
    /// Blocks and attribute values syn could not read (compared before/after a rewrite).
    errors: usize,
    /// Depth inside `view!` bodies: outside, a `format!` builds anything (a URL, a log line).
    in_view: usize,
    out: Found,
}

/// `None` when the file does not tokenize (the caller falls back to the literal scanner).
pub(crate) fn analyze(path: &str, src: &str) -> Option<Found> {
    let tokens: TokenStream = src.parse().ok()?;
    let mut cx = run(path, src, tokens);
    cx.out.spots.sort_by_key(|s| s.start);
    cx.out.spots.dedup_by_key(|s| s.start);
    Some(cx.out)
}

fn run<'a>(path: &'a str, src: &'a str, tokens: TokenStream) -> Cx<'a> {
    let (source, targets) = crate::i18nize::locales();
    let mut cx = Cx {
        path,
        src,
        pairs: pair_fns(src, &source, &targets),
        source,
        targets,
        seen: BTreeSet::new(),
        errors: 0,
        in_view: 0,
        out: Found::default(),
    };
    // The whole file through syn: `view!` bodies, and the pair helpers called anywhere else.
    match syn::parse_file(src) {
        Ok(file) => Other {
            cx: &mut cx,
            calls: 1,
        }
        .visit_file(&file),
        Err(_) => cx.walk(tokens),
    }
    cx
}

/// `en`, `fr`, `pt` (a parameter or variable named like a language).
fn is_locale(s: &str, known: &[&str]) -> bool {
    let l = s.to_ascii_lowercase();
    known
        .iter()
        .any(|k| k.eq_ignore_ascii_case(&l) || k.split(['-', '_']).next() == Some(l.as_str()))
}

/// `pub fn tr(fr, en)` helpers by (module, name), for the files importing them
/// (`use super::alternatives::tr;`). Filled by [`register`] before the files are analyzed.
static SHARED: std::sync::RwLock<BTreeMap<(String, String), Pair>> =
    std::sync::RwLock::new(BTreeMap::new());

/// Remember the pair helpers `path` defines, so that other files importing them are understood.
pub(crate) fn register(path: &str, src: &str) {
    let (source, targets) = crate::i18nize::locales();
    if !src.contains("fn ") || source.is_empty() {
        return;
    }
    let stem = path
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".rs"))
        .unwrap_or_default();
    let stem = if stem == "mod" {
        path.rsplit('/').nth(1).unwrap_or_default()
    } else {
        stem
    };
    let mut shared = SHARED.write().unwrap();
    for (name, pair) in local_pair_fns(src, &source, &targets) {
        shared.insert((stem.to_string(), name), pair);
    }
}

/// Helpers taking one string per language, named after the languages: defined in this file, or
/// imported from a file that defines one.
fn pair_fns(src: &str, source: &str, targets: &[String]) -> BTreeMap<String, Pair> {
    let mut out = local_pair_fns(src, source, targets);
    for ((module, name), pair) in SHARED.read().unwrap().iter() {
        let imported = src.lines().any(|l| {
            let l = l.trim_start();
            (l.starts_with("use ") || l.starts_with("pub use "))
                && l.contains(&format!("{module}::"))
                && l.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|w| w == name)
        });
        if imported && !out.contains_key(name) {
            out.insert(name.clone(), pair.clone());
        }
    }
    out
}

fn local_pair_fns(src: &str, source: &str, targets: &[String]) -> BTreeMap<String, Pair> {
    let mut out = BTreeMap::new();
    if source.is_empty() {
        return out;
    }
    let known: Vec<&str> = std::iter::once(source)
        .chain(targets.iter().map(String::as_str))
        .collect();
    let Ok(file) = syn::parse_file(src) else {
        return out;
    };
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        let names: Vec<String> = f
            .sig
            .inputs
            .iter()
            .filter_map(|a| match a {
                syn::FnArg::Typed(p) => match &*p.pat {
                    syn::Pat::Ident(i) if matches!(&*p.ty, syn::Type::Reference(_)) => {
                        Some(i.ident.to_string())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        if names.len() != 2
            || f.sig.inputs.len() != 2
            || names[0] == names[1]
            || !names.iter().all(|n| is_locale(n, &known))
            || !names.iter().any(|n| n == source)
        {
            continue;
        }
        let ret = match &f.sig.output {
            syn::ReturnType::Type(_, t) => match &**t {
                syn::Type::Path(p) if p.path.is_ident("String") => Ret::String,
                syn::Type::Reference(r) if matches!(&*r.elem, syn::Type::Path(p) if p.path.is_ident("str")) => {
                    Ret::Str
                }
                _ => Ret::Other,
            },
            syn::ReturnType::Default => Ret::Other,
        };
        out.insert(
            f.sig.ident.to_string(),
            Pair {
                locales: [names[0].clone(), names[1].clone()],
                ret,
            },
        );
    }
    out
}

fn punct(t: Option<&TokenTree>, c: char) -> bool {
    matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == c)
}

fn str_lit(t: &TokenTree) -> Option<(String, Range<usize>)> {
    let TokenTree::Literal(l) = t else {
        return None;
    };
    match syn::Lit::new(l.clone()) {
        syn::Lit::Str(s) => Some((s.value(), l.span().byte_range())),
        _ => None,
    }
}

/// Where the value of an expression goes.
#[derive(Clone, Copy)]
enum Ctx {
    /// Rendered (child block, visible attribute of an HTML element): `&str` or `String` alike.
    Rendered(&'static str),
    /// A visible prop of a component: its type is declared by the component.
    Typed(&'static str),
    /// Not shown to the user (`class=`, `href=`, `on:click=`): never copy.
    Code,
}

/// One outcome of an expression.
enum Leaf {
    /// A string literal (`owned`: `"x".to_string()`, already a `String`).
    Lit {
        span: Range<usize>,
        text: String,
        owned: bool,
    },
    /// Inline translations of one message (locale => text).
    Pair {
        span: Range<usize>,
        texts: BTreeMap<String, String>,
        ret: Ret,
    },
    /// Already a `String`.
    Str,
    Unknown,
}

impl<'a> Cx<'a> {
    fn line(&self, at: usize) -> u32 {
        self.src[..at.min(self.src.len())].matches('\n').count() as u32 + 1
    }

    /// Every `view!` of a token stream, at any depth.
    fn walk(&mut self, ts: TokenStream) {
        let tt: Vec<TokenTree> = ts.into_iter().collect();
        let mut i = 0;
        while i < tt.len() {
            match &tt[i] {
                TokenTree::Ident(id)
                    if id == "view"
                        && punct(tt.get(i + 1), '!')
                        && matches!(tt.get(i + 2), Some(TokenTree::Group(_))) =>
                {
                    if let Some(TokenTree::Group(g)) = tt.get(i + 2) {
                        self.nodes(g.stream());
                    }
                    i += 3;
                }
                TokenTree::Group(g) => {
                    self.walk(g.stream());
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }

    /// The nodes of a `view!` body: tags, text literals and `{blocks}` in one flat run.
    fn nodes(&mut self, ts: TokenStream) {
        self.in_view += 1;
        self.view_nodes(ts);
        self.in_view -= 1;
    }

    fn view_nodes(&mut self, ts: TokenStream) {
        let tt: Vec<TokenTree> = ts.into_iter().collect();
        let mut i = 0;
        while i < tt.len() {
            match &tt[i] {
                TokenTree::Punct(p) if p.as_char() == '<' => {
                    let closing = punct(tt.get(i + 1), '/') || punct(tt.get(i + 1), '!');
                    // Name: `div`, `Show`, `leptos_router::A`, `my-element`.
                    let mut j = i + 1;
                    if !closing && matches!(tt.get(j), Some(TokenTree::Ident(_))) {
                        j += 1;
                        while matches!(tt.get(j), Some(TokenTree::Punct(p)) if matches!(p.as_char(), '-' | ':'))
                        {
                            j += 1;
                            if matches!(tt.get(j), Some(TokenTree::Ident(_))) {
                                j += 1;
                            }
                        }
                    }
                    // rstml: the open tag ends at the first `>` outside a group.
                    let Some(end) = (j..tt.len()).find(|&k| punct(tt.get(k), '>')) else {
                        return;
                    };
                    if !closing && j > i + 1 {
                        let name: String = tt[i + 1..j].iter().map(|t| t.to_string()).collect();
                        let html = name.starts_with(|c: char| c.is_ascii_lowercase())
                            && !name.contains("::");
                        let stop = if punct(tt.get(end.wrapping_sub(1)), '/') {
                            end - 1
                        } else {
                            end
                        };
                        self.attrs(&tt[j.min(stop)..stop], html);
                    }
                    i = end + 1;
                }
                TokenTree::Literal(_) => {
                    if let Some((text, span)) = str_lit(&tt[i]) {
                        let brace = |t: Option<&TokenTree>| matches!(t, Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace);
                        let glued = (i > 0 && brace(tt.get(i - 1))) || brace(tt.get(i + 1));
                        self.text_node(text, span, glued);
                    }
                    i += 1;
                }
                TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                    self.block(g.stream(), Ctx::Rendered("text"));
                    i += 1;
                }
                TokenTree::Group(g) => {
                    self.walk(g.stream());
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }

    fn text_node(&mut self, text: String, span: Range<usize>, glued: bool) {
        let t = text.trim();
        if !scan::looks_like_text(t) {
            return;
        }
        // `{secs}" s. "`: a unit glued to a value is not a message on its own.
        let letters = t.chars().filter(|c| c.is_alphabetic()).count();
        let unit = glued && t.len() != text.len() && letters < 3;
        let fragment = glued && (text.starts_with(' ') || text.ends_with(' '));
        self.push(
            span,
            text,
            "text",
            Wrap::Text,
            (!unit).then_some(()).ok_or(
                "a unit next to a value: write the whole sentence as one message with a placeholder",
            ),
            BTreeMap::new(),
            fragment.then_some("part of a sentence built around a value: keep the word order free in translation by merging it into one message with a placeholder"),
        );
    }

    /// The attributes of an open tag (`html`: a lowercase HTML element, not a component).
    fn attrs(&mut self, tt: &[TokenTree], html: bool) {
        let mut k = 0;
        while k < tt.len() {
            let TokenTree::Ident(_) = &tt[k] else {
                if let TokenTree::Group(g) = &tt[k] {
                    self.walk(g.stream());
                }
                k += 1;
                continue;
            };
            let start = k;
            k += 1;
            while matches!(tt.get(k), Some(TokenTree::Punct(p)) if matches!(p.as_char(), '-' | ':'))
                && matches!(tt.get(k + 1), Some(TokenTree::Ident(_)))
            {
                k += 2;
            }
            let name: String = tt[start..k].iter().map(|t| t.to_string()).collect();
            if !punct(tt.get(k), '=') {
                continue;
            }
            k += 1;
            let role = scan::attr_role(&name.replace('_', "-"));
            let ctx = match (role, html) {
                (Some(r), true) => Ctx::Rendered(r),
                (Some(r), false) => Ctx::Typed(r),
                (None, _) => Ctx::Code,
            };
            let alone = |k: usize| !matches!(tt.get(k + 1), Some(TokenTree::Punct(p)) if matches!(p.as_char(), '.' | '?'));
            match tt.get(k) {
                Some(t @ TokenTree::Literal(_)) if alone(k) => {
                    if let (Some((text, span)), Some(role)) = (str_lit(t), role) {
                        let word = text.bytes().all(|c| c.is_ascii_lowercase() || c == b'_');
                        if scan::looks_like_text(&text) && (html || !word) {
                            self.push(
                                span,
                                text,
                                role,
                                Wrap::Attr,
                                html.then_some(()).ok_or(
                                    "a component prop: its type is declared by the component",
                                ),
                                BTreeMap::new(),
                                None,
                            );
                        }
                    }
                    k += 1;
                }
                Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace && alone(k) => {
                    self.block(g.stream(), ctx);
                    k += 1;
                }
                Some(_) => {
                    // A bare expression: syn says where it ends.
                    let rest: TokenStream = tt[k..].iter().cloned().collect();
                    let parsed = (|input: syn::parse::ParseStream| {
                        let e: Expr = input.parse()?;
                        let left: TokenStream = input.parse()?;
                        Ok((e, left.into_iter().count()))
                    })
                    .parse2(rest);
                    match parsed {
                        Ok((e, left)) => {
                            self.expr(&e, ctx);
                            k = tt.len() - left;
                        }
                        Err(_) => {
                            self.errors += 1;
                            self.walk(tt[k..].iter().cloned().collect());
                            return;
                        }
                    }
                }
                None => {}
            }
        }
    }

    /// A `{ … }` child or attribute value: statements, the last one being the value.
    fn block(&mut self, ts: TokenStream, ctx: Ctx) {
        let Ok(stmts) = syn::Block::parse_within.parse2(ts.clone()) else {
            // `{..}` / `{..props}` spreads are not statements.
            if !punct(ts.clone().into_iter().next().as_ref(), '.') {
                self.errors += 1;
            }
            return self.walk(ts);
        };
        let n = stmts.len();
        for (i, s) in stmts.iter().enumerate() {
            match s {
                Stmt::Expr(e, None) if i + 1 == n => self.expr(e, ctx),
                s => Other { cx: self, calls: 0 }.visit_stmt(s),
            }
        }
    }

    /// An expression whose value goes to `ctx`.
    fn expr(&mut self, e: &Expr, ctx: Ctx) {
        self.value(e, ctx, true)
    }

    /// `whole`: `e` is the entire value of a block or attribute (it may become a closure);
    /// `false` for an argument of `format!`, which only needs `Display`.
    fn value(&mut self, e: &Expr, ctx: Ctx, whole: bool) {
        let mut leaves = vec![];
        self.leaves(e, !whole, whole, &mut leaves);
        for (l, _) in &leaves {
            if let Leaf::Lit { span, .. } | Leaf::Pair { span, .. } = l {
                self.seen.insert(span.start);
            }
        }
        if matches!(ctx, Ctx::Code) {
            return Other { cx: self, calls: 0 }.visit_expr(e);
        }
        // `copy("fr", "en")` picks a language code (a file name, a URL): not a message.
        leaves.retain(|l| match &l.0 {
            Leaf::Pair { texts, .. } => !texts.iter().all(|(loc, text)| loc == text),
            _ => true,
        });
        let copy = |l: &(Leaf, bool)| match &l.0 {
            Leaf::Lit { text, .. } => is_copy(text),
            Leaf::Pair { .. } => true,
            _ => false,
        };
        if leaves.iter().any(copy) {
            let unknown = leaves.iter().any(|l| matches!(l.0, Leaf::Unknown));
            let (role, why) = match ctx {
                Ctx::Rendered(r) if unknown => (
                    r,
                    Some(
                        "another branch of this expression is not a plain string: all branches must keep one type",
                    ),
                ),
                Ctx::Rendered(r) => (r, None),
                Ctx::Typed(r) => (
                    r,
                    Some("a component prop: its type is declared by the component"),
                ),
                Ctx::Code => unreachable!(),
            };
            // `&str` leaves become `String`: the non-copy literals of the tree follow.
            let retyped = leaves.iter().any(|l| match &l.0 {
                Leaf::Lit { owned, text, .. } => !owned && is_copy(text),
                Leaf::Pair { ret, .. } => *ret != Ret::String,
                _ => false,
            });
            for (leaf, direct) in leaves {
                match leaf {
                    Leaf::Lit { span, text, owned } => {
                        self.seen.insert(span.start);
                        if is_copy(&text) {
                            self.push(
                                span,
                                text,
                                role,
                                Wrap::Call { closure: direct },
                                why.map_or(Ok(()), Err),
                                BTreeMap::new(),
                                None,
                            );
                        } else if why.is_none() && retyped && !owned {
                            let lit = &self.src[span.clone()];
                            self.out
                                .aux
                                .push((span.start, span.end, format!("{lit}.to_string()")));
                        }
                    }
                    Leaf::Pair {
                        span,
                        mut texts,
                        ret,
                    } => {
                        self.seen.insert(span.start);
                        let Some(text) = texts.remove(&self.source) else {
                            continue;
                        };
                        // A helper returning a closure is only replaceable as the whole value.
                        let why = why.or((ret == Ret::Other && !direct).then_some(
                            "this helper does not return a string here: replace the call by hand",
                        ));
                        self.push(
                            span,
                            text,
                            role,
                            Wrap::Call { closure: direct },
                            why.map_or(Ok(()), Err),
                            texts,
                            None,
                        );
                    }
                    Leaf::Str | Leaf::Unknown => {}
                }
            }
        }
        Other { cx: self, calls: 0 }.visit_expr(e);
    }

    /// `if en { "A" } else { "B" }` on a condition naming one project language.
    fn locale_if(&self, i: &syn::ExprIf) -> Option<BTreeMap<String, String>> {
        if self.source.is_empty() {
            return None;
        }
        let known: Vec<&str> = std::iter::once(self.source.as_str())
            .chain(self.targets.iter().map(String::as_str))
            .collect();
        let mut named = BTreeSet::new();
        let mut plain = true;
        fn scan_cond(
            ts: TokenStream,
            known: &[&str],
            named: &mut BTreeSet<String>,
            plain: &mut bool,
        ) {
            for t in ts {
                match t {
                    TokenTree::Ident(id) => {
                        let s = id.to_string();
                        if let Some(k) = known.iter().find(|k| k.eq_ignore_ascii_case(&s)) {
                            named.insert(k.to_string());
                        }
                    }
                    TokenTree::Literal(l) => {
                        if let syn::Lit::Str(s) = syn::Lit::new(l) {
                            match known.iter().find(|k| **k == s.value()) {
                                Some(k) => {
                                    named.insert(k.to_string());
                                }
                                None => *plain = false,
                            }
                        }
                    }
                    // `!en`, `a != b`, `a && b`, `a || b`: not a plain "is this language" test.
                    TokenTree::Punct(p) if matches!(p.as_char(), '!' | '&' | '|' | '<' | '>') => {
                        *plain = false
                    }
                    TokenTree::Punct(_) => {}
                    TokenTree::Group(g) => scan_cond(g.stream(), known, named, plain),
                }
            }
        }
        use quote::ToTokens as _;
        scan_cond(i.cond.to_token_stream(), &known, &mut named, &mut plain);
        if !plain || named.len() != 1 || !self.reads_locale(&i.cond) {
            return None;
        }
        let yes = named.into_iter().next()?;
        // The `else` is every other language: the source, or the only target.
        let no = if yes != self.source {
            self.source.clone()
        } else if let [only] = self.targets.as_slice() {
            only.clone()
        } else {
            return None;
        };
        let lit = |b: &syn::Block| match b.stmts.as_slice() {
            [Stmt::Expr(Expr::Lit(l), None)] => match &l.lit {
                syn::Lit::Str(s) => Some(s.value()),
                _ => None,
            },
            _ => None,
        };
        let Some((_, alt)) = &i.else_branch else {
            return None;
        };
        let Expr::Block(alt) = &**alt else {
            return None;
        };
        let (a, b) = (lit(&i.then_branch)?, lit(&alt.block)?);
        (is_copy(&a) || is_copy(&b)).then(|| [(yes, a), (no, b)].into())
    }

    /// The condition tests the interface language, not a flag about something else (the language
    /// of an article): it names a `locale` / `lang`, or is a local bound from one
    /// (`let en = is_en();`, `let en = auth.locale.get() == Locale::En;`).
    fn reads_locale(&self, cond: &Expr) -> bool {
        let langy = |s: &str| {
            let l = s.to_ascii_lowercase();
            l.contains("locale") || l.contains("lang") || l.contains("is_en") || l.contains("is_fr")
        };
        use quote::ToTokens as _;
        if langy(&cond.to_token_stream().to_string()) {
            return true;
        }
        let Expr::Path(p) = cond else { return false };
        let Some(id) = p.path.get_ident() else {
            return false;
        };
        let pat = format!("let {id} = ");
        let mut inits = self
            .src
            .match_indices(&pat)
            .map(|(k, _)| &self.src[k..self.src[k..].find(';').map_or(self.src.len(), |e| k + e)])
            .peekable();
        inits.peek().is_some() && inits.all(langy)
    }

    /// The outcomes of `e`. `direct`: `e` is the whole value (no branch, no closure around it).
    fn leaves(&self, e: &Expr, in_closure: bool, direct: bool, out: &mut Vec<(Leaf, bool)>) {
        let span = |x: &dyn Spanned| x.span().byte_range();
        let str_of = |x: &Expr| match x {
            Expr::Lit(l) => match &l.lit {
                syn::Lit::Str(s) => Some(s.value()),
                _ => None,
            },
            _ => None,
        };
        let last = |p: &syn::Path| p.segments.last().map(|s| s.ident.to_string());
        let pair_call = |c: &syn::ExprCall| -> Option<Leaf> {
            let Expr::Path(p) = &*c.func else { return None };
            let pair = self.pairs.get(&last(&p.path)?)?;
            let (a, b) = (str_of(c.args.first()?)?, str_of(c.args.get(1)?)?);
            (c.args.len() == 2).then(|| Leaf::Pair {
                span: span(c),
                texts: [(pair.locales[0].clone(), a), (pair.locales[1].clone(), b)].into(),
                ret: pair.ret,
            })
        };
        let tail = |b: &syn::Block| match b.stmts.last() {
            Some(Stmt::Expr(e, None)) => Some(e.clone()),
            _ => None,
        };
        let leaf = match e {
            Expr::Lit(_) => match str_of(e) {
                Some(text) => Leaf::Lit {
                    span: span(e),
                    text,
                    owned: false,
                },
                None => Leaf::Unknown,
            },
            Expr::Paren(p) => return self.leaves(&p.expr, in_closure, direct, out),
            Expr::Group(g) => return self.leaves(&g.expr, in_closure, direct, out),
            Expr::MethodCall(m)
                if m.args.is_empty() && (m.method == "to_string" || m.method == "to_owned") =>
            {
                match &*m.receiver {
                    r @ Expr::Lit(_) => match str_of(r) {
                        Some(text) => Leaf::Lit {
                            span: span(m),
                            text,
                            owned: true,
                        },
                        None => Leaf::Str,
                    },
                    Expr::Call(c) => match pair_call(c) {
                        Some(Leaf::Pair { texts, .. }) => Leaf::Pair {
                            span: span(m),
                            texts,
                            ret: Ret::String,
                        },
                        _ => Leaf::Str,
                    },
                    _ => Leaf::Str,
                }
            }
            // `tr("a", "b")()`: a helper returning a closure, called on the spot => a `&str`.
            Expr::Call(c)
                if c.args.is_empty()
                    && matches!(&*c.func, Expr::Call(inner) if pair_call(inner).is_some()) =>
            {
                match &*c.func {
                    Expr::Call(inner) => match pair_call(inner) {
                        Some(Leaf::Pair { texts, .. }) => Leaf::Pair {
                            span: span(c),
                            texts,
                            ret: Ret::Str,
                        },
                        _ => Leaf::Unknown,
                    },
                    _ => Leaf::Unknown,
                }
            }
            Expr::Call(c) => match pair_call(c) {
                Some(l) => l,
                None => match &*c.func {
                    Expr::Path(p)
                        if last(&p.path).is_some_and(|n| STRING_FNS.contains(&n.as_str())) =>
                    {
                        Leaf::Str
                    }
                    _ => Leaf::Unknown,
                },
            },
            Expr::Macro(m) if m.mac.path.is_ident("format") => Leaf::Str,
            Expr::If(i) => {
                if let Some(texts) = self.locale_if(i) {
                    Leaf::Pair {
                        span: span(i),
                        texts,
                        ret: Ret::Str,
                    }
                } else {
                    match (tail(&i.then_branch), &i.else_branch) {
                        (Some(a), Some((_, b))) => {
                            self.leaves(&a, in_closure, false, out);
                            return self.leaves(b, in_closure, false, out);
                        }
                        _ => Leaf::Unknown,
                    }
                }
            }
            Expr::Match(m) if !m.arms.is_empty() => {
                for arm in &m.arms {
                    self.leaves(&arm.body, in_closure, false, out);
                }
                return;
            }
            Expr::Block(b) if b.label.is_none() && b.attrs.is_empty() => match tail(&b.block) {
                Some(t) => return self.leaves(&t, in_closure, direct, out),
                None => Leaf::Unknown,
            },
            Expr::Closure(c)
                if !in_closure
                    && c.inputs.is_empty()
                    && matches!(c.output, syn::ReturnType::Default) =>
            {
                return self.leaves(&c.body, true, false, out);
            }
            _ => Leaf::Unknown,
        };
        out.push((leaf, direct));
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        span: Range<usize>,
        raw: String,
        role: &'static str,
        wrap: Wrap,
        clean: Result<(), &'static str>,
        translations: BTreeMap<String, String>,
        note: Option<&'static str>,
    ) {
        let text = raw.trim().to_string();
        let pre = raw[..raw.len() - raw.trim_start().len()].to_string();
        let post = raw[raw.trim_end().len()..].to_string();
        self.out.spots.push(Spot {
            start: span.start,
            end: span.end,
            attr: matches!(wrap, Wrap::Attr),
            clean: clean.is_ok(),
            wrap,
            pre,
            post,
            translations: translations
                .into_iter()
                .map(|(l, v)| (l, v.trim().to_string()))
                .collect(),
            note: note.map(String::from),
            copy: Hardcoded {
                at: SourceRef {
                    path: self.path.to_string(),
                    line: self.line(span.start),
                },
                suggested_key: scan::suggest_key(self.path, &text, role),
                text,
                ui_role: Some(role.to_string()),
                component: None,
                reason: clean.err().map(String::from),
            },
        });
    }
}

/// Copy, not a value: `"submit"` (one lowercase word) is an identifier.
fn is_copy(s: &str) -> bool {
    let t = s.trim();
    scan::looks_like_text(t) && !t.bytes().all(|c| c.is_ascii_lowercase())
}

/// Expressions whose value glot cannot follow: nested `view!`, and the literals of `if` / `match`
/// branches and pair helpers used as plain values.
struct Other<'c, 'a> {
    cx: &'c mut Cx<'a>,
    /// Depth inside call arguments: a literal there is an argument, not a rendered value.
    calls: usize,
}

impl<'ast> Visit<'ast> for Other<'_, '_> {
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if m.path.is_ident("view") {
            self.cx.nodes(m.tokens.clone());
        } else if !["html", "rsx"].iter().any(|n| m.path.is_ident(n)) {
            // `format!("{} {}", copy("Choisir", "Choose"), n)`: arguments are expressions.
            let args = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
                .parse2(m.tokens.clone());
            match args {
                // `String` and `&str` format alike: an argument of `format!` can become `t(…)`.
                Ok(args) if m.path.is_ident("format") && self.cx.in_view > 0 => {
                    for a in args.iter().skip(1) {
                        let a = match a {
                            Expr::Assign(x) => &*x.right,
                            a => a,
                        };
                        self.cx.value(a, Ctx::Rendered("text"), false);
                    }
                }
                Ok(args) => {
                    self.calls += 1;
                    for a in &args {
                        self.visit_expr(a);
                    }
                    self.calls -= 1;
                }
                Err(_) => self.cx.walk(m.tokens.clone()),
            }
        }
    }

    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        self.calls += 1;
        syn::visit::visit_expr_call(self, c);
        self.calls -= 1;
    }

    fn visit_expr_method_call(&mut self, c: &'ast syn::ExprMethodCall) {
        self.visit_expr(&c.receiver);
        self.calls += 1;
        for a in &c.args {
            self.visit_expr(a);
        }
        self.calls -= 1;
    }

    fn visit_expr(&mut self, e: &'ast Expr) {
        let start = e.span().byte_range().start;
        let branchy = matches!(e, Expr::If(_) | Expr::Match(_)) && self.calls == 0;
        let pair = matches!(e, Expr::Call(_) | Expr::MethodCall(_));
        if (branchy || pair) && !self.cx.seen.contains(&start) {
            let mut leaves = vec![];
            self.cx.leaves(e, true, false, &mut leaves);
            for (leaf, _) in leaves {
                match leaf {
                    Leaf::Lit { span, text, .. }
                        if branchy && is_copy(&text) && self.cx.seen.insert(span.start) =>
                    {
                        self.cx.push(
                            span,
                            text,
                            "text",
                            Wrap::Call { closure: false },
                            Err("the value is used by other code (a variable, an argument): its type is declared there"),
                            BTreeMap::new(),
                            None,
                        );
                    }
                    Leaf::Pair {
                        span,
                        mut texts,
                        ret,
                    } if !texts.iter().all(|(loc, text)| loc == text)
                        && self.cx.seen.insert(span.start) =>
                    {
                        if let Some(text) = texts.remove(&self.cx.source) {
                            // Same type as `t()`: replaceable anywhere.
                            self.cx.push(
                                span,
                                text,
                                "text",
                                Wrap::Call { closure: false },
                                if ret == Ret::String {
                                    Ok(())
                                } else {
                                    Err("the value is used by other code (a variable, an argument) that expects a `&'static str`")
                                },
                                texts,
                                None,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        syn::visit::visit_expr(self, e);
    }
}

/// Parse check of a rewritten file: the whole file with syn, then every `view!` body the way the
/// analysis reads it. The rewrite must not add a block or attribute value that fails to parse.
pub fn verify(old: &str, new: &str) -> Result<(), String> {
    syn::parse_file(new).map_err(|e| format!("line {}: {e}", e.span().start().line))?;
    let count = |s: &str| s.parse::<TokenStream>().map(|t| run("", s, t).errors);
    match (count(old), count(new)) {
        (Ok(a), Ok(b)) if b <= a => Ok(()),
        (_, Err(e)) => Err(format!("does not tokenize: {e}")),
        _ => Err("a rewritten `view!` expression does not parse".into()),
    }
}
