//! Remote MCP end to end over real HTTP: rmcp client with an API token, then the full OAuth flow
//! (DCR → authorize with a dashboard session → token → MCP call → refresh → revoke), scope and auth refusals,
//! audit log, translate_repo against a fake GitHub. Skipped when Postgres is unreachable.
use axum::{Router, body::Body, extract::Request, http::StatusCode, response::IntoResponse};
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};

async fn fake_github(req: Request<Body>) -> axum::response::Response {
    let r = |v: Value| axum::Json(v).into_response();
    match (
        req.method().as_str(),
        req.uri().path().trim_start_matches("/repos/acme/app"),
    ) {
        ("GET", "/git/ref/heads/main") => r(json!({"object": {"sha": "base1"}})),
        ("GET", "/git/commits/base1") => r(json!({"tree": {"sha": "tree1"}})),
        ("GET", p) if p.starts_with("/contents/") => StatusCode::NOT_FOUND.into_response(),
        ("POST", "/git/trees") => r(json!({"sha": "tree2"})),
        ("POST", "/git/commits") => r(json!({"sha": "commit2"})),
        ("POST", "/git/refs") => r(json!({"ref": "x"})),
        ("POST", "/pulls") => r(json!({"html_url": "https://github.com/acme/app/pull/9"})),
        _ => StatusCode::IM_A_TEAPOT.into_response(),
    }
}

async fn mcp_client(
    url: String,
    token: &str,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let t = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
    );
    ().serve(t).await.expect("initialize")
}

async fn call(
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

fn tool_names(tools: &[rmcp::model::Tool]) -> Vec<String> {
    let mut v: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn remote_mcp_end_to_end() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let gh = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gh_url = format!("http://{}", gh.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(gh, Router::new().fallback(fake_github))
            .await
            .unwrap()
    });
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
    glot_server::jobs::spawn_worker(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = glot_server::router(state.clone(), "http://localhost:3320");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // Dashboard user, project with the mock LLM, API token.
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let res = http
        .post(format!("{base}/api/auth/signup"))
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
    let api = |m: reqwest::Method, path: &str| {
        http.request(m, format!("{base}{path}"))
            .header("cookie", &cookie)
    };
    let r = api(reqwest::Method::POST, "/api/projects")
        .json(&json!({"name": "Demo", "slug": "demo", "source_locale": "en", "target_locales": ["fr"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let r = api(reqwest::Method::PATCH, "/api/projects/demo")
        .json(&json!({"llm": {"provider": "mock", "model": "mock"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let api_token = api(reqwest::Method::POST, "/api/tokens")
        .json(&json!({"name": "mcp"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // ---- refusals ----
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
    let post = |tok: Option<&str>, body: &Value| {
        let mut r = http
            .post(format!("{base}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .json(body);
        if let Some(t) = tok {
            r = r.bearer_auth(t);
        }
        r.send()
    };
    let r = post(None, &init).await.unwrap();
    assert_eq!(r.status(), 401);
    let www = r.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        www.contains(&format!(
            "resource_metadata=\"{base}/.well-known/oauth-protected-resource\""
        )),
        "{www}"
    );
    assert_eq!(
        post(Some("glot_at_nope"), &init).await.unwrap().status(),
        401
    );
    assert_eq!(
        post(Some(&api_token), &init).await.unwrap().status(),
        400,
        "API token without project"
    );
    let evil = http
        .post(format!("{base}/mcp?project=demo"))
        .bearer_auth(&api_token)
        .header("origin", "https://evil.example")
        .header("accept", "application/json, text/event-stream")
        .json(&init)
        .send()
        .await
        .unwrap();
    assert_eq!(evil.status(), 403, "foreign Origin rejected");

    // ---- API token: full toolset ----
    let c = mcp_client(format!("{base}/mcp?project=demo"), &api_token).await;
    let tools = tool_names(&c.list_all_tools().await.unwrap());
    assert_eq!(tools.len(), 19, "{tools:?}"); // 11 + 5 annotations_* + 3 experiment_*
    let (ok, v) = call(
        &c,
        "propose_string",
        json!({"text": "Save changes", "suggested_key": "Settings.Save"}),
    )
    .await;
    assert!(ok, "{v}");
    assert_eq!(v["key"], "settings.save");
    let (ok, v) = call(
        &c,
        "add_string",
        json!({"key": "settings.save", "text": "Save {count} changes", "note": "button"}),
    )
    .await;
    assert!(ok && v["action"] == "created", "{v}");
    let (ok, v) = call(
        &c,
        "add_string",
        json!({"key": "settings.save", "text": "Other"}),
    )
    .await;
    assert!(!ok, "no silent overwrite: {v}");
    let (ok, v) = call(&c, "add_string", json!({"key": "bad", "text": "Hi {name"})).await;
    assert!(!ok, "ICU syntax checked: {v}");
    let (ok, v) = call(&c, "translate", json!({"wait_seconds": 30})).await;
    assert!(ok && v["job"]["status"] == "done", "{v}");
    let (ok, v) = call(
        &c,
        "get_context",
        json!({"keys": ["settings.save", "missing.key"]}),
    )
    .await;
    assert!(ok, "{v}");
    assert_eq!(
        v["keys"][0]["translations"]["fr"]["status"], "machine",
        "{v}"
    );
    assert_eq!(v["keys"][1]["in_source_catalog"], false);
    let (ok, v) = call(
        &c,
        "set_translation",
        json!({"locale": "fr", "key": "settings.save", "value": "Enregistrer"}),
    )
    .await;
    assert!(!ok, "missing placeholder refused: {v}");
    let (ok, v) = call(&c, "set_translation", json!({"locale": "fr", "key": "settings.save", "value": "Enregistrer {count} modifications"})).await;
    assert!(ok, "{v}");
    let (ok, v) = call(
        &c,
        "glossary_upsert",
        json!({"term": "Workspace", "translations": {"fr": "Espace de travail"}}),
    )
    .await;
    assert!(ok && v["written"] == true, "{v}");
    let (ok, v) = call(
        &c,
        "glossary_upsert",
        json!({"term": "workspace", "translations": {"fr": "Espace"}}),
    )
    .await;
    assert!(ok && v["written"] == false, "conflict reported: {v}");
    let (ok, v) = call(&c, "glossary_lookup", json!({"text": "My workspace"})).await;
    assert!(ok && v["terms"].as_array().unwrap().len() == 1, "{v}");
    let (ok, v) = call(&c, "check", json!({})).await;
    assert!(ok && v["total"].is_number(), "{v}");
    let (ok, v) = call(&c, "status", json!({})).await;
    assert!(ok && v["stats"]["keys"] == 1, "{v}");
    let (ok, v) = call(&c, "review", json!({"locale": "fr"})).await;
    assert!(ok, "{v}");
    let (ok, v) = call(&c, "translate_repo", json!({})).await;
    assert!(
        !ok && v.as_str().unwrap().contains("GitHub"),
        "no repo configured yet: {v}"
    );
    sqlx::query("INSERT INTO feedback_settings (project_id, github_repo, github_token_enc) SELECT id, 'acme/app', $1 FROM projects WHERE slug = 'demo'")
        .bind(glot_server::auth::encrypt(&state.secret, "ghp_test"))
        .execute(&pool)
        .await
        .unwrap();
    let (ok, v) = call(&c, "translate_repo", json!({})).await;
    assert!(
        ok && v["pr_url"] == "https://github.com/acme/app/pull/9" && v["files"] == 3,
        "{v}"
    );
    c.cancel().await.unwrap();

    // ---- OAuth 2.1: discovery → DCR → consent → token → MCP ----
    let prm: Value = http
        .get(format!("{base}/.well-known/oauth-protected-resource"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prm["resource"], format!("{base}/mcp"));
    let issuer = prm["authorization_servers"][0]
        .as_str()
        .unwrap()
        .to_string();
    let asm: Value = http
        .get(format!("{issuer}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    let reg = http.post(asm["registration_endpoint"].as_str().unwrap())
        .json(&json!({"client_name": "Lovable", "redirect_uris": ["https://lovable.dev/cb"], "token_endpoint_auth_method": "none"}))
        .send().await.unwrap();
    assert_eq!(reg.status(), 201);
    let client_id = reg.json::<Value>().await.unwrap()["client_id"]
        .as_str()
        .unwrap()
        .to_string();
    let bad = http
        .post(format!("{base}/oauth/register"))
        .json(&json!({"redirect_uris": ["http://evil.com/cb"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    let verifier = "a".repeat(43) + "-verifier";
    let challenge = glot_server::mcp_remote::oauth::pkce_s256(&verifier);
    let q = format!(
        "response_type=code&client_id={client_id}&redirect_uri=https%3A%2F%2Flovable.dev%2Fcb&code_challenge={challenge}&code_challenge_method=S256&state=xyz&scope=read+translate+write&resource={}",
        urlencode(&format!("{base}/mcp"))
    );
    let r = http
        .get(format!(
            "{}?{q}",
            asm["authorization_endpoint"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_redirection());
    assert_eq!(
        r.headers()["location"].to_str().unwrap(),
        format!("http://localhost:3320/oauth/authorize?{q}")
    );
    let bad = http
        .get(format!(
            "{base}/oauth/authorize?{}",
            q.replace("lovable.dev", "evil.com")
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad.status(),
        400,
        "unregistered redirect_uri is shown, not followed"
    );

    let info: Value = api(reqwest::Method::GET, &format!("/api/oauth/authorize?{q}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["client_name"], "Lovable");
    let params: Value = reqwest::Url::parse(&format!("http://x/?{q}"))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let mut consent = params.clone();
    consent["approve"] = json!(true);
    consent["project"] = json!("demo");
    consent["scopes"] = json!(["read"]); // the user unticked translate + write
    let redirect = api(reqwest::Method::POST, "/api/oauth/authorize")
        .json(&consent)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["redirect"]
        .as_str()
        .unwrap()
        .to_string();
    let ru = reqwest::Url::parse(&redirect).unwrap();
    let get_q = |k: &str| {
        ru.query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
    };
    assert_eq!(get_q("state").as_deref(), Some("xyz"));
    assert_eq!(get_q("iss").as_deref(), Some(issuer.as_str()));
    let code = get_q("code").unwrap();

    let tok = |form: Vec<(&'static str, String)>| {
        http.post(format!("{base}/oauth/token")).form(&form).send()
    };
    let wrong = tok(vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.clone()),
        ("code_verifier", "b".repeat(43)),
        ("client_id", client_id.clone()),
    ])
    .await
    .unwrap();
    assert_eq!(
        wrong.status(),
        400,
        "bad PKCE verifier (and the code is now burnt)"
    );
    // A fresh consent, exchanged correctly.
    let redirect = api(reqwest::Method::POST, "/api/oauth/authorize")
        .json(&consent)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["redirect"]
        .as_str()
        .unwrap()
        .to_string();
    let code = reqwest::Url::parse(&redirect)
        .unwrap()
        .query_pairs()
        .find(|(n, _)| n == "code")
        .unwrap()
        .1
        .into_owned();
    let t: Value = tok(vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.clone()),
        ("code_verifier", verifier.clone()),
        ("client_id", client_id.clone()),
        ("redirect_uri", "https://lovable.dev/cb".into()),
        ("resource", format!("{base}/mcp")),
    ])
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(t["scope"], "read", "{t}");
    let access = t["access_token"].as_str().unwrap().to_string();
    let refresh = t["refresh_token"].as_str().unwrap().to_string();
    let replay = tok(vec![
        ("grant_type", "authorization_code".into()),
        ("code", code),
        ("code_verifier", verifier.clone()),
        ("client_id", client_id.clone()),
    ])
    .await
    .unwrap();
    assert_eq!(replay.status(), 400, "codes are single use");

    let c = mcp_client(format!("{base}/mcp"), &access).await;
    let tools = tool_names(&c.list_all_tools().await.unwrap());
    assert_eq!(
        tools,
        [
            "annotations_list",
            "check",
            "experiment_status",
            "get_context",
            "glossary_lookup",
            "propose_string",
            "status"
        ]
    );
    let (ok, v) = call(&c, "status", json!({})).await;
    assert!(ok && v["slug"] == "demo", "{v}");
    c.cancel().await.unwrap();
    let write = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "add_string", "arguments": {"key": "x", "text": "y"}}});
    let r = post(Some(&access), &write).await.unwrap();
    assert_eq!(r.status(), 403);
    let www = r.headers()["www-authenticate"].to_str().unwrap();
    assert!(
        www.contains("error=\"insufficient_scope\"") && www.contains("scope=\"write\""),
        "{www}"
    );

    // Refresh rotates; revocation kills the grant.
    let t2: Value = tok(vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh.clone()),
        ("client_id", client_id.clone()),
    ])
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let access2 = t2["access_token"].as_str().unwrap().to_string();
    assert_eq!(
        post(Some(&access), &init).await.unwrap().status(),
        401,
        "old access token rotated out"
    );
    assert_eq!(
        tok(vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh),
            ("client_id", client_id.clone())
        ])
        .await
        .unwrap()
        .status(),
        400
    );
    assert_eq!(post(Some(&access2), &init).await.unwrap().status(), 200);
    let r = http
        .post(format!("{base}/oauth/revoke"))
        .form(&[("token", t2["refresh_token"].as_str().unwrap())])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(post(Some(&access2), &init).await.unwrap().status(), 401);

    // Denied consent goes back to the client with access_denied.
    let mut deny = params;
    deny["approve"] = json!(false);
    let redirect = api(reqwest::Method::POST, "/api/oauth/authorize")
        .json(&deny)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["redirect"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        redirect.starts_with("https://lovable.dev/cb?") && redirect.contains("error=access_denied"),
        "{redirect}"
    );

    // Audit log: every tool call, with the OAuth client when there is one.
    let (n, oauth_calls): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(client_id) FROM mcp_audit")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(n == 17 && oauth_calls == 1, "{n} {oauth_calls}");
    let failed: i64 = sqlx::query_scalar("SELECT count(*) FROM mcp_audit WHERE NOT ok")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(failed >= 4);

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}

fn urlencode(s: &str) -> String {
    reqwest::Url::parse_with_params("http://x/", &[("v", s)])
        .unwrap()
        .query()
        .unwrap()[2..]
        .to_string()
}
