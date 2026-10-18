use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

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
    provider::{Completion, CompletionRequest, Provider},
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
    /// Uncached input tokens.
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cost_usd += other.cost_usd;
    }

    /// Usage of one call, cache writes/reads priced at their real rates.
    pub fn of_completion(c: &Completion) -> Self {
        Self {
            input_tokens: c.input_tokens as u64,
            output_tokens: c.output_tokens as u64,
            cache_creation_tokens: c.cache_creation_tokens as u64,
            cache_read_tokens: c.cache_read_tokens as u64,
            cost_usd: pricing::cost_cached_usd(
                &c.model,
                c.input_tokens,
                c.cache_creation_tokens,
                c.cache_read_tokens,
                c.output_tokens,
            ),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn of(model: &str, input: u32, output: u32) -> Self {
        Self {
            input_tokens: input as u64,
            output_tokens: output as u64,
            cost_usd: pricing::cost_usd(model, input, output),
            ..Default::default()
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
        self.translate_from(req, &BTreeMap::new()).await
    }

    /// First-pass prompts of every batch of `req`, keyed by `request_id` (Message Batches custom ids).
    pub fn first_pass(&self, req: &Request) -> Vec<(String, CompletionRequest)> {
        req.items
            .chunks(self.batch_size)
            .map(|chunk| {
                let batch: Vec<_> = chunk.iter().map(|i| (i, None)).collect();
                let c = completion_request(req, &self.model, &batch);
                (request_id(&c), c)
            })
            .collect()
    }

    /// Same pipeline, but a batch's first pass comes from `firsts[request_id]` when present (Message
    /// Batch result, billed at `BATCH_DISCOUNT`); missing ones and all retries are direct calls.
    pub async fn translate_from(
        &self,
        req: Request,
        firsts: &BTreeMap<String, Completion>,
    ) -> Result<(Vec<Translated>, Usage)> {
        // Futures built eagerly (they are lazy): a closure inside the stream breaks `Send` (rustc higher-ranked lifetime issue).
        let batches: Vec<_> = req
            .items
            .chunks(self.batch_size)
            .map(|chunk| {
                let first = (!firsts.is_empty())
                    .then(|| {
                        let batch: Vec<_> = chunk.iter().map(|i| (i, None)).collect();
                        firsts.get(&request_id(&completion_request(&req, &self.model, &batch)))
                    })
                    .flatten();
                self.batch(&req, chunk, first)
            })
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

    async fn batch(
        &self,
        req: &Request,
        items: &[Item],
        first: Option<&Completion>,
    ) -> Result<(Vec<Translated>, Usage)> {
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
            let call = match first.filter(|_| n == 0) {
                Some(c) => {
                    let (m, mut u) = parse_completion(c, model);
                    u.cost_usd *= pricing::BATCH_DISCOUNT;
                    Ok((m, u))
                }
                None => self.call(req, model, &batch).await,
            };
            let (values, u) = match call {
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
        let fit = item.context.fit.as_ref();
        let c = Constraints {
            max_length: item.context.max_length,
            // A measured pixel budget replaces the (noisy) length-ratio guess.
            max_length_ratio: fit.is_none().then_some(self.max_length_ratio),
        };
        let mut issues =
            validate::validate(&item.source, value, &req.target_locale, req.syntax, &c);
        issues.extend(fit.and_then(|f| glot_core::measure::check(value, req.syntax, f)));
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
        Ok(parse_completion(&c, model))
    }
}

fn parse_completion(c: &Completion, model: &str) -> (Map<String, Value>, Usage) {
    let map = parse_json(&c.text).and_then(|v| match v.get("translations") {
        Some(Value::Object(m)) => Some(m.clone()),
        _ => v.as_object().cloned(),
    });
    if map.is_none() {
        tracing::warn!(model, "could not parse JSON from model output");
    }
    (map.unwrap_or_default(), Usage::of_completion(c))
}

/// Content hash of a request (16 hex chars): stable Message Batches custom id.
pub fn request_id(c: &CompletionRequest) -> String {
    glot_core::hash::hash(&serde_json::to_string(c).unwrap_or_default())
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

/// Per-item probability that the first pass fails validation (placeholders, plurals, glossary…).
/// Each retry halves it; escalation sees a quarter. A failing item re-sends the whole system prompt.
const ITEM_FAILURE_RATE: f64 = 0.03;
/// Worst case: every batch makes all its retries + the escalation, each carrying this share of items.
const RETRY_SHARE_MAX: f64 = 0.30;
/// Thinking models reason before answering, even for a single retried item.
const REASONING_FLOOR: u64 = 400;
/// Char heuristic vs real tokenizers (newer Claude tokenizers count up to ~30 % more).
const TOKENIZER_MAX: f64 = 1.3;

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
    if thinks(model) { 3.5 } else { 1.0 }
}

fn thinks(model: &str) -> bool {
    model.starts_with("claude-") && model.contains("-5-")
}

/// Smallest system prompt the provider caches (shorter prefixes silently don't).
fn min_cacheable(model: &str) -> u64 {
    if model.starts_with("claude-") {
        if model.contains("-5") || model.contains("4-7") || model.contains("4-8") {
            512
        } else {
            4096
        }
    } else {
        1024
    }
}

/// Dry-run cost estimate of one locale request.
#[derive(Debug, Clone, Default)]
pub struct Estimate {
    /// Expected run: first pass + judge + probable retries/escalation, prompt cache warm after the first calls.
    pub expected: Usage,
    /// Pessimistic bound: tokenizer variance, every batch retried `MAX_RETRIES` times and escalated to
    /// `fallback_model`, no cache hit.
    pub max_cost_usd: f64,
    /// `expected` with the first pass sent through the Message Batches API (`--batch`, -50 %).
    pub batch_cost_usd: f64,
}

/// Expected cost of a run: dry-run estimate built from the real prompts (system + rules + glossary + per-item context + JSON
/// envelope, per batch), the expected JSON output (plural forms of the target locale included), image
/// crops, the judge pass, and the retry/escalation calls — each of which re-sends the system prompt.
/// No provider call.
pub fn estimate(cfg: &Config, req: &Request) -> Usage {
    estimate_full(cfg, req, &BTreeMap::new()).expected
}

/// `estimate` with image crops (`--visual`), the pessimistic bound and the `--batch` price.
pub fn estimate_full(cfg: &Config, req: &Request, crops: &BTreeMap<Key, String>) -> Estimate {
    let mut est = Estimate::default();
    let mut expected_batched = 0.0;
    let model = cfg.llm.model.as_str();
    let fallback = cfg.llm.fallback_model.as_deref().filter(|m| *m != model);
    let factor = output_factor(&req.target_locale);
    let src_cats = plural::categories(&req.source_locale).len().max(1) as f64;
    let tgt_cats = plural::categories(&req.target_locale).len() as f64;
    let item_out = |i: &Item| {
        let plural = if i.source.contains("plural,") {
            (tgt_cats / src_cats).max(1.0)
        } else {
            1.0
        };
        estimate_tokens(&format!("\"{}\": \"\",", i.key))
            + (estimate_tokens(&i.source) as f64 * factor * plural).ceil() as u64
    };
    let images = |items: &[&Item]| -> u64 {
        items
            .iter()
            .filter_map(|i| crops.get(&i.key))
            .take(crate::visual::MAX_IMAGES)
            .map(|b| crate::visual::image_tokens(b))
            .sum()
    };
    // One call: (system, user incl. images, visible output) tokens.
    let call = |c: &CompletionRequest, items: &[&Item], visible: u64| {
        let out = (visible as f64 * reasoning_factor(&c.model)).ceil() as u64;
        let out = if thinks(&c.model) {
            out.max(REASONING_FLOOR)
        } else {
            out
        };
        (
            estimate_tokens(&c.system),
            estimate_tokens(&c.user) + images(items),
            out,
        )
    };
    // (model, weight, system, user, output, system cached?)
    let mut add = |m: &str,
                   w: f64,
                   (sys, user, out): (u64, u64, u64),
                   cached: bool,
                   batched: bool| {
        let cacheable = sys >= min_cacheable(m);
        let (input, write, read) = match (cacheable, cached) {
            (false, _) => (sys + user, 0, 0),
            (true, false) => (user, sys, 0),
            (true, true) => (user, 0, sys),
        };
        let cost =
            w * pricing::cost_cached_usd(m, input as u32, write as u32, read as u32, out as u32);
        let u = &mut est.expected;
        u.input_tokens += (w * input as f64).round() as u64;
        u.cache_creation_tokens += (w * write as f64).round() as u64;
        u.cache_read_tokens += (w * read as f64).round() as u64;
        u.output_tokens += (w * out as f64).round() as u64;
        u.cost_usd += cost;
        expected_batched += if batched {
            cost * pricing::BATCH_DISCOUNT
        } else {
            cost
        };
        // Worst case: tokenizer variance, system re-written (never read) on every call.
        let t = |x: u64| (x as f64 * TOKENIZER_MAX).ceil() as u32;
        pricing::cost_cached_usd(m, t(user), t(sys), 0, t(out))
    };
    let mut max = 0.0;
    for (n, chunk) in req.items.chunks(cfg.llm.batch_size.max(1)).enumerate() {
        let all: Vec<&Item> = chunk.iter().collect();
        let len = chunk.len() as f64;
        // First pass. Up to CONCURRENCY batches start together: they all write the cache.
        let batch: Vec<_> = chunk.iter().map(|i| (i, None)).collect();
        let first = call(
            &completion_request(req, model, &batch),
            &all,
            estimate_tokens(r#"{"translations": {}}"#)
                + all.iter().map(|i| item_out(i)).sum::<u64>(),
        );
        max += add(model, 1.0, first, n >= CONCURRENCY, true);

        // Retries: a rejected attempt + its issues go back with each failing item.
        let retry = |m: &str, share: f64| {
            let k = ((len * share).ceil() as usize).clamp(1, chunk.len());
            let picked = &all[..k];
            let prev: Vec<Translated> = picked
                .iter()
                .map(|i| Translated {
                    key: i.key.clone(),
                    value: i.source.clone(),
                    model: m.into(),
                    score: None,
                    issues: vec![Issue {
                        code: IssueCode::PlaceholderMissing,
                        severity: Severity::Error,
                        message: "placeholder {name} is missing from the translation".into(),
                    }],
                    attempts: 1,
                })
                .collect();
            let batch: Vec<_> = picked
                .iter()
                .zip(&prev)
                .map(|(i, p)| (*i, Some(p)))
                .collect();
            call(
                &completion_request(req, m, &batch),
                picked,
                picked.iter().map(|i| item_out(i)).sum(),
            )
        };
        let p = |rate: f64| 1.0 - (1.0 - rate).powf(len);
        for k in 0..MAX_RETRIES as i32 {
            let rate = ITEM_FAILURE_RATE / 2f64.powi(k);
            add(model, p(rate), retry(model, rate), true, false);
            max += add(model, 0.0, retry(model, RETRY_SHARE_MAX), false, false);
        }
        if let Some(fb) = fallback {
            let rate = ITEM_FAILURE_RATE / 4.0;
            add(fb, p(rate), retry(fb, rate), false, false);
            max += add(fb, 0.0, retry(fb, RETRY_SHARE_MAX), false, false);
        }

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
            let c = call(
                &judge::request(jm, &req.source_locale, &req.target_locale, &ji),
                &[],
                out,
            );
            max += add(jm, 1.0, c, n >= CONCURRENCY, false);
        }
    }
    est.max_cost_usd = max.max(est.expected.cost_usd);
    est.batch_cost_usd = expected_batched;
    est
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
    s += "6. Respect `max_length` (visible characters) when given; otherwise stay close to the source length: UI space is limited. `fit` is a hard pixel budget measured in the running app (box width, lines, font): the rendered translation must fit it, so pick a shorter idiomatic wording up front (never truncate or add an ellipsis).\n";
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
            opt("fit", json!(c.fit.as_ref().map(glot_core::measure::hint)));
            opt(
                "fit_chars",
                json!(c.fit.as_ref().map(glot_core::measure::approx_chars)),
            );
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
        // Glossary lives in the system prompt only (stable, cached prefix).
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
