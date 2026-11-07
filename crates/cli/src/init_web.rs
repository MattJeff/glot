//! `glot init` for the JS web libraries whose catalogs sit outside the usual `locales/` folders:
//! Paraglide (inlang), Docusaurus, Astro Starlight, svelte-i18n, sveltekit-i18n, qwik-speak,
//! ember-intl and @solid-primitives/i18n. Plus the locales an app declares in its i18n config
//! (Transloco `langs`, Nuxt / Docusaurus / Astro / next-intl / Lingui `locales`).
use std::collections::BTreeSet;
use std::path::Path;

use super::{Detected, find_in, is_locale};

pub fn detect(root: &Path, has: &dyn Fn(&str) -> bool) -> Option<Detected> {
    if has("@inlang/paraglide-js") || root.join("project.inlang/settings.json").is_file() {
        return paraglide(root);
    }
    let found = |stack, syntax, dirs: &[&str], default: &str| {
        let (mut files, locales) = find_in(root, dirs);
        if files.is_empty() {
            files.push(default.to_string());
        }
        Some(Detected {
            stack,
            syntax,
            files,
            locales,
            source: None,
        })
    };
    if has("@docusaurus/core") {
        return docusaurus(root);
    }
    if has("@astrojs/starlight") {
        // UI strings (`src/content/i18n/{locale}.json`, i18next syntax); pages are Markdown.
        return found(
            "starlight",
            "i18next",
            &["src/content/i18n"],
            "src/content/i18n/{locale}.json",
        );
    }
    if has("ember-intl") {
        return found(
            "ember-intl",
            "icu",
            &["translations"],
            "translations/{locale}.yaml",
        );
    }
    if has("svelte-i18n") {
        // intl-messageformat: ICU.
        return found(
            "svelte-i18n",
            "icu",
            &[
                "src/lib/i18n/locales",
                "src/lib/locales",
                "src/lib/lang",
                "src/locales",
                "src/lang",
                "src/i18n",
                "src/lib/i18n",
                "locales",
                "messages",
                "static/locales",
            ],
            "src/lib/i18n/locales/{locale}.json",
        );
    }
    if has("sveltekit-i18n") {
        // @sveltekit-i18n/parser-default: `{{name}}`.
        return found(
            "sveltekit-i18n",
            "i18next",
            &[
                "src/lib/translations",
                "src/lib/i18n",
                "src/translations",
                "src/locales",
            ],
            "src/lib/translations/{locale}/common.json",
        );
    }
    if has("qwik-speak") {
        // `{{name}}` params, `i18n/{lang}/{asset}.json`.
        return found(
            "qwik-speak",
            "i18next",
            &["i18n", "public/i18n", "src/i18n"],
            "i18n/{locale}/app.json",
        );
    }
    None
}

/// `@solid-primitives/i18n` templates (`resolveTemplate`) are `{{ name }}`.
pub fn syntax_override(has: &dyn Fn(&str) -> bool) -> Option<(&'static str, &'static str)> {
    has("@solid-primitives/i18n").then_some(("@solid-primitives/i18n", "i18next"))
}

/// Transloco scopes: `src/assets/i18n/admin/{locale}.json` beside the root catalog => one
/// `src/assets/i18n/{ns}/{locale}.json` pattern (keys `admin:users`).
pub fn transloco_scopes(root: &Path, files: &[String]) -> Option<String> {
    let dir = files
        .iter()
        .find_map(|f| f.strip_suffix("/{locale}.json"))?;
    let scoped = std::fs::read_dir(root.join(dir)).ok()?.flatten().any(|e| {
        e.path().is_dir()
            && std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .any(|f| {
                    let n = f.file_name().to_string_lossy().to_string();
                    n.strip_suffix(".json").is_some_and(is_locale)
                })
    });
    scoped.then(|| format!("{dir}/{{ns}}/{{locale}}.json"))
}

/// `project.inlang/settings.json`: `pathPattern` (string or list), `locales`.
fn paraglide(root: &Path) -> Option<Detected> {
    let s: serde_json::Value = std::fs::read_to_string(root.join("project.inlang/settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let pat = &s["plugin.inlang.messageFormat"]["pathPattern"];
    let mut files: Vec<String> = match pat {
        serde_json::Value::String(p) => vec![p.clone()],
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
    .into_iter()
    .map(|p| {
        p.trim_start_matches("./")
            .replace("{languageTag}", "{locale}")
    })
    .collect();
    if files.is_empty() {
        files.push("messages/{locale}.json".into());
    }
    let locales = s
        .get("locales")
        .or_else(|| s.get("languageTags"))
        .and_then(|l| l.as_array())
        .map(|l| {
            l.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(Detected {
        stack: "paraglide",
        syntax: "icu",
        files,
        locales,
        source: s
            .get("baseLocale")
            .or_else(|| s.get("sourceLanguageTag"))
            .and_then(|l| l.as_str())
            .map(str::to_string),
    })
}

/// Docusaurus `i18n/{locale}/**/*.json` (`code.json`, theme navbar/footer, plugin options):
/// message objects `{"id": {"message", "description"}}`. Markdown under the plugin folders is
/// page content, not a catalog.
fn docusaurus(root: &Path) -> Option<Detected> {
    fn walk(dir: &Path, rel: &str, depth: usize, out: &mut BTreeSet<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let r = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if e.path().is_dir() {
                if depth < 2 {
                    walk(&e.path(), &r, depth + 1, out);
                }
            } else if name.ends_with(".json") {
                out.insert(r);
            }
        }
    }
    let mut files = BTreeSet::new();
    let mut locales = BTreeSet::new();
    for e in std::fs::read_dir(root.join("i18n"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() && is_locale(&name) {
            locales.insert(name);
            walk(&e.path(), "", 0, &mut files);
        }
    }
    let mut files: Vec<String> = files
        .into_iter()
        .map(|f| format!("i18n/{{locale}}/{f}"))
        .collect();
    if files.is_empty() {
        files.push("i18n/{locale}/code.json".into());
    }
    Some(Detected {
        stack: "docusaurus",
        syntax: "icu",
        files,
        locales,
        source: None,
    })
}

/// The source locale an i18n config declares: `defaultLocale` (Nuxt, Astro, Docusaurus,
/// next-intl, next-i18next), Lingui `sourceLocale`, Transloco `defaultLang`.
pub fn configured_source(root: &Path) -> Option<String> {
    const FILES: &[&str] = &[
        "lingui.config.ts",
        "lingui.config.js",
        "transloco.config.ts",
        "transloco.config.js",
        "nuxt.config.ts",
        "nuxt.config.js",
        "i18n.config.ts",
        "astro.config.mjs",
        "astro.config.ts",
        "docusaurus.config.js",
        "docusaurus.config.ts",
        "i18n/routing.ts",
        "src/i18n/routing.ts",
        "next-i18next.config.js",
    ];
    let value = |text: &str, key: &str| -> Option<String> {
        let rest = &text[text.find(key)? + key.len()..];
        let rest = rest.trim_start_matches(['"', '\'']).trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let q = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'))?;
        let v = &rest[1..1 + rest[1..].find(q)?];
        is_locale(v).then(|| v.to_string())
    };
    FILES.iter().find_map(|f| {
        let text = std::fs::read_to_string(root.join(f)).ok()?;
        ["sourceLocale", "defaultLocale", "defaultLang"]
            .iter()
            .find_map(|k| value(&text, k))
    })
}

/// vue-i18n `<i18n>` blocks in single-file components: one `{dir}/{ns}.vue` pattern per top-level
/// folder holding such components (keys `components/Hello:title`), and the locales they hold.
// ponytail: two folders with the same component path (`pages/Foo`, `components/Foo`) share a key prefix.
pub fn vue_sfc(root: &Path) -> Option<(Vec<String>, BTreeSet<String>)> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let (p, name) = (e.path(), e.file_name().to_string_lossy().to_string());
            if p.is_dir() {
                if depth < 8
                    && !name.starts_with('.')
                    && !["node_modules", "dist", "build", "public", "vendor"]
                        .contains(&name.as_str())
                {
                    walk(&p, depth + 1, out);
                }
            } else if name.ends_with(".vue") {
                out.push(p);
            }
        }
    }
    let mut vue = vec![];
    walk(root, 0, &mut vue);
    let (mut dirs, mut locales) = (BTreeSet::new(), BTreeSet::new());
    for p in vue {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let found = glot_core::formats::vue_sfc::locales(&text);
        if found.is_empty() {
            continue;
        }
        locales.extend(found.into_iter().filter(|l| is_locale(l)));
        let rel = p.strip_prefix(root).ok()?;
        let top = match rel.components().count() {
            1 => String::new(),
            _ => format!(
                "{}/",
                rel.components().next()?.as_os_str().to_string_lossy()
            ),
        };
        dirs.insert(top);
    }
    (!dirs.is_empty()).then(|| {
        (
            dirs.into_iter().map(|d| format!("{d}{{ns}}.vue")).collect(),
            locales,
        )
    })
}

/// Locales listed in the app's i18n config files.
pub fn configured_locales(root: &Path) -> BTreeSet<String> {
    const FILES: &[(&str, &str)] = &[
        ("transloco.config.ts", "langs"),
        ("transloco.config.js", "langs"),
        ("nuxt.config.ts", "locales"),
        ("nuxt.config.js", "locales"),
        ("astro.config.mjs", "locales"),
        ("astro.config.ts", "locales"),
        ("docusaurus.config.js", "locales"),
        ("docusaurus.config.ts", "locales"),
        ("lingui.config.ts", "locales"),
        ("lingui.config.js", "locales"),
        ("i18n/routing.ts", "locales"),
        ("src/i18n/routing.ts", "locales"),
        ("next-i18next.config.js", "locales"),
    ];
    let mut out = BTreeSet::new();
    for (f, key) in FILES {
        if let Ok(text) = std::fs::read_to_string(root.join(f)) {
            out.extend(declared(&text, key));
        }
    }
    out
}

/// `key: ['en', 'fr']`, `key: [{ code: 'en' }]`, `key: { root: { lang: 'en' }, fr: {…} }`.
fn declared(text: &str, key: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(i) = rest.find(key) {
        let after = rest[i + key.len()..].trim_start();
        rest = &rest[i + key.len()..];
        let Some(body) = after.strip_prefix(':').map(str::trim_start) else {
            continue;
        };
        let Some(open) = body.chars().next().filter(|c| matches!(c, '[' | '{')) else {
            continue;
        };
        let close = if open == '[' { ']' } else { '}' };
        let (mut depth, mut end) = (0usize, body.len());
        for (j, c) in body.char_indices() {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    end = j;
                    break;
                }
            }
        }
        out.extend(tokens(&body[1..end], open == '{'));
        break;
    }
    out
}

/// Locale tokens of a `locales` body: depth-1 strings (array form) or keys (object form),
/// `code:` / `lang:` values anywhere.
fn tokens(body: &str, object: bool) -> Vec<String> {
    let mut out = vec![];
    let mut depth = 0usize;
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    let mut last_word = String::new();
    while i < chars.len() {
        let c = chars[i];
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth = depth.saturating_sub(1),
            '\'' | '"' | '`' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&x| x == c)
                    .map_or(chars.len(), |p| i + 1 + p);
                let s: String = chars[i + 1..end.min(chars.len())].iter().collect();
                let next = chars[end.min(chars.len() - 1) + 1..]
                    .iter()
                    .find(|c| !c.is_whitespace());
                let is_key = next == Some(&':');
                if (depth == 0 && (!object || is_key))
                    || matches!(last_word.as_str(), "code" | "lang")
                {
                    out.push(s);
                }
                last_word.clear();
                i = end + 1;
                continue;
            }
            c if c.is_alphanumeric() || c == '_' || c == '-' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '-'))
                {
                    i += 1;
                }
                let w: String = chars[start..i].iter().collect();
                let next = chars[i..].iter().find(|c| !c.is_whitespace());
                if object && depth == 0 && next == Some(&':') {
                    out.push(w.clone());
                }
                last_word = w;
                continue;
            }
            ':' | ' ' | '\n' | '\t' => {}
            _ => last_word.clear(),
        }
        i += 1;
    }
    out.retain(|l| is_locale(l));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let p = dir.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        dir
    }

    fn detected(files: &[(&str, &str)]) -> (String, String, Vec<String>, Vec<String>) {
        let dir = project(files);
        let d = super::super::detect(dir.path());
        (
            d.stack.into(),
            d.syntax.into(),
            d.files,
            d.locales.into_iter().collect(),
        )
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn declared_source_locale() {
        let src = |files: &[(&str, &str)]| super::super::detect(project(files).path()).source;
        let pkg = |d: &str| format!(r#"{{"dependencies":{{"{d}":"1"}}}}"#);
        let para = pkg("@inlang/paraglide-js");
        assert_eq!(
            src(&[
                ("package.json", &para),
                (
                    "project.inlang/settings.json",
                    r#"{"baseLocale":"de","locales":["de","en"]}"#
                ),
            ]),
            Some("de".into())
        );
        let docu = pkg("@docusaurus/core");
        assert_eq!(
            src(&[
                ("package.json", &docu),
                (
                    "docusaurus.config.js",
                    "export default { i18n: { defaultLocale: 'fr', locales: ['fr', 'en'] } };"
                ),
            ]),
            Some("fr".into())
        );
        let nuxt = pkg("@nuxtjs/i18n");
        assert_eq!(
            src(&[
                ("package.json", &nuxt),
                (
                    "nuxt.config.ts",
                    "export default defineNuxtConfig({ i18n: { defaultLocale: 'es', locales: ['es', 'en'] } })"
                ),
            ]),
            Some("es".into())
        );
        let lingui = pkg("@lingui/core");
        assert_eq!(
            src(&[
                ("package.json", &lingui),
                (
                    "lingui.config.ts",
                    "export default { sourceLocale: \"cs\", locales: [\"cs\", \"en\"] }"
                )
            ]),
            Some("cs".into())
        );
        assert_eq!(
            src(&[
                ("pubspec.yaml", "dependencies:\n  slang: ^4.0.0\n"),
                ("slang.yaml", "base_locale: de\n"),
                ("lib/i18n/de.i18n.json", "{}"),
                ("lib/i18n/en.i18n.json", "{}")
            ]),
            Some("de".into())
        );
        assert_eq!(
            src(&[(
                "translations/app_fr.ts",
                "<?xml version=\"1.0\"?>\n<!DOCTYPE TS>\n<TS version=\"2.1\" language=\"fr\" sourcelanguage=\"de_DE\">\n</TS>\n"
            )]),
            Some("de".into())
        );
        assert_eq!(
            src(&[
                ("package.json", r#"{"dependencies":{"@angular/core":"20"}}"#),
                (
                    "angular.json",
                    r#"{"projects":{"app":{"i18n":{"sourceLocale":"en-US","locales":{"de":"src/locale/messages.de.xtb"}}}}}"#
                ),
            ]),
            Some("en-US".into())
        );
        assert_eq!(src(&[("package.json", &pkg("next-intl"))]), None);
    }

    #[test]
    fn vue_single_file_components() {
        let pkg = r#"{"dependencies":{"vue-i18n":"11"}}"#;
        let app = "<template/>\n<i18n>\n{ \"en\": {\"a\": \"A\"}, \"ja\": {} }\n</i18n>\n";
        let nav = "<i18n lang=\"yaml\">\nen:\n  b: B\nde:\n  b: Bb\n</i18n>\n";
        let (stack, _, files, locales) = detected(&[
            ("package.json", pkg),
            ("src/App.vue", app),
            ("src/components/Nav.vue", nav),
            ("src/components/Plain.vue", "<template/>"),
            ("node_modules/x/X.vue", app),
        ]);
        assert_eq!(stack, "vue-i18n");
        assert_eq!(files, s(&["src/{ns}.vue"]));
        assert_eq!(locales, s(&["de", "en", "ja"]));
        // Beside a JSON catalog, both are kept (new keys go to the JSON one).
        let (_, _, files, _) = detected(&[
            ("package.json", pkg),
            ("src/locales/en.json", "{}"),
            ("components/App.vue", app),
            ("pages/index.vue", nav),
        ]);
        assert_eq!(
            files,
            s(&[
                "src/locales/{locale}.json",
                "components/{ns}.vue",
                "pages/{ns}.vue"
            ])
        );
    }

    #[test]
    fn detects_js_web_libraries() {
        let pkg = |d: &str| format!(r#"{{"dependencies":{{{d}}}}}"#);
        // (files, (stack, syntax, catalogs, locales))
        type Case<'a> = (
            Vec<(&'a str, String)>,
            (&'a str, &'a str, Vec<String>, Vec<String>),
        );
        let cases: Vec<Case> = vec![
            (
                vec![
                    ("package.json", pkg(r#""@inlang/paraglide-js":"2""#)),
                    (
                        "project.inlang/settings.json",
                        r#"{"baseLocale":"en","locales":["en","de"],"plugin.inlang.messageFormat":{"pathPattern":"./messages/{locale}.json"}}"#.into(),
                    ),
                ],
                ("paraglide", "icu", s(&["messages/{locale}.json"]), s(&["de", "en"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""@docusaurus/core":"3""#)),
                    ("docusaurus.config.js", "export default { i18n: { defaultLocale: 'en', locales: ['en', 'fr'] } };".into()),
                    ("i18n/fr/code.json", "{}".into()),
                    ("i18n/fr/docusaurus-theme-classic/navbar.json", "{}".into()),
                    ("i18n/fr/docusaurus-plugin-content-docs/current/intro.md", "# Intro".into()),
                ],
                (
                    "docusaurus",
                    "icu",
                    s(&["i18n/{locale}/code.json", "i18n/{locale}/docusaurus-theme-classic/navbar.json"]),
                    s(&["en", "fr"]),
                ),
            ),
            (
                vec![
                    ("package.json", pkg(r#""astro":"5","@astrojs/starlight":"0.36""#)),
                    ("src/content/i18n/en.json", "{}".into()),
                ],
                ("starlight", "i18next", s(&["src/content/i18n/{locale}.json"]), s(&["en"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""ember-intl":"7""#)),
                    ("translations/en-us.yaml", "hello: Hello".into()),
                    ("translations/fr-fr.yaml", "hello: Bonjour".into()),
                ],
                ("ember-intl", "icu", s(&["translations/{locale}.yaml"]), s(&["en-us", "fr-fr"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""svelte-i18n":"4""#)),
                    ("src/lib/i18n/locales/en.json", "{}".into()),
                ],
                ("svelte-i18n", "icu", s(&["src/lib/i18n/locales/{locale}.json"]), s(&["en"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""sveltekit-i18n":"2""#)),
                    ("src/lib/translations/en/common.json", "{}".into()),
                ],
                ("sveltekit-i18n", "i18next", s(&["src/lib/translations/{locale}/common.json"]), s(&["en"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""qwik-speak":"0.23""#)),
                    ("i18n/en-US/app.json", "{}".into()),
                ],
                ("qwik-speak", "i18next", s(&["i18n/{locale}/app.json"]), s(&["en-US"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""vue-i18n":"11""#)),
                    ("src/locales/en.yaml", "hello: Hello".into()),
                ],
                ("vue-i18n", "vue", s(&["src/locales/{locale}.yaml"]), s(&["en"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""@jsverse/transloco":"7""#)),
                    ("transloco.config.ts", "export default { rootTranslationsPath: 'src/assets/i18n/', langs: ['en', 'es'] };".into()),
                    ("src/assets/i18n/en.json", "{}".into()),
                    ("src/assets/i18n/admin/en.json", "{}".into()),
                ],
                (
                    "transloco",
                    "i18next",
                    s(&["src/assets/i18n/{locale}.json", "src/assets/i18n/{ns}/{locale}.json"]),
                    s(&["en", "es"]),
                ),
            ),
            (
                vec![
                    ("package.json", pkg(r#""solid-js":"1","@solid-primitives/i18n":"2""#)),
                    ("src/i18n/en.ts", "export const dict = { hi: 'Hi {{ name }}' };".into()),
                    ("src/i18n/fr.ts", "export const dict = { hi: 'Salut {{ name }}' };".into()),
                ],
                ("@solid-primitives/i18n", "i18next", s(&["src/i18n/{locale}.ts"]), s(&["en", "fr"])),
            ),
            (
                vec![
                    ("package.json", pkg(r#""astro":"5""#)),
                    ("astro.config.mjs", "export default { i18n: { defaultLocale: 'en', locales: ['en', 'es'] } };".into()),
                    ("src/i18n/ui.ts", "export const ui = {\n  en: { 'nav.home': 'Home' },\n  es: { 'nav.home': 'Inicio' },\n} as const;\n".into()),
                ],
                ("TS/JS dictionaries", "icu", s(&["src/i18n/ui.ts"]), s(&["en", "es"])),
            ),
        ];
        for (files, (stack, syntax, f, l)) in cases {
            let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
            assert_eq!(
                detected(&files),
                (stack.to_string(), syntax.to_string(), f, l),
                "{stack}"
            );
        }
    }

    #[test]
    fn declared_locales() {
        let l = |t: &str, k: &str| declared(t, k).into_iter().collect::<Vec<_>>();
        assert_eq!(
            l("langs: ['en', 'es'], keysManager: {}", "langs"),
            ["en", "es"]
        );
        assert_eq!(
            l(
                "locales: [{ code: 'en', file: 'en.json', dir: 'ltr' }, { code: 'fr-CA', file: 'fr.json' }]",
                "locales"
            ),
            ["en", "fr-CA"]
        );
        assert_eq!(
            l(
                "locales: { root: { label: 'English', lang: 'en' }, fr: { label: 'Français' }, 'pt-br': { label: 'Português' } }",
                "locales"
            ),
            ["en", "fr", "pt-br"]
        );
        assert_eq!(
            l(
                "i18n: { defaultLocale: 'en', locales: ['en', 'fr'] }",
                "locales"
            ),
            ["en", "fr"]
        );
        assert!(l("const locales = getLocales()", "locales").is_empty());
    }
}
