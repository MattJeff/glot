//! `glot init` detection of static site generators and CMS themes (Shopify, WordPress, Hugo,
//! Jekyll + polyglot; Docusaurus and Starlight catalogs are found by `init_web`), and the
//! `[[docs]]` presets of Markdown sites translated whole by `glot docs` (Docusaurus, VitePress,
//! Starlight, MkDocs, Hugo).
use std::collections::BTreeSet;
use std::path::Path;

use super::{Detected, is_locale};

pub fn detect(root: &Path) -> Option<Detected> {
    shopify(root)
        .or_else(|| wordpress(root))
        .or_else(|| hugo(root))
        .or_else(|| jekyll(root))
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

fn read(root: &Path, files: &[&str]) -> Option<(String, String)> {
    files.iter().find_map(|f| {
        std::fs::read_to_string(root.join(f))
            .ok()
            .map(|t| (f.to_string(), t))
    })
}

/// Locale codes quoted in `text` after `key` (`locales: ['en', 'fr']`, `languages: ["en","fr"]`).
fn listed(text: &str, key: &str) -> BTreeSet<String> {
    let Some((_, rest)) = text.split_once(key) else {
        return BTreeSet::new();
    };
    let Some(open) = rest.find('[') else {
        return BTreeSet::new();
    };
    let body = &rest[open + 1..rest[open..].find(']').map_or(rest.len(), |e| open + e)];
    body.split(',')
        .map(|s| s.trim().trim_matches(['"', '\'', '`']).to_string())
        .filter(|s| is_locale(s))
        .collect()
}

/// `locales/en.default.json` (source) + `locales/fr.json`, theme editor strings in
/// `locales/*.schema.json`; Liquid `{{ name }}`, plurals as `one`/`other` sub-objects.
fn shopify(root: &Path) -> Option<Detected> {
    if !root.join("layout/theme.liquid").is_file() {
        return None;
    }
    let (mut locales, mut schema) = (BTreeSet::new(), false);
    for n in names(&root.join("locales")) {
        let Some(stem) = n.strip_suffix(".json") else {
            continue;
        };
        let stem = match stem.strip_suffix(".schema") {
            Some(s) => {
                schema = true;
                s
            }
            None => stem,
        };
        let loc = stem.strip_suffix(".default").unwrap_or(stem);
        if is_locale(loc) {
            locales.insert(loc.to_string());
        }
    }
    let mut files = vec!["locales/{locale}.json".to_string()];
    if schema {
        files.push("locales/{locale}.schema.json".into());
    }
    Some(Detected {
        stack: "shopify-theme",
        syntax: "template",
        files,
        locales,
        source: None,
    })
}

/// A plugin (root `.php` with a `Plugin Name:` header) or theme (`style.css` with `Theme Name:`):
/// `languages/{text-domain}-{locale}.po` (themes: `languages/{locale}.po`), the `.pot` as source.
fn wordpress(root: &Path) -> Option<Detected> {
    let header = |text: &str, field: &str| {
        text.lines().find_map(|l| {
            let (_, v) = l.split_once(field)?;
            let v = v.trim().trim_end_matches("*/").trim();
            (!v.is_empty()).then(|| v.to_string())
        })
    };
    let plugin = names(root)
        .into_iter()
        .filter(|n| n.ends_with(".php"))
        .find_map(|n| {
            let t = std::fs::read_to_string(root.join(&n)).ok()?;
            header(&t, "Plugin Name:").map(|_| t)
        });
    let (text, theme) = match plugin {
        Some(t) => (t, false),
        None => {
            let t = std::fs::read_to_string(root.join("style.css")).ok()?;
            header(&t, "Theme Name:")?;
            (t, true)
        }
    };
    let slug = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let domain = header(&text, "Text Domain:").unwrap_or(slug);
    let dir = header(&text, "Domain Path:")
        .unwrap_or_else(|| "/languages".into())
        .trim_matches('/')
        .to_string();
    let prefix = if theme {
        String::new()
    } else {
        format!("{domain}-")
    };
    // The `.pot` is English: `en` is the source, targets are WordPress locales (`fr_FR`).
    let mut locales: BTreeSet<String> = ["en".to_string()].into();
    for n in names(&root.join(&dir)) {
        if let Some(loc) = n.strip_suffix(".po").and_then(|s| s.strip_prefix(&prefix))
            && is_locale(loc)
        {
            locales.insert(loc.to_string());
        }
    }
    Some(Detected {
        stack: if theme {
            "wordpress-theme"
        } else {
            "wordpress-plugin"
        },
        syntax: "printf",
        files: vec![format!("{dir}/{prefix}{{locale}}.po")],
        locales,
        source: None,
    })
}

const HUGO_CONFIGS: &[&str] = &[
    "hugo.toml",
    "hugo.yaml",
    "hugo.yml",
    "hugo.json",
    "config.toml",
    "config.yaml",
    "config.yml",
    "config/_default/hugo.toml",
    "config/_default/config.toml",
    "config/_default/languages.toml",
];

/// `i18n/{locale}.toml` (or `.yaml` / `.json`); Go templates `{{ .Count }}`, CLDR plural tables.
fn hugo(root: &Path) -> Option<Detected> {
    let (file, cfg) = read(root, HUGO_CONFIGS)?;
    if !file.starts_with("hugo.") && !cfg.to_ascii_lowercase().contains("baseurl") {
        return None;
    }
    let mut locales = BTreeSet::new();
    let mut ext = None;
    for n in names(&root.join("i18n")) {
        if let Some((stem, e)) = n.rsplit_once('.')
            && is_locale(stem)
            && matches!(e, "toml" | "yaml" | "yml" | "json")
        {
            locales.insert(stem.to_string());
            ext.get_or_insert(e.to_string());
        }
    }
    // `[languages.fr]` (TOML) / `languages:\n  fr:` (YAML).
    let mut yaml_languages = false;
    for l in cfg.lines() {
        let t = l.trim();
        if !l.starts_with(' ') {
            yaml_languages = t == "languages:";
        }
        let code = t
            .strip_prefix("[languages.")
            .and_then(|r| r.strip_suffix(']'))
            .or_else(|| {
                (yaml_languages && l.starts_with("  ") && !l.starts_with("   "))
                    .then(|| t.strip_suffix(':'))
                    .flatten()
            });
        if let Some(c) = code.map(|c| c.trim_matches('"').to_ascii_lowercase())
            && is_locale(&c)
        {
            locales.insert(c);
        }
    }
    let ext = ext.unwrap_or_else(|| "toml".into());
    Some(Detected {
        stack: "hugo",
        syntax: "template",
        files: vec![format!("i18n/{{locale}}.{ext}")],
        locales,
        source: None,
    })
}

/// jekyll-polyglot: `_data/{locale}/*.yml` read as `site.data[site.active_lang]` (or
/// `_data/{locale}.yml`), locales from `languages:` in `_config.yml`.
fn jekyll(root: &Path) -> Option<Detected> {
    let cfg = std::fs::read_to_string(root.join("_config.yml")).ok()?;
    let gems = std::fs::read_to_string(root.join("Gemfile")).unwrap_or_default();
    let polyglot = cfg.contains("jekyll-polyglot") || gems.contains("jekyll-polyglot");
    let mut locales = listed(&cfg, "languages:");
    let data = root.join("_data");
    let mut files = BTreeSet::new();
    for n in names(&data) {
        let p = data.join(&n);
        if p.is_dir() && is_locale(&n) {
            locales.insert(n.clone());
            for f in names(&p)
                .into_iter()
                .filter(|f| f.ends_with(".yml") || f.ends_with(".yaml"))
            {
                files.insert(format!("_data/{{locale}}/{f}"));
            }
        } else if let Some((stem, e)) = n.rsplit_once('.')
            && is_locale(stem)
            && matches!(e, "yml" | "yaml")
        {
            locales.insert(stem.to_string());
            files.insert(format!("_data/{{locale}}.{e}"));
        }
    }
    if files.is_empty() {
        if !polyglot {
            return None;
        }
        files.insert("_data/{locale}/strings.yml".into());
    }
    Some(Detected {
        stack: if polyglot {
            "jekyll-polyglot"
        } else {
            "jekyll"
        },
        syntax: "template",
        files: files.into_iter().collect(),
        locales,
        source: None,
    })
}

const DOCUSAURUS: &[&str] = &[
    "docusaurus.config.js",
    "docusaurus.config.ts",
    "docusaurus.config.mjs",
    "docusaurus.config.cjs",
];

/// glot.toml sections for the detected stack: Starlight's flat dotted UI keys, then `[[docs]]`.
pub fn extra_toml(root: &Path, stack: &str, source: &str) -> String {
    let mut s = String::new();
    if stack == "starlight" {
        s += "\n# Starlight UI strings are flat dotted keys (`search.label`).\n[i18next]\nkey_separator = false\n";
    }
    s + &docs_toml(root, source)
}

/// `[[docs]]` entries for the Markdown sites found under `root` (empty when none).
pub fn docs_toml(root: &Path, source: &str) -> String {
    let mut out: Vec<(String, String, &str)> = vec![];
    let has = |p: &str| root.join(p).exists();
    let any = |ps: &[&str]| ps.iter().any(|p| has(p));
    if any(DOCUSAURUS) {
        if has("docs") {
            out.push((
                "docs/**/*.md*".into(),
                "i18n/{locale}/docusaurus-plugin-content-docs/current/{path}.{ext}".into(),
                "Docusaurus docs",
            ));
        }
        if has("blog") {
            out.push((
                "blog/**/*.md*".into(),
                "i18n/{locale}/docusaurus-plugin-content-blog/{path}.{ext}".into(),
                "Docusaurus blog",
            ));
        }
    }
    let vitepress = [".vitepress", "docs/.vitepress"]
        .into_iter()
        .find(|d| has(d));
    if let Some(d) = vitepress {
        let base = d.trim_end_matches(".vitepress");
        out.push((
            format!("{base}**/*.md"),
            format!("{base}{{locale}}/{{path}}.{{ext}}"),
            "VitePress (root locale at the docs root)",
        ));
    }
    let pkg = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
    if pkg.contains("\"@astrojs/starlight\"") && has("src/content/docs") {
        let base = if has(&format!("src/content/docs/{source}")) {
            format!("src/content/docs/{source}/")
        } else {
            "src/content/docs/".into()
        };
        out.push((
            format!("{base}**/*.md*"),
            "src/content/docs/{locale}/{path}.{ext}".into(),
            "Starlight",
        ));
    }
    if let Ok(mk) = std::fs::read_to_string(root.join("mkdocs.yml"))
        && mk.contains("i18n")
    {
        if mk.contains("docs_structure: folder") {
            out.push((
                format!("docs/{source}/**/*.md"),
                "docs/{locale}/{path}.{ext}".into(),
                "mkdocs-static-i18n (folder)",
            ));
        } else {
            out.push((
                "docs/**/*.md".into(),
                "docs/{path}.{locale}.{ext}".into(),
                "mkdocs-static-i18n (suffix)",
            ));
        }
    }
    if read(root, HUGO_CONFIGS).is_some() && has("content") {
        if has(&format!("content/{source}")) {
            out.push((
                format!("content/{source}/**/*.md"),
                "content/{locale}/{path}.{ext}".into(),
                "Hugo (one content directory per language)",
            ));
        } else {
            out.push((
                "content/**/*.md".into(),
                "content/{path}.{locale}.{ext}".into(),
                "Hugo (translation by file name)",
            ));
        }
    }
    if out.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\n# Markdown / MDX pages translated whole by `glot docs` (front matter, text; code, links,\n\
         # shortcodes and MDX kept). `{path}` = path under the glob's folder without extension.\n",
    );
    for (src, tgt, what) in out {
        s += &format!("[[docs]]  # {what}\nsource = \"{src}\"\ntarget = \"{tgt}\"\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("glot-cms-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (p, c) in files {
            let p = d.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        d
    }

    fn det(d: &Path) -> (String, String, Vec<String>, Vec<String>) {
        let x = detect(d).expect("detected");
        (
            x.stack.into(),
            x.syntax.into(),
            x.files,
            x.locales.into_iter().collect(),
        )
    }

    #[test]
    fn shopify_theme() {
        let d = tmp(
            "shopify",
            &[
                ("layout/theme.liquid", ""),
                ("locales/en.default.json", "{}"),
                ("locales/fr.json", "{}"),
                ("locales/pt-BR.json", "{}"),
                ("locales/en.default.schema.json", "{}"),
            ],
        );
        assert_eq!(
            det(&d),
            (
                "shopify-theme".into(),
                "template".into(),
                vec![
                    "locales/{locale}.json".into(),
                    "locales/{locale}.schema.json".into()
                ],
                vec!["en".into(), "fr".into(), "pt-BR".into()]
            )
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn wordpress_plugin_and_theme() {
        let d = tmp(
            "wp",
            &[
                (
                    "my-plugin.php",
                    "<?php\n/**\n * Plugin Name: My Plugin\n * Text Domain: my-plugin\n * Domain Path: /languages\n */\n",
                ),
                ("languages/my-plugin.pot", ""),
                ("languages/my-plugin-fr_FR.po", ""),
            ],
        );
        let (stack, syntax, files, locales) = det(&d);
        assert_eq!(
            (stack.as_str(), syntax.as_str()),
            ("wordpress-plugin", "printf")
        );
        assert_eq!(files, ["languages/my-plugin-{locale}.po"]);
        assert_eq!(locales, ["en", "fr_FR"]);
        std::fs::remove_dir_all(d).unwrap();
        let t = tmp(
            "wptheme",
            &[
                (
                    "style.css",
                    "/*\nTheme Name: Twenty\nText Domain: twenty\n*/",
                ),
                ("languages/de_DE.po", ""),
            ],
        );
        let (stack, _, files, locales) = det(&t);
        assert_eq!(stack, "wordpress-theme");
        assert_eq!(files, ["languages/{locale}.po"]);
        assert_eq!(locales, ["de_DE", "en"]);
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn hugo_and_its_docs() {
        let d = tmp(
            "hugo",
            &[
                (
                    "hugo.toml",
                    "baseURL = 'https://x.org/'\n[languages.en]\nweight = 1\n[languages.fr]\nweight = 2\n",
                ),
                ("i18n/en.toml", "home = \"Home\"\n"),
                ("content/posts/a.md", "# A\n"),
            ],
        );
        assert_eq!(
            det(&d),
            (
                "hugo".into(),
                "template".into(),
                vec!["i18n/{locale}.toml".into()],
                vec!["en".into(), "fr".into()]
            )
        );
        let docs = docs_toml(&d, "en");
        assert!(
            docs.contains(
                "source = \"content/**/*.md\"\ntarget = \"content/{path}.{locale}.{ext}\""
            ),
            "{docs}"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn jekyll_polyglot() {
        let d = tmp(
            "jekyll",
            &[
                (
                    "_config.yml",
                    "languages: [\"en\", \"fr\", \"de\"]\nplugins:\n  - jekyll-polyglot\n",
                ),
                ("_data/en/strings.yml", "nav:\n  home: Home\n"),
            ],
        );
        assert_eq!(
            det(&d),
            (
                "jekyll-polyglot".into(),
                "template".into(),
                vec!["_data/{locale}/strings.yml".into()],
                vec!["de".into(), "en".into(), "fr".into()]
            )
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn docusaurus_docs_preset() {
        let d = tmp(
            "docusaurus",
            &[
                ("docusaurus.config.js", "module.exports = {};"),
                ("docs/intro.md", "# Intro\n"),
                ("blog/a.md", ""),
            ],
        );
        let docs = docs_toml(&d, "en");
        assert!(
            docs.contains("i18n/{locale}/docusaurus-plugin-content-docs/current/{path}.{ext}"),
            "{docs}"
        );
        assert!(
            docs.contains("i18n/{locale}/docusaurus-plugin-content-blog/{path}.{ext}"),
            "{docs}"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn markdown_sites_presets() {
        let v = tmp(
            "vitepress",
            &[
                ("docs/.vitepress/config.mts", ""),
                ("docs/index.md", "# Hi\n"),
            ],
        );
        assert!(
            docs_toml(&v, "en")
                .contains("source = \"docs/**/*.md\"\ntarget = \"docs/{locale}/{path}.{ext}\"")
        );
        std::fs::remove_dir_all(v).unwrap();
        let m = tmp(
            "mkdocs",
            &[(
                "mkdocs.yml",
                "plugins:\n  - i18n:\n      docs_structure: folder\n",
            )],
        );
        assert!(docs_toml(&m, "en").contains("source = \"docs/en/**/*.md\""));
        std::fs::remove_dir_all(m).unwrap();
        let s = tmp(
            "starlight",
            &[
                (
                    "package.json",
                    "{\"dependencies\":{\"@astrojs/starlight\":\"^0.30\"}}",
                ),
                ("src/content/docs/index.mdx", ""),
            ],
        );
        assert!(docs_toml(&s, "en").contains("source = \"src/content/docs/**/*.md*\"\ntarget = \"src/content/docs/{locale}/{path}.{ext}\""));
        std::fs::remove_dir_all(s).unwrap();
    }
}
