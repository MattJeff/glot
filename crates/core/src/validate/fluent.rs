//! `Syntax::Fluent`: Project Fluent patterns as glot keeps them (`formats/fluent.rs`): text,
//! `{ $var }`, `{ -term }` / `{ message.attr }` (references, kept as is), `{ "literal" }`,
//! `{ NUMBER($n, …) }` and select expressions with one `*[default]` variant per line.
use super::{CATEGORIES, Choice, Kind, Node, Parser, flush};

/// `Syntax::Plain`: `{name}` placeholders (`[A-Za-z0-9_]`), every other character is text.
// ponytail: lives here because it is tiny; `{{x}}` is text since a naive `replace("{x}")` runtime
// would not define it as a placeholder on purpose.
pub(super) fn plain(text: &str) -> Vec<Node> {
    let (mut out, mut buf) = (vec![], String::new());
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        let after = &rest[at + 1..];
        if let Some(lit) = after.strip_prefix('{') {
            // `{{…}}` literal.
            let end = lit.find("}}").map_or(rest.len(), |e| at + 2 + e + 2);
            buf.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        let n = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        if n > 0 && after[n..].starts_with('}') {
            buf.push_str(&rest[..at]);
            flush(&mut out, &mut buf);
            out.push(Node::Arg {
                name: after[..n].to_string(),
                format: None,
            });
            rest = &after[n + 1..];
        } else {
            buf.push_str(&rest[..at + 1]);
            rest = after;
        }
    }
    buf.push_str(rest);
    flush(&mut out, &mut buf);
    out
}

fn is_fid(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

impl Parser {
    pub(super) fn fluent(&mut self) -> Result<Vec<Node>, String> {
        self.fluent_pattern(false)
    }

    /// Text and placeables up to the end or, in a variant, up to the next variant line / the `}`.
    fn fluent_pattern(&mut self, variant: bool) -> Result<Vec<Node>, String> {
        let (mut out, mut text) = (vec![], String::new());
        while let Some(c) = self.peek() {
            match c {
                '{' => {
                    flush(&mut out, &mut text);
                    self.i += 1;
                    out.push(self.fluent_placeable()?);
                }
                '}' if variant => break,
                '}' => {
                    return self.err("unbalanced `}`; a literal brace is written { \"}\" }");
                }
                '<' => self.push_tag_or_text(&mut out, &mut text),
                '\n' if variant => {
                    let mut j = self.i;
                    while self.s.get(j).is_some_and(|c| c.is_whitespace()) {
                        j += 1;
                    }
                    let next = (self.s.get(j).copied(), self.s.get(j + 1).copied());
                    if matches!(next, (Some('[' | '}'), _) | (Some('*'), Some('['))) {
                        break;
                    }
                    text.push(c);
                    self.i += 1;
                }
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut out, &mut text);
        Ok(out)
    }

    /// After `{`: an expression, or a select expression, then `}`.
    fn fluent_placeable(&mut self) -> Result<Node, String> {
        self.ws();
        let (node, selector) = self.fluent_expr()?;
        self.ws();
        if !self.starts("->") {
            self.expect('}', "unclosed placeable; add `}`")?;
            return Ok(node);
        }
        self.i += 2;
        let Some((name, ordinal)) = selector else {
            return self
                .err("only a variable, a function call or a term attribute can be a selector");
        };
        let (mut options, mut default) = (vec![], None);
        loop {
            self.ws();
            let star = match self.peek() {
                Some('}') => {
                    self.i += 1;
                    break;
                }
                None => return self.err("unclosed select expression; add `}`"),
                Some('*') => {
                    self.i += 1;
                    true
                }
                _ => false,
            };
            self.expect('[', "expected a variant `[key]` or `*[other]`")?;
            let key = self
                .take_while(|c| c != ']' && c != '\n')
                .trim()
                .to_string();
            self.expect(']', "unclosed variant key; add `]`")?;
            while self.peek() == Some(' ') {
                self.i += 1;
            }
            let body = self.fluent_pattern(true)?;
            let key = if key.parse::<f64>().is_ok() {
                format!("={key}")
            } else {
                key
            };
            if star {
                if default.is_some() {
                    return self.err("a select expression has exactly one default variant `*[…]`");
                }
                default = Some(options.len());
            }
            options.push((key, body));
        }
        let Some(d) = default else {
            return self.err("a select expression needs a default variant, e.g. `*[other]`");
        };
        // The default variant answers every other value: it is ICU's `other`.
        if !options.iter().any(|(k, _)| k == "other") {
            let body = options[d].1.clone();
            options.push(("other".into(), body));
        }
        let plural = options.iter().all(|(k, _)| {
            CATEGORIES.contains(&k.as_str())
                || k.starts_with('=')
                || super::untranslated_keyword(k).is_some()
        });
        let (kind, keyword) = match () {
            _ if ordinal => (Kind::Ordinal, "selectordinal"),
            _ if plural && !name.starts_with('-') => (Kind::Plural, "plural"),
            _ => (Kind::Select, "select"),
        };
        Ok(Node::Choice(Choice {
            name,
            keyword: keyword.into(),
            kind,
            options,
        }))
    }

    /// An inline expression and, when it can select, its name and whether it is ordinal.
    fn fluent_expr(&mut self) -> Result<(Node, Option<(String, bool)>), String> {
        match self.peek() {
            Some('$') => {
                self.i += 1;
                let n = self.take_while(is_fid);
                if n.is_empty() {
                    return self.err("`$` must be followed by a variable name");
                }
                Ok((
                    Node::Arg {
                        name: n.clone(),
                        format: None,
                    },
                    Some((n, false)),
                ))
            }
            Some('"') => {
                self.i += 1;
                let mut lit = String::new();
                loop {
                    match self.bump() {
                        Some('"') => break,
                        Some('\\') => lit.extend(self.bump()),
                        Some('\n') | None => return self.err("unclosed string literal; add `\"`"),
                        Some(c) => lit.push(c),
                    }
                }
                Ok((Node::Text(lit), None))
            }
            Some('{') => {
                self.i += 1;
                Ok((self.fluent_placeable()?, None))
            }
            Some(c)
                if c.is_ascii_digit()
                    || (c == '-' && self.peek_at(1).is_some_and(|c| c.is_ascii_digit())) =>
            {
                self.i += 1;
                let n = self.take_while(|c| c.is_ascii_digit() || c == '.');
                Ok((Node::Text(format!("{c}{n}")), None))
            }
            Some(c) if c == '-' || c.is_ascii_alphabetic() => {
                let term = c == '-';
                self.i += term as usize;
                let id = self.take_while(is_fid);
                if id.is_empty() {
                    return self.err("expected a term name after `-`");
                }
                let attr = if self.peek() == Some('.') {
                    self.i += 1;
                    format!(".{}", self.take_while(is_fid))
                } else {
                    String::new()
                };
                self.ws();
                let args = if self.peek() == Some('(') {
                    Some(self.fluent_args()?)
                } else {
                    None
                };
                // `NUMBER($n, type: "ordinal")`: a formatted variable.
                if !term && attr.is_empty() && id.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                    let args = args.unwrap_or_default();
                    let var = args
                        .split('$')
                        .nth(1)
                        .map(|v| v.chars().take_while(|c| is_fid(*c)).collect::<String>())
                        .filter(|v| !v.is_empty());
                    let ordinal = args.replace(' ', "").contains("type:\"ordinal\"");
                    return Ok(match var {
                        Some(v) => (
                            Node::Arg {
                                name: v.clone(),
                                format: Some(id),
                            },
                            Some((v, ordinal)),
                        ),
                        None => (Node::Ref(format!("{id}()")), None),
                    });
                }
                let name = format!("{}{id}{attr}", if term { "-" } else { "" });
                let selector = (term && !attr.is_empty()).then(|| (name.clone(), false));
                Ok((Node::Ref(name), selector))
            }
            _ => self.err(
                "expected a variable `$name`, a term `-name`, a message reference, a string or a number",
            ),
        }
    }

    /// `( … )` call arguments, returned without the parentheses.
    fn fluent_args(&mut self) -> Result<String, String> {
        self.i += 1;
        let (mut out, mut depth) = (String::new(), 0);
        loop {
            match self.bump() {
                Some(')') if depth == 0 => return Ok(out),
                Some(c) => {
                    depth += (c == '(') as i32 - (c == ')') as i32;
                    out.push(c);
                }
                None => return self.err("unclosed function call; add `)`"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::{IssueCode, Syntax, placeholders, validate};

    fn codes(src: &str, tgt: &str, locale: &str) -> Vec<IssueCode> {
        validate(src, tgt, locale, Syntax::Fluent, &Default::default())
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    const PLURAL: &str = "{ $count ->\n    [one] One email.\n   *[other] { $count } emails.\n}";

    #[test]
    fn placeholders_and_refs() {
        let ph = placeholders(
            "Hi { $name }, { -brand } costs { NUMBER($price, minimumFractionDigits: 2) } { \"}\" }",
            Syntax::Fluent,
        )
        .unwrap();
        assert_eq!(ph.into_iter().collect::<Vec<_>>(), ["name", "price"]);
        assert!(codes("Hi { $name }", "Salut { $name }", "fr").is_empty());
        assert_eq!(
            codes("Hi { $name }", "Salut { $nom }", "fr"),
            [IssueCode::PlaceholderMissing, IssueCode::PlaceholderExtra]
        );
        // A translated term reference.
        assert_eq!(
            codes("Open { -brand }", "Ouvrir { -marque }", "fr"),
            [IssueCode::PlaceholderMissing]
        );
        assert_eq!(
            codes("Hi { $name", "Salut { $name", "fr"),
            [IssueCode::SyntaxError]
        );
        assert_eq!(codes("a", "b } c", "fr"), [IssueCode::SyntaxError]);
    }

    #[test]
    fn plain() {
        let c = |s, t| {
            validate(s, t, "en", Syntax::Plain, &Default::default())
                .into_iter()
                .map(|i| i.code)
                .collect::<Vec<_>>()
        };
        assert!(
            c(
                "Bonjour {n}, écris {{sujet}} : Bearer <clé>",
                "Hi {n}, write {{topic}}: Bearer <key>"
            )
            .is_empty()
        );
        assert_eq!(
            c("{n} vidéos", "{m} videos"),
            [IssueCode::PlaceholderMissing, IssueCode::PlaceholderExtra]
        );
        assert!(c("L'accolade { seule", "A lone { brace").is_empty());
    }

    #[test]
    fn selects() {
        let fr = "{ $count ->\n    [one] Un e-mail.\n   *[other] { $count } e-mails.\n}";
        // French: only the soft `many` warning.
        let issues = validate(PLURAL, fr, "fr", Syntax::Fluent, &Default::default());
        assert!(!crate::validate::has_errors(&issues), "{issues:?}");
        // Polish needs few / many.
        assert!(codes(PLURAL, fr, "pl").contains(&IssueCode::PluralCategoryMissing));
        // No default variant.
        let bad = "{ $count ->\n    [one] Un.\n    [other] Plusieurs.\n}";
        assert_eq!(codes(PLURAL, bad, "fr"), [IssueCode::SyntaxError]);
        // Gender select with a non-`other` default, ordinal.
        let g = "{ $g ->\n    [male] il\n   *[neutral] iel\n}";
        assert!(
            codes(g, g, "fr")
                .iter()
                .all(|c| *c == IssueCode::Untranslated)
        );
        let o = "{ NUMBER($pos, type: \"ordinal\") ->\n    [one] {$pos}st\n    [two] {$pos}nd\n    [few] {$pos}rd\n   *[other] {$pos}th\n}";
        assert!(!codes(o, o, "en").contains(&IssueCode::SyntaxError));
        // Translated selector key.
        let un = "{ $count ->\n    [un] Un.\n   *[other] Plusieurs.\n}";
        assert!(codes(PLURAL, un, "fr").contains(&IssueCode::KeywordTranslated));
    }
}
