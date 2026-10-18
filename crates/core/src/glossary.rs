use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::config::{read_optional, write_file};
use crate::validate::{Issue, IssueCode, Severity};
use crate::{Error, Locale, Result, plural::language};

pub const GLOSSARY_FILE: &str = "glossary.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Glossary {
    #[serde(default, rename = "term")]
    pub terms: Vec<Term>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Term {
    pub term: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub do_not_translate: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub translations: BTreeMap<Locale, String>,
    #[serde(default)]
    pub forbidden: BTreeMap<Locale, Vec<String>>,
}

impl Glossary {
    /// Missing file => empty glossary.
    pub fn load(path: &Path) -> Result<Self> {
        match read_optional(path)? {
            Some(text) => toml::from_str(&text).map_err(|e| Error::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
            None => Ok(Self::default()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        write_file(path, &text)
    }

    /// Terms that occur in `source` (word-boundary match, case-insensitive unless case_sensitive).
    pub fn matches(&self, source: &str) -> Vec<&Term> {
        self.terms
            .iter()
            .filter(|t| find_word(source, &t.term, t.case_sensitive))
            .collect()
    }

    /// GlossaryViolation (expected translation absent, or forbidden variant used) and
    /// DoNotTranslateViolation (term missing verbatim from target). Errors.
    pub fn check(&self, source: &str, target: &str, locale: &str) -> Vec<Issue> {
        let mut issues = Vec::new();
        let error = |code, message| Issue {
            code,
            severity: Severity::Error,
            message,
        };
        for t in self.matches(source) {
            if t.do_not_translate {
                if !contains(target, &t.term, t.case_sensitive) {
                    issues.push(error(
                        IssueCode::DoNotTranslateViolation,
                        format!(
                            "`{}` must not be translated: keep `{}` verbatim in the translation.",
                            t.term, t.term
                        ),
                    ));
                }
                continue;
            }
            let expected = lookup(&t.translations, locale);
            if let Some(exp) = expected
                && !contains(target, exp, t.case_sensitive)
            {
                issues.push(error(
                    IssueCode::GlossaryViolation,
                    format!(
                        "Glossary: `{}` must be translated as `{exp}` in {locale}.",
                        t.term
                    ),
                ));
            }
            for f in lookup(&t.forbidden, locale).into_iter().flatten() {
                if contains(target, f, false) {
                    let hint = expected
                        .map(|e| format!(" Use `{e}` instead."))
                        .unwrap_or_default();
                    issues.push(error(
                        IssueCode::GlossaryViolation,
                        format!(
                            "Glossary: `{f}` is a forbidden translation of `{}` in {locale}.{hint}",
                            t.term
                        ),
                    ));
                }
            }
        }
        issues
    }

    /// Insert or replace by `term` (case-insensitive).
    pub fn upsert(&mut self, term: Term) {
        let key = term.term.to_lowercase();
        match self.terms.iter_mut().find(|t| t.term.to_lowercase() == key) {
            Some(t) => *t = term,
            None => self.terms.push(term),
        }
    }
}

/// Exact locale first, then the language subtag ("pt-BR" => "pt").
fn lookup<'a, T>(map: &'a BTreeMap<Locale, T>, locale: &str) -> Option<&'a T> {
    map.get(locale).or_else(|| map.get(&language(locale)))
}

fn contains(hay: &str, needle: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        hay.contains(needle)
    } else {
        hay.to_lowercase().contains(&needle.to_lowercase())
    }
}

fn find_word(hay: &str, needle: &str, case_sensitive: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let (h, n) = if case_sensitive {
        (hay.to_string(), needle.to_string())
    } else {
        (hay.to_lowercase(), needle.to_lowercase())
    };
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    h.match_indices(&n).any(|(i, m)| {
        !h[..i].chars().next_back().is_some_and(is_word)
            && !h[i + m.len()..].chars().next().is_some_and(is_word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        toml::from_str(
            r#"
[[term]]
term = "Workspace"
translations = { fr = "Espace de travail" }
forbidden = { fr = ["Espace de travaux"] }

[[term]]
term = "glot"
do_not_translate = true

[[term]]
term = "API"
case_sensitive = true
"#,
        )
        .unwrap()
    }

    #[test]
    fn matching() {
        let g = glossary();
        let names = |s| {
            g.matches(s)
                .iter()
                .map(|t| t.term.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("Open your workspace, glot!"), ["Workspace", "glot"]);
        assert!(names("Workspaces and glotting").is_empty());
        assert!(names("the api").is_empty());
        assert_eq!(names("the API"), ["API"]);
        assert_eq!(names("Élan Workspace"), ["Workspace"]);
    }

    #[test]
    fn checking() {
        let g = glossary();
        assert!(
            g.check("Your workspace", "Ton espace de travail", "fr")
                .is_empty()
        );
        assert!(
            g.check("Your workspace", "Ton espace de travail", "fr-CA")
                .is_empty()
        );
        let i = g.check("Your workspace", "Ton espace de travaux", "fr");
        assert_eq!(i.len(), 2);
        assert!(i.iter().all(|i| i.code == IssueCode::GlossaryViolation));
        let i = g.check("Try glot", "Essaie glotte", "fr");
        assert_eq!(i.len(), 0, "substring still contains glot");
        let i = g.check("Try glot", "Essaie l'outil", "fr");
        assert_eq!(i[0].code, IssueCode::DoNotTranslateViolation);
    }

    #[test]
    fn upsert_and_files() {
        let mut g = glossary();
        g.upsert(Term {
            term: "workspace".into(),
            do_not_translate: true,
            ..Default::default()
        });
        assert_eq!(g.terms.len(), 3);
        assert!(g.terms[0].do_not_translate);
        g.upsert(Term {
            term: "Team".into(),
            ..Default::default()
        });
        assert_eq!(g.terms.len(), 4);
        let p = std::env::temp_dir().join(format!(
            "glot-glossary-{}/glossary.toml",
            std::process::id()
        ));
        assert_eq!(Glossary::load(&p).unwrap(), Glossary::default());
        g.save(&p).unwrap();
        assert_eq!(Glossary::load(&p).unwrap(), g);
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}
