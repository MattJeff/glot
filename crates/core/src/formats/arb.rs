//! Flutter ARB (`app_en.arb`): flat ICU messages, `@key` metadata (description, placeholders)
//! becomes translator notes, `@@locale` and other `@@` globals are kept.
use serde_json::{Map, Value};

use super::Parsed;
use crate::catalog::Catalog;

fn object(text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str(text).map_err(|e| e.to_string())? {
        Value::Object(m) => Ok(m),
        _ => Err("ARB root must be a JSON object".into()),
    }
}

pub fn parse(text: &str) -> Result<Parsed, String> {
    let m = object(text)?;
    let mut p = Parsed::default();
    for (k, v) in &m {
        if k.starts_with('@') {
            continue;
        }
        let Some(s) = v.as_str() else { continue };
        p.entries.insert(k.clone(), s.to_string());
        if let Some(meta) = m.get(&format!("@{k}")) {
            let mut note = meta
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(ph) = meta.get("placeholders").and_then(Value::as_object) {
                let list: Vec<String> = ph
                    .iter()
                    .map(|(name, d)| {
                        let attr = |f| d.get(f).and_then(Value::as_str);
                        match (attr("type"), attr("example")) {
                            (Some(t), Some(e)) => format!("{{{name}}}: {t}, e.g. {e}"),
                            (Some(t), None) => format!("{{{name}}}: {t}"),
                            (None, Some(e)) => format!("{{{name}}}: e.g. {e}"),
                            _ => format!("{{{name}}}"),
                        }
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
                p.notes.insert(k.clone(), note);
            }
        }
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let mut m = existing.and_then(|t| object(t).ok()).unwrap_or_else(|| {
        let mut m = Map::new();
        m.insert("@@locale".into(), Value::String(cat.locale.clone()));
        m
    });
    let gone: Vec<String> = m
        .keys()
        .filter(|k| !k.starts_with('@') && !cat.entries.contains_key(*k))
        .cloned()
        .collect();
    for k in gone {
        m.shift_remove(&k);
        m.shift_remove(&format!("@{k}"));
    }
    for (k, v) in &cat.entries {
        // Existing keys keep their position (preserve_order); new ones are appended.
        m.insert(k.clone(), Value::String(v.clone()));
    }
    let mut out = serde_json::to_string_pretty(&Value::Object(m)).expect("strings serialize");
    if existing.is_none_or(|t| t.ends_with('\n')) {
        out.push('\n');
    }
    out
}
