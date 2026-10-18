//! Glossary mining: heuristic term candidates from the source catalog, LLM picks the real terms and their
//! canonical translation per locale, then a deterministic `Glossary::check` lists the inconsistent keys.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use glot_core::{
    Key, Locale,
    catalog::Catalog,
    glossary::{Glossary, Term},
};

use crate::{
    Result,
    engine::Workspace,
    provider::{self, CompletionRequest},
    review::retranslate_keys,
    translate::{Usage, parse_json, payload_block},
};

const MAX_CANDIDATES: usize = 30;
const EXAMPLES: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub term: String,
    /// Source keys containing the term.
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mined {
    pub term: Term,
    pub occurrences: usize,
    /// locale -> keys whose translation does not use the canonical translation.
    pub inconsistent: BTreeMap<Locale, Vec<Key>>,
}

// ponytail: English stopwords only; source catalogs in other languages get noisier candidates (the LLM filters them).
const STOP: &[&str] = &[
    "the", "and", "for", "with", "your", "you", "this", "that", "from", "have", "has", "are",
    "was", "were", "will", "can", "not", "all", "any", "our", "its", "into", "out", "about",
    "more", "less", "than", "then", "there", "their", "them", "they", "what", "when", "which",
    "who", "how", "why", "been", "being", "here", "just", "also", "only", "some", "such", "each",
    "other", "over", "under", "after", "before", "while", "would", "could", "should", "may",
    "might", "must", "yet", "too", "very", "now", "new", "one", "two", "per", "via", "but", "if",
    "or", "of", "to", "in", "on", "at", "by", "is", "it", "be", "as", "an", "a", "no", "yes", "do",
    "does", "did", "we", "us", "my", "me",
];

/// Drop ICU arguments `{…}` (nested) and tags `<…>`, keep the plain text of plural branches.
fn plain(s: &str) -> String {
    let (mut out, mut depth, mut tag) = (String::new(), 0usize, false);
    let mut arg_head = false; // inside `{name, plural,` before the first branch
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            '{' => {
                depth += 1;
                arg_head = depth % 2 == 1;
                out.push(' ');
            }
            '}' => {
                depth = depth.saturating_sub(1);
                arg_head = depth % 2 == 1; // back between branches: skip selectors
                out.push(' ');
            }
            _ if tag || arg_head => {}
            _ => out.push(c),
        }
    }
    out
}

/// Frequent capitalized words (product names, mid-sentence) and frequent non-stopword uni/bigrams.
/// ponytail: frequency heuristic, no NLP; the LLM makes the final selection.
pub fn candidates(source: &Catalog, glossary: &Glossary) -> Vec<Candidate> {
    let mut hits: BTreeMap<String, (BTreeSet<Key>, usize)> = BTreeMap::new(); // term -> (keys, capitalized-bonus)
    for (key, text) in &source.entries {
        for sentence in plain(text).split(['.', '!', '?', ':', ';', '\n', '(', ')', ',', '"']) {
            let words: Vec<&str> = sentence
                .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
                .map(|w| w.trim_matches(['\'', '-']))
                .filter(|w| !w.is_empty())
                .collect();
            let content = |w: &str| {
                w.chars().count() >= 3
                    && !STOP.contains(&w.to_lowercase().as_str())
                    && !w.chars().all(|c| c.is_numeric())
            };
            for (i, w) in words.iter().enumerate() {
                if !content(w) {
                    continue;
                }
                let cap = i > 0 && w.starts_with(char::is_uppercase);
                let term = if cap { w.to_string() } else { w.to_lowercase() };
                let e = hits.entry(term).or_default();
                e.0.insert(key.clone());
                e.1 += cap as usize;
                if let Some(n) = words.get(i + 1).filter(|n| content(n)) {
                    hits.entry(format!("{} {}", w.to_lowercase(), n.to_lowercase()))
                        .or_default()
                        .0
                        .insert(key.clone());
                }
            }
        }
    }
    let known: Vec<String> = glossary
        .terms
        .iter()
        .map(|t| t.term.to_lowercase())
        .collect();
    let mut out: Vec<(usize, Candidate)> = hits
        .into_iter()
        .filter(|(t, _)| !known.iter().any(|k| t.to_lowercase().contains(k.as_str())))
        .filter(|(t, (keys, cap))| keys.len() >= if *cap > 0 || t.contains(' ') { 2 } else { 3 })
        .map(|(term, (keys, cap))| {
            (
                keys.len() + cap,
                Candidate {
                    term,
                    keys: keys.into_iter().collect(),
                },
            )
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.term.cmp(&b.1.term)));
    // A bigram as frequent as its words makes the unigrams redundant.
    let bigrams: Vec<Candidate> = out
        .iter()
        .filter(|(_, c)| c.term.contains(' '))
        .map(|(_, c)| c.clone())
        .collect();
    out.retain(|(_, c)| {
        c.term.contains(' ')
            || !bigrams
                .iter()
                .any(|b| b.keys == c.keys && b.term.split(' ').any(|w| w == c.term.to_lowercase()))
    });
    out.into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, c)| c)
        .collect()
}

const SYSTEM: &str = "You build the terminology glossary of a SaaS product from its UI strings.\n\
You get candidate terms (extracted by frequency) with example source strings and their current translations.\n\
For each candidate decide `keep`: true only for real product/domain terms that must be translated consistently \
(feature names, product names, business objects, roles); false for generic words. \
For kept terms: `do_not_translate` true for brand/product names kept verbatim; otherwise give the canonical `translations` per target locale — \
the form that is the best/most common in the examples (use the exact word form as it appears in the translations); \
plus a short `description` in the source language.\n\
Output only one JSON object, no prose: {\"terms\": [{\"term\": \"...\", \"keep\": true, \"do_not_translate\": false, \"description\": \"...\", \"translations\": {\"fr\": \"...\"}}]}";

pub(crate) fn request(model: &str, ws: &Workspace, cands: &[Candidate]) -> CompletionRequest {
    let source = ws.source();
    let payload = json!({
        "task": "mine",
        "source_locale": ws.config.source_locale,
        "target_locales": ws.config.target_locales,
        "candidates": cands.iter().map(|c| json!({
            "term": c.term,
            "occurrences": c.keys.len(),
            "examples": c.keys.iter().take(EXAMPLES).map(|k| json!({
                "source": source.entries[k],
                "translations": ws.config.target_locales.iter()
                    .filter_map(|l| Some((l.clone(), json!(ws.catalogs.get(l)?.entries.get(k)?))))
                    .collect::<serde_json::Map<_, _>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    CompletionRequest {
        model: model.into(),
        system: SYSTEM.into(),
        user: format!(
            "Select glossary terms among these {} candidates.\n\n{}",
            cands.len(),
            payload_block(&payload)
        ),
        max_tokens: 8_000,
        json: true,
        images: vec![],
    }
}

/// Keys (by locale) whose existing translation violates `term`.
pub fn inconsistent(ws: &Workspace, term: &Term) -> BTreeMap<Locale, Vec<Key>> {
    let g = Glossary {
        terms: vec![term.clone()],
    };
    let mut out: BTreeMap<Locale, Vec<Key>> = BTreeMap::new();
    for loc in &ws.config.target_locales {
        let Some(cat) = ws.catalogs.get(loc) else {
            continue;
        };
        for (k, s) in &ws.source().entries {
            if let Some(t) = cat.entries.get(k).filter(|t| !t.trim().is_empty())
                && !g.check(s, t, loc).is_empty()
            {
                out.entry(loc.clone()).or_default().push(k.clone());
            }
        }
    }
    out
}

pub async fn mine(ws: &Workspace) -> Result<(Vec<Mined>, Usage)> {
    let cands = candidates(ws.source(), &ws.glossary);
    if cands.is_empty() {
        return Ok((vec![], Usage::default()));
    }
    let model = ws
        .config
        .llm
        .judge_model
        .clone()
        .unwrap_or_else(|| ws.config.llm.model.clone());
    let c = provider::from_config(&ws.config.llm, None)?
        .complete(request(&model, ws, &cands))
        .await?;
    let usage = Usage::of(&c.model, c.input_tokens, c.output_tokens);
    let parsed = parse_json(&c.text).unwrap_or(Value::Null);
    let mut out = vec![];
    for t in parsed["terms"].as_array().into_iter().flatten() {
        let Some(cand) = cands.iter().find(|c| t["term"].as_str() == Some(&c.term)) else {
            continue;
        };
        if t["keep"] != true {
            continue;
        }
        let term = Term {
            term: cand.term.clone(),
            description: t["description"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(String::from),
            do_not_translate: t["do_not_translate"] == true,
            // Brand names are case-specific: "Invoices" (in "Acme Invoices") must not swallow the
            // common noun "invoices" and flag every translated sentence as inconsistent.
            case_sensitive: t["do_not_translate"] == true,
            translations: t["translations"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(l, _)| ws.config.target_locales.contains(l))
                .filter_map(|(l, v)| Some((l.clone(), v.as_str()?.to_string())))
                .collect(),
            forbidden: BTreeMap::new(),
        };
        out.push(Mined {
            inconsistent: inconsistent(ws, &term),
            occurrences: cand.keys.len(),
            term,
        });
    }
    Ok((out, usage))
}

/// Write the mined terms to glossary.toml, then retranslate the inconsistent keys (terminology refactor).
pub async fn apply(ws: &mut Workspace, mined: &[Mined]) -> Result<Usage> {
    let mut keys: BTreeMap<Locale, Vec<Key>> = BTreeMap::new();
    for m in mined {
        ws.glossary.upsert(m.term.clone());
        for (l, ks) in &m.inconsistent {
            let e = keys.entry(l.clone()).or_default();
            e.extend(ks.iter().cloned());
            e.sort();
            e.dedup();
        }
    }
    ws.save_glossary()?;
    retranslate_keys(ws, keys).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_code() {
        assert_eq!(
            plain("Hi <b>{name}</b>, {n, plural, one {# file} other {# files}}")
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["Hi", ",", "#", "file", "#", "files"]
        );
    }

    #[test]
    fn candidate_heuristic() {
        let mut src = Catalog::new("en");
        for (k, v) in [
            ("a", "Open your Workspace"),
            ("b", "Invite people to the Workspace"),
            ("c", "Billing settings"),
            ("d", "Update billing settings"),
            ("e", "Billing settings saved"),
            ("f", "Save"),
        ] {
            src.entries.insert(k.into(), v.into());
        }
        let c = candidates(&src, &Glossary::default());
        let terms: Vec<&str> = c.iter().map(|c| c.term.as_str()).collect();
        assert!(
            terms.contains(&"Workspace") && terms.contains(&"billing settings"),
            "{terms:?}"
        );
        assert!(
            !terms.contains(&"settings") && !terms.contains(&"Save"),
            "{terms:?}"
        );
        let known = Glossary {
            terms: vec![Term {
                term: "workspace".into(),
                ..Default::default()
            }],
        };
        assert!(
            !candidates(&src, &known)
                .iter()
                .any(|c| c.term == "Workspace")
        );
    }

    #[tokio::test]
    async fn mine_and_apply_with_mock() {
        let (_d, mut ws) = crate::review::tests::workspace(
            r#"{"a": "⟦fr⟧ Open the Workspace", "b": "⟦fr⟧ Supprimer l'espace", "c": "⟦fr⟧ Save"}"#,
        );
        let (mined, _) = mine(&ws).await.unwrap();
        let w = mined
            .iter()
            .find(|m| m.term.term == "Workspace")
            .expect("Workspace mined");
        assert_eq!(w.term.translations["fr"], "Workspace");
        assert_eq!(w.inconsistent["fr"], vec!["b".to_string()]);
        apply(&mut ws, &mined).await.unwrap();
        ws.reload().unwrap();
        assert!(ws.glossary.terms.iter().any(|t| t.term == "Workspace"));
        assert_eq!(ws.catalogs["fr"].entries["b"], "⟦fr⟧ Delete the Workspace");
    }
}
