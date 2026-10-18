//! Glossary mining: heuristic term candidates from the source catalog, LLM picks the real terms and their
//! canonical translation per locale, then a deterministic `Glossary::check` lists the inconsistent keys.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use glot_core::{
    Key, Locale,
    catalog::Catalog,
    glossary::{Glossary, Term, find},
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

// ponytail: English word lists only; source catalogs in other languages get noisier candidates (the LLM filters them).
#[rustfmt::skip]
/// Function words, common UI verbs and adverbs: never part of a term.
const STOP: &[&str] = &[
    "the", "and", "for", "with", "your", "you", "this", "that", "from", "have", "has", "are", "was",
    "were", "will", "can", "not", "all", "any", "our", "its", "into", "out", "about", "more",
    "less", "than", "then", "there", "their", "them", "they", "what", "when", "which", "who", "how",
    "why", "been", "being", "here", "just", "also", "only", "some", "such", "each", "other", "over",
    "under", "after", "before", "while", "would", "could", "should", "may", "might", "must", "yet",
    "too", "very", "now", "one", "two", "per", "via", "but", "if", "or", "of", "to", "in", "on",
    "at", "by", "is", "it", "be", "as", "an", "a", "no", "yes", "do", "does", "did", "we", "us", "new",
    "my", "me", "his", "her", "him", "she", "within", "without", "between", "during", "through",
    "across", "around", "off", "up", "down", "every", "many", "much", "few", "own", "same", "both",
    "either", "nothing", "something", "anything", "everything", "everyone", "someone", "first",
    "last", "next", "previous", "again", "already", "always", "never", "still", "even", "ever",
    "soon", "later", "today", "tomorrow", "yesterday", "ago", "anytime", "automatically", "fast",
    "faster", "quickly", "please", "sorry", "thank", "thanks", "welcome", "hello", "okay", "sure",
    "really", "done", "ready", "get", "got", "let", "make", "made", "take", "took", "keep", "see",
    "seen", "go", "went", "come", "back", "left", "right", "add", "added", "remove", "removed",
    "delete", "deleted", "save", "saved", "cancel", "cancelled", "canceled", "create", "created",
    "send", "sent", "open", "close", "closed", "use", "used", "using", "need", "want", "try",
    "show", "showing", "find", "found", "look", "looking", "know", "mark", "marked", "set", "start",
    "started", "stop", "skip", "press", "click", "tap", "select", "choose", "enter", "sign",
    "signed", "log", "happen", "happening", "remind", "removes", "takes", "makes", "gets", "sends",
    "ending", "expires", "renews", "used", "undone", "ends", "receive", "received",
];
#[rustfmt::skip]
/// Adjectives and verb-like nouns: may start a term ("due date") but never stand alone or end one.
const HEAD_ONLY: &[&str] = &[
    "due", "paid", "unpaid", "overdue", "current", "recent", "latest", "monthly", "yearly",
    "annual", "popular", "simple", "good", "great", "free", "full", "main", "default", "active",
    "pending", "available", "required", "optional", "public", "private", "search", "view", "update",
    "edit", "filter", "review", "invite", "upgrade", "manage", "most", "change", "archive",
    "frequently", "asked",
];
#[rustfmt::skip]
/// Generic nouns: may end a term ("due date") but are not terms alone.
const GENERIC: &[&str] = &[
    "date", "day", "week", "month", "year", "hour", "minute", "time", "item", "name", "email",
    "number", "amount", "page", "section", "step", "value", "detail", "information", "setting",
    "option", "message", "button", "link", "label", "text", "list", "status", "error", "address",
    "description", "note", "action", "profile", "question", "people", "morning",
];
const MAX_NGRAM: usize = 3;

/// Drop ICU arguments `{…}` (nested) and tags `<…>`, keep the plain text of plural branches
/// (one per line, so `# seat` / `# seats` never join into a bigram).
fn plain(s: &str) -> String {
    let (mut out, mut depth, mut tag) = (String::new(), 0usize, false);
    let mut arg_head = false; // inside `{name, plural,` before the first branch
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => {
                tag = false;
                out.push(' ');
            }
            '{' => {
                depth += 1;
                arg_head = depth % 2 == 1;
                out.push('\n');
            }
            '}' => {
                depth = depth.saturating_sub(1);
                arg_head = depth % 2 == 1; // back between branches: skip selectors
                out.push('\n');
            }
            _ if tag || arg_head => {}
            '’' => out.push('\''),
            _ => out.push(c),
        }
    }
    out
}

/// English singular: invoices -> invoice, companies -> company, boxes -> box, status stays.
fn lemma(w: &str) -> String {
    let w = w.to_lowercase();
    if let Some(s) = w.strip_suffix("ies").filter(|s| s.len() >= 3) {
        return format!("{s}y");
    }
    for es in ["sses", "shes", "ches", "xes"] {
        if w.ends_with(es) && w.len() > es.len() + 1 {
            return w[..w.len() - 2].to_string();
        }
    }
    match w.strip_suffix('s') {
        Some(s) if s.len() >= 3 && !["s", "u", "i"].iter().any(|e| s.ends_with(e)) => s.into(),
        _ => w,
    }
}

/// Candidate terms by frequency: nominal uni/bi/trigrams (English plural folded into the singular)
/// and runs of capitalized words (product names such as "Acme Invoices").
/// Unigrams need `max(3, entries/50)` keys, multi-word and capitalized terms 2.
/// ponytail: word lists + frequency, no NLP; the LLM makes the final selection.
pub fn candidates(source: &Catalog, glossary: &Glossary) -> Vec<Candidate> {
    #[derive(Default)]
    struct Hit {
        keys: BTreeSet<Key>,
        cap: usize,
        /// Surface forms of a lemma ("billing settings" for "billing setting").
        forms: BTreeMap<String, usize>,
    }
    let mut hits: BTreeMap<String, Hit> = BTreeMap::new();
    let is = |list: &[&str], w: &str| {
        list.contains(&w.to_lowercase().as_str()) || list.contains(&lemma(w).as_str())
    };
    let content = |w: &str| {
        w.chars().count() >= 3
            && !w.contains('\'')
            && !STOP.contains(&w.to_lowercase().as_str())
            && w.chars().any(char::is_alphabetic)
    };
    for (key, text) in &source.entries {
        for sentence in plain(text).split([
            '.', '!', '?', ':', ';', '\n', '(', ')', ',', '"', '/', '·', '—', '…', '→', '←', '|',
        ]) {
            let words: Vec<&str> = sentence
                .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
                .map(|w| w.trim_matches(['\'', '-']))
                .filter(|w| !w.is_empty())
                .collect();
            let cap = |w: &str| content(w) && w.starts_with(char::is_uppercase);
            // Runs of 2-3 capitalized words are names: count them whole, never their words.
            let mut in_run = vec![false; words.len()];
            let mut i = 0;
            while i < words.len() {
                let n = words[i..].iter().take_while(|w| cap(w)).count();
                if (2..=MAX_NGRAM).contains(&n) {
                    let h = hits.entry(words[i..i + n].join(" ")).or_default();
                    h.keys.insert(key.clone());
                    h.cap += 1;
                    in_run[i..i + n].iter_mut().for_each(|r| *r = true);
                }
                i += n.max(1);
            }
            for (i, w) in words.iter().enumerate() {
                if in_run[i] || !content(w) {
                    continue;
                }
                if !is(HEAD_ONLY, w) && !is(GENERIC, w) {
                    let capitalized = i > 0 && w.starts_with(char::is_uppercase);
                    let h = hits
                        .entry(if capitalized { w.to_string() } else { lemma(w) })
                        .or_default();
                    h.keys.insert(key.clone());
                    h.cap += capitalized as usize;
                    *h.forms
                        .entry(if capitalized {
                            w.to_string()
                        } else {
                            w.to_lowercase()
                        })
                        .or_default() += 1;
                }
                if is(GENERIC, w) {
                    continue; // generic nouns end terms, never start them
                }
                for n in 2..=MAX_NGRAM {
                    let Some(g) = words.get(i..i + n) else { break };
                    if g.iter()
                        .enumerate()
                        .any(|(j, w)| in_run[i + j] || !content(w))
                    {
                        break;
                    }
                    if is(HEAD_ONLY, g[n - 1]) {
                        continue;
                    }
                    let mut t: Vec<String> = g.iter().map(|w| w.to_lowercase()).collect();
                    let surface = t.join(" ");
                    t[n - 1] = lemma(&t[n - 1]);
                    let h = hits.entry(t.join(" ")).or_default();
                    h.keys.insert(key.clone());
                    *h.forms.entry(surface).or_default() += 1;
                }
            }
        }
    }
    // A capitalized word also used lowercase is a common noun ("Workspace" / "workspace").
    let caps: Vec<String> = hits
        .keys()
        .filter(|t| !t.contains(' ') && t.starts_with(char::is_uppercase))
        .cloned()
        .collect();
    for c in caps {
        if hits.contains_key(&lemma(&c)) {
            let h = hits.remove(&c).unwrap();
            let common = hits.get_mut(&lemma(&c)).unwrap();
            common.keys.extend(h.keys);
            for (f, n) in h.forms {
                *common.forms.entry(f).or_default() += n;
            }
        }
    }
    let known: Vec<String> = glossary
        .terms
        .iter()
        .map(|t| t.term.to_lowercase())
        .collect();
    let min_uni = 3.max(source.entries.len() / 50);
    let mut out: Vec<(usize, Candidate)> = hits
        .into_iter()
        .filter(|(t, _)| !known.iter().any(|k| t.to_lowercase().contains(k.as_str())))
        .filter(|(t, h)| {
            h.keys.len()
                >= if h.cap > 0 || t.contains(' ') {
                    2
                } else {
                    min_uni
                }
        })
        .map(|(lemma, h)| {
            // The lemma when it is a real form, else the most frequent form.
            let term = match h.forms.iter().max_by_key(|(_, n)| **n) {
                Some((f, _)) if !h.forms.contains_key(&lemma) => f.clone(),
                _ => lemma,
            };
            (
                h.keys.len() + h.cap,
                Candidate {
                    term,
                    keys: h.keys.into_iter().collect(),
                },
            )
        })
        .collect();
    // A longer term found in the same keys makes its sub-terms redundant.
    let all: Vec<Candidate> = out.iter().map(|(_, c)| c.clone()).collect();
    out.retain(|(_, c)| {
        let me = format!(" {} ", c.term.to_lowercase());
        !all.iter().any(|b| {
            b.term.len() > c.term.len()
                && b.keys == c.keys
                && format!(" {} ", b.term.to_lowercase()).contains(&me)
        })
    });
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.term.cmp(&b.1.term)));
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

/// Keys (by locale) whose existing translation violates `term`, inflected forms accepted.
/// `names` (do-not-translate terms) mask their occurrences: "Acme Invoices" is not the noun "invoice".
pub fn inconsistent(ws: &Workspace, term: &Term, names: &[Term]) -> BTreeMap<Locale, Vec<Key>> {
    let mut g = Glossary {
        terms: names.to_vec(),
    };
    g.upsert(term.clone());
    let mut out: BTreeMap<Locale, Vec<Key>> = BTreeMap::new();
    for loc in &ws.config.target_locales {
        let Some(cat) = ws.catalogs.get(loc) else {
            continue;
        };
        for (k, s) in &ws.source().entries {
            if let Some(t) = cat.entries.get(k).filter(|t| !t.trim().is_empty())
                && g.violations(s, t, loc, true)
                    .iter()
                    .any(|(v, _)| v.term == term.term)
            {
                out.entry(loc.clone()).or_default().push(k.clone());
            }
        }
    }
    out
}

/// A do-not-translate term must be a name: never used lowercase in the source ("Invoices" in
/// "Acme Invoices" vs "your invoices"), never a word of a translated term.
fn is_name(term: &str, source: &Catalog, common: &[Term]) -> bool {
    let lower_use = term.chars().any(char::is_uppercase)
        && source
            .entries
            .values()
            .any(|s| find(s, term, false, true) && !find(s, term, true, false));
    !lower_use && !common.iter().any(|c| find(&c.term, term, false, true))
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
    let usage = Usage::of_completion(&c);
    let parsed = parse_json(&c.text).unwrap_or(Value::Null);
    let mut terms: Vec<(Term, usize)> = vec![];
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
            // Brand names are case-specific: "Acme Invoices" must not swallow the common noun.
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
        terms.push((term, cand.keys.len()));
    }
    let common: Vec<Term> = terms
        .iter()
        .map(|(t, _)| t)
        .chain(&ws.glossary.terms)
        .filter(|t| !t.do_not_translate)
        .cloned()
        .collect();
    terms.retain(|(t, _)| !t.do_not_translate || is_name(&t.term, ws.source(), &common));
    let names: Vec<Term> = terms
        .iter()
        .map(|(t, _)| t)
        .chain(&ws.glossary.terms)
        .filter(|t| t.do_not_translate)
        .cloned()
        .collect();
    let out = terms
        .into_iter()
        .map(|(term, occurrences)| Mined {
            inconsistent: inconsistent(ws, &term, &names),
            occurrences,
            term,
        })
        .collect();
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

    /// Strings from examples/demo-saas after `glot i18nize`.
    fn demo() -> Catalog {
        let mut src = Catalog::new("en");
        for (i, v) in [
            "Welcome to Acme Invoices 👋",
            "<span>A</span>Acme Invoices",
            "Here's what's happening with your invoices today.",
            "You have {count, plural, one {# overdue invoice} other {# overdue invoices}}",
            "{count, plural, one {# seat} other {# seats}} left",
            "1 seat left",
            "{count, plural, one {# step} other {# steps}} left",
            "Due date",
            "Due date",
            "Invoice {number} marked as paid",
            "Send invoices, get paid faster. Cancel anytime.",
            "Get started",
            "Search invoices, customers…",
            "Customers",
            "Deleting the workspace removes all invoices and customers.",
            "Let's get your workspace ready.",
            "Delete workspace",
            "Billed to {customer} · {due_at}",
            "Customer",
            "{team_count} of {count, plural, one {# seat} other {# seats}} used.",
        ]
        .iter()
        .enumerate()
        {
            src.entries.insert(format!("k{i}"), v.to_string());
        }
        src
    }

    #[test]
    fn demo_candidates() {
        let c = candidates(&demo(), &Glossary::default());
        let terms: Vec<&str> = c.iter().map(|c| c.term.as_str()).collect();
        for t in [
            "Acme Invoices",
            "invoice",
            "customer",
            "workspace",
            "seat",
            "due date",
        ] {
            assert!(terms.contains(&t), "{t} missing from {terms:?}");
        }
        for t in [
            "Invoices",
            "invoices",
            "get",
            "left",
            "due",
            "paid",
            "seat seats",
            "step",
            "AAcme",
        ] {
            assert!(!terms.contains(&t), "{t} in {terms:?}");
        }
        let inv = c.iter().find(|c| c.term == "invoice").unwrap();
        assert!(inv.keys.len() >= 5, "plural forms folded: {:?}", inv.keys);
    }

    #[test]
    fn names_are_never_common_words() {
        let src = demo();
        let common = [Term {
            term: "invoice".into(),
            ..Default::default()
        }];
        assert!(is_name("Acme Invoices", &src, &common));
        assert!(
            !is_name("Invoices", &src, &[]),
            "used lowercase in the source"
        );
        assert!(!is_name("Invoice", &src, &common));
        assert_eq!(lemma("companies"), "company");
        assert_eq!(lemma("status"), "status");
        assert_eq!(lemma("boxes"), "box");
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
