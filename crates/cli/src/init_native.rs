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

/// Apple stacks: `[extract]` reads Swift (tests excluded), keys looked up through SwiftGen-style `L10n.tr`.
pub fn swift_extract(
    stack: &str,
    ex: &mut glot_core::config::ExtractConfig,
) -> Option<Vec<String>> {
    if !matches!(stack, "ios" | "xcode") {
        return None;
    }
    ex.exclude = vec!["**/*Tests/**".into(), "**/Preview Content/**".into()];
    ex.functions = vec!["L10n.tr".into()];
    Some(vec!["**/*.swift".into()])
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
    if let Some(d) = webext(root) {
        return Some(d);
    }
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
        let (mut files, mut locales, mut legacy) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for p in &lproj {
            let dir = p.parent().unwrap();
            let loc = dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches(".lproj")
                .to_string();
            if !is_locale(&loc) {
                // Pre-2010 Xcode names: glot needs ISO codes in the path.
                if loc != "Base" && loc.starts_with(|c: char| c.is_ascii_uppercase()) {
                    legacy.insert(loc);
                }
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
            // New keys go to the first file: `Localizable.*` (the default table), never `InfoPlist.strings`.
            let mut files: Vec<String> = files.into_iter().collect();
            files.sort_by_key(|f| {
                (
                    !f.contains("/Localizable."),
                    f.contains("/InfoPlist."),
                    f.clone(),
                )
            });
            return d("ios", "printf", files, locales);
        }
        if !legacy.is_empty() {
            eprintln!(
                "Found {} folders: rename them to ISO codes (English.lproj => en.lproj, French.lproj => fr.lproj) in Xcode, then run glot init again.",
                legacy
                    .iter()
                    .map(|l| format!("{l}.lproj"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
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
    if let Some(d) = flutter_json(root) {
        return Some(d);
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
            } else if let Some(loc) = parent
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|l| is_locale(l))
            {
                // Lingui: `<dir>/<locale>/messages.po`.
                let base = rel(root, parent.parent().unwrap());
                let prefix = if base.is_empty() {
                    String::new()
                } else {
                    format!("{base}/")
                };
                files.insert(format!("{prefix}{{locale}}/{file}"));
                locales.insert(loc);
            }
        }
        if !files.is_empty() {
            // msgids are the source text (English by convention) even without an `en.po`.
            locales.insert("en".into());
            return d("gettext", "printf", files.into_iter().collect(), locales);
        }
    }
    if let Some(d) = symfony(root) {
        return Some(d);
    }
    if let Some(d) = backend(root) {
        return Some(d);
    }
    // Rails: `config/locales/**/{locale}.yml` and `*.{locale}.yml` (`devise.en.yml`), one pattern
    // per file so a key new to a target lands in the mirror of its source file.
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    let base = root.join("config/locales");
    for p in find(&base, &|p| ext(p, "yml") || ext(p, "yaml")) {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let (stem, e) = name.rsplit_once('.').unwrap();
        let (pre, loc) = match stem.rsplit_once('.') {
            Some((pre, l)) if is_locale(l) => (format!("{pre}."), l),
            _ if is_locale(stem) => (String::new(), stem),
            _ => continue,
        };
        let dir = with_dir("config/locales", &rel(&base, p.parent().unwrap()));
        let dir = dir.trim_end_matches('/');
        files.insert(format!("{dir}/{pre}{{locale}}.{e}"));
        locales.insert(loc.to_string());
    }
    if !locales.is_empty() {
        // New keys without a home go to the first file: the main `{locale}.yml`.
        let mut files: Vec<String> = files.into_iter().collect();
        files.sort_by_key(|f| {
            (
                !f.ends_with("/{locale}.yml"),
                f.matches('/').count(),
                f.clone(),
            )
        });
        return d("rails", "rails", files, locales);
    }
    None
}

/// WebExtensions: `_locales/{locale}/messages.json`, source = `default_locale` of `manifest.json`.
fn webext(root: &Path) -> Option<Detected> {
    let msgs = find(root, &|p| {
        p.file_name().is_some_and(|n| n == "messages.json")
            && p.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .is_some_and(|n| n == "_locales")
    });
    let dir = msgs.first()?.parent()?.parent()?;
    let mut locales: BTreeSet<String> = msgs
        .iter()
        .filter(|p| p.parent().and_then(Path::parent) == Some(dir))
        .filter_map(|p| Some(p.parent()?.file_name()?.to_string_lossy().to_string()))
        .filter(|l| is_locale(l))
        .collect();
    let manifest = std::fs::read_to_string(dir.parent()?.join("manifest.json")).unwrap_or_default();
    if let Some(l) = serde_json::from_str::<serde_json::Value>(&manifest)
        .ok()
        .and_then(|m| m["default_locale"].as_str().map(str::to_string))
    {
        locales.insert(l);
    }
    Some(Detected {
        stack: "webextension",
        syntax: "icu",
        files: vec![with_dir(&rel(root, dir), "{locale}/messages.json")],
        locales,
    })
}

/// Flutter JSON libraries (plurals as `{"one": …, "other": …}` sub-objects, `{}` / `{name}` args):
/// easy_localization `assets/translations/{locale}.json`, slang `lib/i18n/{locale}.i18n.json`.
fn flutter_json(root: &Path) -> Option<Detected> {
    let pubspec = std::fs::read_to_string(root.join("pubspec.yaml")).ok()?;
    let (stack, dirs, suffix): (_, &[&str], _) = if pubspec.contains("easy_localization") {
        (
            "easy_localization",
            &[
                "assets/translations",
                "assets/i18n",
                "assets/lang",
                "assets/l10n",
            ],
            ".json",
        )
    } else if pubspec.contains("slang") {
        (
            "slang",
            &["lib/i18n", "assets/i18n", "lib/l10n"],
            ".i18n.json",
        )
    } else {
        return None;
    };
    for dir in dirs {
        let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
        for e in read_dir(&root.join(dir)) {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(suffix) else {
                continue;
            };
            // slang `strings_de.i18n.json` / `de.i18n.json`.
            let (pre, loc) = match stem.rsplit_once('_') {
                Some((pre, l)) if is_locale(l) => (format!("{pre}_"), l),
                _ => (String::new(), stem),
            };
            if is_locale(loc) {
                files.insert(format!("{dir}/{pre}{{locale}}{suffix}"));
                locales.insert(loc.to_string());
            }
        }
        if !files.is_empty() {
            return Some(Detected {
                stack,
                syntax: "printf",
                files: files.into_iter().collect(),
                locales,
            });
        }
    }
    None
}

/// Symfony: `translations/<domain>[+intl-icu].<locale>.<yaml|xlf|…>` (`default_path` of
/// `config/packages/translation.yaml`), `messages` first so new keys land there.
fn symfony(root: &Path) -> Option<Detected> {
    let composer = std::fs::read_to_string(root.join("composer.json")).unwrap_or_default();
    if !composer.contains("\"symfony/") {
        return None;
    }
    let cfg =
        std::fs::read_to_string(root.join("config/packages/translation.yaml")).unwrap_or_default();
    let dir = cfg
        .lines()
        .find_map(|l| l.trim().strip_prefix("default_path:"))
        .map(|v| {
            v.trim()
                .trim_matches(['"', '\''])
                .replace("%kernel.project_dir%/", "")
        })
        .unwrap_or_else(|| "translations".into());
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for e in read_dir(&root.join(&dir)) {
        let name = e.file_name().to_string_lossy().to_string();
        let Some((stem, x)) = name.rsplit_once('.') else {
            continue;
        };
        let Some((domain, loc)) = stem.rsplit_once('.').filter(|(_, l)| is_locale(l)) else {
            continue;
        };
        if glot_core::formats::native(glot_core::catalog::Format::Auto, &e.path()).is_some() {
            files.insert(format!("{dir}/{domain}.{{locale}}.{x}"));
            locales.insert(loc.to_string());
        }
    }
    if files.is_empty() {
        return None;
    }
    // ICU when any domain opts in (`+intl-icu`); legacy `%name%` reads as plain text under ICU.
    let mut files: Vec<String> = files.into_iter().collect();
    files.sort_by_key(|f| {
        (
            !f.contains("/messages"),
            !f.contains("+intl-icu"),
            f.clone(),
        )
    });
    Some(Detected {
        stack: "symfony",
        syntax: "icu",
        files,
        locales,
    })
}

fn read_dir(p: &Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .collect()
}

fn with_dir(base: &str, file: &str) -> String {
    if base.is_empty() {
        file.to_string()
    } else {
        format!("{base}/{file}")
    }
}

/// Back-end catalogs: Laravel `lang/`, Angular / Symfony XLIFF, Java `.properties`, .NET `.resx`.
fn backend(root: &Path) -> Option<Detected> {
    let composer = std::fs::read_to_string(root.join("composer.json")).unwrap_or_default();
    // Laravel: `lang/{locale}.json` (text keys, first so new keys land there) + `lang/{locale}/*.php`.
    if composer.contains("laravel/framework") {
        for dir in ["lang", "resources/lang"] {
            let (mut json, mut php, mut locales) = (false, BTreeSet::new(), BTreeSet::new());
            for e in read_dir(&root.join(dir)) {
                let (p, name) = (e.path(), e.file_name().to_string_lossy().to_string());
                if p.is_dir() && is_locale(&name) {
                    locales.insert(name);
                    for f in read_dir(&p).iter().filter(|f| ext(&f.path(), "php")) {
                        php.insert(format!(
                            "{dir}/{{locale}}/{}",
                            f.file_name().to_string_lossy()
                        ));
                    }
                } else if let Some(l) = name.strip_suffix(".json").filter(|l| is_locale(l)) {
                    json = true;
                    locales.insert(l.to_string());
                }
            }
            let files: Vec<String> = json
                .then(|| format!("{dir}/{{locale}}.json"))
                .into_iter()
                .chain(php)
                .collect();
            if !files.is_empty() {
                return Some(Detected {
                    stack: "laravel",
                    syntax: "laravel",
                    files,
                    locales,
                });
            }
        }
    }
    // Angular `angular.json`: `i18n.sourceLocale` + `i18n.locales.{l}` (path or `{translation}`).
    let angular = std::fs::read_to_string(root.join("angular.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    for p in angular.iter().flat_map(|v| {
        v["projects"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
    }) {
        let i = &p["i18n"];
        let Some(locs) = i["locales"].as_object().filter(|l| !l.is_empty()) else {
            continue;
        };
        let src = i["sourceLocale"]
            .as_str()
            .or(i["sourceLocale"]["code"].as_str())
            .unwrap_or("en-US");
        let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::from([src.to_string()]));
        for (l, v) in locs {
            let path = v
                .as_str()
                .or(v["translation"].as_str())
                .or(v["translation"][0].as_str());
            if let Some(path) = path.filter(|p| p.contains(&format!(".{l}."))) {
                files.insert(path.replace(&format!(".{l}."), ".{locale}."));
                locales.insert(l.clone());
            }
        }
        if !files.is_empty() {
            return Some(Detected {
                stack: "angular",
                syntax: "icu",
                files: files.into_iter().collect(),
                locales,
            });
        }
    }
    // `<stem>.<locale>.xlf` (+ Angular `<stem>.xlf` source, its `source-language` is the source).
    let xlf = find(root, &|p| ext(p, "xlf") || ext(p, "xliff"));
    let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for p in &xlf {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let e = p.extension().unwrap().to_string_lossy().to_string();
        let base = &name[..name.len() - e.len() - 1];
        if let Some((stem, loc)) = base.rsplit_once('.').filter(|(_, l)| is_locale(l)) {
            files.insert(with_dir(
                &rel(root, p.parent().unwrap()),
                &format!("{stem}.{{locale}}.{e}"),
            ));
            locales.insert(loc.to_string());
            if let Ok(t) = std::fs::read_to_string(p.with_file_name(format!("{stem}.{e}"))) {
                let lang = ["source-language=\"", "srcLang=\""].iter().find_map(|a| {
                    t.split_once(a)
                        .and_then(|(_, r)| r.split_once('"'))
                        .map(|(l, _)| l.to_string())
                });
                locales.extend(lang);
            }
        }
    }
    if !files.is_empty() {
        let stack = if composer.contains("symfony/") {
            "symfony"
        } else if angular.is_some() {
            "angular"
        } else {
            "xliff"
        };
        return Some(Detected {
            stack,
            syntax: "icu",
            files: files.into_iter().collect(),
            locales,
        });
    }
    // Java `messages_<locale>.properties` and .NET `Strings.<locale>.resx` next to their
    // locale-less source file (the default bundle, English by convention).
    for (e, sep, stack, syntax) in [
        ("properties", '_', "java", "icu"),
        ("resx", '.', ".net", "printf"),
    ] {
        let (mut files, mut locales) = (BTreeSet::new(), BTreeSet::new());
        for p in find(root, &|p| ext(p, e)) {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let base = &name[..name.len() - e.len() - 1];
            let hit = base.match_indices(sep).map(|(i, _)| i).find(|&i| {
                is_locale(&base[i + 1..])
                    && p.with_file_name(format!("{}.{e}", &base[..i])).is_file()
            });
            if let Some(i) = hit {
                let pattern = format!("{}{sep}{{locale}}.{e}", &base[..i]);
                files.insert(with_dir(&rel(root, p.parent().unwrap()), &pattern));
                locales.insert(base[i + 1..].to_string());
            }
        }
        if !files.is_empty() {
            locales.insert("en".into());
            let stack = if e == "properties"
                && ["pom.xml", "build.gradle", "build.gradle.kts"]
                    .iter()
                    .any(|f| root.join(f).is_file())
            {
                "spring"
            } else {
                stack
            };
            return Some(Detected {
                stack,
                syntax,
                files: files.into_iter().collect(),
                locales,
            });
        }
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
                // Several tables: new keys must land in Localizable, never InfoPlist.
                &[
                    ("App/en.lproj/Auth.strings", ""),
                    ("App/en.lproj/InfoPlist.strings", ""),
                    ("App/en.lproj/Localizable.strings", ""),
                    ("App/fr.lproj/Localizable.strings", ""),
                    ("App/Base.lproj/Main.strings", ""),
                ],
                "ios",
                "printf",
                &[
                    "App/{locale}.lproj/Localizable.strings",
                    "App/{locale}.lproj/Auth.strings",
                    "App/{locale}.lproj/InfoPlist.strings",
                ],
                &["en", "fr"],
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
                &[("locale/fr/LC_MESSAGES/django.po", "")],
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

    #[test]
    #[allow(clippy::type_complexity)]
    fn detects_backend_catalogs() {
        let cases: &[(&[(&str, &str)], &str, &str, &[&str], &[&str])] = &[
            (
                &[
                    (
                        "composer.json",
                        r#"{"require":{"laravel/framework":"^11.0"}}"#,
                    ),
                    ("lang/en/auth.php", "<?php return [];"),
                    ("lang/fr/auth.php", "<?php return [];"),
                    ("lang/fr/validation.php", "<?php return [];"),
                    ("lang/fr.json", "{}"),
                ],
                "laravel",
                "laravel",
                &[
                    "lang/{locale}.json",
                    "lang/{locale}/auth.php",
                    "lang/{locale}/validation.php",
                ],
                &["en", "fr"],
            ),
            (
                &[
                    (
                        "angular.json",
                        r#"{"projects":{"app":{"i18n":{"sourceLocale":"en-US","locales":{"fr":"src/locale/messages.fr.xlf","de":{"translation":"src/locale/messages.de.xlf"}}}}}}"#,
                    ),
                    ("src/locale/messages.xlf", ""),
                ],
                "angular",
                "icu",
                &["src/locale/messages.{locale}.xlf"],
                &["de", "en-US", "fr"],
            ),
            (
                &[
                    (
                        "composer.json",
                        r#"{"require":{"symfony/translation":"7.*"}}"#,
                    ),
                    ("translations/messages.en.xlf", ""),
                    ("translations/messages.fr.xlf", ""),
                ],
                "symfony",
                "icu",
                &["translations/messages.{locale}.xlf"],
                &["en", "fr"],
            ),
            (
                &[
                    ("pom.xml", ""),
                    ("src/main/resources/messages.properties", ""),
                    ("src/main/resources/messages_fr.properties", ""),
                    ("src/main/resources/messages_pt_BR.properties", ""),
                    ("src/main/resources/application.properties", ""),
                ],
                "spring",
                "icu",
                &["src/main/resources/messages_{locale}.properties"],
                &["en", "fr", "pt_BR"],
            ),
            (
                &[
                    ("App/App.csproj", ""),
                    ("App/Resources/Strings.resx", ""),
                    ("App/Resources/Strings.de.resx", ""),
                    ("App/Form1.resx", ""),
                ],
                ".net",
                "printf",
                &["App/Resources/Strings.{locale}.resx"],
                &["de", "en"],
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
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn detects_json_libs_and_backends() {
        let cases: &[(&[(&str, &str)], &str, &str, &[&str], &[&str])] = &[
            (
                &[
                    (
                        "manifest.json",
                        r#"{"manifest_version":3,"default_locale":"en"}"#,
                    ),
                    ("_locales/en/messages.json", "{}"),
                    ("_locales/pt_BR/messages.json", "{}"),
                ],
                "webextension",
                "icu",
                &["_locales/{locale}/messages.json"],
                &["en", "pt_BR"],
            ),
            (
                &[
                    (
                        "pubspec.yaml",
                        "dependencies:\n  easy_localization: ^3.0.0\n",
                    ),
                    ("assets/translations/en.json", "{}"),
                    ("assets/translations/fr-FR.json", "{}"),
                ],
                "easy_localization",
                "printf",
                &["assets/translations/{locale}.json"],
                &["en", "fr-FR"],
            ),
            (
                &[
                    ("pubspec.yaml", "dependencies:\n  slang: ^4.0.0\n"),
                    ("lib/i18n/strings_en.i18n.json", "{}"),
                    ("lib/i18n/strings_de.i18n.json", "{}"),
                ],
                "slang",
                "printf",
                &["lib/i18n/strings_{locale}.i18n.json"],
                &["de", "en"],
            ),
            (
                &[
                    (
                        "composer.json",
                        r#"{"require":{"symfony/framework-bundle":"7.*"}}"#,
                    ),
                    (
                        "config/packages/translation.yaml",
                        "framework:\n    translator:\n        default_path: '%kernel.project_dir%/i18n'\n",
                    ),
                    ("i18n/validators.en.yaml", ""),
                    ("i18n/messages+intl-icu.en.yaml", ""),
                    ("i18n/messages+intl-icu.fr.yaml", ""),
                    ("i18n/README.md", ""),
                ],
                "symfony",
                "icu",
                &[
                    "i18n/messages+intl-icu.{locale}.yaml",
                    "i18n/validators.{locale}.yaml",
                ],
                &["en", "fr"],
            ),
            (
                // One pattern per file: `devise.ok` new to `fr` goes to `devise.fr.yml`.
                &[
                    ("config/locales/devise.en.yml", "en:\n"),
                    ("config/locales/en.yml", "en:\n"),
                    ("config/locales/models/user.en.yml", "en:\n"),
                    ("config/locales/fr.yml", "fr:\n"),
                ],
                "rails",
                "rails",
                &[
                    "config/locales/{locale}.yml",
                    "config/locales/devise.{locale}.yml",
                    "config/locales/models/user.{locale}.yml",
                ],
                &["en", "fr"],
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
    }
}
