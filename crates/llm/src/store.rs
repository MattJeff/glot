//! App Store / Google Play listings (`glot labs store`): the prompts and the mock. Same idea as
//! `seo.rs`: keywords are what natives type in that store and country, not a translation of the
//! English ones. Limits are checked by the CLI's deterministic validator, never trusted from here.
use serde_json::{Value, json};

use crate::{
    Result,
    provider::{CompletionRequest, Provider},
    translate::{Usage, parse_json, payload_block},
};

const LISTING: &str = "You are a native App Store Optimization (ASO) copywriter. Localize an app store listing for the target locale's store and country. \
Rules: (1) `keywords` is keyword RESEARCH, not translation: list 15-30 single words (or very short terms for languages without spaces) that people in that country really type in the App Store / Google Play to find this kind of app, most relevant first, each with a short `why` (search intent, local wording, volume). \
Never use competitor or brand names (see `avoid_brands`), never repeat words already in the name or subtitle (the store indexes them already), no singular+plural pairs. \
(2) name, subtitle, promotional_text, description, short_description: natural native copy, keep the app's brand name unchanged, weave in the best keywords naturally, keep facts (no invented features, prices or claims). \
Each field must fit `limits` (characters; keywords in UTF-8 bytes once joined with commas). If `violations` is given, rewrite only those fields shorter. \
Output only one JSON object: {\"name\": \"\", \"subtitle\": \"\", \"promotional_text\": \"\", \"description\": \"\", \"short_description\": \"\", \"keywords\": [{\"term\": \"\", \"why\": \"\"}]}";

const DRAFT: &str = "You are an App Store Optimization (ASO) copywriter. From the app's name, category and the strings of its UI, \
draft the source-language store listing: subtitle, promotional_text, description (benefits first, short paragraphs, only features the UI strings show), \
short_description (Google Play), and `keywords` (15-30 single words people type to find such an app, most relevant first, each with a `why`; no brand names, none already in the name). \
Each field must fit `limits`. Output only one JSON object: {\"subtitle\": \"\", \"promotional_text\": \"\", \"description\": \"\", \"short_description\": \"\", \"keywords\": [{\"term\": \"\", \"why\": \"\"}]}";

const NOTES: &str = "You write App Store / Google Play release notes (\"What's New\") for end users in the target locale. \
Input: developer changes (commit subjects or changelog lines). Keep only what a user notices, group it, write 2-6 short bullets (\"• \"), benefits not internals, \
no commit hashes, ticket ids or developer jargon, no invented changes. Must fit `max_chars`. Output only one JSON object: {\"notes\": \"\"}";

/// One call for `payload["task"]` (`store_listing`, `store_draft`, `store_release_notes`).
pub async fn ask(provider: &dyn Provider, model: &str, payload: &Value) -> Result<(Value, Usage)> {
    let system = match payload["task"].as_str() {
        Some("store_draft") => DRAFT,
        Some("store_release_notes") => NOTES,
        _ => LISTING,
    };
    let c = provider
        .complete(CompletionRequest {
            model: model.into(),
            system: system.into(),
            user: payload_block(payload),
            max_tokens: 8_000,
            json: true,
            images: vec![],
        })
        .await?;
    Ok((
        parse_json(&c.text).unwrap_or(Value::Null),
        Usage::of_completion(&c),
    ))
}

/// Mock: fields are `⟦locale⟧ source`, so a source near its limit overflows once and the retry
/// (with `violations`) has to shorten it; keywords are the source ones plus the name's words and
/// description words, so the validator has duplicates to drop.
pub(crate) fn mock(p: &Value) -> Value {
    let loc = p["target_locale"].as_str().unwrap_or("xx");
    let tr = |s: &str| {
        if s.trim().is_empty() {
            String::new()
        } else {
            crate::mock::fake_translate(s, loc, &[])
        }
    };
    let shorten = |field: &str, s: String| match p["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["field"] == field)
    {
        Some(v) => s
            .chars()
            .take(v["max"].as_u64().unwrap_or(0) as usize)
            .collect(),
        None => s,
    };
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 4)
            .map(str::to_lowercase)
            .collect()
    };
    match p["task"].as_str() {
        Some("store_release_notes") => {
            let max = p["max_chars"].as_u64().unwrap_or(500) as usize;
            let notes: Vec<String> = p["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| Some(format!("• {}", tr(c.as_str()?))))
                .collect();
            json!({ "notes": notes.join("\n").chars().take(max).collect::<String>() })
        }
        Some("store_draft") => {
            let strings: Vec<&str> = p["app_strings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let first = |max: usize| {
                strings
                    .iter()
                    .find(|s| (10..=max).contains(&s.chars().count()))
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            };
            let mut kw: Vec<String> = vec![];
            for w in strings.iter().flat_map(|s| words(s)) {
                if !kw.contains(&w) && kw.len() < 25 {
                    kw.push(w);
                }
            }
            json!({
                "subtitle": first(30), "short_description": first(80), "promotional_text": first(170),
                "description": strings.iter().take(20).copied().collect::<Vec<_>>().join("\n"),
                "keywords": kw.iter().map(|t| json!({"term": t, "why": "frequent in the app's UI"})).collect::<Vec<_>>(),
            })
        }
        _ => {
            let s = &p["source"];
            let f = |k: &str| shorten(k, tr(s[k].as_str().unwrap_or_default()));
            let mut kw: Vec<String> = s["keywords"]
                .as_str()
                .unwrap_or_default()
                .split(',')
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect();
            kw.extend(words(s["name"].as_str().unwrap_or_default()));
            kw.extend(words(s["description"].as_str().unwrap_or_default()));
            json!({
                "name": f("name"), "subtitle": f("subtitle"), "promotional_text": f("promotional_text"),
                "description": f("description"), "short_description": f("short_description"),
                "keywords": kw.iter().map(|t| json!({"term": t, "why": format!("⟦{loc}⟧ searched as \"{t}\"")})).collect::<Vec<_>>(),
            })
        }
    }
}
