//! Apple String Catalog (`.xcstrings`): one JSON file for every locale. Plural / device
//! variations become ICU (`{count, plural, …}`, `{device, select, …}`), substitutions
//! (`%#@name@`) become `{name, plural, …}` inline. Written in Xcode's layout (`"key" : value`).
use serde_json::{Map, Value, json};

use super::strings::find_token;
use super::{Choice, Parsed, choices, folded_choice, icu_choice, whole_choice};
use crate::catalog::Catalog;

fn source_language(root: &Value) -> &str {
    root.get("sourceLanguage")
        .and_then(Value::as_str)
        .unwrap_or("en")
}

pub fn parse(text: &str, locale: &str) -> Result<Parsed, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let src = source_language(&root);
    let mut p = Parsed::default();
    let Some(strings) = root.get("strings").and_then(Value::as_object) else {
        return Ok(p);
    };
    for (key, e) in strings {
        if e.get("shouldTranslate") == Some(&Value::Bool(false)) {
            continue;
        }
        if let Some(c) = e.get("comment").and_then(Value::as_str) {
            p.notes.insert(key.clone(), c.to_string());
        }
        match e.get("localizations").and_then(|l| l.get(locale)) {
            Some(l) => {
                let mut stale = false;
                p.entries.insert(key.clone(), loc_icu(l, &mut stale));
                if stale {
                    p.stale.insert(key.clone());
                }
            }
            // The key is the source text until a source localization overrides it.
            None if locale == src => {
                p.entries.insert(key.clone(), key.clone());
            }
            None => {}
        }
    }
    Ok(p)
}

/// A localization (string unit or variations, plus substitutions) as one ICU message.
fn loc_icu(l: &Value, stale: &mut bool) -> String {
    let mut s = node_icu(l, "count", stale);
    if let Some(subs) = l.get("substitutions").and_then(Value::as_object) {
        for (name, sub) in subs {
            if let Some((a, b)) = find_token(&s, name) {
                let icu = node_icu(sub, name, stale);
                s.replace_range(a..b, &icu);
            }
        }
    }
    s
}

fn node_icu(n: &Value, plural_var: &str, stale: &mut bool) -> String {
    if let Some(u) = n.get("stringUnit") {
        if u.get("state").and_then(Value::as_str) == Some("needs_review") {
            *stale = true;
        }
        return u
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    let Some((kind, forms)) = n
        .get("variations")
        .and_then(Value::as_object)
        .and_then(|v| v.iter().next())
    else {
        return String::new();
    };
    let mut forms: Vec<(String, String)> = forms
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, sub)| (k.clone(), node_icu(sub, plural_var, stale)))
        .collect();
    // Xcode stores categories alphabetically; ICU reads best in CLDR order.
    const ORDER: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
    if kind == "plural" {
        forms.sort_by_key(|(k, _)| ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()));
    }
    match kind.as_str() {
        "plural" => icu_choice(plural_var, "plural", &forms),
        other => icu_choice(other, "select", &forms),
    }
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let mut root: Value = existing
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_else(|| json!({"sourceLanguage": cat.locale, "strings": {}, "version": "1.0"}));
    let src = source_language(&root).to_string();
    let locale = cat.locale.as_str();
    let Some(strings) = root.get_mut("strings").and_then(Value::as_object_mut) else {
        return existing.unwrap_or_default().to_string();
    };
    // Removed keys: the whole string for the source locale, else this locale's localization.
    let gone: Vec<String> = strings
        .iter()
        .filter(|(k, e)| {
            !cat.entries.contains_key(*k) && e.get("shouldTranslate") != Some(&Value::Bool(false))
        })
        .map(|(k, _)| k.clone())
        .collect();
    for k in gone {
        if locale == src {
            strings.shift_remove(&k);
        } else if let Some(l) = strings[&k]
            .get_mut("localizations")
            .and_then(Value::as_object_mut)
        {
            l.shift_remove(locale);
        }
    }
    for (k, v) in &cat.entries {
        if !strings.contains_key(k) {
            insert_sorted(strings, k.clone(), json!({}));
        }
        let e = strings.get_mut(k).unwrap();
        let current = e.get("localizations").and_then(|l| l.get(locale));
        let current_text = match current {
            Some(l) => Some(loc_icu(l, &mut false)),
            None if locale == src => Some(k.clone()),
            None => None,
        };
        if current_text.as_ref() == Some(v) {
            continue;
        }
        let source_loc = e.get("localizations").and_then(|l| l.get(&src)).cloned();
        let node = icu_loc(v, source_loc.as_ref().or(current));
        let Some(obj) = e.as_object_mut() else {
            continue;
        };
        if !obj.contains_key("localizations") {
            insert_sorted(obj, "localizations".into(), json!({}));
        }
        let locs = obj["localizations"].as_object_mut().unwrap();
        if locs.contains_key(locale) {
            locs[locale] = node;
        } else {
            insert_sorted(locs, locale.into(), node);
        }
    }
    let mut out = String::new();
    xcode_json(&root, 0, &mut out);
    if existing.is_none_or(|t| t.ends_with('\n')) {
        out.push('\n');
    }
    out
}

/// ICU message => localization node. `like` provides substitution `argNum`/`formatSpecifier`.
fn icu_loc(v: &str, like: Option<&Value>) -> Value {
    let has_subs = like.is_some_and(|l| l.get("substitutions").is_some());
    if let Some(c) = whole_choice(v).or_else(|| folded_choice(v).filter(|_| !has_subs)) {
        return variations(&c);
    }
    let subs: Vec<Choice> = choices(v)
        .into_iter()
        .filter(|c| c.kind == "plural")
        .collect();
    if subs.is_empty() {
        return unit(v);
    }
    let mut value = v.to_string();
    let mut map = Map::new();
    for c in subs.iter().rev() {
        value.replace_range(c.start..c.end, &format!("%#@{}@", c.var));
    }
    for c in &subs {
        let old = like.and_then(|l| l.pointer(&format!("/substitutions/{}", c.var)));
        let get = |f: &str, d: Value| old.and_then(|o| o.get(f)).cloned().unwrap_or(d);
        map.insert(
            c.var.clone(),
            json!({
                "argNum": get("argNum", json!(1)),
                "formatSpecifier": get("formatSpecifier", json!("lld")),
                "variations": variations(c)["variations"].clone(),
            }),
        );
    }
    let mut node = unit(&value);
    node["substitutions"] = Value::Object(map);
    node
}

fn variations(c: &Choice) -> Value {
    let kind = if c.kind == "plural" {
        "plural"
    } else {
        c.var.as_str()
    };
    let mut forms: Vec<&(String, String)> = c
        .forms
        .iter()
        .filter(|(k, _)| !k.starts_with('='))
        .collect();
    forms.sort_by(|a, b| a.0.cmp(&b.0));
    let forms: Map<String, Value> = forms
        .into_iter()
        .map(|(k, t)| (k.clone(), icu_loc(t, None)))
        .collect();
    json!({ "variations": { kind: forms } })
}

fn unit(v: &str) -> Value {
    json!({"stringUnit": {"state": "translated", "value": v}})
}

/// Insert keeping an alphabetical map alphabetical (Xcode sorts keys); existing order is kept.
fn insert_sorted(map: &mut Map<String, Value>, key: String, value: Value) {
    let old = std::mem::take(map);
    let mut pending = Some((key, value));
    for (k, v) in old {
        if pending.as_ref().is_some_and(|(p, _)| *p < k) {
            let (p, pv) = pending.take().unwrap();
            map.insert(p, pv);
        }
        map.insert(k, v);
    }
    if let Some((p, pv)) = pending {
        map.insert(p, pv);
    }
}

/// Xcode's JSON layout: 2-space indent, `"key" : value`, empty objects as `{\n\n}`.
fn xcode_json(v: &Value, ind: usize, out: &mut String) {
    let pad = |n: usize| " ".repeat(n);
    match v {
        Value::Object(m) if m.is_empty() => {
            *out += &format!("{{\n\n{}}}", pad(ind));
        }
        Value::Object(m) => {
            out.push_str("{\n");
            for (i, (k, val)) in m.iter().enumerate() {
                *out += &format!("{}{} : ", pad(ind + 2), Value::String(k.clone()));
                xcode_json(val, ind + 2, out);
                out.push_str(if i + 1 < m.len() { ",\n" } else { "\n" });
            }
            *out += &format!("{}}}", pad(ind));
        }
        Value::Array(a) if a.is_empty() => out.push_str("[\n\n]"),
        Value::Array(a) => {
            out.push_str("[\n");
            for (i, val) in a.iter().enumerate() {
                out.push_str(&pad(ind + 2));
                xcode_json(val, ind + 2, out);
                out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
            }
            *out += &format!("{}]", pad(ind));
        }
        other => out.push_str(&other.to_string()),
    }
}
