//! Project-wide passes run before `i18nize` analyzes files one by one:
//! - props rendered as text by local components (`<StatCard delta="..." />`),
//! - module-level UI config (`export const NAV = [{ label: "Team" }]`) turned into hooks
//!   (`useNav()`) so the copy can be translated at render time.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use glot_extract::i18nize::{TECH_KEYS, is_component_name, looks_like_copy};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::scope::ScopeFlags;

pub struct Source {
    pub path: String,
    pub src: String,
}

pub fn parse<'a>(alloc: &'a Allocator, path: &str, src: &'a str) -> Option<Program<'a>> {
    let st = SourceType::from_path(path).ok()?;
    let st = if st.is_typescript() {
        st
    } else {
        st.with_jsx(true)
    };
    let r = Parser::new(alloc, src, st).parse();
    (!r.diagnostics.has_errors()).then_some(r.program)
}

/// Apply non-overlapping `(start, end, replacement)` edits.
pub fn apply_edits(src: &str, mut edits: Vec<(u32, u32, String)>) -> Option<String> {
    edits.sort_by_key(|(s, e, _)| (*s, s != e));
    let mut out = String::with_capacity(src.len() + 256);
    let mut pos = 0usize;
    for (s, e, rep) in edits {
        let (s, e) = (s as usize, e as usize);
        if s < pos {
            return None;
        }
        out.push_str(&src[pos..s]);
        out.push_str(&rep);
        pos = e;
    }
    out.push_str(&src[pos..]);
    Some(out)
}

pub fn line_indent(src: &str, offset: u32) -> String {
    let ls = src[..offset as usize].rfind('\n').map_or(0, |i| i + 1);
    src[ls..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// Insert point + indent for a statement at the top of a function body.
fn body_insert(src: &str, body: &FunctionBody, fn_start: u32) -> (u32, String) {
    let pos = body
        .directives
        .last()
        .map_or(body.span.start + 1, |d| d.span.end);
    let indent = match body.statements.first() {
        Some(s) if src[body.span.start as usize..s.span().start as usize].contains('\n') => {
            line_indent(src, s.span().start)
        }
        _ => format!("{}  ", line_indent(src, fn_start)),
    };
    (pos, indent)
}

/// Function start offset -> name, for `const X = () =>` and `const X = memo(() => ..)`.
fn fn_target(it: &VariableDeclarator) -> Option<(u32, String)> {
    let BindingPattern::BindingIdentifier(id) = &it.id else {
        return None;
    };
    let f = match it.init.as_ref()?.get_inner_expression() {
        e @ (Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)) => e,
        Expression::CallExpression(c) => c.arguments.first()?.as_expression()?,
        _ => return None,
    };
    matches!(
        f,
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
    )
    .then(|| (f.span().start, id.name.to_string()))
}

const TEXT_ATTRS: &[&str] = &["title", "placeholder", "alt", "aria-label", "label"];

// ───────────────────────────── text props ─────────────────────────────

/// (component, local name -> prop name, `props` identifier)
type PropsFrame = Option<(String, HashMap<String, String>, Option<String>)>;
/// Outermost function: (name, async, body insert point).
type OuterFn = (Option<String>, bool, Option<(u32, String)>);

#[derive(Default)]
struct PropsVisitor {
    targets: HashMap<u32, String>,
    stack: Vec<PropsFrame>,
    out: BTreeMap<String, BTreeSet<String>>,
}

impl PropsVisitor {
    fn enter(&mut self, start: u32, name: Option<String>, params: &FormalParameters) {
        let name = name.or_else(|| self.targets.get(&start).cloned());
        let frame = name
            .filter(|n| n.starts_with(|c: char| c.is_ascii_uppercase()))
            .map(|n| {
                let mut locals = HashMap::new();
                let mut whole = None;
                match params.items.first().map(|p| &p.pattern) {
                    Some(BindingPattern::ObjectPattern(o)) => {
                        for p in &o.properties {
                            let Some(key) = p.key.static_name() else {
                                continue;
                            };
                            let local = match &p.value {
                                BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
                                BindingPattern::AssignmentPattern(a) => match &a.left {
                                    BindingPattern::BindingIdentifier(id) => {
                                        Some(id.name.to_string())
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            if let Some(l) = local {
                                locals.insert(l, key.to_string());
                            }
                        }
                    }
                    Some(BindingPattern::BindingIdentifier(id)) => {
                        whole = Some(id.name.to_string())
                    }
                    _ => {}
                }
                (n, locals, whole)
            });
        // Nested non-component functions (map callbacks) keep the enclosing component's props.
        let frame = frame.or_else(|| self.stack.last().cloned().flatten());
        self.stack.push(frame);
    }

    fn prop_of(&self, e: &Expression) -> Option<(String, String)> {
        let (comp, locals, whole) = self.stack.last()?.as_ref()?;
        let prop = match e.get_inner_expression() {
            Expression::Identifier(id) => locals.get(id.name.as_str())?.clone(),
            Expression::StaticMemberExpression(m) => match m.object.get_inner_expression() {
                Expression::Identifier(o) if Some(o.name.as_str()) == whole.as_deref() => {
                    m.property.name.to_string()
                }
                _ => return None,
            },
            _ => return None,
        };
        (prop != "children").then(|| (comp.clone(), prop))
    }

    fn record(&mut self, e: &Expression) {
        if let Some((c, p)) = self.prop_of(e) {
            self.out.entry(c).or_default().insert(p);
        }
    }
}

impl<'a> Visit<'a> for PropsVisitor {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some((s, n)) = fn_target(it) {
            self.targets.insert(s, n);
        }
        walk::walk_variable_declarator(self, it);
    }
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        self.enter(
            it.span.start,
            it.id.as_ref().map(|i| i.name.to_string()),
            &it.params,
        );
        walk::walk_function(self, it, flags);
        self.stack.pop();
    }
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.enter(it.span.start, None, &it.params);
        walk::walk_arrow_function_expression(self, it);
        self.stack.pop();
    }
    fn visit_jsx_child(&mut self, it: &JSXChild<'a>) {
        if let JSXChild::ExpressionContainer(c) = it
            && let Some(e) = c.expression.as_expression()
        {
            self.record(e);
        }
        walk::walk_jsx_child(self, it);
    }
    fn visit_jsx_attribute(&mut self, it: &JSXAttribute<'a>) {
        if let JSXAttributeName::Identifier(n) = &it.name
            && TEXT_ATTRS.contains(&n.name.as_str())
            && let Some(JSXAttributeValue::ExpressionContainer(c)) = &it.value
            && let Some(e) = c.expression.as_expression()
        {
            self.record(e);
        }
        walk::walk_jsx_attribute(self, it);
    }
}

/// Component name -> props it renders as JSX text (or in a text attribute).
pub fn text_props(files: &[Source]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in files {
        let alloc = Allocator::default();
        let Some(program) = parse(&alloc, &f.path, &f.src) else {
            continue;
        };
        let mut v = PropsVisitor::default();
        v.visit_program(&program);
        for (c, ps) in v.out {
            out.entry(c).or_default().extend(ps);
        }
    }
    out
}

// ───────────────────────────── config hooks ─────────────────────────────

struct Config {
    path: String,
    name: String,
    hook: String,
}

/// `PLANS` / `STATUS_META`: SCREAMING_CASE marks UI config; camelCase data (`invoices`) is left alone.
fn screaming(n: &str) -> bool {
    n.len() >= 2
        && n.starts_with(|c: char| c.is_ascii_uppercase())
        && n.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// `plans`, `pricingTiers`: camelCase data a component maps over (`metadata` & co. are Next's).
fn camel_data(n: &str) -> bool {
    n.starts_with(|c: char| c.is_ascii_lowercase())
        && n.chars().all(|c| c.is_ascii_alphanumeric())
        && !["metadata", "viewport", "config"].contains(&n)
}

fn hook_name(n: &str) -> String {
    if !screaming(n) {
        return format!("use{}{}", n[..1].to_ascii_uppercase(), &n[1..]);
    }
    let camel: String = n
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let w = w.to_ascii_lowercase();
            w[..1].to_ascii_uppercase() + &w[1..]
        })
        .collect();
    format!("use{camel}")
}

fn has_copy(e: &Expression) -> bool {
    match e.get_inner_expression() {
        Expression::StringLiteral(s) => looks_like_copy(&s.value),
        Expression::ArrayExpression(a) => a
            .elements
            .iter()
            .any(|x| x.as_expression().is_some_and(has_copy)),
        Expression::ObjectExpression(o) => o.properties.iter().any(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) => {
                p.key
                    .static_name()
                    .is_none_or(|k| !TECH_KEYS.contains(&k.as_ref()))
                    && has_copy(&p.value)
            }
            _ => false,
        }),
        _ => false,
    }
}

/// A copy-only key (`description`, `cta`...) holds copy: `[{ name, email, role }]` is data.
fn has_ui_copy(e: &Expression) -> bool {
    const UI: &[&str] = &[
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
        "caption",
        "features",
    ];
    match e.get_inner_expression() {
        Expression::ArrayExpression(a) => a
            .elements
            .iter()
            .any(|x| x.as_expression().is_some_and(has_ui_copy)),
        Expression::ObjectExpression(o) => o.properties.iter().any(|p| match p {
            ObjectPropertyKind::ObjectProperty(p) => {
                p.key
                    .static_name()
                    .is_some_and(|k| UI.contains(&k.as_ref()))
                    && has_copy(&p.value)
                    || has_ui_copy(&p.value)
            }
            _ => false,
        }),
        _ => false,
    }
}

/// Module-level `const NAME = <literal with copy>`: (declaration, declarator).
fn config_decls<'b, 'a>(
    program: &'b Program<'a>,
) -> Vec<(&'b VariableDeclaration<'a>, &'b VariableDeclarator<'a>)> {
    program
        .body
        .iter()
        .filter_map(|s| match s {
            Statement::VariableDeclaration(d) => Some(&**d),
            Statement::ExportDeclaration(e) => match &e.declaration {
                Declaration::VariableDeclaration(d) => Some(&**d),
                _ => None,
            },
            _ => None,
        })
        .filter(|d| d.kind == VariableDeclarationKind::Const && d.declarations.len() == 1)
        .filter_map(|d| {
            let decl = &d.declarations[0];
            let BindingPattern::BindingIdentifier(id) = &decl.id else {
                return None;
            };
            let init = decl.init.as_ref()?;
            ((screaming(&id.name) || camel_data(&id.name) && has_ui_copy(init))
                && matches!(
                    init.get_inner_expression(),
                    Expression::ArrayExpression(_) | Expression::ObjectExpression(_)
                )
                && has_copy(init))
            .then_some((d, decl))
        })
        .collect()
}

/// Resolve an import specifier to a project file (relative, `@/` and `~/` aliases).
fn resolve(from: &str, spec: &str, known: &BTreeSet<&str>) -> Option<String> {
    let base = if spec.starts_with("./") || spec.starts_with("../") {
        let dir = Path::new(from).parent().unwrap_or(Path::new(""));
        let mut parts: Vec<String> = dir
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        for seg in spec.split('/') {
            match seg {
                "." | "" => {}
                ".." => {
                    parts.pop()?;
                }
                s => parts.push(s.to_string()),
            }
        }
        vec![parts.join("/")]
    } else {
        // ponytail: assumes the usual `"@/*": ["./*"]` or `["./src/*"]` tsconfig paths.
        let rest = spec
            .strip_prefix("@/")
            .or_else(|| spec.strip_prefix("~/"))?;
        vec![
            rest.to_string(),
            format!("src/{rest}"),
            format!("app/{rest}"),
        ]
    };
    const EXTS: &[&str] = &[
        "",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        "/index.ts",
        "/index.tsx",
        "/index.js",
        "/index.jsx",
    ];
    base.iter()
        .flat_map(|b| EXTS.iter().map(move |e| format!("{b}{e}")))
        .find(|c| known.contains(c.as_str()))
}

#[derive(Default)]
struct Uses {
    refs: Vec<Ref>,
    unsafe_: Vec<(String, String)>,
}

struct Ref {
    name: String,
    span: (u32, u32),
    kind: RefKind,
}

enum RefKind {
    Type,
    /// Value read inside a component/hook body: hoist `const NAME = useName()` there.
    InComponent(u32, String),
    Unsafe(String),
}

struct RefVisitor<'s> {
    src: &'s str,
    watched: &'s BTreeSet<String>,
    targets: HashMap<u32, String>,
    outer: Vec<OuterFn>,
    uses: Uses,
}

impl RefVisitor<'_> {
    fn enter(
        &mut self,
        start: u32,
        name: Option<String>,
        is_async: bool,
        body: Option<(u32, String)>,
    ) {
        let name = name.or_else(|| self.targets.get(&start).cloned());
        self.outer.push((name, is_async, body));
    }
}

impl<'a> Visit<'a> for RefVisitor<'_> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some((s, n)) = fn_target(it) {
            self.targets.insert(s, n);
        }
        walk::walk_variable_declarator(self, it);
    }
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let body = it
            .body
            .as_ref()
            .map(|b| body_insert(self.src, b, it.span.start));
        self.enter(
            it.span.start,
            it.id.as_ref().map(|i| i.name.to_string()),
            it.r#async,
            body,
        );
        walk::walk_function(self, it, flags);
        self.outer.pop();
    }
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let body = it
            .get_function_body()
            .map(|b| body_insert(self.src, b, it.span.start));
        self.enter(it.span.start, None, it.r#async, body);
        walk::walk_arrow_function_expression(self, it);
        self.outer.pop();
    }
    fn visit_ts_type_query(&mut self, it: &TSTypeQuery<'a>) {
        if let TSTypeQueryExprName::IdentifierReference(id) = &it.expr_name
            && self.watched.contains(id.name.as_str())
        {
            self.uses.refs.push(Ref {
                name: id.name.to_string(),
                span: (it.span.start, it.span.end),
                kind: RefKind::Type,
            });
            return;
        }
        walk::walk_ts_type_query(self, it);
    }
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if !self.watched.contains(it.name.as_str()) {
            return;
        }
        let kind = match self.outer.first() {
            None => RefKind::Unsafe("used at module level".into()),
            Some((Some(n), false, Some((pos, indent)))) if is_component_name(n) => {
                RefKind::InComponent(*pos, indent.clone())
            }
            Some((Some(n), true, _)) => RefKind::Unsafe(format!("used in async `{n}`")),
            Some((n, _, _)) => RefKind::Unsafe(format!(
                "used in `{}`, not a component",
                n.as_deref().unwrap_or("anonymous function")
            )),
        };
        self.uses.refs.push(Ref {
            name: it.name.to_string(),
            span: (it.span.start, it.span.end),
            kind,
        });
    }
    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        for s in &it.specifiers {
            if self.watched.contains(s.local.name().as_str()) {
                self.uses
                    .unsafe_
                    .push((s.local.name().to_string(), "re-exported by name".into()));
            }
        }
    }
}

pub struct ConfigPlan {
    /// (path, old, new)
    pub files: Vec<(String, String, String)>,
    pub converted: Vec<String>,
    pub warnings: Vec<String>,
}

/// Turn module-level UI config into hooks when every use is inside a (non-async) component.
pub fn config_hooks(files: &[Source]) -> ConfigPlan {
    let known: BTreeSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut configs: Vec<Config> = vec![];
    // Every `use*` identifier in the project, once: a per-config `contains` over all files was
    // O(configs x project size) (minutes on a repo with large data/dictionary modules).
    let taken: BTreeSet<&str> = files
        .iter()
        .flat_map(|f| {
            f.src
                .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        })
        .filter(|w| w.starts_with("use"))
        .collect();
    // Files `glot i18nize` generated (i18n.ts: `LOCALES = ["en", "fr"]`) hold no UI copy.
    for f in files
        .iter()
        .filter(|f| !f.src.contains("Generated by `glot i18nize`"))
    {
        let alloc = Allocator::default();
        let Some(program) = parse(&alloc, &f.path, &f.src) else {
            continue;
        };
        for (_, d) in config_decls(&program) {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            let mut hook = hook_name(&id.name);
            if taken.contains(hook.as_str()) {
                hook.push_str("Config");
            }
            configs.push(Config {
                path: f.path.clone(),
                name: id.name.to_string(),
                hook,
            });
        }
    }
    let mut plan = ConfigPlan {
        files: vec![],
        converted: vec![],
        warnings: vec![],
    };
    if configs.is_empty() {
        return plan;
    }

    // Per file: which config names it sees (own + imported), import specifier spans, references.
    let mut unsafe_: BTreeMap<(String, String), String> = BTreeMap::new();
    #[allow(clippy::type_complexity)]
    let mut per_file: Vec<(
        usize,
        Vec<(String, (u32, u32))>,
        Vec<Ref>,
        BTreeMap<String, String>,
    )> = vec![];
    for (fi, f) in files.iter().enumerate() {
        let alloc = Allocator::default();
        let Some(program) = parse(&alloc, &f.path, &f.src) else {
            continue;
        };
        // local name -> defining file
        let mut visible: BTreeMap<String, String> = configs
            .iter()
            .filter(|c| c.path == f.path)
            .map(|c| (c.name.clone(), c.path.clone()))
            .collect();
        let mut specs = vec![];
        for s in &program.body {
            let (spec, items): (&str, Vec<&ImportDeclarationSpecifier>) = match s {
                Statement::ImportDeclaration(d) => (
                    d.source.value.as_str(),
                    d.specifiers.iter().flatten().collect(),
                ),
                Statement::ExportFromDeclaration(d) => {
                    if let Some(target) = resolve(&f.path, d.source.value.as_str(), &known) {
                        for c in configs.iter().filter(|c| c.path == target) {
                            if d.specifiers
                                .iter()
                                .any(|s| s.local.name() == c.name.as_str())
                            {
                                unsafe_.insert(
                                    (c.path.clone(), c.name.clone()),
                                    format!("re-exported from {}", f.path),
                                );
                            }
                        }
                    }
                    continue;
                }
                Statement::ExportAllDeclaration(d) => {
                    if let Some(target) = resolve(&f.path, d.source.value.as_str(), &known) {
                        for c in configs.iter().filter(|c| c.path == target) {
                            unsafe_.insert(
                                (c.path.clone(), c.name.clone()),
                                format!("re-exported from {}", f.path),
                            );
                        }
                    }
                    continue;
                }
                _ => continue,
            };
            let Some(target) = resolve(&f.path, spec, &known) else {
                continue;
            };
            for item in items {
                match item {
                    ImportDeclarationSpecifier::ImportSpecifier(is) => {
                        let Some(c) = configs
                            .iter()
                            .find(|c| c.path == target && is.imported.name() == c.name.as_str())
                        else {
                            continue;
                        };
                        if is.local.name != c.name.as_str() {
                            unsafe_.insert(
                                (c.path.clone(), c.name.clone()),
                                format!("imported under another name in {}", f.path),
                            );
                        } else {
                            visible.insert(c.name.clone(), c.path.clone());
                            specs.push((c.name.clone(), (is.span.start, is.span.end)));
                        }
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                        for c in configs.iter().filter(|c| c.path == target) {
                            unsafe_.insert(
                                (c.path.clone(), c.name.clone()),
                                format!("namespace import in {}", f.path),
                            );
                        }
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {}
                }
            }
        }
        if visible.is_empty() {
            continue;
        }
        let watched: BTreeSet<String> = visible.keys().cloned().collect();
        let mut v = RefVisitor {
            src: &f.src,
            watched: &watched,
            targets: HashMap::new(),
            outer: vec![],
            uses: Uses::default(),
        };
        v.visit_program(&program);
        let key = |n: &str| (visible[n].clone(), n.to_string());
        for (n, why) in v.uses.unsafe_ {
            unsafe_.insert(key(&n), format!("{why} in {}", f.path));
        }
        for r in &v.uses.refs {
            if let RefKind::Unsafe(why) = &r.kind {
                let line = f.src[..r.span.0 as usize].matches('\n').count() + 1;
                unsafe_.insert(key(&r.name), format!("{why} ({}:{line})", f.path));
            }
        }
        per_file.push((fi, specs, v.uses.refs, visible));
    }

    // camelCase data is converted only when a component reads it (else it may be plain data).
    let rendered: BTreeSet<(String, String)> = per_file
        .iter()
        .flat_map(|(_, _, refs, visible)| {
            refs.iter()
                .filter(|r| matches!(r.kind, RefKind::InComponent(..)))
                .map(|r| (visible[&r.name].clone(), r.name.clone()))
        })
        .collect();
    configs.retain(|c| screaming(&c.name) || rendered.contains(&(c.path.clone(), c.name.clone())));
    let ok = |c: &Config| !unsafe_.contains_key(&(c.path.clone(), c.name.clone()));
    for c in &configs {
        match unsafe_.get(&(c.path.clone(), c.name.clone())) {
            Some(why) => plan.warnings.push(format!(
                "{}: `{}` left as is ({why}); translate it at render with t(item.labelKey)",
                c.path, c.name
            )),
            None => plan
                .converted
                .push(format!("{}: {} -> {}()", c.path, c.name, c.hook)),
        }
    }
    let hook_of = |def: &str, name: &str| {
        configs
            .iter()
            .find(|c| c.name == name && c.path == def)
            .filter(|c| ok(c))
            .map(|c| c.hook.clone())
    };

    let mut by_file: BTreeMap<usize, Vec<(u32, u32, String)>> = BTreeMap::new();
    // Declarations -> hooks.
    for (fi, f) in files.iter().enumerate() {
        let alloc = Allocator::default();
        let Some(program) = parse(&alloc, &f.path, &f.src) else {
            continue;
        };
        for (decl, d) in config_decls(&program) {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            let Some(c) = configs
                .iter()
                .find(|c| c.path == f.path && c.name == id.name.as_str())
                .filter(|c| ok(c))
            else {
                continue;
            };
            let init = d.init.as_ref().unwrap().span();
            let ret = d
                .type_annotation
                .as_ref()
                .map(|a| {
                    let s = a.type_annotation.span();
                    format!(": {}", &f.src[s.start as usize..s.end as usize])
                })
                .unwrap_or_default();
            let e = by_file.entry(fi).or_default();
            e.push((
                decl.span.start,
                init.start,
                format!("function {}(){ret} {{\n  return ", c.hook),
            ));
            e.push((init.end, decl.span.end, ";\n}".into()));
        }
    }
    // Imports, type queries, hoisted `const NAME = useName();`.
    for (fi, specs, refs, visible) in per_file {
        let e = by_file.entry(fi).or_default();
        for (name, (s, end)) in specs {
            if let Some(h) = hook_of(&visible[&name], &name) {
                e.push((s, end, h));
            }
        }
        let mut hoisted = BTreeSet::new();
        for r in refs {
            let Some(h) = hook_of(&visible[&r.name], &r.name) else {
                continue;
            };
            match r.kind {
                RefKind::Type => e.push((r.span.0, r.span.1, format!("ReturnType<typeof {h}>"))),
                RefKind::InComponent(pos, indent) => {
                    if hoisted.insert((pos, r.name.clone())) {
                        e.push((pos, pos, format!("\n{indent}const {} = {h}();", r.name)));
                    }
                }
                RefKind::Unsafe(_) => {}
            }
        }
    }
    for (fi, edits) in by_file {
        if edits.is_empty() {
            continue;
        }
        let f = &files[fi];
        let alloc = Allocator::default();
        match apply_edits(&f.src, edits) {
            Some(new) if parse(&alloc, &f.path, &new).is_some() => {
                plan.files.push((f.path.clone(), f.src.clone(), new))
            }
            _ => plan.warnings.push(format!(
                "{}: config refactor skipped (edits do not apply)",
                f.path
            )),
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(path: &str, s: &str) -> Source {
        Source {
            path: path.into(),
            src: s.into(),
        }
    }

    #[test]
    fn finds_text_props() {
        let files = [src(
            "components/StatCard.tsx",
            "export function StatCard({ label, value, delta, tone }: P) {\n  return <div title={label}><p>{value}</p><p className={tone}>{delta}</p></div>;\n}\nexport const Row = (props: Q) => <td>{props.hint}</td>;\n",
        )];
        let p = text_props(&files);
        assert_eq!(
            p["StatCard"].iter().map(String::as_str).collect::<Vec<_>>(),
            ["delta", "label", "value"]
        );
        assert!(p["Row"].contains("hint"));
    }

    #[test]
    fn config_to_hooks_across_files() {
        let files = [
            src(
                "lib/data.ts",
                "export const NAV = [\n  { href: \"/\", label: \"Dashboard\" },\n];\n\nexport const PLANS = [{ name: \"Starter\", cta: \"Start for free\" }];\n\nexport const invoices = [{ description: \"Consulting hours\" }];\n",
            ),
            src(
                "components/Sidebar.tsx",
                "import { NAV } from \"@/lib/data\";\n\nexport function Sidebar() {\n  return <nav>{NAV.map((i) => <a key={i.href}>{i.label}</a>)}</nav>;\n}\n",
            ),
            src(
                "components/PlanCard.tsx",
                "import { PLANS } from \"../lib/data\";\n\nexport function PlanCard({ plan }: { plan: (typeof PLANS)[number] }) {\n  return <p>{plan.cta}</p>;\n}\nconst FIRST = PLANS[0];\n",
            ),
        ];
        let plan = config_hooks(&files);
        assert_eq!(plan.converted, ["lib/data.ts: NAV -> useNav()"]);
        assert_eq!(plan.warnings.len(), 1, "{:?}", plan.warnings);
        assert!(plan.warnings[0].contains("PLANS") && plan.warnings[0].contains("module level"));
        let get = |p: &str| plan.files.iter().find(|f| f.0 == p).map(|f| f.2.as_str());
        assert_eq!(
            get("lib/data.ts").unwrap(),
            "export function useNav() {\n  return [\n  { href: \"/\", label: \"Dashboard\" },\n];\n}\n\nexport const PLANS = [{ name: \"Starter\", cta: \"Start for free\" }];\n\nexport const invoices = [{ description: \"Consulting hours\" }];\n"
        );
        assert_eq!(
            get("components/Sidebar.tsx").unwrap(),
            "import { useNav } from \"@/lib/data\";\n\nexport function Sidebar() {\n  const NAV = useNav();\n  return <nav>{NAV.map((i) => <a key={i.href}>{i.label}</a>)}</nav>;\n}\n"
        );
        assert!(get("components/PlanCard.tsx").is_none());

        // camelCase data mapped by a component (create-next-app style) becomes a hook too.
        let files = [src(
            "app/page.tsx",
            "const plans = [{ name: \"Starter\", description: \"For small teams\" }];\n\nexport default function Home() {\n  return <ul>{plans.map((p) => <li key={p.name}>{p.description}</li>)}</ul>;\n}\n",
        )];
        let plan = config_hooks(&files);
        assert_eq!(plan.converted, ["app/page.tsx: plans -> usePlans()"]);
        assert!(
            plan.files[0].2.contains("const plans = usePlans();"),
            "{}",
            plan.files[0].2
        );
    }

    #[test]
    fn type_queries_and_local_configs() {
        let files = [
            src(
                "lib/data.ts",
                "export const PLANS: Plan[] = [{ name: \"Starter\", features: [\"Email support\"] }];\n",
            ),
            src(
                "components/PlanCard.tsx",
                "import { PLANS } from \"@/lib/data\";\n\nexport function PlanCard({ plan }: { plan: (typeof PLANS)[number] }) {\n  return <p>{plan.name}</p>;\n}\nexport function List() {\n  return PLANS.map((p) => <PlanCard key={p.name} plan={p} />);\n}\n",
            ),
        ];
        let plan = config_hooks(&files);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        let card = &plan
            .files
            .iter()
            .find(|f| f.0 == "components/PlanCard.tsx")
            .unwrap()
            .2;
        assert!(
            card.contains("plan: (ReturnType<typeof usePlans>)[number]"),
            "{card}"
        );
        assert!(
            card.contains("export function List() {\n  const PLANS = usePlans();"),
            "{card}"
        );
        let data = &plan.files.iter().find(|f| f.0 == "lib/data.ts").unwrap().2;
        assert!(
            data.starts_with("export function usePlans(): Plan[] {\n  return [{"),
            "{data}"
        );
    }

    #[test]
    fn technical_defaults_are_not_config_copy() {
        // Field test: a connection-defaults object and a WebRTC config held only identifiers.
        let files = [src(
            "src/pages/Setup.tsx",
            "const DEFAULTS = { host: 'postgres', user: 'root', language: 'en' };\nconst RTC = { bundlePolicy: 'max-bundle', rtcpMuxPolicy: 'require' };\nconst TABS = [{ id: 'a', label: 'General' }];\n\nexport default function Setup() {\n  return <p>{DEFAULTS.host}{RTC.bundlePolicy}{TABS[0].label}</p>;\n}\n",
        )];
        let plan = config_hooks(&files);
        assert_eq!(plan.converted, ["src/pages/Setup.tsx: TABS -> useTabs()"]);
    }
}
