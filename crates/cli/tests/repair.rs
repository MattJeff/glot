//! New-user papercuts: invalid translations get repaired, mock shows $0, `--provider`,
//! file order kept, config/data copy reported by `extract`.
use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn glot(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("glot").unwrap();
    c.current_dir(dir)
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG")
        .env_remove("ANTHROPIC_API_KEY");
    c
}

fn out(c: &mut Command) -> String {
    let o = c.output().unwrap();
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

/// next-intl project whose fr has a broken placeholder (`{nom}`) and a non-alphabetical order.
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
        r#"{"home":{"title":"Home"},"auth":{"login":{"title":"Welcome back, {name}","cta":"Sign in"}}}"#,
    )
    .unwrap();
    fs::write(
        r.join("messages/fr.json"),
        "{\n  \"home\": {\n    \"title\": \"Accueil\"\n  },\n  \"auth\": {\n    \"login\": {\n      \"title\": \"Bon retour, {nom}\"\n    }\n  }\n}\n",
    )
    .unwrap();
    fs::create_dir_all(r.join("app")).unwrap();
    fs::write(
        r.join("app/page.tsx"),
        r#"import { useTranslations } from "next-intl";
const plans = [{ name: "Pro", description: "For individuals and small teams" }];
export default function Page() {
  const t = useTranslations("auth.login");
  return <main><h1>{t("title", { name: "x" })}</h1><button>{t("cta")}</button></main>;
}
"#,
    )
    .unwrap();
    glot(r)
        .args(["init", "--yes", "--provider", "mock"])
        .assert()
        .success();
    dir
}

#[test]
fn invalid_translation_is_repaired_and_order_kept() {
    let dir = project();
    let r = dir.path();
    glot(r).arg("check").assert().failure();
    let s = out(glot(r).arg("status"));
    assert!(s.contains("1 invalid"), "{s}");

    let s = out(glot(r).args(["translate", "--dry-run"]));
    assert!(s.contains("model mock"), "{s}");
    assert!(s.contains("Estimated cost: $0"), "{s}");
    assert!(!s.contains("claude"), "{s}");

    glot(r).args(["translate", "--yes"]).assert().success();
    glot(r).arg("check").assert().success();
    let fr = fs::read_to_string(r.join("messages/fr.json")).unwrap();
    assert!(fr.contains("{name}") && !fr.contains("{nom}"), "{fr}");
    // Existing order kept (home before auth), the new key appended after the existing `title`.
    let (home, auth) = (fr.find("\"home\"").unwrap(), fr.find("\"auth\"").unwrap());
    assert!(home < auth, "{fr}");
    assert!(
        fr.find("\"title\": \"⟦").unwrap() < fr.find("\"cta\"").unwrap(),
        "{fr}"
    );

    // A hand edit that breaks validation is never overwritten: point at `glot set`.
    let broken = fr.replace("{name}", "{nom}");
    fs::write(r.join("messages/fr.json"), broken).unwrap();
    let s = out(glot(r).args(["translate", "--yes"]));
    assert!(s.contains("glot set fr auth.login.title"), "{s}");
    assert!(s.contains("Nothing to translate"), "{s}");
}

#[test]
fn provider_flag_and_missing_key_hint() {
    let dir = project();
    let r = dir.path();
    let toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    fs::write(
        r.join("glot.toml"),
        toml.replace(r#"provider = "mock""#, r#"provider = "anthropic""#),
    )
    .unwrap();
    let s = out(glot(r).args(["translate", "--yes"]));
    assert!(s.contains("glot translate --provider mock"), "{s}");
    glot(r)
        .args(["translate", "--yes", "--provider", "mock"])
        .assert()
        .success();
}

#[test]
fn extract_reports_config_strings() {
    let dir = project();
    let s = out(glot(dir.path()).arg("extract"));
    assert!(s.contains("Not converted yet (config/data) (2)"), "{s}");
    assert!(s.contains("For individuals and small teams"), "{s}");
    assert!(!s.contains("nothing to report"), "{s}");
}
