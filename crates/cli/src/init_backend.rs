//! `glot init` for back-end libraries whose catalogs the generic search misses or misnames:
//! go-i18n v2 (`active.{locale}.toml|json|yaml`, Go templates), Node `i18n` (mashpie,
//! `locales/{locale}.json`, `%s` / `{{name}}`), nestjs-i18n (`src/i18n/{locale}/*.json`).
//! Rails, gettext (Django, Flask, Phoenix, PHP), Laravel, Symfony, Spring and .NET are in
//! `init_native.rs`; WordPress in `init_cms.rs`.
use std::collections::BTreeSet;
use std::path::Path;

use super::native::{find, rel};
use super::{Detected, is_locale};

pub fn detect(root: &Path, has: &dyn Fn(&str) -> bool) -> Option<Detected> {
    go_i18n(root)
        .or_else(|| nest(root, has))
        .or_else(|| i18n_node(root, has))
}

/// go-i18n v2: `<stem>.<locale>.<ext>` or `<locale>.<ext>` message files (`translate.*` are
/// `goi18n merge` work files, not catalogs).
fn go_i18n(root: &Path) -> Option<Detected> {
    let go_mod = std::fs::read_to_string(root.join("go.mod")).ok()?;
    if !go_mod.contains("github.com/nicksnyder/go-i18n") {
        return None;
    }
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for p in find(root, &|p| {
        p.extension()
            .is_some_and(|e| ["toml", "json", "yaml", "yml"].contains(&&*e.to_string_lossy()))
    }) {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let (stem, e) = name.rsplit_once('.').unwrap();
        let (pre, loc) = match stem.rsplit_once('.') {
            Some((pre, l)) if is_locale(l) && pre != "translate" => (format!("{pre}."), l),
            None if is_locale(stem) => (String::new(), stem),
            _ => continue,
        };
        let dir = rel(root, p.parent().unwrap());
        let dir = if dir.is_empty() {
            dir
        } else {
            format!("{dir}/")
        };
        files.insert(format!("{dir}{pre}{{locale}}.{e}"));
        locales.insert(loc.to_string());
    }
    // ponytail: no message file yet => the README's layout; go-i18n's default language is `en`.
    let files = if files.is_empty() {
        vec!["active.{locale}.toml".to_string()]
    } else {
        files.into_iter().collect()
    };
    Some(Detected {
        stack: "go-i18n",
        syntax: "template",
        files,
        locales,
    })
}

/// nestjs-i18n: one folder per locale, one JSON file per namespace (`src/i18n/en/common.json`).
fn nest(root: &Path, has: &dyn Fn(&str) -> bool) -> Option<Detected> {
    if !has("nestjs-i18n") {
        return None;
    }
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for base in ["src/i18n", "i18n", "src/locales"] {
        for e in std::fs::read_dir(root.join(base))
            .into_iter()
            .flatten()
            .flatten()
        {
            let loc = e.file_name().to_string_lossy().to_string();
            if !e.path().is_dir() || !is_locale(&loc) {
                continue;
            }
            locales.insert(loc);
            for f in find(&e.path(), &|p| p.extension().is_some_and(|x| x == "json")) {
                files.insert(format!("{base}/{{locale}}/{}", rel(&e.path(), &f)));
            }
        }
        if !files.is_empty() {
            break;
        }
    }
    let files = if files.is_empty() {
        vec!["src/i18n/{locale}/common.json".to_string()]
    } else {
        files.into_iter().collect()
    };
    // `{name}` arguments and `one`/`other` sub-objects: the ICU reading.
    Some(Detected {
        stack: "nestjs-i18n",
        syntax: "icu",
        files,
        locales,
    })
}

/// Node `i18n` (mashpie): `locales/{locale}.json`, sprintf `%s` and mustache `{{name}}`.
fn i18n_node(root: &Path, has: &dyn Fn(&str) -> bool) -> Option<Detected> {
    if !has("i18n") {
        return None;
    }
    let locales: BTreeSet<String> = std::fs::read_dir(root.join("locales"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_suffix(".json")
                .filter(|l| is_locale(l))
                .map(str::to_string)
        })
        .collect();
    Some(Detected {
        stack: "i18n (node)",
        // ponytail: `%s` / `%d` checked; mustache `{{name}}` is kept as text (not validated).
        syntax: "printf",
        files: vec!["locales/{locale}.json".into()],
        locales,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(files: &[(&str, &str)]) -> Option<(String, String, Vec<String>, Vec<String>)> {
        let dir = tempfile::tempdir().unwrap();
        let mut deps = BTreeSet::new();
        for (f, body) in files {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
            if *f == "package.json" {
                let v: serde_json::Value = serde_json::from_str(body).unwrap();
                deps.extend(v["dependencies"].as_object().unwrap().keys().cloned());
            }
        }
        let has = |n: &str| deps.contains(n);
        detect(dir.path(), &has).map(|d| {
            (
                d.stack.to_string(),
                d.syntax.to_string(),
                d.files,
                d.locales.into_iter().collect(),
            )
        })
    }

    #[test]
    fn go_i18n_message_files() {
        let go = (
            "go.mod",
            "module x\nrequire github.com/nicksnyder/go-i18n/v2 v2.7.0\n",
        );
        let d = det(&[
            go,
            ("active.en.toml", ""),
            ("active.fr.toml", ""),
            ("translate.fr.toml", ""),
            ("locales/en.json", "{}"),
            ("locales/es.json", "{}"),
            ("config.yaml", ""),
        ])
        .unwrap();
        assert_eq!((d.0.as_str(), d.1.as_str()), ("go-i18n", "template"));
        assert_eq!(d.2, ["active.{locale}.toml", "locales/{locale}.json"]);
        assert_eq!(d.3, ["en", "es", "fr"]);
        // Fresh project.
        assert_eq!(det(&[go]).unwrap().2, ["active.{locale}.toml"]);
        // Not go-i18n.
        assert!(det(&[("go.mod", "module x\n"), ("active.en.toml", "")]).is_none());
    }

    #[test]
    fn node_libraries() {
        let d = det(&[
            ("package.json", r#"{"dependencies":{"nestjs-i18n":"^10"}}"#),
            ("src/i18n/en/common.json", "{}"),
            ("src/i18n/en/auth/errors.json", "{}"),
            ("src/i18n/fr/common.json", "{}"),
        ])
        .unwrap();
        assert_eq!((d.0.as_str(), d.1.as_str()), ("nestjs-i18n", "icu"));
        assert_eq!(
            d.2,
            [
                "src/i18n/{locale}/auth/errors.json",
                "src/i18n/{locale}/common.json"
            ]
        );
        assert_eq!(d.3, ["en", "fr"]);
        let d = det(&[
            (
                "package.json",
                r#"{"dependencies":{"i18n":"^0.15","express":"^5"}}"#,
            ),
            ("locales/en.json", "{}"),
            ("locales/de.json", "{}"),
        ])
        .unwrap();
        assert_eq!((d.0.as_str(), d.1.as_str()), ("i18n (node)", "printf"));
        assert_eq!(d.2, ["locales/{locale}.json"]);
        assert_eq!(d.3, ["de", "en"]);
    }
}
