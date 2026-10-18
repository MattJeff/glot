use async_trait::async_trait;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

use glot_core::{config::Config, context::KeyContext, lock::Origin, validate::Syntax};
use glot_llm::{
    engine::{TranslateOptions, Workspace},
    mock::MockProvider,
    provider::{Completion, CompletionRequest, Provider},
    translate::{Item, Request, Translator},
};

fn project(extra_llm: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("glot.toml"),
        format!("source_locale = \"en\"\ntarget_locales = [\"fr\", \"ja\"]\nfiles = [\"messages/{{locale}}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\njudge_model = \"mock\"\n{extra_llm}\n"),
    )
    .unwrap();
    fs::write(root.join("glossary.toml"), "[[term]]\nterm = \"Workspace\"\ntranslations = { fr = \"Espace de travail\" }\n\n[[term]]\nterm = \"glot\"\ndo_not_translate = true\n").unwrap();
    fs::create_dir_all(root.join("messages")).unwrap();
    fs::write(
        root.join("messages/en.json"),
        r#"{"auth":{"login":{"cta":"Sign in to glot","hello":"Hello <b>{name}</b>"}},"files":{"count":"{n, plural, one {# file} other {# files}}"},"ws":{"title":"Your Workspace"}}"#,
    )
    .unwrap();
    // Pre-existing (untracked) translation.
    fs::write(
        root.join("messages/fr.json"),
        r#"{"ws":{"title":"Votre Espace de travail"},"old":{"gone":"Supprimé"}}"#,
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn mock_pipeline_end_to_end() {
    let dir = project("");
    let mut ws = Workspace::open(&dir.path().join("messages")).unwrap();
    assert_eq!(ws.status().locales["fr"].missing, 3);

    let dry = ws
        .translate(TranslateOptions {
            dry_run: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(dry.planned["fr"].len(), 3);
    assert_eq!(dry.planned["ja"].len(), 4);
    assert!(dry.written.is_empty());
    assert!(!dir.path().join("glot.lock").exists());

    let report = ws.translate(TranslateOptions::default()).await.unwrap();
    assert!(report.rejected.is_empty(), "{:#?}", report.rejected);
    assert_eq!(report.written["fr"].len(), 3);
    assert_eq!(report.written["ja"].len(), 4);
    assert!(report.written["fr"].iter().all(|t| t.score == Some(100)));
    // Dry-run estimate is built from the real prompts: within ±35 % of what the run consumed.
    let (est, real) = (
        dry.estimated_input_tokens as f64,
        report.usage.input_tokens as f64,
    );
    assert!(
        (est / real - 1.0).abs() <= 0.35,
        "estimated {est} vs real {real} input tokens"
    );
    assert!(dry.estimated_output_tokens > 0);

    let ws = Workspace::open(dir.path()).unwrap();
    let fr = &ws.catalogs["fr"].entries;
    assert_eq!(fr["auth.login.hello"], "⟦fr⟧ Hello <b>{name}</b>");
    assert_eq!(fr["auth.login.cta"], "⟦fr⟧ Sign in to glot");
    assert_eq!(
        ws.catalogs["ja"].entries["files.count"],
        "⟦ja⟧ {n, plural, other {# files}}"
    );
    assert!(
        ws.check()
            .iter()
            .all(|f| f.issue.severity != glot_core::validate::Severity::Error),
        "{:#?}",
        ws.check()
    );

    let lock = &ws.lock.entries["fr"];
    assert_eq!(
        lock["auth.login.cta"].origin,
        Origin::Machine {
            model: "mock".into()
        }
    );
    assert_eq!(lock["ws.title"].origin, Origin::Imported);
    assert!(!lock.contains_key("old.gone"));
    let plan = ws.plan();
    assert!(plan.missing.is_empty() && plan.stale.is_empty() && plan.untracked.is_empty());
}

#[tokio::test]
async fn workspace_editing_ops() {
    let dir = project("");
    let mut ws = Workspace::open(dir.path()).unwrap();
    ws.add_source("settings.save", "Save changes", Some("Primary button"))
        .unwrap();
    let p = ws.propose("save   CHANGES!", Some("Settings.Save Button"));
    assert_eq!(p.key, "settings.save_button");
    assert_eq!(
        p.duplicates,
        vec![("settings.save".to_string(), "Save changes".to_string())]
    );
    assert!(!p.key_exists);
    assert_eq!(
        ws.propose("Open Workspace", None).glossary_terms,
        vec!["Workspace"]
    );

    let bad = ws
        .set_translation("fr", "auth.login.hello", "Bonjour")
        .unwrap();
    assert!(glot_core::validate::has_errors(&bad));
    assert!(!ws.catalogs["fr"].entries.contains_key("auth.login.hello"));
    ws.set_translation("fr", "auth.login.hello", "Bonjour <b>{name}</b>")
        .unwrap();

    ws.remove_keys(&["auth.login.cta".into()]).unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    assert_eq!(
        ws.catalogs["fr"].entries["auth.login.hello"],
        "Bonjour <b>{name}</b>"
    );
    assert_eq!(
        ws.lock.entries["fr"]["auth.login.hello"].origin,
        Origin::Human
    );
    assert_eq!(ws.lock.notes["settings.save"], "Primary button");
    assert!(!ws.source().entries.contains_key("auth.login.cta"));
}

#[tokio::test]
async fn budget_is_enforced() {
    let dir = project("budget_usd = 0.0000001");
    fs::write(
        dir.path().join("glot.toml"),
        fs::read_to_string(dir.path().join("glot.toml"))
            .unwrap()
            .replace("model = \"mock\"", "model = \"claude-sonnet-5-5\""),
    )
    .unwrap();
    let mut ws = Workspace::open(dir.path()).unwrap();
    let err = ws.translate(TranslateOptions::default()).await.unwrap_err();
    assert!(matches!(err, glot_llm::Error::Budget { .. }), "{err}");
}

/// Drops `{name}` for the "cheap" model; "strong" behaves.
struct Breaker {
    calls: AtomicU32,
}

#[async_trait]
impl Provider for Breaker {
    fn name(&self) -> &str {
        "breaker"
    }
    async fn complete(&self, req: CompletionRequest) -> glot_llm::Result<Completion> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let model = req.model.clone();
        let mut c = MockProvider.complete(req).await?;
        if model == "cheap" {
            c.text = c.text.replace("{name}", "");
        }
        Ok(c)
    }
}

fn config(fallback: Option<&str>) -> Config {
    let mut cfg: Config = toml::from_str(
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"{locale}.json\"]",
    )
    .unwrap();
    cfg.llm.model = "cheap".into();
    cfg.llm.fallback_model = fallback.map(Into::into);
    cfg
}

fn request() -> Request {
    let item = |key: &str, source: &str| Item {
        key: key.into(),
        source: source.into(),
        context: KeyContext::default(),
        previous: None,
    };
    Request {
        source_locale: "en".into(),
        target_locale: "fr".into(),
        items: vec![item("a", "Hello {name}"), item("b", "Goodbye")],
        glossary: vec![],
        syntax: Syntax::Icu,
        style: None,
    }
}

#[tokio::test]
async fn retries_then_escalates() {
    let p = Box::new(Breaker {
        calls: AtomicU32::new(0),
    });
    let t = Translator::new(&config(Some("strong")), p);
    let (out, _) = t.translate(request()).await.unwrap();
    let a = out.iter().find(|t| t.key == "a").unwrap();
    assert_eq!(
        (a.model.as_str(), a.attempts, a.value.as_str()),
        ("strong", 4, "⟦fr⟧ Hello {name}")
    );
    assert!(!glot_core::validate::has_errors(&a.issues));
    let b = out.iter().find(|t| t.key == "b").unwrap();
    assert_eq!((b.model.as_str(), b.attempts), ("cheap", 1));
}

#[tokio::test]
async fn rejected_without_fallback() {
    let t = Translator::new(
        &config(None),
        Box::new(Breaker {
            calls: AtomicU32::new(0),
        }),
    );
    let (out, _) = t.translate(request()).await.unwrap();
    let a = out.iter().find(|t| t.key == "a").unwrap();
    assert_eq!(a.attempts, 3);
    assert!(glot_core::validate::has_errors(&a.issues));
}

/// Real API call: `ANTHROPIC_API_KEY=... cargo test -p glot-llm -- --ignored`.
#[tokio::test]
#[ignore]
async fn anthropic_live() {
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        return;
    }
    let mut cfg = config(Some("claude-sonnet-5-5"));
    cfg.llm.provider = "anthropic".into();
    cfg.llm.model = "claude-haiku-5-5".into();
    cfg.llm.judge_model = Some("claude-haiku-5-5".into());
    let t = Translator::new(
        &cfg,
        glot_llm::provider::from_config(&cfg.llm, None).unwrap(),
    );
    let mut req = request();
    req.items[1].source = "{n, plural, one {# file deleted} other {# files deleted}}".into();
    let (out, usage) = t.translate(req).await.unwrap();
    eprintln!("{out:#?}\n{usage:?}");
    assert!(
        out.iter()
            .all(|t| !glot_core::validate::has_errors(&t.issues))
    );
    assert!(usage.cost_usd > 0.0);
}

/// rmcp / axum spawn these futures: they must stay `Send`.
#[allow(dead_code)]
fn futures_are_send(ws: &mut Workspace, t: &Translator) {
    fn assert_send<T: Send>(_: T) {}
    assert_send(ws.translate(TranslateOptions::default()));
    assert_send(t.translate(request()));
}
