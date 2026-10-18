//! JSON shapes beyond `key => string`:
//! - plural sub-objects (easy_localization, slang, i18n-js, go-i18n): `{"inbox": {"one": …, "other": …}}`
//!   is one message `{_count, plural, one {…} other {…}}` in memory, re-split into the target
//!   locale's CLDR categories on write (the `_count` variable marks it: an ICU string written by
//!   hand with `{count, plural, …}` stays a string);
//! - message objects (`Format::JsonMessage`): FormatJS extracted `{"id": {"defaultMessage", "description"}}`
//!   and WebExtensions `_locales/{l}/messages.json` `{"key": {"message", "description", "placeholders"}}`:
//!   only the message is translated, `description` (and placeholder examples) become translator
//!   notes, everything else is kept. Chrome `$NAME$` placeholders are `{NAME}` in memory, so the
//!   ICU validator checks them, and new target entries get the source's `placeholders`.
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{Parsed, folded_choice, icu_choice, whole_choice};
use crate::Key;
use crate::catalog::Catalog;

const CATS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
const VAR: &str = "_count";

// ---------------------------------------------------------------- plural sub-objects

/// Fold `parent{sep}<category>` families (and nothing else under `parent`) into one ICU plural.
/// Needs `other` plus another category, or a lone `other` in locales with no other category (ja).
/// `zero` is "count is 0" (`=0`) unless the locale has a CLDR `zero`.
pub(crate) fn fold_plurals(entries: &mut BTreeMap<Key, String>, sep: &str, locale: &str) {
    if sep.is_empty() {
        return;
    }
    let cats = crate::plural::categories(locale);
    let only_other = cats == [crate::plural::PluralCategory::Other];
    let cldr_zero = cats.contains(&crate::plural::PluralCategory::Zero);
    let mut fams: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut not_plural: BTreeSet<&str> = BTreeSet::new();
    for k in entries.keys() {
        let Some((parent, last)) = k.rsplit_once(sep) else {
            continue;
        };
        if CATS.contains(&last) {
            fams.entry(parent).or_default().push(last);
        } else {
            not_plural.insert(parent);
        }
        // `a.b.one` under `a`: `a` is not a family.
        let mut p = parent;
        while let Some((up, _)) = p.rsplit_once(sep) {
            not_plural.insert(up);
            p = up;
        }
    }
    let mut folded = vec![];
    for (parent, mut members) in fams {
        if not_plural.contains(parent)
            || entries.contains_key(parent)
            || !members.contains(&"other")
            || (members.len() < 2 && !only_other)
        {
            continue;
        }
        members.sort_by_key(|c| CATS.iter().position(|x| x == c));
        let forms: Vec<(String, String)> = members
            .iter()
            .map(|c| {
                let sel = if *c == "zero" && !cldr_zero { "=0" } else { c };
                (
                    sel.to_string(),
                    entries[&format!("{parent}{sep}{c}")].clone(),
                )
            })
            .collect();
        let text = icu_choice(VAR, "plural", &forms);
        // Branch text with unbalanced braces would not survive the round trip.
        if whole_choice(&text).map(|c| c.forms) == Some(forms) {
            folded.push((
                parent.to_string(),
                members.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                text,
            ));
        }
    }
    for (parent, members, text) in folded {
        for c in members {
            entries.remove(&format!("{parent}{sep}{c}"));
        }
        entries.insert(parent, text);
    }
}

/// Inverse of [`fold_plurals`], in write order: each `{_count, plural, …}` message becomes
/// `key{sep}<category>` entries (CLDR order, `=0` => `zero`).
pub(crate) fn unfold_plurals(entries: Vec<(Key, String)>, sep: &str) -> Vec<(Key, String)> {
    let mut out = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        // Text around the plural (`pre {…} post`) goes into every branch, as in YAML / Android.
        let choice = folded_choice(&v).filter(|c| {
            c.var == VAR
                && c.kind == "plural"
                && c.forms
                    .iter()
                    .all(|(f, _)| f == "=0" || CATS.contains(&f.as_str()))
        });
        match choice {
            Some(c) if !sep.is_empty() => {
                let mut forms: BTreeMap<usize, (String, String)> = BTreeMap::new();
                for (f, text) in c.forms {
                    let cat = if f == "=0" { "zero" } else { f.as_str() };
                    let pos = CATS.iter().position(|x| *x == cat).expect("filtered");
                    // A CLDR `zero` form wins over `=0`.
                    if f != "=0" || !forms.contains_key(&pos) {
                        forms.insert(pos, (format!("{k}{sep}{cat}"), text));
                    }
                }
                out.extend(forms.into_values());
            }
            _ => out.push((k, v)),
        }
    }
    out
}

// ---------------------------------------------------------------- message objects

fn object(text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str(text).map_err(|e| e.to_string())? {
        Value::Object(m) => Ok(m),
        _ => Err("catalog root must be a JSON object".into()),
    }
}

/// `_locales/{locale}/messages.json` (WebExtensions).
pub(crate) fn is_webext(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == "messages.json")
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|n| n == "_locales")
}

/// The message field of an entry object.
fn field(o: &Map<String, Value>) -> Option<(&'static str, &str)> {
    ["message", "defaultMessage"]
        .into_iter()
        .find_map(|f| Some((f, o.get(f)?.as_str()?)))
}

/// Lowercased placeholder names declared by a Chrome entry.
fn declared(o: &Map<String, Value>) -> BTreeSet<String> {
    o.get("placeholders")
        .and_then(Value::as_object)
        .map(|p| p.keys().map(|k| k.to_ascii_lowercase()).collect())
        .unwrap_or_default()
}

/// Chrome `$NAME$` => `{NAME}` for declared names (`$$` is a literal `$`).
fn from_chrome(s: &str, names: &BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(a) = after.strip_prefix('$') {
            out.push_str("$$");
            rest = a;
            continue;
        }
        let n = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '@'))
            .unwrap_or(after.len());
        if n > 0 && after[n..].starts_with('$') && names.contains(&after[..n].to_ascii_lowercase())
        {
            out.push_str(&format!("{{{}}}", &after[..n]));
            rest = &after[n + 1..];
        } else {
            out.push('$');
            rest = after;
        }
    }
    out + rest
}

/// Inverse of [`from_chrome`].
fn to_chrome(s: &str, names: &BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(n) if names.contains(&after[..n].to_ascii_lowercase()) => {
                out.push_str(&format!("${}$", &after[..n]));
                rest = &after[n + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out + rest
}

pub(crate) fn parse(text: &str, path: &Path) -> Result<Parsed, String> {
    let webext = is_webext(path);
    let mut p = Parsed::default();
    for (k, v) in object(text)? {
        let Some(o) = v.as_object() else { continue };
        let Some((_, msg)) = field(o) else { continue };
        let names = declared(o);
        p.entries.insert(
            k.clone(),
            if webext {
                from_chrome(msg, &names)
            } else {
                msg.to_string()
            },
        );
        let mut note = o
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(ph) = o.get("placeholders").and_then(Value::as_object) {
            let list: Vec<String> = ph
                .iter()
                .map(|(name, d)| match d.get("example").and_then(Value::as_str) {
                    Some(e) => format!("{{{name}}} e.g. {e}"),
                    None => format!("{{{name}}}"),
                })
                .collect();
            if !list.is_empty() {
                if !note.is_empty() {
                    note += " ";
                }
                note += &format!("(placeholders: {})", list.join("; "));
            }
        }
        if !note.is_empty() {
            p.notes.insert(k, note);
        }
    }
    Ok(p)
}

/// The source locale's file of a WebExtension (`default_locale` of `manifest.json`, else `en`).
fn webext_source(path: &Path) -> Option<PathBuf> {
    let locales = path.parent()?.parent()?;
    let manifest = std::fs::read_to_string(locales.parent()?.join("manifest.json")).ok();
    let default = manifest
        .and_then(|m| serde_json::from_str::<Value>(&m).ok())
        .and_then(|m| m.get("default_locale")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "en".into());
    let src = locales.join(default).join("messages.json");
    (src != path).then_some(src)
}

pub(crate) fn write(existing: Option<&str>, cat: &Catalog, path: &Path) -> String {
    let webext = is_webext(path);
    let mut m = existing.and_then(|t| object(t).ok()).unwrap_or_default();
    // ponytail: the writer reads the source file itself for the `placeholders` new target entries need.
    let source = if webext {
        webext_source(path)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| object(&t).ok())
            .unwrap_or_default()
    } else {
        Map::new()
    };
    let name = m
        .values()
        .filter_map(Value::as_object)
        .find_map(|o| field(o).map(|(f, _)| f))
        .unwrap_or(if webext { "message" } else { "defaultMessage" });
    m.retain(|k, v| !v.is_object() || cat.entries.contains_key(k));
    for (k, v) in &cat.entries {
        let entry = m
            .entry(k.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let o = entry.as_object_mut().expect("object");
        let ph = source
            .get(k)
            .and_then(|s| s.get("placeholders"))
            .filter(|_| !o.contains_key("placeholders"))
            .cloned();
        let mut names = declared(o);
        names.extend(
            ph.iter()
                .flat_map(Value::as_object)
                .flat_map(|m| m.keys())
                .map(|k| k.to_ascii_lowercase()),
        );
        let field = field(o).map_or(name, |(f, _)| f);
        let text = if webext {
            to_chrome(v, &names)
        } else {
            v.clone()
        };
        o.insert(field.into(), Value::String(text));
        if let Some(ph) = ph {
            o.insert("placeholders".into(), ph);
        }
    }
    let mut out = serde_json::to_string_pretty(&Value::Object(m)).expect("strings serialize");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glot-json-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entries(pairs: &[(&str, &str)]) -> BTreeMap<Key, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn plural_objects_fold_and_unfold() {
        let mut e = entries(&[
            ("money.zero", "none"),
            ("money.one", "one"),
            ("money.other", "{} many"),
            ("menu.one", "Option one"),
            ("menu.label", "Menu"),
            ("solo.other", "x"),
        ]);
        fold_plurals(&mut e, ".", "en");
        assert_eq!(
            e["money"],
            "{_count, plural, =0 {none} one {one} other {{} many}}"
        );
        // Not a family: a non-category sibling, or a lone `other` in a locale with `one`.
        assert!(e.contains_key("menu.one") && e.contains_key("solo.other"));
        let out = unfold_plurals(e.clone().into_iter().collect(), ".");
        assert_eq!(
            out.iter()
                .filter(|(k, _)| k.starts_with("money"))
                .cloned()
                .collect::<Vec<_>>(),
            [
                ("money.zero".to_string(), "none".to_string()),
                ("money.one".into(), "one".into()),
                ("money.other".into(), "{} many".into())
            ]
        );
        // ja: a lone `other` is the whole family; a hand-written ICU string stays a string.
        let mut ja = entries(&[("n.other", "{} 件")]);
        fold_plurals(&mut ja, ".", "ja");
        assert_eq!(ja["n"], "{_count, plural, other {{} 件}}");
        let pre = vec![(
            "k".to_string(),
            "» {_count, plural, one {a} other {b}}".to_string(),
        )];
        assert_eq!(
            unfold_plurals(pre, ".")[0],
            ("k.one".to_string(), "» a".to_string())
        );
        let icu = vec![(
            "k".to_string(),
            "{count, plural, one {#} other {#}}".to_string(),
        )];
        assert_eq!(unfold_plurals(icu.clone(), "."), icu);
    }

    #[test]
    fn json_catalog_plural_objects_round_trip() {
        let dir = tmp("plural");
        let p = dir.join("en.json");
        let src = "{\n  \"hello\": \"Hello {}\",\n  \"money\": {\n    \"zero\": \"none\",\n    \"one\": \"one\",\n    \"other\": \"{} many\"\n  }\n}\n";
        std::fs::write(&p, src).unwrap();
        let c = Catalog::read(&p, "en", crate::catalog::Format::Auto).unwrap();
        assert_eq!(
            c.entries["money"],
            "{_count, plural, =0 {none} one {one} other {{} many}}"
        );
        c.write(&p, crate::catalog::Format::Auto).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), src);
        // Polish target: categories of the locale, written as a sub-object.
        let mut pl = Catalog::new("pl");
        pl.entries.insert(
            "money".into(),
            "{_count, plural, one {jeden} few {kilka} many {wiele} other {inne}}".into(),
        );
        let q = dir.join("pl.json");
        pl.write(&q, crate::catalog::Format::Auto).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&q).unwrap()).unwrap();
        assert_eq!(v["money"]["few"], "kilka");
        assert_eq!(
            Catalog::read(&q, "pl", crate::catalog::Format::Auto).unwrap(),
            pl
        );
    }

    #[test]
    fn formatjs_extracted() {
        let path = Path::new("lang/en.json");
        let src = r#"{"a1b2c3":{"defaultMessage":"Hello {name}","description":"greeting"}}"#;
        let p = parse(src, path).unwrap();
        assert_eq!(p.entries["a1b2c3"], "Hello {name}");
        assert_eq!(p.notes["a1b2c3"], "greeting");
        // Target: the message is translated, the description kept as is.
        let mut fr = Catalog::new("fr");
        fr.entries.insert("a1b2c3".into(), "Bonjour {name}".into());
        let out = write(Some(src), &fr, Path::new("lang/fr.json"));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["a1b2c3"]["defaultMessage"], "Bonjour {name}");
        assert_eq!(v["a1b2c3"]["description"], "greeting");
        let new = write(None, &fr, Path::new("lang/fr.json"));
        assert_eq!(
            new,
            "{\n  \"a1b2c3\": {\n    \"defaultMessage\": \"Bonjour {name}\"\n  }\n}\n"
        );
        assert_eq!(
            parse(
                &write(
                    Some(src),
                    &Catalog {
                        locale: "en".into(),
                        entries: p.entries.clone()
                    },
                    path
                ),
                path
            )
            .unwrap(),
            p
        );
    }

    #[test]
    fn webext_messages() {
        let dir = tmp("webext");
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"manifest_version":3,"default_locale":"en"}"#,
        )
        .unwrap();
        let en = dir.join("_locales/en/messages.json");
        std::fs::create_dir_all(en.parent().unwrap()).unwrap();
        let src = r#"{"hi":{"message":"Hello $USER$, $$5","description":"d","placeholders":{"user":{"content":"$1","example":"Bob"}}}}"#;
        std::fs::write(&en, src).unwrap();
        let p = parse(src, &en).unwrap();
        assert_eq!(p.entries["hi"], "Hello {USER}, $$5");
        assert_eq!(p.notes["hi"], "d (placeholders: {user} e.g. Bob)");
        let fr = dir.join("_locales/fr/messages.json");
        let mut cat = Catalog::new("fr");
        cat.entries
            .insert("hi".into(), "Bonjour {USER}, $$5".into());
        let out = write(None, &cat, &fr);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hi"]["message"], "Bonjour $USER$, $$5");
        assert_eq!(v["hi"]["placeholders"]["user"]["content"], "$1");
        assert!(v["hi"].get("description").is_none());
        // Source round trip is byte-stable modulo pretty-printing.
        let back = write(
            Some(src),
            &Catalog {
                locale: "en".into(),
                entries: p.entries.clone(),
            },
            &en,
        );
        assert_eq!(
            serde_json::from_str::<Value>(&back).unwrap(),
            serde_json::from_str::<Value>(src).unwrap()
        );
    }
}
