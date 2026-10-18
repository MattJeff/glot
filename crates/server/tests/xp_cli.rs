//! `glot experiments` (CLI binary) and the remote MCP experiment tools against a live local server
//! (real Postgres, mock LLM; skipped when Postgres is unreachable). Needs the CLI built in the same target dir:
//! `cargo build -p glot-cli` before `cargo test -p glot-server --test xp_cli`.
//! Flow: glot.experiments.toml → validate → push (variants translated, draft) → start → (winner) → promote
//! everywhere + cross-language test → pull (definitions only) → pause/resume/results; then the same through MCP.
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
};
use base64::Engine;
use http_body_util::BodyExt;

const KEY: &str = include_str!("fixtures/github-app-test-key.pem");
type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

/// Fake GitHub: the linked repo `acme/app@main` holds glot.toml and messages/de.json (no glot.lock yet).
async fn fake_github(State(log): State<Log>, req: Request<Body>) -> axum::response::Response {
    let (m, p) = (req.method().to_string(), req.uri().path().to_string());
    let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
        .unwrap_or(Value::Null);
    log.lock().unwrap().push((m.clone(), p.clone(), body));
    let r = |v: Value| axum::Json(v).into_response();
    let file =
        |t: &str| r(json!({ "content": base64::engine::general_purpose::STANDARD.encode(t) }));
    match (m.as_str(), p.as_str()) {
        ("POST", "/app/installations/7/access_tokens") => r(json!({ "token": "ghs_test" })),
        ("GET", "/repos/acme/app/contents/glot.toml") => file(
            "source_locale = \"en\"\ntarget_locales = [\"de\", \"fr\"]\nfiles = [\"messages/{locale}.json\"]\n",
        ),
        ("GET", "/repos/acme/app/contents/messages/de.json") => file(
            r#"{"home":{"hero":{"title":"Liefern Sie in {n} Sprachen","cta":"Kostenlos starten"}},"nav":{"home":"Start"}}"#,
        ),
        ("GET", "/repos/acme/app/git/ref/heads/main") => r(json!({ "object": { "sha": "base" } })),
        ("GET", "/repos/acme/app/git/commits/base") => r(json!({ "tree": { "sha": "t0" } })),
        ("POST", "/repos/acme/app/git/trees") => r(json!({ "sha": "t1" })),
        ("POST", "/repos/acme/app/git/commits") => r(json!({ "sha": "c1" })),
        ("POST", "/repos/acme/app/git/refs") => r(json!({})),
        ("POST", "/repos/acme/app/pulls") => {
            r(json!({ "number": 9, "html_url": "https://github.com/acme/app/pull/9" }))
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The tree and the PR of the last promotion pull request.
fn last_pr(log: &Log) -> (Value, Value) {
    let log = log.lock().unwrap();
    let find = |p: &str| {
        log.iter()
            .rev()
            .find(|(m, x, _)| m == "POST" && x.ends_with(p))
            .map(|e| e.2.clone())
            .unwrap_or(Value::Null)
    };
    (find("/git/trees"), find("/pulls"))
}

fn glot_bin() -> std::path::PathBuf {
    // target/<dir>/debug/deps/xp_cli-… → target/<dir>/debug/glot
    let bin = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("glot");
    assert!(
        bin.exists(),
        "build the CLI first: cargo build -p glot-cli ({} missing)",
        bin.display()
    );
    bin
}

fn glot(dir: &Path, token: &str, args: &[&str]) -> (bool, Value, String) {
    let out = Command::new(glot_bin())
        .args(args)
        .current_dir(dir)
        .env("GLOT_TOKEN", token)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let text = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
    (
        out.status.success(),
        serde_json::from_str(&stdout).unwrap_or(Value::Null),
        text,
    )
}

async fn mcp_call(
    c: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> (bool, Value) {
    let mut p = CallToolRequestParams::new(name);
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    let r = c.call_tool(p).await.expect("call_tool");
    let text = r
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default();
    (
        r.is_error != Some(true),
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

// multi_thread: the CLI runs as a blocking child process while this runtime serves its requests.
#[tokio::test(flavor = "multi_thread")]
async fn experiments_cli_and_mcp() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let log: Log = Default::default();
    let gh = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gh_url = format!("http://{}", gh.local_addr().unwrap());
    let fake = Router::new().fallback(fake_github).with_state(log.clone());
    tokio::spawn(async move { axum::serve(gh, fake).await.unwrap() });
    // SAFETY: single test in this binary, set before the server reads it.
    unsafe { std::env::set_var("GLOT_GITHUB_API", &gh_url) };
    let db = format!("glot_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = glot_server::router(state.clone(), "http://localhost:3321");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http = reqwest::Client::new();
    let res = http
        .post(format!("{base}/api/auth/signup"))
        .json(&json!({"email": "x@p.co", "password": "password123", "name": "Xavier"}))
        .send()
        .await
        .unwrap();
    let cookie = res.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let api = |m: reqwest::Method, path: &str, body: Value| {
        http.request(m, format!("{base}{path}"))
            .header("cookie", &cookie)
            .json(&body)
            .send()
    };
    let r = api(reqwest::Method::POST, "/api/projects", json!({"name": "Acme", "slug": "acme", "source_locale": "en", "target_locales": ["de", "fr"]})).await.unwrap();
    assert_eq!(r.status(), 201);
    api(
        reqwest::Method::PATCH,
        "/api/projects/acme",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
    )
    .await
    .unwrap();
    for (k, en, de) in [
        (
            "home.hero.title",
            "Ship in {n} languages",
            "Liefern Sie in {n} Sprachen",
        ),
        ("home.hero.cta", "Start free", "Kostenlos starten"),
    ] {
        let r = api(
            reqwest::Method::POST,
            "/api/projects/acme/keys",
            json!({"key": k, "source": en}),
        )
        .await
        .unwrap();
        assert!(r.status().is_success());
        let r = api(
            reqwest::Method::PUT,
            &format!("/api/projects/acme/keys/{k}/translations/de"),
            json!({"value": de}),
        )
        .await
        .unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    }
    // The project is linked to acme/app through the GitHub App.
    let enc = |s: &str| glot_server::auth::encrypt(&state.secret, s);
    sqlx::query("INSERT INTO github_app (app_id, slug, client_id, client_secret_enc, private_key_enc, webhook_secret_enc, html_url) VALUES (42, 'glot-test', 'Iv1.test', $1, $2, $3, 'https://github.com/apps/glot-test')")
        .bind(enc("cs")).bind(enc(KEY)).bind(enc("whsec"))
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO github_links (project_id, installation_id, repo, branch) SELECT id, 7, 'acme/app', 'main' FROM projects WHERE slug = 'acme'")
        .execute(&pool).await.unwrap();
    let token = api(reqwest::Method::POST, "/api/tokens", json!({"name": "cli"}))
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // The repo: catalogs + definitions in Git.
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir_all(d.join("messages")).unwrap();
    std::fs::write(
        d.join("glot.toml"),
        format!("source_locale = \"en\"\ntarget_locales = [\"de\", \"fr\"]\nfiles = [\"messages/{{locale}}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n[server]\nurl = \"{base}\"\nproject = \"acme\"\n"),
    )
    .unwrap();
    std::fs::write(
        d.join("messages/en.json"),
        r#"{"home":{"hero":{"title":"Ship in {n} languages","cta":"Start free"}}}"#,
    )
    .unwrap();
    std::fs::write(
        d.join("messages/de.json"),
        r#"{"home":{"hero":{"title":"Liefern Sie in {n} Sprachen","cta":"Kostenlos starten"}}}"#,
    )
    .unwrap();
    std::fs::write(d.join("messages/fr.json"), "{}").unwrap();
    let defs = r#"
[segments.de-google]
all = [{ attr = "country", in = ["DE"] }, { attr = "source", eq = "search" }]

[[experiments]]
name = "hero-de"
prefix = "home.hero"
segments = ["de-google"]
locales = ["en", "de"]
facts = ["2,000 teams"]

[[experiments.variants]]
intent = "social proof"
source = { "home.hero.title" = "Join 2,000 teams shipping in {n} languages" }
"#;
    std::fs::write(d.join("glot.experiments.toml"), defs).unwrap();

    // CI: invalid definitions fail `validate` and `glot check`, valid ones pass.
    std::fs::write(
        d.join("glot.experiments.toml"),
        defs.replace("2,000 teams shipping", "3,000 teams shipping"),
    )
    .unwrap();
    let (ok, _, out) = glot(d, &token, &["experiments", "validate"]);
    assert!(!ok && out.contains("unsourced_number"), "{out}");
    let (ok, _, out) = glot(d, &token, &["check"]);
    assert!(!ok && out.contains("experiment:hero-de"), "{out}");
    std::fs::write(d.join("glot.experiments.toml"), defs).unwrap();
    let (ok, _, out) = glot(d, &token, &["experiments", "validate"]);
    assert!(ok, "{out}");

    // push: segment + draft experiment with every (key, locale) filled.
    let (ok, v, out) = glot(d, &token, &["experiments", "push", "--json"]);
    assert!(ok, "{out}");
    assert_eq!(v["pushed"][1]["action"], "created", "{v}");
    let (ok, v, out) = glot(d, &token, &["experiments", "status", "hero-de", "--json"]);
    assert!(ok, "{out}");
    assert_eq!(v["experiment"]["status"], "draft");
    assert_eq!(v["readiness"]["ready"], true, "{v}");
    let arm = &v["experiment"]["arms"][1];
    assert_eq!(
        arm["values"]["en"]["home.hero.title"],
        "Join 2,000 teams shipping in {n} languages"
    );
    assert_eq!(
        arm["values"]["de"]["home.hero.cta"], "Kostenlos starten",
        "untouched key keeps the official text"
    );
    assert!(
        arm["values"]["de"]["home.hero.title"]
            .as_str()
            .unwrap()
            .contains("{n}")
    );
    // Push again: unchanged (no new arm, still a draft).
    let (ok, v, _) = glot(d, &token, &["experiments", "push", "--json"]);
    assert!(ok && v["pushed"][1]["action"] == "updated", "{v}");

    // resume on a draft starts it.
    let (ok, v, out) = glot(d, &token, &["experiments", "resume", "hero-de", "--json"]);
    assert!(ok, "{out}");
    assert_eq!(v["status"], "running");
    let id = v["id"].as_str().unwrap().to_string();
    let seg = v["cells"][0]["id"].as_str().unwrap().to_string();
    // Definitions are frozen once started.
    let (ok, v, _) = glot(d, &token, &["experiments", "push", "--json"]);
    assert!(ok && v["pushed"][1]["action"] == "frozen", "{v}");

    // Promotion needs a declared winner.
    let (ok, _, out) = glot(d, &token, &["experiments", "promote", "hero-de", "--yes"]);
    assert!(!ok && out.contains("no declared winner"), "{out}");
    // The stats engine (XP2) declares arm 1 the winner in the DACH-search cell.
    sqlx::query("UPDATE xp_cells SET winner = 1 WHERE experiment_id = $1 AND cell = $2")
        .bind(uuid::Uuid::parse_str(&id).unwrap())
        .bind(&seg)
        .execute(&pool)
        .await
        .unwrap();
    let arm_r = |p: f64, lift: f64, lo: f64, hi: f64| json!({"mean": 0.05, "p_best": p, "p_beats_control": p, "lift": lift, "lift_lo": lo, "lift_hi": hi, "weight": 0.5});
    let analysis = json!({"arm_ids": [0, 1], "look": 1, "decision_look": true, "threshold": 0.99, "paused": false,
        "global": {"name": "global", "views": 4200, "arms": [arm_r(0.02, 0.0, 0.0, 0.0), arm_r(0.98, 0.12, 0.04, 0.2)], "winner": 1},
        "cells": [], "segments": []});
    sqlx::query("INSERT INTO xp_analyses (experiment_id, look, decision, paused, analysis) VALUES ($1, 1, true, false, $2)")
        .bind(uuid::Uuid::parse_str(&id).unwrap())
        .bind(&analysis)
        .execute(&pool)
        .await
        .unwrap();
    let (ok, v, _) = glot(d, &token, &["experiments", "status", "hero-de", "--json"]);
    assert!(ok);
    assert_eq!(v["proposals"][0]["arm"], 1, "{v}");
    assert_eq!(
        v["proposals"][0]["locales"],
        json!(["fr"]),
        "fr was not tested"
    );

    // Promote everywhere + launch the cross-language test in one command.
    let (ok, v, out) = glot(
        d,
        &token,
        &[
            "experiments",
            "promote",
            "hero-de",
            "--propagate",
            "--yes",
            "--json",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(v["experiment"]["status"], "promoted");
    assert_eq!(v["propagated"]["started"], true, "{v}");
    let px = &v["propagated"]["experiment"];
    assert_eq!(px["locales"], json!(["fr"]));
    assert_eq!(px["status"], "running");
    assert!(
        px["arms"][1]["values"]["fr"]["home.hero.title"]
            .as_str()
            .unwrap()
            .contains("{n}")
    );
    // Official de translation = the winner (history + lockfile on the next `glot pull`).
    let keys: Value = api(
        reqwest::Method::GET,
        "/api/projects/acme/keys?q=home.hero.title",
        Value::Null,
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(
        keys.to_string().contains(
            &*arm["values"]["de"]["home.hero.title"]
                .as_str()
                .unwrap()
                .replace('"', "\\\"")
        ),
        "{keys}"
    );

    // Git = source of truth: a PR rewrites the de catalog (other keys kept) and records the winner in glot.lock.
    let (tree, pr) = last_pr(&log);
    let winner_de = arm["values"]["de"]["home.hero.title"]
        .as_str()
        .unwrap()
        .to_string();
    let tree_s = tree.to_string();
    assert!(
        tree_s.contains("messages/de.json")
            && tree_s.contains("glot.lock")
            && tree_s.contains("experiment winner · hero-de"),
        "{tree}"
    );
    let de_file = tree["tree"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "messages/de.json")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    let de_json: Value = serde_json::from_str(&de_file).unwrap();
    assert_eq!(de_json["home"]["hero"]["title"], winner_de.as_str());
    assert_eq!(de_json["nav"]["home"], "Start", "untouched keys stay");
    assert_eq!(pr["base"], "main");
    let body = pr["body"].as_str().unwrap();
    for part in [
        "hero-de",
        "everywhere",
        "lift +12.0 % (95 % interval +4.0 % to +20.0 %)",
        "P(best) 98.0 %",
        "4200 visitors",
        "Duration",
        "/app/acme/experiments?xp=",
    ] {
        assert!(
            body.contains(part),
            "{part} missing from the PR body: {body}"
        );
    }

    // pull: definitions only (no state, weights or counts).
    let (ok, v, out) = glot(d, &token, &["experiments", "pull", "--json"]);
    assert!(ok, "{out}");
    assert_eq!(v["experiments"].as_array().unwrap().len(), 2);
    let toml = std::fs::read_to_string(d.join("glot.experiments.toml")).unwrap();
    assert!(
        toml.contains("de-google")
            && toml.contains("social proof")
            && !toml.contains("weights")
            && !toml.contains("winner"),
        "{toml}"
    );
    let (ok, _, out) = glot(d, &token, &["experiments", "validate"]);
    assert!(ok, "pulled definitions validate: {out}");

    // list / pause / resume / results on the cross-language test.
    let pname = px["name"].as_str().unwrap().to_string();
    let (ok, v, _) = glot(d, &token, &["experiments", "list", "--json"]);
    assert!(ok && v.as_array().unwrap().len() == 2);
    let (ok, v, _) = glot(d, &token, &["experiments", "pause", &pname, "--json"]);
    assert!(ok && v["status"] == "paused", "{v}");
    let (ok, v, _) = glot(d, &token, &["experiments", "resume", &pname, "--json"]);
    assert!(ok && v["status"] == "running", "{v}");
    let (ok, v, out) = glot(d, &token, &["experiments", "results", &pname, "--json"]);
    assert!(ok && v.get("latest").is_some(), "{out}");

    // ---------- remote MCP: same flows, confirmation required ----------
    let t = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp?project=acme"))
            .auth_header(token.clone()),
    );
    let c = ().serve(t).await.expect("initialize");
    let names: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for n in [
        "experiment_create",
        "experiment_status",
        "experiment_promote",
    ] {
        assert!(names.contains(&n.to_string()), "{names:?}");
    }
    // "Crée une expérience sur le hero pour le trafic Google en Allemagne": no traffic recorded yet.
    let ask = json!({"prefix": "home.hero", "segment": {"country": ["DE"], "source": "search"}, "intents": ["direct_benefit"]});
    let (ok, v) = mcp_call(&c, "experiment_create", ask.clone()).await;
    assert!(
        ok && v["created"] == false && v["reason"].as_str().unwrap().contains("traffic"),
        "{v}"
    );
    let mut ask = ask;
    ask["allow_low_traffic"] = json!(true);
    // The fr test still owns the keys: create a draft only.
    ask["start"] = json!(false);
    let (ok, v) = mcp_call(&c, "experiment_create", ask.clone()).await;
    assert!(ok && v["confirmation_required"] == true, "{v}");
    let (ok, list) = mcp_call(&c, "experiment_status", json!({})).await;
    assert!(
        ok && list["experiments"].as_array().unwrap().len() == 2,
        "nothing created before confirmation: {list}"
    );
    ask["confirm"] = json!(true);
    let (ok, v) = mcp_call(&c, "experiment_create", ask).await;
    assert!(ok, "{v}");
    assert_eq!(v["experiment"]["status"], "draft", "{v}");
    assert_eq!(
        v["experiment"]["segments"],
        json!(["de-google"]),
        "the existing segment with the same rule is reused: {v}"
    );
    assert!(
        v["experiment"]["arms"].as_array().unwrap().len() >= 2,
        "{v}"
    );
    let (ok, v) = mcp_call(&c, "experiment_status", json!({"experiment": "hero-de"})).await;
    assert!(ok && v["experiment"]["status"] == "promoted", "{v}");
    // Promote guard: the fr test has no winner yet.
    let (ok, v) = mcp_call(
        &c,
        "experiment_promote",
        json!({"experiment": pname, "scope": "segment"}),
    )
    .await;
    assert!(!ok && v.to_string().contains("no declared winner"), "{v}");
    sqlx::query("UPDATE xp_cells SET winner = 1 WHERE experiment_id = (SELECT id FROM xp_experiments WHERE name = $1) AND cell = '*'")
        .bind(&pname)
        .execute(&pool)
        .await
        .unwrap();
    let (ok, v) = mcp_call(
        &c,
        "experiment_promote",
        json!({"experiment": pname, "scope": "segment"}),
    )
    .await;
    assert!(
        ok && v["confirmation_required"] == true
            && v["question"].as_str().unwrap().contains("permanently"),
        "{v}"
    );
    let (ok, v) = mcp_call(
        &c,
        "experiment_promote",
        json!({"experiment": pname, "scope": "segment", "confirm": true}),
    )
    .await;
    assert!(ok && v["experiment"]["status"] == "personalized", "{v}");

    // Segment promotion: recorded in the repo too (.glot/personalizations.json), and by `glot pull`.
    let (tree, pr) = last_pr(&log);
    assert!(
        tree.to_string().contains(".glot/personalizations.json")
            && pr["title"].as_str().unwrap().contains("personalize"),
        "{tree} {pr}"
    );
    let (ok, _, out) = glot(d, &token, &["pull"]);
    assert!(ok, "{out}");
    let pers = std::fs::read_to_string(d.join(".glot/personalizations.json")).unwrap();
    assert!(
        pers.contains(&pname) && pers.contains("\"arm\": 1"),
        "{pers}"
    );
    let de = std::fs::read_to_string(d.join("messages/de.json")).unwrap();
    assert!(
        de.contains(&winner_de),
        "glot pull brings the promoted text: {de}"
    );
    let lock = std::fs::read_to_string(d.join("glot.lock")).unwrap();
    assert!(lock.contains("experiment winner"), "{lock}");

    drop(c);
    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
