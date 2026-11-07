//! `glot init` for Rust apps (Leptos, Yew, Dioxus, Tauri backends, CLIs), by what the crates use:
//! leptos_i18n (`[package.metadata.leptos-i18n]`, `locales/{locale}.json`), Fluent `.ftl`
//! (fluent-templates, i18n-embed, dioxus-i18n, fluent-rs), rust-i18n (`i18n!("locales")`,
//! `_version: 1|2` YAML), else hand-rolled `static DICT: &[(&str, &str, &str)]` tables (cliponit).
//! gettext-rs `.po` falls through to the gettext detection.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::native::{find, rel, webext};
use super::{Detected, is_locale};
use glot_core::formats::rust_dict;

fn manifests(root: &Path) -> Vec<(PathBuf, String)> {
    find(root, &|p| p.file_name().is_some_and(|n| n == "Cargo.toml"))
        .into_iter()
        .filter_map(|p| Some((p.parent()?.to_path_buf(), std::fs::read_to_string(&p).ok()?)))
        .collect()
}

/// `name = …`, `name.workspace = true` or `[dependencies.name]` (`-` and `_` are the same crate).
fn has_dep(toml: &str, name: &str) -> bool {
    let norm = |s: &str| s.replace('-', "_");
    let name = norm(name);
    toml.lines().any(|l| {
        let l = norm(l.trim());
        l.strip_prefix(&name)
            .is_some_and(|r| r.trim_start().starts_with(['=', '.']))
            || l.ends_with(&format!("dependencies.{name}]"))
    })
}

fn prefix(root: &Path, dir: &Path) -> String {
    let r = rel(root, dir);
    let r = r.trim_start_matches("./").trim_end_matches('/');
    if r.is_empty() {
        String::new()
    } else {
        format!("{r}/")
    }
}

fn rs_files(root: &Path) -> Vec<PathBuf> {
    find(root, &|p| p.extension().is_some_and(|e| e == "rs"))
}

/// `key = "value"` in the `[section]` of a TOML text.
fn toml_get(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = section.is_empty();
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == format!("[{section}]");
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches(['"', '\'']).to_string()
}

pub fn detect(root: &Path) -> Option<Detected> {
    let ms = manifests(root);
    if ms.is_empty() {
        return None;
    }
    let dep = |n: &str| ms.iter().any(|(_, t)| has_dep(t, n));
    let mut d = leptos_i18n(root, &ms)
        .or_else(|| fluent(root, &dep))
        .or_else(|| dep("rust-i18n").then(|| rust_i18n(root, &ms)).flatten())
        .or_else(|| dict(root))?;
    // A browser extension next to the app (cliponit): one config, `auto` reads both.
    if let Some(w) = webext(root) {
        d.files.extend(w.files);
        d.locales.extend(w.locales);
    }
    Some(d)
}

/// A stack detected here.
pub fn is_rust(stack: &str) -> bool {
    matches!(
        stack,
        "leptos_i18n"
            | "fluent"
            | "fluent-templates"
            | "i18n-embed"
            | "dioxus-i18n"
            | "rust-i18n"
            | "rust (string tables)"
    )
}

/// `[extract] include` globs for the crates' sources.
pub fn includes(root: &Path) -> Vec<String> {
    manifests(root)
        .iter()
        .filter(|(dir, _)| dir.join("src").is_dir())
        .map(|(dir, _)| format!("{}src/**/*.rs", prefix(root, dir)))
        .collect()
}

/// The source locale the app declares, when it does.
pub fn source(root: &Path) -> Option<String> {
    let ms = manifests(root);
    if ms.is_empty() {
        return None;
    }
    for (dir, t) in &ms {
        if let Some(v) = toml_get(t, "package.metadata.leptos-i18n", "default") {
            return Some(unquote(&v));
        }
        let i18n = std::fs::read_to_string(dir.join("i18n.toml")).unwrap_or_default();
        if let Some(v) = toml_get(&i18n, "", "fallback_language") {
            return Some(unquote(&v));
        }
    }
    let rs = rs_files(root);
    // The first locale column of the biggest table.
    let table = rs
        .iter()
        .filter_map(|p| rust_dict::tables(&std::fs::read_to_string(p).ok()?).ok())
        .flatten()
        .filter(|t| !t.locales.is_empty() && t.is_keyed())
        .max_by_key(|t| t.keys().count())
        .map(|t| t.locales[0].clone());
    if table.is_some() {
        return table;
    }
    for p in &rs {
        let text = std::fs::read_to_string(p).unwrap_or_default();
        // fluent-templates `static_loader!`, rust-i18n `i18n!("locales", fallback = "en")`,
        // leptos_i18n_build `Config::new("en")`.
        for pat in ["fallback_language:", "fallback =", "Config::new("] {
            if let Some(at) = text.find(pat) {
                let v = unquote(text[at + pat.len()..].split([',', ')', '}', '\n']).next()?);
                if is_locale(&v) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn leptos_i18n(root: &Path, ms: &[(PathBuf, String)]) -> Option<Detected> {
    let (dir, toml) = ms.iter().find(|(_, t)| has_dep(t, "leptos_i18n"))?;
    let sec = "package.metadata.leptos-i18n";
    let list = |k: &str| -> Vec<String> {
        toml_get(toml, sec, k)
            .map(|v| {
                v.trim_matches(['[', ']'])
                    .split(',')
                    .map(unquote)
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let ldir = dir.join(
        toml_get(toml, sec, "locales-dir")
            .map(|v| unquote(&v))
            .unwrap_or_else(|| "locales".into()),
    );
    let mut locales: BTreeSet<String> = list("locales").into_iter().collect();
    locales.extend(toml_get(toml, sec, "default").map(|v| unquote(&v)));
    // Without metadata (leptos_i18n_build in build.rs): the locale files themselves.
    let entries: Vec<PathBuf> = std::fs::read_dir(&ldir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().to_string());
    if locales.is_empty() {
        locales.extend(
            entries
                .iter()
                .filter_map(|p| stem(p))
                .filter(|l| is_locale(l)),
        );
    }
    let ext = entries
        .iter()
        .filter_map(|p| p.extension()?.to_str().map(str::to_string))
        .find(|e| matches!(e.as_str(), "json" | "yaml" | "yml" | "ftl"))
        .unwrap_or_else(|| "json".into());
    let base = prefix(root, &ldir);
    let namespaces = list("namespaces");
    let files = if namespaces.is_empty() {
        vec![format!("{base}{{locale}}.{ext}")]
    } else {
        namespaces
            .iter()
            .map(|ns| format!("{base}{{locale}}/{ns}.{ext}"))
            .collect()
    };
    let syntax = if ext == "ftl" { "fluent" } else { "i18next" };
    Some(Detected {
        stack: "leptos_i18n",
        syntax,
        files,
        locales,
    })
}

fn fluent(root: &Path, dep: &dyn Fn(&str) -> bool) -> Option<Detected> {
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for p in find(root, &|p| p.extension().is_some_and(|e| e == "ftl")) {
        let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else {
            continue;
        };
        let name = name.to_string_lossy();
        let dir_name = parent.file_name().map(|n| n.to_string_lossy().to_string());
        if let Some(loc) = dir_name.filter(|l| is_locale(l)) {
            // `locales/en-US/main.ftl`: one pattern per file name.
            files.insert(format!(
                "{}{{locale}}/{name}",
                prefix(root, parent.parent().unwrap_or(root))
            ));
            locales.insert(loc);
        } else if let Some(loc) = name.strip_suffix(".ftl").filter(|l| is_locale(l)) {
            files.insert(format!("{}{{locale}}.ftl", prefix(root, parent)));
            locales.insert(loc.to_string());
        }
    }
    if files.is_empty() {
        return None;
    }
    let stack = ["fluent-templates", "i18n-embed", "dioxus-i18n", "fluent"]
        .into_iter()
        .find(|n| dep(n))
        .unwrap_or("fluent");
    Some(Detected {
        stack,
        syntax: "fluent",
        files: files.into_iter().collect(),
        locales,
    })
}

fn rust_i18n(root: &Path, ms: &[(PathBuf, String)]) -> Option<Detected> {
    let (dir, _) = ms.iter().find(|(_, t)| has_dep(t, "rust-i18n"))?;
    // `i18n!("path", …)`, relative to the crate; `i18n!()` = `locales`.
    let sub = find(&dir.join("src"), &|p| {
        p.extension().is_some_and(|e| e == "rs")
    })
    .into_iter()
    .filter_map(|p| std::fs::read_to_string(p).ok())
    .find_map(|t| {
        let at = t.find("i18n!(")?;
        let arg = t[at + 6..].split([',', ')']).next()?.trim();
        Some(if arg.starts_with('"') {
            unquote(arg)
        } else {
            "locales".into()
        })
    })
    .unwrap_or_else(|| "locales".into());
    let ldir = dir.join(sub);
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for p in find(&ldir, &|p| {
        p.extension()
            .is_some_and(|e| matches!(e.to_str(), Some("yml" | "yaml" | "json" | "toml")))
    }) {
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let stem = p
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let ext = p
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let base = prefix(root, p.parent().unwrap_or(&ldir));
        if text.lines().any(|l| l.trim() == "_version: 2") {
            // Every locale in one file: the locale leaves.
            files.insert(rel(root, &p));
            locales.extend(text.lines().filter_map(|l| {
                let k = l.strip_prefix("  ")?.trim_start().split_once(':')?.0.trim();
                is_locale(k).then(|| k.to_string())
            }));
        } else if is_locale(&stem) {
            files.insert(format!("{base}{{locale}}.{ext}"));
            locales.insert(stem);
        }
    }
    (!files.is_empty()).then(|| Detected {
        stack: "rust-i18n",
        syntax: "rails",
        files: files.into_iter().collect(),
        locales,
    })
}

/// Hand-rolled `static DICT: &[(&str, &str, &str)] = &[("key", "fr", "en"), …]` tables.
fn dict(root: &Path) -> Option<Detected> {
    let (mut files, mut locales) = (vec![], BTreeSet::new());
    for p in rs_files(root) {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        if !text.contains("&[(&") {
            continue;
        }
        let Ok(tables) = rust_dict::tables(&text) else {
            continue;
        };
        let keyed = |t: &rust_dict::Table| t.keys().count() >= 3 && t.is_keyed();
        let mut hit = false;
        for t in tables.iter().filter(|t| keyed(t)) {
            if t.locales.is_empty() {
                eprintln!(
                    "note: {}: table `{}` looks like translations but its columns are not named; add `// (key, fr, en)` right above it and run glot init again.",
                    rel(root, &p),
                    t.name
                );
                continue;
            }
            hit = true;
            locales.extend(t.locales.iter().cloned());
        }
        if hit {
            files.push(rel(root, &p));
        }
    }
    if files.is_empty() {
        return None;
    }
    // New keys go to the first file: the biggest dictionary (`src/i18n.rs`).
    files.sort_by_key(|f| {
        std::cmp::Reverse(
            std::fs::read_to_string(root.join(f))
                .map(|t| t.len())
                .unwrap_or(0),
        )
    });
    Some(Detected {
        stack: "rust (string tables)",
        // `tn(key, n)` is `t(key).replace("{n}", …)`: no ICU, braces and `<…>` are text.
        syntax: "plain",
        files,
        locales,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glot-init-rust-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (p, c) in files {
            let p = d.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        d
    }

    #[test]
    fn cliponit_tables() {
        let d = tmp(
            "dict",
            &[
                ("Cargo.toml", "[package]\nname = \"api\"\n"),
                (
                    "web/Cargo.toml",
                    "[package]\nname = \"web\"\n[dependencies]\nleptos = \"0.8\"\n",
                ),
                (
                    "web/src/i18n.rs",
                    "// (clé, fr, en)\nstatic DICT: &[(&str, &str, &str)] = &[\n    (\"a\", \"A\", \"A\"),\n    (\"b\", \"B\", \"B\"),\n    (\"c\", \"C\", \"C\"),\n    (\"d\", \"D\", \"D\"),\n    (\"e\", \"E\", \"E\"),\n    (\"f\", \"F\", \"F\"),\n];\n",
                ),
                (
                    "web/src/pages/tools.rs",
                    "static DICT: &[(&str, &str, &str)] = &[(\"x\", \"X\", \"X\"), (\"y\", \"Y\", \"Y\"), (\"z\", \"Z\", \"Z\")];\nfn t() { DICT.iter().map(|(_, fr, en)| 0); }\n",
                ),
                (
                    "web/src/pages/styles.rs",
                    "static S: &[(&str, &str, &str)] = &[(\"Bold title\", \"x\", \"y\"), (\"b\", \"\", \"\"), (\"c\", \"\", \"\")];\n",
                ),
                (
                    "extension/manifest.json",
                    "{\"default_locale\": \"en\", \"manifest_version\": 3}",
                ),
                ("extension/_locales/en/messages.json", "{}"),
            ],
        );
        let det = detect(&d).unwrap();
        assert_eq!(det.stack, "rust (string tables)");
        assert_eq!(
            det.files[..2],
            ["web/src/i18n.rs", "web/src/pages/tools.rs"]
        );
        assert!(
            det.files.iter().any(|f| f.contains("_locales/{locale}")),
            "{:?}",
            det.files
        );
        assert_eq!(det.locales, ["en", "fr"].map(String::from).into());
        assert_eq!(source(&d).as_deref(), Some("fr"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn fluent_templates_and_i18n_embed() {
        let d = tmp(
            "ftl",
            &[
                (
                    "Cargo.toml",
                    "[dependencies]\nfluent-templates = \"0.13\"\n",
                ),
                ("locales/en-US/main.ftl", "hello = Hello\n"),
                ("locales/fr/main.ftl", "hello = Bonjour\n"),
                ("locales/fr/errors.ftl", "e = E\n"),
                (
                    "src/main.rs",
                    "static_loader! { static LOCALES = { locales: \"./locales\", fallback_language: \"en-US\", }; }\n",
                ),
            ],
        );
        let det = detect(&d).unwrap();
        assert_eq!((det.stack, det.syntax), ("fluent-templates", "fluent"));
        assert_eq!(
            det.files,
            ["locales/{locale}/errors.ftl", "locales/{locale}/main.ftl"]
        );
        assert_eq!(det.locales, ["en-US", "fr"].map(String::from).into());
        assert_eq!(source(&d).as_deref(), Some("en-US"));
        std::fs::remove_dir_all(&d).unwrap();
        let d = tmp(
            "embed",
            &[
                (
                    "Cargo.toml",
                    "[dependencies.i18n-embed]\nversion = \"0.15\"\n",
                ),
                (
                    "i18n.toml",
                    "fallback_language = \"en\"\n[fluent]\nassets_dir = \"i18n\"\n",
                ),
                ("i18n/en/app.ftl", "a = A\n"),
                ("i18n/de/app.ftl", "a = A\n"),
            ],
        );
        let det = detect(&d).unwrap();
        assert_eq!(det.stack, "i18n-embed");
        assert_eq!(det.files, ["i18n/{locale}/app.ftl"]);
        assert_eq!(source(&d).as_deref(), Some("en"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn leptos_i18n_and_rust_i18n() {
        let d = tmp(
            "leptos",
            &[
                (
                    "Cargo.toml",
                    "[dependencies]\nleptos_i18n = \"0.5\"\n\n[package.metadata.leptos-i18n]\ndefault = \"fr\"\nlocales = [\"fr\", \"en\"]\n",
                ),
                ("locales/en.json", "{}"),
                ("locales/fr.json", "{}"),
            ],
        );
        let det = detect(&d).unwrap();
        assert_eq!((det.stack, det.syntax), ("leptos_i18n", "i18next"));
        assert_eq!(det.files, ["locales/{locale}.json"]);
        assert_eq!(source(&d).as_deref(), Some("fr"));
        std::fs::remove_dir_all(&d).unwrap();
        let d = tmp(
            "ri18n",
            &[
                ("Cargo.toml", "[dependencies]\nrust-i18n = \"3\"\n"),
                (
                    "src/main.rs",
                    "rust_i18n::i18n!(\"i18n\", fallback = \"en\");\n",
                ),
                (
                    "i18n/app.yml",
                    "_version: 2\nhello:\n  en: Hello\n  zh-CN: Ni hao\n",
                ),
            ],
        );
        let det = detect(&d).unwrap();
        assert_eq!((det.stack, det.syntax), ("rust-i18n", "rails"));
        assert_eq!(det.files, ["i18n/app.yml"]);
        assert_eq!(det.locales, ["en", "zh-CN"].map(String::from).into());
        assert_eq!(source(&d).as_deref(), Some("en"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
