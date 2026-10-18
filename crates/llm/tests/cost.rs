//! Cost accounting: estimate vs real with forced retries, and `translate --batch` against a fake
//! Message Batches API (local HTTP server, no real LLM call).
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};

use glot_core::{config::Config, context::KeyContext, validate::Syntax};
use glot_llm::{
    engine::{BATCH_FILE, TranslateOptions, Workspace},
    mock::MockProvider,
    provider::{Completion, CompletionRequest, Provider},
    translate::{self, Item, Request, Translator, estimate_tokens},
};

/// Mock answers, billed like a real thinking model: heuristic token counts, ×3.5 reasoning (min 400),
/// no cache hit. The main model drops `{name}` on its first two attempts (=> 2 retries + escalation).
struct Flaky {
    attempts: Mutex<BTreeMap<String, u32>>,
}

#[async_trait]
impl Provider for Flaky {
    fn name(&self) -> &str {
        "flaky"
    }
    async fn complete(&self, req: CompletionRequest) -> glot_llm::Result<Completion> {
        let input = estimate_tokens(&req.system) + estimate_tokens(&req.user);
        let model = req.model.clone();
        let mut c = MockProvider.complete(req).await?;
        if model == "claude-haiku-5-5" {
            let mut a = self.attempts.lock().unwrap();
            let n = a.entry(model.clone()).or_default();
            *n += 1;
            if *n <= 3 {
                // calls 1-3 = first pass + 2 retries of the single batch
                c.text = c.text.replace("{name}", "");
            }
        }
        c.output_tokens = ((estimate_tokens(&c.text) as f64 * 3.5) as u32).max(400);
        c.input_tokens = input as u32;
        c.model = model;
        Ok(c)
    }
}

#[tokio::test]
async fn forced_retries_stay_within_estimate_range() {
    let mut cfg: Config = toml::from_str(
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"{locale}.json\"]",
    )
    .unwrap();
    cfg.llm.model = "claude-haiku-5-5".into();
    cfg.llm.fallback_model = Some("claude-sonnet-5-5".into());
    let items: Vec<Item> = (0..40)
        .map(|i| Item {
            key: format!("screen.item_{i}"),
            source: if i % 10 == 0 {
                format!(
                    "Hello {{name}}, you have {{n, plural, one {{# file}} other {{# files}}}} ({i})"
                )
            } else {
                format!("Save the changes of section {i} before leaving")
            },
            context: KeyContext {
                description: Some("Settings page".into()),
                ..Default::default()
            },
            previous: None,
        })
        .collect();
    let req = Request {
        source_locale: "en".into(),
        target_locale: "fr".into(),
        items,
        glossary: vec![],
        syntax: Syntax::Icu,
        style: None,
    };
    let est = translate::estimate_full(&cfg, &req, &BTreeMap::new());
    let t = Translator::new(
        &cfg,
        Box::new(Flaky {
            attempts: Mutex::default(),
        }),
    );
    let (out, usage) = t.translate(req).await.unwrap();
    assert!(
        out.iter()
            .all(|t| !glot_core::validate::has_errors(&t.issues))
    );
    assert!(
        out.iter()
            .any(|t| t.model == "claude-sonnet-5-5" && t.attempts == 4)
    );
    let (lo, hi, real) = (est.expected.cost_usd, est.max_cost_usd, usage.cost_usd);
    eprintln!("estimate ${lo:.5} <= real ${real:.5} <= max ${hi:.5}");
    assert!(
        lo <= real && real <= hi,
        "real ${real:.5} outside [${lo:.5}, ${hi:.5}]"
    );
}

#[derive(Default)]
struct Fake {
    base: String,
    submitted: Vec<Value>,
    posts: u32,
    status_calls: u32,
    direct: u32,
    cache_marked: bool,
}

type S = Arc<Mutex<Fake>>;

fn to_request(params: &Value) -> CompletionRequest {
    CompletionRequest {
        model: params["model"].as_str().unwrap_or_default().into(),
        system: params["system"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .into(),
        user: params["messages"][0]["content"]
            .as_array()
            .and_then(|c| c.last())
            .and_then(|b| b["text"].as_str())
            .unwrap_or_default()
            .into(),
        max_tokens: 1000,
        json: true,
        images: vec![],
    }
}

async fn message(params: &Value) -> Value {
    let req = to_request(params);
    let model = req.model.clone();
    let c = MockProvider.complete(req).await.unwrap();
    json!({
        "model": model, "stop_reason": "end_turn",
        "content": [{"type": "text", "text": c.text}],
        "usage": {"input_tokens": 50, "output_tokens": c.output_tokens, "cache_read_input_tokens": 200, "cache_creation_input_tokens": 0},
    })
}

async fn fake_server() -> S {
    let state: S = Arc::default();
    let app = Router::new()
        .route(
            "/v1/messages/batches",
            post(|State(s): State<S>, Json(b): Json<Value>| async move {
                let mut f = s.lock().unwrap();
                f.posts += 1;
                f.submitted = b["requests"].as_array().cloned().unwrap_or_default();
                f.cache_marked = f.submitted.iter().all(|r| {
                    r["params"]["system"][0]["cache_control"]["type"] == "ephemeral"
                });
                Json(json!({"id": "msgbatch_1", "processing_status": "in_progress", "request_counts": {"processing": f.submitted.len()}}))
            }),
        )
        .route(
            "/v1/messages/batches/{id}",
            get(|State(s): State<S>| async move {
                let mut f = s.lock().unwrap();
                f.status_calls += 1;
                // First poll fails hard: simulates an interrupted run.
                if f.status_calls == 1 {
                    return Err(StatusCode::BAD_REQUEST);
                }
                let n = f.submitted.len();
                Ok(Json(json!({"id": "msgbatch_1", "processing_status": "ended",
                    "request_counts": {"processing": 0, "succeeded": n - 1, "errored": 1},
                    "results_url": format!("{}/results", f.base)})))
            }),
        )
        .route(
            "/results",
            get(|State(s): State<S>| async move {
                let reqs = s.lock().unwrap().submitted.clone();
                let mut lines = vec![];
                for (n, r) in reqs.iter().enumerate() {
                    let result = if n == 0 {
                        json!({"type": "errored", "error": {"type": "overloaded_error"}})
                    } else {
                        json!({"type": "succeeded", "message": message(&r["params"]).await})
                    };
                    lines.push(json!({"custom_id": r["custom_id"], "result": result}).to_string());
                }
                lines.join("\n")
            }),
        )
        .route(
            "/v1/messages",
            post(|State(s): State<S>, Json(b): Json<Value>| async move {
                s.lock().unwrap().direct += 1;
                Json(message(&b).await)
            }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.lock().unwrap().base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    state
}

#[tokio::test]
async fn batch_submit_resume_and_direct_fallback() {
    let fake = fake_server().await;
    let base = fake.lock().unwrap().base.clone();
    // SAFETY: variable unique to this test, read only by this test's provider.
    unsafe { std::env::set_var("GLOT_TEST_BATCH_KEY", "test") };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("glot.toml"),
        format!("source_locale = \"en\"\ntarget_locales = [\"fr\", \"ja\"]\nfiles = [\"messages/{{locale}}.json\"]\n\n[llm]\nprovider = \"anthropic\"\nmodel = \"claude-haiku-5-5\"\napi_key_env = \"GLOT_TEST_BATCH_KEY\"\nbase_url = \"{base}\"\nbatch_size = 2\n"),
    )
    .unwrap();
    fs::write(
        root.join("glossary.toml"),
        "[[term]]\nterm = \"Workspace\"\ntranslations = { fr = \"Espace de travail\" }\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("messages")).unwrap();
    fs::write(
        root.join("messages/en.json"),
        r#"{"a":"Hello <b>{name}</b>","b":"Your Workspace","c":"{n, plural, one {# file} other {# files}}"}"#,
    )
    .unwrap();

    let mut ws = Workspace::open(root).unwrap();
    ws.batch = true;
    let dry = ws
        .translate(TranslateOptions {
            dry_run: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(dry.estimated_cost_usd > 0.0);
    assert!(dry.estimated_cost_usd < dry.estimated_cost_max_usd);
    // Interrupted: the batch id is kept for the next run.
    assert!(ws.translate(TranslateOptions::default()).await.is_err());
    assert!(root.join(BATCH_FILE).exists());

    let mut ws = Workspace::open(root).unwrap();
    ws.batch = true;
    let report = ws.translate(TranslateOptions::default()).await.unwrap();
    {
        let f = fake.lock().unwrap();
        assert_eq!(f.posts, 1, "resumed, not resubmitted");
        assert_eq!(f.submitted.len(), 4); // fr: 2 batches, ja: 2 batches
        assert!(f.cache_marked, "system prompt carries cache_control");
        assert_eq!(f.direct, 1, "the errored batch request is redone directly");
    }
    assert!(report.rejected.is_empty(), "{:#?}", report.rejected);
    assert_eq!(report.written["fr"].len() + report.written["ja"].len(), 6);
    assert!(report.usage.cache_read_tokens > 0);
    assert!(report.usage.cost_usd > 0.0);
    assert!(!root.join(BATCH_FILE).exists());
    let ws = Workspace::open(root).unwrap();
    // Glossary now only lives in the (cached) system prompt.
    assert_eq!(
        ws.catalogs["fr"].entries["b"],
        "⟦fr⟧ Your Espace de travail"
    );
}
