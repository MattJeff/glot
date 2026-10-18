use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use glot_core::{
    Key, Locale,
    config::Config,
    context::KeyContext,
    glossary::{Glossary, Term},
    plural,
    validate::{self, Constraints, Issue, IssueCode, Severity, Syntax},
};

use crate::{
    Result,
    judge::{self, JudgeItem},
    pricing,
    provider::{CompletionRequest, Provider},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub key: Key,
    pub source: String,
    pub context: KeyContext,
    /// Previous translation (stale case) — hint for minimal edits.
    pub previous: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub source_locale: Locale,
    pub target_locale: Locale,
    pub items: Vec<Item>,
    /// Glossary terms matching at least one item.
    pub glossary: Vec<Term>,
    pub syntax: Syntax,
    pub style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Translated {
    pub key: Key,
    pub value: String,
    pub model: String,
    /// Judge score 0-100 when a judge ran.
    pub score: Option<u8>,
    /// Remaining issues after retries (errors => value must not be written).
    pub issues: Vec<Issue>,
    pub attempts: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost_usd += other.cost_usd;
    }

    pub(crate) fn of(model: &str, input: u32, output: u32) -> Self {
        Self {
            input_tokens: input as u64,
            output_tokens: output as u64,
            cost_usd: pricing::cost_usd(model, input, output),
        }
    }
}

const MAX_RETRIES: u8 = 2;
const CONCURRENCY: usize = 4;
const MAX_TOKENS: u32 = 16_000;

/// Guarded pipeline: batch prompt (JSON in/out) -> validate + glossary check per item ->
/// retry failing items with issues in the prompt (max 2) -> escalate to fallback_model ->
/// optional judge -> items under score_threshold get a LowScore warning.
pub struct Translator {
    provider: Box<dyn Provider>,
    model: String,
    fallback_model: Option<String>,
    judge_model: Option<String>,
    batch_size: usize,
    score_threshold: u8,
    max_length_ratio: f32,
}

impl Translator {
    pub fn new(cfg: &Config, provider: Box<dyn Provider>) -> Self {
        Self {
            provider,
            model: cfg.llm.model.clone(),
            fallback_model: cfg
                .llm
                .fallback_model
                .clone()
                .filter(|m| m != &cfg.llm.model),
            judge_model: cfg.llm.judge_model.clone(),
            batch_size: cfg.llm.batch_size.max(1),
            score_threshold: cfg.qa.score_threshold,
            max_length_ratio: cfg.qa.max_length_ratio,
        }
    }

    pub async fn translate(&self, req: Request) -> Result<(Vec<Translated>, Usage)> {
        // Futures built eagerly (they are lazy): a closure inside the stream breaks `Send` (rustc higher-ranked lifetime issue).
        let batches: Vec<_> = req
            .items
            .chunks(self.batch_size)
            .map(|chunk| self.batch(&req, chunk))
            .collect();
        let results: Vec<Result<(Vec<Translated>, Usage)>> =
            stream::iter(batches).buffered(CONCURRENCY).collect().await;
        let mut out = vec![];
        let mut usage = Usage::default();
        for r in results {
            let (t, u) = r?;
            out.extend(t);
            usage.add(&u);
        }
        Ok((out, usage))
    }

    async fn batch(&self, req: &Request, items: &[Item]) -> Result<(Vec<Translated>, Usage)> {
        let mut usage = Usage::default();
        let mut done: Vec<Translated> = items
            .iter()
            .map(|i| Translated {
                key: i.key.clone(),
                value: String::new(),
                model: self.model.clone(),
                score: None,
                issues: vec![],
                attempts: 0,
            })
            .collect();

        // Attempt plan: first pass, MAX_RETRIES targeted retries, then one escalation.
        let mut models = vec![self.model.as_str(); 1 + MAX_RETRIES as usize];
        models.extend(self.fallback_model.as_deref());
        for (n, model) in models.into_iter().enumerate() {
            let todo: Vec<usize> = (0..items.len())
                .filter(|&i| n == 0 || validate::has_errors(&done[i].issues))
                .collect();
            if todo.is_empty() {
                break;
            }
            let batch: Vec<(&Item, Option<&Translated>)> = todo
                .iter()
                .map(|&i| (&items[i], (n > 0).then_some(&done[i])))
                .collect();
            let (values, u) = match self.call(req, model, &batch).await {
                Ok(r) => r,
                Err(e) if n == 0 => return Err(e),
                Err(e) => {
                    tracing::warn!("retry call failed: {e}");
                    (Map::new(), Usage::default())
                }
            };
            usage.add(&u);
            for &i in &todo {
                let t = &mut done[i];
                t.attempts += 1;
                match values.get(&items[i].key).and_then(Value::as_str) {
                    Some(v) => {
                        t.value = v.to_string();
                        t.model = model.to_string();
                        t.issues = self.check(req, &items[i], v);
                    }
                    // Keep a previous (failing) value if the model skipped the key this time.
                    None if t.value.is_empty() => {
                        t.issues = vec![Issue {
                            code: IssueCode::Empty,
                            severity: Severity::Error,
                            message: "the model returned no translation for this key".into(),
                        }]
                    }
                    None => {}
                }
            }
        }

        if let Some(jm) = &self.judge_model {
            let ji: Vec<JudgeItem> = items
                .iter()
                .zip(&done)
                .filter(|(_, t)| !validate::has_errors(&t.issues))
                .map(|(i, t)| JudgeItem {
                    key: i.key.clone(),
                    source: i.source.clone(),
                    target: t.value.clone(),
                    context: i.context.clone(),
                })
                .collect();
            if !ji.is_empty() {
                match judge::judge(
                    self.provider.as_ref(),
                    jm,
                    &req.source_locale,
                    &req.target_locale,
                    &ji,
                )
                .await
                {
                    Ok((verdicts, u)) => {
                        usage.add(&u);
                        for v in verdicts {
                            let Some(t) = done.iter_mut().find(|t| t.key == v.key) else {
                                continue;
                            };
                            t.score = Some(v.score);
                            if v.score < self.score_threshold {
                                let mut message =
                                    format!("judge score {} < {}", v.score, self.score_threshold);
                                if !v.notes.is_empty() {
                                    message += &format!(": {}", v.notes);
                                }
                                if let Some(s) = v.suggestion {
                                    message += &format!(" (suggestion: {s})");
                                }
                                t.issues.push(Issue {
                                    code: IssueCode::LowScore,
                                    severity: Severity::Warning,
                                    message,
                                });
                            }
                        }
                    }
                    Err(e) => tracing::warn!("judge failed: {e}"),
                }
            }
        }
        Ok((done, usage))
    }

    fn check(&self, req: &Request, item: &Item, value: &str) -> Vec<Issue> {
        let c = Constraints {
            max_length: item.context.max_length,
            max_length_ratio: Some(self.max_length_ratio),
        };
        let mut issues =
            validate::validate(&item.source, value, &req.target_locale, req.syntax, &c);
        issues.extend(
            Glossary {
                terms: req.glossary.clone(),
            }
            .check(&item.source, value, &req.target_locale),
        );
        issues
    }

    async fn call(
        &self,
        req: &Request,
        model: &str,
        batch: &[(&Item, Option<&Translated>)],
    ) -> Result<(Map<String, Value>, Usage)> {
        let c = self
            .provider
            .complete(completion_request(req, model, batch))
            .await?;
        let usage = Usage::of(&c.model, c.input_tokens, c.output_tokens);
        let map = parse_json(&c.text).and_then(|v| match v.get("translations") {
            Some(Value::Object(m)) => Some(m.clone()),
            _ => v.as_object().cloned(),
        });
        if map.is_none() {
            tracing::warn!(model, "could not parse JSON from model output");
        }
        Ok((map.unwrap_or_default(), usage))
    }
}

/// The exact prompt the pipeline sends for one batch (`prev` set on retries).
pub(crate) fn completion_request(
    req: &Request,
    model: &str,
    batch: &[(&Item, Option<&Translated>)],
) -> CompletionRequest {
    CompletionRequest {
        model: model.into(),
        system: system_prompt(req),
        user: user_prompt(req, batch),
        max_tokens: MAX_TOKENS,
        json: true,
        images: vec![],
    }
}

/// Fraction of extra calls budgeted for retries of failing items / escalation (first pass ≈ 100 %).
pub const RETRY_MARGIN: f64 = 0.10;

/// Token count heuristic: ASCII text (English prompt, JSON, keys) ≈ 3.5 chars/token; other scripts
/// tokenize far denser (CJK ≈ 1 char/token, Arabic/Cyrillic ≈ 2), counted at 1.5 chars/token (conservative).
pub fn estimate_tokens(s: &str) -> u64 {
    let ascii = s.bytes().filter(u8::is_ascii).count();
    let other = s.chars().count() - ascii;
    (ascii as f64 / 3.5 + other as f64 / 1.5).ceil() as u64
}

/// Output tokens of a translation relative to the source's tokens (English-like source).
fn output_factor(locale: &str) -> f64 {
    match locale.split(['-', '_']).next().unwrap_or(locale) {
        "ja" | "zh" | "ko" => 1.8,
        "ar" | "he" | "fa" | "ru" | "uk" | "bg" | "el" | "hi" | "th" => 2.0,
        _ => 1.3, // Latin scripts: ~20-30 % longer than English.
    }
}

/// Hidden reasoning billed as output by models that think by default. Measured on the glot dashboard
/// (304 keys → fr, claude-haiku-5-5): 23.4k real output tokens for 6.6k of visible JSON ≈ 3.5x.
fn reasoning_factor(model: &str) -> f64 {
    if model.starts_with("claude-") && model.contains("-5-") {
        3.5
    } else {
        1.0
    }
}

/// Dry-run estimate built from the real first-pass prompts (system + rules + glossary + per-item
/// context + JSON envelope, per batch), the expected JSON output, the judge pass when configured,
/// plus `RETRY_MARGIN`. No provider call.
pub fn estimate(cfg: &Config, req: &Request) -> Usage {
    let factor = output_factor(&req.target_locale);
    let mut u = Usage::default();
    let mut add = |c: &CompletionRequest, out: u64| {
        let input = estimate_tokens(&c.system) + estimate_tokens(&c.user);
        let out = (out as f64 * reasoning_factor(&c.model)).ceil() as u64;
        u.add(&Usage::of(&c.model, input as u32, out as u32));
    };
    for chunk in req.items.chunks(cfg.llm.batch_size.max(1)) {
        let batch: Vec<_> = chunk.iter().map(|i| (i, None)).collect();
        // Expected answer: {"translations": {"<key>": "<target ≈ source × factor>"}}.
        let out = estimate_tokens(r#"{"translations": {}}"#)
            + chunk
                .iter()
                .map(|i| {
                    estimate_tokens(&format!("\"{}\": \"\",", i.key))
                        + (estimate_tokens(&i.source) as f64 * factor).ceil() as u64
                })
                .sum::<u64>();
        add(&completion_request(req, &cfg.llm.model, &batch), out);
        if let Some(jm) = &cfg.llm.judge_model {
            // ponytail: the source stands in for the unknown target in the judge payload.
            let ji: Vec<JudgeItem> = chunk
                .iter()
                .map(|i| JudgeItem {
                    key: i.key.clone(),
                    source: i.source.clone(),
                    target: i.source.clone(),
                    context: i.context.clone(),
                })
                .collect();
            let out = chunk
                .iter()
                .map(|i| {
                    estimate_tokens(&format!(
                        "\"{}\": {{\"errors\": [], \"suggestion\": null}},",
                        i.key
                    ))
                })
                .sum::<u64>();
            add(
                &judge::request(jm, &req.source_locale, &req.target_locale, &ji),
                out,
            );
        }
    }
    let m = 1.0 + RETRY_MARGIN;
    Usage {
        input_tokens: (u.input_tokens as f64 * m).ceil() as u64,
        output_tokens: (u.output_tokens as f64 * m).ceil() as u64,
        cost_usd: u.cost_usd * m,
    }
}

fn syntax_rules(syntax: Syntax) -> &'static str {
    match syntax {
        Syntax::Icu => {
            "ICU MessageFormat: placeholders look like {name} or {count, number}. Inside {x, plural, ...}, {x, select, ...} and {x, selectordinal, ...} never translate the argument name, the keyword (plural/select/selectordinal/offset) or the selectors (zero/one/two/few/many/other/=0/=1, select values); translate only the text inside the option braces, and keep `#` as-is. Keep '' escaping."
        }
        Syntax::I18next => {
            "i18next: placeholders look like {{name}} or {{count, number}}; nesting looks like $t(key) and must be kept verbatim. Keep <0>, <1>... indexed tags."
        }
        Syntax::Mf2 => {
            "Unicode MessageFormat 2: keep every {$var}, {$var :function option=value}, .input/.local/.match declarations and variant keys verbatim; translate only the pattern text."
        }
    }
}

fn system_prompt(req: &Request) -> String {
    let cats: Vec<&str> = plural::categories(&req.target_locale)
        .iter()
        .map(|c| c.as_str())
        .collect();
    let mut s = format!(
        "You are a senior software localization specialist. You translate user-interface strings of a SaaS web product from `{src}` to `{tgt}`, \
writing exactly what a native {tgt} product team would ship.\n\n",
        src = req.source_locale,
        tgt = req.target_locale
    );
    if let Some(style) = req.style.as_deref().filter(|s| !s.trim().is_empty()) {
        s += &format!("Style guide (follow it): {style}\n\n");
    }
    s += "Strict rules — a translation breaking any of them is rejected automatically:\n";
    s += "1. Keep every placeholder/variable exactly as written in the source (same names, same braces, never translated, never added or removed). Reordering them is fine when the grammar needs it.\n";
    s += &format!("2. {}\n", syntax_rules(req.syntax));
    s += &format!(
        "3. Plural messages in `{}` must contain exactly these CLDR categories: {} — add the missing ones and drop the others; explicit =N cases from the source are kept.\n",
        req.target_locale,
        cats.join(", ")
    );
    s += "4. Keep markup tags (<b>…</b>, <link>…</link>, <0>…</0>, <br/>) with the same names, nesting and count; translate only the text between them.\n";
    s += "5. Keep leading/trailing whitespace and line breaks as in the source.\n";
    s += "6. Respect `max_length` (visible characters) when given; otherwise stay close to the source length: UI space is limited.\n";
    s += "7. UI tone: concise, natural, idiomatic; buttons are short verbs, no trailing period unless the source has one; use the target locale's punctuation, quotes and spacing conventions.\n";
    s += "8. Use `description`, `ui_role`, `component` and `neighbors` to disambiguate meaning (e.g. a verb vs a noun). When `previous` is given the source changed: edit the previous translation minimally.\n";
    s += "9. When `issues_to_fix` is given, your earlier attempt was rejected for these reasons: fix them all.\n";
    if !req.glossary.is_empty() {
        s += "\nGlossary (mandatory):\n";
        for t in &req.glossary {
            if t.do_not_translate {
                s += &format!("- \"{}\": never translate, keep it verbatim.\n", t.term);
                continue;
            }
            let mut line = format!("- \"{}\"", t.term);
            if let Some(tr) = t.translations.get(&req.target_locale) {
                line += &format!(" => \"{tr}\"");
            }
            if let Some(d) = &t.description {
                line += &format!(" ({d})");
            }
            if let Some(f) = t
                .forbidden
                .get(&req.target_locale)
                .filter(|f| !f.is_empty())
            {
                line += &format!(
                    "; never use: {}",
                    f.iter()
                        .map(|x| format!("\"{x}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            s += &line;
            s += "\n";
        }
    }
    s += "\nOutput: only one JSON object, no prose, no code fence: {\"translations\": {\"<key>\": \"<translated string>\"}} with one entry per input key.";
    s
}

fn user_prompt(req: &Request, batch: &[(&Item, Option<&Translated>)]) -> String {
    let items: Vec<Value> = batch
        .iter()
        .map(|(i, prev)| {
            let c = &i.context;
            let mut o = Map::new();
            o.insert("key".into(), json!(i.key));
            o.insert("source".into(), json!(i.source));
            let mut opt = |k: &str, v: Value| {
                if !v.is_null() {
                    o.insert(k.into(), v);
                }
            };
            opt("description", json!(c.description));
            opt("ui_role", json!(c.ui_role));
            opt("component", json!(c.component));
            opt("max_length", json!(c.max_length));
            if !c.neighbors.is_empty() {
                opt(
                    "neighbors",
                    Value::Object(
                        c.neighbors
                            .iter()
                            .take(8)
                            .map(|(k, v)| (k.clone(), json!(v)))
                            .collect(),
                    ),
                );
            }
            opt("previous", json!(i.previous));
            if let Some(t) = prev.filter(|t| !t.value.is_empty() || !t.issues.is_empty()) {
                opt("rejected_attempt", json!(t.value));
                opt(
                    "issues_to_fix",
                    json!(
                        t.issues
                            .iter()
                            .filter(|x| x.severity == Severity::Error)
                            .map(|x| x.message.clone())
                            .collect::<Vec<_>>()
                    ),
                );
            }
            Value::Object(o)
        })
        .collect();
    let payload = json!({
        "task": "translate",
        "source_locale": req.source_locale,
        "target_locale": req.target_locale,
        "glossary": req.glossary,
        "items": items,
    });
    format!(
        "Translate these {} UI strings to `{}`.\n\n{}",
        batch.len(),
        req.target_locale,
        payload_block(&payload)
    )
}

pub(crate) fn payload_block(v: &Value) -> String {
    format!(
        "<payload>\n{}\n</payload>",
        serde_json::to_string_pretty(v).unwrap_or_default()
    )
}

pub(crate) fn extract_payload(user: &str) -> Option<Value> {
    let start = user.find("<payload>")? + "<payload>".len();
    let end = user.rfind("</payload>")?;
    serde_json::from_str(user.get(start..end)?).ok()
}

/// Lenient JSON object extraction (code fences, prose around it).
pub(crate) fn parse_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(t) {
        return Some(v);
    }
    let (a, b) = (t.find('{')?, t.rfind('}')?);
    serde_json::from_str(t.get(a..=b)?)
        .ok()
        .filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::parse_json;

    #[test]
    fn lenient_json() {
        assert_eq!(
            parse_json("```json\n{\"translations\":{\"a\":\"b\"}}\n```").unwrap()["translations"]["a"],
            "b"
        );
        assert_eq!(parse_json("Sure! {\"a\": 1} done").unwrap()["a"], 1);
        assert!(parse_json("nope").is_none());
    }
}
