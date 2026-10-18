use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use glot_core::{Key, context::KeyContext};

use crate::{
    Result,
    provider::{CompletionRequest, Provider},
    translate::{Usage, parse_json, payload_block},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeItem {
    pub key: Key,
    pub source: String,
    pub target: String,
    pub context: KeyContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub key: Key,
    /// 0-100 (MQM-style: 100 - weighted error penalties).
    pub score: u8,
    /// Short explanation of the main problems, empty if none.
    pub notes: String,
    /// Optional better candidate.
    pub suggestion: Option<String>,
}

const SYSTEM: &str = "You are an expert localization quality reviewer applying the MQM framework to SaaS user-interface strings.\n\
For each item, compare `target` with `source` (use description/ui_role to understand the UI context) and list every translation error.\n\
Categories: accuracy (mistranslation, omission, addition, untranslated), fluency (grammar, spelling, punctuation, unnatural phrasing), \
terminology (inconsistent or wrong product terms), style (wrong register/tone, too long for UI), locale convention (numbers, dates, quotes, spacing).\n\
Severity: minor (does not hinder understanding), major (changes or obscures meaning, clearly unnatural), critical (wrong meaning, offensive, broken UI).\n\
Placeholders like {name}, {{name}}, ICU plural/select syntax and tags are code: do not report them unless they are broken.\n\
Do not report preferential changes. If there is no error, return an empty list.\n\
When there is at least one major or critical error, give a corrected `suggestion`, otherwise null.\n\
Output only one JSON object, no prose: {\"verdicts\": {\"<key>\": {\"errors\": [{\"category\": \"accuracy\", \"severity\": \"major\", \"description\": \"...\"}], \"suggestion\": null}}}";

fn penalty(severity: &str) -> u32 {
    match severity {
        "critical" => 25,
        "major" => 10,
        _ => 2,
    }
}

/// Judge prompt for one batch (also used by dry-run cost estimates).
pub(crate) fn request(
    model: &str,
    source_locale: &str,
    target_locale: &str,
    items: &[JudgeItem],
) -> CompletionRequest {
    let payload = json!({
        "task": "judge",
        "source_locale": source_locale,
        "target_locale": target_locale,
        "items": items.iter().map(|i| json!({
            "key": i.key, "source": i.source, "target": i.target,
            "description": i.context.description, "ui_role": i.context.ui_role,
        })).collect::<Vec<_>>(),
    });
    let user = format!(
        "Review these {} translations from `{source_locale}` to `{target_locale}`.\n\n{}",
        items.len(),
        payload_block(&payload)
    );
    CompletionRequest {
        model: model.into(),
        system: SYSTEM.into(),
        user,
        max_tokens: 16_000,
        json: true,
        images: vec![],
    }
}

/// LLM-as-judge (GEMBA-MQM style), batched.
pub async fn judge(
    provider: &dyn Provider,
    model: &str,
    source_locale: &str,
    target_locale: &str,
    items: &[JudgeItem],
) -> Result<(Vec<Verdict>, Usage)> {
    let c = provider
        .complete(request(model, source_locale, target_locale, items))
        .await?;
    let usage = Usage::of(&c.model, c.input_tokens, c.output_tokens);
    let parsed = parse_json(&c.text).unwrap_or(Value::Null);
    let verdicts = items
        .iter()
        .filter_map(|i| {
            let v = &parsed["verdicts"][&i.key];
            let errors = v["errors"].as_array()?;
            let lost: u32 = errors
                .iter()
                .map(|e| penalty(e["severity"].as_str().unwrap_or("minor")))
                .sum();
            let notes = errors
                .iter()
                .map(|e| {
                    format!(
                        "[{} {}] {}",
                        e["severity"].as_str().unwrap_or("minor"),
                        e["category"].as_str().unwrap_or("?"),
                        e["description"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            Some(Verdict {
                key: i.key.clone(),
                score: 100u32.saturating_sub(lost) as u8,
                notes,
                suggestion: v["suggestion"].as_str().map(String::from),
            })
        })
        .collect();
    Ok((verdicts, usage))
}
