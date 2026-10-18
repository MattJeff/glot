//! Deterministic offline provider: "translates" to `⟦locale⟧ text`, keeping placeholders, ICU syntax and tags.
//! It reads the JSON payload the pipeline embeds in the user prompt (see `translate::payload_block`).
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use glot_core::plural;

use crate::Result;
use crate::provider::{Completion, CompletionRequest, Provider};

pub struct MockProvider;

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn complete(&self, req: CompletionRequest) -> Result<Completion> {
        let mut payload = crate::translate::extract_payload(&req.user).unwrap_or(Value::Null);
        if payload.is_object() && payload.get("glossary").is_none() {
            payload["glossary"] = system_glossary(&req.system, payload["target_locale"].as_str());
        }
        let text = respond(&payload).to_string();
        Ok(Completion {
            input_tokens: ((req.system.len() + req.user.len()) / 4) as u32,
            output_tokens: (text.len() / 4) as u32,
            text,
            model: "mock".into(), // priced as free whatever model was asked
            ..Default::default()
        })
    }
}

/// Glossary lines of the translation system prompt: `- "term" => "translation" …`.
fn system_glossary(system: &str, locale: Option<&str>) -> Value {
    let terms: Vec<Value> = system
        .lines()
        .filter_map(|l| {
            let (term, rest) = l.strip_prefix("- \"")?.split_once("\" => \"")?;
            let tr = rest.split('"').next()?;
            Some(json!({"term": term, "translations": {locale?: tr}}))
        })
        .collect();
    Value::Array(terms)
}

fn respond(p: &Value) -> Value {
    let locale = p["target_locale"].as_str().unwrap_or("xx");
    let items = p["items"].as_array().cloned().unwrap_or_default();
    if p["task"] == "keys" {
        return crate::i18nize::mock_keys(p);
    }
    if p["task"] == "shorten" {
        return crate::visual::mock_shorten(p);
    }
    if p["task"] == "review" || p["task"] == "mine" {
        return crate::review::mock(p);
    }
    if p["task"] == "judge" {
        let verdicts: Map<String, Value> = items
            .iter()
            .filter_map(|i| {
                Some((
                    i["key"].as_str()?.to_string(),
                    json!({"errors": [], "suggestion": null}),
                ))
            })
            .collect();
        return json!({ "verdicts": verdicts });
    }
    let mut glossary = vec![];
    for t in p["glossary"].as_array().into_iter().flatten() {
        if let (Some(term), Some(tr)) = (t["term"].as_str(), t["translations"][locale].as_str()) {
            glossary.push((term.to_string(), tr.to_string()));
        }
    }
    let translations: Map<String, Value> = items
        .iter()
        .filter_map(|i| {
            let key = i["key"].as_str()?.to_string();
            // Rejected for its pixel budget: answer like a model would, with a shorter wording.
            if let Some(v) = crate::visual::mock_fit(i) {
                return Some((key, Value::String(v)));
            }
            Some((
                key,
                Value::String(fake_translate(i["source"].as_str()?, locale, &glossary)),
            ))
        })
        .collect();
    json!({ "translations": translations })
}

pub fn fake_translate(source: &str, locale: &str, glossary: &[(String, String)]) -> String {
    let mut s = source.to_string();
    for (term, tr) in glossary {
        s = s.replace(term.as_str(), tr);
    }
    let cats: Vec<&str> = plural::categories(locale)
        .iter()
        .map(|c| c.as_str())
        .collect();
    let s = rewrite(&s, &cats);
    let start = s.len() - s.trim_start().len();
    format!("{}⟦{locale}⟧ {}", &s[..start], &s[start..])
}

/// Index of the `}` closing the `{` at `open`.
fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Rewrite ICU plural blocks so they carry exactly the target locale's categories (+ explicit `=N`).
fn rewrite(s: &str, cats: &[&str]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(rel) = s[i..].find('{') {
        let open = i + rel;
        out.push_str(&s[i..open]);
        let Some(close) = matching(s, open) else {
            out.push_str(&s[open..]);
            return out;
        };
        out.push_str(&rewrite_arg(&s[open + 1..close], cats));
        i = close + 1;
    }
    out.push_str(&s[i..]);
    out
}

fn rewrite_arg(inner: &str, cats: &[&str]) -> String {
    let mut parts = inner.splitn(3, ',');
    let (arg, kind, rest) = (parts.next().unwrap_or(""), parts.next(), parts.next());
    let (Some(kind), Some(rest)) = (kind, rest) else {
        return format!("{{{inner}}}");
    };
    let k = kind.trim();
    if !matches!(k, "plural" | "select" | "selectordinal") {
        return format!("{{{inner}}}");
    }
    // Parse `selector {content}` pairs (plus an optional `offset:n`).
    let mut opts: Vec<(String, String)> = vec![];
    let mut prefix = String::new();
    let mut j = 0;
    while j < rest.len() {
        let Some(b) = rest[j..].find('{').map(|b| j + b) else {
            break;
        };
        let selector = rest[j..b].trim().to_string();
        let Some(e) = matching(rest, b) else { break };
        let content = rewrite(&rest[b + 1..e], cats);
        let selector = match selector.split_once(char::is_whitespace) {
            Some((off, sel)) if off.starts_with("offset:") => {
                prefix = format!("{off} ");
                sel.trim().to_string()
            }
            _ => selector,
        };
        opts.push((selector, content));
        j = e + 1;
    }
    let body: Vec<(String, String)> = if k == "plural" {
        let get = |c: &str| {
            opts.iter()
                .find(|(s, _)| s == c)
                .or_else(|| opts.iter().find(|(s, _)| s == "other"))
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        opts.iter()
            .filter(|(s, _)| s.starts_with('='))
            .cloned()
            .chain(cats.iter().map(|c| (c.to_string(), get(c))))
            .collect()
    } else {
        opts
    };
    let body: Vec<String> = body
        .into_iter()
        .map(|(s, c)| format!("{s} {{{c}}}"))
        .collect();
    format!("{{{arg},{kind}, {prefix}{}}}", body.join(" "))
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn rewrites_plural_categories() {
        let src = "Hi <b>{name}</b>, {n, plural, =0 {none} one {# file} other {# files}}";
        assert_eq!(
            rewrite(src, &["one", "many", "other"]),
            "Hi <b>{name}</b>, {n, plural, =0 {none} one {# file} many {# files} other {# files}}"
        );
        assert_eq!(
            rewrite("{n, plural, one {# x} other {# y}}", &["other"]),
            "{n, plural, other {# y}}"
        );
        assert_eq!(
            rewrite("{g, select, male {He} other {They}}", &["other"]),
            "{g, select, male {He} other {They}}"
        );
    }
}
