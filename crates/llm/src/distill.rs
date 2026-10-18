//! `glot labs distill`: the project's reviewed translations become (1) a fine-tuning dataset (chat
//! JSONL, train/eval split), (2) few-shot retrieval for a free local model (RAG: the k nearest
//! approved pairs + glossary in every request, inside the guarded pipeline), (3) an honest eval of
//! local vs local+RAG vs the configured cloud model on the held-out split, ending in a routing proposal.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use glot_core::{
    Key, Locale,
    config::LlmConfig,
    context::KeyContext,
    glossary::Term,
    hash::hash,
    lock::Origin,
    validate::{self, Constraints},
};

use crate::{
    Error, Result,
    engine::{TranslateOptions, TranslateReport, Workspace},
    judge::{self, JudgeItem},
    provider::{self, Provider},
    tm::{Tm, Unit, normalize},
    translate::{Item, Request, Translated, Translator, Usage},
};

pub const DIR: &str = ".glot/distill";
const BEGIN: &str = "# >>> glot labs distill";
const END: &str = "# <<< glot labs distill";

/// One approved source → target pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub key: Key,
    pub source_locale: Locale,
    pub target_locale: Locale,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Dataset {
    pub train: Vec<Pair>,
    pub eval: Vec<Pair>,
    pub skipped_secret: usize,
    pub skipped_invalid: usize,
    pub duplicates: usize,
}

/// Reviewed (or hand-written) translations still current in the catalogs and glot.lock, valid,
/// without secrets, deduplicated; `eval_pct` % of them held out (stable hash split).
pub fn collect(ws: &Workspace, eval_pct: u32) -> Dataset {
    let mut ds = Dataset::default();
    let src_loc = &ws.config.source_locale;
    let source = ws.source();
    let mut seen = BTreeSet::new();
    for loc in &ws.config.target_locales {
        let (Some(entries), Some(cat)) = (ws.lock.entries.get(loc), ws.catalogs.get(loc)) else {
            continue;
        };
        for (key, e) in entries {
            if !(e.reviewed || e.origin == Origin::Human) || e.needs_review.is_some() {
                continue;
            }
            let (Some(s), Some(t)) = (source.entries.get(key), cat.entries.get(key)) else {
                continue;
            };
            // Stale (source changed) or edited since review: not what a human approved.
            if s.is_empty() || hash(s) != e.source_hash || hash(t) != e.target_hash {
                continue;
            }
            if looks_secret(s) || looks_secret(t) {
                ds.skipped_secret += 1;
                continue;
            }
            let c = Constraints::default();
            if validate::has_errors(&validate::validate(s, t, loc, ws.config.syntax, &c))
                || validate::has_errors(&ws.glossary.check(s, t, loc))
            {
                ds.skipped_invalid += 1;
                continue;
            }
            let note = ws.lock.notes.get(key).cloned();
            if !seen.insert((loc.clone(), normalize(s), note.clone())) {
                ds.duplicates += 1;
                continue;
            }
            let p = Pair {
                key: key.clone(),
                source_locale: src_loc.clone(),
                target_locale: loc.clone(),
                source: s.clone(),
                target: t.clone(),
                note,
            };
            let bucket = u32::from_str_radix(&hash(&format!("{loc}\n{}", normalize(s)))[..4], 16)
                .unwrap_or(0)
                % 100;
            if bucket < eval_pct {
                ds.eval.push(p);
            } else {
                ds.train.push(p);
            }
        }
    }
    // A tiny project still gets something to evaluate on.
    if eval_pct > 0 && ds.eval.is_empty() && ds.train.len() >= 2 {
        ds.eval.extend(ds.train.pop());
    }
    ds
}

/// API keys, tokens, private keys: never exported nor sent as examples.
pub fn looks_secret(s: &str) -> bool {
    const PREFIXES: [&str; 11] = [
        "sk-",
        "sk_live_",
        "pk_live_",
        "rk_live_",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "glpat-",
    ];
    s.contains("-----BEGIN")
        || s.split(|c: char| c.is_whitespace() || "\"'`=:,;()<>[]{}".contains(c))
            .any(|t| {
                (t.len() >= 16 && (PREFIXES.iter().any(|p| t.starts_with(p)) || t.starts_with("AIza")))
                    // ponytail: shape heuristic (long mixed alnum token), not an entropy test.
                    || (t.len() >= 32
                        && t.chars().all(|c| c.is_ascii_alphanumeric() || "-_+/".contains(c))
                        && t.chars().any(|c| c.is_ascii_digit())
                        && t.chars().any(|c| c.is_ascii_alphabetic()))
            })
}

fn glossary_lines(terms: &[&Term], loc: &str) -> String {
    terms
        .iter()
        .map(|t| match (t.do_not_translate, t.translations.get(loc)) {
            (true, _) => format!("- \"{}\": keep verbatim", t.term),
            (_, Some(tr)) => format!("- \"{}\" => \"{tr}\"", t.term),
            _ => format!("- \"{}\"", t.term),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Standard fine-tuning record: `{"messages": [system, user, assistant]}`.
pub fn chat(ws: &Workspace, p: &Pair) -> Value {
    let mut system = format!(
        "You translate user-interface strings of this product from {} to {}. Keep placeholders, ICU plural/select syntax and markup tags exactly; answer with the translation only.",
        p.source_locale, p.target_locale
    );
    if let Some(style) = ws
        .config
        .llm
        .style
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        system += &format!("\nStyle: {style}");
    }
    let terms = ws.glossary.matches(&p.source);
    if !terms.is_empty() {
        system += &format!("\nGlossary:\n{}", glossary_lines(&terms, &p.target_locale));
    }
    let user = match &p.note {
        Some(n) => format!("{}\n\nContext: {n}", p.source),
        None => p.source.clone(),
    };
    json!({"messages": [
        {"role": "system", "content": system},
        {"role": "user", "content": user},
        {"role": "assistant", "content": p.target},
    ]})
}

/// `train.jsonl`, `eval.jsonl` and `pairs.json` (eval references) under `dir`.
pub fn write(ws: &Workspace, ds: &Dataset, dir: &Path) -> Result<()> {
    let io = |e: std::io::Error| Error::Other(format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    for (name, pairs) in [("train", &ds.train), ("eval", &ds.eval)] {
        let text: String = pairs
            .iter()
            .map(|p| chat(ws, p).to_string() + "\n")
            .collect();
        std::fs::write(dir.join(format!("{name}.jsonl")), text).map_err(io)?;
    }
    let pairs = serde_json::to_string_pretty(ds).unwrap_or_default();
    std::fs::write(dir.join("pairs.json"), pairs + "\n").map_err(io)
}

/// Retrieval pool: train pairs + validated TM units, minus anything whose source is in `exclude`
/// (the eval split: no leak of the answer into the prompt).
pub fn pool(ws: &Workspace, train: &[Pair], exclude: &[Pair]) -> Vec<Pair> {
    let banned: BTreeSet<(String, String)> = exclude
        .iter()
        .map(|p| (p.target_locale.clone(), normalize(&p.source)))
        .collect();
    let tm = Tm::load(&ws.root).unwrap_or_default();
    let mut out: Vec<Pair> = train.to_vec();
    let mut have: BTreeSet<(String, String)> = out
        .iter()
        .map(|p| (p.target_locale.clone(), normalize(&p.source)))
        .collect();
    for u in tm.units {
        let id = (u.target_locale.clone(), normalize(&u.source));
        if u.source_locale != ws.config.source_locale
            || banned.contains(&id)
            || looks_secret(&u.source)
            || looks_secret(&u.target)
            || !have.insert(id)
        {
            continue;
        }
        out.push(Pair {
            key: String::new(),
            source_locale: u.source_locale,
            target_locale: u.target_locale,
            source: u.source,
            target: u.target,
            note: u.context,
        });
    }
    out.retain(|p| !banned.contains(&(p.target_locale.clone(), normalize(&p.source))));
    out
}

/// The `k` pool pairs of `locale` lexically closest to `source` (character-bigram Dice).
// ponytail: linear scan + bigram Dice; local embeddings (`/v1/embeddings`) if lexical recall proves too weak.
pub fn nearest<'a>(pool: &'a [Pair], locale: &str, source: &str, k: usize) -> Vec<&'a Pair> {
    let n = normalize(source).to_lowercase();
    let mut scored: Vec<(f64, &Pair)> = pool
        .iter()
        .filter(|p| p.target_locale == locale)
        .map(|p| {
            (
                strsim::sorensen_dice(&normalize(&p.source).to_lowercase(), &n),
                p,
            )
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(k).map(|(_, p)| p).collect()
}

/// A translation request for `items` (key, source, previous), glossary + style of the project,
/// and with `pool` the k nearest approved pairs per item (few-shot, in `description`, like the TM).
pub fn request(
    ws: &Workspace,
    locale: &str,
    items: &[(Key, String, Option<String>)],
    pool: Option<&[Pair]>,
    k: usize,
) -> Request {
    let items: Vec<Item> = items
        .iter()
        .map(|(key, source, previous)| {
            let mut description = ws.lock.notes.get(key).cloned();
            let examples: Vec<String> = pool
                .map(|p| nearest(p, locale, source, k))
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p.source != *source)
                .map(|p| format!("- {:?} => {:?}", p.source, p.target))
                .collect();
            if !examples.is_empty() {
                let ex = format!(
                    "Approved translations of OTHER strings of this project, for terminology and tone only (translate `source` itself, never copy an example):\n{}",
                    examples.join("\n")
                );
                description = Some(match description {
                    Some(d) => format!("{d}\n{ex}"),
                    None => ex,
                });
            }
            Item {
                key: key.clone(),
                source: source.clone(),
                context: KeyContext {
                    key: key.clone(),
                    description,
                    ..Default::default()
                },
                previous: previous.clone(),
            }
        })
        .collect();
    let mut glossary: Vec<Term> = vec![];
    for i in &items {
        for t in ws.glossary.matches(&i.source) {
            if !glossary.iter().any(|g| g.term == t.term) {
                glossary.push(t.clone());
            }
        }
    }
    Request {
        source_locale: ws.config.source_locale.clone(),
        target_locale: locale.to_string(),
        items,
        glossary,
        syntax: ws.config.syntax,
        style: ws.config.llm.style.clone(),
    }
}

/// Key class used for routing: ICU messages, short labels (≤ 4 words), sentences.
pub fn class(source: &str) -> &'static str {
    if ["plural,", "select,", "selectordinal,"]
        .iter()
        .any(|k| source.contains(k))
    {
        "icu"
    } else if source.split_whitespace().count() <= 4 {
        "label"
    } else {
        "sentence"
    }
}

/// Local model config (`provider` local / openai-compatible / mock); with `escalate`, the project's
/// cloud model becomes the fallback (cross-provider, `routing::fallback_provider`).
pub fn local_config(
    cloud: &LlmConfig,
    provider: &str,
    model: &str,
    base_url: Option<String>,
    escalate: bool,
) -> LlmConfig {
    LlmConfig {
        provider: provider.into(),
        model: model.into(),
        base_url,
        api_key_env: None,
        judge_model: None,
        budget_usd: cloud.budget_usd,
        fallback_model: escalate.then(|| cloud.model.clone()),
        fallback_provider: escalate.then(|| cloud.provider.clone()),
        fallback_base_url: cloud.base_url.clone(),
        fallback_api_key_env: cloud.api_key_env.clone(),
        ..cloud.clone()
    }
}

async fn run(ws: &Workspace, llm: &LlmConfig, req: Request) -> Result<(Vec<Translated>, Usage)> {
    let mut cfg = ws.config.clone();
    cfg.llm = llm.clone();
    Translator::new(&cfg, provider::from_config(llm, None)?)
        .with_fallback_provider(crate::routing::fallback_provider(llm)?)
        .translate(req)
        .await
}

/// What `translate` did: local first pass (+ escalation), written through the TM.
#[derive(Debug, Default)]
pub struct Outcome {
    pub local: usize,
    pub escalated: usize,
    pub rejected: Vec<(Locale, Translated)>,
    pub usage: Usage,
    pub report: TranslateReport,
}

/// Translate the plan (missing + stale) with the local model + RAG through the guarded pipeline,
/// then write what validated via `Workspace::translate` (exact TM hits, no further LLM call).
pub async fn translate(
    ws: &mut Workspace,
    llm: &LlmConfig,
    k: usize,
    locales: Option<Vec<Locale>>,
    only_class: Option<&str>,
) -> Result<Outcome> {
    let plan = ws
        .translate(TranslateOptions {
            locales: locales.clone(),
            dry_run: true,
            ..Default::default()
        })
        .await?
        .planned;
    let ds = collect(ws, 0);
    let pool = pool(ws, &ds.train, &[]);
    let mut tm = Tm::load(&ws.root)?;
    let mut out = Outcome::default();
    let mut accepted: Vec<Key> = vec![];
    for (loc, keys) in plan {
        let items: Vec<(Key, String, Option<String>)> = keys
            .into_iter()
            .filter_map(|k| {
                let s = ws.source().entries.get(&k)?.clone();
                let prev = ws
                    .catalogs
                    .get(&loc)
                    .and_then(|c| c.entries.get(&k))
                    .cloned();
                Some((k, s, prev))
            })
            .filter(|(_, s, _)| only_class.is_none_or(|c| class(s) == c))
            .collect();
        if items.is_empty() {
            continue;
        }
        let (res, u) = run(ws, llm, request(ws, &loc, &items, Some(&pool), k)).await?;
        out.usage.add(&u);
        for t in res {
            if validate::has_errors(&t.issues) {
                out.rejected.push((loc.clone(), t));
                continue;
            }
            if t.model == llm.model {
                out.local += 1;
            } else {
                out.escalated += 1;
            }
            let source = ws.source().entries[&t.key].clone();
            // ponytail: written as exact TM hits, so glot.lock says model "tm" rather than the local model.
            tm.add(Unit {
                source_locale: ws.config.source_locale.clone(),
                target_locale: loc.clone(),
                source,
                target: t.value,
                context: ws.lock.notes.get(&t.key).cloned(),
            });
            accepted.push(t.key);
        }
    }
    if accepted.is_empty() {
        return Ok(out);
    }
    tm.save(&ws.root)?;
    accepted.sort();
    accepted.dedup();
    out.report = ws
        .translate(TranslateOptions {
            locales,
            keys: Some(accepted),
            ..Default::default()
        })
        .await?;
    Ok(out)
}

/// One evaluated lane: a model, with or without retrieval.
pub struct Lane {
    pub name: String,
    pub llm: LlmConfig,
    pub rag: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ClassScore {
    pub n: usize,
    pub valid: usize,
    /// Items in the judge mean: invalid ones (0) and valid ones the judge returned a verdict for.
    pub judged: usize,
    pub judge: f64,
    pub ref_sim: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Score {
    pub lane: String,
    pub model: String,
    pub n: usize,
    /// Output passing the validator after the pipeline's retries.
    pub valid: usize,
    /// Valid on the first attempt.
    pub first_pass: usize,
    /// Mean judge score 0-100 (invalid output counts 0, valid output without a verdict is left
    /// out); None without a judge.
    pub judge: Option<f64>,
    /// Items in the judge mean (see `ClassScore::judged`).
    pub judged: usize,
    /// Mean character-bigram similarity to the human reference, 0-1 (invalid output counts 0).
    pub ref_sim: f64,
    pub cost_usd: f64,
    pub secs: f64,
    pub classes: BTreeMap<String, ClassScore>,
}

impl Score {
    /// The quality figure compared across lanes: the judge when there is one, else reference similarity.
    pub fn quality(&self) -> f64 {
        self.judge.unwrap_or(self.ref_sim * 100.0)
    }
}

/// Run every lane on the eval split (retrieval from `pool` only), judge the outputs.
pub async fn eval(
    ws: &Workspace,
    eval: &[Pair],
    pool: &[Pair],
    lanes: &[Lane],
    judge: Option<(&dyn Provider, &str)>,
    k: usize,
) -> Result<(Vec<Score>, Usage)> {
    let mut by_loc: BTreeMap<&str, Vec<&Pair>> = BTreeMap::new();
    for p in eval {
        by_loc.entry(&p.target_locale).or_default().push(p);
    }
    let mut judge_usage = Usage::default();
    let mut scores = vec![];
    for lane in lanes {
        let mut s = Score {
            lane: lane.name.clone(),
            model: lane.llm.model.clone(),
            ..Default::default()
        };
        let mut judge_sum = 0.0;
        for (loc, pairs) in &by_loc {
            let items: Vec<_> = pairs
                .iter()
                .map(|p| (p.key.clone(), p.source.clone(), None))
                .collect();
            let req = request(ws, loc, &items, lane.rag.then_some(pool), k);
            let t0 = Instant::now();
            let (res, u) = run(ws, &lane.llm, req).await?;
            s.secs += t0.elapsed().as_secs_f64();
            s.cost_usd += u.cost_usd;
            let mut verdicts: BTreeMap<Key, u8> = BTreeMap::new();
            if let Some((jp, jm)) = judge {
                let ji: Vec<JudgeItem> = res
                    .iter()
                    .zip(pairs)
                    .filter(|(t, _)| !validate::has_errors(&t.issues))
                    .map(|(t, p)| JudgeItem {
                        key: t.key.clone(),
                        source: p.source.clone(),
                        target: t.value.clone(),
                        context: KeyContext {
                            key: t.key.clone(),
                            description: p.note.clone(),
                            ..Default::default()
                        },
                    })
                    .collect();
                for chunk in ji.chunks(40) {
                    match judge::judge(jp, jm, &ws.config.source_locale, loc, chunk).await {
                        Ok((v, u)) => {
                            judge_usage.add(&u);
                            verdicts.extend(v.into_iter().map(|v| (v.key, v.score)));
                        }
                        Err(e) => tracing::warn!("judge failed: {e}"),
                    }
                }
            }
            for (t, p) in res.iter().zip(pairs) {
                let ok = !validate::has_errors(&t.issues);
                let sim = if ok {
                    strsim::sorensen_dice(&t.value, &p.target)
                } else {
                    0.0
                };
                let j = if ok {
                    verdicts.get(&t.key).map(|&v| v as f64)
                } else {
                    Some(0.0)
                };
                let judged = j.is_some() as usize;
                let j = j.unwrap_or(0.0);
                s.n += 1;
                s.valid += ok as usize;
                s.first_pass += (ok && t.attempts == 1) as usize;
                s.ref_sim += sim;
                judge_sum += j;
                s.judged += judged;
                let c = s.classes.entry(class(&p.source).into()).or_default();
                c.n += 1;
                c.valid += ok as usize;
                c.ref_sim += sim;
                c.judge += j;
                c.judged += judged;
            }
        }
        let n = s.n.max(1) as f64;
        s.ref_sim /= n;
        s.judge = judge.map(|_| judge_sum / s.judged.max(1) as f64);
        for c in s.classes.values_mut() {
            c.ref_sim /= c.n.max(1) as f64;
            c.judge /= c.judged.max(1) as f64;
        }
        scores.push(s);
    }
    Ok((scores, judge_usage))
}

/// Recommendation sentence + commented glot.toml block, from scores named "local", "local+rag", "cloud".
pub fn recommend(scores: &[Score], local: &LlmConfig, cloud: &LlmConfig) -> (String, String) {
    let get = |n: &str| scores.iter().find(|s| s.lane == n);
    let (Some(cloud_s), Some(best)) = (
        get("cloud"),
        // max_by keeps the last of equals: retrieval wins ties.
        ["local", "local+rag"]
            .iter()
            .filter_map(|n| get(n))
            .max_by(|a, b| a.quality().total_cmp(&b.quality())),
    ) else {
        return ("nothing to compare".into(), String::new());
    };
    let judged = cloud_s.judge.is_some();
    let cq = |c: &ClassScore| if judged { c.judge } else { c.ref_sim * 100.0 };
    let rate = |v: usize, n: usize| v as f64 / n.max(1) as f64;
    let pct = 100.0 * best.quality() / cloud_s.quality().max(1e-9);
    let mut local_classes = vec![];
    let mut cloud_classes = vec![];
    for (name, c) in &cloud_s.classes {
        let l = best.classes.get(name).cloned().unwrap_or_default();
        if cq(&l) >= 0.95 * cq(c) && rate(l.valid, l.n) >= rate(c.valid, c.n) {
            local_classes.push(name.as_str());
        } else {
            cloud_classes.push(name.as_str());
        }
    }
    let label = if best.lane == "local+rag" {
        format!("{} + RAG", local.model)
    } else {
        local.model.clone()
    };
    let head = format!(
        "{label}: ${:.4}, {pct:.0} % of {}'s {} on this project (validator {:.0} % vs {:.0} %, cloud ${:.4})",
        best.cost_usd,
        cloud.model,
        if judged {
            "judge score"
        } else {
            "reference similarity"
        },
        100.0 * rate(best.valid, best.n),
        100.0 * rate(cloud_s.valid, cloud_s.n),
        cloud_s.cost_usd,
    );
    let advice = if local_classes.is_empty() {
        format!("keep {} for every key class", cloud.model)
    } else if cloud_classes.is_empty() {
        format!(
            "route every key locally, {} only when the validator or judge rejects the local output",
            cloud.model
        )
    } else {
        format!(
            "route '{}' keys locally, keep {} for '{}'",
            local_classes.join("', '"),
            cloud.model,
            cloud_classes.join("', '")
        )
    };
    let text = format!("{head} → {advice}");
    if local_classes.is_empty() {
        return (text, String::new());
    }
    let class_flag = if cloud_classes.is_empty() {
        String::new()
    } else {
        format!(" --class {}", local_classes.join(","))
    };
    let base_url = local
        .base_url
        .as_deref()
        .map(|u| format!("\n# base_url = \"{u}\""))
        .unwrap_or_default();
    let block = format!(
        "{BEGIN} (proposed routing, {date}, {n} eval pairs)\n\
# {text}\n\
# Local first pass, {cloud_model} only on validator failure or low judge score (llm.fallback_provider):\n\
# [llm]\n# provider = \"{prov}\"\n# model = \"{model}\"{base_url}\n# fallback_provider = \"{cprov}\"\n# fallback_model = \"{cloud_model}\"\n\
# Or per key class, keeping the cloud default:\n\
#   glot labs distill translate --model {model} --escalate{class_flag}\n\
{END}\n",
        date = &glot_core::lock::utc_now()[..10],
        n = cloud_s.n,
        cloud_model = cloud.model,
        prov = local.provider,
        model = local.model,
        cprov = cloud.provider,
    );
    (text, block)
}

/// Replace the previous proposal block in glot.toml (comments only: the config is unchanged).
pub fn write_proposal(root: &Path, block: &str) -> Result<()> {
    let path = root.join("glot.toml");
    let io = |e: std::io::Error| Error::Other(format!("{}: {e}", path.display()));
    let text = std::fs::read_to_string(&path).map_err(io)?;
    let mut kept = match (text.find(BEGIN), text.find(END)) {
        (Some(a), Some(b)) if b > a => {
            let end = text[b..].find('\n').map_or(text.len(), |i| b + i + 1);
            format!("{}{}", &text[..a], &text[end..])
        }
        _ => text,
    };
    if !kept.is_empty() && !kept.ends_with("\n\n") {
        kept += if kept.ends_with('\n') { "\n" } else { "\n\n" };
    }
    std::fs::write(&path, kept + block).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(s: &str, t: &str) -> Pair {
        Pair {
            key: s.into(),
            source_locale: "en".into(),
            target_locale: "fr".into(),
            source: s.into(),
            target: t.into(),
            note: None,
        }
    }

    #[test]
    fn secrets_classes_nearest() {
        assert!(looks_secret("Your key: sk-ant-api03-abcdefghijklmnop"));
        assert!(looks_secret("token=ghp_0123456789abcdefABCDEF"));
        assert!(looks_secret("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8"));
        assert!(looks_secret("-----BEGIN PRIVATE KEY-----"));
        assert!(!looks_secret("Save {count} changes to your workspace"));
        assert!(!looks_secret("Visit https://example.com/settings/billing"));

        assert_eq!(class("Save"), "label");
        assert_eq!(class("Your invoice is ready to download now"), "sentence");
        assert_eq!(class("{n, plural, one {# file} other {# files}}"), "icu");

        let pool = vec![
            pair("Delete workspace", "Supprimer l'espace"),
            pair("Invite members", "Inviter des membres"),
            pair("Delete project", "Supprimer le projet"),
        ];
        let n = nearest(&pool, "fr", "Delete workspace now", 2);
        assert_eq!(n[0].source, "Delete workspace");
        assert_eq!(n[1].source, "Delete project");
        assert!(nearest(&pool, "de", "Delete", 3).is_empty());
    }

    #[test]
    fn recommendation_routes_matching_classes() {
        let mk = |lane: &str, label: f64, sentence: f64, cost: f64| {
            let mut classes = BTreeMap::new();
            for (c, j) in [("label", label), ("sentence", sentence)] {
                classes.insert(
                    c.to_string(),
                    ClassScore {
                        n: 10,
                        valid: 10,
                        judged: 10,
                        judge: j,
                        ref_sim: 0.5,
                    },
                );
            }
            Score {
                lane: lane.into(),
                model: lane.into(),
                n: 20,
                valid: 20,
                first_pass: 20,
                judge: Some((label + sentence) / 2.0),
                cost_usd: cost,
                classes,
                ..Default::default()
            }
        };
        let cloud = LlmConfig::default();
        let local = local_config(&cloud, "local", "qwen3:8b", None, true);
        let scores = [
            mk("local", 80.0, 60.0, 0.0),
            mk("local+rag", 95.0, 70.0, 0.0),
            mk("cloud", 96.0, 95.0, 0.01),
        ];
        let (text, block) = recommend(&scores, &local, &cloud);
        assert!(text.starts_with("qwen3:8b + RAG: $0.0000, 86 %"), "{text}");
        assert!(text.contains("route 'label' keys locally"), "{text}");
        assert!(block.contains("--class label") && block.contains("# model = \"qwen3:8b\""));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("glot.toml"), "source_locale = \"en\"\n").unwrap();
        write_proposal(dir.path(), &block).unwrap();
        write_proposal(dir.path(), &block).unwrap();
        let text = std::fs::read_to_string(dir.path().join("glot.toml")).unwrap();
        assert_eq!(text.matches(BEGIN).count(), 1);
        assert!(text.starts_with("source_locale = \"en\"\n\n# >>>"));
    }
}
