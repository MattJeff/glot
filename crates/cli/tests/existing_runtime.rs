//! A repository that already runs its own i18n (Orizn's shape, anonymized): a hand-written
//! glot.toml scoped to a landing's JSON catalogs, the site's own provider elsewhere, module
//! constants in other pages. `glot localize` translates the catalogs and touches nothing else:
//! no library installed, no provider or config file added, no constant turned into a hook,
//! nothing rewritten outside the catalogs' folder. The strings found are listed in the report.
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

fn files(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(t) = std::fs::read_to_string(&p) {
                out.insert(p.strip_prefix(root).unwrap().display().to_string(), t);
            }
        }
    }
    out
}

#[test]
fn own_runtime_and_scoped_catalogs_are_respected() {
    for own_provider in [true, false] {
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
        w("next.config.ts", "export default {};\n");
        w(
            "glot.toml",
            "# Landing catalogs only; the rest of the site keeps its own dictionaries.\nsource_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"src/app/landing/messages/{locale}.json\"]\nformat = \"json-flat\"\nsyntax = \"icu\"\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
        );
        w(
            "src/app/landing/messages/en.json",
            "{\n  \"hero.h1\": \"Entry requirements for every passport\"\n}\n",
        );
        w("src/app/landing/messages/fr.json", "{}\n");
        w(
            "src/app/landing/Landing.tsx",
            "export default function Landing({ c }: { c: any }) {\n  return <main><h1>{c.hero.h1}</h1><a href=\"/demo\">Book a demo</a></main>;\n}\n",
        );
        if own_provider {
            w(
                "src/lib/i18n/context.tsx",
                "\"use client\";\nimport { createContext, useContext } from \"react\";\nconst Ctx = createContext({ t: (k: string) => k });\nexport function useI18n() { return useContext(Ctx); }\nexport function I18nProvider({ children }: { children: React.ReactNode }) { const t = (k: string) => k; return <Ctx.Provider value={{ t }}>{children}</Ctx.Provider>; }\n",
            );
            w(
                "src/app/layout.tsx",
                "import { I18nProvider } from \"@/lib/i18n/context\";\nexport default function L({ children }: { children: React.ReactNode }) { return <html><body><I18nProvider>{children}</I18nProvider></body></html>; }\n",
            );
        }
        w(
            "src/app/admin/page.tsx",
            "const RANGES = [{ label: \"7 jours\", value: \"7d\" }, { label: \"30 jours\", value: \"30d\" }];\nexport default function Admin() {\n  return <ul>{RANGES.map((r) => <li key={r.value}>{r.label}</li>)}<li>Last refresh</li></ul>;\n}\n",
        );
        let before = files(r);
        let report = r.join("../report.json");
        let out = Command::new(env!("CARGO_BIN_EXE_glot"))
            .args(["localize", "--yes", "--no-install", "--provider", "mock"])
            .current_dir(r)
            .env("NO_COLOR", "1")
            .env("GLOT_REPORT", &report)
            .output()
            .unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{log}");
        let after = files(r);
        let catalog = |p: &str| {
            p.starts_with("src/app/landing/messages/")
                || p == "glot.lock"
                || p.starts_with(".glot/")
        };
        for (path, text) in &after {
            if !catalog(path) {
                assert_eq!(
                    before.get(path),
                    Some(text),
                    "{path} changed or was added:\n{log}"
                );
            }
        }
        for (path, text) in &before {
            if !catalog(path) {
                assert_eq!(after.get(path), Some(text), "{path} removed:\n{log}");
            }
        }
        assert!(
            !log.contains("npm install"),
            "no library to install:\n{log}"
        );
        let fr: serde_json::Value =
            serde_json::from_str(&after["src/app/landing/messages/fr.json"]).unwrap();
        assert!(
            fr["hero.h1"].is_string(),
            "catalogs still translated:\n{log}"
        );
        // What was found in the landing is listed for the PR, with the catalog row to add.
        let rep: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        let left = rep["left"].as_array().unwrap();
        assert!(
            left.iter()
                .any(|l| l["path"] == "src/app/landing/Landing.tsx"
                    && l["text"] == "Book a demo"
                    && l["suggestion"]
                        .as_str()
                        .unwrap()
                        .contains("\"Book a demo\"")),
            "{rep}"
        );
        assert!(
            left.iter()
                .all(|l| l["path"].as_str().unwrap().starts_with("src/app/landing/")),
            "only the scope is scanned: {rep}"
        );
        assert_eq!(rep["files_rewritten"], 0);
        let _ = std::fs::remove_file(&report);
    }
}
