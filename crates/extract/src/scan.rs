//! Single-file scanner: an oxc `Visit` that records key usages and hardcoded strings.
use std::collections::HashMap;

use glot_core::context::SourceRef;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::scope::ScopeFlags;

use crate::{ExtractReport, Hardcoded, Usage};

/// Calls that return a translator function rather than translating a key.
const HOOKS: &[&str] = &[
    "useTranslations",
    "getTranslations",
    "useTranslation",
    "useI18n",
];
/// Elements whose text content is code or a translation default, never UI copy to extract.
/// Code-like elements, or any element opting out with the HTML-standard `translate="no"`
/// (package names, commands, example values): their text and attributes are never extracted.
pub(crate) fn skips_text(tag: &str, el: &JSXOpeningElement) -> bool {
    SKIP_TEXT_IN.contains(&tag)
        || el.attributes.iter().any(|a| {
            matches!(a, JSXAttributeItem::Attribute(a)
                if matches!(&a.name, JSXAttributeName::Identifier(id) if id.name == "translate")
                    && attr_str(a).is_some_and(|(v, _)| v == "no"))
        })
}

pub(crate) const SKIP_TEXT_IN: &[&str] = &[
    "code",
    "pre",
    "kbd",
    "samp",
    "script",
    "style",
    "Trans",
    "FormattedMessage",
];

pub(crate) fn scan(path: &str, source: &str, key_fns: &[String]) -> ExtractReport {
    let vue;
    let (text, source_type) = if path.ends_with(".vue") {
        let (s, ts) = vue_scripts(source);
        vue = s;
        (
            vue.as_str(),
            SourceType::from_extension(if ts { "ts" } else { "js" }).unwrap(),
        )
    } else {
        match SourceType::from_path(path) {
            Ok(st) if st.is_typescript() => (source, st),
            Ok(st) => (source, st.with_jsx(true)),
            Err(_) => return ExtractReport::default(),
        }
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, text, source_type).parse();

    let mut line_starts = vec![0];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut s = Scanner {
        path,
        src: text,
        line_starts,
        key_fns,
        is_test: [".test.", ".spec.", "__tests__/", ".stories."]
            .iter()
            .any(|p| path.contains(p)),
        scopes: vec![HashMap::new()],
        components: vec![],
        frames: vec![],
        skip_text: 0,
        report: ExtractReport {
            files_scanned: 1,
            ..Default::default()
        },
    };
    s.visit_program(&parsed.program);
    s.report
}

/// Keep only `<script>` blocks of a Vue SFC, blanking the rest so offsets and lines stay valid.
fn vue_scripts(src: &str) -> (String, bool) {
    let mut out: Vec<u8> = src
        .bytes()
        .map(|b| if b == b'\n' { b'\n' } else { b' ' })
        .collect();
    let (mut pos, mut ts) = (0, false);
    while let Some(i) = src[pos..].find("<script") {
        let open = pos + i;
        let Some(gt) = src[open..].find('>') else {
            break;
        };
        let body = open + gt + 1;
        ts |= src[open..body].contains("lang=\"ts\"");
        let end = src[body..]
            .find("</script>")
            .map_or(src.len(), |e| body + e);
        out[body..end].copy_from_slice(&src.as_bytes()[body..end]);
        pos = end;
    }
    (String::from_utf8(out).expect("ascii boundaries"), ts)
}

#[derive(Clone, Copy)]
enum Frame {
    /// JSX element; `None` for generic containers (div, span...).
    Element(Option<&'static str>),
    /// Attribute or call argument that fixes the role for everything inside.
    Hard(Option<&'static str>),
}

struct Scanner<'s> {
    path: &'s str,
    src: &'s str,
    line_starts: Vec<usize>,
    key_fns: &'s [String],
    is_test: bool,
    /// Translator bindings per function scope: local name -> key prefix.
    scopes: Vec<HashMap<String, String>>,
    components: Vec<String>,
    frames: Vec<Frame>,
    skip_text: usize,
    report: ExtractReport,
}

impl Scanner<'_> {
    fn at(&self, offset: u32) -> SourceRef {
        SourceRef {
            path: self.path.to_string(),
            line: self.line_starts.partition_point(|&s| s <= offset as usize) as u32,
        }
    }

    fn lookup(&self, name: &str) -> Option<String> {
        self.scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }

    fn role(&self) -> Option<&'static str> {
        let mut in_jsx = false;
        for f in self.frames.iter().rev() {
            match *f {
                Frame::Element(Some(r)) => return Some(r),
                Frame::Element(None) => in_jsx = true,
                Frame::Hard(r) => return if in_jsx { Some("text") } else { r },
            }
        }
        in_jsx.then_some("text")
    }

    /// Key prefix if `name` (dotted callee) translates a key, `None` otherwise.
    fn key_prefix_for(&self, name: &str) -> Option<String> {
        if let Some(p) = self.lookup(name) {
            return Some(p);
        }
        if let Some((obj, method)) = name.rsplit_once('.') {
            if matches!(method, "rich" | "markup" | "raw" | "has") {
                return self.lookup(obj);
            }
            if matches!(method, "formatMessage" | "$t") || name == "i18next.t" {
                return Some(String::new());
            }
        }
        if HOOKS.contains(&name) {
            return None;
        }
        (name == "formatMessage" || self.key_fns.iter().any(|f| f == name)).then(String::new)
    }

    fn add_usage(&mut self, key: String, offset: u32) {
        let usage = Usage {
            at: self.at(offset),
            ui_role: self.role().map(Into::into),
            component: self.components.last().cloned(),
        };
        self.report.usages.entry(key).or_default().push(usage);
    }

    fn record_key_arg(&mut self, arg: &Expression, prefix: &str, offset: u32) {
        match arg.get_inner_expression() {
            e @ Expression::ObjectExpression(_) => {
                if let Some(id) = obj_prop(e, "id") {
                    self.record_key_arg(id, prefix, offset);
                }
            }
            Expression::ConditionalExpression(c) => {
                self.record_key_arg(&c.consequent, prefix, offset);
                self.record_key_arg(&c.alternate, prefix, offset);
            }
            e => {
                if let Some(k) = str_of(e) {
                    self.add_usage(join(prefix, strip_ns(&k)), offset);
                    return;
                }
                self.report.dynamic.push(self.at(offset));
                let quasi = match e {
                    Expression::TemplateLiteral(t) => t
                        .quasis
                        .first()
                        .and_then(|q| q.value.cooked)
                        .filter(|q| !q.is_empty()),
                    _ => None,
                };
                let dyn_prefix = match quasi {
                    Some(q) => Some(join(prefix, strip_ns(&q))),
                    None if !prefix.is_empty() => Some(format!("{prefix}.")),
                    None => None,
                };
                self.report.dynamic_prefixes.extend(dyn_prefix);
            }
        }
    }

    fn hardcoded(&mut self, raw: &str, role: &'static str, offset: u32) {
        if self.is_test || self.skip_text > 0 {
            return;
        }
        let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if !looks_like_text(&text) {
            return;
        }
        self.report.hardcoded.push(Hardcoded {
            at: self.at(offset),
            suggested_key: suggest_key(self.path, &text, role),
            text,
            ui_role: Some(role.into()),
            component: self.components.last().cloned(),
        });
    }

    fn bind(&mut self, name: &str, prefix: String) {
        self.scopes
            .last_mut()
            .expect("program scope")
            .insert(name.to_string(), prefix);
    }
}

impl<'a> Visit<'a> for Scanner<'_> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let comp = it
            .id
            .as_ref()
            .map(|id| id.name.as_str())
            .filter(|n| is_pascal(n));
        self.components.extend(comp.map(String::from));
        self.scopes.push(HashMap::new());
        walk::walk_function(self, it, flags);
        self.scopes.pop();
        if comp.is_some() {
            self.components.pop();
        }
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.scopes.push(HashMap::new());
        walk::walk_arrow_function_expression(self, it);
        self.scopes.pop();
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some(prefix) = it.init.as_ref().and_then(translator_prefix) {
            match &it.id {
                BindingPattern::BindingIdentifier(id) => self.bind(&id.name, prefix),
                BindingPattern::ObjectPattern(p) => {
                    for prop in &p.properties {
                        if let (Some("t"), BindingPattern::BindingIdentifier(id)) =
                            (prop.key.static_name().as_deref(), &prop.value)
                        {
                            self.bind(&id.name, prefix.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        let comp = match (&it.id, &it.init) {
            (
                BindingPattern::BindingIdentifier(id),
                Some(
                    Expression::ArrowFunctionExpression(_)
                    | Expression::FunctionExpression(_)
                    | Expression::CallExpression(_),
                ),
            ) if is_pascal(&id.name) => Some(id.name.to_string()),
            _ => None,
        };
        let pushed = comp.is_some();
        self.components.extend(comp);
        walk::walk_variable_declarator(self, it);
        if pushed {
            self.components.pop();
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let name = callee_name(&it.callee);
        if let Some(n) = &name {
            if let Some(prefix) = self.key_prefix_for(n) {
                if let Some(arg) = it.arguments.first().and_then(|a| a.as_expression()) {
                    self.record_key_arg(arg, &prefix, it.span.start);
                }
            } else if n == "defineMessages"
                && let Some(Expression::ObjectExpression(o)) =
                    it.arguments.first().and_then(|a| a.as_expression())
            {
                for p in &o.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = p
                        && let Some(id) = obj_prop(&p.value, "id")
                    {
                        self.record_key_arg(id, "", p.span.start);
                    }
                }
            }
        }
        let toast = name.is_some_and(|n| n == "toast" || n.starts_with("toast."));
        if toast {
            self.frames.push(Frame::Hard(Some("toast")));
        }
        walk::walk_call_expression(self, it);
        if toast {
            self.frames.pop();
        }
    }

    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        let tag = elem_name(&it.opening_element.name);
        let skip = skips_text(&tag, &it.opening_element);
        self.frames.push(Frame::Element(tag_role(&tag)));
        self.skip_text += skip as usize;
        walk::walk_jsx_element(self, it);
        self.skip_text -= skip as usize;
        self.frames.pop();
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        let key_attr = match elem_name(&it.name).as_str() {
            "Trans" => "i18nKey",
            "FormattedMessage" => "id",
            _ => "",
        };
        for attr in &it.attributes {
            if let JSXAttributeItem::Attribute(a) = attr
                && !key_attr.is_empty()
                && matches!(&a.name, JSXAttributeName::Identifier(id) if id.name == key_attr)
                && let Some((k, _)) = attr_str(a)
            {
                self.add_usage(strip_ns(&k).to_string(), a.span.start);
            }
        }
        walk::walk_jsx_opening_element(self, it);
    }

    fn visit_jsx_attribute(&mut self, it: &JSXAttribute<'a>) {
        let role = match &it.name {
            JSXAttributeName::Identifier(id) => attr_role(&id.name),
            JSXAttributeName::NamespacedName(_) => None,
        };
        if let Some(r) = role
            && let Some((text, off)) = attr_str(it)
        {
            self.hardcoded(&text, r, off);
        }
        self.frames.push(Frame::Hard(role));
        walk::walk_jsx_attribute(self, it);
        self.frames.pop();
    }

    fn visit_jsx_text(&mut self, it: &JSXText<'a>) {
        let raw = &self.src[it.span.start as usize..it.span.end as usize];
        let lead = raw.len() - raw.trim_start().len();
        let role = self.role().unwrap_or("text");
        self.hardcoded(raw, role, it.span.start + lead as u32);
    }

    fn visit_jsx_expression_container(&mut self, it: &JSXExpressionContainer<'a>) {
        if matches!(self.frames.last(), Some(Frame::Element(_)))
            && let Some(e) = it.expression.as_expression()
            && let Some(s) = str_of(e)
        {
            let role = self.role().unwrap_or("text");
            self.hardcoded(&s, role, e.span().start);
        }
        walk::walk_jsx_expression_container(self, it);
    }
}

pub(crate) fn callee_name(e: &Expression) -> Option<String> {
    match e.get_inner_expression() {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::ThisExpression(_) => Some("this".into()),
        Expression::StaticMemberExpression(m) => {
            Some(format!("{}.{}", callee_name(&m.object)?, m.property.name))
        }
        _ => None,
    }
}

fn str_of(e: &Expression) -> Option<String> {
    match e.get_inner_expression() {
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        Expression::TemplateLiteral(t) => t.single_quasi().map(|q| q.to_string()),
        _ => None,
    }
}

fn obj_prop<'b, 'a>(e: &'b Expression<'a>, name: &str) -> Option<&'b Expression<'a>> {
    let Expression::ObjectExpression(o) = e.get_inner_expression() else {
        return None;
    };
    o.properties.iter().find_map(|p| match p {
        ObjectPropertyKind::ObjectProperty(p) if p.key.static_name().as_deref() == Some(name) => {
            Some(&p.value)
        }
        _ => None,
    })
}

/// `useTranslations("ns")`, `await getTranslations({namespace})`, `useTranslation(ns, {keyPrefix})`, `useI18n()`.
fn translator_prefix(init: &Expression) -> Option<String> {
    let mut e = init.get_inner_expression();
    if let Expression::AwaitExpression(a) = e {
        e = a.argument.get_inner_expression();
    }
    let Expression::CallExpression(call) = e else {
        return None;
    };
    let arg = |i: usize| call.arguments.get(i).and_then(|a| a.as_expression());
    match callee_name(&call.callee)?.as_str() {
        "useTranslations" | "getTranslations" => Some(
            arg(0)
                .and_then(|a| str_of(a).or_else(|| obj_prop(a, "namespace").and_then(str_of)))
                .unwrap_or_default(),
        ),
        // i18next namespaces select a file, not a key prefix (see crate docs).
        "useTranslation" => Some(
            arg(1)
                .and_then(|a| obj_prop(a, "keyPrefix"))
                .and_then(str_of)
                .unwrap_or_default(),
        ),
        "useI18n" => Some(String::new()),
        _ => None,
    }
}

fn attr_str(a: &JSXAttribute) -> Option<(String, u32)> {
    match a.value.as_ref()? {
        JSXAttributeValue::StringLiteral(s) => Some((s.value.to_string(), s.span.start)),
        JSXAttributeValue::ExpressionContainer(c) => {
            let e = c.expression.as_expression()?;
            Some((str_of(e)?, e.span().start))
        }
        _ => None,
    }
}

pub(crate) fn elem_name(n: &JSXElementName) -> String {
    match n {
        JSXElementName::Identifier(id) => id.name.to_string(),
        JSXElementName::IdentifierReference(r) => r.name.to_string(),
        JSXElementName::MemberExpression(m) => m.property.name.to_string(),
        JSXElementName::NamespacedName(n) => n.name.name.to_string(),
        JSXElementName::ThisExpression(_) => String::new(),
    }
}

pub(crate) fn tag_role(tag: &str) -> Option<&'static str> {
    let t = tag.to_ascii_lowercase();
    let b = t.as_bytes();
    Some(match t.as_str() {
        "a" => "link",
        _ if b.len() == 2 && b[0] == b'h' && (b'1'..=b'6').contains(&b[1]) => "heading",
        "legend" => "label",
        _ if t.ends_with("button") => "button",
        _ if t.ends_with("title") || t.ends_with("heading") => "heading",
        _ if t.ends_with("label") => "label",
        _ if t.ends_with("link") => "link",
        _ => return None,
    })
}

pub(crate) fn attr_role(name: &str) -> Option<&'static str> {
    Some(match name {
        "placeholder" => "placeholder",
        "title" => "title",
        "label" => "label",
        "alt" | "aria-label" | "aria-description" => "aria",
        _ => return None,
    })
}

fn is_pascal(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// i18next `ns:key` -> `key`. glot catalogs are one namespace-less tree per locale.
pub(crate) fn strip_ns(key: &str) -> &str {
    match key.split_once(':') {
        Some((ns, rest))
            if !ns.is_empty()
                && ns
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            rest
        }
        _ => key,
    }
}

/// Heuristic: user-visible copy vs. code-ish strings (class names, urls, ids, paths...).
pub(crate) fn looks_like_text(s: &str) -> bool {
    if s.chars().count() < 2 || !s.chars().any(char::is_alphabetic) {
        return false;
    }
    if s.contains(char::is_whitespace) {
        return !s.contains("://") || s.split_whitespace().count() > 2;
    }
    let lower_then_upper = s
        .as_bytes()
        .windows(2)
        .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
    let codey = s.contains([
        '_', '.', '/', '\\', '#', '{', '}', '<', '>', '=', ':', '@', '[', '(', ';',
    ]) || (s.contains('-') && !s.contains(|c: char| c.is_uppercase()));
    !(lower_then_upper || codey)
}

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "to", "of", "and", "or", "for", "in", "on", "at", "is", "are", "be", "your",
    "my", "with", "this", "that", "it",
];

pub(crate) fn suggest_key(path: &str, text: &str, role: &str) -> String {
    let prefix = path_prefix(path);
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| fold_ascii(&w.to_lowercase()))
        .filter(|w| !w.is_empty())
        .collect();
    let significant: Vec<&String> = words
        .iter()
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .collect();
    let picked = if significant.is_empty() {
        words.iter().collect()
    } else {
        significant
    };
    let mut slug = picked
        .iter()
        .take(4)
        .map(|w| w.as_str())
        .collect::<Vec<_>>()
        .join("_");
    if slug.is_empty() {
        slug = format!("text_{:08x}", fnv(text) as u32);
    }
    let suffix = match role {
        "button" | "placeholder" | "label" | "aria" => role,
        "heading" => "title",
        "title" => "tooltip",
        _ => "",
    };
    if !suffix.is_empty() && !slug.ends_with(suffix) {
        slug = format!("{slug}_{suffix}");
    }
    format!(
        "{}.{slug}",
        if prefix.is_empty() { "common" } else { &prefix }
    )
}

/// `app/(app)/settings/billing/page.tsx` -> `settings.billing`, `components/Navbar.tsx` -> `navbar`.
pub(crate) fn path_prefix(path: &str) -> String {
    let path = path.rsplit_once('.').map_or(path, |(p, _)| p);
    let mut segs: Vec<&str> = path.split('/').collect();
    if let Some(last) = segs.last()
        && matches!(
            *last,
            "page"
                | "layout"
                | "index"
                | "route"
                | "template"
                | "loading"
                | "error"
                | "not-found"
                | "default"
        )
    {
        segs.pop();
    }
    segs.iter()
        .filter(|s| {
            !matches!(
                **s,
                "" | "." | ".." | "src" | "app" | "pages" | "components"
            )
        })
        .filter(|s| !s.starts_with(['(', '[', '@', '_']))
        .map(|s| snake(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn snake(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in fold_ascii(s).chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_string()
}

// ponytail: Latin-1 accents only; other scripts fall back to a hashed slug.
fn fold_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        out.push_str(match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
            'ç' => "c",
            'è' | 'é' | 'ê' | 'ë' => "e",
            'ì' | 'í' | 'î' | 'ï' => "i",
            'ñ' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => "o",
            'ù' | 'ú' | 'û' | 'ü' => "u",
            'ý' | 'ÿ' => "y",
            'ß' => "ss",
            'œ' => "oe",
            'æ' => "ae",
            _ => "",
        });
    }
    out
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}
