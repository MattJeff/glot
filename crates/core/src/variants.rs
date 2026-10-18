//! Register variants as locale axes: `fr` + `informal` => `fr-x-informal` (BCP 47 private use).
//!
//! ```toml
//! [variants]
//! fr = ["formal", "informal"]   # vous / tu
//! de = ["sie", "du"]
//! en = ["plain"]                # plain language (accessibility)
//! ja = ["keigo"]
//! ```
//! Variant catalogs live next to the others (`messages/fr-x-informal.json`). They are generated from
//! their **base** locale (not the source) so wording stays consistent, and validated against it:
//! same placeholders, plural categories and tags as the base.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

use crate::catalog::Catalog;
use crate::config::{CONFIG_FILE, read_optional};
use crate::validate::{Constraints, Issue, IssueCode, Syntax, validate};
use crate::{Error, Key, Result};

/// base locale -> variant names.
pub type Variants = BTreeMap<String, Vec<String>>;

/// `[variants]` of `glot.toml` (empty when absent).
// ponytail: read beside `Config` rather than as a field of it, so the struct literals building a
// `Config` (server jobs/insights) keep compiling; fold it into `Config` when those go through Default.
pub fn load(root: &Path) -> Result<Variants> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        variants: Variants,
    }
    let path = root.join(CONFIG_FILE);
    let Some(text) = read_optional(&path)? else {
        return Ok(Variants::new());
    };
    toml::from_str::<File>(&text)
        .map(|f| f.variants)
        .map_err(|e| Error::Parse {
            path,
            message: e.to_string(),
        })
}

/// `fr` + `informal` => `fr-x-informal`.
pub fn tag(base: &str, variant: &str) -> String {
    format!("{base}-x-{variant}")
}

/// `fr-x-informal` => (`fr`, `informal`); `None` for plain locales.
pub fn split(locale: &str) -> Option<(&str, &str)> {
    locale
        .split_once("-x-")
        .filter(|(b, v)| !b.is_empty() && !v.is_empty())
}

/// Every (base, variant tag) declared, in config order.
pub fn locales(v: &Variants) -> Vec<(String, String)> {
    v.iter()
        .flat_map(|(base, names)| names.iter().map(move |n| (base.clone(), tag(base, n))))
        .collect()
}

/// What the LLM must do to turn the base text into this variant (goes into the style guide).
pub fn instruction(base: &str, variant: &str) -> String {
    let how = match variant {
        "formal" | "sie" | "vous" => {
            "formal register: address the user formally (vous in French, Sie in German, usted in Spanish)"
        }
        "informal" | "du" | "tu" => {
            "informal register: address the user informally (tu in French, du in German, tú in Spanish)"
        }
        "plain" => {
            "plain language for accessibility: short common words, short sentences, no jargon or idioms (CEFR A2-B1)"
        }
        "keigo" => {
            "keigo: polite honorific Japanese (sonkeigo/kenjōgo where appropriate, desu/masu)"
        }
        other => {
            return format!(
                "Variant `{other}` of `{base}`: rewrite the `{base}` text in the `{other}` style, changing only what that style requires."
            );
        }
    };
    format!(
        "Register variant of `{base}` — {how}. The source is already in `{base}`: rewrite it in that register, changing only what the register requires; keep terminology identical."
    )
}

/// Variant entries vs their base: placeholders, plural categories and tags must match the base.
/// Identical text is fine (many strings have no addressing form).
pub fn check(base: &Catalog, variant: &Catalog, syntax: Syntax) -> Vec<(Key, Issue)> {
    let mut out = vec![];
    for (key, value) in &variant.entries {
        let Some(src) = base.entries.get(key).filter(|s| !s.is_empty()) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        for i in validate(src, value, &variant.locale, syntax, &Constraints::default()) {
            if i.code != IssueCode::Untranslated {
                out.push((key.clone(), i));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{IssueCode, Severity};

    #[test]
    fn tags_config_and_check() {
        assert_eq!(tag("fr", "informal"), "fr-x-informal");
        assert_eq!(split("fr-x-informal"), Some(("fr", "informal")));
        assert_eq!(split("pt-BR"), None);
        assert_eq!(crate::plural::categories("fr-x-informal").len(), 3);
        assert!(instruction("fr", "informal").contains("tu"));
        assert!(instruction("de", "custom").contains("`custom`"));

        let dir = std::env::temp_dir().join(format!("glot-variants-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"{locale}.json\"]\n[variants]\nfr = [\"formal\", \"informal\"]\nen = [\"plain\"]\n",
        )
        .unwrap();
        let v = load(&dir).unwrap();
        // The regular config still loads (unknown table ignored).
        crate::config::Config::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            locales(&v),
            [
                ("en".into(), "en-x-plain".into()),
                ("fr".into(), "fr-x-formal".into()),
                ("fr".into(), "fr-x-informal".into())
            ]
        );

        let base = Catalog::parse(
            r#"{"hi": "Bonjour {name}, vous avez {n, plural, one {# message} many {# messages} other {# messages}}", "save": "Enregistrer"}"#,
            "fr",
        )
        .unwrap();
        let ok = Catalog::parse(
            r#"{"hi": "Salut {name}, tu as {n, plural, one {# message} many {# messages} other {# messages}}", "save": "Enregistrer"}"#,
            "fr-x-informal",
        )
        .unwrap();
        assert_eq!(check(&base, &ok, Syntax::Icu), []);
        let bad = Catalog::parse(
            r#"{"hi": "Salut {nom}, tu as des messages"}"#,
            "fr-x-informal",
        )
        .unwrap();
        let issues = check(&base, &bad, Syntax::Icu);
        assert!(issues.iter().any(|(k, i)| k == "hi"
            && i.code == IssueCode::PlaceholderMissing
            && i.severity == Severity::Error));
    }
}
