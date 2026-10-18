//! « Connect GitHub » end to end: manifest → app stored → install/OAuth callback → repo picked → job clones a local
//! git remote (file://), runs the real `glot` binary (mock provider), pushes `glot/localize` and opens a PR on a fake
//! GitHub API → signed push webhooks refresh the PR, then open `glot/update` once the first PR is closed.
//! Needs `cargo build -p glot-cli` (same target dir). Skipped when Postgres is unreachable.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Method, Request, StatusCode, header},
    response::IntoResponse,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{
    collections::HashMap,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

const KEY: &str = include_str!("fixtures/github-app-test-key.pem"); // test-only key, not a real app's
const INSTALL_TOKEN: &str = "ghs_installation_SECRET_123";

#[derive(Default)]
struct Fake {
    log: Vec<(String, String, Value)>,
    /// PR number -> state
    prs: HashMap<i64, String>,
}
type Shared = Arc<Mutex<Fake>>;

async fn fake_github(State(f): State<Shared>, req: Request<Body>) -> axum::response::Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let bytes = req.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut f = f.lock().unwrap();
    f.log.push((method.clone(), path.clone(), body.clone()));
    let r = |v: Value| axum::Json(v).into_response();
    let repo_call = path.starts_with("/repos/") || path.starts_with("/installation/");
    if repo_call && auth != format!("Bearer {INSTALL_TOKEN}") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match (method.as_str(), path.as_str()) {
        ("POST", "/app-manifests/code123/conversions") => r(json!({
            "id": 42, "slug": "glot-test", "client_id": "Iv1.test", "client_secret": "client-secret",
            "webhook_secret": "whsec", "pem": KEY, "html_url": "https://github.com/apps/glot-test",
        })),
        ("POST", "/login/oauth/access_token") => r(json!({ "access_token": "ghu_user" })),
        ("GET", "/user/installations") if auth == "Bearer ghu_user" => {
            r(json!({ "installations": [{ "id": 7, "account": { "login": "acme" } }] }))
        }
        ("POST", "/app/installations/7/access_tokens") => {
            // App JWT: RS256, iss = client id.
            let jwt = auth.trim_start_matches("Bearer ");
            let parts: Vec<&str> = jwt.split('.').collect();
            assert_eq!(parts.len(), 3);
            let claims: Value = serde_json::from_slice(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(parts[1])
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(claims["iss"], "Iv1.test");
            r(json!({ "token": INSTALL_TOKEN, "expires_at": "2099-01-01T00:00:00Z" }))
        }
        ("GET", "/installation/repositories") => r(json!({ "repositories": [
            { "full_name": "acme/app", "default_branch": "main", "private": true }
        ] })),
        ("GET", "/repos/acme/app/pulls") => r(json!([])),
        ("POST", "/repos/acme/app/pulls") => {
            let n = f.prs.len() as i64 + 1;
            f.prs.insert(n, "open".into());
            r(json!({ "number": n, "html_url": format!("https://github.com/acme/app/pull/{n}") }))
        }
        (m, p) if p.starts_with("/repos/acme/app/pulls/") => {
            let n: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
            let state = f.prs.get(&n).cloned().unwrap_or("closed".into());
            r(
                json!({ "number": n, "state": state, "html_url": format!("https://github.com/acme/app/pull/{n}"), "method": m }),
            )
        }
        ("POST", p) if p.starts_with("/repos/acme/app/issues/") => r(json!({ "id": 1 })),
        _ => StatusCode::IM_A_TEAPOT.into_response(),
    }
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=dev", "-c", "user.email=dev@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn sign(body: &str) -> String {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"whsec").unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

const PAGE: &str = r#"export default function Home() {
  return (
    <main>
      <h1>Welcome to Acme</h1>
      <p>Track your invoices in one place.</p>
      <button>Start free trial</button>
    </main>
  );
}
"#;

#[tokio::test]
async fn connect_github_end_to_end() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    let glot = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("glot");
    assert!(
        glot.exists(),
        "build the CLI first: cargo build -p glot-cli ({})",
        glot.display()
    );

    // Remote: bare repo with a small Next.js app on `main`.
    let tmp = std::env::temp_dir().join(format!("glot-gh-e2e-{}", uuid::Uuid::new_v4().simple()));
    let remote = tmp.join("remotes/acme/app.git");
    std::fs::create_dir_all(&remote).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let work = tmp.join("work");
    std::fs::create_dir_all(work.join("app")).unwrap();
    std::fs::write(
        work.join("package.json"),
        r#"{"name":"acme","private":true,"dependencies":{"next":"16.0.0","react":"19.0.0","react-dom":"19.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(work.join("app/page.tsx"), PAGE).unwrap();
    std::fs::write(
        work.join("app/layout.tsx"),
        "export default function RootLayout({ children }: { children: React.ReactNode }) {\n  return (<html lang=\"en\"><body>{children}</body></html>);\n}\n",
    )
    .unwrap();
    sh(&work, &["init", "-q", "-b", "main"]);
    sh(&work, &["add", "-A"]);
    sh(&work, &["commit", "-qm", "init"]);
    sh(
        &work,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    sh(&work, &["push", "-q", "origin", "main"]);

    let fake: Shared = Default::default();
    let gh = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gh_url = format!("http://{}", gh.local_addr().unwrap());
    let fake_app = Router::new().fallback(fake_github).with_state(fake.clone());
    tokio::spawn(async move { axum::serve(gh, fake_app).await.unwrap() });
    // SAFETY: set before any server code reads them; this test binary has a single test.
    unsafe {
        std::env::set_var("GLOT_GITHUB_API", &gh_url);
        std::env::set_var("GLOT_GITHUB_WEB", &gh_url);
        std::env::set_var(
            "GLOT_GITHUB_GIT",
            format!("file://{}", tmp.join("remotes").display()),
        );
        std::env::set_var("GLOT_BIN", &glot);
        std::env::set_var("GLOT_ADMIN_EMAILS", "boss@example.com");
        std::env::set_var("GLOT_WEB_ORIGIN", "http://localhost:3100");
    }

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
    let app = glot_server::router(state, "http://localhost:3100");

    let mut cookie = String::new();
    let mut call = async |method: Method,
                          uri: &str,
                          body: Value,
                          extra: &[(&str, String)]|
           -> (StatusCode, Value, String) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !cookie.is_empty() {
            req = req.header(header::COOKIE, format!("glot_session={cookie}"));
        }
        for (k, v) in extra {
            req = req.header(*k, v);
        }
        let body = match &body {
            Value::Null => Body::empty(),
            Value::String(s) => Body::from(s.clone()),
            v => Body::from(v.to_string()),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        if let Some(c) = res.headers().get(header::SET_COOKIE) {
            cookie = c
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .trim_start_matches("glot_session=")
                .to_string();
        }
        let location = res
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            location,
        )
    };
    let get = Method::GET;
    let post = Method::POST;

    let (s, ..) = call(
        post.clone(),
        "/api/auth/signup",
        json!({"email": "boss@example.com", "password": "correct horse", "name": "Boss"}),
        &[],
    )
    .await;
    assert!(s.is_success());

    // 1. Admin creates the app from the manifest.
    let (_, v, _) = call(get.clone(), "/api/github/app", Value::Null, &[]).await;
    assert_eq!(
        (v["configured"].as_bool(), v["can_admin"].as_bool()),
        (Some(false), Some(true))
    );
    let (s, v, _) = call(
        post.clone(),
        "/api/github/app/manifest",
        json!({"name": "glot-test"}),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let manifest: Value = serde_json::from_str(v["manifest"].as_str().unwrap()).unwrap();
    assert_eq!(
        manifest["default_permissions"],
        json!({"contents": "write", "pull_requests": "write", "issues": "write", "metadata": "read"})
    );
    assert_eq!(
        manifest["hook_attributes"]["url"],
        "http://localhost:3100/api/github/webhook"
    );
    let action = v["action"].as_str().unwrap();
    assert!(action.starts_with(&format!("{gh_url}/settings/apps/new?state=")));
    let st = action.rsplit("state=").next().unwrap().to_string();
    let (s, _, loc) = call(
        get.clone(),
        &format!("/api/github/app/callback?code=code123&state={st}"),
        Value::Null,
        &[],
    )
    .await;
    assert!(
        s.is_redirection() && loc.ends_with("/admin/github-app?created=1"),
        "{s} {loc}"
    );
    // States are single use.
    let (s, ..) = call(
        get.clone(),
        &format!("/api/github/app/callback?code=code123&state={st}"),
        Value::Null,
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v, _) = call(get.clone(), "/api/github/app", Value::Null, &[]).await;
    assert_eq!(v["slug"], "glot-test");
    let enc: Vec<u8> = sqlx::query_scalar("SELECT private_key_enc FROM github_app")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&enc).contains("PRIVATE KEY"),
        "private key stored encrypted"
    );

    // 2. Project, install, pick a repo.
    call(post.clone(), "/api/projects", json!({"name": "Web", "slug": "web", "source_locale": "en", "target_locales": ["fr", "de"]}), &[]).await;
    call(
        Method::PATCH,
        "/api/projects/web",
        json!({"llm": {"provider": "mock", "model": "mock"}}),
        &[],
    )
    .await;
    let (_, v, _) = call(get.clone(), "/api/projects/web/github", Value::Null, &[]).await;
    assert_eq!(v["app"]["slug"], "glot-test");
    assert_eq!(v["installations"], json!([]));
    // An installation id the user never proved access to is refused.
    let (s, ..) = call(
        get.clone(),
        "/api/projects/web/github/repos?installation_id=7",
        Value::Null,
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, v, _) = call(
        post.clone(),
        "/api/projects/web/github/connect",
        Value::Null,
        &[],
    )
    .await;
    let install = v["install_url"].as_str().unwrap();
    assert!(install.starts_with(&format!("{gh_url}/apps/glot-test/installations/new?state=")));
    let st = install.rsplit("state=").next().unwrap();
    let (s, _, loc) = call(
        get.clone(),
        &format!(
            "/api/github/callback?code=oauth1&installation_id=7&setup_action=install&state={st}"
        ),
        Value::Null,
        &[],
    )
    .await;
    assert!(
        s.is_redirection() && loc == "http://localhost:3100/app/web/github?installed=1",
        "{loc}"
    );
    let (_, v, _) = call(
        get.clone(),
        "/api/projects/web/github/repos?installation_id=7",
        Value::Null,
        &[],
    )
    .await;
    assert_eq!(v[0]["full_name"], "acme/app");
    let (s, v, _) = call(
        Method::PUT,
        "/api/projects/web/github/link",
        json!({"installation_id": 7, "repo": "evil/other"}),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v, _) = call(
        Method::PUT,
        "/api/projects/web/github/link",
        json!({"installation_id": 7, "repo": "acme/app"}),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["link"]["branch"], "main");
    let job = v["job"]["id"].as_str().unwrap().to_string();

    macro_rules! wait {
        ($id:expr) => {{
            let id: String = $id.to_string();
            let mut out = Value::Null;
            for _ in 0..600 {
                let (_, j, _) =
                    call(get.clone(), &format!("/api/jobs/{id}"), Value::Null, &[]).await;
                if j["status"] == "done" || j["status"] == "failed" {
                    out = j;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            assert!(!out.is_null(), "job {id} did not finish");
            out
        }};
    }
    let j = wait!(job);
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["progress"], json!({"done": 6, "total": 6}));

    // 3. Branch pushed with catalogs (no workflow file), PR opened with the report.
    let check = tmp.join("check");
    sh(
        &tmp,
        &[
            "clone",
            "-q",
            "-b",
            "glot/localize",
            remote.to_str().unwrap(),
            check.to_str().unwrap(),
        ],
    );
    let fr: Value =
        serde_json::from_str(&std::fs::read_to_string(check.join("messages/fr.json")).unwrap())
            .unwrap();
    assert!(!fr.as_object().unwrap().is_empty());
    assert!(check.join("glot.toml").exists() && check.join("glot.lock").exists());
    assert!(!check.join(".github/workflows/glot.yml").exists());
    assert!(
        std::fs::read_to_string(check.join("app/page.tsx"))
            .unwrap()
            .contains("useTranslations")
    );
    let pr = {
        let f = fake.lock().unwrap();
        assert!(
            f.log
                .iter()
                .any(|(_, p, b)| p == "/app/installations/7/access_tokens"
                    && b["repositories"] == json!(["app"]))
        );
        f.log
            .iter()
            .find(|(m, p, _)| m == "POST" && p == "/repos/acme/app/pulls")
            .unwrap()
            .2
            .clone()
    };
    assert_eq!(
        (pr["head"].as_str(), pr["base"].as_str()),
        (Some("glot/localize"), Some("main"))
    );
    let body = pr["body"].as_str().unwrap();
    if let Ok(p) = std::env::var("GLOT_DUMP_PR") {
        std::fs::write(p, body).unwrap();
    }
    for needle in [
        "| `fr` |",
        "| `de` |",
        "3 hardcoded strings",
        "npm install next-intl",
        "glot/update",
        "100 %",
    ] {
        assert!(body.contains(needle), "PR body lacks {needle:?}:\n{body}");
    }
    assert!(!body.contains(INSTALL_TOKEN));
    let (_, v, _) = call(
        get.clone(),
        "/api/projects/web/keys?locale=fr",
        Value::Null,
        &[],
    )
    .await;
    assert_eq!(v["total"], 3, "catalogs synced into the project");
    let (_, v, _) = call(get.clone(), "/api/projects/web/github", Value::Null, &[]).await;
    assert_eq!(v["link"]["pr_number"], 1);
    assert_eq!(v["link"]["pr_url"], "https://github.com/acme/app/pull/1");
    let err: Option<String> =
        sqlx::query_scalar("SELECT string_agg(coalesce(error, ''), '') FROM jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!err.unwrap_or_default().contains(INSTALL_TOKEN));

    // 4. Webhooks: bad signature refused; a push on main refreshes the open PR #1.
    let push = json!({"ref": "refs/heads/main", "repository": {"full_name": "acme/app"}, "installation": {"id": 7}}).to_string();
    let (s, ..) = call(
        post.clone(),
        "/api/github/webhook",
        Value::String(push.clone()),
        &[
            ("x-github-event", "push".into()),
            ("x-hub-signature-256", "sha256=00".into()),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    std::fs::write(
        work.join("app/page.tsx"),
        PAGE.replace(
            "<button>Start free trial</button>",
            "<button>Start free trial</button>\n      <a>Read the docs</a>",
        ),
    )
    .unwrap();
    sh(&work, &["commit", "-qam", "docs link"]);
    sh(&work, &["push", "-q", "origin", "main"]);
    let hook = |event: &str, body: &str| {
        vec![
            ("x-github-event", event.to_string()),
            ("x-hub-signature-256", sign(body)),
        ]
    };
    let (s, v, _) = call(
        post.clone(),
        "/api/github/webhook",
        Value::String(push.clone()),
        &hook("push", &push),
    )
    .await;
    assert_eq!((s, v["queued"].as_i64()), (StatusCode::ACCEPTED, Some(1)));
    let (_, v, _) = call(get.clone(), "/api/projects/web/github", Value::Null, &[]).await;
    let j = wait!(v["job"]["id"].as_str().unwrap());
    assert_eq!(
        (j["status"].as_str(), j["kind"].as_str()),
        (Some("done"), Some("github_update")),
        "{j}"
    );
    {
        let f = fake.lock().unwrap();
        assert_eq!(f.prs.len(), 1, "one glot PR at a time");
        assert!(f.log.iter().any(|(m, p, b)| m == "PATCH"
            && p == "/repos/acme/app/pulls/1"
            && b["body"].as_str().unwrap().contains("4 hardcoded strings")));
        assert!(f.log.iter().any(|(m, p, b)| m == "POST"
            && p == "/repos/acme/app/issues/1/comments"
            && b["body"].as_str().unwrap().contains("glot")));
    }

    // 5. PR merged (closed webhook) → next push opens `glot/update`.
    sh(&work, &["fetch", "-q", "origin"]);
    sh(&work, &["merge", "-q", "--no-edit", "origin/glot/localize"]);
    std::fs::write(
        work.join("app/page.tsx"),
        std::fs::read_to_string(work.join("app/page.tsx"))
            .unwrap()
            .replace("</main>", "  <footer>Made in Lyon</footer>\n    </main>"),
    )
    .unwrap();
    sh(&work, &["commit", "-qam", "footer"]);
    sh(&work, &["push", "-q", "origin", "main"]);
    fake.lock().unwrap().prs.insert(1, "closed".into());
    let closed = json!({"action": "closed", "number": 1, "repository": {"full_name": "acme/app"}, "installation": {"id": 7}}).to_string();
    let (s, ..) = call(
        post.clone(),
        "/api/github/webhook",
        Value::String(closed.clone()),
        &hook("pull_request", &closed),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (_, v, _) = call(get.clone(), "/api/projects/web/github", Value::Null, &[]).await;
    assert_eq!(v["link"]["pr_number"], Value::Null);
    call(
        post.clone(),
        "/api/github/webhook",
        Value::String(push.clone()),
        &hook("push", &push),
    )
    .await;
    let (_, v, _) = call(get.clone(), "/api/projects/web/github", Value::Null, &[]).await;
    let j = wait!(v["job"]["id"].as_str().unwrap());
    assert_eq!(j["status"], "done", "{j}");
    let pr2 = fake
        .lock()
        .unwrap()
        .log
        .iter()
        .rfind(|(m, p, _)| m == "POST" && p == "/repos/acme/app/pulls")
        .unwrap()
        .2
        .clone();
    assert_eq!(pr2["head"], "glot/update");
    let (_, v, _) = call(get.clone(), "/api/projects/web/github", Value::Null, &[]).await;
    assert_eq!(v["link"]["pr_number"], 2);
    let upd = tmp.join("upd");
    sh(
        &tmp,
        &[
            "clone",
            "-q",
            "-b",
            "glot/update",
            remote.to_str().unwrap(),
            upd.to_str().unwrap(),
        ],
    );
    let page = std::fs::read_to_string(upd.join("app/page.tsx")).unwrap();
    assert!(
        !page.contains("Made in Lyon</footer>"),
        "new string converted:\n{page}"
    );

    // Scratch dirs cleaned up.
    let jobs: Vec<String> = sqlx::query_scalar("SELECT id::text FROM jobs")
        .fetch_all(&pool)
        .await
        .unwrap();
    for j in jobs {
        assert!(!std::env::temp_dir().join(format!("glot-gh-{j}")).exists());
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
