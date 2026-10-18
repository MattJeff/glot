//! `glot localize`, `glot agent-prompt`, `glot provenance`, `.agents/skills`, Gemini preset (mock provider, offline).
use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn glot(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("glot").unwrap();
    c.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("GLOT_REVIEWER", "Ana")
        .env_remove("RUST_LOG");
    c
}

/// next-intl project whose strings are already in catalogs (no hardcoded copy).
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
        r#"{"home":{"title":"Welcome, {name}","cta":"Start"}}"#,
    )
    .unwrap();
    dir
}

fn stdout(c: &mut Command) -> String {
    String::from_utf8(c.assert().success().get_output().stdout.clone()).unwrap()
}

#[test]
fn localize_one_step_idempotent_with_provenance() {
    let dir = project();
    let r = dir.path();

    // Dry run on a fresh project: nothing written.
    glot(r)
        .args(["localize", "--dry-run", "--targets", "fr,de"])
        .assert()
        .success();
    assert!(!r.join("glot.toml").exists());

    let full = [
        "localize",
        "--yes",
        "--no-i18nize",
        "--targets",
        "fr,de",
        "--provider",
        "mock",
    ];
    glot(r).args(full).assert().success();
    for f in [
        "glot.toml",
        ".claude/skills/glot/SKILL.md",
        ".agents/skills/glot/SKILL.md",
        "messages/fr.json",
        "messages/de.json",
    ] {
        assert!(r.join(f).exists(), "{f} missing");
    }
    let skill = fs::read_to_string(r.join(".agents/skills/glot/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: glot\ndescription: "));
    let lock = fs::read_to_string(r.join("glot.lock")).unwrap();
    // Idempotent: second run changes nothing.
    glot(r).args(full).assert().success();
    assert_eq!(fs::read_to_string(r.join("glot.lock")).unwrap(), lock);

    // Provenance: machine entries carry model + date; `glot set` records the reviewer.
    glot(r)
        .args(["set", "fr", "home.cta", "Commencer"])
        .assert()
        .success();
    let rows: serde_json::Value =
        serde_json::from_str(&stdout(glot(r).args(["provenance", "export"]))).unwrap();
    let row = |loc: &str, key: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|x| x["locale"] == loc && x["key"] == key)
            .unwrap()
            .clone()
    };
    let m = row("de", "home.title");
    assert_eq!(m["origin"], "machine");
    assert_eq!(m["ai_generated"], true);
    assert_eq!(m["model"], "mock");
    assert!(m["translated_at"].as_str().unwrap().ends_with('Z'));
    let h = row("fr", "home.cta");
    assert_eq!(
        (h["origin"].as_str(), h["reviewed_by"].as_str()),
        (Some("human"), Some("Ana"))
    );

    let csv = stdout(glot(r).args(["provenance", "export", "--format", "csv"]));
    assert!(csv.starts_with("locale,key,origin,ai_generated,model,translated_at"));
    assert!(stdout(glot(r).args(["provenance", "export", "--format", "html"])).contains("<table>"));

    glot(r).args(["provenance", "catalog"]).assert().success();
    let cat: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(r.join("messages/fr.provenance.json")).unwrap())
            .unwrap();
    assert_eq!(cat["home.cta"]["reviewed"], true);
    assert_eq!(cat["home.title"]["origin"], "machine");
    // The metadata file is not mistaken for a catalog.
    glot(r).arg("check").assert().success();
}

#[test]
fn agent_prompt_uses_config() {
    let dir = project();
    let r = dir.path();
    glot(r)
        .args(["init", "--yes", "--provider", "mock", "--targets", "ja,es"])
        .assert()
        .success();
    let p = stdout(glot(r).arg("agent-prompt"));
    assert!(
        p.contains("useTranslations") && p.contains("--targets ja,es") && p.contains("(ja, es)")
    );
    assert!(p.contains("messages/en.json"));
    let p2 = stdout(glot(r).args(["localize", "--print-agent-prompt"]));
    assert_eq!(p, p2);
    assert!(stdout(glot(r).args(["agent-prompt", "--for", "v0"])).contains("outside this builder"));
}

#[test]
fn init_gemini_preset() {
    let dir = project();
    let r = dir.path();
    glot(r)
        .args(["init", "--yes", "--provider", "gemini", "--targets", "fr"])
        .assert()
        .success();
    let toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    assert!(toml.contains(r#"provider = "openai-compatible""#));
    assert!(toml.contains(r#"model = "gemini-3-flash-preview""#));
    assert!(toml.contains("generativelanguage.googleapis.com/v1beta/openai"));
    assert!(toml.contains(r#"api_key_env = "GEMINI_API_KEY""#));
}
