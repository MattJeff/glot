//! Back-translation review: "trust a language you don't speak".
//! The LLM back-translates the target literally into the source language, explains its choices and
//! gives a fidelity verdict. Also used for semantic drift (stale source: keep vs retranslate).
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

use glot_core::{Key, Locale, glossary::Glossary, hash::hash, lock::Origin};

use crate::{
    Error, Result,
    engine::{TranslateOptions, Workspace},
    provider::{self, CompletionRequest, Provider},
    translate::{Usage, parse_json, payload_block},
};

pub const CACHE_FILE: &str = ".glot/review.json";
const BATCH: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fidelity {
    Same,
    Nuance,
    Different,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewItem {
    pub key: Key,
    pub source: String,
    pub target: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub key: Key,
    pub back_translation: String,
    pub verdict: Fidelity,
    pub explanation: String,
    pub suggestion: Option<String>,
}

const SYSTEM: &str = "You help product teams trust translations in languages they do not speak.\n\
For each item: (1) back-translate `target` LITERALLY into the source language (keep its exact meaning, register and oddities, do not fix it); \
(2) explain briefly in the source language (1-3 sentences) the translator's choices: register/formality, glossary terms used, plural handling, \
anything adapted culturally or reworded; (3) give a fidelity verdict comparing target with `source`: \
\"same\" (same meaning), \"nuance\" (same intent, noticeable shift in tone/emphasis/precision), \"different\" (meaning changed, missing or added). \
If the verdict is not \"same\", give a corrected `suggestion` in the target language (keep placeholders, ICU syntax and tags exactly), else null.\n\
Placeholders like {name}, ICU plural/select syntax and tags are code: keep them as-is in the back-translation.\n\
Output only one JSON object, no prose: {\"reviews\": {\"<key>\": {\"back_translation\": \"...\", \"explanation\": \"...\", \"verdict\": \"same\", \"suggestion\": null}}}";

pub(crate) fn request(
    model: &str,
    source_locale: &str,
    target_locale: &str,
    glossary: &Glossary,
    items: &[ReviewItem],
) -> CompletionRequest {
    let mut terms = vec![];
    for i in items {
        for t in glossary.matches(&i.source) {
            if !terms.iter().any(|x: &Value| x["term"] == t.term.as_str()) {
                terms.push(
                    json!({"term": t.term, "do_not_translate": t.do_not_translate,
                    "translation": t.translations.get(target_locale)}),
                );
            }
        }
    }
    let payload = json!({
        "task": "review",
        "source_locale": source_locale,
        "target_locale": target_locale,
        "glossary": terms,
        "items": items.iter().map(|i| json!({
            "key": i.key, "source": i.source, "target": i.target, "description": i.description,
        })).collect::<Vec<_>>(),
    });
    CompletionRequest {
        model: model.into(),
        system: SYSTEM.into(),
        user: format!(
            "Back-translate and review these {} `{target_locale}` translations of `{source_locale}` strings.\n\n{}",
            items.len(),
            payload_block(&payload)
        ),
        max_tokens: 16_000,
        json: true,
        images: vec![],
    }
}

/// Batched back-translation review. Items the model skipped are omitted from the result.
pub async fn review(
    provider: &dyn Provider,
    model: &str,
    source_locale: &str,
    target_locale: &str,
    glossary: &Glossary,
    items: &[ReviewItem],
) -> Result<(Vec<Review>, Usage)> {
    let mut out = vec![];
    let mut usage = Usage::default();
    for chunk in items.chunks(BATCH) {
        let c = provider
            .complete(request(
                model,
                source_locale,
                target_locale,
                glossary,
                chunk,
            ))
            .await?;
        usage.add(&Usage::of_completion(&c));
        let parsed = parse_json(&c.text).unwrap_or(Value::Null);
        for i in chunk {
            let v = &parsed["reviews"][&i.key];
            let Some(back) = v["back_translation"].as_str() else {
                continue;
            };
            let verdict = serde_json::from_value(v["verdict"].clone()).unwrap_or(Fidelity::Nuance);
            out.push(Review {
                key: i.key.clone(),
                back_translation: back.into(),
                verdict,
                explanation: v["explanation"].as_str().unwrap_or_default().into(),
                suggestion: v["suggestion"]
                    .as_str()
                    .filter(|s| !s.is_empty() && verdict != Fidelity::Same)
                    .map(String::from),
            });
        }
    }
    Ok((out, usage))
}

/// Mock provider answers (deterministic): back-translation strips the `⟦locale⟧ ` prefix;
/// mining keeps every candidate with the term itself as translation.
pub(crate) fn mock(p: &Value) -> Value {
    let locale = p["target_locale"].as_str().unwrap_or("xx");
    if p["task"] == "mine" {
        let locales = p["target_locales"].clone();
        let terms: Vec<Value> = p["candidates"].as_array().into_iter().flatten().map(|c| {
            let t = &c["term"];
            let tr: serde_json::Map<String, Value> = locales.as_array().into_iter().flatten()
                .filter_map(|l| Some((l.as_str()?.to_string(), t.clone()))).collect();
            json!({"term": t, "keep": true, "do_not_translate": false, "description": "mock", "translations": tr})
        }).collect();
        return json!({ "terms": terms });
    }
    let prefix = format!("⟦{locale}⟧ ");
    let reviews: serde_json::Map<String, Value> = p["items"].as_array().into_iter().flatten().filter_map(|i| {
        let (src, tgt) = (i["source"].as_str()?, i["target"].as_str()?);
        let back = tgt.replacen(&prefix, "", 1);
        let same = back == src;
        Some((i["key"].as_str()?.to_string(), json!({
            "back_translation": back,
            "explanation": "mock: back-translation strips the locale prefix",
            "verdict": if same { "same" } else { "different" },
            "suggestion": if same { Value::Null } else { json!(crate::mock::fake_translate(src, locale, &[])) },
        })))
    }).collect();
    json!({ "reviews": reviews })
}

// ---------- local workspace ----------

/// One row of `glot review`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRow {
    pub key: Key,
    pub locale: Locale,
    pub source: String,
    pub target: String,
    pub back_translation: String,
    pub verdict: Fidelity,
    pub explanation: String,
    pub suggestion: Option<String>,
    pub cached: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Cache {
    /// locale -> key -> entry; valid while both hashes match.
    entries: BTreeMap<Locale, BTreeMap<Key, Cached>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached {
    source_hash: String,
    target_hash: String,
    review: Review,
}

fn load_cache(root: &Path) -> Cache {
    std::fs::read_to_string(root.join(CACHE_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(root: &Path, c: &Cache) -> Result<()> {
    let path = root.join(CACHE_FILE);
    std::fs::create_dir_all(path.parent().expect("has parent"))
        .and_then(|_| std::fs::write(&path, serde_json::to_string_pretty(c).unwrap_or_default()))
        .map_err(|e| Error::Other(format!("cannot write {}: {e}", path.display())))
}

fn review_model(ws: &Workspace) -> String {
    let l = &ws.config.llm;
    l.judge_model.clone().unwrap_or_else(|| l.model.clone())
}

/// Review existing translations (cache `.glot/review.json`, only uncached pairs hit the LLM).
/// `locales` None = every target locale, `keys` None = every translated key.
pub async fn review_workspace(
    ws: &Workspace,
    locales: Option<&[Locale]>,
    keys: Option<&[Key]>,
    refresh: bool,
) -> Result<(Vec<ReviewRow>, Usage)> {
    let mut cache = load_cache(&ws.root);
    let source = ws.source();
    let mut rows = vec![];
    let mut usage = Usage::default();
    let mut provider: Option<Box<dyn Provider>> = None;
    for loc in &ws.config.target_locales {
        if locales.is_some_and(|w| !w.contains(loc)) {
            continue;
        }
        let Some(cat) = ws.catalogs.get(loc) else {
            continue;
        };
        let pairs: Vec<(&Key, &String, &String)> = source
            .entries
            .iter()
            .filter(|(k, _)| keys.is_none_or(|w| w.contains(*k)))
            .filter_map(|(k, s)| Some((k, s, cat.entries.get(k).filter(|t| !t.trim().is_empty())?)))
            .filter(|(_, s, _)| !s.trim().is_empty())
            .collect();
        let lc = cache.entries.entry(loc.clone()).or_default();
        let mut todo = vec![];
        for (k, s, t) in &pairs {
            match lc.get(*k) {
                Some(c) if !refresh && c.source_hash == hash(s) && c.target_hash == hash(t) => {}
                _ => todo.push(ReviewItem {
                    key: (*k).clone(),
                    source: (*s).clone(),
                    target: (*t).clone(),
                    description: ws.lock.notes.get(*k).cloned(),
                }),
            }
        }
        if !todo.is_empty() {
            if provider.is_none() {
                provider = Some(provider::from_config(&ws.config.llm, None)?);
            }
            let p = provider.as_deref().expect("set above");
            let (got, u) = review(
                p,
                &review_model(ws),
                &ws.config.source_locale,
                loc,
                &ws.glossary,
                &todo,
            )
            .await?;
            usage.add(&u);
            for r in got {
                let i = todo
                    .iter()
                    .find(|i| i.key == r.key)
                    .expect("reviewed key was asked");
                lc.insert(
                    r.key.clone(),
                    Cached {
                        source_hash: hash(&i.source),
                        target_hash: hash(&i.target),
                        review: r,
                    },
                );
            }
        }
        for (k, s, t) in pairs {
            let Some(c) = lc.get(k) else { continue };
            rows.push(ReviewRow {
                key: k.clone(),
                locale: loc.clone(),
                source: s.clone(),
                target: t.clone(),
                back_translation: c.review.back_translation.clone(),
                verdict: c.review.verdict,
                explanation: c.review.explanation.clone(),
                suggestion: c.review.suggestion.clone(),
                cached: !todo.iter().any(|i| &i.key == k),
            });
        }
    }
    save_cache(&ws.root, &cache)?;
    // Worst first: `different`, then `nuance`, then `same` (stable within each group).
    rows.sort_by_key(|r| match r.verdict {
        Fidelity::Different => 0,
        Fidelity::Nuance => 1,
        Fidelity::Same => 2,
    });
    Ok((rows, usage))
}

/// Stale (locale, key) after a source change: is the current target still faithful to the new source?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Drift {
    pub locale: Locale,
    pub key: Key,
    pub source: String,
    pub target: String,
    pub back_translation: String,
    pub verdict: Fidelity,
    pub explanation: String,
    /// true = cosmetic change, the translation stays valid (refresh the lock hash); false = retranslate.
    pub keep: bool,
}

/// Classify every stale entry. Only `same` keeps: a nuance after a source edit is worth a retranslation.
pub async fn stale(ws: &Workspace) -> Result<(Vec<Drift>, Usage)> {
    let plan = ws.plan();
    let source = ws.source();
    let mut out = vec![];
    let mut usage = Usage::default();
    let mut by_locale: BTreeMap<&Locale, Vec<ReviewItem>> = BTreeMap::new();
    for (loc, key) in &plan.stale {
        if let (Some(s), Some(t)) = (
            source.entries.get(key),
            ws.catalogs.get(loc).and_then(|c| c.entries.get(key)),
        ) {
            by_locale.entry(loc).or_default().push(ReviewItem {
                key: key.clone(),
                source: s.clone(),
                target: t.clone(),
                description: ws.lock.notes.get(key).cloned(),
            });
        }
    }
    if by_locale.is_empty() {
        return Ok((out, usage));
    }
    let provider = provider::from_config(&ws.config.llm, None)?;
    for (loc, items) in by_locale {
        let (got, u) = review(
            provider.as_ref(),
            &review_model(ws),
            &ws.config.source_locale,
            loc,
            &ws.glossary,
            &items,
        )
        .await?;
        usage.add(&u);
        for r in got {
            let i = items.iter().find(|i| i.key == r.key).expect("asked");
            out.push(Drift {
                locale: loc.clone(),
                key: r.key,
                source: i.source.clone(),
                target: i.target.clone(),
                back_translation: r.back_translation,
                keep: r.verdict == Fidelity::Same,
                verdict: r.verdict,
                explanation: r.explanation,
            });
        }
    }
    Ok((out, usage))
}

/// Apply drift decisions: keep => refresh the lock source hash (origin/score kept);
/// otherwise retranslate those keys (forced, per locale).
pub async fn apply_drift(ws: &mut Workspace, drifts: &[Drift]) -> Result<Usage> {
    let mut retranslate: BTreeMap<Locale, Vec<Key>> = BTreeMap::new();
    for d in drifts {
        if d.keep {
            let (origin, score) = ws
                .lock
                .entries
                .get(&d.locale)
                .and_then(|m| m.get(&d.key))
                .map(|e| (e.origin.clone(), e.score))
                .unwrap_or((Origin::Imported, None));
            ws.lock
                .record(&d.locale, &d.key, &d.source, &d.target, origin, score);
        } else {
            retranslate
                .entry(d.locale.clone())
                .or_default()
                .push(d.key.clone());
        }
    }
    ws.lock.save(&ws.root)?;
    retranslate_keys(ws, retranslate).await
}

/// Forced retranslation of specific keys per locale.
pub async fn retranslate_keys(
    ws: &mut Workspace,
    keys: BTreeMap<Locale, Vec<Key>>,
) -> Result<Usage> {
    let mut usage = Usage::default();
    for (loc, keys) in keys {
        let r = ws
            .translate(TranslateOptions {
                locales: Some(vec![loc]),
                keys: Some(keys),
                force: true,
                dry_run: false,
            })
            .await?;
        usage.add(&r.usage);
    }
    Ok(usage)
}

/// `--fix`: write each non-`same` suggestion (validated; rejected ones are returned with their issues).
#[allow(clippy::type_complexity)] // ponytail: (key, locale, reason)
pub fn apply_suggestions(
    ws: &mut Workspace,
    rows: &[ReviewRow],
) -> Result<(usize, Vec<(Key, Locale, String)>)> {
    let (mut ok, mut rejected) = (0, vec![]);
    for r in rows.iter().filter(|r| r.verdict != Fidelity::Same) {
        let Some(s) = &r.suggestion else { continue };
        let issues = ws.set_translation(&r.locale, &r.key, s)?;
        if glot_core::validate::has_errors(&issues) {
            let msg = issues
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            rejected.push((r.key.clone(), r.locale.clone(), msg));
        } else {
            ok += 1;
        }
    }
    Ok((ok, rejected))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn workspace(fr: &str) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(
            p.join("glot.toml"),
            "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(p.join("messages")).unwrap();
        std::fs::write(
            p.join("messages/en.json"),
            r#"{"a": "Open the Workspace", "b": "Delete the Workspace", "c": "Save"}"#,
        )
        .unwrap();
        std::fs::write(p.join("messages/fr.json"), fr).unwrap();
        let ws = Workspace::open(p).unwrap();
        (dir, ws)
    }

    #[tokio::test]
    async fn mock_back_translation_and_cache() {
        let (_d, ws) = workspace(r#"{"a": "⟦fr⟧ Open the Workspace", "b": "⟦fr⟧ Remove it"}"#);
        let (rows, _) = review_workspace(&ws, None, None, false).await.unwrap();
        assert_eq!(rows.len(), 2);
        // Worst first, though `b` sorts after `a`.
        assert_eq!(rows[0].verdict, Fidelity::Different);
        assert_eq!(
            rows[0].suggestion.as_deref(),
            Some("⟦fr⟧ Delete the Workspace")
        );
        assert_eq!(rows[1].back_translation, "Open the Workspace");
        assert_eq!(rows[1].verdict, Fidelity::Same);
        assert!(!rows[1].cached);
        let (rows, u) = review_workspace(&ws, Some(&["fr".into()]), Some(&["a".into()]), false)
            .await
            .unwrap();
        assert!(rows.len() == 1 && rows[0].cached && u.input_tokens == 0);
    }

    #[tokio::test]
    async fn fix_and_drift() {
        let (_d, mut ws) = workspace(r#"{"a": "⟦fr⟧ Open the Workspace", "b": "⟦fr⟧ Remove it"}"#);
        let (rows, _) = review_workspace(&ws, None, None, false).await.unwrap();
        let (ok, rejected) = apply_suggestions(&mut ws, &rows).unwrap();
        assert_eq!((ok, rejected.len()), (1, 0));
        assert_eq!(ws.catalogs["fr"].entries["b"], "⟦fr⟧ Delete the Workspace");

        // Lock both, then change sources: `a` only cosmetically (mock: target unchanged = same), `b` in meaning.
        ws.lock.record(
            "fr",
            "a",
            "Open the workspace",
            "⟦fr⟧ Open the Workspace",
            Origin::Human,
            None,
        );
        ws.lock.record(
            "fr",
            "b",
            "Delete it",
            "⟦fr⟧ Delete the Workspace",
            Origin::Human,
            None,
        );
        ws.lock.save(&ws.root).unwrap();
        ws.reload().unwrap();
        assert_eq!(ws.plan().stale.len(), 2);
        std::fs::write(
            ws.root.join("messages/en.json"),
            r#"{"a": "Open the Workspace", "b": "Archive the Workspace", "c": "Save"}"#,
        )
        .unwrap();
        ws.reload().unwrap();
        let (drifts, _) = stale(&ws).await.unwrap();
        let keep: Vec<_> = drifts.iter().map(|d| (d.key.as_str(), d.keep)).collect();
        assert_eq!(keep, vec![("a", true), ("b", false)]);
        apply_drift(&mut ws, &drifts).await.unwrap();
        ws.reload().unwrap();
        assert!(ws.plan().stale.is_empty());
        assert_eq!(ws.catalogs["fr"].entries["a"], "⟦fr⟧ Open the Workspace");
        assert_eq!(ws.catalogs["fr"].entries["b"], "⟦fr⟧ Archive the Workspace");
    }
}
