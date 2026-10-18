//! `glot translate` end to end (mock provider) on every native format: read, plan, translate,
//! validate, patch the file in place, re-read.
use std::fs;
use std::path::Path;

use glot_llm::engine::{TranslateOptions, Workspace};

const FIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../core/tests/fixtures/formats"
);

fn fixture(name: &str) -> String {
    fs::read_to_string(Path::new(FIX).join(name)).unwrap()
}

fn project(
    files: &str,
    syntax: &str,
    targets: &str,
    content: &[(&str, String)],
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::write(
        r.join("glot.toml"),
        format!("source_locale = \"en\"\ntarget_locales = {targets}\nfiles = {files}\nsyntax = \"{syntax}\"\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\nfallback_model = \"mock\"\n"),
    )
    .unwrap();
    for (path, body) in content {
        let p = r.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    dir
}

/// Translate everything, assert nothing was rejected and `check` is clean; returns the reopened workspace.
async fn translate(dir: &Path) -> Workspace {
    let mut ws = Workspace::open(dir).unwrap();
    let report = ws.translate(TranslateOptions::default()).await.unwrap();
    assert!(
        report.rejected.values().all(Vec::is_empty),
        "{:#?}",
        report.rejected
    );
    assert!(report.written.values().any(|w| !w.is_empty()));
    let ws = Workspace::open(dir).unwrap();
    let errors: Vec<_> = ws
        .check()
        .into_iter()
        .filter(|f| f.issue.severity == glot_core::validate::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let plan = ws.plan();
    assert!(
        plan.missing.is_empty() && plan.stale.is_empty(),
        "{plan:#?}"
    );
    ws
}

fn read(dir: &Path, p: &str) -> String {
    fs::read_to_string(dir.join(p)).unwrap()
}

#[tokio::test]
async fn xcstrings() {
    let dir = project(
        r#"["App/Localizable.xcstrings"]"#,
        "printf",
        r#"["fr", "ja"]"#,
        &[(
            "App/Localizable.xcstrings",
            fixture("Localizable.xcstrings"),
        )],
    );
    let before = Workspace::open(dir.path()).unwrap();
    // `needs_review` is planned as stale; the comment is a translator note.
    assert_eq!(
        before.plan().stale,
        [("fr".to_string(), "Hello, %@!".to_string())]
    );
    assert_eq!(
        before.lock.notes["Hello, %@!"],
        "Greeting on the home screen"
    );
    let ws = translate(dir.path()).await;
    let fr = &ws.catalogs["fr"].entries;
    assert_eq!(fr["Hello, %@!"], "⟦fr⟧ Hello, %@!");
    assert_eq!(fr["Save"], "⟦fr⟧ Save");
    assert!(
        fr["%lld items"].contains("%lld article"),
        "existing translation kept"
    );
    let ja = &ws.catalogs["ja"].entries["%lld items"];
    assert_eq!(ja, "{count, plural, other {⟦ja⟧ %lld items}}");
    let text = read(dir.path(), "App/Localizable.xcstrings");
    assert!(text.contains("\"shouldTranslate\" : false"));
    assert!(text.contains("\"argNum\" : 1"), "substitutions kept");
    assert!(!text.contains("needs_review"));
}

#[tokio::test]
async fn strings_and_stringsdict() {
    let dir = project(
        r#"["ios/{locale}.lproj/Localizable.strings", "ios/{locale}.lproj/Localizable.stringsdict"]"#,
        "printf",
        r#"["fr", "ru"]"#,
        &[
            (
                "ios/en.lproj/Localizable.strings",
                fixture("Localizable.strings"),
            ),
            (
                "ios/en.lproj/Localizable.stringsdict",
                fixture("Localizable.stringsdict"),
            ),
        ],
    );
    let ws = translate(dir.path()).await;
    assert_eq!(
        ws.catalogs["fr"].entries["welcome.greeting"],
        "⟦fr⟧ Hello, %@!"
    );
    // Plurals land in the target `.stringsdict`, with the target's categories.
    let dict = read(dir.path(), "ios/ru.lproj/Localizable.stringsdict");
    for cat in ["one", "few", "many", "other"] {
        assert!(dict.contains(&format!("<key>{cat}</key>")), "{dict}");
    }
    assert!(
        dict.contains("%2$@ has %1$#@unread@"),
        "positional token kept: {dict}"
    );
    assert!(
        dict.contains("<string>ld</string>"),
        "value type from the branch text: {dict}"
    );
    let strings = read(dir.path(), "ios/fr.lproj/Localizable.strings");
    assert!(!strings.contains("plural"), "{strings}");
}

#[tokio::test]
async fn android() {
    let dir = project(
        r#"["app/src/main/res/values-{locale}/strings.xml"]"#,
        "printf",
        r#"["fr", "pt-BR"]"#,
        &[(
            "app/src/main/res/values/strings.xml",
            fixture("strings.xml"),
        )],
    );
    let ws = translate(dir.path()).await;
    let pt = read(dir.path(), "app/src/main/res/values-pt-rBR/strings.xml");
    assert!(!pt.contains("app_name"), "translatable=false is not copied");
    assert!(
        pt.contains("<string name=\"dont_worry\">⟦pt-BR⟧ Don\\'t worry, it\\'s saved.</string>"),
        "{pt}"
    );
    assert!(pt.contains("<item quantity=\"many\">"), "{pt}");
    assert!(pt.contains("<string-array name=\"planets\">"), "{pt}");
    assert_eq!(
        ws.catalogs["fr"].entries["welcome"],
        "⟦fr⟧ Welcome back, %1$s! You have %2$d tasks."
    );
    // The source file is untouched.
    assert_eq!(
        read(dir.path(), "app/src/main/res/values/strings.xml"),
        fixture("strings.xml")
    );
}

#[tokio::test]
async fn arb() {
    let dir = project(
        r#"["lib/l10n/app_{locale}.arb"]"#,
        "icu",
        r#"["de"]"#,
        &[("lib/l10n/app_en.arb", fixture("app_en.arb"))],
    );
    let before = Workspace::open(dir.path()).unwrap();
    assert!(before.lock.notes["greeting"].contains("userName"));
    translate(dir.path()).await;
    let de: serde_json::Value =
        serde_json::from_str(&read(dir.path(), "lib/l10n/app_de.arb")).unwrap();
    assert_eq!(de["@@locale"], "de");
    assert_eq!(de["greeting"], "⟦de⟧ Hello {userName}");
}

#[tokio::test]
async fn gettext() {
    // Source = msgids (an `en.po` with empty msgstrs, as `makemessages -l en` writes it).
    let en = "msgid \"\"\nmsgstr \"\"\n\"Language: en\\n\"\n\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n\n#. Title of the dashboard page\nmsgid \"Dashboard\"\nmsgstr \"\"\n\n#, python-format\nmsgid \"Welcome, %(name)s!\"\nmsgstr \"\"\n\n#, python-format\nmsgid \"You have %(count)s unread message\"\nmsgid_plural \"You have %(count)s unread messages\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n\nmsgctxt \"verb\"\nmsgid \"Open\"\nmsgstr \"\"\n\nmsgid \"\"\n\"A long text that spans \"\n\"several lines.\\n\"\nmsgstr \"\"\n";
    let dir = project(
        r#"["locale/{locale}/LC_MESSAGES/django.po"]"#,
        "printf",
        r#"["fr"]"#,
        &[
            ("locale/en/LC_MESSAGES/django.po", en.to_string()),
            ("locale/fr/LC_MESSAGES/django.po", fixture("fr.po")),
        ],
    );
    let before = Workspace::open(dir.path()).unwrap();
    assert_eq!(
        before.source().entries["Welcome, %(name)s!"],
        "Welcome, %(name)s!"
    );
    let plan = before.plan();
    assert_eq!(plan.missing, [("fr".into(), "Welcome, %(name)s!".into())]);
    assert_eq!(
        plan.stale,
        [("fr".into(), "You have %(count)s unread message".into())]
    );
    translate(dir.path()).await;
    let fr = read(dir.path(), "locale/fr/LC_MESSAGES/django.po");
    assert!(fr.contains("msgstr \"⟦fr⟧ Welcome, %(name)s!\""), "{fr}");
    assert!(
        fr.contains("#, python-format\nmsgid \"You have"),
        "fuzzy dropped:\n{fr}"
    );
    assert!(fr.contains("msgstr \"Tableau de bord\""), "existing kept");
    assert!(fr.contains("#~ msgid \"Old\""), "obsolete kept");
}

#[tokio::test]
async fn rails_yaml() {
    let dir = project(
        r#"["config/locales/{locale}.yml"]"#,
        "rails",
        r#"["es"]"#,
        &[("config/locales/en.yml", fixture("en.yml"))],
    );
    let ws = translate(dir.path()).await;
    let es = read(dir.path(), "config/locales/es.yml");
    assert!(es.starts_with("es:\n"), "{es}");
    assert_eq!(ws.catalogs["es"].entries["greeting"], "⟦es⟧ Hello %{name}!");
    assert!(
        ws.catalogs["es"].entries["inbox.messages"].contains("many {"),
        "es has one/many/other"
    );
}
