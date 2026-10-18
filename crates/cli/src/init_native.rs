//! `glot init` detection of mobile / gettext / Rails catalogs (`glot_core::formats`).
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{Detected, is_locale};

const SKIP: &[&str] = &[
    "node_modules",
    ".git",
    "Pods",
    "build",
    "DerivedData",
    ".build",
    ".dart_tool",
    "target",
    "vendor",
    ".gradle",
    "venv",
    ".venv",
];

/// Files under `root` (depth <= 6) matching `pred`, sorted.
fn find(root: &Path, pred: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, pred: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if depth < 6 && !SKIP.contains(&name.as_str()) && !name.starts_with('.') {
                    walk(&p, depth + 1, pred, out);
                }
            } else if pred(&p) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    walk(root, 0, pred, &mut out);
    out.sort();
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn ext(p: &Path, e: &str) -> bool {
    p.extension().is_some_and(|x| x == e)
}

/// Android qualifier => BCP 47 (`pt-rBR` => `pt-BR`, `b+sr+Latn` => `sr-Latn`).
fn from_qualifier(q: &str) -> Option<String> {
    let tag = match q.strip_prefix("b+") {
        Some(b) => b.replace('+', "-"),
        None => q.replacen("-r", "-", 1),
    };
    is_locale(&tag).then_some(tag)
}

pub fn detect(root: &Path) -> Option<Detected> {
    let d = |stack, syntax, files: Vec<String>, locales: BTreeSet<String>| {
        Some(Detected {
            stack,
            syntax,
            files,
            locales,
        })
    };
    // Apple String Catalog: locales are the file's localizations.
    let xc = find(root, &|p| ext(p, "xcstrings"));
    if let Some(first) = xc.first() {
        let mut locales = BTreeSet::new();
        if let Ok(v) = std::fs::read_to_string(first)
            .map_err(|_| ())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).map_err(|_| ()))
        {
            locales.extend(v["sourceLanguage"].as_str().map(str::to_string));
            for e in v["strings"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                locales.extend(
                    e["localizations"]
                        .as_object()
                        .into_iter()
                        .flat_map(|l| l.keys().cloned()),
                );
            }
        }
        return d(
            "xcode",
            "printf",
            xc.iter().map(|p| rel(root, p)).collect(),
            locales,
        );
    }
    // Legacy `xx.lproj/Localizable.strings` (+ `.stringsdict`).
    let lproj = find(root, &|p| {
        (ext(p, "strings") || ext(p, "stringsdict"))
            && p.parent()
                .and_then(|d| d.file_name())
                .is_some_and(|n| n.to_string_lossy().ends_with(".lproj"))
    });
    if !lproj.is_empty() {
        let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
        for p in &lproj {
            let dir = p.parent().unwrap();
            let loc = dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches(".lproj")
                .to_string();
            if !is_locale(&loc) {
                continue;
            }
            let base = rel(root, dir.parent().unwrap());
            let file = p.file_name().unwrap().to_string_lossy().to_string();
            let prefix = if base.is_empty() {
                String::new()
            } else {
                format!("{base}/")
            };
            files.insert(format!("{prefix}{{locale}}.lproj/{file}"));
            locales.insert(loc);
        }
        if !files.is_empty() {
            return d("ios", "printf", files.into_iter().collect(), locales);
        }
    }
    // Android: `res/values/strings.xml` is the source, `values-<qualifier>` the targets.
    let android = find(root, &|p| {
        p.file_name().is_some_and(|n| n == "strings.xml")
            && p.parent()
                .and_then(|d| d.file_name())
                .is_some_and(|n| n == "values")
            && rel(root, p).contains("res/")
    });
    if let Some(src) = android.first() {
        let res = src.parent().unwrap().parent().unwrap();
        let mut locales: BTreeSet<String> = ["en".to_string()].into();
        for e in std::fs::read_dir(res).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(loc) = name.strip_prefix("values-").and_then(from_qualifier)
                && e.path().join("strings.xml").is_file()
            {
                locales.insert(loc);
            }
        }
        return d(
            "android",
            "printf",
            vec![format!("{}/values-{{locale}}/strings.xml", rel(root, res))],
            locales,
        );
    }
    // Flutter gen-l10n: `l10n.yaml` (arb-dir, template-arb-file) next to `pubspec.yaml`.
    if root.join("pubspec.yaml").is_file() {
        let cfg = std::fs::read_to_string(root.join("l10n.yaml")).unwrap_or_default();
        let get = |k: &str| {
            cfg.lines().find_map(|l| {
                l.trim()
                    .strip_prefix(k)?
                    .trim()
                    .strip_prefix(':')
                    .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
            })
        };
        let dir = get("arb-dir").unwrap_or_else(|| "lib/l10n".into());
        let template = get("template-arb-file").unwrap_or_else(|| "app_en.arb".into());
        let prefix = template
            .rsplit_once('_')
            .map_or("app".to_string(), |(p, _)| p.to_string());
        let mut locales = BTreeSet::new();
        for e in std::fs::read_dir(root.join(&dir))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(loc) = name
                .strip_prefix(&format!("{prefix}_"))
                .and_then(|n| n.strip_suffix(".arb"))
                .filter(|l| is_locale(l))
            {
                locales.insert(loc.to_string());
            }
        }
        if !locales.is_empty() || root.join("l10n.yaml").is_file() {
            return d(
                "flutter",
                "icu",
                vec![format!("{dir}/{prefix}_{{locale}}.arb")],
                locales,
            );
        }
    }
    // gettext: `<dir>/<locale>/LC_MESSAGES/<domain>.po` or `<dir>/<locale>.po`.
    let po = find(root, &|p| ext(p, "po"));
    if !po.is_empty() {
        let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
        for p in &po {
            let file = p.file_name().unwrap().to_string_lossy().to_string();
            let parent = p.parent().unwrap();
            if parent.file_name().is_some_and(|n| n == "LC_MESSAGES") {
                let ldir = parent.parent().unwrap();
                let loc = ldir.file_name().unwrap().to_string_lossy().to_string();
                if is_locale(&loc) {
                    files.insert(format!(
                        "{}/{{locale}}/LC_MESSAGES/{file}",
                        rel(root, ldir.parent().unwrap())
                    ));
                    locales.insert(loc);
                }
            } else if let Some(loc) = file.strip_suffix(".po").filter(|l| is_locale(l)) {
                let base = rel(root, parent);
                let prefix = if base.is_empty() {
                    String::new()
                } else {
                    format!("{base}/")
                };
                files.insert(format!("{prefix}{{locale}}.po"));
                locales.insert(loc.to_string());
            }
        }
        if !files.is_empty() {
            return d("gettext", "printf", files.into_iter().collect(), locales);
        }
    }
    // Rails: `config/locales/<locale>.yml` (root key = locale).
    let mut locales = BTreeSet::new();
    for e in std::fs::read_dir(root.join("config/locales"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(loc) = name.strip_suffix(".yml").filter(|l| is_locale(l)) {
            locales.insert(loc.to_string());
        }
    }
    if !locales.is_empty() {
        return d(
            "rails",
            "rails",
            vec!["config/locales/{locale}.yml".into()],
            locales,
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn detects_each_stack() {
        let cases: &[(&[(&str, &str)], &str, &str, &[&str], &[&str])] = &[
            (
                &[(
                    "App/Localizable.xcstrings",
                    r#"{"sourceLanguage":"en","strings":{"Hi":{"localizations":{"fr":{}}}},"version":"1.0"}"#,
                )],
                "xcode",
                "printf",
                &["App/Localizable.xcstrings"],
                &["en", "fr"],
            ),
            (
                &[
                    ("ios/en.lproj/Localizable.strings", ""),
                    ("ios/en.lproj/Localizable.stringsdict", ""),
                    ("ios/de.lproj/Localizable.strings", ""),
                ],
                "ios",
                "printf",
                &[
                    "ios/{locale}.lproj/Localizable.strings",
                    "ios/{locale}.lproj/Localizable.stringsdict",
                ],
                &["de", "en"],
            ),
            (
                &[
                    ("android/app/src/main/res/values/strings.xml", ""),
                    ("android/app/src/main/res/values-pt-rBR/strings.xml", ""),
                    ("android/app/src/main/res/values-b+sr+Latn/strings.xml", ""),
                    ("android/app/src/main/res/values-night/colors.xml", ""),
                ],
                "android",
                "printf",
                &["android/app/src/main/res/values-{locale}/strings.xml"],
                &["en", "pt-BR", "sr-Latn"],
            ),
            (
                &[
                    ("pubspec.yaml", ""),
                    (
                        "l10n.yaml",
                        "arb-dir: lib/l10n\ntemplate-arb-file: app_en.arb\n",
                    ),
                    ("lib/l10n/app_en.arb", "{}"),
                    ("lib/l10n/app_fr.arb", "{}"),
                ],
                "flutter",
                "icu",
                &["lib/l10n/app_{locale}.arb"],
                &["en", "fr"],
            ),
            (
                &[
                    ("locale/fr/LC_MESSAGES/django.po", ""),
                    ("locale/en/LC_MESSAGES/django.po", ""),
                ],
                "gettext",
                "printf",
                &["locale/{locale}/LC_MESSAGES/django.po"],
                &["en", "fr"],
            ),
            (
                &[
                    ("config/locales/en.yml", "en:\n"),
                    ("config/locales/es.yml", "es:\n"),
                ],
                "rails",
                "rails",
                &["config/locales/{locale}.yml"],
                &["en", "es"],
            ),
        ];
        for (files, stack, syntax, patterns, locales) in cases {
            let dir = tempfile::tempdir().unwrap();
            for (f, body) in *files {
                touch(dir.path(), f, body);
            }
            let d = detect(dir.path()).unwrap_or_else(|| panic!("{stack} not detected"));
            assert_eq!((d.stack, d.syntax), (*stack, *syntax));
            assert_eq!(d.files, *patterns, "{stack}");
            assert_eq!(
                d.locales.iter().map(String::as_str).collect::<Vec<_>>(),
                *locales,
                "{stack}"
            );
        }
        assert!(detect(tempfile::tempdir().unwrap().path()).is_none());
    }
}
