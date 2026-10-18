//! Copy variants by intention for a coherent group of keys (title + subtitle + CTA): written once in the source
//! language by a copywriter prompt, then translated into every target locale by the normal guarded pipeline
//! (`translate::Translator`: validator, glossary, pixel budget, retries, fallback model). Every (key, locale)
//! then goes through the deterministic copy policy (`glot_core::policy`). A variant is usable only when all its
//! cells pass: no validator error, no policy error.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use glot_core::{
    Key, Locale,
    context::KeyContext,
    glossary::{Glossary, Term},
    measure,
    policy::{self, Finding, PolicyConfig},
    validate::{self, Constraints, Issue, Syntax},
};

use crate::{
    Result,
    provider::{CompletionRequest, Provider},
    translate::{self, Item, Request, Translator, Usage},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    SocialProof,
    DirectBenefit,
    HonestUrgency,
    Simplicity,
    Curiosity,
    Authority,
    Reassurance,
    /// Speaks to the targeted segment ("for teams in Germany").
    Segment,
}

impl Intent {
    pub const ALL: [Intent; 8] = [
        Intent::SocialProof,
        Intent::DirectBenefit,
        Intent::HonestUrgency,
        Intent::Simplicity,
        Intent::Curiosity,
        Intent::Authority,
        Intent::Reassurance,
        Intent::Segment,
    ];

    /// "social proof", "Social-Proof", "social_proof" -> SocialProof.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase().replace([' ', '-'], "_");
        Self::ALL.into_iter().find(|i| i.id() == s)
    }

    pub fn id(self) -> &'static str {
        match self {
            Intent::SocialProof => "social_proof",
            Intent::DirectBenefit => "direct_benefit",
            Intent::HonestUrgency => "honest_urgency",
            Intent::Simplicity => "simplicity",
            Intent::Curiosity => "curiosity",
            Intent::Authority => "authority",
            Intent::Reassurance => "reassurance",
            Intent::Segment => "segment",
        }
    }

    fn guide(self) -> &'static str {
        match self {
            Intent::SocialProof => {
                "show that others like the reader use it; cite only `facts` (customers, ratings), verbatim; no facts = speak of peers without numbers"
            }
            Intent::DirectBenefit => {
                "lead with the concrete outcome for the reader, in their words"
            }
            Intent::HonestUrgency => {
                "give a real reason to act now: a deadline or stock only if a fact states it, otherwise the cost of waiting (\"ship this week, not next quarter\"); never invented scarcity"
            }
            Intent::Simplicity => "make it feel effortless: fewer words, one idea, plain verbs",
            Intent::Curiosity => {
                "open a question the page answers, without clickbait or withheld essentials"
            }
            Intent::Authority => {
                "lean on expertise or recognitions stated in `facts` only; never rankings (\"#1\", \"best\", \"leader\") unless a fact states them"
            }
            Intent::Reassurance => {
                "remove the fear of trying (no card, cancel anytime, data stays yours) only if true per the official text or facts"
            }
            Intent::Segment => {
                "address the targeted segment explicitly and naturally (their country, role or situation)"
            }
        }
    }
}

/// One key of the group, with its official text in every locale (`control`, source locale included).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyKey {
    pub key: Key,
    pub source: String,
    /// Official text per locale (the control arm): numbers, prices and legal mentions are checked against it.
    pub control: BTreeMap<Locale, String>,
    /// ui_role, description, max_length, fit (pixel budget)…
    pub context: KeyContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    pub source_locale: Locale,
    /// Target locales (the source locale is always produced).
    pub locales: Vec<Locale>,
    pub keys: Vec<CopyKey>,
    pub intents: Vec<Intent>,
    /// Where the group is shown ("pricing page hero").
    pub page: Option<String>,
    /// Brand tone (project style guide).
    pub style: Option<String>,
    /// Human description of the targeted segment ("teams in Germany coming from search").
    pub segment: Option<String>,
    /// The only claims a variant may cite ("2 000 teams", "rated 4.8/5", "offer ends 31 Oct").
    pub facts: Vec<String>,
    pub syntax: Syntax,
    pub glossary: Vec<Term>,
    pub policy: PolicyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    pub key: Key,
    pub locale: Locale,
    pub value: String,
    pub issues: Vec<Issue>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Generated {
    pub intent: Intent,
    /// The copywriter's one-line reason (shown in the wizard).
    pub rationale: String,
    /// Every (key, locale) produced, source locale first.
    pub cells: Vec<Cell>,
    /// All cells free of validator and policy errors.
    pub ok: bool,
}

impl Generated {
    /// `{locale: {key: value}}` (the experiment's arm values).
    pub fn values(&self) -> BTreeMap<Locale, BTreeMap<Key, String>> {
        let mut out: BTreeMap<Locale, BTreeMap<Key, String>> = BTreeMap::new();
        for c in &self.cells {
            out.entry(c.locale.clone())
                .or_default()
                .insert(c.key.clone(), c.value.clone());
        }
        out
    }
}

const MAX_RETRIES: usize = 2;

/// Source-language variants (1 call + up to 2 targeted retries), then one guarded translation per locale.
pub async fn generate(
    provider: &dyn Provider,
    model: &str,
    translator: &Translator,
    b: &Brief,
) -> Result<(Vec<Generated>, Usage)> {
    let mut usage = Usage::default();
    let mut drafts: BTreeMap<Intent, (String, Vec<Cell>)> = BTreeMap::new();
    for attempt in 0..=MAX_RETRIES {
        let todo: Vec<Intent> = b
            .intents
            .iter()
            .copied()
            .filter(|i| drafts.get(i).is_none_or(|d| !cells_ok(&d.1)))
            .collect();
        if todo.is_empty() {
            break;
        }
        let rejected: Vec<Value> = todo
            .iter()
            .filter_map(|i| drafts.get(i).map(|d| (i, d)))
            .map(|(i, (_, cells))| json!({"intent": i, "texts": cells.iter().map(|c| (c.key.clone(), json!(c.value))).collect::<serde_json::Map<_, _>>(), "issues_to_fix": problems(cells)}))
            .collect();
        let req = request(b, model, &todo, &rejected);
        let call = provider.complete(req).await;
        let c = match call {
            Ok(c) => c,
            Err(e) if attempt == 0 => return Err(e),
            Err(e) => {
                tracing::warn!("copy retry failed: {e}");
                break;
            }
        };
        usage.add(&Usage::of_completion(&c));
        let parsed = translate::parse_json(&c.text).unwrap_or(Value::Null);
        for v in parsed["variants"].as_array().into_iter().flatten() {
            let Some(intent) = v["intent"].as_str().and_then(Intent::parse) else {
                continue;
            };
            if !todo.contains(&intent) {
                continue;
            }
            let cells = b
                .keys
                .iter()
                .map(|k| {
                    let value = v["texts"][&k.key].as_str().unwrap_or("").trim().to_string();
                    check_source(b, k, value)
                })
                .collect();
            let rationale = v["rationale"].as_str().unwrap_or("").to_string();
            drafts.insert(intent, (rationale, cells));
        }
    }

    // Translate every usable draft: one guarded batch per locale, items keyed "intent|key".
    let ready: Vec<Intent> = b
        .intents
        .iter()
        .copied()
        .filter(|i| drafts.get(i).is_some_and(|d| cells_ok(&d.1)))
        .collect();
    let mut out: BTreeMap<Intent, Generated> = BTreeMap::new();
    for i in &b.intents {
        let (rationale, cells) = drafts.remove(i).unwrap_or_default();
        out.insert(
            *i,
            Generated {
                intent: *i,
                rationale,
                ok: false,
                cells,
            },
        );
    }
    let targets: Vec<&Locale> = b
        .locales
        .iter()
        .filter(|l| **l != b.source_locale && !ready.is_empty())
        .collect();
    let mut requests = vec![];
    for locale in &targets {
        let items: Vec<Item> = ready
            .iter()
            .flat_map(|i| {
                let src = &out[i].cells;
                b.keys.iter().zip(src).map(move |(k, c)| Item {
                    key: format!("{}|{}", i.id(), k.key),
                    source: c.value.clone(),
                    context: KeyContext {
                        description: Some(format!(
                            "Marketing variant ({}) of `{}`{}. Keep its persuasion intent and length; add no claim, number, deadline or superlative.{}",
                            i.id(),
                            k.key,
                            b.segment.as_deref().map(|s| format!(" for {s}")).unwrap_or_default(),
                            k.context.description.as_deref().map(|d| format!(" Key note: {d}")).unwrap_or_default(),
                        )),
                        ..k.context.clone()
                    },
                    previous: None,
                })
            })
            .collect();
        let glossary: Vec<Term> = Glossary {
            terms: b.glossary.clone(),
        }
        .matches(
            &items
                .iter()
                .map(|i| i.source.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .into_iter()
        .cloned()
        .collect();
        requests.push(Request {
            source_locale: b.source_locale.clone(),
            target_locale: (*locale).clone(),
            items,
            glossary,
            syntax: b.syntax,
            style: b.style.clone(),
        });
    }
    // Locales in parallel (each batch already retries / escalates on its own).
    let results =
        futures::future::join_all(requests.into_iter().map(|r| translator.translate(r))).await;
    for (locale, r) in targets.into_iter().zip(results) {
        let (done, u) = r?;
        usage.add(&u);
        for t in done {
            let Some((intent, key)) = t.key.split_once('|') else {
                continue;
            };
            let (Some(intent), Some(k)) =
                (Intent::parse(intent), b.keys.iter().find(|k| k.key == key))
            else {
                continue;
            };
            let findings = policy::check(&t.value, locale, control(k, locale), &b.facts, &b.policy);
            if let Some(g) = out.get_mut(&intent) {
                g.cells.push(Cell {
                    key: k.key.clone(),
                    locale: locale.clone(),
                    value: t.value,
                    issues: t.issues,
                    findings,
                });
            }
        }
    }
    let n = b.keys.len() * (1 + b.locales.iter().filter(|l| **l != b.source_locale).count());
    let out = out
        .into_values()
        .map(|mut g| {
            g.ok = g.cells.len() == n && cells_ok(&g.cells);
            g
        })
        .collect();
    Ok((out, usage))
}

fn control<'a>(k: &'a CopyKey, locale: &str) -> &'a str {
    k.control
        .get(locale)
        .map(String::as_str)
        .unwrap_or(&k.source)
}

fn cells_ok(cells: &[Cell]) -> bool {
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !validate::has_errors(&c.issues) && !policy::has_errors(&c.findings))
}

fn problems(cells: &[Cell]) -> Vec<String> {
    cells
        .iter()
        .flat_map(|c| {
            let i = c
                .issues
                .iter()
                .filter(|i| i.severity == validate::Severity::Error)
                .map(|i| i.message.clone());
            let f = c
                .findings
                .iter()
                .filter(|f| f.severity == validate::Severity::Error)
                .map(|f| f.message.clone());
            i.chain(f).map(move |m| format!("{}: {m}", c.key))
        })
        .collect()
}

/// Same checks a translation gets (placeholders, ICU, tags, length, pixel budget, glossary), against the
/// official source text, plus the copy policy.
fn check_source(b: &Brief, k: &CopyKey, value: String) -> Cell {
    let fit = k.context.fit.as_ref();
    let mut issues = if value.is_empty() {
        vec![Issue {
            code: validate::IssueCode::Empty,
            severity: validate::Severity::Error,
            message: "no text for this key".into(),
        }]
    } else {
        let c = Constraints {
            max_length: k.context.max_length,
            max_length_ratio: fit.is_none().then_some(1.6),
        };
        validate::validate(&k.source, &value, &b.source_locale, b.syntax, &c)
            .into_iter()
            .filter(|i| i.code != validate::IssueCode::Untranslated)
            .collect()
    };
    issues.extend(fit.and_then(|f| measure::check(&value, b.syntax, f)));
    issues.extend(
        Glossary {
            terms: b.glossary.clone(),
        }
        .check(&k.source, &value, &b.source_locale),
    );
    let findings = policy::check(
        &value,
        &b.source_locale,
        control(k, &b.source_locale),
        &b.facts,
        &b.policy,
    );
    Cell {
        key: k.key.clone(),
        locale: b.source_locale.clone(),
        value,
        issues,
        findings,
    }
}

fn request(b: &Brief, model: &str, todo: &[Intent], rejected: &[Value]) -> CompletionRequest {
    let mut system = format!(
        "You are a senior conversion copywriter for a SaaS product, writing UI copy in `{}`. For each requested \
intent, write ONE coherent variant of a group of UI strings shown together (e.g. title + subtitle + call to action): \
the strings must read as one message.\n\nRules — a variant breaking any of them is rejected automatically:\n\
1. Keep each string's job (`ui_role`) and the product promise of the official text; buttons stay short verbs.\n\
2. Keep every placeholder, ICU plural/select and markup tag exactly as in `source`.\n\
3. Cite only `facts`, verbatim. Never invent numbers, percentages, prices, dates, deadlines, stock levels, live \
counters (\"12 people are viewing\"), rankings (\"#1\", \"best\", \"leader\") or guarantees.\n\
4. Never change or drop a price or legal mention of the official text (\"incl. VAT\", \"*\").\n\
5. Decline or secondary options stay neutral: never shame the reader (\"No thanks, I prefer paying more\").\n\
6. Respect `max_length` and `fit` (pixel budget measured in the app): pick a shorter wording, never truncate.\n\
7. When `rejected` is given, those attempts failed for the listed reasons: fix them all.\n",
        b.source_locale
    );
    if let Some(s) = b.style.as_deref().filter(|s| !s.trim().is_empty()) {
        system += &format!("\nBrand style guide (follow it): {s}\n");
    }
    system += "\nAnswer with JSON only: {\"variants\": [{\"intent\": \"<intent id>\", \"rationale\": \"<one sentence>\", \"texts\": {\"<key>\": \"<text>\"}}]}";
    let keys: Vec<Value> = b
        .keys
        .iter()
        .map(|k| {
            json!({
                "key": k.key,
                "source": k.source,
                "official": control(k, &b.source_locale),
                "ui_role": k.context.ui_role,
                "description": k.context.description,
                "max_length": k.context.max_length,
                "fit": k.context.fit.as_ref().map(measure::hint),
            })
        })
        .collect();
    let payload = json!({
        "task": "copy_variants",
        "source_locale": b.source_locale,
        "page": b.page,
        "segment": b.segment,
        "facts": b.facts,
        "intents": todo.iter().map(|i| json!({"id": i, "guide": i.guide()})).collect::<Vec<_>>(),
        "keys": keys,
        "rejected": rejected,
    });
    CompletionRequest {
        model: model.into(),
        system,
        user: translate::payload_block(&payload),
        max_tokens: 4096,
        json: true,
        images: vec![],
    }
}

/// Offline answer (`provider = "mock"`): the official text with an intent tag, placeholders kept.
pub(crate) fn mock(p: &Value) -> Value {
    let variants: Vec<Value> = p["intents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["id"].as_str())
        .map(|id| {
            let texts: serde_json::Map<String, Value> = p["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| {
                    Some((
                        k["key"].as_str()?.to_string(),
                        json!(format!(
                            "{} ({})",
                            k["official"].as_str()?,
                            id.replace('_', " ")
                        )),
                    ))
                })
                .collect();
            json!({"intent": id, "rationale": format!("mock {id}"), "texts": texts})
        })
        .collect();
    json!({ "variants": variants })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Completion;
    use async_trait::async_trait;
    use glot_core::config::Config;
    use std::sync::Mutex;

    fn brief(intents: Vec<Intent>) -> Brief {
        let key = |k: &str, en: &str, fr: &str, de: &str, role: &str| CopyKey {
            key: k.into(),
            source: en.into(),
            control: [("en", en), ("fr", fr), ("de", de)]
                .into_iter()
                .map(|(l, v)| (l.into(), v.into()))
                .collect(),
            context: KeyContext {
                key: k.into(),
                ui_role: Some(role.into()),
                ..Default::default()
            },
        };
        Brief {
            source_locale: "en".into(),
            locales: vec!["en".into(), "fr".into(), "de".into()],
            keys: vec![
                key(
                    "hero.title",
                    "Translate your app",
                    "Traduisez votre app",
                    "Übersetzen Sie Ihre App",
                    "heading",
                ),
                key(
                    "hero.cta",
                    "Start free",
                    "Commencer gratuitement",
                    "Kostenlos starten",
                    "button",
                ),
            ],
            intents,
            page: Some("home hero".into()),
            style: Some("friendly, concise".into()),
            segment: Some("teams in Germany".into()),
            facts: vec!["2,000 teams".into()],
            syntax: Syntax::Icu,
            glossary: vec![],
            policy: PolicyConfig::default(),
        }
    }

    fn translator() -> Translator {
        let cfg: Config = toml::from_str(
            "source_locale = \"en\"\ntarget_locales = [\"fr\", \"de\"]\nfiles = []\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"",
        )
        .unwrap();
        Translator::new(&cfg, Box::new(crate::mock::MockProvider))
    }

    #[tokio::test]
    async fn mock_generates_every_key_and_locale() {
        let b = brief(vec![Intent::SocialProof, Intent::Segment]);
        let (out, _) = generate(&crate::mock::MockProvider, "mock", &translator(), &b)
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        for g in &out {
            assert!(g.ok, "{g:?}");
            let v = g.values();
            assert_eq!(v.len(), 3);
            assert!(v["de"]["hero.cta"].starts_with("⟦de⟧"), "{v:?}");
        }
    }

    /// Scripted model: first a fake-urgency + "#1" variant, then a clean one once the policy reasons come back.
    struct Scripted(Mutex<Vec<String>>, Mutex<Vec<String>>);
    #[async_trait]
    impl Provider for Scripted {
        fn name(&self) -> &str {
            "scripted"
        }
        async fn complete(&self, req: CompletionRequest) -> Result<Completion> {
            self.1.lock().unwrap().push(req.user.clone());
            Ok(Completion {
                text: self.0.lock().unwrap().remove(0),
                model: "scripted".into(),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn policy_errors_are_sent_back_and_fixed() {
        let bad = r#"{"variants":[{"intent":"honest_urgency","rationale":"r","texts":{"hero.title":"Only 2 seats left: the #1 translator","hero.cta":"Start free"}}]}"#;
        let good = r#"{"variants":[{"intent":"honest_urgency","rationale":"r","texts":{"hero.title":"Ship your app in every language this week","hero.cta":"Start free"}}]}"#;
        let p = Scripted(
            Mutex::new(vec![bad.into(), good.into()]),
            Mutex::new(vec![]),
        );
        let (out, _) = generate(&p, "m", &translator(), &brief(vec![Intent::HonestUrgency]))
            .await
            .unwrap();
        assert!(out[0].ok, "{:?}", out[0]);
        let calls = p.1.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[1].contains("fake_urgency") || calls[1].contains("urgency or scarcity"),
            "{}",
            calls[1]
        );
        assert!(calls[1].contains("ranking"), "{}", calls[1]);
    }

    #[tokio::test]
    async fn stubborn_policy_violation_is_rejected_not_served() {
        let bad = r#"{"variants":[{"intent":"authority","rationale":"r","texts":{"hero.title":"The best translator, guaranteed","hero.cta":"Start free"}}]}"#;
        let p = Scripted(Mutex::new(vec![bad.into(); 3]), Mutex::new(vec![]));
        let (out, _) = generate(&p, "m", &translator(), &brief(vec![Intent::Authority]))
            .await
            .unwrap();
        assert!(!out[0].ok);
        assert_eq!(out[0].cells.len(), 2, "not translated");
        assert!(
            out[0].cells[0]
                .findings
                .iter()
                .any(|f| f.rule == "unprovable_claim")
        );
    }

    #[test]
    fn intents_parse() {
        assert_eq!(Intent::parse("Social proof"), Some(Intent::SocialProof));
        assert_eq!(Intent::parse("honest-urgency"), Some(Intent::HonestUrgency));
        assert_eq!(Intent::parse("dark pattern"), None);
    }

    /// One real call when ANTHROPIC_API_KEY is set (skipped otherwise): `cargo test -p glot-llm real_copy -- --nocapture`.
    #[tokio::test]
    async fn real_copy_variants_when_key_present() {
        let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
            eprintln!("ANTHROPIC_API_KEY unset: skipped");
            return;
        };
        let model = "claude-opus-5-5";
        let cfg = glot_core::config::LlmConfig {
            provider: "anthropic".into(),
            model: model.into(),
            ..Default::default()
        };
        let p = crate::provider::from_config(&cfg, Some(key)).unwrap();
        let mut b = brief(vec![Intent::SocialProof, Intent::HonestUrgency]);
        b.locales = vec!["en".into()];
        let (out, u) = generate(p.as_ref(), model, &translator(), &b)
            .await
            .unwrap();
        eprintln!(
            "{}\ncost ${:.4}",
            serde_json::to_string_pretty(&out).unwrap(),
            u.cost_usd
        );
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|g| g.cells.len() == 2));
    }
}
