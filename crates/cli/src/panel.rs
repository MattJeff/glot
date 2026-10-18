//! `glot labs panel`: a synthetic panel of native users per market. `packages/agent-qa` captures each
//! route × locale (screenshot + text + calls to action); a persona per locale answers a 5-second test,
//! CTA comprehension, trust, non-native wording and cultural mismatches; a judge scores the answers
//! against the source-locale intent. Output: comprehension per locale vs source ("localization debt"),
//! top problems tied to keys with validated suggestions, `.glot/panel/report.{html,json}`.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glot_core::validate::{self, Constraints};
use glot_llm::engine::Workspace;
use glot_llm::panel::{self as p, JudgeItem, Page, Persona, Score, Verdict};
use glot_llm::provider;
use glot_llm::translate::Usage;

use crate::style::*;

#[derive(clap::Args)]
pub struct PanelArgs {
    /// Panel config: `product`, `routes: [{route, value_proposition}]`, `personas: {<locale>: {...} | [...]}`.
    #[arg(long, default_value = "glot.panel.yaml")]
    pub config: PathBuf,
    /// Routes to test (comma separated). Default: the config's routes, else `/`.
    #[arg(long, value_delimiter = ',')]
    pub routes: Vec<String>,
    /// Locales (comma separated). Default: source + all targets. The source locale is always the baseline.
    #[arg(long, value_delimiter = ',')]
    pub locales: Vec<String>,
    #[arg(long, default_value = "http://localhost:3000")]
    pub base_url: String,
    /// URL layout; the source locale gets no prefix. E.g. "{route}?lang={locale}".
    #[arg(long, default_value = "/{locale}{route}")]
    pub url_pattern: String,
    /// Cookies (comma separated) set to the locale. Default: NEXT_LOCALE,glot-locale; "" sets none.
    #[arg(long)]
    pub locale_cookie: Option<String>,
    #[arg(long, default_value = "1280x800")]
    pub viewport: String,
    /// Ask the `[llm]` provider of glot.toml (a vision model). Default: the offline mock (deterministic, free).
    #[arg(long)]
    pub live: bool,
    /// Model for `--live` (default: `[llm] model`).
    #[arg(long)]
    pub model: Option<String>,
    /// Max USD for the run (default: `[llm] budget_usd`). Refuses to start above it, stops when reached.
    #[arg(long)]
    pub budget: Option<f64>,
    /// Capture and print the estimated cost only.
    #[arg(long)]
    pub dry_run: bool,
    /// Reuse `<out>/capture.json` instead of opening a browser.
    #[arg(long)]
    pub reuse_capture: bool,
    /// Post the top problems as annotations of the live review (`[server]` + GLOT_TOKEN).
    #[arg(long)]
    pub annotate: bool,
    #[arg(long, default_value = ".glot/panel")]
    pub out: PathBuf,
    #[arg(long)]
    pub json: bool,
}

/// Concurrent persona calls.
const CONCURRENCY: usize = 4;
/// Annotations posted per run (the rest stays in the report).
const MAX_ANNOTATIONS: usize = 20;

#[derive(Serialize)]
struct Answer {
    route: String,
    locale: String,
    url: String,
    screenshot: String,
    persona: Persona,
    verdict: Option<Verdict>,
    score: Option<Score>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, Default, Clone)]
struct LocaleScore {
    /// 0-100, mean over routes × personas.
    comprehension: Option<f64>,
    /// Source comprehension minus this one, in points.
    debt: Option<f64>,
    /// 1-5.
    trust: Option<f64>,
    answers: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Problem {
    locale: String,
    kind: &'static str,
    quote: String,
    why: String,
    key: Option<String>,
    /// Full catalog value proposed for `key` (the quote replaced inside it).
    suggestion: Option<String>,
    /// The glot validator accepted `suggestion` (placeholders, plurals, tags, glossary).
    valid: Option<bool>,
    issues: Vec<String>,
    routes: Vec<String>,
    personas: Vec<String>,
}

#[derive(Serialize)]
struct Report {
    source_locale: String,
    provider: String,
    model: String,
    estimated_usd: f64,
    spent_usd: f64,
    budget_usd: Option<f64>,
    value_propositions: BTreeMap<String, String>,
    locales: BTreeMap<String, LocaleScore>,
    routes: BTreeMap<String, BTreeMap<String, Option<f64>>>,
    problems: Vec<Problem>,
    answers: Vec<Answer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<Value>,
}

pub async fn run(a: PanelArgs) -> Result<ExitCode> {
    let mut ws = crate::open()?;
    let source = ws.config.source_locale.clone();
    let mut locales: Vec<String> = if a.locales.is_empty() {
        ws.config.target_locales.clone()
    } else {
        a.locales.clone()
    };
    locales.retain(|l| *l != source);
    locales.insert(0, source.clone());
    let out = ws.root.join(&a.out);

    if !a.reuse_capture {
        capture(&ws, &a, &locales)?;
    }
    let cap: Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("capture.json"))
            .with_context(|| format!("no {}/capture.json", a.out.display()))?,
    )?;
    let config = cap["config"].clone();
    let pages: Vec<Page> = serde_json::from_value::<Vec<Page>>(cap["pages"].clone())?
        .into_iter()
        .filter(|pg| locales.contains(&pg.locale))
        .collect();
    if pages.is_empty() {
        bail!("the capture has no page for {}", locales.join(","));
    }

    if a.live {
        if ws.config.llm.provider == "mock" {
            bail!("--live uses the [llm] provider of glot.toml, which is \"mock\"");
        }
    } else {
        ws.config.llm.provider = "mock".into();
    }
    let model = if a.live {
        a.model.clone().unwrap_or(ws.config.llm.model.clone())
    } else {
        "mock".into()
    };
    let budget = a.budget.or(ws.config.llm.budget_usd);

    // Persona calls, then one judge call per route.
    let mut jobs = vec![];
    for pg in &pages {
        let img = glot_llm::visual::png_b64(&out.join(&pg.screenshot)).ok();
        for persona in p::personas(&pg.locale, &config) {
            let req = p::persona_request(&model, &source, &persona, pg, img.clone());
            jobs.push((pg, persona, req));
        }
    }
    let routes: Vec<String> = dedup(pages.iter().map(|pg| pg.route.clone()));
    let value_props: BTreeMap<String, String> = routes
        .iter()
        .map(|r| (r.clone(), value_proposition(&config, &pages, r, &source)))
        .collect();
    let blank = Verdict::default();
    let judge_estimate: f64 = routes
        .iter()
        .map(|r| {
            let n = jobs.iter().filter(|j| j.0.route == *r).count();
            let items: Vec<JudgeItem> = (0..n)
                .map(|i| JudgeItem {
                    id: format!("xx#{i}"),
                    verdict: &blank,
                })
                .collect();
            let ctas = source_ctas(&pages, r, &source);
            p::estimate_usd(&p::judge_request(
                &model,
                &source,
                &value_props[r],
                &ctas,
                &items,
            ))
        })
        .sum();
    let estimated = jobs.iter().map(|j| p::estimate_usd(&j.2)).sum::<f64>() + judge_estimate;
    let provider_name = ws.config.llm.provider.clone();
    anstream::eprintln!(
        "{BOLD}panel{BOLD:#}: {} route(s) × {} locale(s) = {} persona call(s) + {} judge call(s), {provider_name}/{model}, estimated ≤ ${estimated:.4}{}",
        routes.len(),
        dedup(pages.iter().map(|pg| pg.locale.clone())).len(),
        jobs.len(),
        routes.len(),
        budget
            .map(|b| format!(" (budget ${b:.2})"))
            .unwrap_or_default()
    );
    if let Some(b) = budget
        && estimated > b
    {
        bail!(
            "estimated ${estimated:.4} exceeds the budget ${b:.2}: raise --budget, or test fewer routes/locales"
        );
    }
    if a.dry_run {
        return Ok(ExitCode::SUCCESS);
    }

    let prov = provider::from_config(&ws.config.llm, None)?;
    let mut usage = Usage::default();
    let mut answers: Vec<Answer> = vec![];
    for chunk in jobs.chunks(CONCURRENCY) {
        let over = budget.is_some_and(|b| usage.cost_usd >= b);
        let res = futures::future::join_all(chunk.iter().map(|(_, _, req)| {
            let prov = &prov;
            async move {
                if over {
                    return Err("skipped: budget reached".to_string());
                }
                prov.complete(req.clone()).await.map_err(|e| e.to_string())
            }
        }))
        .await;
        for ((pg, persona, _), r) in chunk.iter().zip(res) {
            let (verdict, error) = match r {
                Ok(c) => {
                    usage.add(&Usage::of_completion(&c));
                    match p::parse_verdict(&c.text) {
                        Some(v) => (Some(v), None),
                        None => (None, Some("unreadable answer".to_string())),
                    }
                }
                Err(e) => (None, Some(e)),
            };
            answers.push(Answer {
                route: pg.route.clone(),
                locale: pg.locale.clone(),
                url: pg.url.clone(),
                screenshot: pg.screenshot.clone(),
                persona: persona.clone(),
                verdict,
                score: None,
                error,
            });
        }
    }
    for r in &routes {
        if budget.is_some_and(|b| usage.cost_usd >= b) {
            break;
        }
        let idx: Vec<usize> = (0..answers.len())
            .filter(|&i| answers[i].route == *r && answers[i].verdict.is_some())
            .collect();
        if idx.is_empty() {
            continue;
        }
        let items: Vec<JudgeItem> = idx
            .iter()
            .map(|&i| JudgeItem {
                id: i.to_string(),
                verdict: answers[i].verdict.as_ref().unwrap(),
            })
            .collect();
        let req = p::judge_request(
            &model,
            &source,
            &value_props[r],
            &source_ctas(&pages, r, &source),
            &items,
        );
        match prov.complete(req).await {
            Ok(c) => {
                usage.add(&Usage::of_completion(&c));
                for (id, s) in p::parse_judge(&c.text) {
                    if let Some(a) = id.parse::<usize>().ok().and_then(|i| answers.get_mut(i)) {
                        a.score = Some(s);
                    }
                }
            }
            Err(e) => anstream::eprintln!("{YELLOW}judge failed for {r}:{YELLOW:#} {e}"),
        }
    }

    let (locale_scores, route_scores) = aggregate(&answers, &locales, &source);
    let problems = problems(&ws, &answers, &pages);
    let mut report = Report {
        source_locale: source.clone(),
        provider: provider_name,
        model,
        estimated_usd: estimated,
        spent_usd: usage.cost_usd,
        budget_usd: budget,
        value_propositions: value_props,
        locales: locale_scores,
        routes: route_scores,
        problems,
        answers,
        annotations: None,
    };
    if a.annotate {
        report.annotations = Some(annotate(&ws, &report.problems).await);
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    std::fs::write(out.join("report.html"), html(&report, &out))?;

    if a.json {
        crate::print_json(&report)?;
    } else {
        print_summary(&report, &a.out);
    }
    Ok(ExitCode::SUCCESS)
}

fn capture(ws: &Workspace, a: &PanelArgs, locales: &[String]) -> Result<()> {
    let mut c = crate::fit::node_tool("agent-qa");
    c.current_dir(&ws.root)
        .arg("panel")
        .args(["--locales", &locales.join(",")])
        .args(["--source-locale", &ws.config.source_locale])
        .args(["--base-url", &a.base_url])
        .args(["--url-pattern", &a.url_pattern])
        .args(["--catalogs", &ws.config.files.join(",")])
        .args(["--viewport", &a.viewport])
        .arg("--config")
        .arg(&a.config)
        .arg("--out")
        .arg(&a.out);
    if !a.routes.is_empty() {
        c.args(["--routes", &a.routes.join(",")]);
    }
    if let Some(k) = &a.locale_cookie {
        c.args(["--locale-cookie", k]);
    }
    let s = c
        .status()
        .with_context(|| crate::fit::tool_missing("agent-qa"))?;
    if !s.success() {
        bail!("agent-qa panel capture failed");
    }
    Ok(())
}

fn dedup(it: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    for s in it {
        if !v.contains(&s) {
            v.push(s);
        }
    }
    v
}

/// The config's `value_proposition` of the route, else what the source page says first (h1 — description).
fn value_proposition(config: &Value, pages: &[Page], route: &str, source: &str) -> String {
    config["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["route"] == route)
        .and_then(|r| r["value_proposition"].as_str())
        .map(String::from)
        .or_else(|| {
            pages
                .iter()
                .find(|pg| pg.route == route && pg.locale == source)
                .map(|pg| {
                    if pg.headline.is_empty() {
                        pg.title.clone()
                    } else {
                        pg.headline.clone()
                    }
                })
        })
        .unwrap_or_default()
}

fn source_ctas(pages: &[Page], route: &str, source: &str) -> Vec<p::Cta> {
    pages
        .iter()
        .find(|pg| pg.route == route && pg.locale == source)
        .map(|pg| pg.ctas.clone())
        .unwrap_or_default()
}

fn mean(v: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = v.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

type RouteScores = BTreeMap<String, BTreeMap<String, Option<f64>>>;

fn aggregate(
    answers: &[Answer],
    locales: &[String],
    source: &str,
) -> (BTreeMap<String, LocaleScore>, RouteScores) {
    let mut by_locale: BTreeMap<String, LocaleScore> = locales
        .iter()
        .map(|l| {
            let of = || answers.iter().filter(move |a| a.locale == *l);
            let s = LocaleScore {
                comprehension: mean(
                    of().filter_map(|a| a.score.as_ref())
                        .map(Score::comprehension),
                ),
                debt: None,
                trust: mean(
                    of().filter_map(|a| a.verdict.as_ref())
                        .map(|v| v.trust.score as f64),
                ),
                answers: of().filter(|a| a.verdict.is_some()).count(),
            };
            (l.clone(), s)
        })
        .collect();
    let base = by_locale.get(source).and_then(|s| s.comprehension);
    for s in by_locale.values_mut() {
        s.debt = base.zip(s.comprehension).map(|(b, c)| b - c);
    }
    let mut by_route: RouteScores = BTreeMap::new();
    for a in answers {
        by_route.entry(a.route.clone()).or_default();
    }
    for (r, m) in by_route.iter_mut() {
        for l in locales {
            m.insert(
                l.clone(),
                mean(
                    answers
                        .iter()
                        .filter(|a| a.route == *r && a.locale == *l)
                        .filter_map(|a| a.score.as_ref())
                        .map(Score::comprehension),
                ),
            );
        }
    }
    (by_locale, by_route)
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Literal parts of a message around top-level `{…}` arguments, normalized.
fn literals(msg: &str) -> Vec<String> {
    let (mut parts, mut cur, mut depth) = (vec![], String::new(), 0i32);
    for c in msg.chars() {
        match c {
            '{' => {
                if depth == 0 {
                    parts.push(std::mem::take(&mut cur));
                }
                depth += 1;
            }
            '}' => depth -= 1,
            _ if depth == 0 => cur.push(c),
            _ => {}
        }
    }
    parts.push(cur);
    parts
        .iter()
        .map(|s| norm(s))
        .filter(|s| !s.is_empty())
        .collect()
}

/// The catalog key whose message renders as `quote` (placeholders filled), contains it, or is the
/// largest message inside it. ponytail: plural/select branches are matched as one block, not per branch.
fn find_key<'a>(cat: &'a BTreeMap<String, String>, quote: &str) -> Option<(&'a str, &'a str)> {
    let q = norm(quote);
    if q.chars().count() < 2 {
        return None;
    }
    let mut best: Option<(usize, &str, &str)> = None;
    for (k, v) in cat {
        let lits = literals(v);
        let len: usize = lits.iter().map(|l| l.chars().count()).sum();
        if len < 2 {
            continue;
        }
        let in_order = || {
            let mut at = 0;
            lits.iter().all(|l| match q[at..].find(l.as_str()) {
                Some(i) => {
                    at += i + l.len();
                    true
                }
                None => false,
            })
        };
        let score = if lits.len() == 1 && lits[0] == q {
            usize::MAX
        } else if in_order() {
            len * 2
        } else if lits.len() == 1 && lits[0].contains(&q) {
            q.chars().count()
        } else {
            continue;
        };
        if best.is_none_or(|b| score > b.0) {
            best = Some((score, k, v));
        }
    }
    best.map(|(_, k, v)| (k, v))
}

fn problems(ws: &Workspace, answers: &[Answer], pages: &[Page]) -> Vec<Problem> {
    let empty = BTreeMap::new();
    let mut out: Vec<Problem> = vec![];
    let mut push = |pr: Problem| {
        let same = out.iter_mut().find(|o| {
            o.locale == pr.locale
                && o.kind == pr.kind
                && match (&o.key, &pr.key) {
                    (Some(a), Some(b)) => a == b,
                    _ => norm(&o.quote) == norm(&pr.quote),
                }
        });
        match same {
            Some(o) => {
                for r in pr.routes {
                    if !o.routes.contains(&r) {
                        o.routes.push(r);
                    }
                }
                for n in pr.personas {
                    if !o.personas.contains(&n) {
                        o.personas.push(n);
                    }
                }
            }
            None => out.push(pr),
        }
    };
    for a in answers {
        let Some(v) = &a.verdict else { continue };
        let cat = ws
            .catalogs
            .get(a.locale.as_str())
            .map(|c| &c.entries)
            .unwrap_or(&empty);
        let base = |kind, quote: &str, why: &str| Problem {
            locale: a.locale.clone(),
            kind,
            quote: quote.into(),
            why: why.into(),
            key: None,
            suggestion: None,
            valid: None,
            issues: vec![],
            routes: vec![a.route.clone()],
            personas: vec![a.persona.name.clone()],
        };
        let page = pages
            .iter()
            .find(|pg| pg.route == a.route && pg.locale == a.locale);
        for (kind, list) in [("non_native", &v.non_native), ("cultural", &v.cultural)] {
            for r in list {
                let mut pr = base(kind, &r.quote, &r.why);
                // A short quote ("$29") is found through the page line that holds it.
                let found = find_key(cat, &r.quote).or_else(|| {
                    let text = page.map(|pg| pg.text.as_str()).unwrap_or_default();
                    let line = text.lines().find(|l| l.contains(r.quote.trim()))?;
                    find_key(cat, line)
                });
                if let Some((key, value)) = found {
                    pr.key = Some(key.into());
                    pr.suggestion = r
                        .suggestion
                        .as_deref()
                        .and_then(|s| apply(value, &r.quote, s));
                    if let Some(s) = &pr.suggestion {
                        let issues = check(ws, &a.locale, key, s);
                        pr.valid = Some(!issues.iter().any(|i| i.0));
                        pr.issues = issues.into_iter().map(|i| i.1).collect();
                    }
                }
                push(pr);
            }
        }
        let missed = a
            .score
            .as_ref()
            .map(|s| s.missed.clone())
            .unwrap_or_default();
        for c in &v.ctas {
            if c.sure && !missed.contains(&c.id) {
                continue;
            }
            let Some(cta) = page.and_then(|pg| pg.ctas.iter().find(|x| x.id == c.id)) else {
                continue;
            };
            let why = format!(
                "Expected: {}",
                if c.expects_source.is_empty() {
                    &c.expects
                } else {
                    &c.expects_source
                }
            );
            let mut pr = base("cta", &cta.text, &why);
            pr.key = cta.key.clone();
            push(pr);
        }
    }
    let rank = |p: &Problem| {
        (
            std::cmp::Reverse(p.personas.len() * p.routes.len()),
            p.valid != Some(true),
            p.key.is_none(),
            ["non_native", "cta", "cultural"]
                .iter()
                .position(|k| *k == p.kind),
        )
    };
    out.sort_by_key(rank);
    out
}

/// The full value with `quote` replaced by `suggestion` (the persona quotes what it saw, often a part).
fn apply(value: &str, quote: &str, suggestion: &str) -> Option<String> {
    let (s, q) = (suggestion.trim(), quote.trim());
    if s.is_empty() {
        return None;
    }
    if norm(value) == norm(q) {
        return Some(s.into());
    }
    value.contains(q).then(|| value.replacen(q, s, 1))
}

/// (is_error, message) of the glot validator + glossary for a proposed value.
fn check(ws: &Workspace, locale: &str, key: &str, value: &str) -> Vec<(bool, String)> {
    let Some(src) = ws.source().entries.get(key) else {
        return vec![(true, "key missing from the source catalog".into())];
    };
    let c = Constraints {
        max_length: None,
        max_length_ratio: Some(ws.config.qa.max_length_ratio),
    };
    let mut issues = validate::validate(src, value, locale, ws.config.syntax, &c);
    issues.extend(ws.glossary.check(src, value, locale));
    issues
        .into_iter()
        .map(|i| (validate::has_errors(std::slice::from_ref(&i)), i.message))
        .collect()
}

async fn annotate(ws: &Workspace, problems: &[Problem]) -> Value {
    let api = match glot_mcp::annotations::ReviewApi::from_workspace(ws) {
        Ok(a) => a,
        Err(e) => {
            anstream::eprintln!("{YELLOW}--annotate skipped:{YELLOW:#} {e}");
            return json!({"posted": 0, "error": e});
        }
    };
    let (mut posted, mut errors) = (0, vec![]);
    for pr in problems
        .iter()
        .filter(|p| p.key.is_some())
        .take(MAX_ANNOTATIONS)
    {
        let key = pr.key.as_deref().unwrap_or_default();
        let route = pr.routes.first().map(String::as_str).unwrap_or("/");
        let text = ws
            .catalogs
            .get(pr.locale.as_str())
            .and_then(|c| c.entries.get(key))
            .cloned()
            .unwrap_or_else(|| pr.quote.clone());
        // Same anchor as `glot annotations add`: the widget re-anchors on [data-glot-key].
        let anchor = json!({
            "key": key, "key_source": "manual", "selector": format!("[data-glot-key=\"{key}\"]"),
            "text": text, "url": format!("glot://cli/{}{}", pr.locale, if route == "/" { "" } else { route }),
            "locale": pr.locale, "viewport": { "w": 0, "h": 0 },
        });
        let body = format!(
            "[glot panel · {} · {}] {}",
            pr.kind,
            pr.personas.join(", "),
            pr.why
        );
        let suggestion = pr.suggestion.as_ref().filter(|_| pr.valid == Some(true));
        match api
            .post(
                "/annotations",
                json!({ "anchor": anchor, "body": body, "suggestion": suggestion }),
            )
            .await
        {
            Ok(_) => posted += 1,
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        anstream::eprintln!(
            "{YELLOW}annotations:{YELLOW:#} {} failed ({})",
            errors.len(),
            errors[0]
        );
    }
    json!({ "posted": posted, "errors": errors })
}

fn print_summary(r: &Report, out: &Path) {
    let f = |v: Option<f64>, d: usize| v.map(|x| format!("{x:.d$}")).unwrap_or("–".into());
    anstream::println!(
        "{BOLD}{:<8} {:>13} {:>6} {:>7}{BOLD:#}",
        "locale",
        "comprehension",
        "debt",
        "trust"
    );
    for (l, s) in &r.locales {
        let c = match s.debt {
            Some(d) if d >= 15.0 => RED,
            Some(d) if d >= 5.0 => YELLOW,
            _ => GREEN,
        };
        anstream::println!(
            "{:<8} {c}{:>13}{c:#} {:>6} {:>7}{}",
            l,
            f(s.comprehension, 0),
            f(s.debt, 0),
            f(s.trust, 1),
            if *l == r.source_locale {
                "  (baseline)"
            } else {
                ""
            }
        );
    }
    for pr in r.problems.iter().take(8) {
        anstream::println!(
            "{YELLOW}{:<10}{YELLOW:#} {:<5} {} {DIM}{}{DIM:#} — {}",
            pr.kind,
            pr.locale,
            pr.key.as_deref().unwrap_or("(no key)"),
            pr.quote,
            pr.why
        );
        if let (Some(s), Some(true)) = (&pr.suggestion, pr.valid) {
            anstream::println!("           {GREEN}→ {s}{GREEN:#}");
        }
    }
    anstream::println!(
        "{DIM}{} problem(s), ${:.4} spent — {}{DIM:#}",
        r.problems.len(),
        r.spent_usd,
        out.join("report.html").display()
    );
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One self-contained page: screenshots inlined, no script, readable without colour.
fn html(r: &Report, out: &Path) -> String {
    let f = |v: Option<f64>, d: usize| v.map(|x| format!("{x:.d$}")).unwrap_or("–".into());
    let mut h = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>glot panel report</title><style>
:root{--bg:#fff;--fg:#1a1a1a;--mute:#555;--line:#ddd;--bad:#a30000;--warn:#7a4b00;--ok:#0b6b2b}
@media (prefers-color-scheme:dark){:root{--bg:#141414;--fg:#eee;--mute:#aaa;--line:#333;--bad:#ff8080;--warn:#ffc266;--ok:#7ddc9a}}
body{background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif;max-width:72rem;margin:0 auto;padding:1rem}
table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{border:1px solid var(--line);padding:.4rem .6rem;text-align:left;vertical-align:top}
.n{text-align:right;font-variant-numeric:tabular-nums}.bad{color:var(--bad)}.warn{color:var(--warn)}.ok{color:var(--ok)}
.mute{color:var(--mute)}img{max-width:100%;height:auto;border:1px solid var(--line)}details{border:1px solid var(--line);margin:.6rem 0;padding:.4rem .8rem}
summary{cursor:pointer;font-weight:600}q{font-style:italic}code{font-size:.9em}
a:focus,summary:focus{outline:3px solid currentColor;outline-offset:2px}
</style></head><body><main>
<h1>Synthetic user panel</h1>
"#,
    );
    h += &format!(
        "<p class=\"mute\">Provider <code>{}</code>, model <code>{}</code> · estimated ${:.4}, spent ${:.4}{}. Personas are synthetic: treat their verdicts as leads to check, not as user research.</p>",
        esc(&r.provider),
        esc(&r.model),
        r.estimated_usd,
        r.spent_usd,
        r.budget_usd
            .map(|b| format!(" of ${b:.2}"))
            .unwrap_or_default()
    );
    h += "<h2>Comprehension by locale</h2><table><caption class=\"mute\">Comprehension = 5-second test and call-to-action understanding (0–100). Localization debt = source score minus locale score.</caption><thead><tr><th scope=\"col\">Locale</th><th scope=\"col\" class=\"n\">Comprehension</th><th scope=\"col\" class=\"n\">Localization debt</th><th scope=\"col\" class=\"n\">Trust (1–5)</th><th scope=\"col\" class=\"n\">Answers</th></tr></thead><tbody>";
    for (l, s) in &r.locales {
        let (cls, word) = match s.debt {
            Some(d) if d >= 15.0 => ("bad", " (high)"),
            Some(d) if d >= 5.0 => ("warn", " (medium)"),
            Some(_) => ("ok", ""),
            None => ("mute", ""),
        };
        h += &format!(
            "<tr><th scope=\"row\">{}{}</th><td class=\"n\">{}</td><td class=\"n {cls}\">{}{word}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
            esc(l),
            if *l == r.source_locale {
                " <span class=\"mute\">(baseline)</span>"
            } else {
                ""
            },
            f(s.comprehension, 0),
            f(s.debt, 0),
            f(s.trust, 1),
            s.answers
        );
    }
    h += "</tbody></table>";
    if !r.routes.is_empty() {
        let locs: Vec<&String> = r.locales.keys().collect();
        h += "<h2>By page</h2><table><thead><tr><th scope=\"col\">Route</th><th scope=\"col\">Expected value proposition</th>";
        for l in &locs {
            h += &format!("<th scope=\"col\" class=\"n\">{}</th>", esc(l));
        }
        h += "</tr></thead><tbody>";
        for (route, m) in &r.routes {
            h += &format!(
                "<tr><th scope=\"row\"><code>{}</code></th><td>{}</td>",
                esc(route),
                esc(r
                    .value_propositions
                    .get(route)
                    .map(String::as_str)
                    .unwrap_or(""))
            );
            for l in &locs {
                h += &format!(
                    "<td class=\"n\">{}</td>",
                    f(m.get(*l).copied().flatten(), 0)
                );
            }
            h += "</tr>";
        }
        h += "</tbody></table>";
    }
    h += &format!("<h2>Top problems ({})</h2>", r.problems.len());
    if r.problems.is_empty() {
        h += "<p>None reported.</p>";
    } else {
        h += "<table><thead><tr><th scope=\"col\">Locale</th><th scope=\"col\">Kind</th><th scope=\"col\">Text on the page</th><th scope=\"col\">Key</th><th scope=\"col\">Why</th><th scope=\"col\">Suggestion</th><th scope=\"col\">Seen by</th></tr></thead><tbody>";
        for pr in &r.problems {
            let sugg = match (&pr.suggestion, pr.valid) {
                (Some(s), Some(true)) => format!("<span class=\"ok\">✓ valid</span> {}", esc(s)),
                (Some(s), _) => format!(
                    "<span class=\"bad\">✗ rejected</span> {} <span class=\"mute\">{}</span>",
                    esc(s),
                    esc(&pr.issues.join("; "))
                ),
                _ => "<span class=\"mute\">–</span>".into(),
            };
            h += &format!(
                "<tr><td>{}</td><td>{}</td><td><q>{}</q></td><td><code>{}</code></td><td>{}</td><td>{sugg}</td><td>{} <span class=\"mute\">on {}</span></td></tr>",
                esc(&pr.locale),
                pr.kind.replace('_', "-"),
                esc(&pr.quote),
                esc(pr.key.as_deref().unwrap_or("–")),
                esc(&pr.why),
                esc(&pr.personas.join(", ")),
                esc(&pr.routes.join(", "))
            );
        }
        h += "</tbody></table>";
    }
    h += "<h2>Answers</h2>";
    for a in &r.answers {
        let pers = &a.persona;
        h += &format!(
            "<details><summary><code>{}</code> · {} · {}, {}, {} — comprehension {}</summary>",
            esc(&a.route),
            esc(&a.locale),
            esc(&pers.name),
            pers.age,
            esc(&pers.occupation),
            f(a.score.as_ref().map(Score::comprehension), 0)
        );
        if let Ok(b64) = glot_llm::visual::png_b64(&out.join(&a.screenshot)) {
            h += &format!(
                "<img src=\"data:image/png;base64,{b64}\" alt=\"Screenshot of {} in {} as the persona saw it (above the fold)\" loading=\"lazy\">",
                esc(&a.route),
                esc(&a.locale)
            );
        }
        h += &format!(
            "<p class=\"mute\">Market conventions: {}</p>",
            esc(&pers.expectations)
        );
        match &a.verdict {
            None => {
                h += &format!(
                    "<p class=\"bad\">No answer: {}</p>",
                    esc(a.error.as_deref().unwrap_or("?"))
                )
            }
            Some(v) => {
                h += &format!(
                    "<p><strong>5-second test:</strong> <q>{}</q>{}</p><p><strong>Trust:</strong> {}/5 — {}</p>",
                    esc(&v.five_second.answer),
                    if v.five_second.answer_source != v.five_second.answer {
                        format!(
                            " <span class=\"mute\">({})</span>",
                            esc(&v.five_second.answer_source)
                        )
                    } else {
                        String::new()
                    },
                    v.trust.score,
                    esc(&v.trust.reason)
                );
                if !v.ctas.is_empty() {
                    h += "<ul>";
                    for c in &v.ctas {
                        h += &format!(
                            "<li>CTA {}: {}{}</li>",
                            c.id,
                            esc(&c.expects),
                            if c.sure {
                                ""
                            } else {
                                " <span class=\"warn\">(unsure)</span>"
                            }
                        );
                    }
                    h += "</ul>";
                }
            }
        }
        h += "</details>";
    }
    h += "</main></body></html>\n";
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_keys_from_quotes() {
        let cat: BTreeMap<String, String> = [
            ("hero.title", "Faites-vous payer plus vite"),
            ("greet", "Bonjour, {name} !"),
            ("nav.pricing", "Tarifs"),
            ("cta", "Voir les offres →"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        assert_eq!(
            find_key(&cat, "Faites-vous  payer plus vite").unwrap().0,
            "hero.title"
        );
        assert_eq!(find_key(&cat, "Bonjour, Ada !").unwrap().0, "greet");
        assert_eq!(find_key(&cat, "payer plus vite").unwrap().0, "hero.title");
        assert_eq!(find_key(&cat, "Voir les offres →").unwrap().0, "cta");
        assert!(find_key(&cat, "Rien à voir").is_none());
        assert_eq!(
            apply(
                "Faites-vous payer plus vite",
                "payer plus vite",
                "être payé plus vite"
            ),
            Some("Faites-vous être payé plus vite".into())
        );
        assert_eq!(apply("Tarifs", "tarifs", "Prix"), Some("Prix".into()));
        assert_eq!(
            apply("Bonjour, {name} !", "Bonjour, Ada !", "Salut Ada"),
            None
        );
    }

    fn answer(locale: &str, comp: (f64, Option<f64>), trust: u8) -> Answer {
        Answer {
            route: "/".into(),
            locale: locale.into(),
            url: String::new(),
            screenshot: String::new(),
            persona: p::default_persona(locale),
            verdict: Some(Verdict {
                trust: p::Trust {
                    score: trust,
                    reason: String::new(),
                },
                ..Default::default()
            }),
            score: Some(Score {
                five_second: comp.0,
                ctas: comp.1,
                missed: vec![],
            }),
            error: None,
        }
    }

    #[test]
    fn debt_is_source_minus_locale() {
        let a = [
            answer("en", (1.0, Some(1.0)), 5),
            answer("fr", (0.5, Some(0.5)), 2),
            answer("ja", (0.8, None), 4),
        ];
        let locs: Vec<String> = ["en", "fr", "ja", "de"].map(String::from).into();
        let (l, r) = aggregate(&a, &locs, "en");
        assert_eq!(l["en"].debt, Some(0.0));
        assert_eq!(l["fr"].comprehension, Some(50.0));
        assert_eq!(l["fr"].debt, Some(50.0));
        assert_eq!(l["ja"].debt.map(|d| d.round()), Some(20.0));
        assert_eq!(l["de"].debt, None);
        assert_eq!(r["/"]["fr"], Some(50.0));
        let h = html(
            &Report {
                source_locale: "en".into(),
                provider: "mock".into(),
                model: "mock".into(),
                estimated_usd: 0.0,
                spent_usd: 0.0,
                budget_usd: None,
                value_propositions: BTreeMap::new(),
                locales: l,
                routes: r,
                problems: vec![],
                answers: a.into(),
                annotations: None,
            },
            Path::new("/nonexistent"),
        );
        assert!(h.contains("50 (high)") && h.contains("<html lang=\"en\">"));
    }
}
