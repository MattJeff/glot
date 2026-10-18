use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn glot(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("glot").unwrap();
    c.current_dir(dir)
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG");
    c
}

/// Fake next-intl project: en source, partial fr target, one component.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::write(
        r.join("package.json"),
        r#"{"dependencies":{"next":"15","next-intl":"4"}}"#,
    )
    .unwrap();
    fs::create_dir_all(r.join("messages")).unwrap();
    fs::write(
        r.join("messages/en.json"),
        r#"{"auth":{"login":{"title":"Welcome back, {name}","cta":"Sign in"}},"home":{"title":"Home"}}"#,
    )
    .unwrap();
    fs::write(
        r.join("messages/fr.json"),
        r#"{"auth":{"login":{"cta":"Se connecter"}}}"#,
    )
    .unwrap();
    fs::create_dir_all(r.join("app")).unwrap();
    fs::write(
        r.join("app/page.tsx"),
        r#"import { useTranslations } from "next-intl";
export default function Page() {
  const t = useTranslations("auth.login");
  return <main><h1>{t("title", { name: "x" })}</h1><button>{t("cta")}</button><p>Hardcoded hello</p></main>;
}
"#,
    )
    .unwrap();
    dir
}

fn init(r: &Path) {
    glot(r)
        .args(["init", "--yes", "--provider", "mock"])
        .assert()
        .success();
}

#[test]
fn init_is_idempotent_and_writes_integration() {
    let dir = project();
    let r = dir.path();
    init(r);
    let toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    assert!(toml.contains(r#"source_locale = "en""#));
    assert!(toml.contains(r#"target_locales = ["fr"]"#));
    assert!(toml.contains("messages/{locale}.json"));
    assert!(toml.contains(r#"provider = "mock""#));
    for f in [
        "glossary.toml",
        ".mcp.json",
        "CLAUDE.md",
        ".claude/skills/glot/SKILL.md",
        ".cursor/rules/glot.mdc",
        ".github/workflows/glot.yml",
    ] {
        assert!(r.join(f).exists(), "{f} missing");
    }
    let mcp: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(r.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["glot"]["args"][0], "mcp");

    // Second run: nothing duplicated, user edits kept.
    fs::write(
        r.join("CLAUDE.md"),
        fs::read_to_string(r.join("CLAUDE.md")).unwrap() + "\nmy notes\n",
    )
    .unwrap();
    init(r);
    let claude = fs::read_to_string(r.join("CLAUDE.md")).unwrap();
    assert_eq!(claude.matches("<!-- glot:start -->").count(), 1);
    assert!(claude.contains("my notes"));
    assert_eq!(fs::read_to_string(r.join("glot.toml")).unwrap(), toml);
}

#[test]
fn status_check_translate_extract() {
    let dir = project();
    let r = dir.path();
    init(r);

    let out = glot(r)
        .args(["status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let st: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(st["keys"], 3);
    assert_eq!(st["locales"]["fr"]["missing"], 2);

    // Broken placeholder => check fails with exit 1 and a GitHub annotation.
    glot(r)
        .args(["set", "fr", "auth.login.title", "Bon retour"])
        .assert()
        .code(1);
    fs::write(
        r.join("messages/fr.json"),
        r#"{"auth":{"login":{"cta":"Se connecter","title":"Bon retour"}}}"#,
    )
    .unwrap();
    let gh = glot(r)
        .args(["check", "--format", "github"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8(gh)
            .unwrap()
            .contains("::error file=messages/fr.json")
    );
    fs::write(
        r.join("messages/fr.json"),
        r#"{"auth":{"login":{"cta":"Se connecter"}}}"#,
    )
    .unwrap();

    glot(r).args(["translate", "--dry-run"]).assert().success();
    glot(r).args(["translate", "--yes"]).assert().success();
    let fr: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(r.join("messages/fr.json")).unwrap()).unwrap();
    assert!(fr["home"]["title"].is_string());
    assert!(r.join("glot.lock").exists());
    glot(r).arg("check").assert().success();

    let out = glot(r)
        .args(["extract", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ex: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        ex["hardcoded"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["text"] == "Hardcoded hello")
    );
    assert!(
        ex["unused"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "home.title")
    );
}

#[test]
fn errors_without_workspace() {
    let dir = tempfile::tempdir().unwrap();
    glot(dir.path()).arg("status").assert().code(2);
}

#[test]
fn prune_keeps_keys_named_outside_extract_scope() {
    let dir = project();
    let r = dir.path();
    init(r);
    fs::write(
        r.join("messages/en.json"),
        r#"{"auth":{"login":{"title":"Welcome back, {name}","cta":"Sign in"}},"home":{"legacy":"Old","dead":"Gone"}}"#,
    )
    .unwrap();
    fs::create_dir_all(r.join("utils")).unwrap();
    fs::write(
        r.join("utils/legacy.ts"),
        r#"export const k = t("legacy");"#,
    )
    .unwrap();
    glot(r).args(["unused", "--prune"]).assert().success();
    let en = fs::read_to_string(r.join("messages/en.json")).unwrap();
    assert!(en.contains("legacy") && !en.contains("dead"), "{en}");
}

/// `glot push` sends the extracted code context; a one-shot fake server captures the body.
#[test]
fn push_sends_code_context() {
    use std::io::{BufRead, BufReader, Read, Write};
    let dir = project();
    let r = dir.path();
    init(r);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    toml.push_str(&format!(
        "\n[server]\nurl = \"http://127.0.0.1:{port}\"\nproject = \"demo\"\n"
    ));
    fs::write(r.join("glot.toml"), toml).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut len = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        let reply = r#"{"created":3,"updated":0,"conflicts":[]}"#;
        write!(
            &stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        )
        .unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()
    });
    glot(r)
        .env("GLOT_TOKEN", "glot_x")
        .arg("push")
        .assert()
        .success();
    let body = server.join().unwrap();
    let cta = &body["context"]["auth.login.cta"];
    assert_eq!(cta["ui_role"], "button", "{body}");
    assert_eq!(cta["refs"][0]["path"], "app/page.tsx");
    assert_eq!(cta["refs"][0]["line"], 4);
    assert!(
        body["context"]["home.title"]["refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
