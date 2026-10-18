//! Quality score + review routing, translation memory (exact / TMX), cross-provider escalation.
use async_trait::async_trait;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use glot_core::{config::Config, context::KeyContext, validate::Syntax};
use glot_llm::{
    engine::{TranslateOptions, Workspace},
    mock::MockProvider,
    provider::{Completion, CompletionRequest, Provider},
    tm::{TM_FILE, Tm},
    translate::{Item, Request, Translator},
};

const EN: &str = r#"{"home":{"hi":"Hi","welcome":"Welcome back to your glot dashboard today"},"legal":{"terms":"I accept the terms of service and the privacy"}}"#;

fn project(qa: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("glot.toml"),
        format!("source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{{locale}}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n\n[qa]\n{qa}\n"),
    )
    .unwrap();
    fs::create_dir_all(root.join("messages")).unwrap();
    fs::write(root.join("messages/en.json"), EN).unwrap();
    dir
}

#[tokio::test]
async fn threshold_and_risk_route_to_queue() {
    // "⟦fr⟧ Hi" is > 1.1 × "Hi" + 3: a too_long warning, score 85 < 90.
    let dir =
        project("review_threshold = 90\nmax_length_ratio = 1.1\n[qa.risk]\nlegal = [\"legal.*\"]");
    let mut ws = Workspace::open(dir.path()).unwrap();
    let report = ws.translate(TranslateOptions::default()).await.unwrap();
    assert_eq!(report.needs_review["fr"], ["home.hi", "legal.terms"]);

    let lock = &ws.lock.entries["fr"];
    assert_eq!(
        lock["home.hi"].needs_review.as_deref(),
        Some("score 85 < 90")
    );
    assert_eq!(lock["home.hi"].score, Some(85));
    assert_eq!(
        lock["legal.terms"].needs_review.as_deref(),
        Some("risk: legal")
    );
    assert_eq!(lock["home.welcome"].needs_review, None);
    assert_eq!(lock["home.welcome"].score, Some(100));
    let text = fs::read_to_string(dir.path().join("glot.lock")).unwrap();
    assert!(text.contains(r#""needs_review": "risk: legal""#));

    // `check` reports pending entries as warnings.
    let pending = |ws: &Workspace| {
        ws.check()
            .into_iter()
            .filter(|f| f.issue.message.starts_with("pending human review"))
            .count()
    };
    assert_eq!(pending(&ws), 2);

    // Only validated entries feed the TM; approval adds the rest.
    assert_eq!(Tm::load(dir.path()).unwrap().units.len(), 1);
    ws.approve("fr", "home.hi").unwrap();
    ws.set_translation(
        "fr",
        "legal.terms",
        "J'accepte les conditions d'utilisation et la confidentialité",
    )
    .unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    assert_eq!(pending(&ws), 0);
    assert!(ws.lock.entries["fr"]["home.hi"].reviewed);
    assert_eq!(Tm::load(dir.path()).unwrap().units.len(), 3);
}

#[tokio::test]
async fn tm_exact_reuse_costs_zero_and_tmx_roundtrips() {
    let a = project("");
    let mut ws = Workspace::open(a.path()).unwrap();
    let first = ws.translate(TranslateOptions::default()).await.unwrap();
    assert!(first.usage.input_tokens > 0);
    assert_eq!(first.written["fr"].len(), 3);

    // Same catalog again (targets and lock gone): everything comes from the TM, 0 token.
    fs::remove_file(a.path().join("messages/fr.json")).unwrap();
    fs::remove_file(a.path().join("glot.lock")).unwrap();
    let mut ws = Workspace::open(a.path()).unwrap();
    let dry = ws
        .translate(TranslateOptions {
            dry_run: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(dry.planned.is_empty() && dry.estimated_input_tokens == 0);
    assert_eq!(dry.tm_reused["fr"].len(), 3);
    assert!(dry.tm_saved.input_tokens > 0);
    let second = ws.translate(TranslateOptions::default()).await.unwrap();
    assert_eq!(second.usage.input_tokens + second.usage.output_tokens, 0);
    assert_eq!(second.tm_reused["fr"].len(), 3);
    assert_eq!(
        ws.catalogs["fr"].entries["home.hi"], "⟦fr⟧ Hi",
        "TM value written"
    );

    // Another project imports the TMX export: also 0 token.
    let b = project("");
    let tmx = Tm::load(a.path()).unwrap().to_tmx();
    let mut tm = Tm::default();
    for u in Tm::from_tmx(&tmx).unwrap() {
        tm.add(u);
    }
    tm.save(b.path()).unwrap();
    assert!(b.path().join(TM_FILE).exists());
    let mut ws = Workspace::open(b.path()).unwrap();
    let r = ws.translate(TranslateOptions::default()).await.unwrap();
    assert_eq!(r.usage.input_tokens, 0);
    assert_eq!(r.tm_reused["fr"].len(), 3);

    // A fuzzy-only match still goes to the LLM.
    fs::write(
        b.path().join("messages/en.json"),
        EN.replace("today", "now!"),
    )
    .unwrap();
    let mut ws = Workspace::open(b.path()).unwrap();
    let r = ws.translate(TranslateOptions::default()).await.unwrap();
    assert!(r.usage.input_tokens > 0);
    assert_eq!(r.written["fr"].len(), 1);
}

/// Counts calls; `drop_placeholders` simulates a weak local model, `low_judge` a judge that finds a critical error.
struct Fake {
    calls: Arc<AtomicU32>,
    drop_placeholders: bool,
    low_judge: bool,
}

#[async_trait]
impl Provider for Fake {
    fn name(&self) -> &str {
        "fake"
    }
    async fn complete(&self, req: CompletionRequest) -> glot_llm::Result<Completion> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.low_judge && req.user.contains("\"task\": \"judge\"") {
            return Ok(Completion {
                text: r#"{"verdicts": {"hello": {"errors": [{"category": "accuracy", "severity": "critical", "description": "wrong meaning"}], "suggestion": null}}}"#.into(),
                model: req.model,
                ..Default::default()
            });
        }
        let mut c = MockProvider.complete(req).await?;
        if self.drop_placeholders {
            c.text = c.text.replace("{name}", "");
        }
        Ok(c)
    }
}

fn fake(calls: &Arc<AtomicU32>, drop_placeholders: bool, low_judge: bool) -> Box<dyn Provider> {
    Box::new(Fake {
        calls: calls.clone(),
        drop_placeholders,
        low_judge,
    })
}

fn request() -> Request {
    Request {
        source_locale: "en".into(),
        target_locale: "fr".into(),
        items: vec![Item {
            key: "hello".into(),
            source: "Hello {name}".into(),
            context: KeyContext::default(),
            previous: None,
        }],
        glossary: vec![],
        syntax: Syntax::Icu,
        style: None,
    }
}

fn config(judge: bool) -> Config {
    let judge = if judge {
        "judge_model = \"local-judge\"\n"
    } else {
        ""
    };
    toml::from_str(&format!(
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"m/{{locale}}.json\"]\n[llm]\nprovider = \"local\"\nmodel = \"translategemma\"\nfallback_model = \"remote\"\nfallback_provider = \"anthropic\"\n{judge}"
    ))
    .unwrap()
}

#[tokio::test]
async fn validator_failure_escalates_to_other_provider() {
    let (local, remote) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let t = Translator::new(&config(false), fake(&local, true, false))
        .with_fallback_provider(Some(fake(&remote, false, false)));
    let (out, _) = t.translate(request()).await.unwrap();
    assert_eq!(out[0].value, "⟦fr⟧ Hello {name}");
    assert_eq!(out[0].model, "remote");
    assert_eq!(local.load(Ordering::SeqCst), 3, "first pass + 2 retries");
    assert_eq!(remote.load(Ordering::SeqCst), 1, "one escalation");
}

#[tokio::test]
async fn low_judge_score_escalates_to_other_provider() {
    let (local, remote) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let t = Translator::new(&config(true), fake(&local, false, true))
        .with_fallback_provider(Some(fake(&remote, false, false)));
    let (out, _) = t.translate(request()).await.unwrap();
    assert_eq!(out[0].model, "remote");
    assert_eq!(out[0].score, None);
    assert_eq!(local.load(Ordering::SeqCst), 2, "first pass + judge");
    assert_eq!(remote.load(Ordering::SeqCst), 1);
}
