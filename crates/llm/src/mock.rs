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

/// `mock-noisy` (glot-bench): the mock, but a seeded share of answers is broken the way real models
/// break ICU (lost/invented placeholder, dropped plural branch, translated keyword, broken tag or
/// brace). Items sent back with `issues_to_fix` break 4x less often, so retries can recover.
/// Seed: `GLOT_MOCK_SEED` (default 42); rate: `GLOT_MOCK_NOISE` (default 0.15).
pub struct NoisyMock {
    pub seed: u64,
    pub rate: f64,
}

impl NoisyMock {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse().ok());
        Self {
            seed: env("GLOT_MOCK_SEED").unwrap_or(42.0) as u64,
            rate: env("GLOT_MOCK_NOISE").unwrap_or(0.15),
        }
    }
}

#[async_trait]
impl Provider for NoisyMock {
    fn name(&self) -> &str {
        "mock-noisy"
    }

    async fn complete(&self, req: CompletionRequest) -> Result<Completion> {
        use std::hash::{Hash, Hasher};
        let payload = crate::translate::extract_payload(&req.user).unwrap_or(Value::Null);
        let model = req.model.clone();
        let mut c = MockProvider.complete(req).await?;
        let Ok(mut v) = serde_json::from_str::<Value>(&c.text) else {
            return Ok(c);
        };
        let Some(tr) = v.get_mut("translations").and_then(Value::as_object_mut) else {
            return Ok(c);
        };
        for item in payload["items"].as_array().into_iter().flatten() {
            let Some(key) = item["key"].as_str() else {
                continue;
            };
            let Some(Value::String(s)) = tr.get_mut(key) else {
                continue;
            };
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (self.seed, &model, key, item.get("issues_to_fix").is_some()).hash(&mut h);
            let h = h.finish();
            let rate = if item.get("issues_to_fix").is_some() {
                self.rate / 4.0
            } else {
                self.rate
            };
            if (h % 10_000) as f64 / 10_000.0 < rate {
                *s = corrupt(s, h >> 16);
            }
        }
        c.text = v.to_string();
        Ok(c)
    }
}

/// One realistic breakage, picked by `h` among those that apply to `s`.
pub fn corrupt(s: &str, h: u64) -> String {
    let drop_arg = |s: &str| {
        let a = s.find('{')?;
        let b = a + s[a..].find('}')?;
        let inner = &s[a + 1..b];
        (!inner.is_empty() && inner.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .then(|| format!("{}{}", &s[..a], &s[b + 1..]))
    };
    let drop_branch = |s: &str| {
        let i = [" one {", " few {", " many {", " two {", " zero {"]
            .iter()
            .find_map(|p| s.find(p))?;
        let open = i + s[i..].find('{')?;
        let close = matching(s, open)?;
        Some(format!("{}{}", &s[..i], &s[close + 1..]))
    };
    let keyword = |s: &str| {
        ["plural,", "select,", "selectordinal,", "other {"]
            .iter()
            .find(|k| s.contains(**k))
            .map(|k| {
                s.replacen(
                    k,
                    &k.replace("plural", "pluriel")
                        .replace("select", "choix")
                        .replace("other", "autre"),
                    1,
                )
            })
    };
    let tag = |s: &str| {
        let a = s.find("</")?;
        let b = a + s[a..].find('>')?;
        Some(format!("{}{}", &s[..a], &s[b + 1..]))
    };
    let brace = |s: &str| s.rfind('}').map(|i| format!("{}{}", &s[..i], &s[i + 1..]));
    let invent = |s: &str| Some(format!("{s} {{count}}"));
    type Breaker<'a> = &'a dyn Fn(&str) -> Option<String>;
    let all: [Breaker; 6] = [&drop_arg, &drop_branch, &keyword, &tag, &brace, &invent];
    (0..all.len())
        .find_map(|k| all[(h as usize + k) % all.len()](s))
        .unwrap_or_else(|| s.to_string())
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
    if p["task"] == "culture" {
        return crate::culture::mock(p);
    }
    if p["task"] == "panel" || p["task"] == "panel_judge" {
        return crate::panel::mock(p);
    }
    if p["task"] == "keys" {
        return crate::i18nize::mock_keys(p);
    }
    if p["task"] == "copy_variants" {
        return crate::copywriting::mock(p);
    }
    if p["task"].as_str().is_some_and(|t| t.starts_with("store_")) {
        return crate::store::mock(p);
    }
    if p["task"] == "seo_keywords" {
        return crate::seo::mock(p);
    }
    if p["task"] == "rewrite" {
        return crate::rewrite::mock(p);
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
    fn noisy_breaks() {
        let s = "Hi <b>{name}</b>, {n, plural, one {# file} other {# files}}";
        for h in 0..6 {
            assert_ne!(super::corrupt(s, h), s, "{h}");
        }
        assert_eq!(super::corrupt("Save", 0), "Save {count}");
    }

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
