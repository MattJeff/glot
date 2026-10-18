#![allow(clippy::type_complexity)] // ponytail: report tallies are local tuple maps; name them if they spread
//! glot-bench: does the glot pipeline (validator, glossary, retries, escalation, pixel budget) ship
//! fewer broken UI strings than a plain "translate this JSON" prompt? See web/content/docs/benchmark.md.
//!
//! `glot-bench run`    — translates the dataset per (model, locale, condition), cached + resumable.
//! `glot-bench report` — judges every output (glot validator + independent FormatJS/Intl judge) and
//!                       writes report.md / report.json / chart.svg next to the run.
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use glot_core::{
    config::Config,
    context::KeyContext,
    glossary::{Glossary, Term},
    measure::{self, Fit},
    validate::{self, Constraints, Severity, Syntax},
};
use glot_llm::{
    pricing,
    provider::{self, Completion, CompletionRequest, Provider},
    translate::{self, Item, Request, Translator, Usage},
};

#[derive(Parser)]
#[command(
    name = "glot-bench",
    about = "Raw LLM prompt vs the glot pipeline on 500 hard UI strings"
)]
struct Cli {
    /// Directory holding data/, judge/, results/ and cache/.
    #[arg(long, default_value = "bench", global = true)]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Run {
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "claude-haiku-5-5,claude-sonnet-5-5,gpt-5-mini"
        )]
        models: Vec<String>,
        #[arg(long, value_delimiter = ',', default_value = "fr,de,es,pl,ru,ar,ja,zh")]
        locales: Vec<String>,
        #[arg(long, value_delimiter = ',', default_value = "raw,glot")]
        conditions: Vec<String>,
        /// Messages per locale (evenly spread over the dataset, so every category is represented).
        #[arg(long, default_value_t = 500)]
        sample: usize,
        /// Hard cap: refused up front if the estimate exceeds it, aborted at runtime when spent.
        #[arg(long)]
        budget_usd: f64,
        /// Offline: every model is served by `mock-noisy` (GLOT_MOCK_SEED, GLOT_MOCK_NOISE).
        #[arg(long)]
        mock: bool,
        /// Print the estimate and exit.
        #[arg(long)]
        dry_run: bool,
        /// Run directory (default: <dir>/results/<today>). Re-running resumes it.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Report {
        /// Run directory (default: the latest under <dir>/results/).
        #[arg(long)]
        run: Option<PathBuf>,
        /// Also refresh the results section of web/content/docs/benchmark.md.
        #[arg(long)]
        docs: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct Msg {
    id: String,
    category: String,
    source: String,
    description: String,
    ui_role: String,
    #[serde(default)]
    fit: Option<Fit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Output {
    id: String,
    target: String,
    /// glot condition: false = rejected by the pipeline, never written. raw: always true.
    written: bool,
    attempts: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Cell {
    model: String,
    locale: String,
    condition: String,
    outputs: Vec<Output>,
    usage: Usage,
    calls: u64,
    /// Sum of per-call latencies (cache hits replay the latency of the original call).
    call_ms: u64,
    wall_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            models,
            locales,
            conditions,
            sample,
            budget_usd,
            mock,
            dry_run,
            out,
        } => {
            let out = out.unwrap_or_else(|| {
                cli.dir
                    .join("results")
                    .join(chrono::Local::now().format("%Y-%m-%d").to_string())
            });
            run(
                &cli.dir,
                &out,
                &models,
                &locales,
                &conditions,
                sample,
                budget_usd,
                mock,
                dry_run,
            )
            .await
        }
        Cmd::Report { run, docs } => {
            let run = match run {
                Some(r) => r,
                None => latest_run(&cli.dir.join("results"))?,
            };
            report(&cli.dir, &run, docs.as_deref())
        }
    }
}

fn load_messages(dir: &Path) -> Result<Vec<Msg>> {
    let path = dir.join("data/messages.jsonl");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).with_context(|| format!("bad line: {l}")))
        .collect()
}

fn load_glossary(dir: &Path) -> Result<Glossary> {
    Ok(Glossary::load(&dir.join("data/glossary.toml"))?)
}

/// Every k-th message: a sample keeps the category mix of the full set.
fn sample(msgs: &[Msg], n: usize) -> Vec<Msg> {
    let n = n.clamp(1, msgs.len());
    (0..n).map(|i| msgs[i * msgs.len() / n].clone()).collect()
}

fn provider_of(model: &str, mock: bool) -> &'static str {
    match () {
        _ if mock => "mock-noisy",
        _ if model.starts_with("claude-") => "anthropic",
        _ => "openai",
    }
}

/// Escalation target of the glot condition (same provider as the model).
fn fallback_of(model: &str) -> Option<&'static str> {
    match model {
        m if m.starts_with("claude-haiku") => Some("claude-sonnet-5-5"),
        m if m.starts_with("gpt-5-mini") || m.starts_with("gpt-5-nano") => Some("gpt-5"),
        _ => None,
    }
}

fn config(model: &str, locale: &str, mock: bool) -> Result<Config> {
    Ok(serde_json::from_value(json!({
        "source_locale": "en",
        "target_locales": [locale],
        "files": ["messages/{locale}.json"],
        "syntax": "icu",
        "llm": {
            "provider": provider_of(model, mock),
            "model": model,
            "fallback_model": fallback_of(model),
        },
    }))?)
}

fn request(msgs: &[Msg], glossary: &Glossary, locale: &str) -> Request {
    let mut terms: Vec<Term> = vec![];
    for m in msgs {
        for t in glossary.matches(&m.source) {
            if !terms.iter().any(|x| x.term == t.term) {
                terms.push(t.clone());
            }
        }
    }
    Request {
        source_locale: "en".into(),
        target_locale: locale.into(),
        items: msgs
            .iter()
            .map(|m| Item {
                key: m.id.clone(),
                source: m.source.clone(),
                context: KeyContext {
                    key: m.id.clone(),
                    description: Some(m.description.clone()),
                    ui_role: Some(m.ui_role.clone()),
                    fit: m.fit.clone(),
                    ..Default::default()
                },
                previous: None,
            })
            .collect(),
        glossary: terms,
        syntax: Syntax::Icu,
        style: None,
    }
}

// ------------------------------------------------------------------ the "raw" condition

const RAW_BATCH: usize = 40;

/// What a developer would write: JSON in/out, keep placeholders, the glossary, the UI context.
/// No CLDR plural list, no pixel budget, no validation, no retry.
fn raw_request(req: &Request, model: &str, items: &[Item]) -> CompletionRequest {
    let mut system = format!(
        "Translate the UI strings of a web app from English to {}. Keep placeholders like {{name}}, ICU MessageFormat syntax (plural/select) and HTML-like tags unchanged. Return only a JSON object: {{\"translations\": {{\"<key>\": \"<translation>\"}}}}.",
        req.target_locale
    );
    if !req.glossary.is_empty() {
        system += "\n\nGlossary:\n";
        for t in &req.glossary {
            match t.translations.get(&req.target_locale) {
                _ if t.do_not_translate => system += &format!("- \"{}\": keep as is\n", t.term),
                Some(tr) => system += &format!("- \"{}\" => \"{tr}\"\n", t.term),
                None => {}
            }
        }
    }
    let payload = json!({
        "task": "translate",
        "target_locale": req.target_locale,
        "items": items.iter().map(|i| json!({"key": i.key, "source": i.source, "context": i.context.description})).collect::<Vec<_>>(),
    });
    CompletionRequest {
        model: model.into(),
        system,
        user: format!(
            "<payload>\n{}\n</payload>",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        ),
        max_tokens: 16_000,
        json: true,
        images: vec![],
    }
}

/// Lenient JSON object: bare, fenced or wrapped in prose.
fn parse_json(text: &str) -> Option<Value> {
    let t = text.trim();
    let (a, b) = (t.find('{')?, t.rfind('}')?);
    serde_json::from_str(t.get(a..=b)?)
        .ok()
        .filter(Value::is_object)
}

async fn run_raw(p: &dyn Provider, req: &Request, model: &str) -> Result<(Vec<Output>, Usage)> {
    let mut out = vec![];
    let mut usage = Usage::default();
    for chunk in req.items.chunks(RAW_BATCH) {
        let c = p.complete(raw_request(req, model, chunk)).await?;
        usage.add(&Usage::of_completion(&c));
        let v = parse_json(&c.text).unwrap_or_default();
        let map = v
            .get("translations")
            .filter(|t| t.is_object())
            .unwrap_or(&v);
        for i in chunk {
            out.push(Output {
                id: i.key.clone(),
                target: map
                    .get(&i.key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                written: true,
                attempts: 1,
            });
        }
    }
    Ok((out, usage))
}

// ------------------------------------------------------------------ cache, budget, latency

#[derive(Default)]
struct Meter {
    /// Fresh (non-cached) spend of the whole run.
    spent: f64,
}

/// Wraps a provider: answers from `<dir>/cache/` when the exact request was already paid for
/// (resume after interruption, re-runs free), enforces the budget on fresh calls, times calls.
struct Cached {
    inner: Box<dyn Provider>,
    dir: Option<PathBuf>,
    budget: f64,
    meter: Arc<Mutex<Meter>>,
    cell: Mutex<(u64, u64)>, // (calls, ms)
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    completion: Completion,
    ms: u64,
}

#[async_trait]
impl Provider for Cached {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn complete(&self, req: CompletionRequest) -> glot_llm::Result<Completion> {
        let path = self.dir.as_ref().map(|d| {
            d.join(format!(
                "{}-{}.json",
                self.inner.name(),
                translate::request_id(&req)
            ))
        });
        if let Some(e) = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<CacheEntry>(&t).ok())
        {
            let mut c = self.cell.lock().unwrap();
            *c = (c.0 + 1, c.1 + e.ms);
            return Ok(e.completion);
        }
        let spent = self.meter.lock().unwrap().spent;
        if spent >= self.budget {
            return Err(glot_llm::Error::Budget {
                spent,
                budget: self.budget,
            });
        }
        let t = Instant::now();
        let completion = self.inner.complete(req).await?;
        let ms = t.elapsed().as_millis() as u64;
        self.meter.lock().unwrap().spent += Usage::of_completion(&completion).cost_usd;
        {
            let mut c = self.cell.lock().unwrap();
            *c = (c.0 + 1, c.1 + ms);
        }
        if let Some(p) = path {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(
                &p,
                serde_json::to_string(&CacheEntry {
                    completion: completion.clone(),
                    ms,
                })
                .unwrap(),
            );
        }
        Ok(completion)
    }
}

struct SharedProvider(Arc<Cached>);

#[async_trait]
impl Provider for SharedProvider {
    fn name(&self) -> &str {
        self.0.name()
    }
    async fn complete(&self, req: CompletionRequest) -> glot_llm::Result<Completion> {
        self.0.complete(req).await
    }
}

// ------------------------------------------------------------------ estimate + run

/// Visible output tokens × hidden reasoning for thinking models (`translate::estimate` already
/// counts it for Claude 5.x; gpt-5 reasons by default too).
fn reasoning(model: &str) -> f64 {
    if model.starts_with("claude-") && model.contains("-5-") || model.starts_with("gpt-5") {
        3.5
    } else {
        1.0
    }
}

fn out_factor(locale: &str) -> f64 {
    match locale {
        "ja" | "zh" => 1.8,
        "ar" | "ru" => 2.0,
        _ => 1.3,
    }
}

/// (expected, max) USD of one cell, no provider call.
fn estimate(cfg: &Config, req: &Request, model: &str, condition: &str) -> (f64, f64) {
    if condition == "glot" {
        let e = translate::estimate_full(cfg, req, &BTreeMap::new());
        // ponytail: gpt-5 reasoning is not in translate::estimate; scale its output share here.
        let extra = if model.starts_with("gpt-5") {
            let (_, o) = pricing::price_per_mtok(model).unwrap_or_default();
            e.expected.output_tokens as f64 * 2.5 * o / 1e6
        } else {
            0.0
        };
        return (e.expected.cost_usd + extra, e.max_cost_usd + extra * 1.3);
    }
    let mut cost = 0.0;
    for chunk in req.items.chunks(RAW_BATCH) {
        let c = raw_request(req, model, chunk);
        let input = translate::estimate_tokens(&c.system) + translate::estimate_tokens(&c.user);
        let visible: f64 = chunk
            .iter()
            .map(|i| {
                translate::estimate_tokens(&i.source) as f64 * out_factor(&req.target_locale) + 8.0
            })
            .sum();
        let out = (visible * reasoning(model)).max(400.0);
        cost += pricing::cost_usd(model, input as u32, out as u32);
    }
    (cost, cost * 1.3)
}

fn cell_path(out: &Path, model: &str, locale: &str, condition: &str) -> PathBuf {
    out.join("cells").join(format!(
        "{}__{locale}__{condition}.json",
        model.replace('/', "_")
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    dir: &Path,
    out: &Path,
    models: &[String],
    locales: &[String],
    conditions: &[String],
    n: usize,
    budget: f64,
    mock: bool,
    dry_run: bool,
) -> Result<()> {
    for c in conditions {
        if c != "raw" && c != "glot" {
            bail!("unknown condition {c} (raw | glot)");
        }
    }
    let msgs = sample(&load_messages(dir)?, n);
    let glossary = load_glossary(dir)?;
    for m in msgs.iter().filter(|m| m.fit.is_some()) {
        if measure::check(&m.source, Syntax::Icu, m.fit.as_ref().unwrap())
            .is_some_and(|i| i.severity == Severity::Error)
        {
            eprintln!("warning: source {} is itself wider than its box", m.id);
        }
    }

    let mut cells = vec![];
    let (mut expected, mut max) = (0.0, 0.0);
    println!(
        "{:<20} {:<6} {:<5} {:>10} {:>10}",
        "model", "locale", "cond", "expected$", "max$"
    );
    for model in models {
        for locale in locales {
            for cond in conditions {
                let path = cell_path(out, model, locale, cond);
                let cfg = config(model, locale, mock)?;
                let req = request(&msgs, &glossary, locale);
                let done = path.exists();
                let (e, m) = if done {
                    (0.0, 0.0)
                } else {
                    estimate(&cfg, &req, model, cond)
                };
                println!(
                    "{model:<20} {locale:<6} {cond:<5} {e:>10.4} {m:>10.4}{}",
                    if done { "  (done, skipped)" } else { "" }
                );
                expected += e;
                max += m;
                if !done {
                    cells.push((model.clone(), locale.clone(), cond.clone(), cfg, req, path));
                }
            }
        }
    }
    println!(
        "{} messages/locale, {} cells to run — expected ${expected:.2}, worst case ${max:.2}, budget ${budget:.2}{}",
        msgs.len(),
        cells.len(),
        if mock { " (mock: real spend $0)" } else { "" }
    );
    if expected > budget {
        bail!(
            "refused: expected cost ${expected:.2} exceeds --budget-usd {budget:.2} (lower --sample or drop a model)"
        );
    }
    if dry_run {
        return Ok(());
    }

    let meter = Arc::new(Mutex::new(Meter::default()));
    for (model, locale, cond, cfg, req, path) in cells {
        let cache = (!mock).then(|| dir.join("cache"));
        let cached = Arc::new(Cached {
            inner: provider::from_config(&cfg.llm, None)?,
            dir: cache,
            budget,
            meter: meter.clone(),
            cell: Mutex::new((0, 0)),
        });
        let t = Instant::now();
        let (outputs, usage) = if cond == "raw" {
            run_raw(cached.as_ref(), &req, &model).await?
        } else {
            let tr = Translator::new(&cfg, Box::new(SharedProvider(cached.clone())));
            let (done, usage) = tr.translate(req).await?;
            let outputs = done
                .into_iter()
                .map(|t| Output {
                    written: !validate::has_errors(&t.issues),
                    id: t.key,
                    target: t.value,
                    attempts: t.attempts,
                })
                .collect();
            (outputs, usage)
        };
        let (calls, call_ms) = *cached.cell.lock().unwrap();
        let cell = Cell {
            model: model.clone(),
            locale: locale.clone(),
            condition: cond.clone(),
            outputs,
            usage,
            calls,
            call_ms,
            wall_ms: t.elapsed().as_millis() as u64,
        };
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, serde_json::to_string_pretty(&cell)?)?;
        println!(
            "done {model} {locale} {cond}: ${:.4}, {calls} calls, spent so far ${:.4}",
            cell.usage.cost_usd,
            meter.lock().unwrap().spent
        );
    }
    println!("next: glot-bench report --run {}", out.display());
    Ok(())
}

// ------------------------------------------------------------------ judging + report

fn latest_run(results: &Path) -> Result<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(results)
        .with_context(|| format!("no runs in {}", results.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("cells").is_dir())
        .collect();
    dirs.sort();
    dirs.pop().context("no run directory with cells/")
}

/// glot's own validator, judging like the pipeline does (errors only), glossary with inflections.
fn glot_issues(m: &Msg, target: &str, locale: &str, glossary: &Glossary) -> BTreeSet<String> {
    let code = |c: &validate::IssueCode| {
        serde_json::to_value(c)
            .unwrap()
            .as_str()
            .unwrap_or("?")
            .to_string()
    };
    let mut out = BTreeSet::new();
    for i in validate::validate(
        &m.source,
        target,
        locale,
        Syntax::Icu,
        &Constraints::default(),
    ) {
        if i.severity == Severity::Error {
            out.insert(code(&i.code));
        }
    }
    if out.contains("syntax_error") || out.contains("empty") {
        return out;
    }
    // Terms the source really contains (exact word), then inflected target forms are accepted.
    let used = Glossary {
        terms: glossary.matches(&m.source).into_iter().cloned().collect(),
    };
    for (_, i) in used.violations(&m.source, target, locale, true) {
        out.insert(code(&i.code));
    }
    if let Some(i) = m
        .fit
        .as_ref()
        .and_then(|f| measure::check(target, Syntax::Icu, f))
        && i.severity == Severity::Error
    {
        out.insert(code(&i.code));
    }
    out
}

/// FormatJS parser + Intl.PluralRules (bench/judge). None when node or the package is missing.
fn independent(dir: &Path, rows: &[Value]) -> Option<Vec<BTreeSet<String>>> {
    let script = dir.join("judge/icu-judge.mjs");
    if !dir.join("judge/node_modules").is_dir() {
        eprintln!(
            "warning: independent judge unavailable (run `npm install` in {}/judge)",
            dir.display()
        );
        return None;
    }
    let input = std::env::temp_dir().join(format!("glot-bench-judge-{}.jsonl", std::process::id()));
    let text: String = rows.iter().map(|r| r.to_string() + "\n").collect();
    std::fs::write(&input, text).ok()?;
    let out = std::process::Command::new("node")
        .arg(&script)
        .arg(&input)
        .output();
    let _ = std::fs::remove_file(&input);
    let out = out.ok().filter(|o| o.status.success()).or_else(|| {
        eprintln!("warning: independent judge failed (is node installed?)");
        None
    })?;
    let lines: Vec<BTreeSet<String>> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .and_then(|v| {
                    v["issues"].as_array().map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                })
                .unwrap_or_default()
        })
        .collect();
    (lines.len() == rows.len()).then_some(lines)
}

#[derive(Debug, Default, Serialize)]
struct Row {
    model: String,
    locale: String,
    condition: String,
    n: usize,
    /// Written to the catalog yet broken (glot validator or independent judge).
    broken: usize,
    broken_pct: f64,
    /// glot: rejected by the pipeline, left untranslated rather than written broken.
    rejected: usize,
    rejected_pct: f64,
    /// Broken according to the independent judge alone (subset check of glot's validator).
    broken_independent: usize,
    by_code: BTreeMap<String, usize>,
    /// Broken messages per dataset category.
    by_category: BTreeMap<String, usize>,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    calls: u64,
    latency_ms_per_call: f64,
    wall_s: f64,
}

fn report(dir: &Path, run: &Path, docs: Option<&Path>) -> Result<()> {
    let msgs: BTreeMap<String, Msg> = load_messages(dir)?
        .into_iter()
        .map(|m| (m.id.clone(), m))
        .collect();
    let glossary = load_glossary(dir)?;
    let mut cells: Vec<Cell> = vec![];
    let mut paths: Vec<PathBuf> = std::fs::read_dir(run.join("cells"))?
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    for p in paths {
        cells.push(
            serde_json::from_str(&std::fs::read_to_string(&p)?)
                .with_context(|| p.display().to_string())?,
        );
    }
    if cells.is_empty() {
        bail!("no cells in {}", run.display());
    }
    let rows_in: Vec<Value> = cells
        .iter()
        .flat_map(|c| {
            c.outputs.iter().map(|o| {
                json!({"id": o.id, "locale": c.locale, "source": msgs.get(&o.id).map(|m| m.source.as_str()).unwrap_or(""), "target": o.target})
            })
        })
        .collect();
    let judged = independent(dir, &rows_in);
    let mut k = 0;
    let mut rows = vec![];
    for c in &cells {
        let mut r = Row {
            model: c.model.clone(),
            locale: c.locale.clone(),
            condition: c.condition.clone(),
            n: c.outputs.len(),
            cost_usd: c.usage.cost_usd,
            input_tokens: c.usage.input_tokens + c.usage.cache_creation_tokens,
            output_tokens: c.usage.output_tokens,
            cache_read_tokens: c.usage.cache_read_tokens,
            calls: c.calls,
            latency_ms_per_call: c.call_ms as f64 / c.calls.max(1) as f64,
            wall_s: c.wall_ms as f64 / 1000.0,
            ..Default::default()
        };
        for o in &c.outputs {
            let ind = judged.as_ref().map(|j| j[k].clone()).unwrap_or_default();
            k += 1;
            if !o.written {
                r.rejected += 1;
                continue;
            }
            let Some(m) = msgs.get(&o.id) else { continue };
            let mut issues = glot_issues(m, &o.target, &c.locale, &glossary);
            if ind.iter().any(|i| !i.starts_with("warn:")) {
                r.broken_independent += 1;
            }
            // `warn:` = the judge's soft findings (fr/es/it/pt `many`, which glot also only warns about).
            issues.extend(
                ind.into_iter()
                    .filter(|i| !i.starts_with("warn:"))
                    .map(|i| format!("independent:{i}")),
            );
            if !issues.is_empty() {
                r.broken += 1;
                *r.by_category.entry(m.category.clone()).or_default() += 1;
                for i in issues {
                    *r.by_code.entry(i).or_default() += 1;
                }
            }
        }
        let pct = |x: usize| 100.0 * x as f64 / r.n.max(1) as f64;
        r.broken_pct = pct(r.broken);
        r.rejected_pct = pct(r.rejected);
        rows.push(r);
    }

    let md = markdown(&rows, judged.is_some(), run);
    std::fs::write(run.join("report.md"), &md)?;
    std::fs::write(
        run.join("report.json"),
        serde_json::to_string_pretty(
            &json!({"independent_judge": judged.is_some(), "rows": rows}),
        )?,
    )?;
    std::fs::write(run.join("chart.svg"), svg(&rows))?;
    println!("{md}");
    println!("wrote {}/report.md, report.json, chart.svg", run.display());
    if let Some(docs) = docs {
        update_docs(docs, &md, run)?;
    }
    Ok(())
}

/// (model, condition) totals across locales.
fn totals(rows: &[Row]) -> Vec<(String, String, usize, usize, usize, usize, f64)> {
    let mut t: BTreeMap<(String, String), (usize, usize, usize, usize, f64)> = BTreeMap::new();
    for r in rows {
        let e = t.entry((r.model.clone(), r.condition.clone())).or_default();
        e.0 += r.n;
        e.1 += r.broken;
        e.2 += r.rejected;
        e.3 += r.broken_independent;
        e.4 += r.cost_usd;
    }
    t.into_iter()
        .map(|((m, c), (n, b, rj, bi, cost))| (m, c, n, b, rj, bi, cost))
        .collect()
}

fn markdown(rows: &[Row], independent: bool, run: &Path) -> String {
    let pct = |a: usize, n: usize| 100.0 * a as f64 / n.max(1) as f64;
    let mut s = format!(
        "# glot-bench — {}\n\n",
        run.file_name().unwrap_or_default().to_string_lossy()
    );
    s += "**Broken** = written to the catalog and failing glot's validator (ICU syntax, placeholders, CLDR plural categories, keywords, tags, glossary, pixel width) or the independent FormatJS + `Intl.PluralRules` judge. **Rejected** = refused by glot after retries/escalation and left untranslated instead of shipped broken (raw prompts never reject).\n\n";
    if !independent {
        s += "> Independent judge unavailable for this report: glot's validator only.\n\n";
    }
    s += "## Summary (all locales)\n\n| Model | Condition | Messages | Broken | Broken (independent judge) | Rejected | Cost |\n|---|---|---:|---:|---:|---:|---:|\n";
    for (m, c, n, b, rj, bi, cost) in totals(rows) {
        s += &format!(
            "| {m} | {c} | {n} | {:.1} % | {:.1} % | {:.1} % | ${cost:.4} |\n",
            pct(b, n),
            pct(bi, n),
            pct(rj, n)
        );
    }
    s += "\n![broken messages per model](chart.svg)\n\n## Per locale\n\n| Model | Locale | Condition | Broken | Rejected | Cost | Tokens in/out (cache read) | Calls | Latency/call |\n|---|---|---|---:|---:|---:|---|---:|---:|\n";
    for r in rows {
        s += &format!(
            "| {} | {} | {} | {:.1} % | {:.1} % | ${:.4} | {}/{} ({}) | {} | {:.0} ms |\n",
            r.model,
            r.locale,
            r.condition,
            r.broken_pct,
            r.rejected_pct,
            r.cost_usd,
            r.input_tokens,
            r.output_tokens,
            r.cache_read_tokens,
            r.calls,
            r.latency_ms_per_call
        );
    }
    let mut codes: BTreeMap<(String, String), (BTreeMap<String, usize>, BTreeMap<String, usize>)> =
        BTreeMap::new();
    for r in rows {
        let e = codes
            .entry((r.model.clone(), r.condition.clone()))
            .or_default();
        for (k, v) in &r.by_code {
            *e.0.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &r.by_category {
            *e.1.entry(k.clone()).or_default() += v;
        }
    }
    let list = |by: &BTreeMap<String, usize>| {
        let l: Vec<String> = by.iter().map(|(k, v)| format!("{k} {v}")).collect();
        if l.is_empty() {
            "—".to_string()
        } else {
            l.join(", ")
        }
    };
    s += "\n## Why messages broke\n\n| Model | Condition | Issues (count) | Broken per category |\n|---|---|---|---|\n";
    for ((m, c), (by, cat)) in codes {
        s += &format!("| {m} | {c} | {} | {} |\n", list(&by), list(&cat));
    }
    s
}

fn svg(rows: &[Row]) -> String {
    let t = totals(rows);
    let (w, bar, gap, left) = (640.0, 22.0, 10.0, 220.0);
    let max = t
        .iter()
        .map(|x| 100.0 * x.3 as f64 / x.2.max(1) as f64)
        .fold(1.0f64, f64::max);
    let h = 40.0 + t.len() as f64 * (bar + gap) + 20.0;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" font-family=\"Inter, system-ui, sans-serif\" font-size=\"13\">\n<rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>\n<text x=\"12\" y=\"22\" font-weight=\"600\">Broken messages written to the catalog (lower is better)</text>\n"
    );
    for (i, (m, c, n, b, ..)) in t.iter().enumerate() {
        let y = 40.0 + i as f64 * (bar + gap);
        let p = 100.0 * *b as f64 / (*n).max(1) as f64;
        let len = (w - left - 70.0) * p / max;
        let color = if c == "glot" { "#16a34a" } else { "#dc2626" };
        s += &format!(
            "<text x=\"12\" y=\"{:.0}\">{m} · {c}</text><rect x=\"{left}\" y=\"{y:.0}\" width=\"{len:.1}\" height=\"{bar}\" rx=\"3\" fill=\"{color}\"/><text x=\"{:.1}\" y=\"{:.0}\">{p:.1} %</text>\n",
            y + 16.0,
            left + len + 6.0,
            y + 16.0
        );
    }
    s + "</svg>\n"
}

const START: &str = "<!-- bench:results:start -->";
const END: &str = "<!-- bench:results:end -->";

fn update_docs(path: &Path, md: &str, run: &Path) -> Result<()> {
    let doc = std::fs::read_to_string(path)?;
    let (Some(a), Some(b)) = (doc.find(START), doc.find(END)) else {
        bail!("{} has no {START} … {END} markers", path.display());
    };
    // Drop the report title, demote headings one level, point the chart at the run directory.
    let body: String = md
        .lines()
        .skip(1)
        .map(|l| {
            if l.starts_with('#') {
                format!("#{l}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace(
            "](chart.svg)",
            &format!("](../../../{}/chart.svg)", run.display()),
        );
    let new = format!(
        "{}{START}\n_Run `{}`._\n{body}\n{END}{}",
        &doc[..a],
        run.display(),
        &doc[b + END.len()..]
    );
    std::fs::write(path, new)?;
    println!("updated {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_spreads_and_parses() {
        let msgs = load_messages(Path::new("../../bench")).unwrap();
        assert_eq!(msgs.len(), 500);
        let s = sample(&msgs, 50);
        let cats: BTreeSet<_> = s.iter().map(|m| m.category.as_str()).collect();
        assert!(cats.len() >= 12, "{cats:?}");
        // Every source is valid ICU for glot's own parser.
        for m in &msgs {
            assert!(
                validate::placeholders(&m.source, Syntax::Icu).is_ok(),
                "{}",
                m.source
            );
        }
    }

    #[test]
    fn judges_broken_targets() {
        let g = load_glossary(Path::new("../../bench")).unwrap();
        let m = Msg {
            id: "x".into(),
            category: "plural".into(),
            source: "{count, plural, one {# Project} other {# Projects}}".into(),
            description: String::new(),
            ui_role: "label".into(),
            fit: None,
        };
        assert!(
            glot_issues(
                &m,
                "{count, plural, one {# projet} many {# projets} other {# projets}}",
                "fr",
                &g
            )
            .is_empty()
        );
        assert!(
            glot_issues(
                &m,
                "{count, plural, one {# projekt} other {# projektów}}",
                "pl",
                &g
            )
            .contains("plural_category_missing")
        );
        assert!(
            glot_issues(
                &m,
                "{count, plural, one {# truc} many {# trucs} other {# trucs}}",
                "fr",
                &g
            )
            .contains("glossary_violation")
        );
    }
}
