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
        self.violations(source, target, locale, false)
            .into_iter()
            .map(|(_, i)| i)
            .collect()
    }

    /// Like `check`, with the offending term. Common terms are not matched inside a do-not-translate
    /// occurrence ("invoice" in "Acme Invoices"). `inflected`: case-insensitive terms and expected
    /// translations also match inflected forms ("invoices", "espaces de travail", "Rechnungen").
    pub fn violations(
        &self,
        source: &str,
        target: &str,
        locale: &str,
        inflected: bool,
    ) -> Vec<(&Term, Issue)> {
        let mut masked = source.to_string();
        for t in self.terms.iter().filter(|t| t.do_not_translate) {
            for r in spans(source, &t.term, t.case_sensitive) {
                masked.replace_range(r.clone(), &" ".repeat(r.len()));
            }
        }
        let mut issues = Vec::new();
        let error = |code, message| Issue {
            code,
            severity: Severity::Error,
            message,
        };
        for t in &self.terms {
            let hay = if t.do_not_translate { source } else { &masked };
            if !find(hay, &t.term, t.case_sensitive, inflected) {
                continue;
            }
            if t.do_not_translate {
                if !contains(target, &t.term, t.case_sensitive) {
                    issues.push((
                        t,
                        error(
                            IssueCode::DoNotTranslateViolation,
                            format!(
                                "`{}` must not be translated: keep `{}` verbatim in the translation.",
                                t.term, t.term
                            ),
                        ),
                    ));
                }
                continue;
            }
            let expected = lookup(&t.translations, locale);
            if let Some(exp) = expected
                && !contains(target, exp, t.case_sensitive)
                && !(inflected && find(target, exp, t.case_sensitive, true))
            {
                issues.push((
                    t,
                    error(
                        IssueCode::GlossaryViolation,
                        format!(
                            "Glossary: `{}` must be translated as `{exp}` in {locale}.",
                            t.term
                        ),
                    ),
                ));
            }
            for f in lookup(&t.forbidden, locale).into_iter().flatten() {
                if contains(target, f, false) {
                    let hint = expected
                        .map(|e| format!(" Use `{e}` instead."))
                        .unwrap_or_default();
                    issues.push((
                        t,
                        error(
                            IssueCode::GlossaryViolation,
                            format!(
                                "Glossary: `{f}` is a forbidden translation of `{}` in {locale}.{hint}",
                                t.term
                            ),
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
    !spans(hay, needle, case_sensitive).is_empty()
}

/// Byte ranges of the word-boundary occurrences of `needle` in `hay` (char-wise case folding).
fn spans(hay: &str, needle: &str, case_sensitive: bool) -> Vec<std::ops::Range<usize>> {
    if needle.is_empty() {
        return vec![];
    }
    let eq =
        |a: char, b: char| a == b || (!case_sensitive && a.to_lowercase().eq(b.to_lowercase()));
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = vec![];
    for (i, _) in hay.char_indices() {
        if hay[..i].chars().next_back().is_some_and(is_word) {
            continue;
        }
        let mut rest = hay[i..].char_indices();
        let mut end = Some(i);
        for n in needle.chars() {
            end = match rest.next() {
                Some((j, c)) if eq(c, n) => Some(i + j + c.len_utf8()),
                _ => None,
            };
            if end.is_none() {
                break;
            }
        }
        if let Some(e) = end
            && !hay[e..].chars().next().is_some_and(is_word)
        {
            out.push(i..e);
        }
    }
    out
}

/// Word-boundary match; with `inflected` (case-insensitive only) each word may be an inflected form.
pub fn find(hay: &str, needle: &str, case_sensitive: bool, inflected: bool) -> bool {
    if !inflected || case_sensitive {
        return find_word(hay, needle, case_sensitive);
    }
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let (h, n) = (words(hay), words(needle));
    !n.is_empty()
        && h.windows(n.len())
            .any(|w| w.iter().zip(&n).all(|(a, b)| same_lemma(a, b)))
}

/// Same word up to a short inflectional suffix: invoice/invoices, travail/travaux, Rechnung/Rechnungen.
/// ponytail: suffix heuristic, no stemmer; misses stem changes (Konto/Konten), may join plan/planet.
pub fn same_lemma(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let p = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (s, l) = (a.len().min(b.len()), a.len().max(b.len()));
    a == b || (p >= 4 && l - p <= 3 && s - p <= 2 && (s >= 6 || p == s))
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
    fn dnt_masks_common_terms_and_inflections() {
        let g: Glossary = toml::from_str(
            r#"
[[term]]
term = "Acme Invoices"
do_not_translate = true
case_sensitive = true

[[term]]
term = "invoice"
translations = { fr = "facture", de = "Rechnung" }

[[term]]
term = "workspace"
translations = { fr = "espace de travail" }
"#,
        )
        .unwrap();
        // "Invoices" inside the brand is not the common noun.
        assert!(
            g.check(
                "Welcome to Acme Invoices",
                "Bienvenue sur Acme Invoices",
                "fr"
            )
            .is_empty()
        );
        assert_eq!(
            g.check("Acme Invoices sends invoices", "Acme Invoices envoie", "fr")
                .len(),
            0,
            "exact mode: `invoice` not a word of `invoices`"
        );
        let v = |s, t, l| g.violations(s, t, l, true).len();
        assert_eq!(
            v(
                "Acme Invoices sends invoices",
                "Acme Invoices envoie des factures",
                "fr"
            ),
            0
        );
        assert_eq!(v("Your invoices", "Ihre Rechnungen", "de"), 0);
        assert_eq!(v("All workspaces", "Tous les espaces de travail", "fr"), 0);
        assert_eq!(v("Your invoices", "Vos notes", "fr"), 1);
        assert!(same_lemma("travail", "travaux") && !same_lemma("date", "data"));
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
