//! `glot init`: stack detection, config files and AI-agent integration.
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use crate::style::{BOLD, DIM, GREEN, YELLOW};

#[derive(clap::Args)]
pub struct InitArgs {
    /// Source locale (default: `en` if found, else the first detected locale).
    #[arg(long)]
    source: Option<String>,
    /// Target locales, comma separated (default: other detected locales).
    #[arg(long, value_delimiter = ',')]
    targets: Vec<String>,
    /// LLM provider: anthropic | openai | openai-compatible | gemini (Gemini 3 Flash preset) | mock.
    #[arg(long)]
    provider: Option<String>,
    /// Accept detected defaults without prompting.
    #[arg(short, long)]
    yes: bool,
    /// Overwrite existing files.
    #[arg(long)]
    force: bool,
}

#[path = "init_native.rs"]
mod native;

const MARK_START: &str = "<!-- glot:start -->";
const MARK_END: &str = "<!-- glot:end -->";

#[derive(Debug, PartialEq)]
pub struct Detected {
    pub stack: &'static str,
    pub syntax: &'static str,
    pub files: Vec<String>,
    pub locales: BTreeSet<String>,
}

pub fn detect(root: &Path) -> Detected {
    let deps: BTreeSet<String> = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|pkg| {
            ["dependencies", "devDependencies"]
                .iter()
                .filter_map(|k| pkg.get(k)?.as_object())
                .flat_map(|o| o.keys().cloned())
                .collect()
        })
        .unwrap_or_default();
    let has = |n: &str| {
        deps.iter()
            .any(|d| d == n || d.starts_with(&format!("{n}/")))
    };
    // Mobile / gettext / Rails catalogs, unless a JS i18n library says otherwise.
    let js = [
        "next-intl",
        "i18next",
        "react-i18next",
        "next-i18next",
        "react-intl",
        "@lingui",
        "vue-i18n",
        "@ngx-translate/core",
        "@jsverse/transloco",
        "@ngneat/transloco",
        "@nuxtjs/i18n",
        "i18n-js",
    ];
    if !js.iter().any(|n| has(n)) {
        if let Some(d) = native::detect(root) {
            return d;
        }
        // Laravel before any `lang/` file (`php artisan lang:publish`): text keys in JSON.
        let composer = std::fs::read_to_string(root.join("composer.json")).unwrap_or_default();
        if composer.contains("\"laravel/framework\"") {
            return Detected {
                stack: "laravel",
                syntax: "laravel",
                files: vec!["lang/{locale}.json".into()],
                locales: BTreeSet::new(),
            };
        }
    }
    // Lingui: gettext PO (its default) holding ICU messages.
    if has("@lingui")
        && let Some(d) = native::detect(root).filter(|d| d.stack == "gettext")
    {
        return Detected {
            stack: "lingui",
            syntax: "icu",
            ..d
        };
    }
    let (stack, syntax, default_file) = if has("next-intl") {
        ("next-intl", "icu", "messages/{locale}.json")
    } else if has("react-i18next") || has("i18next") || has("next-i18next") {
        ("i18next", "i18next", "public/locales/{locale}/common.json")
    } else if has("react-intl") {
        ("react-intl", "icu", "lang/{locale}.json")
    } else if has("@lingui") {
        ("lingui", "icu", "src/locales/{locale}/messages.po")
    } else if has("@ngx-translate/core") {
        // `{{ value }}` interpolation: the i18next parser.
        ("ngx-translate", "i18next", "src/assets/i18n/{locale}.json")
    } else if has("@jsverse/transloco") || has("@ngneat/transloco") {
        ("transloco", "i18next", "src/assets/i18n/{locale}.json")
    } else if has("@nuxtjs/i18n") {
        ("nuxt-i18n", "vue", "i18n/locales/{locale}.json")
    } else if has("i18n-js") {
        // `%{name}`, plurals as `{"one": …, "other": …}` sub-objects.
        ("i18n-js", "rails", "src/locales/{locale}.json")
    } else if has("vue-i18n") {
        ("vue-i18n", "vue", "src/locales/{locale}.json")
    } else {
        // No i18n library yet: `glot i18nize` will install react-i18next outside Next.js, whose
        // catalogs use i18next syntax and are bundled from the app's source dir.
        match crate::setup::detect(root).map(|p| (p.framework, p.base)) {
            Some((f, base)) if f.lib() == glot_extract::i18nize::Lib::ReactI18next => {
                let file = if base == "app/" {
                    "app/locales/{locale}.json"
                } else {
                    "src/locales/{locale}.json"
                };
                ("react (no i18n library yet)", "i18next", file)
            }
            // `glot i18nize` installs next-intl, whose convention is `messages/{locale}.json`.
            Some(_) => (
                "next.js (no i18n library yet)",
                "icu",
                "messages/{locale}.json",
            ),
            _ => ("generic", "icu", "locales/{locale}.json"),
        }
    };

    let configured = configured_dirs(root);
    let (mut files, mut locales) = find_in(
        root,
        &configured.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    // i18next: the catalogs its init imports, before guessing directories.
    if files.is_empty() && stack == "i18next" {
        (files, locales) = i18next_imports(root);
    }
    if files.is_empty() {
        (files, locales) = find_catalogs(root);
    }
    let (mut stack, mut syntax) = (stack, syntax);
    if files.is_empty() {
        (files, locales) = find_js_catalogs(root);
        if !files.is_empty() && stack != "i18next" {
            stack = "TS/JS dictionaries";
        }
    }
    // AI-builder pattern: one `{ en: {...}, fr: {...} }` dictionary read by a hand-rolled
    // `useLanguage().t(key)` (in `translations.ts` or inline in `LanguageContext.tsx`).
    if files.is_empty()
        && stack != "i18next"
        && let Some((f, l)) = find_dictionary(root)
    {
        (files, locales) = (vec![f], l);
        stack = if custom_runtime(root).is_some() {
            "custom (LanguageContext)"
        } else {
            "TS/JS dictionaries"
        };
        syntax = "icu";
    }
    if files.is_empty() {
        files.push(default_file.to_string());
    }
    if stack == "i18next" {
        files = ns_patterns(files);
    }
    Detected {
        stack,
        syntax,
        files,
        locales,
    }
}

/// i18next namespaces: `dir/{locale}/common.json` + `dir/{locale}/home.json` => one
/// `dir/{locale}/{ns}.json` pattern (keys `common:title`, `home:title`, no collision).
fn ns_patterns(files: Vec<String>) -> Vec<String> {
    let dir = |f: &str| {
        let (d, name) = f.strip_suffix(".json")?.rsplit_once("{locale}/")?;
        (!name.contains(['/', '{', '*'])).then(|| d.to_string())
    };
    let mut out: Vec<String> = vec![];
    for f in &files {
        let p = match dir(f) {
            Some(d) if files.iter().filter(|g| dir(g).as_ref() == Some(&d)).count() >= 2 => {
                format!("{d}{{locale}}/{{ns}}.json")
            }
            _ => f.clone(),
        };
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// `[i18next]` section from the app's i18next options (`keySeparator`, `nsSeparator`), empty if unset.
fn i18next_section(root: &Path) -> String {
    const FILES: &[&str] = &[
        "i18n.ts",
        "i18n.js",
        "src/i18n.ts",
        "src/i18n.js",
        "src/i18n/index.ts",
        "src/i18n/index.js",
        "src/i18n/config.ts",
        "src/lib/i18n.ts",
        "src/lib/i18n.js",
        "app/i18n.ts",
        "lib/i18n.ts",
        "next-i18next.config.js",
        "next-i18next.config.mjs",
        "next-i18next.config.cjs",
        "next-i18next.config.ts",
        "i18next-parser.config.js",
        "i18next-parser.config.mjs",
        "i18next-parser.config.cjs",
        "i18next-parser.config.ts",
        "i18next-parser.config.json",
        "i18next.config.ts",
        "i18next.config.js",
    ];
    let text: String = FILES
        .iter()
        .filter_map(|f| std::fs::read_to_string(root.join(f)).ok())
        .collect::<Vec<_>>()
        .join("\n");
    // ponytail: first `name: value` occurrence, comments not skipped.
    let find = |name: &str| -> Option<String> {
        let i = text.find(name)?;
        let rest = text[i + name.len()..]
            .trim_start_matches(['"', '\''])
            .trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        if rest.starts_with("false") {
            return Some("false".into());
        }
        let q = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'))?;
        let end = rest[1..].find(q)?;
        Some(serde_json::to_string(&rest[1..1 + end]).expect("string"))
    };
    let mut out = String::new();
    for (js, key) in [
        ("keySeparator", "key_separator"),
        ("nsSeparator", "ns_separator"),
    ] {
        if let Some(v) = find(js) {
            out += &format!("{key} = {v}\n");
        }
    }
    if out.is_empty() {
        out
    } else {
        format!(
            "\n# From the app's i18next options (`false` = keys are never split).\n[i18next]\n{out}"
        )
    }
}

/// Hand-written dictionaries: `src/i18n/{en,fr}.ts`, `lib/i18n/locales/{locale}.ts`,
/// `dictionaries/<namespace>/{locale}.ts` (one pattern per directory), or i18next resources inlined
/// in `src/i18n.{js,ts}`. A directory counts when its path says i18n or it holds 3+ locale files
/// (`id.ts` / `ms.ts` utilities elsewhere are not locales).
fn find_js_catalogs(root: &Path) -> (Vec<String>, BTreeSet<String>) {
    const LANGS: &[&str] = &[
        "en", "fr", "de", "es", "it", "pt", "nl", "ja", "ko", "zh", "ru", "ar", "hi", "th", "vi",
        "tl", "tr", "pl", "sv", "da", "nb", "fi", "cs", "el", "he", "id", "ms", "uk", "ro", "hu",
        "bg", "fa", "bn", "ca",
    ];
    const EXTS: &[&str] = &["ts", "js", "mjs"];
    fn walk(dir: &Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir()
                && depth < 7
                && !name.starts_with('.')
                && ![
                    "node_modules",
                    "dist",
                    "build",
                    "out",
                    "coverage",
                    "public",
                    "target",
                ]
                .contains(&name.as_str())
            {
                out.push(e.path());
                walk(&e.path(), depth + 1, out);
            }
        }
    }
    let mut dirs = vec![];
    walk(root, 0, &mut dirs);
    let mut patterns = BTreeSet::new();
    let mut locales = BTreeSet::new();
    for dir in dirs {
        let rel = dir
            .strip_prefix(root)
            .unwrap_or(&dir)
            .to_string_lossy()
            .replace('\\', "/");
        let mut found: Vec<(String, String)> = vec![];
        for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if let Some((stem, ext)) = name.rsplit_once('.')
                && EXTS.contains(&ext)
                && is_locale(stem)
                && LANGS.contains(&stem.split(['-', '_']).next().unwrap_or(""))
            {
                found.push((stem.to_string(), ext.to_string()));
            }
        }
        let lower = rel.to_ascii_lowercase();
        let named = [
            "i18n",
            "locale",
            "lang",
            "translation",
            "dictionar",
            "messages",
        ]
        .iter()
        .any(|w| lower.contains(w));
        if found.is_empty() || !(named || found.len() >= 3) {
            continue;
        }
        for (stem, ext) in found {
            patterns.insert(format!("{rel}/{{locale}}.{ext}"));
            locales.insert(stem);
        }
    }
    if patterns.is_empty() {
        // i18next resources inlined in the init file.
        for rel in [
            "src/i18n.js",
            "src/i18n.ts",
            "src/i18n/index.js",
            "src/i18n/index.ts",
            "src/i18n/config.ts",
            "src/i18n/config.js",
            "src/lib/i18n.ts",
            "src/lib/i18n.js",
        ] {
            let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
                continue;
            };
            for l in LANGS {
                let parsed = glot_core::formats::parse(
                    glot_core::catalog::Format::Js,
                    Path::new(rel),
                    &text,
                    l,
                );
                if parsed.is_ok_and(|p| !p.entries.is_empty()) {
                    locales.insert(l.to_string());
                }
            }
            if locales.len() >= 2 {
                return (vec![rel.to_string()], locales);
            }
            locales.clear();
        }
    }
    (patterns.into_iter().collect(), locales)
}

/// Catalogs imported by the i18next init (`import en from './locales/en.json'`,
/// `import fr from '@/i18n/translations/fr'`): patterns from the import paths, not guessed dirs.
fn i18next_imports(root: &Path) -> (Vec<String>, BTreeSet<String>) {
    const INITS: &[&str] = &[
        "src/i18n.ts",
        "src/i18n.js",
        "src/i18n/index.ts",
        "src/i18n/index.js",
        "src/i18n/config.ts",
        "src/i18n/config.js",
        "src/i18n/i18n.ts",
        "src/lib/i18n.ts",
        "src/lib/i18n.js",
        "src/lib/i18n/index.ts",
        "src/config/i18n.ts",
        "i18n.ts",
        "i18n.js",
    ];
    let mut found: Vec<(String, String)> = vec![];
    for rel in INITS {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("import "))
        {
            let Some((_, spec)) = line.rsplit_once(" from ") else {
                continue;
            };
            let spec = spec.trim().trim_end_matches(';').trim_matches(['"', '\'']);
            let path = match spec.strip_prefix("@/") {
                Some(r) => format!("src/{r}"),
                None if spec.starts_with('.') => {
                    let mut segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
                    for s in spec.split('/') {
                        match s {
                            "." | "" => {}
                            ".." => {
                                segs.pop();
                            }
                            s => segs.push(s),
                        }
                    }
                    segs.join("/")
                }
                None => continue,
            };
            let Some(file) = ["", ".json", ".ts", ".js"]
                .iter()
                .map(|e| format!("{path}{e}"))
                .find(|f| root.join(f).is_file())
            else {
                continue;
            };
            // Locale segment, from the end: `locales/en.json`, `locales/en/common.json`.
            let segs: Vec<&str> = file.split('/').collect();
            let stem = |s: &str| s.split('.').next().unwrap_or(s).to_string();
            if let Some(i) = (0..segs.len()).rev().find(|&i| is_locale(&stem(segs[i]))) {
                let loc = stem(segs[i]);
                let mut p: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
                p[i] = p[i].replacen(&loc, "{locale}", 1);
                found.push((p.join("/"), loc));
            }
        }
    }
    // A pattern counts when 2+ locales share it (a lone `./utils/id` is not a catalog).
    let mut patterns = BTreeSet::new();
    let mut locales = BTreeSet::new();
    for (p, _) in &found {
        let locs: BTreeSet<&String> = found
            .iter()
            .filter(|(q, _)| q == p)
            .map(|(_, l)| l)
            .collect();
        if locs.len() >= 2 {
            patterns.insert(p.clone());
            locales.extend(locs.into_iter().cloned());
        }
    }
    (patterns.into_iter().collect(), locales)
}

/// Source files (`.ts/.tsx/.js/.jsx`, no tests/declarations) outside build and vendor dirs.
fn code_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = e.path();
            if path.is_dir() {
                if depth < 7
                    && !name.starts_with('.')
                    && ![
                        "node_modules",
                        "dist",
                        "build",
                        "out",
                        "coverage",
                        "public",
                        "target",
                        "vendor",
                    ]
                    .contains(&name.as_str())
                {
                    walk(root, &path, depth + 1, out);
                }
            } else if [".ts", ".tsx", ".js", ".jsx"]
                .iter()
                .any(|x| name.ends_with(x))
                && !name.ends_with(".d.ts")
                && !name.contains(".test.")
                && !name.contains(".spec.")
                && !name.contains(".config.")
            {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = vec![];
    walk(root, root, 0, &mut out);
    out.sort();
    out
}

/// One TS/JS file holding a dictionary keyed by locale (`translations.ts`, or inline in
/// `LanguageContext.tsx`): the one with the most source entries.
fn find_dictionary(root: &Path) -> Option<(String, BTreeSet<String>)> {
    const HINTS: &[&str] = &[
        "transl",
        "i18n",
        "lang",
        "locale",
        "dictionar",
        "messages",
        "context",
        "intl",
        "strings",
    ];
    let mut best: Option<(usize, String, BTreeSet<String>)> = None;
    for rel in code_files(root) {
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_ascii_lowercase();
        if !HINTS.iter().any(|h| name.contains(h)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let locs = glot_core::formats::js::dictionary_locales(&text);
        let Some(first) = locs.first().filter(|_| locs.len() >= 2) else {
            continue;
        };
        let n = glot_core::formats::parse(
            glot_core::catalog::Format::Js,
            Path::new(&rel),
            &text,
            first,
        )
        .map_or(0, |p| p.entries.len());
        if n > 0 && best.as_ref().is_none_or(|b| n > b.0) {
            best = Some((n, rel, locs.into_iter().collect()));
        }
    }
    best.map(|(_, f, l)| (f, l))
}

/// The app's hand-rolled translation hook: `export const useLanguage = …` in a file with a
/// `createContext` (or the dictionary), whose result exposes `t`. The import specifier comes from
/// a component already importing it (`@/contexts/LanguageContext`).
pub fn custom_runtime(root: &Path) -> Option<glot_extract::i18nize::CustomRuntime> {
    let files: Vec<(String, String)> = code_files(root)
        .into_iter()
        .filter_map(|r| Some((r.clone(), std::fs::read_to_string(root.join(&r)).ok()?)))
        .collect();
    let (hook, file, text) = files.iter().find_map(|(rel, text)| {
        if !text.contains("createContext") && !text.contains("translations") {
            return None;
        }
        ["export const use", "export function use"]
            .iter()
            .find_map(|m| {
                text.match_indices(m).find_map(|(i, _)| {
                    let rest: String = text[i + m.len()..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    let l = rest.to_ascii_lowercase();
                    ["lang", "locale", "transl", "i18n"]
                        .iter()
                        .any(|w| l.contains(w))
                        .then(|| (format!("use{rest}"), rel.clone(), text))
                })
            })
    })?;
    let call = format!("{hook}(");
    let destructures_t = |l: &str| {
        ["{ t }", "{ t,", ", t }", ", t,"]
            .iter()
            .any(|p| l.contains(p))
    };
    let uses_t = files
        .iter()
        .any(|(_, t)| t.lines().any(|l| l.contains(&call) && destructures_t(l)));
    let defines_t = ["const t =", "function t(", "t: (", "t = ("]
        .iter()
        .any(|p| text.contains(p));
    if !uses_t && !defines_t {
        return None;
    }
    let spec = files.iter().find_map(|(_, t)| {
        t.lines().find_map(|l| {
            let l = l.trim();
            if !l.starts_with("import ") || !l.contains(&hook) {
                return None;
            }
            let s = l.rsplit_once(" from ")?.1.trim().trim_end_matches(';');
            let s = s.trim_matches(['"', '\'']);
            (!s.starts_with('.')).then(|| s.to_string())
        })
    });
    let module = file
        .rsplit_once('.')
        .map_or(file.clone(), |(m, _)| m.to_string());
    Some(glot_extract::i18nize::CustomRuntime { hook, module, spec })
}

/// First conventional directory holding `{locale}.json` files or `{locale}/*.json` folders.
fn find_catalogs(root: &Path) -> (Vec<String>, BTreeSet<String>) {
    const DIRS: &[&str] = &[
        "messages",
        "locales",
        "public/locales",
        "src/locales",
        "src/messages",
        "src/i18n/locales",
        "src/i18n/messages",
        "src/i18n/resources",
        "src/i18n",
        "lang",
        "src/lang",
        "i18n",
        "translations",
        // ngx-translate / Transloco (Angular), Nuxt i18n v9.
        "src/assets/i18n",
        "public/i18n",
        "assets/i18n",
        "i18n/locales",
    ];
    find_in(root, DIRS)
}

/// Catalog directory set in the library config: Transloco `rootTranslationsPath`, Nuxt i18n
/// `langDir` (relative to `i18n/` since v9, to the root before).
fn configured_dirs(root: &Path) -> Vec<String> {
    let mut out = vec![];
    for (file, key) in [
        ("transloco.config.ts", "rootTranslationsPath"),
        ("transloco.config.js", "rootTranslationsPath"),
        ("nuxt.config.ts", "langDir"),
        ("nuxt.config.js", "langDir"),
    ] {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        let value = text.split_once(key).and_then(|(_, r)| {
            let r = r.trim_start().strip_prefix(':')?.trim_start();
            let q = r.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
            r[1..].split(q).next()
        });
        let Some(v) = value.map(|v| v.trim_start_matches("./").trim_end_matches('/')) else {
            continue;
        };
        if key == "langDir" {
            out.push(format!("i18n/{v}"));
        }
        out.push(v.to_string());
    }
    out
}

/// First of `dirs` holding `{locale}.json` files or `{locale}/*.json` folders.
fn find_in(root: &Path, dirs: &[&str]) -> (Vec<String>, BTreeSet<String>) {
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        let mut patterns = BTreeSet::new();
        let mut locales = BTreeSet::new();
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() && is_locale(&name) {
                for f in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    let f = f.file_name().to_string_lossy().to_string();
                    if f.ends_with(".json") {
                        patterns.insert(format!("{dir}/{{locale}}/{f}"));
                        locales.insert(name.clone());
                    }
                }
            } else if let Some(stem) = name.strip_suffix(".json").filter(|s| is_locale(s)) {
                patterns.insert(format!("{dir}/{{locale}}.json"));
                locales.insert(stem.to_string());
            }
        }
        if !patterns.is_empty() {
            return (patterns.into_iter().collect(), locales);
        }
    }
    (vec![], BTreeSet::new())
}

/// `en`, `fr`, `pt-BR`, `zh_Hant`.
fn is_locale(s: &str) -> bool {
    let (lang, region) = match s.split_once(['-', '_']) {
        Some((l, r)) => (l, Some(r)),
        None => (s, None),
    };
    (2..=3).contains(&lang.len())
        && lang.chars().all(|c| c.is_ascii_lowercase())
        && region.is_none_or(|r| {
            (2..=4).contains(&r.len()) && r.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

pub fn run(args: InitArgs) -> Result<()> {
    let root = std::env::current_dir()?;
    let d = detect(&root);
    anstream::println!(
        "{BOLD}glot init{BOLD:#}  stack: {GREEN}{}{GREEN:#}  catalogs: {}",
        d.stack,
        d.files.join(", ")
    );
    if d.stack == "i18next" && d.locales.is_empty() {
        anstream::println!(
            "{YELLOW}warning:{YELLOW:#} i18next is installed but no catalog was found (looked at the files imported by src/i18n*, src/i18n/config.*, src/lib/i18n.*, then src/locales, src/i18n/**, public/locales). glot will create {}: if your translations live elsewhere, set `files` in glot.toml (e.g. `src/i18n/locales/{{locale}}.json`).",
            d.files.join(", ")
        );
    }

    // Hardcoded copy, scanned before choosing the source: a French-only app is `fr`, not `en`.
    let report = glot_extract::extract(
        &root,
        &glot_core::config::ExtractConfig {
            include: includes(&root),
            ..Default::default()
        },
    )
    .ok();
    let guessed = report
        .as_ref()
        .and_then(|r| guess_language(r.hardcoded.iter().chain(&r.config).map(|h| h.text.as_str())));
    let mut source = args.source.clone().unwrap_or_else(|| {
        if d.locales.contains("en") {
            "en".into()
        } else if d.locales.is_empty() {
            guessed.unwrap_or("en").into()
        } else {
            d.locales.first().cloned().unwrap()
        }
    });
    let mut targets: Vec<String> = if args.targets.is_empty() {
        let found: Vec<String> = d
            .locales
            .iter()
            .filter(|l| **l != source)
            .cloned()
            .collect();
        if found.is_empty() {
            ["en", "fr", "es", "de"]
                .iter()
                .filter(|l| **l != source)
                .take(3)
                .map(|l| l.to_string())
                .collect()
        } else {
            found
        }
    } else {
        args.targets.clone()
    };
    let interactive = !args.yes && std::io::stdin().is_terminal();
    if !interactive && args.targets.is_empty() && d.locales.is_empty() {
        anstream::println!(
            "{DIM}No locales found and no terminal to ask: source {source}, targets {} (change with --source / --targets fr,de).{DIM:#}",
            targets.join(", ")
        );
    }
    if interactive {
        if args.source.is_none() {
            source = prompt("Source locale", &source)?;
        }
        if args.targets.is_empty() {
            targets = prompt("Target locales (comma separated)", &targets.join(","))?
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    targets.retain(|t| *t != source);
    if targets.is_empty() {
        bail!("no target locales: pass --targets fr,de");
    }

    let provider = args.provider.clone().unwrap_or_else(|| {
        if std::env::var_os("ANTHROPIC_API_KEY").is_none()
            && std::env::var_os("OPENAI_API_KEY").is_some()
        {
            "openai".into()
        } else {
            "anthropic".into()
        }
    });
    let w = Writer {
        root: &root,
        force: args.force,
    };
    w.file(
        "glot.toml",
        &glot_toml(&root, &d, &source, &targets, &provider)?,
    )?;
    w.file("glossary.toml", GLOSSARY_TOML)?;
    w.mcp_json()?;
    let block = agent_block(&root);
    w.block("CLAUDE.md", &block, true)?;
    // AGENTS.md: Codex, Copilot coding agent, Windsurf, Cursor. replit.md: Replit Agent.
    w.block("AGENTS.md", &block, true)?;
    let replit = root.join(".replit").is_file() || root.join("replit.nix").is_file();
    w.block("replit.md", &block, replit)?;
    // Same skill for Claude Code and the shared Agent Skills path (Codex, Cursor, Copilot, Gemini CLI).
    for dir in [".claude", ".agents"] {
        w.file(
            &format!("{dir}/skills/glot/SKILL.md"),
            &format!("{SKILL_HEAD}{block}{SKILL_TAIL}"),
        )?;
    }
    w.file(".cursor/rules/glot.mdc", &format!("{CURSOR_HEAD}{block}\n"))?;
    w.file(".github/workflows/glot.yml", &workflow(&provider))?;

    let glot = cli_name();
    let pkg = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
    if pkg.contains("\"lovable-tagger\"") || root.join(".bolt").is_dir() {
        anstream::println!(
            "{YELLOW}note:{YELLOW:#} Lovable / Bolt / v0 do not read these files: paste the output of {BOLD}{glot} agent-prompt --for lovable{BOLD:#} into the project's Knowledge (custom instructions)."
        );
    }
    let hardcoded = report.map_or(0, |r| r.hardcoded.len());
    if hardcoded > 0 {
        anstream::println!(
            "\n{BOLD}Next:{BOLD:#} {hardcoded} hardcoded strings found: run {BOLD}{glot} i18nize{BOLD:#} to move them into the catalog and wire the i18n library ({DIM}--dry-run{DIM:#} to preview), then {DIM}{glot} translate{DIM:#}"
        );
    } else {
        anstream::println!(
            "\n{BOLD}Next:{BOLD:#} {DIM}{glot} status{DIM:#} · {DIM}{glot} translate --dry-run{DIM:#} · {DIM}{glot} check{DIM:#}"
        );
    }
    Ok(())
}

/// How the user runs glot, for hints: `npx glot-i18n` when it runs from the npm package.
pub fn cli_name() -> &'static str {
    let exe = std::env::current_exe().unwrap_or_default();
    let s = exe.to_string_lossy();
    if s.contains("node_modules") || s.contains("_npx") {
        "npx glot-i18n"
    } else {
        "glot"
    }
}

fn prompt(label: &str, default: &str) -> Result<String> {
    eprint!("{label} [{default}]: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim();
    Ok(if line.is_empty() {
        default.to_string()
    } else {
        line.to_string()
    })
}

struct Writer<'a> {
    root: &'a Path,
    force: bool,
}

impl Writer<'_> {
    fn report(&self, rel: &str, what: &str) {
        match what {
            "skipped" => anstream::println!(
                "  {YELLOW}skip{YELLOW:#}    {rel} {DIM}(exists, use --force){DIM:#}"
            ),
            _ => anstream::println!("  {GREEN}{what:<7}{GREEN:#} {rel}"),
        }
    }

    fn write(&self, rel: &str, content: &str) -> Result<()> {
        let path = self.root.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
    }

    fn file(&self, rel: &str, content: &str) -> Result<()> {
        let exists = self.root.join(rel).exists();
        if exists && !self.force {
            self.report(rel, "skipped");
            return Ok(());
        }
        self.write(rel, content)?;
        self.report(rel, if exists { "updated" } else { "created" });
        Ok(())
    }

    /// Merge a `glot` server into `.mcp.json`, keeping other servers.
    fn mcp_json(&self) -> Result<()> {
        let rel = ".mcp.json";
        let path = self.root.join(rel);
        let existed = path.exists();
        let mut json: serde_json::Value = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).with_context(|| format!("parsing {rel}"))?,
            Err(_) => serde_json::json!({}),
        };
        let servers = json
            .as_object_mut()
            .context(".mcp.json is not an object")?
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .context(".mcp.json: mcpServers is not an object")?;
        if servers.contains_key("glot") && !self.force {
            self.report(rel, "skipped");
            return Ok(());
        }
        servers.insert(
            "glot".into(),
            // Prefer the installed binary; projects that only use `npx glot-i18n` get the npx form.
            if glot_on_path() {
                serde_json::json!({ "command": "glot", "args": ["mcp"] })
            } else {
                serde_json::json!({ "command": "npx", "args": ["-y", "glot-i18n", "mcp"] })
            },
        );
        self.write(rel, &(serde_json::to_string_pretty(&json)? + "\n"))?;
        self.report(rel, if existed { "updated" } else { "created" });
        Ok(())
    }

    /// Insert our block between markers; replaces an existing block only with --force.
    fn block(&self, rel: &str, block: &str, create: bool) -> Result<()> {
        let path = self.root.join(rel);
        let current = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) if create => String::new(),
            Err(_) => return Ok(()),
        };
        let new = match (current.find(MARK_START), current.find(MARK_END)) {
            (Some(s), Some(e)) if e > s => {
                if !self.force {
                    self.report(rel, "skipped");
                    return Ok(());
                }
                format!(
                    "{}{}{}",
                    &current[..s],
                    block.trim_end(),
                    &current[e + MARK_END.len()..]
                )
            }
            _ if current.is_empty() => block.to_string(),
            _ => format!("{}\n\n{block}", current.trim_end()),
        };
        self.write(rel, &new)?;
        self.report(
            rel,
            if current.is_empty() {
                "created"
            } else {
                "updated"
            },
        );
        Ok(())
    }
}

fn toml_list(v: &[String]) -> String {
    serde_json::to_string(v).unwrap()
}

fn glot_toml(
    root: &Path,
    d: &Detected,
    source: &str,
    targets: &[String],
    provider: &str,
) -> Result<String> {
    let (model, fallback, key_env) = match provider {
        "anthropic" => (
            "claude-haiku-5-5",
            Some("claude-sonnet-5-5"),
            Some("ANTHROPIC_API_KEY"),
        ),
        "openai" => ("gpt-5-mini", Some("gpt-5"), Some("OPENAI_API_KEY")),
        "openai-compatible" => ("llama3.1", None, None),
        // Preset: Gemini through its OpenAI-compatible endpoint.
        "gemini" => ("gemini-3-flash-preview", None, Some("GEMINI_API_KEY")),
        "mock" => ("mock", None, None),
        other => {
            bail!(
                "unknown provider `{other}` (anthropic | openai | openai-compatible | gemini | mock)"
            )
        }
    };
    let mut ex = glot_core::config::ExtractConfig::default();
    let include = native::swift_extract(d.stack, &mut ex).unwrap_or_else(|| includes(root));
    if root.join("manage.py").is_file() {
        // Django: `**/*.py` must not reach virtualenvs or migrations.
        ex.exclude.extend(
            [
                "**/venv/**",
                "**/.venv/**",
                "**/site-packages/**",
                "**/migrations/**",
            ]
            .map(String::from),
        );
    }
    // TS/JS dictionaries are catalogs, not code with hardcoded copy.
    let all: Vec<&str> = std::iter::once(source)
        .chain(targets.iter().map(String::as_str))
        .collect();
    for f in d.files.iter().filter(|f| {
        glot_core::formats::native(glot_core::catalog::Format::Auto, Path::new(f))
            == Some(glot_core::catalog::Format::Js)
    }) {
        ex.exclude
            .push(f.replace("{locale}", &format!("{{{}}}", all.join(","))));
    }
    let mut s = format!(
        "# glot configuration. Docs: `glot --help`.\n\
         source_locale = \"{source}\"\n\
         target_locales = {}\n\
         # Catalog files, `{{locale}}` is replaced by each locale.\n\
         files = {}\n\
         format = \"{}\"   # auto | json-nested | json-flat | json-message | xcstrings | strings | android | arb | po | yaml | js\n\
         syntax = \"{}\"   # icu | i18next | mf2 | printf | rails | vue | laravel\n\n\
         [llm]\n\
         provider = \"{}\"   # anthropic | openai | openai-compatible | mock\n\
         model = \"{model}\"\n",
        toml_list(targets),
        toml_list(&d.files),
        if message_objects(root, d, source) {
            "json-message"
        } else {
            "auto"
        },
        d.syntax,
        if provider == "gemini" {
            "openai-compatible"
        } else {
            provider
        },
    );
    if let Some(f) = fallback {
        s += &format!("fallback_model = \"{f}\"\n");
    }
    if let Some(k) = key_env {
        s += &format!("api_key_env = \"{k}\"\n");
    }
    if provider == "openai-compatible" {
        s += "base_url = \"http://localhost:11434/v1\"\n";
    }
    if provider == "gemini" {
        s += "base_url = \"https://generativelanguage.googleapis.com/v1beta/openai\"\n";
    }
    s += &format!(
        "budget_usd = 5.0\n\
         batch_size = 40\n\
         # style = \"Friendly and concise\"\n\n\
         [qa]\n\
         score_threshold = 80\n\
         max_length_ratio = 1.6\n\n\
         [extract]\n\
         include = {}\n\
         exclude = {}\n\
         functions = {}\n\n\
         # [server]\n\
         # url = \"http://localhost:8790\"\n\
         # project = \"my-saas\"\n\
         # token_env = \"GLOT_TOKEN\"\n",
        toml_list(&include),
        toml_list(&ex.exclude),
        toml_list(&ex.functions),
    );
    if d.stack == "i18next" {
        s += &i18next_section(root);
    }
    Ok(s)
}

/// FormatJS extracted catalog (`{"id": {"defaultMessage": …, "description": …}}`): every entry of
/// the source file is a message object.
fn message_objects(root: &Path, d: &Detected, source: &str) -> bool {
    let Some(f) = d.files.first().filter(|f| f.ends_with(".json")) else {
        return false;
    };
    let v = std::fs::read_to_string(root.join(f.replace("{locale}", source)))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    v.as_ref().and_then(|v| v.as_object()).is_some_and(|m| {
        !m.is_empty()
            && m.values().all(|e| {
                e.get("defaultMessage")
                    .or(e.get("message"))
                    .is_some_and(|x| x.is_string())
            })
    })
}

fn includes(root: &Path) -> Vec<String> {
    let mut include: Vec<String> = ["src", "app", "components", "pages", "lib"]
        .iter()
        .filter(|d| root.join(d).is_dir())
        .map(|d| format!("{d}/**/*.{{ts,tsx,js,jsx,vue}}"))
        .collect();
    // Laravel: `__()` / `@lang` in Blade views and PHP classes.
    if root.join("artisan").is_file() {
        include.extend(["app/**/*.php", "resources/**/*.{php,vue,ts,js}"].map(String::from));
    }
    // Non-JS usages (`glot_extract` literal patterns): Angular templates, Rails, Django, Symfony.
    let pkg = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
    if pkg.contains("\"@angular/core\"") && root.join("src").is_dir() {
        include.push("src/**/*.html".into());
    }
    if root.join("Gemfile").is_file() && root.join("app").is_dir() {
        include.push("app/**/*.{rb,erb,haml,slim}".into());
    }
    if root.join("manage.py").is_file() {
        include.extend(["**/*.py", "**/templates/**/*.html"].map(String::from));
    }
    if root.join("symfony.lock").is_file()
        || root.join("config/packages/translation.yaml").is_file()
    {
        include.extend(["src/**/*.php", "templates/**/*.twig"].map(String::from));
    }
    if include.is_empty() {
        glot_core::config::ExtractConfig::default().include
    } else {
        include
    }
}

/// Language of the app's hardcoded copy, by distinctive function words (`de`, `la`, `que` are shared
/// by several languages and left out). `None` unless one language clearly dominates.
fn guess_language<'a>(texts: impl Iterator<Item = &'a str>) -> Option<&'static str> {
    const WORDS: &[(&str, &[&str])] = &[
        (
            "en",
            &[
                "the", "and", "you", "your", "with", "this", "are", "to", "of", "is", "for",
            ],
        ),
        (
            "fr",
            &[
                "le", "les", "des", "du", "une", "et", "vous", "votre", "est", "pour", "avec",
                "pas", "dans", "sur", "aux", "ce", "au", "tu", "ton", "ta", "tes", "ou",
            ],
        ),
        (
            "es",
            &[
                "el", "los", "las", "y", "para", "por", "del", "usted", "tus", "sus", "está", "más",
            ],
        ),
        (
            "de",
            &[
                "der", "die", "das", "und", "ist", "nicht", "mit", "für", "ein", "eine", "sie",
                "zu", "auf", "ihre", "dein",
            ],
        ),
        (
            "it",
            &[
                "il", "gli", "della", "che", "sono", "per", "nel", "alla", "delle",
            ],
        ),
        (
            "pt",
            &["os", "uma", "não", "você", "com", "em", "seu", "sua", "ao"],
        ),
    ];
    let mut score = [0usize; 6];
    for t in texts {
        for w in t.split(|c: char| !c.is_alphabetic()) {
            let w = w.to_lowercase();
            for (i, (_, words)) in WORDS.iter().enumerate() {
                score[i] += words.contains(&w.as_str()) as usize;
            }
        }
    }
    let mut ranked: Vec<(usize, &str)> = score
        .iter()
        .zip(WORDS)
        .map(|(n, (l, _))| (*n, *l))
        .collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.0));
    // ponytail: stopword vote; a real detector (whatlang) if mixed-language apps show up.
    (ranked[0].0 >= 5 && ranked[0].0 >= 2 * ranked[1].0).then_some(ranked[0].1)
}

const GLOSSARY_TOML: &str = r#"# glot glossary: terms the translator must respect.
# Add terms with `glot glossary add <term> --fr "..."` or edit by hand.
#
# [[term]]
# term = "Workspace"
# description = "A team's shared space"
# translations = { fr = "Espace de travail", de = "Arbeitsbereich" }
# forbidden = { fr = ["Espace de travaux"] }
#
# [[term]]
# term = "Acme"
# do_not_translate = true
"#;

/// The rules block for agents: `glot agent-prompt` (one source of truth) between markers.
fn agent_block(root: &Path) -> String {
    format!(
        "{MARK_START}\n## Internationalization (glot)\n\n{}{MARK_END}\n",
        crate::localize::agent_prompt(root, "agent")
    )
}

const SKILL_HEAD: &str = r#"---
name: glot
description: Localize this app with glot. Use when adding or changing any user-facing text, when asked to make the app multilingual, translate it, add a language or a language switcher, or when touching translation catalogs or glot.toml. Never hand-roll a translations object or LanguageContext.
---

"#;

const SKILL_TAIL: &str = r#"
## Commands

- `glot status`: per-locale coverage and plan.
- `glot extract --json`: hardcoded strings, missing and unused keys.
- `glot add <key> <text> --note <context>`: add a source string.
- `glot translate --yes`: translate missing/stale strings (validated, glossary enforced).
- `glot check --format json`: placeholders, plurals, glossary. Exit 1 on errors.
"#;

const CURSOR_HEAD: &str = r#"---
description: i18n rules (glot) for user-facing strings
globs:
alwaysApply: true
---

"#;

fn workflow(provider: &str) -> String {
    // Mirrors the README snippet; MattJeff/glot is the placeholder for the published repo.
    let key = match provider {
        "openai" | "openai-compatible" => {
            "\n        env:\n          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}"
        }
        "gemini" => "\n        env:\n          GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}",
        "mock" => "",
        _ => "\n          anthropic-api-key: ${{ secrets.ANTHROPIC_API_KEY }}",
    };
    r#"name: glot
on:
  pull_request:

permissions:
  contents: write
  pull-requests: write

jobs:
  i18n:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: ${{ github.event.pull_request.head.repo.full_name }}
          ref: ${{ github.head_ref }}
      - uses: MattJeff/glot/action@v0
        # Secrets are not available to fork PRs: translate only on same-repo branches.
        with:
          command: ${{ github.event.pull_request.head.repo.full_name == github.repository && 'both' || 'check' }}KEY"#
        .replace("KEY", key)
        + "\n"
}

fn glot_on_path() -> bool {
    let exe = if cfg!(windows) { "glot.exe" } else { "glot" };
    // npx puts its temporary bin dir on PATH while it runs: that `glot` disappears afterwards.
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            let s = d.to_string_lossy();
            !s.contains("_npx") && !s.contains("node_modules") && d.join(exe).is_file()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_source_language_from_copy() {
        let fr = [
            "Enregistrer le filtre",
            "Vos coordonnées",
            "Pas encore de compte ?",
            "Ajoutez des équations pour créer une séquence",
            "Chemin d'envoi de la note",
        ];
        assert_eq!(guess_language(fr.into_iter()), Some("fr"));
        let en = [
            "Save the filter",
            "Your details",
            "No account yet? Sign up for free",
            "Add an equation to the sequence",
            "Path of the note",
        ];
        assert_eq!(guess_language(en.into_iter()), Some("en"));
        assert_eq!(guess_language(["OK", "Save"].into_iter()), None);
    }

    #[test]
    fn detects_ts_dictionaries() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(r.join("package.json"), r#"{"dependencies":{"next":"16"}}"#).unwrap();
        let w = |p: &str, c: &str| {
            std::fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
            std::fs::write(r.join(p), c).unwrap();
        };
        for ns in ["home", "billing"] {
            for l in ["en", "fr"] {
                w(
                    &format!("src/lib/i18n/dictionaries/{ns}/{l}.ts"),
                    "export const x = { \"a.b\": \"c\" };\n",
                );
            }
        }
        w("src/lib/i18n/dictionaries/index.ts", "");
        // Not locales: utilities named like language codes outside an i18n directory.
        w("src/utils/id.ts", "");
        w("src/utils/ms.ts", "");
        let d = detect(r);
        assert_eq!(d.stack, "TS/JS dictionaries");
        assert_eq!(
            d.files,
            [
                "src/lib/i18n/dictionaries/billing/{locale}.ts",
                "src/lib/i18n/dictionaries/home/{locale}.ts"
            ]
        );
        assert_eq!(d.locales, ["en", "fr"].map(String::from).into());
        let toml = glot_toml(r, &d, "en", &["fr".into()], "mock").unwrap();
        assert!(
            toml.contains("\"src/lib/i18n/dictionaries/home/{en,fr}.ts\""),
            "{toml}"
        );

        // i18next resources inlined in src/i18n.js.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(
            r.join("package.json"),
            r#"{"dependencies":{"react":"19","i18next":"23"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("src/i18n.js"), "const resources = {\n  en: { translation: { a: 'A' } },\n  fr: { translation: { a: 'B' } },\n};\n").unwrap();
        let d = detect(r);
        assert_eq!(
            (d.stack, d.files.as_slice()),
            ("i18next", ["src/i18n.js".to_string()].as_slice())
        );
        assert_eq!(d.locales.len(), 2);
    }

    #[test]
    fn detects_lingui_vue_laravel() {
        let w = |r: &Path, p: &str, c: &str| {
            std::fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
            std::fs::write(r.join(p), c).unwrap();
        };
        // Lingui with its default PO catalogs: ICU syntax, not printf.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        w(
            r,
            "package.json",
            r#"{"dependencies":{"@lingui/core":"5","@lingui/react":"5"}}"#,
        );
        for l in ["en", "fr"] {
            w(
                r,
                &format!("src/locales/{l}/messages.po"),
                "msgid \"Hi\"\nmsgstr \"\"\n",
            );
        }
        let d = detect(r);
        assert_eq!((d.stack, d.syntax), ("lingui", "icu"));
        assert_eq!(d.files, ["src/locales/{locale}/messages.po"]);
        assert_eq!(d.locales, ["en", "fr"].map(String::from).into());
        // Lingui before its first extract: its default path.
        let dir = tempfile::tempdir().unwrap();
        w(
            dir.path(),
            "package.json",
            r#"{"dependencies":{"@lingui/core":"5"}}"#,
        );
        assert_eq!(
            detect(dir.path()).files,
            ["src/locales/{locale}/messages.po"]
        );

        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        w(
            r,
            "package.json",
            r#"{"dependencies":{"vue":"3","vue-i18n":"11"}}"#,
        );
        w(r, "src/locales/en.json", "{}");
        let d = detect(r);
        assert_eq!((d.stack, d.syntax), ("vue-i18n", "vue"));
        assert_eq!(d.files, ["src/locales/{locale}.json"]);

        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        w(
            r,
            "composer.json",
            r#"{"require":{"laravel/framework":"^12.0"}}"#,
        );
        w(r, "package.json", r#"{"devDependencies":{"vite":"6"}}"#);
        w(r, "artisan", "");
        w(r, "lang/fr.json", "{}");
        w(r, "lang/en/auth.php", "<?php return [];");
        let d = detect(r);
        assert_eq!((d.stack, d.syntax), ("laravel", "laravel"));
        assert_eq!(d.files, ["lang/{locale}.json", "lang/{locale}/auth.php"]);
        std::fs::remove_dir_all(r.join("lang")).unwrap();
        assert_eq!(detect(r).files, ["lang/{locale}.json"]);
        assert!(includes(r).contains(&"resources/**/*.{php,vue,ts,js}".to_string()));
    }

    #[test]
    fn locales() {
        assert!(is_locale("en") && is_locale("pt-BR") && is_locale("zh_Hant"));
        assert!(!is_locale("common") && !is_locale("EN") && !is_locale("e"));
    }

    #[test]
    fn detects_i18next_namespaces() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(
            r.join("package.json"),
            r#"{"dependencies":{"react-i18next":"1"}}"#,
        )
        .unwrap();
        for l in ["en", "fr"] {
            std::fs::create_dir_all(r.join(format!("public/locales/{l}"))).unwrap();
            std::fs::write(r.join(format!("public/locales/{l}/common.json")), "{}").unwrap();
        }
        let d = detect(r);
        assert_eq!(d.stack, "i18next");
        assert_eq!(d.syntax, "i18next");
        assert_eq!(d.files, vec!["public/locales/{locale}/common.json"]);
        assert_eq!(d.locales.len(), 2);
        // Two namespaces: one `{ns}` pattern, so `common:title` and `home:title` never collide.
        std::fs::write(r.join("public/locales/en/home.json"), "{}").unwrap();
        assert_eq!(detect(r).files, vec!["public/locales/{locale}/{ns}.json"]);
        // `keySeparator: false` / `nsSeparator` from the app's config land in glot.toml.
        assert_eq!(i18next_section(r), "");
        std::fs::write(
            r.join("next-i18next.config.js"),
            "module.exports = { i18n: { defaultLocale: 'en' }, keySeparator: false, nsSeparator: '::' };",
        )
        .unwrap();
        let toml = glot_toml(r, &detect(r), "en", &["fr".into()], "mock").unwrap();
        assert!(
            toml.ends_with("[i18next]\nkey_separator = false\nns_separator = \"::\"\n"),
            "{toml}"
        );
        let c: glot_core::config::Config = toml::from_str(&toml).unwrap();
        let files = c.catalog_files(r, "fr");
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].0, r.join("public/locales/fr/home.json"));
        assert_eq!(
            (files[1].1.ns.as_str(), files[1].1.key_separator.as_str()),
            ("home::", "")
        );
    }

    #[test]
    fn vite_without_i18n_lib_gets_i18next_syntax() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(
            r.join("package.json"),
            r#"{"dependencies":{"react":"19","vite":"6"}}"#,
        )
        .unwrap();
        let d = detect(r);
        assert_eq!(
            (d.stack, d.syntax),
            ("react (no i18n library yet)", "i18next")
        );
        assert_eq!(d.files, vec!["src/locales/{locale}.json"]);
    }

    #[test]
    fn next_without_i18n_lib_gets_next_intl_convention() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(r.join("package.json"), r#"{"dependencies":{"next":"16"}}"#).unwrap();
        std::fs::create_dir_all(r.join("app")).unwrap();
        std::fs::write(r.join("app/layout.tsx"), "").unwrap();
        let d = detect(r);
        assert_eq!(d.stack, "next.js (no i18n library yet)");
        assert_eq!(d.files, vec!["messages/{locale}.json"]);
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn detects_angular_nuxt_and_message_objects() {
        let put = |r: &Path, rel: &str, body: &str| {
            std::fs::create_dir_all(r.join(rel).parent().unwrap()).unwrap();
            std::fs::write(r.join(rel), body).unwrap();
        };
        let cases: &[(&[(&str, &str)], &str, &str, &str)] = &[
            (
                &[
                    (
                        "package.json",
                        r#"{"dependencies":{"@ngx-translate/core":"15"}}"#,
                    ),
                    ("src/assets/i18n/en.json", "{}"),
                    ("src/assets/i18n/fr.json", "{}"),
                ],
                "ngx-translate",
                "i18next",
                "src/assets/i18n/{locale}.json",
            ),
            (
                &[
                    (
                        "package.json",
                        r#"{"dependencies":{"@jsverse/transloco":"7"}}"#,
                    ),
                    (
                        "transloco.config.ts",
                        "export default { rootTranslationsPath: 'src/app/i18n/', langs: [] };",
                    ),
                    ("src/app/i18n/en.json", "{}"),
                ],
                "transloco",
                "i18next",
                "src/app/i18n/{locale}.json",
            ),
            (
                &[
                    (
                        "package.json",
                        r#"{"devDependencies":{"@nuxtjs/i18n":"9"}}"#,
                    ),
                    (
                        "nuxt.config.ts",
                        "export default defineNuxtConfig({ i18n: { langDir: 'lang', locales: [] } })",
                    ),
                    ("i18n/lang/en-US.json", "{}"),
                ],
                "nuxt-i18n",
                "vue",
                "i18n/lang/{locale}.json",
            ),
        ];
        for (files, stack, syntax, pattern) in cases {
            let dir = tempfile::tempdir().unwrap();
            for (f, body) in *files {
                put(dir.path(), f, body);
            }
            let d = detect(dir.path());
            assert_eq!((d.stack, d.syntax), (*stack, *syntax));
            assert_eq!(d.files, [*pattern], "{stack}");
        }
        // FormatJS extracted: glot.toml says `json-message`, so `description` is never translated.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(r, "package.json", r#"{"dependencies":{"react-intl":"6"}}"#);
        put(
            r,
            "lang/en.json",
            r#"{"a1":{"defaultMessage":"Hi","description":"greeting"}}"#,
        );
        let d = detect(r);
        assert!(message_objects(r, &d, "en"));
        assert!(
            glot_toml(r, &d, "en", &["fr".into()], "mock")
                .unwrap()
                .contains("format = \"json-message\"")
        );
        put(r, "lang/en.json", r#"{"a1":"Hi"}"#);
        assert!(!message_objects(r, &d, "en"));
    }

    /// AI-builder layouts (anonymized from the Lovable / Bolt / v0 sample).
    #[test]
    fn detects_ai_builder_i18n() {
        let w = |r: &Path, p: &str, c: &str| {
            std::fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
            std::fs::write(r.join(p), c).unwrap();
        };
        let vite = r#"{"dependencies":{"react":"18"},"devDependencies":{"vite":"5","lovable-tagger":"1"}}"#;
        // C1: dictionary inline in LanguageContext.tsx + useLanguage().t.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        w(r, "package.json", vite);
        w(
            r,
            "src/contexts/LanguageContext.tsx",
            "import { createContext, useContext } from 'react';\ntype Language = 'en' | 'fr';\nconst translations: Record<Language, Record<string, string>> = {\n  en: { 'nav.home': 'Home' },\n  fr: { 'nav.home': 'Accueil' },\n};\nconst Ctx = createContext<any>(null);\nexport const LanguageProvider = ({ children }: any) => { const t = (k: string) => translations.en[k]; return <Ctx.Provider value={{ t }}>{children}</Ctx.Provider>; };\nexport const useLanguage = () => useContext(Ctx);\n",
        );
        w(
            r,
            "src/components/Nav.tsx",
            "import { useLanguage } from '@/contexts/LanguageContext';\nexport const Nav = () => { const { t } = useLanguage(); return <a>{t('nav.home')}</a>; };\n",
        );
        let d = detect(r);
        assert_eq!((d.stack, d.syntax), ("custom (LanguageContext)", "icu"));
        assert_eq!(d.files, ["src/contexts/LanguageContext.tsx"]);
        assert_eq!(d.locales, ["en", "fr"].map(String::from).into());
        let rt = custom_runtime(r).unwrap();
        assert_eq!(
            (rt.hook.as_str(), rt.module.as_str(), rt.spec.as_deref()),
            (
                "useLanguage",
                "src/contexts/LanguageContext",
                Some("@/contexts/LanguageContext")
            )
        );
        // The rules never tell the agent to add a second runtime.
        let p = crate::localize::agent_prompt(r, "agent");
        assert!(
            p.contains("const { t } = useLanguage()") && p.contains("do NOT install i18next"),
            "{p}"
        );
        assert!(
            !p.contains("install react-i18next") && !p.contains("useTranslation()"),
            "{p}"
        );
        let block = agent_block(r);
        assert!(
            block.starts_with(MARK_START) && block.contains("NOT create a LanguageContext"),
            "{block}"
        );

        // C2: `src/lib/translations.ts`, typed keys.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        w(r, "package.json", vite);
        w(
            r,
            "src/lib/translations.ts",
            "export type Language = 'en' | 'hi';\nexport const translations = {\n en: { nav_home: 'Home' },\n hi: { nav_home: 'Ghar' },\n};\nexport type TranslationKey = keyof typeof translations.en;\n",
        );
        let d = detect(r);
        assert_eq!(
            (d.stack, d.files.as_slice()),
            (
                "TS/JS dictionaries",
                ["src/lib/translations.ts".to_string()].as_slice()
            )
        );

        // react-i18next: inline resources in src/i18n/config.ts / src/lib/i18n.ts.
        let i18next = r#"{"dependencies":{"react":"18","i18next":"23","react-i18next":"13"}}"#;
        for f in ["src/i18n/config.ts", "src/lib/i18n.ts"] {
            let dir = tempfile::tempdir().unwrap();
            let r = dir.path();
            w(r, "package.json", i18next);
            w(
                r,
                f,
                "import i18n from 'i18next';\nconst resources = {\n en: { translation: { nav: { home: 'Home' } } },\n fr: { translation: { nav: { home: 'Accueil' } } },\n};\ni18n.init({ resources });\n",
            );
            let d = detect(r);
            assert_eq!(
                (d.stack, d.files.as_slice()),
                ("i18next", [f.to_string()].as_slice())
            );
        }
        // Imported resources: the import paths, not a guessed directory.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        w(r, "package.json", i18next);
        w(
            r,
            "src/i18n/config.ts",
            "import i18n from 'i18next';\nimport en from './resources/en.json';\nimport fr from '@/i18n/resources/fr.json';\nimport de from '../translations/de';\ni18n.init({ resources: { en: { translation: en }, fr: { translation: fr } } });\n",
        );
        w(r, "src/i18n/resources/en.json", "{}");
        w(r, "src/i18n/resources/fr.json", "{}");
        w(r, "src/translations/de.ts", "export default {};");
        let d = detect(r);
        assert_eq!(d.files, ["src/i18n/resources/{locale}.json"]);
        assert_eq!(d.locales, ["en", "fr"].map(String::from).into());
        std::fs::remove_file(r.join("src/i18n/config.ts")).unwrap();
        assert_eq!(detect(r).files, ["src/i18n/resources/{locale}.json"]);
        // Nothing found: no locales, so `init` warns instead of silently using public/locales.
        std::fs::remove_dir_all(r.join("src")).unwrap();
        assert!(detect(r).locales.is_empty());
    }
}
