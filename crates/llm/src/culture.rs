//! Cultural adaptation audit (`glot labs culture`): what translation alone does not fix.
//! Per market (target locale), the LLM flags idioms, US-centric references, sensitive symbols, humour,
//! inconsistent register and missing market-specific wording, and proposes a transcreated target.
//! Every proposal goes through the glot validator and the copy policy before it can be applied; applied
//! ones are written as suggestions (`glot.lock` origin `transcreation/<model>`, routed to `glot queue`).
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

use glot_core::{Key, Locale, hash::hash, lock::Origin, policy, validate};

use crate::{
    Error, Result,
    engine::Workspace,
    provider::{self, CompletionRequest, Provider},
    translate::{Usage, parse_json, payload_block},
};

pub const CACHE_FILE: &str = ".glot/culture.json";
/// Lockfile provenance of an applied proposal: `Machine { model: "transcreation/<model>" }`.
pub const ORIGIN_PREFIX: &str = "transcreation/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Low,
}

/// One key-level finding as the model returns it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Raw {
    /// idiom | reference | symbol | humor | tone | legal | format
    pub category: String,
    pub severity: Severity,
    pub problem: String,
    /// Why it does not work for this market.
    pub why: String,
    /// Transcreated target text (target language), or none.
    pub proposal: Option<String>,
}

/// A finding with its validated proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub locale: Locale,
    pub key: Key,
    pub source: String,
    pub target: Option<String>,
    #[serde(flatten)]
    pub raw: Raw,
    /// Validator / policy errors: the proposal cannot be applied.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rejected: Vec<String>,
    pub cached: bool,
}

const SYSTEM: &str = "You are a transcreation lead adapting a software product for one market. Translation is done; \
you look for what translation does NOT fix. For each item (`source` string, current `target` translation if any) flag only real problems \
for the target market:\n\
- idiom: idioms, sports or cultural metaphors that do not carry over (\"home run\", \"piece of cake\", \"ballpark\").\n\
- reference: local references and examples (Thanksgiving, Super Bowl, SSN, ZIP code, state, \"John Doe\", US phone numbers, US-only payment methods).\n\
- symbol: colours, symbols, gestures, emoji or numbers with negative meaning in that market.\n\
- humor: jokes and wordplay that fall flat or offend.\n\
- tone: register inconsistent with the `register` instruction (tu/vous, du/Sie, keigo) or with the rest of the catalog.\n\
- legal: wording the market expects by law or custom (EU prices incl. VAT/TTC/inkl. MwSt., GDPR consent wording, \
DE Impressum, JP 特定商取引法 notice).\n\
- format: dates, numbers, currencies or units written in a foreign convention inside the text.\n\
For each problem give `problem` and `why` (one sentence each, in English) and a `proposal`: the transcreated target text in the target language \
(adapt the idea, not the words; keep placeholders like {name}, ICU plural/select syntax and tags exactly; add no number, price or claim absent \
from the source), or null when the fix belongs in code. severity: \"high\" (misunderstood, offensive or legally risky), \"medium\" (sounds foreign), \
\"low\" (polish). Skip items without a problem. Output only one JSON object: \
{\"findings\": {\"<key>\": [{\"category\": \"idiom\", \"severity\": \"medium\", \"problem\": \"...\", \"why\": \"...\", \"proposal\": \"...\"}]}}";

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub key: Key,
    pub source: String,
    pub target: Option<String>,
    pub note: Option<String>,
}

pub(crate) fn request(
    model: &str,
    source_locale: &str,
    market: &str,
    register: &str,
    items: &[Item],
) -> CompletionRequest {
    let payload = json!({
        "task": "culture",
        "source_locale": source_locale,
        "target_locale": market,
        "register": register,
        "items": items,
    });
    CompletionRequest {
        model: model.into(),
        system: SYSTEM.into(),
        user: format!(
            "Audit these {} strings for the `{market}` market.\n\n{}",
            items.len(),
            payload_block(&payload)
        ),
        max_tokens: 16_000,
        json: true,
        images: vec![],
    }
}

/// One market, batched. Keys the model skipped have no finding.
pub async fn audit(
    provider: &dyn Provider,
    model: &str,
    source_locale: &str,
    market: &str,
    register: &str,
    items: &[Item],
    batch: usize,
) -> Result<(BTreeMap<Key, Vec<Raw>>, Usage)> {
    let mut out = BTreeMap::new();
    let mut usage = Usage::default();
    for chunk in items.chunks(batch.max(1)) {
        let c = provider
            .complete(request(model, source_locale, market, register, chunk))
            .await?;
        usage.add(&Usage::of_completion(&c));
        let parsed = parse_json(&c.text).unwrap_or(Value::Null);
        for i in chunk {
            let got: Vec<Raw> = parsed["findings"][&i.key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .map(|mut r: Raw| {
                    r.proposal = r.proposal.filter(|p| !p.trim().is_empty());
                    r
                })
                .collect();
            out.insert(i.key.clone(), got);
        }
    }
    Ok((out, usage))
}

/// Mock answers (deterministic): known US idioms and references in the source become a finding whose
/// proposal is the mock translation of the source with the phrase replaced by a neutral wording.
pub(crate) fn mock(p: &Value) -> Value {
    const PHRASES: &[(&str, &str, &str)] = &[
        ("piece of cake", "very easy", "idiom"),
        ("home run", "big success", "idiom"),
        ("ballpark", "rough", "idiom"),
        ("touch base", "get in touch", "idiom"),
        ("slam dunk", "sure win", "idiom"),
        ("Thanksgiving", "the holidays", "reference"),
        ("ZIP code", "postal code", "reference"),
        ("SSN", "national ID number", "reference"),
        ("John Doe", "Alex Martin", "reference"),
    ];
    let locale = p["target_locale"].as_str().unwrap_or("xx");
    let mut findings = serde_json::Map::new();
    for i in p["items"].as_array().into_iter().flatten() {
        let (Some(key), Some(src)) = (i["key"].as_str(), i["source"].as_str()) else {
            continue;
        };
        let hits: Vec<Value> = PHRASES
            .iter()
            .filter(|(w, _, _)| src.contains(w))
            .map(|(w, plain, cat)| {
                json!({
                    "category": cat, "severity": "medium",
                    "problem": format!("\"{w}\" is US-specific"),
                    "why": format!("mock: \"{w}\" means nothing literal in {locale}"),
                    "proposal": crate::mock::fake_translate(&src.replace(w, plain), locale, &[]),
                })
            })
            .collect();
        if !hits.is_empty() {
            findings.insert(key.into(), Value::Array(hits));
        }
    }
    json!({ "findings": findings })
}

// ---------- workspace ----------

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    /// locale -> key -> (hash of model + register + source + target, findings).
    entries: BTreeMap<Locale, BTreeMap<Key, (String, Vec<Raw>)>>,
}

fn load_cache(root: &Path) -> Cache {
    std::fs::read_to_string(root.join(CACHE_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn model(ws: &Workspace) -> String {
    let l = &ws.config.llm;
    l.judge_model.clone().unwrap_or_else(|| l.model.clone())
}

/// What the register check holds the market to: the variant's register, the declared variants of the
/// base locale, or the style guide.
pub fn register(ws: &Workspace, locale: &str) -> String {
    let declared = glot_core::variants::load(&ws.root).unwrap_or_default();
    if let Some((base, v)) = glot_core::variants::split(locale) {
        return glot_core::variants::instruction(base, v);
    }
    let style = ws.config.llm.style.clone().unwrap_or_default();
    match declared.get(locale) {
        Some(v) => format!(
            "One register throughout this catalog (the project also ships {} variants of `{locale}` separately). {style}",
            v.join("/")
        ),
        None => format!("One consistent register throughout the catalog. {style}"),
    }
    .trim()
    .to_string()
}

/// Audit every target locale (or `locales`), only uncached (source, target) pairs hit the LLM.
pub async fn audit_workspace(
    ws: &Workspace,
    locales: Option<&[Locale]>,
    keys: Option<&[Key]>,
    refresh: bool,
) -> Result<(Vec<Finding>, Usage)> {
    let mut cache = load_cache(&ws.root);
    let policy_cfg = std::fs::read_to_string(ws.root.join(glot_core::config::CONFIG_FILE))
        .ok()
        .and_then(|t| policy::PolicyConfig::from_toml(&t).ok())
        .unwrap_or_default();
    let model = model(ws);
    let source = ws.source();
    let mut out = vec![];
    let mut usage = Usage::default();
    let mut provider: Option<Box<dyn Provider>> = None;
    for loc in &ws.config.target_locales {
        if locales.is_some_and(|w| !w.contains(loc)) {
            continue;
        }
        let register = register(ws, loc);
        let target = ws.catalogs.get(loc);
        let items: Vec<Item> = source
            .entries
            .iter()
            .filter(|(k, s)| keys.is_none_or(|w| w.contains(*k)) && !s.trim().is_empty())
            .map(|(k, s)| Item {
                key: k.clone(),
                source: s.clone(),
                target: target
                    .and_then(|c| c.entries.get(k))
                    .filter(|t| !t.trim().is_empty())
                    .cloned(),
                note: ws.lock.notes.get(k).cloned(),
            })
            .collect();
        let fp = |i: &Item| {
            hash(&format!(
                "{model}\0{register}\0{}\0{}",
                i.source,
                i.target.as_deref().unwrap_or_default()
            ))
        };
        let lc = cache.entries.entry(loc.clone()).or_default();
        let todo: Vec<Item> = items
            .iter()
            .filter(|i| refresh || lc.get(&i.key).is_none_or(|(h, _)| *h != fp(i)))
            .cloned()
            .collect();
        if !todo.is_empty() {
            if provider.is_none() {
                provider = Some(provider::from_config(&ws.config.llm, None)?);
            }
            let p = provider.as_deref().expect("set above");
            let (got, u) = audit(
                p,
                &model,
                &ws.config.source_locale,
                loc,
                &register,
                &todo,
                ws.config.llm.batch_size,
            )
            .await?;
            usage.add(&u);
            for i in &todo {
                if let Some(raws) = got.get(&i.key) {
                    lc.insert(i.key.clone(), (fp(i), raws.clone()));
                }
            }
        }
        for i in items {
            let Some((_, raws)) = lc.get(&i.key) else {
                continue;
            };
            for raw in raws {
                let rejected = raw
                    .proposal
                    .as_deref()
                    .map(|p| check_proposal(ws, &policy_cfg, loc, &i, p))
                    .unwrap_or_default();
                out.push(Finding {
                    locale: loc.clone(),
                    key: i.key.clone(),
                    source: i.source.clone(),
                    target: i.target.clone(),
                    raw: raw.clone(),
                    rejected,
                    cached: !todo.iter().any(|t| t.key == i.key),
                });
            }
        }
    }
    let path = ws.root.join(CACHE_FILE);
    std::fs::create_dir_all(path.parent().expect("has parent"))
        .and_then(|_| {
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&cache).unwrap_or_default(),
            )
        })
        .map_err(|e| Error::Other(format!("cannot write {}: {e}", path.display())))?;
    out.sort_by(|a, b| {
        (a.raw.severity, &a.locale, &a.key).cmp(&(b.raw.severity, &b.locale, &b.key))
    });
    Ok((out, usage))
}

/// Validator (placeholders, ICU, tags, glossary) + copy policy (no invented number, price or claim).
fn check_proposal(
    ws: &Workspace,
    cfg: &policy::PolicyConfig,
    loc: &str,
    i: &Item,
    proposal: &str,
) -> Vec<String> {
    let mut errs: Vec<String> = ws
        .validate(loc, &i.source, proposal, None)
        .into_iter()
        .filter(|x| x.severity == validate::Severity::Error)
        .map(|x| x.message)
        .collect();
    let control = i.target.as_deref().unwrap_or(&i.source);
    errs.extend(
        policy::check(proposal, loc, control, std::slice::from_ref(&i.source), cfg)
            .into_iter()
            .filter(|f| f.severity == validate::Severity::Error)
            .map(|f| format!("policy {}: {}", f.rule, f.message)),
    );
    errs
}

/// What `apply` did with one finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Applied {
    /// Written as a suggestion, waiting in `glot queue`.
    Suggested,
    /// Human-written or reviewed target: never overwritten (use `glot set`).
    Protected,
    /// No proposal, rejected by the validator/policy, or identical to the current target.
    Skipped,
}

/// Write valid proposals as suggestions: catalog value + lock origin `transcreation/<model>` +
/// `needs_review` (with the previous target), so nothing changes silently and `glot queue` approves it.
/// A human or reviewed target is never touched. One proposal per (locale, key): the first, worst first.
pub fn apply(ws: &mut Workspace, findings: &[Finding]) -> Result<Vec<Applied>> {
    let model = format!("{ORIGIN_PREFIX}{}", model(ws));
    let mut done: Vec<(Locale, Key)> = vec![];
    let mut out = vec![];
    for f in findings {
        let protected = ws
            .lock
            .entries
            .get(&f.locale)
            .and_then(|m| m.get(&f.key))
            .is_some_and(|e| e.origin == Origin::Human || e.reviewed);
        let Some(p) = f.raw.proposal.as_deref().filter(|p| {
            f.rejected.is_empty()
                && f.target.as_deref() != Some(*p)
                && !done.contains(&(f.locale.clone(), f.key.clone()))
        }) else {
            out.push(Applied::Skipped);
            continue;
        };
        if protected {
            out.push(Applied::Protected);
            continue;
        }
        ws.write_entries(&f.locale, &[(f.key.clone(), Some(p.to_string()))])?;
        ws.lock.record(
            &f.locale,
            &f.key,
            &f.source,
            p,
            Origin::Machine {
                model: model.clone(),
            },
            None,
        );
        if let Some(e) = ws
            .lock
            .entries
            .get_mut(&f.locale)
            .and_then(|m| m.get_mut(&f.key))
        {
            let was = f.target.as_deref().unwrap_or("(untranslated)");
            e.needs_review = Some(format!(
                "transcreation ({}): {} — previous: {was}",
                f.raw.category, f.raw.problem
            ));
        }
        done.push((f.locale.clone(), f.key.clone()));
        out.push(Applied::Suggested);
    }
    ws.lock.save(&ws.root)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audit_validate_cache_apply() {
        let (_d, mut ws) = crate::review::tests::workspace(
            r#"{"a": "⟦fr⟧ Open the Workspace", "c": "⟦fr⟧ Save"}"#,
        );
        std::fs::write(
            ws.root.join("messages/en.json"),
            r#"{"a": "Setup is a piece of cake", "b": "Enter your ZIP code for {count} items", "c": "Save", "d": "A home run"}"#,
        )
        .unwrap();
        ws.reload().unwrap();
        // `d` is human-written: never overwritten.
        ws.set_translation("fr", "d", "⟦fr⟧ Un succès").unwrap();
        let (f, u) = audit_workspace(&ws, None, None, false).await.unwrap();
        let keys: Vec<_> = f.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "d"]);
        assert!(u.input_tokens > 0 && f.iter().all(|f| f.rejected.is_empty() && !f.cached));
        assert_eq!(f[1].raw.category, "reference");
        assert_eq!(
            f[1].raw.proposal.as_deref(),
            Some("⟦fr⟧ Enter your postal code for {count} items")
        );

        // Cached: no LLM call the second time.
        let (f2, u) = audit_workspace(&ws, None, None, false).await.unwrap();
        assert!(u.input_tokens == 0 && f2.len() == 3 && f2.iter().all(|f| f.cached));

        // A proposal that drops a placeholder or invents a price is rejected.
        let mut bad = f[1].clone();
        bad.raw.proposal = Some("Code postal pour 3 articles à 9 €".into());
        bad.rejected = check_proposal(
            &ws,
            &Default::default(),
            "fr",
            &Item {
                key: bad.key.clone(),
                source: bad.source.clone(),
                target: None,
                note: None,
            },
            bad.raw.proposal.as_deref().unwrap(),
        );
        assert!(
            bad.rejected.iter().any(|e| e.contains("count")),
            "{:?}",
            bad.rejected
        );
        assert!(
            bad.rejected.iter().any(|e| e.starts_with("policy")),
            "{:?}",
            bad.rejected
        );

        let mut all = f.clone();
        all.push(bad);
        let res = apply(&mut ws, &all).unwrap();
        assert_eq!(
            res,
            [
                Applied::Suggested,
                Applied::Suggested,
                Applied::Protected,
                Applied::Skipped
            ]
        );
        ws.reload().unwrap();
        assert_eq!(ws.catalogs["fr"].entries["a"], "⟦fr⟧ Setup is a very easy");
        assert_eq!(ws.catalogs["fr"].entries["d"], "⟦fr⟧ Un succès");
        let e = &ws.lock.entries["fr"]["a"];
        assert_eq!(
            e.origin,
            Origin::Machine {
                model: "transcreation/mock".into()
            }
        );
        assert!(!e.reviewed);
        assert!(
            e.needs_review
                .as_deref()
                .unwrap()
                .contains("previous: ⟦fr⟧ Open the Workspace")
        );
        // Approved suggestions stop being stale or invalid for `glot translate`.
        assert!(ws.plan().stale.is_empty() && ws.plan().invalid.is_empty());
    }
}
