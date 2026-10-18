//! i18next adoption (anonymised fixtures from docs/research/i18n-methods.md): non-string values,
//! natural-language keys, v4 plural suffixes and namespaces, with the mock provider.
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

fn write(r: &Path, rel: &str, text: &str) {
    let p = r.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn run(r: &Path, args: &[&str]) -> String {
    let out = glot(r).args(args).assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn json(r: &Path, rel: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(r.join(rel)).unwrap()).unwrap()
}

fn init(r: &Path, targets: &str) {
    run(
        r,
        &[
            "init",
            "--yes",
            "--provider",
            "mock",
            "--source",
            "en",
            "--targets",
            targets,
        ],
    );
}

#[test]
fn non_string_values_never_block_and_arrays_are_translated() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "package.json", r#"{"dependencies":{"i18next":"23"}}"#);
    let src = r#"{"list":["a","b"],"n":1,"flag":false,"none":null,"ok":"Ok"}"#;
    write(r, "locales/en.json", src);
    write(r, "locales/fr.json", "{}");
    init(r, "fr");
    run(r, &["status"]);
    run(r, &["translate", "--yes"]);
    run(r, &["check"]);
    assert_eq!(fs::read_to_string(r.join("locales/en.json")).unwrap(), src);
    assert_eq!(
        json(r, "locales/fr.json"),
        serde_json::json!({"list": ["⟦fr⟧ a", "⟦fr⟧ b"], "ok": "⟦fr⟧ Ok"})
    );
}

#[test]
fn natural_language_keys_stay_flat() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "package.json", r#"{"dependencies":{"i18next":"23"}}"#);
    write(
        r,
        "src/i18n.ts",
        "i18n.init({ resources, keySeparator: false, nsSeparator: false });\n",
    );
    write(
        r,
        "locales/en.json",
        r#"{"Welcome back.":"Welcome back.","Save":"Save","menu":{"a":"A"}}"#,
    );
    write(r, "locales/fr.json", "{}");
    init(r, "fr");
    let toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    assert!(
        toml.contains("[i18next]\nkey_separator = false\nns_separator = false\n"),
        "{toml}"
    );
    run(r, &["translate", "--yes"]);
    let fr = json(r, "locales/fr.json");
    assert_eq!(fr["Welcome back."], "⟦fr⟧ Welcome back.");
    assert_eq!(fr["menu.a"], "⟦fr⟧ A");
    run(r, &["check"]);
}

#[test]
fn plural_families_and_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        "package.json",
        r#"{"dependencies":{"i18next":"23","react-i18next":"14"}}"#,
    );
    let common = r#"{"title":"Home","item_one":"{{count}} item","item_other":"{{count}} items","friend_male":"He is a friend","friend_female":"She is a friend","nest":"$t(title) page"}"#;
    write(r, "public/locales/en/common.json", common);
    write(
        r,
        "public/locales/en/home.json",
        r#"{"title":"Welcome home"}"#,
    );
    write(r, "public/locales/pl/common.json", r#"{"title":"Dom"}"#);
    write(
        r,
        "src/App.tsx",
        "import { useTranslation } from 'react-i18next';\n\
         export function A() { const { t } = useTranslation('home'); return <div>{t('title')}{t('common:item', { count: 2 })}{t('common:title')}{t('common:nest')}{t('common:friend', { context: 'male' })}</div>; }\n",
    );
    init(r, "pl");
    let toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    assert!(
        toml.contains(r#"files = ["public/locales/{locale}/{ns}.json"]"#),
        "{toml}"
    );
    let status = run(r, &["status"]);
    assert!(status.contains("pl"), "{status}");
    run(r, &["translate", "--yes"]);
    let pl = json(r, "public/locales/pl/common.json");
    // Exactly pl's CLDR categories; the existing translation and the context keys are kept.
    for cat in ["one", "few", "many", "other"] {
        assert!(pl[format!("item_{cat}")].is_string(), "item_{cat}: {pl}");
    }
    assert!(pl.get("item_zero").is_none() && pl.get("item_two").is_none());
    assert_eq!(pl["title"], "Dom");
    assert_eq!(pl["friend_male"], "⟦pl⟧ He is a friend");
    // `home:title` no longer collides with `common:title`.
    assert_eq!(
        json(r, "public/locales/pl/home.json")["title"],
        "⟦pl⟧ Welcome home"
    );
    // Sources untouched byte for byte.
    assert_eq!(
        fs::read_to_string(r.join("public/locales/en/common.json")).unwrap(),
        common
    );
    run(r, &["check"]);
    // An incomplete family fails `check`.
    let mut broken = pl.clone();
    broken.as_object_mut().unwrap().remove("item_few");
    write(r, "public/locales/pl/common.json", &broken.to_string());
    let out = glot(r).arg("check").assert().failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.contains("common:item") && stdout.contains("few"),
        "{stdout}"
    );
}
