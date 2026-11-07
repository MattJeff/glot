//! Docusaurus `usePluralForm` messages in `i18n/{locale}/code.json` and the theme JSON files:
//! `"One doc tagged|{count} docs tagged"`, one form per CLDR category of the locale, in the order
//! zero, one, two, few, many, other. In memory they are `{count, plural, one {…} other {…}}`, so
//! the validator checks every category the locale needs; written back joined with `|`.
//! A message is plural when its id or description says so (`….plurals`, "Pluralized label …").
use std::path::Path;

use super::{Parsed, folded_choice, icu_choice};
use crate::catalog::Catalog;

/// `i18n/<locale>/code.json` or a theme/plugin file (`i18n/<locale>/docusaurus-…/*.json`).
pub(crate) fn is_code_json(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    s.contains("i18n/") && (s.ends_with("/code.json") || s.contains("/docusaurus-"))
}

fn cats(locale: &str) -> Vec<&'static str> {
    let mut c = crate::plural::categories(locale);
    c.sort();
    c.iter().map(|c| c.as_str()).collect()
}

fn plural(key: &str, note: Option<&String>) -> bool {
    key.to_ascii_lowercase().contains("plural")
        || note.is_some_and(|n| n.to_ascii_lowercase().contains("plural"))
}

pub(crate) fn fold(mut p: Parsed, locale: &str) -> Parsed {
    let cats = cats(locale);
    for (k, v) in p.entries.iter_mut() {
        if !plural(k, p.notes.get(k)) || (!v.contains('|') && cats.len() > 1) {
            continue;
        }
        let forms: Vec<(String, String)> = cats
            .iter()
            .zip(v.split('|'))
            .map(|(c, t)| (c.to_string(), t.to_string()))
            .collect();
        *v = icu_choice("count", "plural", &forms);
    }
    p
}

pub(crate) fn unfold(cat: &Catalog) -> Catalog {
    let order = ["zero", "one", "two", "few", "many", "other"];
    let mut out = cat.clone();
    for v in out.entries.values_mut() {
        let Some(c) = folded_choice(v).filter(|c| {
            c.var == "count"
                && c.kind == "plural"
                && c.forms.iter().all(|(f, _)| order.contains(&f.as_str()))
        }) else {
            continue;
        };
        let mut forms = c.forms.clone();
        forms.sort_by_key(|(f, _)| order.iter().position(|o| o == f));
        *v = forms
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>()
            .join("|");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{Format, native, parse, write};
    use crate::validate::{Constraints, IssueCode, Syntax, validate};

    const EN: &str = r#"{
  "theme.docs.tagDocListPageTitle.nDocsTagged": {
    "message": "One doc tagged|{count} docs tagged",
    "description": "Pluralized label for \"{count} docs tagged\". Use as much plural forms (separated by \"|\") as your language support"
  },
  "theme.common.skipToMainContent": {
    "message": "Skip to main content|now",
    "description": "The skip to content label"
  }
}
"#;

    #[test]
    fn plural_messages_are_plurals() {
        let path = Path::new("site/i18n/en/code.json");
        let f = native(Format::JsonMessage, path).unwrap();
        let en = parse(f, path, EN, "en").unwrap();
        let k = "theme.docs.tagDocListPageTitle.nDocsTagged";
        assert_eq!(
            en.entries[k],
            "{count, plural, one {One doc tagged} other {{count} docs tagged}}"
        );
        assert_eq!(
            en.entries["theme.common.skipToMainContent"], "Skip to main content|now",
            "not described as plural"
        );
        let same = Catalog {
            locale: "en".into(),
            entries: en.entries.clone(),
        };
        assert_eq!(write(f, path, Some(EN), &same), EN, "byte for byte");

        // fr has one / many / other: two forms are not enough.
        let src = &en.entries[k];
        let two = "{count, plural, one {Un doc} other {{count} docs}}";
        let issues = validate(src, two, "fr", Syntax::Icu, &Constraints::default());
        assert!(
            issues
                .iter()
                .any(|i| i.code == IssueCode::PluralCategoryMissing),
            "{issues:?}"
        );
        let fr = Catalog {
            locale: "fr".into(),
            entries: [(
                k.to_string(),
                "{count, plural, other {{count} docs} one {Un doc} many {{count} de docs}}".into(),
            )]
            .into(),
        };
        let out = write(f, Path::new("site/i18n/fr/code.json"), Some(EN), &fr);
        assert!(
            out.contains("\"message\": \"Un doc|{count} de docs|{count} docs\""),
            "{out}"
        );
    }
}
