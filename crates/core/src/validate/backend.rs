//! Back-end message syntaxes.
//!
//! - `Syntax::Symfony`: Symfony Translation. Files suffixed `+intl-icu` hold ICU messages; the
//!   others hold legacy messages: `%name%` and `{{ name }}` (validators) parameters, plurals as
//!   `|`-separated forms with optional intervals (`{0} none|{1} one|]1,Inf[ %count% many`). A
//!   message with an ICU `{n, plural|select, …}` block is parsed as ICU, any other as legacy (a
//!   plain `{name}` counts as a parameter either way).
//! - `Syntax::Java`: `java.text.MessageFormat` (Spring `messages.properties`, `ResourceBundle`):
//!   `{0}`, `{0,number}`, `{0,choice,…}`; a single `'` quotes text up to the next `'` and `''` is
//!   one apostrophe. A message without `{` is not run through MessageFormat (Spring's default
//!   `alwaysUseMessageFormat = false`), so its apostrophes are plain text.
use super::{Node, Parser, flush};

/// `%name%` parameter name characters (Symfony: `%count%`, `%user.name%`).
fn is_param(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// Java MessageFormat text as ICU text with the same meaning: quoted sections become text (their
/// braces quoted ICU-style), `''` stays `''`.
pub(super) fn java_to_icu(s: &str) -> String {
    if !s.contains('{') {
        return s.replace('\'', "''");
    }
    let (mut out, mut quoted) = (String::new(), false);
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' if it.peek() == Some(&'\'') => {
                it.next();
                out.push_str("''");
            }
            '\'' => quoted = !quoted,
            '{' | '}' | '#' | '|' if quoted => {
                out.push('\'');
                out.push(c);
                out.push('\'');
            }
            _ => out.push(c),
        }
    }
    out
}

/// A lone `'` (not `''`) in a MessageFormat message: Java hides everything up to the next `'`.
pub(super) fn java_lone_quote(s: &str) -> bool {
    s.contains('{') && s.replace("''", "").contains('\'')
}

impl Parser {
    /// Legacy Symfony message: text, `%param%`, `{{ param }}`, `{param}`, tags, interval prefixes
    /// (`{0}`, `[2,Inf[`) at the start of each `|` form kept as text.
    pub(super) fn symfony_legacy(&mut self) -> Vec<Node> {
        let (mut out, mut text) = (Vec::new(), String::new());
        let piped = self.s.contains(&'|');
        let mut form_start = true;
        while let Some(c) = self.peek() {
            if form_start && piped && !c.is_whitespace() {
                form_start = false;
                if let Some(n) = self.interval() {
                    text.extend(&self.s[self.i..self.i + n]);
                    self.i += n;
                    continue;
                }
            }
            match c {
                '|' => {
                    form_start = true;
                    text.push(c);
                    self.i += 1;
                }
                '%' => {
                    let n = self.s[self.i + 1..]
                        .iter()
                        .take_while(|c| is_param(**c))
                        .count();
                    if n > 0 && self.peek_at(n + 1) == Some('%') {
                        flush(&mut out, &mut text);
                        let name: String = self.s[self.i + 1..self.i + 1 + n].iter().collect();
                        out.push(Node::Arg { name, format: None });
                        self.i += n + 2;
                    } else {
                        text.push(c);
                        self.i += 1;
                    }
                }
                '{' => {
                    let double = self.peek_at(1) == Some('{');
                    let open = if double { 2 } else { 1 };
                    let close = if double { "}}" } else { "}" };
                    let rest: String = self.s[self.i + open..].iter().collect();
                    let name = rest.find(close).map(|e| rest[..e].trim().to_string());
                    match name.filter(|n| !n.is_empty() && n.chars().all(is_param)) {
                        Some(name) => {
                            flush(&mut out, &mut text);
                            let len = rest.find(close).unwrap();
                            self.i += open + rest[..len].chars().count() + close.len();
                            out.push(Node::Arg { name, format: None });
                        }
                        None => {
                            text.push(c);
                            self.i += 1;
                        }
                    }
                }
                '<' => self.push_tag_or_text(&mut out, &mut text),
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut out, &mut text);
        out
    }

    /// Length of a Symfony interval at the cursor: `{0}`, `{1,2,3}`, `[1,Inf[`, `]-Inf,0]`.
    fn interval(&self) -> Option<usize> {
        let first = self.peek()?;
        let closers: &[char] = match first {
            '{' => &['}'],
            '[' | ']' => &[']', '['],
            _ => return None,
        };
        let n = self.s[self.i + 1..]
            .iter()
            .position(|c| closers.contains(c))?;
        let inner: String = self.s[self.i + 1..self.i + 1 + n].iter().collect();
        let ok = !inner.trim().is_empty()
            && inner.split(',').all(|p| {
                let p = p.trim().trim_start_matches(['-', '+']);
                p == "Inf" || p == "*" || (!p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            });
        ok.then_some(n + 2)
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::{IssueCode, Syntax, placeholders, validate};

    fn codes(src: &str, tgt: &str, locale: &str, syntax: Syntax) -> Vec<IssueCode> {
        validate(src, tgt, locale, syntax, &Default::default())
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn symfony_parameters() {
        let s = Syntax::Symfony;
        let names = |t: &str| placeholders(t, s).unwrap().into_iter().collect::<Vec<_>>();
        assert_eq!(names("Hello %name%, 100% sure"), ["name"]);
        assert_eq!(names("The email {{ value }} is invalid."), ["value"]);
        assert_eq!(names("Hi {name}"), ["name"]);
        assert_eq!(
            names("{0} No apples|{1} One apple|]1,Inf[ %count% apples"),
            ["count"]
        );
        assert!(codes("Hello %name%", "Bonjour %name%", "fr", s).is_empty());
        assert_eq!(
            codes("Hello %name%", "Bonjour %nom%", "fr", s),
            [IssueCode::PlaceholderMissing, IssueCode::PlaceholderExtra]
        );
        assert_eq!(
            codes(
                "Max {{ limit }} chars.",
                "Max {{ limite }} caractères.",
                "fr",
                s
            ),
            [IssueCode::PlaceholderMissing, IssueCode::PlaceholderExtra]
        );
        // `+intl-icu` messages are ICU.
        let icu = "{count, plural, one {# item} other {# items}}";
        let de = "{count, plural, one {# Artikel} other {# Artikel}}";
        assert!(codes(icu, de, "de", s).is_empty());
        assert!(codes(icu, de, "pl", s).contains(&IssueCode::PluralCategoryMissing));
    }

    #[test]
    fn symfony_plural_intervals() {
        let s = Syntax::Symfony;
        let src = "{0} No apples|{1} One apple|]1,Inf[ %count% apples";
        assert!(
            codes(
                src,
                "{0} Aucune pomme|{1} Une pomme|]1,Inf[ %count% pommes",
                "fr",
                s
            )
            .is_empty()
        );
        // An interval dropped or changed.
        assert_eq!(
            codes(
                src,
                "{0} Aucune pomme|{1} Une pomme|%count% pommes",
                "fr",
                s
            ),
            [IssueCode::PluralCategoryMissing]
        );
        // Forms without intervals follow the target's plural rule (ru: 3).
        let two = "One apple|%count% apples";
        assert!(codes(two, "Une pomme|%count% pommes", "fr", s).is_empty());
        assert_eq!(
            codes(two, "Одно яблоко|%count% яблок", "ru", s),
            [IssueCode::PluralCategoryMissing]
        );
    }

    #[test]
    fn java_message_format() {
        let j = Syntax::Java;
        let names = |t: &str| placeholders(t, j).unwrap().into_iter().collect::<Vec<_>>();
        assert_eq!(names("Hello {0}, you have {1,number} messages"), ["0", "1"]);
        assert_eq!(names("It''s {0}"), ["0"]);
        assert_eq!(names("'{0}' is literal, {1} is not"), ["1"]);
        assert_eq!(names("{0,choice,0#no files|1#one file|1<{0} files}"), ["0"]);
        assert!(codes("It''s {0}", "C''est {0}", "fr", j).is_empty());
        // French elision with a single quote: Java would print `lutilisateur {0}`.
        let c = codes("The user {0} left", "L'utilisateur {0} est parti", "fr", j);
        assert!(c.contains(&IssueCode::SyntaxError), "{c:?}");
        // No `{`: not run through MessageFormat, a single apostrophe is text.
        assert!(codes("Welcome", "Bienvenue à l'accueil", "fr", j).is_empty());
    }
}
