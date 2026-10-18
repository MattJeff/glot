//! Live review outside the SaaS, end to end: the real `glot` binary (`glot annotations …`, `glot check`, `glot mcp`
//! over stdio) against an in-process server on 127.0.0.1:8831, the remote MCP tools, and the GitHub mirror (fake
//! GitHub: issue / PR comment, merged PR resolves the annotation). Skipped when Postgres is unreachable.
//! Needs the CLI built in the same target dir: `cargo build -p glot-cli`.
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};

const KEY: &str = include_str!("fixtures/github-app-test-key.pem");
const BASE: &str = "http://127.0.0.1:8831";

type Log = Arc<Mutex<Vec<(String, String, Value)>>>;

async fn fake_github(State(log): State<Log>, req: Request<Body>) -> axum::response::Response {
    let (m, p) = (req.method().to_string(), req.uri().path().to_string());
    let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
        .unwrap_or(Value::Null);
    log.lock().unwrap().push((m.clone(), p.clone(), body));
    let r = |v: Value| axum::Json(v).into_response();
    match (m.as_str(), p.as_str()) {
        ("POST", "/app/installations/7/access_tokens") => r(json!({ "token": "ghs_test" })),
        ("POST", "/repos/acme/app/issues") => {
            r(json!({ "number": 12, "html_url": "https://github.com/acme/app/issues/12" }))
        }
        ("POST", "/repos/acme/app/issues/5/comments") => {
            r(json!({ "html_url": "https://github.com/acme/app/pull/5#issuecomment-1" }))
        }
        _ => StatusCode::IM_A_TEAPOT.into_response(),
    }
}

fn glot_bin() -> PathBuf {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("glot");
    assert!(p.exists(), "build the CLI first: cargo build -p glot-cli");
    p
}

/// Runs `glot args…` in `dir` off the async runtime; returns (success, stdout + stderr).
async fn glot(dir: &Path, token: &str, args: &[&str]) -> (bool, String) {
    let (dir, token) = (dir.to_path_buf(), token.to_string());
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let o = Command::new(glot_bin())
            .args(&args)
            .current_dir(&dir)
            .env("GLOT_TOKEN", &token)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )
    })
    .await
    .unwrap()
}

async fn glot_json(dir: &Path, token: &str, args: &[&str]) -> Value {
    let (ok, out) = glot(dir, token, args).await;
    assert!(ok, "glot {args:?}: {out}");
    serde_json::from_str(&out).unwrap_or_else(|_| panic!("not JSON: {out}"))
}

/// `glot mcp` over stdio: initialize, then each tools/call; returns the text of each result.
async fn local_mcp(
    dir: &Path,
    token: &str,
    calls: Vec<(&'static str, Value)>,
) -> Vec<(bool, String)> {
    let (dir, token) = (dir.to_path_buf(), token.to_string());
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new(glot_bin())
            .arg("mcp")
            .current_dir(&dir)
            .env("GLOT_TOKEN", &token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut send = |v: Value| writeln!(stdin, "{v}").unwrap();
        let mut recv = |id: i64| loop {
            let mut l = String::new();
            assert!(out.read_line(&mut l).unwrap() > 0, "glot mcp closed stdout");
            let v: Value = serde_json::from_str(&l).unwrap();
            if v["id"] == id {
                return v;
            }
        };
        send(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}));
        recv(0);
        send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        let mut res = vec![];
        for (i, (name, args)) in calls.into_iter().enumerate() {
            let id = i as i64 + 1;
            send(json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}}));
            let v = recv(id);
            let r = &v["result"];
            res.push((r["isError"] != true, r["content"][0]["text"].as_str().unwrap_or_default().to_string()));
        }
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        res
    })
    .await
    .unwrap()
}

async fn remote_call(
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

fn sign(body: &str) -> String {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"whsec").unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test(flavor = "multi_thread")]
async fn annotations_outside_the_saas() {
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
    // SAFETY: single test in this binary, set before the server reads them.
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8831")
        .await
        .expect("port 8831 free");
    let app = glot_server::router(state.clone(), "http://localhost:3320");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // Member, project en -> de, API token.
    let http = reqwest::Client::new();
    let res = http
        .post(format!("{BASE}/api/auth/signup"))
        .json(&json!({"email": "a@b.co", "password": "password123", "name": "Ada"}))
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
    let r = http
        .post(format!("{BASE}/api/projects"))
        .header("cookie", &cookie)
        .json(&json!({"name": "Demo", "slug": "demo", "source_locale": "en", "target_locales": ["de"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let token = http
        .post(format!("{BASE}/api/tokens"))
        .header("cookie", &cookie)
        .json(&json!({"name": "cli"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let rest = |m: reqwest::Method, path: &str| {
        http.request(m, format!("{BASE}/api/projects/demo/review{path}"))
            .bearer_auth(&token)
    };

    // The developer's repo: catalogs, a component using the key, [server].
    let dir = std::env::temp_dir().join(format!(
        "glot-annotations-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let w = dir.as_path();
    std::fs::create_dir_all(w.join("messages")).unwrap();
    std::fs::create_dir_all(w.join("app")).unwrap();
    std::fs::write(
        w.join("glot.toml"),
        format!("source_locale = \"en\"\ntarget_locales = [\"de\"]\nfiles = [\"messages/{{locale}}.json\"]\n\n[llm]\nprovider = \"mock\"\n\n[server]\nurl = \"{BASE}\"\nproject = \"demo\"\n"),
    )
    .unwrap();
    std::fs::write(
        w.join("messages/en.json"),
        r#"{"home": {"cta": "Get started", "title": "Welcome"}}"#,
    )
    .unwrap();
    std::fs::write(
        w.join("messages/de.json"),
        r#"{"home": {"cta": "Beginnen Sie", "title": "Willkommen"}}"#,
    )
    .unwrap();
    std::fs::write(
        w.join("app/page.tsx"),
        "import { useTranslations } from 'next-intl';\nexport default function Home() {\n  const t = useTranslations();\n  return <button>{t('home.cta')}</button>;\n}\n",
    )
    .unwrap();
    let (ok, out) = glot(w, &token, &["push"]).await;
    assert!(ok, "push: {out}");

    // Reviewers annotated the site (REST, as the widget would).
    let anchor = |key: &str, text: &str| {
        json!({"key": key, "selector": "main button", "text": text,
        "url": "http://localhost:3000/de?utm=x", "locale": "de", "viewport": {"w": 1280, "h": 800}})
    };
    let a1: Value = rest(reqwest::Method::POST, "/annotations")
        .json(&json!({"anchor": anchor("home.cta", "Beginnen Sie"), "body": "Too formal", "suggestion": "Los geht's"}))
        .send().await.unwrap().json().await.unwrap();
    let a1_id = a1["id"].as_str().expect("annotation created").to_string();
    let a2: Value = rest(reqwest::Method::POST, "/annotations")
        .json(&json!({"anchor": anchor("home.title", "Willkommen"), "body": "Say hello instead"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let a2_id = a2["id"].as_str().unwrap().to_string();

    // ---- CLI ----
    let v = glot_json(w, &token, &["annotations", "list", "--json"]).await;
    assert_eq!(v["total"], 2, "{v}");
    let cta = v["annotations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == a1_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(cta["key"], "home.cta");
    assert_eq!(cta["route"], "/");
    assert_eq!(cta["suggestion"], "Los geht's");
    assert_eq!(cta["local_value"], "Beginnen Sie");
    assert_eq!(cta["catalog_file"], "messages/de.json");
    assert!(
        cta["used_in"].to_string().contains("app/page.tsx:4"),
        "{cta}"
    );
    let v = glot_json(
        w,
        &token,
        &["annotations", "list", "--route", "/pricing", "--json"],
    )
    .await;
    assert_eq!(v["total"], 0);

    let short1 = &a1_id[..8];
    let (ok, out) = glot(w, &token, &["annotations", "reply", short1, "On it"]).await;
    assert!(ok, "{out}");
    let (ok, out) = glot(w, &token, &["annotations", "show", short1]).await;
    assert!(
        ok && out.contains("Too formal") && out.contains("On it") && out.contains("app/page.tsx:4"),
        "{out}"
    );

    // apply: local catalog + server, annotation resolved.
    let (ok, out) = glot(w, &token, &["annotations", "apply", short1]).await;
    assert!(ok && out.contains("local catalog + server"), "{out}");
    let de: Value =
        serde_json::from_str(&std::fs::read_to_string(w.join("messages/de.json")).unwrap())
            .unwrap();
    assert_eq!(de["home"]["cta"], "Los geht's");
    let a: Value = rest(reqwest::Method::GET, &format!("/annotations/{a1_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["status"], "resolved");
    assert_eq!(a["translation"]["value"], "Los geht's");

    // glot check: home.title changed locally while its annotation is open -> warning, not an error.
    std::fs::write(
        w.join("messages/de.json"),
        r#"{"home": {"cta": "Los geht's", "title": "Hallo"}}"#,
    )
    .unwrap();
    let (ok, out) = glot(w, &token, &["check"]).await;
    assert!(ok, "warnings only: {out}");
    assert!(
        out.contains("open_annotation") && out.contains(&a2_id[..8]),
        "{out}"
    );

    // ---- local MCP (glot mcp, stdio) ----
    let r = local_mcp(
        w,
        &token,
        vec![
            ("annotations_list", json!({})),
            (
                "annotation_reply",
                json!({"id": &a2_id[..8], "message": "Changed to Hallo"}),
            ),
            (
                "annotation_resolve",
                json!({"id": a2_id, "message": "Fixed in messages/de.json"}),
            ),
            ("annotations_list", json!({})),
        ],
    )
    .await;
    assert!(r[0].0, "{}", r[0].1);
    let listed: Value = serde_json::from_str(&r[0].1).unwrap();
    assert_eq!(listed["total"], 1, "a1 is resolved: {listed}");
    assert_eq!(listed["annotations"][0]["key"], "home.title");
    assert_eq!(listed["annotations"][0]["local_value"], "Hallo");
    assert!(r[1].0 && r[2].0, "{r:?}");
    assert!(r[2].1.contains("resolved"), "{}", r[2].1);
    assert_eq!(serde_json::from_str::<Value>(&r[3].1).unwrap()["total"], 0);
    let a: Value = rest(reqwest::Method::GET, &format!("/annotations/{a2_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread: Vec<&str> = a["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert_eq!(thread, ["Changed to Hallo", "Fixed in messages/de.json"]);

    let (ok, out) = glot(
        w,
        &token,
        &[
            "annotations",
            "reopen",
            &a2_id,
            "-m",
            "Still formal on mobile",
        ],
    )
    .await;
    assert!(ok, "{out}");
    let (ok, out) = glot(w, &token, &["check"]).await;
    assert!(ok && out.contains("open_annotation"), "{out}");

    // add from the terminal, review link.
    let a3 = glot_json(
        w,
        &token,
        &[
            "annotations",
            "add",
            "Typo here",
            "--key",
            "home.title",
            "--locale",
            "de",
            "--route",
            "/pricing",
            "--json",
        ],
    )
    .await;
    assert_eq!(a3["route"], "/pricing");
    assert_eq!(a3["key"], "home.title");
    let a3_id = a3["id"].as_str().unwrap().to_string();
    let link = glot_json(
        w,
        &token,
        &[
            "annotations",
            "link",
            "--url",
            "http://localhost:3000/de",
            "--access",
            "comment",
            "--json",
        ],
    )
    .await;
    assert!(
        link["url"]
            .as_str()
            .unwrap()
            .starts_with("http://localhost:3000/de?glot_review=glr_"),
        "{link}"
    );
    let ctx: Value = http
        .get(format!("{BASE}/api/review/w/context"))
        .bearer_auth(link["token"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        ctx["access"], "comment",
        "the shared token works for the widget: {ctx}"
    );

    // ---- remote MCP ----
    let t = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("{BASE}/mcp?project=demo"))
            .auth_header(&token),
    );
    let c = ().serve(t).await.expect("initialize");
    let names: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for n in [
        "annotations_list",
        "annotation_reply",
        "annotation_resolve",
        "annotation_apply_suggestion",
        "annotation_rewrite",
    ] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    let (ok, v) = remote_call(&c, "annotations_list", json!({"route": "/pricing"})).await;
    assert!(ok, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["annotations"][0]["id"], a3_id.as_str());
    let (_, v) = remote_call(
        &c,
        "annotations_list",
        json!({"key": "home.cta", "status": "resolved"}),
    )
    .await;
    assert_eq!(
        v["annotations"][0]["used_in"],
        json!(["app/page.tsx:4"]),
        "refs from glot push: {v}"
    );
    let (ok, v) = remote_call(
        &c,
        "annotation_reply",
        json!({"id": a3_id, "message": "Looking"}),
    )
    .await;
    assert!(ok, "{v}");
    let (ok, v) = remote_call(
        &c,
        "annotation_apply_suggestion",
        json!({"id": a3_id, "value": "Hallo {oops}"}),
    )
    .await;
    assert!(!ok, "invalid placeholder refused: {v}");
    let (ok, v) = remote_call(
        &c,
        "annotation_apply_suggestion",
        json!({"id": a3_id, "value": "Hallo!"}),
    )
    .await;
    assert!(
        ok && v["status"] == "resolved" && v["value"] == "Hallo!",
        "{v}"
    );
    let (ok, v) = remote_call(
        &c,
        "annotation_resolve",
        json!({"id": a3_id, "message": "ok", "wontfix": true}),
    )
    .await;
    assert!(ok && v["status"] == "wontfix", "{v}");

    // ---- GitHub mirror + merge ----
    let (ok, out) = glot(w, &token, &["annotations", "github", &a2_id]).await;
    assert!(!ok && out.contains("connect a GitHub repository"), "{out}");
    let enc = |s: &str| glot_server::auth::encrypt(&state.secret, s);
    sqlx::query("INSERT INTO github_app (app_id, slug, client_id, client_secret_enc, private_key_enc, webhook_secret_enc, html_url) VALUES (42, 'glot-test', 'Iv1.test', $1, $2, $3, 'https://github.com/apps/glot-test')")
        .bind(enc("cs")).bind(enc(KEY)).bind(enc("whsec"))
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO github_links (project_id, installation_id, repo, branch, pr_number, pr_url) SELECT id, 7, 'acme/app', 'main', 5, 'https://github.com/acme/app/pull/5' FROM projects WHERE slug = 'demo'")
        .execute(&pool).await.unwrap();
    let (ok, out) = glot(w, &token, &["annotations", "github", &a2_id]).await;
    assert!(
        ok && out.contains("https://github.com/acme/app/issues/12"),
        "{out}"
    );
    let a4: Value = rest(reqwest::Method::POST, "/annotations")
        .json(&json!({"anchor": anchor("home.cta", "Los geht's"), "body": "Exclamation mark?"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let a4_id = a4["id"].as_str().unwrap().to_string();
    let (ok, out) = glot(w, &token, &["annotations", "github", &a4_id, "--pr"]).await;
    assert!(ok && out.contains("pull/5#issuecomment-1"), "{out}");
    {
        let log = log.lock().unwrap();
        let issue = log
            .iter()
            .find(|(m, p, _)| m == "POST" && p == "/repos/acme/app/issues")
            .unwrap();
        assert!(
            issue.2["title"]
                .as_str()
                .unwrap()
                .contains("home.title (de, /)"),
            "{:?}",
            issue.2
        );
        assert!(
            issue.2["body"]
                .as_str()
                .unwrap()
                .contains("Say hello instead")
        );
        let tok = log.iter().find(|(_, p, b)| {
            p.ends_with("/access_tokens") && b["permissions"]["issues"] == "write"
        });
        assert!(tok.is_some(), "issue token narrowed to issues: write");
    }
    // Idempotent: the same issue comes back.
    let (ok, out) = glot(w, &token, &["annotations", "github", &a2_id]).await;
    assert!(ok && out.contains("issues/12"), "{out}");

    let webhook = |body: Value| {
        let s = body.to_string();
        http.post(format!("{BASE}/api/github/webhook"))
            .header("x-github-event", "pull_request")
            .header("x-hub-signature-256", sign(&s))
            .header("content-type", "application/json")
            .body(s)
            .send()
    };
    let pr = |n: i64, merged: bool, body: &str| {
        json!({"action": "closed", "number": n, "installation": {"id": 7},
        "repository": {"full_name": "acme/app"},
        "pull_request": {"merged": merged, "title": "Copy fixes", "body": body, "html_url": format!("https://github.com/acme/app/pull/{n}")}})
    };
    // Closed without merge: nothing happens.
    assert!(
        webhook(pr(13, false, "Fixes #12"))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let a: Value = rest(reqwest::Method::GET, &format!("/annotations/{a2_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["status"], "open");
    assert!(
        webhook(pr(13, true, "Shorter title.\n\nFixes #12"))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let a: Value = rest(reqwest::Method::GET, &format!("/annotations/{a2_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["status"], "resolved");
    assert!(a["comments"].to_string().contains("Fixed by [#13]"), "{a}");
    // The glot PR (where a4 was mirrored as a comment) is merged.
    assert!(
        webhook(pr(5, true, ""))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let a: Value = rest(reqwest::Method::GET, &format!("/annotations/{a4_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["status"], "resolved");
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM review_events WHERE annotation_id = $1 ORDER BY id")
            .bind(uuid::Uuid::parse_str(&a4_id).unwrap())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        kinds.contains(&"github.mirrored".to_string())
            && kinds.last().unwrap() == "annotation.updated",
        "{kinds:?}"
    );

    // Errors are readable.
    let (ok, out) = glot(w, &token, &["annotations", "show", "ffffffff"]).await;
    assert!(!ok && out.contains("no annotation id starts with"), "{out}");
    let (ok, out) = glot(w, "glot_wrong", &["annotations", "list"]).await;
    assert!(!ok && out.contains("401"), "{out}");

    let _ = std::fs::remove_dir_all(&dir);
    drop(pool);
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
