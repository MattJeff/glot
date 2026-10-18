//! `glot i18nize` front half: find hardcoded UI copy as whole ICU messages (JSX runs with
//! `{expr}` and inline tags, concatenations, template literals, plurals) and rewrite the source
//! with span edits (the rest of the file is left byte-for-byte untouched).
use std::collections::{BTreeMap, BTreeSet, HashMap};

use glot_core::validate::Syntax;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::operator::BinaryOperator;
use oxc_syntax::scope::ScopeFlags;
use serde::{Deserialize, Serialize};

use std::sync::RwLock;

use crate::scan::{
    attr_role, callee_name, elem_name, looks_like_text, path_prefix, skips_text, snake,
    suggest_key, tag_role,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lib {
    NextIntl,
    ReactI18next,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    Arg {
        name: String,
    },
    /// `{count, plural, one {# invoice} other {# invoices}}` (arg is always `count`).
    Plural {
        arg: String,
        one: String,
        other: String,
    },
    /// Rich-text tag around plain text: `<b>bold</b>`.
    Tag {
        name: String,
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// JSX children run: replaced by `{t(..)}`.
    Children,
    /// `attr="..."`: replaced by `{t(..)}`.
    AttrString,
    /// Expression position: replaced by `t(..)`.
    Expr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: usize,
    pub parts: Vec<Part>,
    /// ICU rendering, used for display, naming and dedup.
    pub text: String,
    pub role: String,
    pub component: Option<String>,
    pub element: Option<String>,
    pub line: u32,
    /// Deterministic fallback key (`<namespace>.<slug>`).
    pub suggested_key: String,
    #[serde(skip)]
    span: (u32, u32),
    #[serde(skip)]
    kind: Option<Kind>,
    #[serde(skip)]
    comp: usize,
    /// Arg name -> expression source.
    #[serde(skip)]
    args: Vec<(String, String)>,
    /// Tag name -> (opening element source, closing element source).
    #[serde(skip)]
    tags: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    pub line: u32,
    pub text: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct Comp {
    span: (u32, u32),
    is_async: bool,
    hook: Hook,
    blocked: Option<String>,
    /// Translator variable: `t`, or `tr` when the component already binds `t`.
    tname: &'static str,
    /// `useX()` hook returning a literal (module config turned into a hook): every text value is copy.
    config: bool,
    /// Edits applied only when the component gets a translator (`metadata` -> `generateMetadata`).
    extra: Vec<(u32, u32, String)>,
}

#[derive(Debug, Clone)]
enum Hook {
    /// Insert statements at `pos` (after `{` / directives).
    Block { pos: u32, indent: String },
    /// Concise arrow body `start..end`: wrap in `{ const t = ..; return <expr>; }`.
    Expr {
        start: u32,
        end: u32,
        indent: String,
        outer: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct FileAnalysis {
    pub path: String,
    pub namespace: String,
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<Skipped>,
    /// Parse error of the original file (nothing can be rewritten).
    pub error: Option<String>,
    comps: Vec<Comp>,
    use_client: bool,
    imported: BTreeSet<String>,
    import_pos: Option<(u32, bool)>,
}

/// Component name -> props rendered as text by that component (`<StatCard delta="..." />`).
/// ponytail: process-wide because `analyze` is called per file by glot-llm; keyed by name only
/// (two components with the same name merge their props).
static TEXT_PROPS: RwLock<BTreeMap<String, BTreeSet<String>>> = RwLock::new(BTreeMap::new());

/// Register text props found by a project-wide pass, used by the next `analyze` calls.
pub fn set_text_props(props: BTreeMap<String, BTreeSet<String>>) {
    *TEXT_PROPS.write().unwrap() = props;
}

/// Files matched by `[extract]` include/exclude (relative to `root`, sorted).
pub fn source_files(
    root: &std::path::Path,
    cfg: &glot_core::config::ExtractConfig,
) -> Result<Vec<String>, crate::Error> {
    let include = crate::patterns(&cfg.include)?;
    let exclude = crate::patterns(&cfg.exclude)?;
    let opts = glob::MatchOptions {
        require_literal_separator: true,
        ..Default::default()
    };
    let mut files = Vec::new();
    crate::walk(root, root, &mut files)?;
    files.sort();
    let matches = |ps: &[glob::Pattern], f: &str| ps.iter().any(|p| p.matches_with(f, opts));
    files.retain(|f| matches(&include, f) && !matches(&exclude, f));
    Ok(files)
}

/// Namespace for a file: `app/invoices/page.tsx` -> `invoices`, root page -> `home`.
pub fn namespace_for(path: &str) -> String {
    let p = path_prefix(path);
    if p.is_empty() { "home".into() } else { p }
}

fn source_type(path: &str) -> Option<SourceType> {
    match SourceType::from_path(path) {
        Ok(st) if st.is_typescript() => Some(st),
        Ok(st) => Some(st.with_jsx(true)),
        Err(_) => None,
    }
}

/// Re-parse check used before writing anything.
pub fn parses(path: &str, source: &str) -> Result<(), String> {
    let st = source_type(path).ok_or_else(|| format!("unsupported file type: {path}"))?;
    let allocator = Allocator::default();
    let r = Parser::new(&allocator, source, st).parse();
    if r.diagnostics.has_errors() {
        return Err(format!("{} parse error(s)", r.diagnostics.len()));
    }
    Ok(())
}

pub fn analyze(path: &str, source: &str, lib: Lib) -> FileAnalysis {
    let mut fa = FileAnalysis {
        path: path.into(),
        namespace: namespace_for(path),
        ..Default::default()
    };
    let Some(st) = source_type(path) else {
        fa.error = Some("unsupported file type".into());
        return fa;
    };
    if [".test.", ".spec.", "__tests__/", ".stories."]
        .iter()
        .any(|p| path.contains(p))
    {
        return fa;
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, st).parse();
    if parsed.diagnostics.has_errors() {
        fa.error = Some("file does not parse; left untouched".into());
        return fa;
    }
    let program = &parsed.program;
    fa.use_client = program
        .directives
        .iter()
        .any(|d| d.directive == "use client");
    for stmt in &program.body {
        if let Statement::ImportDeclaration(d) = stmt {
            fa.import_pos = Some((d.span.end, true));
            for s in d.specifiers.iter().flatten() {
                if let ImportDeclarationSpecifier::ImportSpecifier(s) = s {
                    fa.imported.insert(s.local.name.to_string());
                }
            }
        }
    }
    if fa.import_pos.is_none() {
        fa.import_pos = program.directives.last().map(|d| (d.span.end, false));
    }
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    let mut a = Analyzer {
        src: source,
        path,
        lib,
        line_starts,
        fa: &mut fa,
        comp_targets: HashMap::new(),
        comp_stack: vec![],
        fn_depth: 0,
        roles: vec![],
        skip_text: 0,
        t_bindings: vec![],
        text_props: TEXT_PROPS.read().unwrap().clone(),
    };
    a.visit_program(program);
    let t_bindings = std::mem::take(&mut a.t_bindings);
    for c in &mut fa.comps {
        let bound = |name: &str| {
            t_bindings
                .iter()
                .any(|(n, p)| n == name && *p >= c.span.0 && *p < c.span.1)
        };
        if bound("t") {
            if bound("tr") {
                c.blocked = Some("`t` and `tr` are already defined in this component".into());
            } else {
                c.tname = "tr";
            }
        }
        if c.blocked.is_none() && c.is_async && lib == Lib::ReactI18next {
            c.blocked = Some("async component: react-i18next hooks need a client component".into());
        }
    }
    let (keep, blocked): (Vec<_>, Vec<_>) = std::mem::take(&mut fa.candidates)
        .into_iter()
        .partition(|c| fa.comps[c.comp].blocked.is_none());
    for c in blocked {
        let reason = fa.comps[c.comp].blocked.clone().unwrap_or_default();
        fa.skipped.push(Skipped {
            line: c.line,
            text: c.text,
            reason,
        });
    }
    fa.candidates = keep;
    for (i, c) in fa.candidates.iter_mut().enumerate() {
        c.id = i;
    }
    fa
}

struct Analyzer<'s, 'f> {
    src: &'s str,
    path: &'s str,
    lib: Lib,
    line_starts: Vec<usize>,
    fa: &'f mut FileAnalysis,
    /// Function start offset -> component name (from `const X = () =>`, `memo(...)`, default export).
    comp_targets: HashMap<u32, String>,
    comp_stack: Vec<(usize, String)>,
    fn_depth: usize,
    roles: Vec<(Option<&'static str>, String)>,
    skip_text: usize,
    t_bindings: Vec<(String, u32)>,
    text_props: BTreeMap<String, BTreeSet<String>>,
}

/// One child of a JSX element, as seen by the run builder.
enum Item<'b, 'a> {
    Text(Span),
    Str(Span, String),
    Arg(Span, &'b Expression<'a>),
    Tag(&'b JSXElement<'a>, String),
    Break(&'b JSXChild<'a>),
}

const UI_KEYS: &[&str] = &[
    "label",
    "title",
    "description",
    "placeholder",
    "text",
    "heading",
    "subtitle",
    "cta",
    "message",
    "tooltip",
    "helper",
    "caption",
];
/// Keys whose values are never copy, even in a config hook.
pub const TECH_KEYS: &[&str] = &[
    "id",
    "key",
    "href",
    "to",
    "url",
    "src",
    "icon",
    "className",
    "class",
    "type",
    "kind",
    "status",
    "variant",
    "color",
    "tone",
    "email",
    "slug",
    "path",
    "route",
    "value",
    "date",
    "currency",
    "locale",
    "format",
    "method",
    "target",
    "rel",
    "size",
];
const INLINE_TAGS: &[&str] = &[
    "b", "strong", "em", "i", "u", "a", "Link", "span", "mark", "small", "abbr",
];

impl<'s> Analyzer<'s, '_> {
    fn line(&self, offset: u32) -> u32 {
        self.line_starts.partition_point(|&s| s <= offset as usize) as u32
    }

    fn text_of(&self, span: Span) -> &'s str {
        &self.src[span.start as usize..span.end as usize]
    }

    fn line_indent(&self, offset: u32) -> String {
        let ls = self.line_starts[self.line(offset) as usize - 1];
        self.src[ls..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    fn role(&self) -> &'static str {
        self.roles.iter().rev().find_map(|r| r.0).unwrap_or("text")
    }

    fn skip(&mut self, offset: u32, text: String, reason: &str) {
        if self.skip_text > 0 || !looks_like_text(&text) {
            return;
        }
        let line = self.line(offset);
        self.fa.skipped.push(Skipped {
            line,
            text,
            reason: reason.into(),
        });
    }

    fn push(&mut self, kind: Kind, span: Span, msg: Msg, role: &str) {
        if self.skip_text > 0 {
            return;
        }
        let text = render_icu(&msg.parts);
        let Some((comp, comp_name)) = self.comp_stack.last().cloned() else {
            return self.skip(span.start, text, "not inside a React component");
        };
        if text.contains(['<', '>']) && !msg.parts.iter().any(|p| matches!(p, Part::Tag { .. })) {
            return self.skip(
                span.start,
                text,
                "contains `<` or `>` (would be read as markup)",
            );
        }
        if let Some(e) = msg.unknown_entity {
            return self.skip(span.start, text, &format!("unknown HTML entity {e}"));
        }
        if self.lib == Lib::ReactI18next && !msg.tags.is_empty() {
            return self.skip(
                span.start,
                text,
                "nested markup needs <Trans> (react-i18next)",
            );
        }
        let slug_text: String = msg
            .parts
            .iter()
            .map(|p| match p {
                Part::Text { text } | Part::Tag { text, .. } => text.as_str(),
                Part::Plural { other, .. } => other.trim_start_matches("# "),
                Part::Arg { .. } => " ",
            })
            .collect();
        let leaf = suggest_key(self.path, &slug_text, role);
        let leaf = leaf.rsplit_once('.').map_or(leaf.as_str(), |(_, l)| l);
        self.fa.candidates.push(Candidate {
            id: 0,
            suggested_key: format!("{}.{leaf}", self.fa.namespace),
            parts: msg.parts,
            text,
            role: role.into(),
            component: Some(comp_name),
            element: self.roles.last().map(|r| r.1.clone()),
            line: self.line(span.start),
            span: (span.start, span.end),
            kind: Some(kind),
            comp,
            args: msg.args,
            tags: msg.tags,
        });
    }

    /// String-ish expression (literal, template, `+` concat) as a message, if it reads as UI copy.
    fn message_of(&self, e: &Expression) -> Option<Msg> {
        let mut b = MsgBuilder::default();
        if !self.flatten(e, &mut b) || !b.has_text {
            return None;
        }
        let msg = b.finish();
        let probe: String = msg
            .parts
            .iter()
            .map(|p| match p {
                Part::Text { text } => text.clone(),
                _ => "X".into(),
            })
            .collect();
        looks_like_text(probe.trim()).then_some(msg)
    }

    fn flatten<'a>(&self, e: &Expression<'a>, b: &mut MsgBuilder) -> bool {
        fn leaves<'b, 'a>(e: &'b Expression<'a>, out: &mut Vec<&'b Expression<'a>>) {
            // Parentheses group: `"Step " + (i + 1)` is one value, not two.
            match e {
                Expression::BinaryExpression(bin) if bin.operator == BinaryOperator::Addition => {
                    leaves(&bin.left, out);
                    leaves(&bin.right, out);
                }
                x => out.push(x.get_inner_expression()),
            }
        }
        let mut ls = vec![];
        leaves(e.get_inner_expression(), &mut ls);
        let stringy = |x: &Expression| {
            matches!(
                x,
                Expression::StringLiteral(_) | Expression::TemplateLiteral(_)
            )
        };
        // ponytail: `1 + 2 + "x"` would be mis-read as a message; nobody writes that for UI copy.
        if !ls.iter().any(|x| stringy(x)) {
            return false;
        }
        for x in ls {
            match x {
                Expression::StringLiteral(s) => b.text(&s.value),
                Expression::TemplateLiteral(t) => {
                    for (i, q) in t.quasis.iter().enumerate() {
                        let Some(c) = &q.value.cooked else {
                            return false;
                        };
                        b.text(c);
                        if let Some(x) = t.expressions.get(i) {
                            if !simple_value(x.get_inner_expression())
                                && !matches!(
                                    x.get_inner_expression(),
                                    Expression::BinaryExpression(_)
                                        | Expression::ConditionalExpression(_)
                                )
                            {
                                return false;
                            }
                            b.arg(self.text_of(x.span()), x);
                        }
                    }
                }
                x => b.arg(self.text_of(x.span()), x),
            }
        }
        true
    }

    /// Candidates in an expression position (JSX child container, attribute, toast arg).
    fn collect_expr<'a>(&mut self, e: &Expression<'a>, role: &str) {
        let inner = e.get_inner_expression();
        match inner {
            Expression::StringLiteral(_)
            | Expression::TemplateLiteral(_)
            | Expression::BinaryExpression(_) => match self.message_of(inner) {
                Some(m) => self.push(Kind::Expr, inner.span(), m, role),
                None => self.visit_expression(inner),
            },
            Expression::ConditionalExpression(c) => {
                self.visit_expression(&c.test);
                self.collect_expr(&c.consequent, role);
                self.collect_expr(&c.alternate, role);
            }
            Expression::LogicalExpression(l) => {
                self.visit_expression(&l.left);
                self.collect_expr(&l.right, role);
            }
            _ => self.visit_expression(inner),
        }
    }

    fn classify<'b, 'a>(&self, child: &'b JSXChild<'a>) -> Item<'b, 'a> {
        match child {
            JSXChild::Text(t) => Item::Text(t.span),
            JSXChild::ExpressionContainer(c) => match c.expression.as_expression() {
                Some(e) => match e.get_inner_expression() {
                    Expression::StringLiteral(s) => Item::Str(c.span, s.value.to_string()),
                    Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
                        match t.quasis[0].value.cooked {
                            Some(q) => Item::Str(c.span, q.to_string()),
                            None => Item::Break(child),
                        }
                    }
                    x if simple_value(x) => Item::Arg(c.span, x),
                    _ => Item::Break(child),
                },
                None => Item::Break(child),
            },
            JSXChild::Element(el) => {
                let tag = elem_name(&el.opening_element.name);
                let plain = INLINE_TAGS.contains(&tag.as_str())
                    && el.closing_element.is_some()
                    && el.children.iter().all(|c| match c {
                        JSXChild::Text(_) => true,
                        JSXChild::ExpressionContainer(c) => {
                            matches!(
                                c.expression
                                    .as_expression()
                                    .map(|e| e.get_inner_expression()),
                                Some(Expression::StringLiteral(_))
                            )
                        }
                        _ => false,
                    })
                    && el
                        .opening_element
                        .attributes
                        .iter()
                        .all(|a| matches!(a, JSXAttributeItem::Attribute(_)));
                if plain {
                    Item::Tag(el, tag)
                } else {
                    Item::Break(child)
                }
            }
            _ => Item::Break(child),
        }
    }

    fn jsx_children<'a>(&mut self, children: &oxc_allocator::Vec<'a, JSXChild<'a>>) {
        if self.skip_text > 0 {
            for c in children {
                self.visit_jsx_child(c);
            }
            return;
        }
        let items: Vec<Item> = children.iter().map(|c| self.classify(c)).collect();
        let mut run: Vec<&Item> = vec![];
        for it in &items {
            let Item::Break(child) = it else {
                run.push(it);
                continue;
            };
            self.flush_run(std::mem::take(&mut run));
            match child {
                JSXChild::ExpressionContainer(c) => {
                    if let Some(e) = c.expression.as_expression() {
                        let role = self.role();
                        self.collect_expr(e, role);
                    }
                }
                other => self.visit_jsx_child(other),
            }
        }
        self.flush_run(run);
    }

    fn flush_run(&mut self, run: Vec<&Item>) {
        // Trim whitespace-only text at both ends.
        let blank = |it: &Item| match it {
            Item::Text(s) => jsx_clean(self.text_of(*s)).trim().is_empty(),
            Item::Str(_, s) => s.trim().is_empty(),
            _ => false,
        };
        let start = run.iter().position(|i| !blank(i));
        let end = run.iter().rposition(|i| !blank(i));
        let (Some(start), Some(end)) = (start, end) else {
            return;
        };
        let run = &run[start..=end];
        let outside_text: String = run
            .iter()
            .filter_map(|i| match i {
                Item::Text(s) => Some(jsx_clean(self.text_of(*s))),
                Item::Str(_, s) => Some(s.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        let texty = outside_text
            .split_whitespace()
            .any(|w| looks_like_text(w.trim_matches(|c: char| !c.is_alphanumeric())))
            || looks_like_text(
                &outside_text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        if !texty {
            for it in run {
                if let Item::Tag(el, _) = it {
                    self.visit_jsx_element(el);
                }
            }
            return;
        }
        let mut b = MsgBuilder::default();
        for it in run {
            match it {
                Item::Text(s) => {
                    let t = decode_entities(&jsx_clean(self.text_of(*s)), &mut b.unknown_entity);
                    b.text(&t)
                }
                Item::Str(_, s) => b.text(s),
                Item::Arg(_, e) => b.arg(self.text_of(e.span()), e),
                Item::Tag(el, tag) => {
                    let mut inner = String::new();
                    for c in &el.children {
                        match c {
                            JSXChild::Text(t) => inner.push_str(&decode_entities(
                                &jsx_clean(self.text_of(t.span)),
                                &mut b.unknown_entity,
                            )),
                            JSXChild::ExpressionContainer(c) => {
                                if let Some(Expression::StringLiteral(s)) = c
                                    .expression
                                    .as_expression()
                                    .map(|e| e.get_inner_expression())
                                {
                                    inner.push_str(&s.value);
                                }
                            }
                            _ => {}
                        }
                    }
                    let open = self.text_of(el.opening_element.span).to_string();
                    let close = el
                        .closing_element
                        .as_ref()
                        .map(|c| self.text_of(c.span).to_string())
                        .unwrap_or_default();
                    b.tag(tag, collapse(&inner), open, close);
                }
                Item::Break(_) => unreachable!(),
            }
        }
        let first = match run[0] {
            Item::Text(s) => {
                let raw = self.text_of(*s);
                s.start + (raw.len() - raw.trim_start().len()) as u32
            }
            Item::Str(s, _) | Item::Arg(s, _) => s.start,
            Item::Tag(el, _) => el.span.start,
            Item::Break(_) => unreachable!(),
        };
        let last = match run[run.len() - 1] {
            Item::Text(s) => {
                let raw = self.text_of(*s);
                s.end - (raw.len() - raw.trim_end().len()) as u32
            }
            Item::Str(s, _) | Item::Arg(s, _) => s.end,
            Item::Tag(el, _) => el.span.end,
            Item::Break(_) => unreachable!(),
        };
        let msg = b.finish();
        let role = self.role();
        self.push(Kind::Children, Span::new(first, last), msg, role);
    }

    fn enter_fn(
        &mut self,
        span: Span,
        name: Option<String>,
        is_async: bool,
        hook: Option<Hook>,
        config: bool,
    ) -> bool {
        self.fn_depth += 1;
        let name = name.or_else(|| self.comp_targets.get(&span.start).cloned());
        match (name, hook) {
            (Some(n), Some(hook)) if is_component_name(&n) => {
                self.fa.comps.push(Comp {
                    span: (span.start, span.end),
                    is_async,
                    hook,
                    blocked: None,
                    tname: "t",
                    config: config && n.starts_with("use"),
                    extra: vec![],
                });
                self.comp_stack.push((self.fa.comps.len() - 1, n));
                true
            }
            _ => false,
        }
    }

    /// Inside a config hook (`useNavItems() { return [...] }`).
    fn in_config(&self) -> bool {
        self.comp_stack
            .last()
            .is_some_and(|(c, _)| self.fa.comps[*c].config)
    }

    /// `export const metadata = { title: "..." }` -> `export async function generateMetadata()`.
    fn metadata_decl(&mut self, decl: &VariableDeclaration<'_>) -> bool {
        let [d] = decl.declarations.as_slice() else {
            return false;
        };
        let (BindingPattern::BindingIdentifier(id), Some(init)) = (&d.id, &d.init) else {
            return false;
        };
        let obj = init.get_inner_expression();
        if id.name != "metadata"
            || self.lib != Lib::NextIntl
            || self.fa.use_client
            || !matches!(obj, Expression::ObjectExpression(_))
        {
            return false;
        }
        let ret = d
            .type_annotation
            .as_ref()
            .map(|a| format!(": Promise<{}>", self.text_of(a.type_annotation.span())))
            .unwrap_or_default();
        let (start, end) = (init.span().start, init.span().end);
        self.fa.comps.push(Comp {
            span: (decl.span.start, decl.span.end),
            is_async: true,
            hook: Hook::Expr {
                start,
                end,
                indent: "  ".into(),
                outer: String::new(),
            },
            blocked: None,
            tname: "t",
            config: false,
            extra: vec![
                (
                    decl.span.start,
                    start,
                    format!("async function generateMetadata(){ret} "),
                ),
                (end, decl.span.end, String::new()),
            ],
        });
        self.comp_stack
            .push((self.fa.comps.len() - 1, "generateMetadata".into()));
        self.fn_depth += 1;
        self.visit_expression(init);
        self.fn_depth -= 1;
        self.comp_stack.pop();
        true
    }

    fn block_hook(&self, body: &FunctionBody, fn_start: u32) -> Hook {
        let pos = body
            .directives
            .last()
            .map_or(body.span.start + 1, |d| d.span.end);
        let indent = match body.statements.first() {
            Some(s) if self.line(s.span().start) != self.line(body.span.start) => {
                self.line_indent(s.span().start)
            }
            _ => format!("{}  ", self.line_indent(fn_start)),
        };
        Hook::Block { pos, indent }
    }
}

impl<'a> Visit<'a> for Analyzer<'_, '_> {
    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        if it.name == "t" || it.name == "tr" {
            self.t_bindings.push((it.name.to_string(), it.span.start));
        }
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        match &it.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) if f.id.is_none() => {
                self.comp_targets.insert(f.span.start, "Default".into());
            }
            ExportDefaultDeclarationKind::ArrowFunctionExpression(f) => {
                self.comp_targets.insert(f.span.start, "Default".into());
            }
            _ => {}
        }
        walk::walk_export_default_declaration(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init) {
            let target =
                match init.get_inner_expression() {
                    e @ (Expression::ArrowFunctionExpression(_)
                    | Expression::FunctionExpression(_)) => Some(e.span().start),
                    Expression::CallExpression(c) => c
                        .arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .filter(|e| {
                            matches!(
                                e,
                                Expression::ArrowFunctionExpression(_)
                                    | Expression::FunctionExpression(_)
                            )
                        })
                        .map(|e| e.span().start),
                    _ => None,
                };
            if let Some(t) = target {
                self.comp_targets.insert(t, id.name.to_string());
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let hook = it.body.as_ref().map(|b| self.block_hook(b, it.span.start));
        let name = it.id.as_ref().map(|i| i.name.to_string());
        let config = it.body.as_ref().is_some_and(|b| returns_literal(b));
        let is_comp = self.enter_fn(it.span, name, it.r#async, hook, config);
        walk::walk_function(self, it, flags);
        self.fn_depth -= 1;
        if is_comp {
            self.comp_stack.pop();
        }
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let hook = match it.get_function_body() {
            Some(b) => Some(self.block_hook(b, it.span.start)),
            None => it.get_expression().map(|e| {
                let outer = self.line_indent(it.span.start);
                Hook::Expr {
                    start: e.span().start,
                    end: e.span().end,
                    indent: format!("{outer}  "),
                    outer,
                }
            }),
        };
        let config = match it.get_function_body() {
            Some(b) => returns_literal(b),
            None => it
                .get_expression()
                .is_some_and(|e| is_literal(e.get_inner_expression())),
        };
        let is_comp = self.enter_fn(it.span, None, it.r#async, hook, config);
        walk::walk_arrow_function_expression(self, it);
        self.fn_depth -= 1;
        if is_comp {
            self.comp_stack.pop();
        }
    }

    fn visit_export_declaration(&mut self, it: &ExportDeclaration<'a>) {
        if let Declaration::VariableDeclaration(d) = &it.declaration
            && self.metadata_decl(d)
        {
            return;
        }
        walk::walk_export_declaration(self, it);
    }

    fn visit_array_expression_element(&mut self, it: &ArrayExpressionElement<'a>) {
        if self.in_config()
            && let Some(e) = it.as_expression()
            && matches!(
                e.get_inner_expression(),
                Expression::StringLiteral(_) | Expression::TemplateLiteral(_)
            )
            && let Some(m) = self.message_of(e)
        {
            self.push(Kind::Expr, e.get_inner_expression().span(), m, "text");
            return;
        }
        walk::walk_array_expression_element(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if self.fn_depth == 0
            && let Some(k) = it.key.static_name()
            && UI_KEYS.contains(&k.as_ref())
            && let Expression::StringLiteral(s) = it.value.get_inner_expression()
        {
            self.skip(
                s.span.start,
                s.value.to_string(),
                "module-level UI config: store a key and translate at render (t(item.labelKey))",
            );
        }
        // Inside a component the translator is in scope: `label: "Team"` -> `label: t("team")`.
        if self.fn_depth > 0
            && !self.comp_stack.is_empty()
            && let Some(k) = it.key.static_name()
            && (UI_KEYS.contains(&k.as_ref())
                || self.in_config() && !TECH_KEYS.contains(&k.as_ref()))
            && let Some(m) = self.message_of(&it.value)
        {
            let in_metadata = self
                .comp_stack
                .last()
                .is_some_and(|(_, n)| n == "generateMetadata");
            let role = match attr_role(&k) {
                // Page `<title>` / meta description, not a hover tooltip.
                _ if in_metadata && k == "title" => "heading",
                _ if in_metadata => "text",
                Some(r) => r,
                None if UI_KEYS.contains(&k.as_ref()) => "label",
                None => "text",
            };
            self.push(Kind::Expr, it.value.get_inner_expression().span(), m, role);
            return;
        }
        walk::walk_object_property(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let name = callee_name(&it.callee).unwrap_or_default();
        if (name == "toast" || name.starts_with("toast.") || name == "alert" || name == "confirm")
            && let Some(first) = it.arguments.first().and_then(|a| a.as_expression())
        {
            self.collect_expr(first, "toast");
            for a in it.arguments.iter().skip(1) {
                self.visit_argument(a);
            }
            return;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        let tag = elem_name(&it.opening_element.name);
        let skip = skips_text(&tag, &it.opening_element);
        self.roles.push((tag_role(&tag), tag.clone()));
        self.skip_text += skip as usize;
        self.visit_jsx_opening_element(&it.opening_element);
        self.jsx_children(&it.children);
        self.skip_text -= skip as usize;
        self.roles.pop();
    }

    fn visit_jsx_fragment(&mut self, it: &JSXFragment<'a>) {
        self.jsx_children(&it.children);
    }

    fn visit_jsx_attribute(&mut self, it: &JSXAttribute<'a>) {
        let role = match &it.name {
            JSXAttributeName::Identifier(id) => attr_role(&id.name).or_else(|| {
                // Prop the target component renders as text: `<StatCard delta="+12%" />`.
                let comp = &self.roles.last()?.1;
                self.text_props
                    .get(comp)?
                    .contains(id.name.as_str())
                    .then_some("text")
            }),
            JSXAttributeName::NamespacedName(_) => None,
        };
        match (role, &it.value) {
            (Some(r), Some(JSXAttributeValue::StringLiteral(s))) => {
                let text = s.value.to_string();
                if looks_like_text(&text) {
                    let mut b = MsgBuilder::default();
                    b.text(&text);
                    let msg = b.finish();
                    self.push(Kind::AttrString, s.span, msg, r);
                }
            }
            (Some(r), Some(JSXAttributeValue::ExpressionContainer(c))) => {
                if let Some(e) = c.expression.as_expression() {
                    self.collect_expr(e, r);
                }
            }
            _ => walk::walk_jsx_attribute(self, it),
        }
    }
}

/// String that reads as UI copy (same heuristic as the hardcoded-string scan).
pub fn looks_like_copy(s: &str) -> bool {
    looks_like_text(s.trim())
}

/// `Button`, `useNavItems`: functions that may hold a translator.
pub fn is_component_name(n: &str) -> bool {
    n.starts_with(|c: char| c.is_ascii_uppercase())
        || n.strip_prefix("use")
            .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase()))
}

fn is_literal(e: &Expression) -> bool {
    matches!(
        e,
        Expression::ArrayExpression(_) | Expression::ObjectExpression(_)
    )
}

/// Body is only `return <array|object literal>`.
fn returns_literal(b: &FunctionBody) -> bool {
    matches!(b.statements.as_slice(), [Statement::ReturnStatement(r)]
        if r.argument.as_ref().is_some_and(|e| is_literal(e.get_inner_expression())))
}

/// Values safe to pass as message arguments (primitives in practice).
fn simple_value(e: &Expression) -> bool {
    let name = |s: &str| -> bool {
        let l = s.to_ascii_lowercase();
        !(l == "children"
            || l.ends_with("icon")
            || l.ends_with("node")
            || l.ends_with("element")
            || l.ends_with("content")
            || l.ends_with("slot"))
    };
    match e {
        Expression::Identifier(id) => name(&id.name),
        Expression::StaticMemberExpression(m) => {
            name(&m.property.name) && simple_value(m.object.get_inner_expression())
                || matches!(
                    m.object.get_inner_expression(),
                    Expression::ThisExpression(_) | Expression::CallExpression(_)
                )
        }
        Expression::ComputedMemberExpression(_) => true,
        Expression::CallExpression(c) => {
            let callee = callee_name(&c.callee).unwrap_or_default();
            let last = callee.rsplit('.').next().unwrap_or_default();
            // Value-returning helpers (`formatDate`, `daysUntil`, `user.name.trim`) become message args;
            // components and render helpers (`Badge(...)`, `renderRow(...)`) may return JSX: keep them out.
            !last.is_empty()
                && !last.starts_with(|ch: char| ch.is_ascii_uppercase())
                && !last.starts_with("render")
                && !last.starts_with("use")
                // `items.map((i) => <li>..</li>)` renders JSX through its callback: not a value.
                && !c.arguments.iter().any(|a| {
                    matches!(
                        a.as_expression(),
                        Some(
                            Expression::JSXElement(_)
                                | Expression::JSXFragment(_)
                                | Expression::ArrowFunctionExpression(_)
                                | Expression::FunctionExpression(_)
                        )
                    )
                })
        }
        Expression::NumericLiteral(_) => true,
        _ => false,
    }
}

fn arg_name(e: &Expression) -> String {
    let raw = match e.get_inner_expression() {
        Expression::Identifier(id) => id.name.to_string(),
        Expression::StaticMemberExpression(m) => {
            if m.property.name == "length" {
                let obj = arg_name(&m.object);
                return if obj == "value" {
                    "count".into()
                } else {
                    format!("{obj}_count")
                };
            }
            m.property.name.to_string()
        }
        Expression::CallExpression(c) => match c.callee.get_inner_expression() {
            Expression::StaticMemberExpression(m) => return arg_name(&m.object),
            _ => match c.arguments.first().and_then(|a| a.as_expression()) {
                Some(a @ (Expression::Identifier(_) | Expression::StaticMemberExpression(_))) => {
                    return arg_name(a);
                }
                _ => {
                    let f = snake(&callee_name(&c.callee).unwrap_or_default());
                    ["format_", "get_", "to_"]
                        .iter()
                        .find_map(|p| f.strip_prefix(p))
                        .map_or(f.clone(), String::from)
                }
            },
        },
        Expression::BinaryExpression(b) => return arg_name(&b.left),
        _ => String::new(),
    };
    let s = snake(&raw);
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        "value".into()
    } else {
        s
    }
}

fn numeric_ish(src: &str, name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    src.ends_with(".length")
        || [
            "count",
            "total",
            "num",
            "qty",
            "quantity",
            "seats",
            "days",
            "hours",
            "minutes",
            "remaining",
            "left",
            "size",
            "limit",
            "page",
        ]
        .iter()
        .any(|w| l.contains(w))
        || l == "n"
}

#[derive(Default)]
struct MsgBuilder {
    parts: Vec<Part>,
    args: Vec<(String, String)>,
    tags: Vec<(String, String, String)>,
    has_text: bool,
    unknown_entity: Option<String>,
    /// Pending arg per part index: (name, src, numeric-ish).
    numeric: Vec<bool>,
}

struct Msg {
    parts: Vec<Part>,
    args: Vec<(String, String)>,
    tags: Vec<(String, String, String)>,
    unknown_entity: Option<String>,
}

impl MsgBuilder {
    fn text(&mut self, s: &str) {
        if s.chars().any(char::is_alphabetic) {
            self.has_text = true;
        }
        if let Some(Part::Text { text }) = self.parts.last_mut() {
            text.push_str(s);
        } else {
            self.parts.push(Part::Text {
                text: s.to_string(),
            });
            self.numeric.push(false);
        }
    }

    fn arg(&mut self, src: &str, e: &Expression) {
        let base = arg_name(e);
        let name = match self.args.iter().find(|(_, s)| s == src) {
            Some((n, _)) => n.clone(),
            None => {
                let mut n = base.clone();
                let mut i = 2;
                while self.args.iter().any(|(a, _)| *a == n) {
                    n = format!("{base}{i}");
                    i += 1;
                }
                self.args.push((n.clone(), src.to_string()));
                n
            }
        };
        self.numeric.push(numeric_ish(src, &name));
        self.parts.push(Part::Arg { name });
    }

    fn tag(&mut self, tag: &str, text: String, open: String, close: String) {
        let base = snake(tag);
        let base = if base.is_empty() {
            "tag".to_string()
        } else {
            base
        };
        let mut name = base.clone();
        let mut i = 2;
        while self.tags.iter().any(|(n, _, _)| *n == name)
            || self.args.iter().any(|(n, _)| *n == name)
        {
            name = format!("{base}{i}");
            i += 1;
        }
        self.has_text |= text.chars().any(char::is_alphabetic);
        self.tags.push((name.clone(), open, close));
        self.parts.push(Part::Tag { name, text });
        self.numeric.push(false);
    }

    fn finish(mut self) -> Msg {
        // Collapse whitespace and trim the ends.
        for p in &mut self.parts {
            if let Part::Text { text } = p {
                *text = collapse_keep_edges(text);
            }
        }
        if let Some(Part::Text { text }) = self.parts.first_mut() {
            *text = text.trim_start().to_string();
        }
        if let Some(Part::Text { text }) = self.parts.last_mut() {
            *text = text.trim_end().to_string();
        }
        // Plural: `{n} invoices` -> `{count, plural, one {# invoice} other {# invoices}}`.
        if let Some(i) = (0..self.parts.len())
            .find(|&i| self.numeric[i] && plural_next(&self.parts, i).is_some())
        {
            let Part::Arg { name } = self.parts[i].clone() else {
                unreachable!()
            };
            let (phrase, rest) = plural_next(&self.parts, i).unwrap();
            // i18next selects plurals on `count`: use that name unless another arg has it.
            // ponytail: with a clash the plural keeps its own name (fine for ICU, wrong for i18next).
            let arg = if self.args.iter().any(|(n, _)| n == "count" && *n != name) {
                name.clone()
            } else {
                "count".to_string()
            };
            if let Some(a) = self.args.iter_mut().find(|(n, _)| *n == name) {
                a.0 = arg.clone();
            }
            let one = format!("# {}", singular_phrase(&phrase));
            let other = format!("# {phrase}");
            self.parts[i] = Part::Plural { arg, one, other };
            self.parts[i + 1] = Part::Text { text: rest };
        }
        self.parts
            .retain(|p| !matches!(p, Part::Text { text } if text.is_empty()));
        Msg {
            parts: self.parts,
            args: self.args,
            tags: self.tags,
            unknown_entity: self.unknown_entity,
        }
    }
}

/// After arg `i`: `" overdue invoices rest"` -> ("overdue invoices", " rest") when a plural noun follows within 2 words.
fn plural_next(parts: &[Part], i: usize) -> Option<(String, String)> {
    let Part::Text { text } = parts.get(i + 1)? else {
        return None;
    };
    let t = text.strip_prefix(' ')?;
    let mut end = 0;
    for n in 0..2 {
        let rest = &t[end..];
        let skip = if n == 0 {
            0
        } else {
            rest.strip_prefix(' ').map(|_| 1)?
        };
        let w_start = end + skip;
        let w_len = t[w_start..]
            .find(|c: char| !c.is_alphabetic())
            .unwrap_or(t.len() - w_start);
        let word = &t[w_start..w_start + w_len];
        if word.is_empty()
            || [
                "of", "the", "a", "an", "to", "in", "on", "for", "and", "or", "with", "from",
            ]
            .contains(&word)
        {
            return None;
        }
        end = w_start + w_len;
        let plural = word.len() >= 3
            && word.to_lowercase() == word
            && word.ends_with('s')
            && !["ss", "us", "is", "as", "os"]
                .iter()
                .any(|s| word.ends_with(s));
        if plural {
            return Some((t[..end].to_string(), t[end..].to_string()));
        }
    }
    None
}

fn singular_phrase(p: &str) -> String {
    match p.rsplit_once(' ') {
        Some((head, w)) => format!("{head} {}", singular(w)),
        None => singular(p),
    }
}

fn singular(w: &str) -> String {
    if let Some(s) = w.strip_suffix("ies") {
        return format!("{s}y");
    }
    for suf in ["ches", "shes", "xes", "sses", "zes"] {
        if w.ends_with(suf) {
            return w[..w.len() - 2].to_string();
        }
    }
    w[..w.len() - 1].to_string()
}

/// JSX whitespace semantics: lines are trimmed, blank lines dropped, joined with a space.
fn jsx_clean(raw: &str) -> String {
    if !raw.contains('\n') {
        return raw.to_string();
    }
    let lines: Vec<&str> = raw.split('\n').collect();
    let n = lines.len();
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let l = if i > 0 { l.trim_start() } else { l };
            if i + 1 < n { l.trim_end() } else { l }
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapse internal whitespace runs to one space, keeping a single leading/trailing space.
fn collapse_keep_edges(s: &str) -> String {
    let mut out = String::new();
    let mut ws = false;
    for c in s.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            ws = true;
        } else {
            if ws {
                out.push(' ');
                ws = false;
            }
            out.push(c);
        }
    }
    if ws {
        out.push(' ');
    }
    out
}

fn decode_entities(s: &str, unknown: &mut Option<String>) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|&j| j <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "copy" => Some('©'),
            "middot" => Some('·'),
            "times" => Some('×'),
            "rarr" => Some('→'),
            "larr" => Some('←'),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => out.push(c),
            None => {
                *unknown = Some(format!("&{ent};"));
                out.push_str(&rest[..=semi]);
            }
        }
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    out
}

fn icu_escape(s: &str) -> String {
    s.replace('{', "'{'").replace('}', "'}'")
}

pub fn render_icu(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Text { text } => icu_escape(text),
            Part::Arg { name } => format!("{{{name}}}"),
            Part::Plural { arg, one, other } => {
                format!(
                    "{{{arg}, plural, one {{{}}} other {{{}}}}}",
                    icu_escape(one),
                    icu_escape(other)
                )
            }
            Part::Tag { name, text } => format!("<{name}>{}</{name}>", icu_escape(text)),
        })
        .collect()
}

/// Catalog entries for a message: `[("", msg)]`, or `[("_one", ..), ("_other", ..)]` for i18next plurals.
pub fn render_message(parts: &[Part], syntax: Syntax) -> Vec<(String, String)> {
    if syntax != Syntax::I18next {
        return vec![(String::new(), render_icu(parts))];
    }
    let render = |one: bool| -> String {
        parts
            .iter()
            .map(|p| match p {
                Part::Text { text } => text.clone(),
                Part::Arg { name } => format!("{{{{{name}}}}}"),
                Part::Plural { arg, one: o, other } => {
                    if one { o } else { other }.replace('#', &format!("{{{{{arg}}}}}"))
                }
                Part::Tag { name, text } => format!("<{name}>{text}</{name}>"),
            })
            .collect()
    };
    if parts.iter().any(|p| matches!(p, Part::Plural { .. })) {
        vec![
            ("_one".into(), render(true)),
            ("_other".into(), render(false)),
        ]
    } else {
        vec![(String::new(), render(false))]
    }
}

/// Rewrite `source` given the full key chosen for each candidate id. Candidates without a key are left alone.
pub fn rewrite(
    fa: &FileAnalysis,
    source: &str,
    keys: &BTreeMap<usize, String>,
    lib: Lib,
) -> Result<String, String> {
    if let Some(e) = &fa.error {
        return Err(e.clone());
    }
    let ns = &fa.namespace;
    let prefix = format!("{ns}.");
    let scoped = lib == Lib::NextIntl && keys.values().all(|k| k.starts_with(&prefix));
    let mut edits: Vec<(u32, u32, String)> = vec![];
    let mut used_comps = BTreeSet::new();
    for c in &fa.candidates {
        let Some(key) = keys.get(&c.id) else { continue };
        used_comps.insert(c.comp);
        let k = if scoped {
            &key[prefix.len()..]
        } else {
            key.as_str()
        };
        let rich = !c.tags.is_empty();
        let mut vals: Vec<String> = c
            .args
            .iter()
            .map(|(n, src)| {
                if n == src {
                    n.clone()
                } else {
                    format!("{n}: {src}")
                }
            })
            .collect();
        for (n, open, close) in &c.tags {
            vals.push(format!("{n}: (chunks) => {open}{{chunks}}{close}"));
        }
        let call = format!(
            "{}{}({k:?}{})",
            fa.comps[c.comp].tname,
            if rich { ".rich" } else { "" },
            if vals.is_empty() {
                String::new()
            } else {
                format!(", {{ {} }}", vals.join(", "))
            }
        );
        let rep = match c.kind {
            Some(Kind::Expr) => call,
            _ => format!("{{{call}}}"),
        };
        edits.push((c.span.0, c.span.1, rep));
    }
    if used_comps.is_empty() {
        return Ok(source.to_string());
    }
    let ns_arg = if scoped {
        format!("{ns:?}")
    } else {
        String::new()
    };
    let (mut need_use, mut need_get) = (false, false);
    for &ci in &used_comps {
        let comp = &fa.comps[ci];
        let t = comp.tname;
        let stmt = match lib {
            Lib::ReactI18next if t == "t" => "const { t } = useTranslation();".to_string(),
            Lib::ReactI18next => format!("const {{ t: {t} }} = useTranslation();"),
            Lib::NextIntl if comp.is_async => {
                need_get = true;
                format!("const {t} = await getTranslations({ns_arg});")
            }
            Lib::NextIntl => {
                need_use = true;
                format!("const {t} = useTranslations({ns_arg});")
            }
        };
        need_use |= lib == Lib::ReactI18next;
        edits.extend(comp.extra.iter().cloned());
        match &comp.hook {
            Hook::Block { pos, indent } => edits.push((*pos, *pos, format!("\n{indent}{stmt}"))),
            Hook::Expr {
                start,
                end,
                indent,
                outer,
            } => {
                edits.push((
                    *start,
                    *start,
                    format!("{{\n{indent}{stmt}\n{indent}return "),
                ));
                edits.push((*end, *end, format!(";\n{outer}}}")));
            }
        }
    }
    let mut imports = vec![];
    match lib {
        Lib::NextIntl => {
            if need_use && !fa.imported.contains("useTranslations") {
                imports.push(r#"import { useTranslations } from "next-intl";"#);
            }
            if need_get && !fa.imported.contains("getTranslations") {
                imports.push(r#"import { getTranslations } from "next-intl/server";"#);
            }
        }
        Lib::ReactI18next => {
            if !fa.imported.contains("useTranslation") {
                imports.push(r#"import { useTranslation } from "react-i18next";"#);
            }
        }
    }
    if !imports.is_empty() {
        let block = imports.join("\n");
        match fa.import_pos {
            Some((p, true)) => edits.push((p, p, format!("\n{block}"))),
            Some((p, false)) => edits.push((p, p, format!("\n\n{block}"))),
            None => edits.push((0, 0, format!("{block}\n\n"))),
        }
    }
    // Stable sort: inserts keep their push order; an insert before a replacement at the same offset.
    edits.sort_by_key(|(s, e, _)| (*s, s != e));
    let mut out = String::with_capacity(source.len() + 256);
    let mut pos = 0usize;
    for (s, e, rep) in &edits {
        let (s, e) = (*s as usize, *e as usize);
        if s < pos {
            return Err("overlapping edits".into());
        }
        out.push_str(&source[pos..s]);
        out.push_str(rep);
        pos = e;
    }
    out.push_str(&source[pos..]);
    parses(&fa.path, &out)
        .map_err(|e| format!("rewrite does not re-parse ({e}); file left untouched"))?;
    Ok(out)
}

/// Minimal unified diff (3 lines of context). Lines are prefixed with ' ', '-', '+'.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // ponytail: O(n*m) LCS on the changed middle only; fine for source files.
    let mut lcs = vec![vec![0u32; bm.len() + 1]; am.len() + 1];
    for i in (0..am.len()).rev() {
        for j in (0..bm.len()).rev() {
            lcs[i][j] = if am[i] == bm[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    // ops: (tag, old_idx, new_idx)
    let mut ops: Vec<(char, usize, usize)> = (0..pre).map(|i| (' ', i, i)).collect();
    let (mut i, mut j) = (0, 0);
    while i < am.len() || j < bm.len() {
        if i < am.len() && j < bm.len() && am[i] == bm[j] {
            ops.push((' ', pre + i, pre + j));
            i += 1;
            j += 1;
        } else if i < am.len() && (j == bm.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            ops.push(('-', pre + i, pre + j));
            i += 1;
        } else {
            ops.push(('+', pre + i, pre + j));
            j += 1;
        }
    }
    for k in 0..suf {
        ops.push((' ', a.len() - suf + k, b.len() - suf + k));
    }
    let mut out = String::new();
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| o.0 != ' ')
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return out;
    }
    out.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(3);
        let mut end = changed[k] + 3;
        while k + 1 < changed.len() && changed[k + 1] <= end + 3 {
            k += 1;
            end = changed[k] + 3;
        }
        let end = end.min(ops.len() - 1);
        let hunk = &ops[start..=end];
        let (oc, nc) = (
            hunk.iter().filter(|o| o.0 != '+').count(),
            hunk.iter().filter(|o| o.0 != '-').count(),
        );
        out.push_str(&format!(
            "@@ -{},{oc} +{},{nc} @@\n",
            hunk[0].1 + 1,
            hunk[0].2 + 1
        ));
        for (tag, oi, ni) in hunk {
            let line = if *tag == '+' { b[*ni] } else { a[*oi] };
            out.push(*tag);
            out.push_str(line);
            out.push('\n');
        }
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(path: &str, src: &str, lib: Lib) -> (FileAnalysis, String) {
        let fa = analyze(path, src, lib);
        assert!(fa.error.is_none(), "{:?}", fa.error);
        let keys = fa
            .candidates
            .iter()
            .map(|c| (c.id, c.suggested_key.clone()))
            .collect();
        let out = rewrite(&fa, src, &keys, lib).unwrap();
        (fa, out)
    }

    fn texts(fa: &FileAnalysis) -> Vec<&str> {
        fa.candidates.iter().map(|c| c.text.as_str()).collect()
    }

    const PAGE: &str = r#"import Link from "next/link";

const NAV = [{ label: "Dashboard", href: "/" }];

export default function InvoicesPage({ user, invoices }: Props) {
  return (
    <main>
      <h1>Invoices</h1>
      <p>
        Hello {user.name}, welcome back
      </p>
      <p>You have {invoices.length} invoices</p>
      <p>Read the <Link href="/docs">docs</Link> first.</p>
      <input placeholder="Search invoices" aria-label={"Search"} />
      <span title={`Owner: ${user.name}`}>{"Due " + user.dueDate}</span>
      {invoices.length === 0 ? "No invoices yet" : <ul>{invoices.map((i) => <li key={i.id}>{i.number}</li>)}</ul>}
      <code>npm install</code>
    </main>
  );
}
"#;

    #[test]
    fn finds_whole_messages() {
        let (fa, out) = run("app/invoices/page.tsx", PAGE, Lib::NextIntl);
        assert_eq!(fa.namespace, "invoices");
        assert_eq!(
            texts(&fa),
            [
                "Invoices",
                "Hello {name}, welcome back",
                "You have {count, plural, one {# invoice} other {# invoices}}",
                "Read the <link>docs</link> first.",
                "Search invoices",
                "Search",
                "Owner: {name}",
                "Due {due_date}",
                "No invoices yet",
            ]
        );
        assert_eq!(fa.skipped.len(), 1, "{:?}", fa.skipped);
        assert!(fa.skipped[0].reason.contains("module-level"));
        assert!(
            out.contains(
                "import Link from \"next/link\";\nimport { useTranslations } from \"next-intl\";"
            ),
            "{out}"
        );
        assert!(
            out.contains("Props) {\n  const t = useTranslations(\"invoices\");\n  return ("),
            "{out}"
        );
        assert!(
            out.contains(r#">{t("due", { due_date: user.dueDate })}</span>"#),
            "{out}"
        );
        assert!(out.contains(r#"<h1>{t("invoices_title")}</h1>"#), "{out}");
        assert!(
            out.contains(r#"{t("hello_welcome_back", { name: user.name })}"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{t("you_have_invoices", { count: invoices.length })}"#),
            "{out}"
        );
        assert!(out.contains(r#"{t.rich("read_docs_first", { link: (chunks) => <Link href="/docs">{chunks}</Link> })}"#), "{out}");
        assert!(
            out.contains(
                r#"placeholder={t("search_invoices_placeholder")} aria-label={t("search_aria")}"#
            ),
            "{out}"
        );
        assert!(
            out.contains(r#"title={t("owner_tooltip", { name: user.name })}"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{invoices.length === 0 ? t("no_invoices_yet") : <ul>"#),
            "{out}"
        );
        assert!(out.contains("<code>npm install</code>"));
        parses("x.tsx", &out).unwrap();
    }

    #[test]
    fn async_server_concise_arrow_and_client() {
        let src = "export default async function Page() {\n  const data = await load();\n  return <h2>Billing settings</h2>;\n}\n";
        let (_, out) = run("app/settings/billing/page.tsx", src, Lib::NextIntl);
        assert_eq!(
            out,
            "import { getTranslations } from \"next-intl/server\";\n\nexport default async function Page() {\n  const t = await getTranslations(\"settings.billing\");\n  const data = await load();\n  return <h2>{t(\"billing_settings_title\")}</h2>;\n}\n"
        );

        let src = "\"use client\";\n\nexport const Empty = () => <p>Nothing here</p>;\n";
        let (_, out) = run("components/Empty.tsx", src, Lib::NextIntl);
        assert_eq!(
            out,
            "\"use client\";\n\nimport { useTranslations } from \"next-intl\";\n\nexport const Empty = () => {\n  const t = useTranslations(\"empty\");\n  return <p>{t(\"nothing_here\")}</p>;\n};\n"
        );
    }

    #[test]
    fn skips_and_react_i18next() {
        let src = "function helper() { return <b>Outside text</b>; }\nexport function Row({ t }) { return <td>Paid now</td>; }\nexport function Ok() { toast.success(\"Invoice sent\"); return <p>Total: {formatMoney(total)}</p>; }\n";
        let fa = analyze("components/row.tsx", src, Lib::ReactI18next);
        let reasons: Vec<&str> = fa.skipped.iter().map(|s| s.reason.as_str()).collect();
        assert_eq!(reasons, ["not inside a React component"]);
        assert_eq!(texts(&fa), ["Paid now", "Invoice sent", "Total: {total}"]);
        let keys = fa
            .candidates
            .iter()
            .map(|c| (c.id, c.suggested_key.clone()))
            .collect();
        let out = rewrite(&fa, src, &keys, Lib::ReactI18next).unwrap();
        assert!(out.contains("const { t } = useTranslation();"), "{out}");
        assert!(
            out.contains(
                r#"const { t: tr } = useTranslation(); return <td>{tr("row.paid_now")}</td>"#
            ),
            "{out}"
        );
        assert!(
            out.contains(r#"toast.success(t("row.invoice_sent"))"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{t("row.total", { total: formatMoney(total) })}"#),
            "{out}"
        );
    }

    #[test]
    fn parens_objects_and_counters() {
        let src = "export function Steps({ i, remaining }) {\n  const tabs = [{ id: \"team\", label: \"Team members\" }];\n  return <p aria-label={\"Mark step \" + (i + 1) + \" as done\"}>{`${remaining} steps left`}</p>;\n}\n";
        let (fa, out) = run("components/Steps.tsx", src, Lib::NextIntl);
        assert_eq!(
            texts(&fa),
            [
                "Team members",
                "Mark step {i} as done",
                "{count, plural, one {# step} other {# steps}} left"
            ]
        );
        assert!(out.contains(r#"label: t("team_members_label")"#), "{out}");
        assert!(
            out.contains(r#"t("mark_step_as_done_aria", { i: i + 1 })"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{t("steps_left", { count: remaining })}"#),
            "{out}"
        );
    }

    #[test]
    fn metadata_config_hooks_text_props_and_map_callbacks() {
        let src = "import type { Metadata } from \"next\";\n\nexport const metadata: Metadata = {\n  title: \"Invoices — Acme\",\n};\n\nexport default function Page() {\n  return <ul>{ITEMS.map((i) => <li key={i}>Last active {i}</li>)}</ul>;\n}\n";
        let (fa, out) = run("app/invoices/page.tsx", src, Lib::NextIntl);
        assert_eq!(texts(&fa), ["Invoices — Acme", "Last active {i}"]);
        assert!(
            out.contains("export async function generateMetadata(): Promise<Metadata> {\n  const t = await getTranslations(\"invoices\");\n  return {\n  title: t(\"invoices_acme_title\"),\n};\n}\n"),
            "{out}"
        );
        assert!(
            out.contains(r#"<li key={i}>{tr("last_active", { i })}</li>"#)
                || out.contains(r#"{t("last_active", { i })}"#),
            "{out}"
        );

        let src = "export function useNav() {\n  return [{ href: \"/\", label: \"Dashboard\", features: [\"Unlimited invoices\"], icon: \"◧\", tagline: \"For small teams\" }];\n}\n";
        let (fa, out) = run("lib/data.ts", src, Lib::NextIntl);
        assert_eq!(
            texts(&fa),
            ["Dashboard", "Unlimited invoices", "For small teams"]
        );
        assert!(
            out.contains("export function useNav() {\n  const t = useTranslations(\"lib.data\");"),
            "{out}"
        );

        set_text_props([("StatCard".to_string(), ["delta".to_string()].into())].into());
        let src = "export function Home() {\n  return <StatCard delta=\"+12% vs last month\" active=\"team\" />;\n}\n";
        let (fa, _) = run("app/page.tsx", src, Lib::NextIntl);
        set_text_props(BTreeMap::new());
        assert_eq!(texts(&fa), ["+12% vs last month"]);
    }

    #[test]
    fn i18next_plural_rendering() {
        let parts = vec![
            Part::Text {
                text: "You have ".into(),
            },
            Part::Plural {
                arg: "count".into(),
                one: "# invoice".into(),
                other: "# invoices".into(),
            },
        ];
        assert_eq!(
            render_message(&parts, Syntax::I18next),
            [
                ("_one".to_string(), "You have {{count}} invoice".to_string()),
                (
                    "_other".to_string(),
                    "You have {{count}} invoices".to_string()
                )
            ]
        );
    }

    #[test]
    fn diff_has_hunks() {
        let d = unified_diff(
            "a.tsx",
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n",
            "1\n2\n3\n4\nX\n6\n7\n8\n9\n",
        );
        assert_eq!(
            d,
            "--- a/a.tsx\n+++ b/a.tsx\n@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+X\n 6\n 7\n 8\n"
        );
    }
}
