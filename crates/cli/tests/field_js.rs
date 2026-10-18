//! Field test pattern (anonymized): a Next app with hand-written TS dictionaries
//! (`src/i18n/{en,fr}.ts`, its own `t()`). glot must read, translate and patch them in place, keep
//! them out of the hardcoded-string scan, and not rewrite the app to another i18n library.
use std::path::Path;
use std::process::Command;

fn glot(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glot"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), log)
}

#[test]
fn ts_dictionaries_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    let w = |p: &str, c: &str| {
        std::fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
        std::fs::write(r.join(p), c).unwrap();
    };
    w(
        "package.json",
        r#"{"dependencies":{"next":"16","react":"19"}}"#,
    );
    w(
        "src/app/layout.tsx",
        "export default function L({ children }) { return <html><body>{children}</body></html>; }\n",
    );
    w(
        "src/i18n/en.ts",
        "export const en = {\n  nav: {\n    home: \"Home\",\n    billing: \"Billing\",\n  },\n  plans: {\n    title: \"Choose a plan\",\n    count: (n: number) => `${n} plans`,\n  },\n};\n",
    );
    w(
        "src/i18n/fr.ts",
        "import type { Dict } from \"./types\";\n\nexport const fr: Dict = {\n  nav: {\n    home: \"Accueil\",\n  },\n  plans: {\n    title: \"Choisissez une offre\",\n    count: (n: number) => `${n} offres`,\n  },\n};\n",
    );
    w(
        "src/i18n/t.ts",
        "export function t(key: string) { return key; }\n",
    );
    w(
        "src/app/page.tsx",
        "import { t } from \"../i18n/t\";\nexport default function P() { return <main><h1>{t(\"plans.title\")}</h1><p>Contact sales today</p></main>; }\n",
    );

    let (ok, log) = glot(r, &["init", "--yes", "--provider", "mock"]);
    assert!(ok, "{log}");
    assert!(log.contains("TS/JS dictionaries"), "{log}");
    let toml = std::fs::read_to_string(r.join("glot.toml")).unwrap();
    assert!(
        toml.contains("files = [\"src/i18n/{locale}.ts\"]"),
        "{toml}"
    );
    assert!(toml.contains("\"src/i18n/{en,fr}.ts\""), "{toml}");

    // Dictionaries are catalogs, not hardcoded strings; the page copy is.
    let (ok, log) = glot(r, &["extract", "--json"]);
    assert!(ok, "{log}");
    let report: serde_json::Value = serde_json::from_str(&log).unwrap();
    let texts: Vec<&str> = report["hardcoded"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report["config"].as_array().unwrap())
        .map(|h| h["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["Contact sales today"]);

    let (ok, log) = glot(r, &["status"]);
    assert!(ok && log.contains("3 keys in source locale en"), "{log}");

    // Own runtime: i18nize refuses instead of adding next-intl.
    let (ok, log) = glot(r, &["i18nize", "--dry-run"]);
    assert!(!ok && log.contains("own i18n runtime"), "{log}");

    // translate (mock) patches fr.ts in place: function value and type import untouched.
    let (ok, log) = glot(r, &["translate", "--yes"]);
    assert!(ok, "{log}");
    let fr = std::fs::read_to_string(r.join("src/i18n/fr.ts")).unwrap();
    assert!(fr.starts_with("import type { Dict } from \"./types\";\n\nexport const fr: Dict = {\n  nav: {\n    home: \"Accueil\",\n    billing: "), "{fr}");
    assert!(fr.contains("count: (n: number) => `${n} offres`,"), "{fr}");
    let (ok, log) = glot(r, &["check"]);
    assert!(ok, "{log}");
}
