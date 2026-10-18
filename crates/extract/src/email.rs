//! Emails as localizable assets (`glot assets --emails`).
//!
//! - **React Email** (`emails/*.tsx`): the copy is converted by the regular `i18nize` (JSX), keys under
//!   `emails.<template>.*`; `export const subject = "…"` becomes a key too ([`i18nize_subject`]).
//!   [`render_jsx`] statically evaluates the template (React Email components, `t()`/`t.rich()` calls,
//!   `PreviewProps`) into a [`Node`] tree: HTML preview per locale, links, subject and preheader.
//! - **HTML / MJML** (`emails/*.html|*.mjml`): never rewritten; [`segments`] finds the copy (text,
//!   `alt`/`title`, `<title>`/`<mj-title>` = subject, `<mj-preview>`/`.preheader` = preheader) with
//!   stable keys, and [`localize`] substitutes translations span by span (markup and links untouched).
//!   `{{var}}` (Handlebars/Mustache) is `{var}` in the catalog, so the validator guards variables.
use std::collections::{BTreeMap, HashMap};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use crate::scan::{elem_name, snake};

pub const SUBJECT_MAX: usize = 60;
pub const PREHEADER_MAX: usize = 100;

/// Locales written right-to-left.
pub fn is_rtl(locale: &str) -> bool {
    matches!(
        locale.split(['-', '_']).next().unwrap_or(""),
        "ar" | "he" | "fa" | "ur" | "yi" | "ps"
    )
}

/// `emails/invoice-paid.tsx` -> `invoice_paid`.
pub fn template_name(path: &str) -> String {
    let stem = path.rsplit('/').next().unwrap_or(path);
    let stem = stem.split('.').next().unwrap_or(stem);
    snake(stem)
}

// ---------------------------------------------------------------- JSX evaluation

/// Statically evaluated JSX. `Msg` wraps what a `t()` call rendered (its key drives budgets).
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    El {
        tag: String,
        attrs: Vec<(String, String)>,
        /// `style={{…}}` as written (camelCase prop, raw value; numbers without unit).
        style: Vec<(String, String)>,
        children: Vec<Node>,
    },
    Text(String),
    Msg {
        key: String,
        children: Vec<Node>,
    },
}

impl Node {
    pub fn text(&self) -> String {
        match self {
            Node::Text(t) => t.clone(),
            Node::El { children, .. } | Node::Msg { children, .. } => {
                children.iter().map(Node::text).collect()
            }
        }
    }

    pub fn walk<'n>(&'n self, f: &mut impl FnMut(&'n Node)) {
        f(self);
        if let Node::El { children, .. } | Node::Msg { children, .. } = self {
            for c in children {
                c.walk(f);
            }
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        match self {
            Node::El { attrs, .. } => attrs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            Node::El { tag, .. } => Some(tag),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rendered {
    pub subject: Option<String>,
    /// Key of the subject when it is translated (`export const subject = (t) => t("…")`).
    pub subject_key: Option<String>,
    pub root: Node,
    /// Keys a `t()` call asked for that the lookup did not know (rendered as the key).
    pub missing: Vec<String>,
    /// Every key rendered (`t()` calls in text and attributes, subject), sorted.
    pub keys: Vec<String>,
}

impl Rendered {
    pub fn preheader(&self) -> Option<String> {
        let mut out = None;
        self.root.walk(&mut |n| {
            if out.is_none() && n.tag() == Some("Preview") {
                out = Some(n.text().trim().to_string());
            }
        });
        out
    }

    /// Every `href` in document order.
    pub fn links(&self) -> Vec<String> {
        let mut out = vec![];
        self.root
            .walk(&mut |n| out.extend(n.attr("href").map(str::to_string)));
        out
    }

    /// The `<Html>` element passes a `dir` prop (else RTL locales render left-to-right when sent).
    pub fn has_dir(&self) -> bool {
        let mut has = false;
        self.root.walk(&mut |n| {
            if matches!(n.tag(), Some("Html" | "html")) && n.attr("dir").is_some() {
                has = true;
            }
        });
        has
    }
}

fn source_type(path: &str) -> SourceType {
    SourceType::from_path(path)
        .unwrap_or_default()
        .with_jsx(true)
}

/// Evaluate a React Email (or OG image) template. `lookup(full_key)` returns the message for the
/// locale being rendered. Props come from `Component.PreviewProps = {…}` (literals only).
// ponytail: static evaluation only (no `.map()`, no runtime conditions: `a && b` renders `b`);
// templates needing real logic keep using react-email's own preview.
pub fn render_jsx(
    path: &str,
    src: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Rendered, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, src, source_type(path)).parse();
    if parsed.diagnostics.has_errors() {
        return Err(format!("{path}: parse error"));
    }
    let mut info = Info::default();
    info.visit_program(&parsed.program);
    let Some(root) = info.html.or(info.first) else {
        return Err(format!("{path}: no JSX found"));
    };
    let mut ev = Eval {
        lookup,
        ns: info.ns,
        vars: info
            .props
            .into_iter()
            .map(|(k, v)| (k, vec![Node::Text(v)]))
            .collect(),
        missing: vec![],
        used: vec![],
        root,
        out: None,
        subject: None,
        subject_key: None,
    };
    ev.visit_program(&parsed.program);
    let root = ev.out.take().ok_or("root JSX not evaluated")?;
    let mut keys = ev.used;
    keys.extend(ev.subject_key.clone());
    keys.sort();
    keys.dedup();
    Ok(Rendered {
        subject: ev.subject,
        subject_key: ev.subject_key,
        root,
        missing: ev.missing,
        keys,
    })
}

#[derive(Default)]
struct Info {
    ns: Option<String>,
    props: BTreeMap<String, String>,
    html: Option<(u32, u32)>,
    first: Option<(u32, u32)>,
}

impl<'a> Visit<'a> for Info {
    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        let span = (it.span.start, it.span.end);
        if self.first.is_none() {
            self.first = Some(span);
        }
        if self.html.is_none() && elem_name(&it.opening_element.name) == "Html" {
            self.html = Some(span);
        }
        walk::walk_jsx_element(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(id) = &it.callee
            && matches!(
                id.name.as_str(),
                "useTranslations" | "getTranslations" | "createTranslator"
            )
            && self.ns.is_none()
        {
            self.ns = match it.arguments.first().and_then(|a| a.as_expression()) {
                Some(Expression::StringLiteral(s)) => Some(s.value.to_string()),
                Some(Expression::ObjectExpression(o)) => {
                    o.properties.iter().find_map(|p| match p {
                        ObjectPropertyKind::ObjectProperty(p)
                            if p.key.static_name().as_deref() == Some("namespace") =>
                        {
                            match &p.value {
                                Expression::StringLiteral(s) => Some(s.value.to_string()),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                }
                _ => None,
            };
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if let AssignmentTarget::StaticMemberExpression(m) = &it.left
            && m.property.name == "PreviewProps"
        {
            let mut e = &it.right;
            while let Expression::TSSatisfiesExpression(s) = e {
                e = &s.expression;
            }
            while let Expression::TSAsExpression(s) = e {
                e = &s.expression;
            }
            if let Expression::ObjectExpression(o) = e {
                for p in &o.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = p
                        && let Some(k) = p.key.static_name()
                        && let Some(v) = literal(&p.value)
                    {
                        self.props.insert(k.to_string(), v);
                    }
                }
            }
        }
        walk::walk_assignment_expression(self, it);
    }
}

fn literal(e: &Expression) -> Option<String> {
    match e {
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        Expression::NumericLiteral(n) => Some(num(n.value)),
        Expression::BooleanLiteral(b) => Some(b.value.to_string()),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
            t.quasis[0].value.cooked.map(|c| c.to_string())
        }
        _ => None,
    }
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        v.to_string()
    }
}

struct Eval<'l> {
    lookup: &'l dyn Fn(&str) -> Option<String>,
    ns: Option<String>,
    /// Props and `chunks` of rich-text tag functions.
    vars: HashMap<String, Vec<Node>>,
    missing: Vec<String>,
    used: Vec<String>,
    root: (u32, u32),
    out: Option<Node>,
    subject: Option<String>,
    subject_key: Option<String>,
}

impl<'a> Visit<'a> for Eval<'_> {
    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        if self.out.is_none() && (it.span.start, it.span.end) == self.root {
            self.out = Some(self.element(it));
        } else {
            walk::walk_jsx_element(self, it);
        }
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init)
            && id.name == "subject"
        {
            let nodes = self.children_of(init);
            self.subject_key = nodes.iter().find_map(|n| match n {
                Node::Msg { key, .. } => Some(key.clone()),
                _ => None,
            });
            self.subject = Some(nodes.iter().map(Node::text).collect());
        }
        walk::walk_variable_declarator(self, it);
    }
}

impl Eval<'_> {
    fn element(&mut self, el: &JSXElement) -> Node {
        let tag = elem_name(&el.opening_element.name);
        let (mut attrs, mut style) = (vec![], vec![]);
        for a in &el.opening_element.attributes {
            let JSXAttributeItem::Attribute(a) = a else {
                continue;
            };
            let name = match &a.name {
                JSXAttributeName::Identifier(id) => id.name.to_string(),
                JSXAttributeName::NamespacedName(n) => {
                    format!("{}:{}", n.namespace.name, n.name.name)
                }
            };
            match &a.value {
                None => attrs.push((name, String::new())),
                Some(JSXAttributeValue::StringLiteral(s)) => {
                    attrs.push((name, s.value.to_string()))
                }
                Some(JSXAttributeValue::ExpressionContainer(c)) => {
                    let Some(e) = c.expression.as_expression() else {
                        continue;
                    };
                    if name == "style"
                        && let Expression::ObjectExpression(o) = e
                    {
                        for p in &o.properties {
                            if let ObjectPropertyKind::ObjectProperty(p) = p
                                && let Some(k) = p.key.static_name()
                                && let Some(v) = self.text_of(&p.value)
                            {
                                style.push((k.to_string(), v));
                            }
                        }
                    } else if let Some(v) = self.text_of(e) {
                        attrs.push((name, v));
                    }
                }
                Some(_) => {}
            }
        }
        let children = self.jsx_children(&el.children);
        Node::El {
            tag,
            attrs,
            style,
            children,
        }
    }

    fn jsx_children(&mut self, children: &[JSXChild]) -> Vec<Node> {
        let mut out = vec![];
        for c in children {
            match c {
                JSXChild::Text(t) => {
                    let s = jsx_text(&t.value);
                    if !s.is_empty() {
                        out.push(Node::Text(decode(&s)));
                    }
                }
                JSXChild::ExpressionContainer(c) => {
                    if let Some(e) = c.expression.as_expression() {
                        out.extend(self.children_of(e));
                    }
                }
                JSXChild::Element(e) => out.push(self.element(e)),
                JSXChild::Fragment(f) => out.extend(self.jsx_children(&f.children)),
                JSXChild::Spread(_) => {}
            }
        }
        out
    }

    fn text_of(&mut self, e: &Expression) -> Option<String> {
        let nodes = self.children_of(e);
        (!nodes.is_empty()).then(|| nodes.iter().map(Node::text).collect())
    }

    fn children_of(&mut self, e: &Expression) -> Vec<Node> {
        match e {
            Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::TemplateLiteral(_)
                if literal(e).is_some() =>
            {
                vec![Node::Text(literal(e).unwrap_or_default())]
            }
            Expression::TemplateLiteral(t) => {
                let mut s = String::new();
                for (i, q) in t.quasis.iter().enumerate() {
                    s.push_str(q.value.cooked.as_deref().unwrap_or(&q.value.raw));
                    if let Some(x) = t.expressions.get(i) {
                        s.push_str(&self.text_of(x).unwrap_or_default());
                    }
                }
                vec![Node::Text(s)]
            }
            Expression::Identifier(id) => self.var(&id.name),
            Expression::StaticMemberExpression(m) => self.var(&m.property.name),
            Expression::ParenthesizedExpression(p) => self.children_of(&p.expression),
            Expression::LogicalExpression(l) => self.children_of(&l.right),
            Expression::ConditionalExpression(c) => self.children_of(&c.consequent),
            Expression::JSXElement(el) => vec![self.element(el)],
            Expression::JSXFragment(f) => self.jsx_children(&f.children),
            Expression::ArrowFunctionExpression(a) => match a.get_expression() {
                Some(body) => self.children_of(body),
                None => vec![],
            },
            Expression::CallExpression(c) => self.t_call(c).unwrap_or_default(),
            _ => vec![],
        }
    }

    fn var(&self, name: &str) -> Vec<Node> {
        self.vars
            .get(name)
            .cloned()
            .unwrap_or_else(|| vec![Node::Text(format!("{{{name}}}"))])
    }

    /// `t("key", {name, link: (chunks) => <a>{chunks}</a>})`, `t.rich(…)`, `t.markup(…)`.
    fn t_call(&mut self, c: &CallExpression) -> Option<Vec<Node>> {
        let callee = match &c.callee {
            Expression::Identifier(id) => id.name.as_str(),
            Expression::StaticMemberExpression(m)
                if matches!(m.property.name.as_str(), "rich" | "markup") =>
            {
                match &m.object {
                    Expression::Identifier(id) => id.name.as_str(),
                    _ => return None,
                }
            }
            _ => return None,
        };
        if !matches!(callee, "t" | "tr") {
            return None;
        }
        let Some(Expression::StringLiteral(k)) =
            c.arguments.first().and_then(|a| a.as_expression())
        else {
            return None;
        };
        let key = k.value.to_string();
        let scoped = self.ns.as_ref().map(|ns| format!("{ns}.{key}"));
        let full = match scoped {
            Some(s) if (self.lookup)(&s).is_some() || (self.lookup)(&key).is_none() => s,
            _ => key,
        };
        self.used.push(full.clone());
        let mut args: HashMap<String, Vec<Node>> = HashMap::new();
        let mut tags: Vec<(String, &ArrowFunctionExpression)> = vec![];
        if let Some(Expression::ObjectExpression(o)) =
            c.arguments.get(1).and_then(|a| a.as_expression())
        {
            for p in &o.properties {
                let ObjectPropertyKind::ObjectProperty(p) = p else {
                    continue;
                };
                let Some(name) = p.key.static_name() else {
                    continue;
                };
                match &p.value {
                    Expression::ArrowFunctionExpression(a) => tags.push((name.to_string(), a)),
                    v => {
                        let nodes = self.children_of(v);
                        args.insert(name.to_string(), nodes);
                    }
                }
            }
        }
        let msg = match (self.lookup)(&full) {
            Some(m) => m,
            None => {
                self.missing.push(full.clone());
                return Some(vec![Node::Msg {
                    key: full.clone(),
                    children: vec![Node::Text(full)],
                }]);
            }
        };
        let children = self.format(&msg, &args, &tags, None);
        Some(vec![Node::Msg {
            key: full,
            children,
        }])
    }

    /// Minimal ICU: `{arg}`, `{n, plural|select|selectordinal, …}` (`#`), `<tag>…</tag>`, `'{'` escapes.
    fn format(
        &mut self,
        msg: &str,
        args: &HashMap<String, Vec<Node>>,
        tags: &[(String, &ArrowFunctionExpression)],
        hash: Option<&str>,
    ) -> Vec<Node> {
        let mut out: Vec<Node> = vec![];
        let mut text = String::new();
        let flush = |text: &mut String, out: &mut Vec<Node>| {
            if !text.is_empty() {
                out.push(Node::Text(std::mem::take(text)));
            }
        };
        let b = msg.as_bytes();
        let mut i = 0;
        while i < msg.len() {
            let rest = &msg[i..];
            if rest.starts_with("''") {
                text.push('\'');
                i += 2;
            } else if rest.starts_with("'{") || rest.starts_with("'}") || rest.starts_with("'<") {
                let end = rest[1..].find('\'').map_or(msg.len(), |j| i + 1 + j);
                text.push_str(&msg[i + 1..end]);
                i = (end + 1).min(msg.len());
            } else if b[i] == b'#' && hash.is_some() {
                text.push_str(hash.unwrap_or_default());
                i += 1;
            } else if b[i] == b'{' {
                let Some(close) = matching(msg, i) else {
                    text.push_str(rest);
                    break;
                };
                flush(&mut text, &mut out);
                out.extend(self.argument(&msg[i + 1..close], args, tags));
                i = close + 1;
            } else if b[i] == b'<'
                && let Some(gt) = rest.find('>')
                && let name = &rest[1..gt]
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && let Some(end) = rest.find(&format!("</{name}>"))
            {
                flush(&mut text, &mut out);
                let inner = self.format(&rest[gt + 1..end], args, tags, hash);
                out.extend(self.tag(name, inner, tags));
                i += end + name.len() + 3;
            } else {
                let c = rest.chars().next().unwrap_or(' ');
                text.push(c);
                i += c.len_utf8();
            }
        }
        flush(&mut text, &mut out);
        out
    }

    fn argument(
        &mut self,
        inner: &str,
        args: &HashMap<String, Vec<Node>>,
        tags: &[(String, &ArrowFunctionExpression)],
    ) -> Vec<Node> {
        let mut parts = inner.splitn(3, ',');
        let name = parts.next().unwrap_or("").trim();
        let value = args.get(name).cloned().unwrap_or_else(|| self.var(name));
        let (Some(kind), Some(opts)) = (parts.next(), parts.next()) else {
            return value;
        };
        let v: String = value.iter().map(Node::text).collect();
        let kind = kind.trim();
        if !matches!(kind, "plural" | "select" | "selectordinal") {
            return value;
        }
        // `selector {content}` pairs.
        let mut pairs: Vec<(String, String)> = vec![];
        let mut j = 0;
        while let Some(o) = opts[j..].find('{').map(|o| j + o) {
            let Some(e) = matching(opts, o) else { break };
            pairs.push((opts[j..o].trim().to_string(), opts[o + 1..e].to_string()));
            j = e + 1;
        }
        let n = v.parse::<f64>().ok();
        let pick = pairs
            .iter()
            .find(|(s, _)| {
                s.strip_prefix('=')
                    .is_some_and(|x| x.parse::<f64>().ok() == n)
            })
            .or_else(|| match kind {
                "select" => pairs.iter().find(|(s, _)| *s == v),
                // ponytail: English-like plural rule for previews (one / other).
                _ => pairs.iter().find(|(s, _)| n == Some(1.0) && s == "one"),
            })
            .or_else(|| pairs.iter().find(|(s, _)| s == "other"))
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        let hash = (kind != "select").then_some(v.as_str());
        self.format(&pick, args, tags, hash)
    }

    fn tag(
        &mut self,
        name: &str,
        inner: Vec<Node>,
        tags: &[(String, &ArrowFunctionExpression)],
    ) -> Vec<Node> {
        let Some((_, f)) = tags.iter().find(|(n, _)| n == name) else {
            return vec![Node::El {
                tag: name.into(),
                attrs: vec![],
                style: vec![],
                children: inner,
            }];
        };
        let param = f.params.items.first().and_then(|p| match &p.pattern {
            BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
            _ => None,
        });
        let saved = param
            .as_ref()
            .map(|p| (p.clone(), self.vars.insert(p.clone(), inner)));
        let out = f
            .get_expression()
            .map(|e| self.children_of(e))
            .unwrap_or_default();
        if let Some((p, old)) = saved {
            match old {
                Some(o) => self.vars.insert(p, o),
                None => self.vars.remove(&p),
            };
        }
        out
    }
}

/// Index of the `}` closing the `{` at `open`.
fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// JSX whitespace: lines trimmed (except the outer edges), blank lines dropped, joined by a space.
fn jsx_text(s: &str) -> String {
    let lines: Vec<&str> = s.split('\n').collect();
    let n = lines.len();
    let mut out: Vec<&str> = vec![];
    for (i, l) in lines.iter().enumerate() {
        let mut l = *l;
        if i > 0 {
            l = l.trim_start();
        }
        if i + 1 < n {
            l = l.trim_end();
        }
        if !l.is_empty() {
            out.push(l);
        }
    }
    if n > 1 {
        out.join(" ")
    } else {
        lines[0].to_string()
    }
}

/// Common entities only.
// ponytail: named entities beyond the usual six and `&nbsp;` stay as written.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&nbsp;", "\u{a0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn esc_attr(s: &str) -> String {
    esc(s).replace('"', "&quot;")
}

/// `fontSize` -> `font-size`.
pub fn kebab(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

const UNITLESS: &[&str] = &[
    "lineHeight",
    "fontWeight",
    "opacity",
    "zIndex",
    "flex",
    "flexGrow",
    "flexShrink",
    "order",
];

fn css(style: &[(String, String)]) -> String {
    style
        .iter()
        .map(|(k, v)| {
            let px = v.parse::<f64>().is_ok() && !UNITLESS.contains(&k.as_str()) && v != "0";
            format!("{}:{v}{}", kebab(k), if px { "px" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Preview HTML of a rendered React Email template for `lang` (RTL locales get `dir="rtl"`).
pub fn to_html(r: &Rendered, lang: &str) -> String {
    let mut out = String::from("<!DOCTYPE html>\n");
    html_node(&r.root, lang, r.subject.as_deref(), &mut out);
    out.push('\n');
    out
}

const VOID: &[&str] = &["img", "hr", "br", "meta", "link", "input"];

fn html_node(n: &Node, lang: &str, subject: Option<&str>, out: &mut String) {
    let (tag, attrs, style, children) = match n {
        Node::Text(t) => return out.push_str(&esc(t)),
        Node::Msg { children, .. } => {
            for c in children {
                html_node(c, lang, subject, out);
            }
            return;
        }
        Node::El {
            tag,
            attrs,
            style,
            children,
        } => (tag, attrs, style, children),
    };
    // React Email components -> what they render (inline styles approximated).
    let (html, extra): (&str, &str) = match tag.as_str() {
        "Html" => ("html", ""),
        "Head" => ("head", ""),
        "Body" => ("body", ""),
        "Container" => ("div", "max-width:37.5em;margin:0 auto"),
        "Section" | "Row" | "Column" => ("div", ""),
        "Text" => ("p", "font-size:14px;line-height:24px;margin:16px 0"),
        "Heading" => (n.attr("as").unwrap_or("h1"), ""),
        "Button" => ("a", "display:inline-block;text-decoration:none"),
        "Link" => ("a", ""),
        "Img" => ("img", "display:block;border:0"),
        "Hr" => ("hr", ""),
        "Preview" => (
            "div",
            "display:none;overflow:hidden;line-height:1px;opacity:0;max-height:0",
        ),
        "Font" | "Tailwind" | "Fragment" => {
            for c in children {
                html_node(c, lang, subject, out);
            }
            return;
        }
        t if t.starts_with(|c: char| c.is_ascii_uppercase()) => ("div", ""),
        t => (t, ""),
    };
    out.push('<');
    out.push_str(html);
    let rtl = is_rtl(lang);
    for (k, v) in attrs {
        let k = match k.as_str() {
            "className" => "class",
            "htmlFor" => "for",
            "as" | "key" | "ref" => continue,
            // The preview shows what a correctly wired template sends.
            "lang" | "dir" if html == "html" => continue,
            k => k,
        };
        out.push_str(&format!(" {k}=\"{}\"", esc_attr(v)));
    }
    if html == "html" {
        out.push_str(&format!(
            " lang=\"{}\" dir=\"{}\"",
            esc_attr(lang),
            if rtl { "rtl" } else { "ltr" }
        ));
    }
    let style = [extra.to_string(), css(style)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(";");
    if !style.is_empty() {
        out.push_str(&format!(" style=\"{}\"", esc_attr(&style)));
    }
    out.push('>');
    if VOID.contains(&html) {
        return;
    }
    if html == "head" {
        out.push_str("<meta charset=\"utf-8\">");
        if let Some(s) = subject {
            out.push_str(&format!("<title>{}</title>", esc(s)));
        }
    }
    for c in children {
        html_node(c, lang, subject, out);
    }
    out.push_str(&format!("</{html}>"));
}

/// `export const subject = "…"` -> `(t: (key: string) => string) => t("<key>")`: the sender resolves
/// it with its translator. Returns the new source and the subject text, `None` if absent or done.
pub fn i18nize_subject(path: &str, src: &str, key: &str) -> Option<(String, String)> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, src, source_type(path)).parse();
    if parsed.diagnostics.has_errors() {
        return None;
    }
    struct Find(Option<(u32, u32, String)>);
    impl<'a> Visit<'a> for Find {
        fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
            if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&it.id, &it.init)
                && id.name == "subject"
                && let Some(text) = literal(init)
            {
                let s = init.span();
                self.0 = Some((s.start, s.end, text));
            }
        }
    }
    let mut f = Find(None);
    f.visit_program(&parsed.program);
    let (s, e, text) = f.0?;
    let ts = source_type(path).is_typescript();
    let rep = format!(
        "({}) => t({key:?})",
        if ts {
            "t: (key: string) => string"
        } else {
            "t"
        }
    );
    Some((
        format!("{}{rep}{}", &src[..s as usize], &src[e as usize..]),
        text,
    ))
}

// ---------------------------------------------------------------- markup (HTML, MJML, SVG)

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    /// Offset of `<`.
    pub start: usize,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A run of copy in markup: element text (whitespace-trimmed span) or an attribute value.
#[derive(Debug, Clone)]
pub struct Seg {
    pub span: (usize, usize),
    /// Decoded text, whitespace collapsed, `{{var}}` -> `{var}`.
    pub text: String,
    pub attr: Option<String>,
    /// Open elements, outermost first (for an attribute: including its element).
    pub stack: Vec<Tag>,
}

const TEXT_ATTRS: &[&str] = &["alt", "title", "aria-label", "placeholder"];
const RAW: &[&str] = &["script", "style", "mj-style", "mj-raw"];
const VOID_MARKUP: &[&str] = &[
    "img", "br", "hr", "meta", "link", "input", "source", "col", "area", "base", "wbr",
];

/// Copy in an HTML / MJML / SVG document, filtered by `keep(stack, attr)`.
// ponytail: a tolerant tag scanner, not an HTML5 parser (no implied end tags beyond void elements);
// inline markup splits a sentence into several segments.
pub fn scan_markup(src: &str, keep: &dyn Fn(&[Tag], Option<&str>) -> bool) -> Vec<Seg> {
    let mut out = vec![];
    let mut stack: Vec<Tag> = vec![];
    let mut i = 0;
    let push_text = |s: usize, e: usize, stack: &[Tag], out: &mut Vec<Seg>| {
        let raw = &src[s..e];
        let t = raw.trim();
        if !has_words(t) || !keep(stack, None) {
            return;
        }
        let lead = raw.len() - raw.trim_start().len();
        out.push(Seg {
            span: (s + lead, s + lead + t.len()),
            text: to_icu(&decode(&t.split_whitespace().collect::<Vec<_>>().join(" "))),
            attr: None,
            stack: stack.to_vec(),
        });
    };
    while let Some(lt) = src[i..].find('<').map(|p| i + p) {
        push_text(i, lt, &stack, &mut out);
        let rest = &src[lt..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(src.len(), |e| lt + e + 3);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = rest.find("]]>").map_or(src.len(), |e| lt + e + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i = rest.find('>').map_or(src.len(), |e| lt + e + 1);
            continue;
        }
        if let Some(close) = rest.strip_prefix("</") {
            let name: String = close
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '>')
                .collect::<String>()
                .to_ascii_lowercase();
            if let Some(p) = stack.iter().rposition(|t| t.name == name) {
                stack.truncate(p);
            }
            i = rest.find('>').map_or(src.len(), |e| lt + e + 1);
            continue;
        }
        // Open tag: name, then attributes (quoted values may contain `>`).
        let b = src.as_bytes();
        let mut j = lt + 1;
        while j < src.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
            j += 1;
        }
        let name = src[lt + 1..j].to_ascii_lowercase();
        if name.is_empty() {
            i = lt + 1;
            continue;
        }
        let mut tag = Tag {
            name: name.clone(),
            attrs: vec![],
            start: lt,
        };
        let mut attr_spans = vec![];
        let mut self_closing = false;
        loop {
            while j < src.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= src.len() {
                break;
            }
            if b[j] == b'>' {
                j += 1;
                break;
            }
            if b[j] == b'/' {
                self_closing = true;
                j += 1;
                continue;
            }
            let ns = j;
            while j < src.len()
                && !b[j].is_ascii_whitespace()
                && !matches!(b[j], b'=' | b'>' | b'/')
            {
                j += 1;
            }
            let an = src[ns..j].to_string();
            if an.is_empty() {
                j += 1;
                continue;
            }
            let mut value = (j, j);
            if j < src.len() && b[j] == b'=' {
                j += 1;
                if j < src.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    let vs = j + 1;
                    let ve = src[vs..].find(q as char).map_or(src.len(), |e| vs + e);
                    value = (vs, ve);
                    j = (ve + 1).min(src.len());
                } else {
                    let vs = j;
                    while j < src.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    value = (vs, j);
                }
            }
            tag.attrs.push((an.clone(), decode(&src[value.0..value.1])));
            attr_spans.push((an, value));
        }
        stack.push(tag);
        for (an, (vs, ve)) in attr_spans {
            let v = &src[vs..ve];
            if TEXT_ATTRS.contains(&an.to_ascii_lowercase().as_str())
                && has_words(v)
                && keep(&stack, Some(&an))
            {
                out.push(Seg {
                    span: (vs, ve),
                    text: to_icu(&decode(v.trim())),
                    attr: Some(an),
                    stack: stack.clone(),
                });
            }
        }
        if self_closing || VOID_MARKUP.contains(&name.as_str()) {
            stack.pop();
        } else if RAW.contains(&name.as_str()) {
            stack.pop();
            let end = format!("</{name}");
            j = src[j..]
                .to_ascii_lowercase()
                .find(&end)
                .map_or(src.len(), |e| j + e);
        }
        i = j;
    }
    push_text(i, src.len(), &stack, &mut out);
    out
}

/// Letters outside `{{…}}` (a lone variable or punctuation is not copy).
fn has_words(s: &str) -> bool {
    let mut depth = 0;
    s.chars().any(|c| {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        depth == 0 && c.is_alphabetic()
    })
}

/// `{{ name }}` -> `{name}`; other braces escaped for ICU.
fn to_icu(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(o) = rest.find("{{") {
        let Some(c) = rest[o..].find("}}").map(|c| o + c) else {
            break;
        };
        let name = rest[o + 2..c].trim();
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            break;
        }
        out.push_str(&esc_icu(&rest[..o]));
        out.push_str(&format!("{{{}}}", name.replace('.', "_")));
        rest = &rest[c + 2..];
    }
    out.push_str(&esc_icu(rest));
    out
}

fn esc_icu(s: &str) -> String {
    s.replace('{', "'{'").replace('}', "'}'")
}

/// Inverse of [`to_icu`] for writing a translation back into markup.
// ponytail: `{a_b}` comes back as `{{a_b}}` even if the source said `{{a.b}}`; use flat var names.
pub fn from_icu(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(o) = rest.find(['{', '\'']) {
        out.push_str(&rest[..o]);
        let r = &rest[o..];
        if let Some(e) = r.strip_prefix("'{'") {
            out.push('{');
            rest = e;
        } else if let Some(e) = r.strip_prefix("'}'") {
            out.push('}');
            rest = e;
        } else if r.starts_with('{')
            && let Some(c) = r.find('}')
        {
            out.push_str(&format!("{{{{{}}}}}", r[1..c].trim()));
            rest = &r[c + 1..];
        } else {
            out.push_str(&r[..1]);
            rest = &r[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Keys for segments: `<prefix>.<id of the nearest element with an id | slug of the text>`, unique.
/// Same text => same key. `special(seg)` names reserved leaves (`subject`, `preheader`).
pub fn name_segments(
    prefix: &str,
    segs: &[Seg],
    special: &dyn Fn(&Seg) -> Option<&'static str>,
) -> Vec<String> {
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    segs.iter()
        .enumerate()
        .map(|(i, s)| {
            let base = special(s)
                .map(str::to_string)
                .or_else(|| {
                    let id = s.stack.last().and_then(|t| t.attr("id")).map(snake);
                    let id = id.filter(|x| !x.is_empty())?;
                    Some(match &s.attr {
                        Some(a) => format!("{id}_{}", snake(a)),
                        None => id,
                    })
                })
                .unwrap_or_else(|| {
                    let words: Vec<&str> = s.text.split_whitespace().take(4).collect();
                    let slug = snake(&words.join(" "));
                    let slug = if slug.is_empty() {
                        format!("text_{i}")
                    } else {
                        slug
                    };
                    match &s.attr {
                        Some(a) => format!("{}_{slug}", snake(a)),
                        None => slug,
                    }
                });
            let mut key = format!("{prefix}.{base}");
            let mut n = 2;
            while taken.get(&key).is_some_and(|t| *t != s.text) {
                key = format!("{prefix}.{base}_{n}");
                n += 1;
            }
            taken.insert(key.clone(), s.text.clone());
            key
        })
        .collect()
}

/// Email segments (HTML or MJML) with their keys under `emails.<template>`.
pub fn segments(path: &str, src: &str) -> Vec<(String, Seg)> {
    let in_ = |st: &[Tag], n: &str| st.iter().any(|t| t.name == n);
    let keep = |st: &[Tag], attr: Option<&str>| {
        if attr.is_some() {
            return !in_(st, "head") && !in_(st, "mj-head");
        }
        let top = st.last().map(|t| t.name.as_str()).unwrap_or("");
        match () {
            _ if matches!(top, "title" | "mj-title" | "mj-preview") => true,
            _ if in_(st, "head") || in_(st, "mj-head") => false,
            _ => true,
        }
    };
    let segs = scan_markup(src, &keep);
    let special = |s: &Seg| -> Option<&'static str> {
        if s.attr.is_some() {
            return None;
        }
        let top = s.stack.last()?;
        if matches!(top.name.as_str(), "title" | "mj-title") {
            return Some("subject");
        }
        let pre = |t: &Tag| {
            t.name == "mj-preview"
                || [t.attr("class"), t.attr("id")]
                    .into_iter()
                    .flatten()
                    .any(|v| v.contains("preheader") || v.contains("preview"))
        };
        s.stack.iter().any(pre).then_some("preheader")
    };
    let prefix = format!("emails.{}", template_name(path));
    let keys = name_segments(&prefix, &segs, &special);
    keys.into_iter().zip(segs).collect()
}

/// Replace each segment span by its translation (escaped for markup).
pub fn localize(src: &str, repl: &[((usize, usize), bool, String)]) -> String {
    let mut sorted: Vec<&((usize, usize), bool, String)> = repl.iter().collect();
    sorted.sort_by_key(|(s, _, _)| s.0);
    let mut out = String::with_capacity(src.len() + 64);
    let mut pos = 0;
    for ((s, e), attr, text) in sorted {
        if *s < pos {
            continue;
        }
        out.push_str(&src[pos..*s]);
        let t = from_icu(text);
        out.push_str(&if *attr { esc_attr(&t) } else { esc(&t) });
        pos = *e;
    }
    out.push_str(&src[pos..]);
    out
}

/// Set `lang`/`dir` on the root `<html>` (or `<mjml>`) tag.
pub fn set_lang_dir(src: &str, lang: &str) -> String {
    let lower = src.to_ascii_lowercase();
    let Some(p) = ["<html", "<mjml"]
        .iter()
        .find_map(|t| lower.find(t).map(|p| p + t.len()))
    else {
        return src.to_string();
    };
    let end = lower[p..].find('>').map_or(p, |e| p + e);
    // Drop existing lang/dir, then add ours.
    let cleaned = strip_attr(&strip_attr(&src[p..end], "lang"), "dir");
    let dir = if is_rtl(lang) { "rtl" } else { "ltr" };
    format!(
        "{}{cleaned} lang=\"{lang}\" dir=\"{dir}\"{}",
        &src[..p],
        &src[end..]
    )
}

fn strip_attr(attrs: &str, name: &str) -> String {
    let lower = attrs.to_ascii_lowercase();
    let pat = format!(" {name}=");
    let Some(s) = lower.find(&pat) else {
        return attrs.to_string();
    };
    let v = s + pat.len();
    let end = match attrs.as_bytes().get(v) {
        Some(q @ (b'"' | b'\'')) => attrs[v + 1..]
            .find(*q as char)
            .map_or(attrs.len(), |e| v + 2 + e),
        _ => attrs[v..].find(' ').map_or(attrs.len(), |e| v + e),
    };
    format!("{}{}", &attrs[..s], &attrs[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const WELCOME: &str = r#"import { Html, Head, Preview, Body, Text, Button, Link, Img } from "@react-email/components";
import { useTranslations } from "next-intl";

export const subject = (t: (key: string) => string) => t("emails.welcome.subject");

export default function Welcome({ name, url }: { name: string; url: string }) {
  const t = useTranslations("emails.welcome");
  return (
    <Html>
      <Head />
      <Preview>{t("preview")}</Preview>
      <Body style={{ fontFamily: "Helvetica", padding: 24 }}>
        <Img src="https://x.test/logo.png" alt={t("logo_alt")} />
        <Text>{t("hello", { name })}</Text>
        <Button href={url}>{t("cta")}</Button>
        <Text>{t.rich("help", { link: (chunks) => <Link href="https://x.test/docs">{chunks}</Link> })}</Text>
        <Text>{t("seats", { count: 3 })}</Text>
      </Body>
    </Html>
  );
}

Welcome.PreviewProps = { name: "Ada", url: "https://x.test/w" } satisfies object;
"#;

    fn cat() -> BTreeMap<String, String> {
        [
            ("emails.welcome.subject", "Welcome!"),
            ("emails.welcome.preview", "Your workspace is ready"),
            ("emails.welcome.logo_alt", "Logo"),
            ("emails.welcome.hello", "Hi {name},"),
            ("emails.welcome.cta", "Open"),
            ("emails.welcome.help", "Read the <link>guide</link>."),
            (
                "emails.welcome.seats",
                "{count, plural, one {# seat} other {# seats}}",
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn renders_react_email_with_translations() {
        let c = cat();
        let r = render_jsx("emails/welcome.tsx", WELCOME, &|k| c.get(k).cloned()).unwrap();
        assert_eq!(r.subject.as_deref(), Some("Welcome!"));
        assert_eq!(r.subject_key.as_deref(), Some("emails.welcome.subject"));
        assert_eq!(r.preheader().as_deref(), Some("Your workspace is ready"));
        assert_eq!(r.links(), vec!["https://x.test/w", "https://x.test/docs"]);
        assert!(r.missing.is_empty(), "{:?}", r.missing);
        assert!(!r.has_dir());
        let html = to_html(&r, "ar");
        assert!(html.contains(r#"<html lang="ar" dir="rtl">"#), "{html}");
        assert!(html.contains("<title>Welcome!</title>"));
        assert!(
            html.contains("<p style=\"font-size:14px;line-height:24px;margin:16px 0\">Hi Ada,</p>"),
            "{html}"
        );
        assert!(
            html.contains(r#"Read the <a href="https://x.test/docs">guide</a>."#),
            "{html}"
        );
        assert!(html.contains("3 seats"));
        assert!(html.contains(r#"alt="Logo""#));
        assert!(html.contains("font-family:Helvetica;padding:24px"));
        assert_eq!(r.keys.len(), 7, "{:?}", r.keys);
    }

    #[test]
    fn subject_becomes_a_key_once() {
        let src = "export const subject = \"Welcome to Acme\";\nexport default function W() { return <p>x</p>; }\n";
        let (out, text) = i18nize_subject("emails/w.tsx", src, "emails.w.subject").unwrap();
        assert_eq!(text, "Welcome to Acme");
        assert!(out.contains(
            r#"export const subject = (t: (key: string) => string) => t("emails.w.subject");"#
        ));
        assert!(i18nize_subject("emails/w.tsx", &out, "emails.w.subject").is_none());
    }

    #[test]
    fn html_email_segments_and_localize() {
        let src = r#"<!doctype html><html><head><title>Your invoice is paid</title><style>p{color:red}</style></head>
<body><div class="preheader">Thanks {{ name }}, here is your receipt.</div>
<p id="intro">We received <b>{{amount}}</b> for your plan.</p>
<a href="https://x.test/i/{{id}}">View invoice</a><img src="a.png" alt="Acme logo"></body></html>"#;
        let segs = segments("emails/invoice.html", src);
        let keys: Vec<(&str, &str)> = segs
            .iter()
            .map(|(k, s)| (k.as_str(), s.text.as_str()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("emails.invoice.subject", "Your invoice is paid"),
                (
                    "emails.invoice.preheader",
                    "Thanks {name}, here is your receipt."
                ),
                ("emails.invoice.intro", "We received"),
                ("emails.invoice.intro_2", "for your plan."),
                ("emails.invoice.view_invoice", "View invoice"),
                ("emails.invoice.alt_acme_logo", "Acme logo"),
            ]
        );
        let repl: Vec<_> = segs
            .iter()
            .map(|(_, s)| (s.span, s.attr.is_some(), format!("<{}> & {}", s.text, "x")))
            .collect();
        let out = set_lang_dir(&localize(src, &repl), "ar");
        assert!(
            out.contains("&lt;Thanks {{name}}, here is your receipt.&gt; &amp; x"),
            "{out}"
        );
        assert!(out.contains(r#"href="https://x.test/i/{{id}}""#));
        assert!(out.contains(r#"<html lang="ar" dir="rtl">"#), "{out}");
        assert!(out.contains("<style>p{color:red}</style>"));
    }

    #[test]
    fn mjml_preview_and_title() {
        let src = "<mjml><mj-head><mj-title>Reset your password</mj-title><mj-preview>Link valid 1 hour</mj-preview><mj-attributes><mj-all font-family=\"Arial\" /></mj-attributes></mj-head><mj-body><mj-section><mj-column><mj-text>Hello {{name}}</mj-text><mj-button href=\"{{url}}\">Reset</mj-button></mj-column></mj-section></mj-body></mjml>";
        let segs = segments("emails/reset.mjml", src);
        let keys: Vec<&str> = segs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "emails.reset.subject",
                "emails.reset.preheader",
                "emails.reset.hello_name",
                "emails.reset.reset"
            ]
        );
        assert_eq!(from_icu("Bonjour {name} '{'x'}'"), "Bonjour {{name}} {x}");
    }
}
