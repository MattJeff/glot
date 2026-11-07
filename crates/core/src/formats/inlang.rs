//! inlang message format (Paraglide JS, `@inlang/plugin-message-format` v4):
//! `messages/{locale}.json` next to `project.inlang/settings.json`.
//!
//! - `$schema` is metadata, kept verbatim; nested objects are dotted keys (`nav.home`).
//! - Variant messages (`"items": [{"declarations", "selectors", "match"}]`): only the `match`
//!   values are text. One plural selector (`local n = count: plural`) folds into
//!   `{count, plural, one {…} other {…}}`, so the validator checks the target's CLDR categories
//!   and new categories (pl `few`/`many`) are written as `n=few` matches. Any other variant
//!   (several selectors, `select`) is one entry per match: `items#gender=female`.
//! - Writing keeps the file's key order and indentation; `declarations`/`selectors` are never
//!   touched, and a new target message copies them from the base locale's file.
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use super::{Parsed, folded_choice, icu_choice, whole_choice};
use crate::catalog::Catalog;

const SCHEMA: &str = "$schema";

/// The base locale's file for `path`, when an ancestor holds `project.inlang/settings.json`
/// with the message-format plugin.
fn settings(path: &Path) -> Option<(PathBuf, Value)> {
    path.ancestors().skip(1).take(6).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("project.inlang/settings.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        v.get("plugin.inlang.messageFormat")?;
        Some((dir.to_path_buf(), v))
    })
}

/// A JSON catalog of a Paraglide / inlang project.
pub(crate) fn is_inlang(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "json") && settings(path).is_some()
}

fn source_file(path: &Path) -> Option<PathBuf> {
    let (root, s) = settings(path)?;
    let base = s
        .get("baseLocale")
        .or_else(|| s.get("sourceLanguageTag"))?
        .as_str()?;
    let pat = &s["plugin.inlang.messageFormat"]["pathPattern"];
    let pats: Vec<&str> = match pat {
        Value::String(p) => vec![p.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    pats.into_iter().find_map(|p| {
        let f = root.join(
            p.trim_start_matches("./")
                .replace("{locale}", base)
                .replace("{languageTag}", base),
        );
        (f != path && f.is_file()).then_some(f)
    })
}

fn object(text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str(text).map_err(|e| e.to_string())? {
        Value::Object(m) => Ok(m),
        _ => Err("catalog root must be a JSON object".into()),
    }
}

fn join(prefix: &str, k: &str) -> String {
    if prefix.is_empty() {
        k.to_string()
    } else {
        format!("{prefix}.{k}")
    }
}

/// `local countPlural = count: plural` for the single selector => (`countPlural`, `count`).
fn plural_selector(v: &Map<String, Value>) -> Option<(String, String)> {
    let sel = match v.get("selectors")?.as_array()?.as_slice() {
        [s] => s.as_str()?.to_string(),
        _ => return None,
    };
    v.get("declarations")?.as_array()?.iter().find_map(|d| {
        let rest = d.as_str()?.trim().strip_prefix("local ")?;
        let (name, expr) = rest.split_once('=')?;
        let (var, func) = expr.split_once(':')?;
        (name.trim() == sel && func.split_whitespace().next() == Some("plural"))
            .then(|| (sel.clone(), var.trim().trim_start_matches('$').to_string()))
    })
}

/// ICU selector for a match value: `one` => `one`, `*` / `other` => `other`, `0` => `=0`.
fn icu_sel(m: &str) -> Option<String> {
    const CATS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
    match m {
        "*" => Some("other".into()),
        c if CATS.contains(&c) => Some(c.into()),
        n if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => Some(format!("={n}")),
        _ => None,
    }
}

/// The variant object of a message (`[{"match": …}]`).
fn variant(v: &Value) -> Option<&Map<String, Value>> {
    match v.as_array()?.as_slice() {
        [Value::Object(o)] if o.get("match").is_some_and(Value::is_object) => Some(o),
        _ => None,
    }
}

/// The variant as one ICU plural, when it has a single plural selector.
fn fold(o: &Map<String, Value>) -> Option<String> {
    let (sel, var) = plural_selector(o)?;
    let mut forms = vec![];
    for (k, t) in o.get("match")?.as_object()? {
        let m = k.strip_prefix(&sel)?.strip_prefix('=')?;
        forms.push((icu_sel(m)?, t.as_str()?.to_string()));
    }
    let text = icu_choice(&var, "plural", &forms);
    // Unbalanced braces in a branch would not come back intact.
    (whole_choice(&text).map(|c| c.forms) == Some(forms)).then_some(text)
}

fn walk(m: &Map<String, Value>, prefix: &str, p: &mut Parsed) {
    for (k, v) in m {
        let key = join(prefix, k);
        match v {
            Value::String(_) if prefix.is_empty() && k == SCHEMA => {}
            Value::String(s) => {
                p.entries.insert(key, s.clone());
            }
            Value::Object(o) => walk(o, &key, p),
            Value::Array(_) => {
                let Some(o) = variant(v) else { continue };
                if let Some(icu) = fold(o) {
                    p.entries.insert(key, icu);
                } else {
                    for (mk, t) in o["match"].as_object().into_iter().flatten() {
                        if let Some(t) = t.as_str() {
                            p.entries.insert(format!("{key}#{mk}"), t.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn parse(text: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    walk(&object(text)?, "", &mut p);
    Ok(p)
}

/// `skeleton` (a variant object) with its `match` values taken from `cat` for message `key`;
/// `None` when `cat` has none of them.
fn fill(skeleton: &Map<String, Value>, key: &str, cat: &Catalog) -> Option<Value> {
    let mut o = skeleton.clone();
    if let Some((sel, _)) = plural_selector(skeleton) {
        // Text around the plural (`Du hast {n, plural, …}.`) goes into every branch.
        let c = folded_choice(cat.entries.get(key)?)?;
        // The file's own spelling of `other` (`*` in Paraglide's examples, `other` in inlang's).
        let other = skeleton["match"]
            .as_object()
            .and_then(|m| m.keys().find(|k| k.ends_with("=*")))
            .map_or("other", |_| "*");
        let mut m = Map::new();
        for (f, t) in c.forms {
            let f = match f.as_str() {
                "other" => other.to_string(),
                f => f.trim_start_matches('=').to_string(),
            };
            m.insert(format!("{sel}={f}"), Value::String(t));
        }
        o.insert("match".into(), Value::Object(m));
    } else {
        let mut any = false;
        for (mk, t) in o.get_mut("match")?.as_object_mut()? {
            if let Some(new) = cat.entries.get(&format!("{key}#{mk}")) {
                *t = Value::String(new.clone());
                any = true;
            }
        }
        if !any {
            return None;
        }
    }
    Some(Value::Array(vec![Value::Object(o)]))
}

/// Update `m` in place: changed values, removed messages dropped; returns the keys written.
fn update(m: &mut Map<String, Value>, prefix: &str, cat: &Catalog, seen: &mut Vec<String>) {
    let keys: Vec<String> = m.keys().cloned().collect();
    for k in keys {
        let key = join(prefix, &k);
        let v = m.get_mut(&k).expect("listed");
        let keep = match v {
            Value::String(_) if prefix.is_empty() && k == SCHEMA => true,
            Value::String(s) => match cat.entries.get(&key) {
                Some(new) => {
                    *s = new.clone();
                    seen.push(key);
                    true
                }
                None => false,
            },
            Value::Object(o) => {
                let was_empty = o.is_empty();
                update(o, &key, cat, seen);
                was_empty || !o.is_empty()
            }
            Value::Array(_) => match variant(v).cloned() {
                Some(sk) => match fill(&sk, &key, cat) {
                    Some(new) => {
                        *v = new;
                        seen.push(key);
                        true
                    }
                    None => false,
                },
                None => true,
            },
            _ => true,
        };
        if !keep {
            m.shift_remove(&k);
        }
    }
}

/// The value at dotted `key` in `m`.
fn at<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    match key.split_once('.') {
        None => m.get(key),
        Some((h, r)) => at(m.get(h)?.as_object()?, r),
    }
}

fn insert(m: &mut Map<String, Value>, key: &str, v: Value) {
    match key.split_once('.') {
        Some((h, r)) if m.get(h).is_none_or(Value::is_object) => {
            let child = m
                .entry(h)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .expect("object");
            insert(child, r, v);
        }
        _ => {
            m.insert(key.to_string(), v);
        }
    }
}

pub(crate) fn write(existing: Option<&str>, cat: &Catalog, path: &Path) -> String {
    let source = source_file(path)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| object(&t).ok())
        .unwrap_or_default();
    let mut m = existing.and_then(|t| object(t).ok()).unwrap_or_default();
    if m.is_empty()
        && let Some(s) = source.get(SCHEMA)
    {
        m.insert(SCHEMA.into(), s.clone());
    }
    let mut seen = vec![];
    update(&mut m, "", cat, &mut seen);
    for (k, v) in &cat.entries {
        let base = k.split_once('#').map_or(k.as_str(), |(b, _)| b);
        if seen.iter().any(|s| s == base) {
            continue;
        }
        match at(&source, base).and_then(variant) {
            Some(sk) => {
                if let Some(new) = fill(sk, base, cat) {
                    insert(&mut m, base, new);
                }
                seen.push(base.to_string());
            }
            None => insert(&mut m, k, Value::String(v.clone())),
        }
    }
    let indent = existing
        .and_then(|t| {
            t.lines().find_map(|l| {
                let ws = &l[..l.len() - l.trim_start().len()];
                (!ws.is_empty() && l.trim_start().starts_with('"')).then_some(ws)
            })
        })
        .unwrap_or("  ");
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    serde::Serialize::serialize(
        &Value::Object(m),
        &mut serde_json::Serializer::with_formatter(&mut out, fmt),
    )
    .expect("json values serialize");
    String::from_utf8(out).expect("utf-8") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = r#"{
  "$schema": "https://inlang.com/schema/inlang-message-format",
  "hello_world": "Hello, {name}!",
  "nav": {
    "home": "Home"
  },
  "items": [
    {
      "declarations": [
        "input count",
        "local countPlural = count: plural"
      ],
      "selectors": [
        "countPlural"
      ],
      "match": {
        "countPlural=one": "{count} item",
        "countPlural=*": "{count} items"
      }
    }
  ],
  "greet": [
    {
      "declarations": [
        "input gender"
      ],
      "selectors": [
        "gender"
      ],
      "match": {
        "gender=female": "She",
        "gender=*": "They"
      }
    }
  ]
}
"#;

    fn project(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glot-inlang-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("project.inlang")).unwrap();
        std::fs::create_dir_all(d.join("messages")).unwrap();
        std::fs::write(
            d.join("project.inlang/settings.json"),
            r#"{"baseLocale":"en","locales":["en","pl"],"plugin.inlang.messageFormat":{"pathPattern":"./messages/{locale}.json"}}"#,
        )
        .unwrap();
        std::fs::write(d.join("messages/en.json"), EN).unwrap();
        d
    }

    #[test]
    fn source_round_trip_is_byte_identical() {
        let d = project("rt");
        let en = d.join("messages/en.json");
        assert!(is_inlang(&en));
        let p = parse(EN).unwrap();
        assert_eq!(p.entries.len(), 5, "{:?}", p.entries);
        assert_eq!(
            p.entries["items"],
            "{count, plural, one {{count} item} other {{count} items}}"
        );
        assert_eq!(p.entries["greet#gender=female"], "She");
        assert!(!p.entries.contains_key(SCHEMA));
        let cat = Catalog {
            locale: "en".into(),
            entries: p.entries,
        };
        assert_eq!(write(Some(EN), &cat, &en), EN);
    }

    #[test]
    fn new_target_gets_the_skeleton_and_its_categories() {
        let d = project("pl");
        let pl = d.join("messages/pl.json");
        let mut cat = Catalog::new("pl");
        for (k, v) in [
            ("hello_world", "Cześć, {name}!"),
            ("nav.home", "Start"),
            (
                "items",
                "{count, plural, one {{count} rzecz} few {{count} rzeczy} many {{count} rzeczy} other {{count} rzeczy}}",
            ),
            ("greet#gender=female", "Ona"),
            ("greet#gender=*", "Oni"),
        ] {
            cat.entries.insert(k.into(), v.into());
        }
        let out = write(None, &cat, &pl);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v[SCHEMA], "https://inlang.com/schema/inlang-message-format");
        assert_eq!(v["nav"]["home"], "Start");
        let items = &v["items"][0];
        assert_eq!(items["selectors"][0], "countPlural");
        assert_eq!(
            items["declarations"][1],
            "local countPlural = count: plural"
        );
        assert_eq!(items["match"]["countPlural=few"], "{count} rzeczy");
        assert_eq!(items["match"]["countPlural=*"], "{count} rzeczy");
        assert_eq!(v["greet"][0]["match"]["gender=female"], "Ona");
        // Read back: same messages.
        assert_eq!(parse(&out).unwrap().entries, cat.entries);
        // A removed message disappears, the rest is untouched.
        cat.entries.remove("items");
        let out2 = write(Some(&out), &cat, &pl);
        assert!(!out2.contains("countPlural") && out2.contains("gender=female"));
    }
}
