//! rust-i18n YAML (`locales/*.yml`, recognized by its top-level `_version:` line).
//! `_version: 1`: one file per locale, keys at the root (the YAML reader as is, `_version` kept but
//! not a message). `_version: 2`: every locale in one file, the locale is the leaf
//! (`hello:\n  en: Hello\n  fr: Bonjour`); glot reads and patches only `locale`'s leaves.
//! Values keep rust-i18n's `%{name}` (`Syntax::Rails`).
use std::path::Path;

use super::{Parsed, yaml};
use crate::catalog::Catalog;

const VERSION: &str = "_version";

pub fn is_rust_i18n(text: &str) -> bool {
    text.lines().any(|l| l.starts_with("_version:"))
}

fn v2(text: &str) -> bool {
    text.lines()
        .any(|l| l.strip_prefix("_version:").is_some_and(|v| v.trim() == "2"))
}

/// Every entry of the file (all locales for v2), keys as the YAML reader flattens them.
fn all(text: &str) -> Result<Parsed, String> {
    // No locale root to strip in rust-i18n files.
    yaml::parse(text, "_")
}

pub fn parse(text: &str, locale: &str) -> Result<Parsed, String> {
    let mut p = all(text)?;
    p.entries.remove(VERSION);
    if !v2(text) {
        return Ok(p);
    }
    let suffix = format!(".{locale}");
    let strip = |m: std::collections::BTreeMap<String, String>| {
        m.into_iter()
            .filter_map(|(k, v)| Some((k.strip_suffix(&suffix)?.to_string(), v)))
            .collect()
    };
    Ok(Parsed {
        entries: strip(p.entries),
        notes: strip(p.notes),
        ..Default::default()
    })
}

pub fn write(existing: &str, cat: &Catalog, path: &Path) -> String {
    let Ok(p) = all(existing) else {
        return existing.to_string();
    };
    let mut full = Catalog::new("_");
    if v2(existing) {
        let suffix = format!(".{}", cat.locale);
        let mut fresh = vec![];
        for (k, v) in &p.entries {
            if !k.ends_with(&suffix) {
                full.entries.insert(k.clone(), v.clone());
            }
        }
        for (k, v) in &cat.entries {
            let leaf = format!("{k}{suffix}");
            if p.entries.contains_key(&leaf) {
                full.entries.insert(leaf, v.clone());
            } else {
                fresh.push((k, v));
            }
        }
        // New leaves go under their `key:` line (a top-level literal key, the v2 layout), never
        // into a nested `messages:\n  bye:` copy of a dotted key.
        let mut text = yaml::write(Some(existing), &full, path);
        for (k, v) in fresh {
            text = add_leaf(&text, k, &cat.locale, v);
        }
        return text;
    } else {
        full.entries = cat.entries.clone();
        if let Some(v) = p.entries.get(VERSION) {
            full.entries.insert(VERSION.into(), v.clone());
        }
    }
    yaml::write(Some(existing), &full, path)
}

/// `  locale: value` at the end of the top-level `key:` block (appended with it when missing).
fn add_leaf(text: &str, key: &str, locale: &str, v: &str) -> String {
    let plain = !v.is_empty()
        && v.trim() == v
        && !v.contains(['\n', '#', '"', '\''])
        && !v.contains(": ")
        && !v.starts_with(|c: char| "&*!|>%@`{[-?,".contains(c));
    let scalar = if plain {
        v.to_string()
    } else {
        format!(
            "\"{}\"",
            v.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let head = |l: &str| {
        let l = l.trim_end();
        l.strip_suffix(':').map(|k| k.trim_matches(['"', '\''])) == Some(key)
    };
    let Some(at) = lines.iter().position(|l| head(l)) else {
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        return out + &format!("{key}:\n  {locale}: {scalar}\n");
    };
    let mut end = at + 1;
    while end < lines.len() && (lines[end].starts_with([' ', '\t']) || lines[end].trim().is_empty())
    {
        end += 1;
    }
    while end > at + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let indent = lines
        .get(at + 1)
        .map(|l| &l[..l.len() - l.trim_start().len()])
        .filter(|i| !i.is_empty() && end > at + 1)
        .unwrap_or("  ");
    let mut out: String = lines[..end].concat();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out += &format!("{indent}{locale}: {scalar}\n");
    out + &lines[end..].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2: &str = "_version: 2\n# Accueil\nhello:\n  en: Hello, %{name}\n  fr: Bonjour, %{name}\nmessages.bye:\n  en: Bye\n  zh-CN: 再见\n";

    #[test]
    fn v2_reads_and_patches_one_locale() {
        let fr = parse(V2, "fr").unwrap();
        assert_eq!(fr.entries.len(), 1);
        assert_eq!(fr.entries["hello"], "Bonjour, %{name}");
        let en = parse(V2, "en").unwrap();
        assert_eq!(en.entries["messages.bye"], "Bye");
        let p = Path::new("locales/app.yml");
        let same = Catalog {
            locale: "en".into(),
            entries: en.entries.clone(),
        };
        assert_eq!(write(V2, &same, p), V2, "byte for byte");
        let mut c = Catalog::new("fr");
        c.entries.insert("hello".into(), "Salut, %{name}".into());
        c.entries.insert("messages.bye".into(), "Au revoir".into());
        let out = write(V2, &c, p);
        assert!(out.contains("  fr: Salut, %{name}\n"), "{out}");
        assert!(out.contains("  en: Hello, %{name}\n"), "{out}");
        assert_eq!(parse(&out, "fr").unwrap().entries, c.entries, "{out}");
        assert_eq!(
            parse(&out, "zh-CN").unwrap().entries["messages.bye"],
            "再见"
        );
        assert!(out.starts_with("_version: 2\n"));
        assert!(
            out.contains("messages.bye:\n  en: Bye\n  zh-CN: 再见\n  fr: Au revoir\n"),
            "{out}"
        );
        let mut d = Catalog::new("de");
        d.entries.insert("brand.new".into(), "Neu: %{x}".into());
        let out = write(V2, &d, p);
        assert!(out.ends_with("brand.new:\n  de: \"Neu: %{x}\"\n"), "{out}");
        assert_eq!(parse(&out, "de").unwrap().entries, d.entries);
    }

    #[test]
    fn v1_keeps_version() {
        let v1 = "_version: 1\nhello: Hello\n";
        let p = parse(v1, "en").unwrap();
        assert_eq!(p.entries.len(), 1);
        let mut c = Catalog::new("en");
        c.entries.insert("hello".into(), "Hi".into());
        assert_eq!(
            write(v1, &c, Path::new("locales/en.yml")),
            "_version: 1\nhello: Hi\n"
        );
    }
}
