//! Bundle localization for glot Edge: text hard-coded in client JavaScript (a React client component's
//! `<p>Cancel anytime</p>` compiles to `jsx("p", {children: "Cancel anytime"})`) is rendered by the browser from
//! the bundle, not from the HTML. Once the Edge translates the HTML, the client renders the source text over it:
//! React error #418 + client re-render. [`localize`] rewrites the string literals of a served chunk that are
//! exactly (after whitespace folding) the source of a translated segment, so the client renders the same text.
//!
//! A real parser (oxc), so strings are never confused with comments, regexes or template parts. Never rewritten:
//! object/class keys, `import`/`export`/`require()`/`import()` sources, directives (`"use client"`), operands of
//! `==`/`!=`/`===`/`!==`/`in`, `case` labels, computed member names (`x["Save"]`), tagged templates, templates
//! with expressions, direct arguments of calls and `new` (`track("Add to cart")`, `t("…")`; except
//! `createElement`/`h` children and `createTextNode`), values of identifier-like keys (`id`, `type`, `variant`…,
//! same list as the payload rewriter). The lookup itself only knows segments seen in rendered pages.
// ponytail: positional heuristic; a string copied to a variable then used as a map key (`m[label]`) is still
// rewritten — the lookup being restricted to rendered segments is what keeps that rare.
use glot_core::edge::hydration::id_key;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_syntax::operator::BinaryOperator;

/// `lookup(normalized source) -> translation`.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The chunk with matching literals translated and how many were, or `None` when nothing changed or the chunk
/// does not parse (served as is: fail-open).
pub fn localize(js: &str, lookup: Lookup) -> Option<(String, usize)> {
    let alloc = Allocator::default();
    // Bundles are modules (Vite/Rollup ESM) or scripts (webpack/Turbopack); a sloppy script may not parse as a
    // module (`with`, octal escapes), so retry as a script.
    let mut parsed = Parser::new(&alloc, js, SourceType::mjs()).parse();
    if parsed.diagnostics.has_errors() {
        parsed = Parser::new(&alloc, js, SourceType::cjs()).parse();
        if parsed.diagnostics.has_errors() {
            return None;
        }
    }
    let mut v = V {
        lookup,
        edits: vec![],
    };
    v.visit_program(&parsed.program);
    if v.edits.is_empty() {
        return None;
    }
    v.edits.sort_by_key(|e| e.0);
    let n = v.edits.len();
    let mut out = String::with_capacity(js.len() + n * 16);
    let mut at = 0;
    for (s, e, r) in v.edits {
        if s < at {
            continue; // nested (cannot happen for literals; defensive)
        }
        out.push_str(&js[at..s]);
        out.push_str(&r);
        at = e;
    }
    out.push_str(&js[at..]);
    Some((out, n))
}

struct V<'l> {
    lookup: Lookup<'l>,
    edits: Vec<(usize, usize, String)>,
}

/// JS string literal for `s` (JSON is valid JS; U+2028/2029 escaped for older engines).
fn quote(s: &str) -> String {
    serde_json::to_string(s)
        .unwrap_or_default()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

impl V<'_> {
    fn text(&mut self, span: oxc_span::Span, value: &str) {
        if !value.chars().any(char::is_alphabetic) {
            return;
        }
        let folded = glot_core::edge::tm::normalize(value);
        let Some(t) = (self.lookup)(&folded).filter(|t| *t != folded) else {
            return;
        };
        // keep the literal's own outer whitespace (JSX text often carries it); HTML whitespace only, like the
        // segmenter (NBSP is text)
        let ws = glot_core::edge::protect::is_html_ws;
        let lead = &value[..value.len() - value.trim_start_matches(ws).len()];
        let trail = &value[value.trim_end_matches(ws).len()..];
        self.edits.push((
            span.start as usize,
            span.end as usize,
            quote(&format!("{lead}{t}{trail}")),
        ));
    }

    /// Visit an expression, but not when it is itself a string/template literal.
    fn non_literal(&mut self, e: &Expression) {
        if !matches!(
            e,
            Expression::StringLiteral(_) | Expression::TemplateLiteral(_)
        ) {
            self.visit_expression(e);
        }
    }

    fn args(&mut self, callee: &Expression, args: &[Argument]) {
        // index of the first argument that is display text: createElement(type, props, ...children)
        let from = match callee_name(callee).as_deref() {
            Some("createElement" | "h") => 2,
            Some("createTextNode") => 0,
            _ => usize::MAX,
        };
        for (i, a) in args.iter().enumerate() {
            match a.as_expression() {
                Some(e) if i < from => self.non_literal(e),
                _ => self.visit_argument(a),
            }
        }
    }
}

fn callee_name(e: &Expression) -> Option<String> {
    match e.without_parentheses() {
        Expression::Identifier(i) => Some(i.name.to_string()),
        Expression::StaticMemberExpression(m) => Some(m.property.name.to_string()),
        Expression::SequenceExpression(s) => s.expressions.last().and_then(callee_name),
        _ => None,
    }
}

impl<'a> Visit<'a> for V<'_> {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.text(it.span, &it.value);
    }

    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        if it.expressions.is_empty()
            && let [q] = it.quasis.as_slice()
            && let Some(c) = &q.value.cooked
        {
            self.text(it.span, c);
        }
        walk::walk_template_literal(self, it);
    }

    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        self.visit_expression(&it.tag);
        for e in &it.quasi.expressions {
            self.visit_expression(e);
        }
    }

    fn visit_property_key(&mut self, _: &PropertyKey<'a>) {}

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if it.key.static_name().is_some_and(|k| id_key(&k)) {
            self.non_literal(&it.value);
        } else {
            self.visit_expression(&it.value);
        }
    }

    fn visit_binary_expression(&mut self, it: &BinaryExpression<'a>) {
        if matches!(
            it.operator,
            BinaryOperator::Equality
                | BinaryOperator::Inequality
                | BinaryOperator::StrictEquality
                | BinaryOperator::StrictInequality
                | BinaryOperator::In
        ) {
            self.non_literal(&it.left);
            self.non_literal(&it.right);
        } else {
            walk::walk_binary_expression(self, it);
        }
    }

    fn visit_switch_case(&mut self, it: &SwitchCase<'a>) {
        if let Some(t) = &it.test {
            self.non_literal(t);
        }
        self.visit_statements(&it.consequent);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        self.visit_expression(&it.object);
        self.non_literal(&it.expression);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.visit_expression(&it.callee);
        self.args(&it.callee, &it.arguments);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        self.visit_expression(&it.callee);
        self.args(&it.callee, &it.arguments);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.non_literal(&it.source);
    }

    fn visit_import_declaration(&mut self, _: &ImportDeclaration<'a>) {}

    fn visit_export_all_declaration(&mut self, _: &ExportAllDeclaration<'a>) {}

    // `export {a as "b"}`, `export {a} from "m"`: names and sources only
    fn visit_export_named_declaration(&mut self, _: &ExportNamedDeclaration<'a>) {}

    fn visit_export_from_declaration(&mut self, _: &ExportFromDeclaration<'a>) {}

    fn visit_directive(&mut self, _: &Directive<'a>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn es(s: &str) -> Option<String> {
        match s {
            "Cancel anytime" => Some("Cancela cuando quieras".into()),
            "Add to cart" => Some("Añadir al carrito".into()),
            "Say \"hi\"" => Some("Di «hola»".into()),
            _ => None,
        }
    }
    fn run(js: &str) -> String {
        localize(js, &es)
            .map(|(o, _)| o)
            .unwrap_or_else(|| js.into())
    }

    #[test]
    fn jsx_children_and_text_positions_are_translated() {
        assert_eq!(
            run(r#"(0,r.jsx)("p",{children:"Cancel anytime"})"#),
            r#"(0,r.jsx)("p",{children:"Cancela cuando quieras"})"#
        );
        assert_eq!(
            run("const a=['Add to cart', `Cancel anytime`];"),
            r#"const a=["Añadir al carrito", "Cancela cuando quieras"];"#
        );
        assert_eq!(
            run(r#"React.createElement("p",null," Cancel  anytime ")"#),
            r#"React.createElement("p",null," Cancela cuando quieras ")"#
        );
        assert_eq!(
            run(r#"x=n?"Add to cart":"Cancel anytime""#),
            r#"x=n?"Añadir al carrito":"Cancela cuando quieras""#
        );
        assert_eq!(run(r#"x={label:"Say \"hi\""}"#), r#"x={label:"Di «hola»"}"#);
    }

    #[test]
    fn identifiers_keys_and_code_are_never_touched() {
        for js in [
            r#"x={"Cancel anytime":1}"#,
            r#"class A{"Cancel anytime"(){}}"#,
            r#"if(s==="Cancel anytime")f()"#,
            r#"if("Add to cart"!=s)f()"#,
            r#"switch(s){case "Add to cart":break}"#,
            r#"m["Add to cart"]=1"#,
            r#"track("Add to cart")"#,
            r#"new Error("Add to cart")"#,
            r#"x={id:"Add to cart",variant:"Cancel anytime"}"#,
            r#"import a from "Add to cart";export * from "Cancel anytime";export {b} from "Add to cart""#,
            r#""Add to cart";f()"#,
            "x=tag`Add to cart`",
            "x=`Add to cart ${n}`",
            "// \"Add to cart\"\n/* 'Cancel anytime' */ x=/Add to cart/",
            r#"if("Add to cart" in o)f()"#,
            r#"import("Add to cart")"#,
        ] {
            assert_eq!(run(js), js, "{js}");
        }
    }

    #[test]
    fn scripts_parse_and_garbage_is_left_alone() {
        // sloppy-mode script (`with`) only parses as a script
        assert_eq!(
            run(r#"with(o){x="Add to cart"}"#),
            r#"with(o){x="Añadir al carrito"}"#
        );
        assert!(localize("x = 'Add to cart' +", &es).is_none());
        assert!(localize("x = 'nothing here'", &es).is_none());
        // a translated value that contains quotes / </script> stays a valid literal
        let t = |_: &str| Some("a\"b</script>\u{2028}".to_string());
        assert_eq!(
            localize(r#"x="Add to cart""#, &t).unwrap().0,
            r#"x="a\"b</script>\u2028""#
        );
    }
}
