//! Site config labels translated by `glot docs`, beside the pages:
//! - MkDocs `nav` titles => mkdocs-static-i18n `nav_translations` of each language in `mkdocs.yml`;
//! - VitePress `themeConfig` texts (nav, sidebar, outline, footer…) of `.vitepress/config.*` =>
//!   `.vitepress/locales/{locale}.json` (`source text => translation`, applied by the config, see
//!   the docs);
//! - Hugo site `title` and menu `name`s => `[languages.<locale>]` of `hugo.toml`.
//!
//! A value someone changed by hand is kept: glot only replaces a value that is missing, equal to
//! the source text, or what it wrote last time (`.glot-docs/labels.json`).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    MkDocs,
    VitePress,
    Hugo,
}

#[derive(Debug)]
pub struct Labels {
    /// Config file, relative to the root.
    pub rel: String,
    /// Source texts, in file order, no duplicates.
    pub texts: Vec<String>,
    kind: Kind,
}

/// locale => config file => source text => value glot wrote.
pub type Written = BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>;

const HUGO: &[&str] = &["hugo.toml", "config.toml", "config/_default/hugo.toml"];

pub fn find(root: &Path, source: &str) -> Vec<Labels> {
    let mut out = vec![];
    let mut push = |rel: String, kind, texts: Vec<String>| {
        let mut seen = std::collections::BTreeSet::new();
        let texts: Vec<String> = texts
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| t.chars().any(char::is_alphabetic) && seen.insert(t.clone()))
            .collect();
        if !texts.is_empty() {
            out.push(Labels { rel, texts, kind });
        }
    };
    if let Ok(mk) = std::fs::read_to_string(root.join("mkdocs.yml"))
        && mk.contains("i18n")
    {
        push("mkdocs.yml".into(), Kind::MkDocs, mkdocs_nav(&mk));
    }
    for dir in [".vitepress", "docs/.vitepress"] {
        for f in ["config.mts", "config.ts", "config.mjs", "config.js"] {
            let rel = format!("{dir}/{f}");
            if let Ok(src) = std::fs::read_to_string(root.join(&rel)) {
                push(rel, Kind::VitePress, vitepress_texts(&src, f));
            }
        }
    }
    if let Some(rel) = HUGO.iter().find(|f| root.join(f).is_file())
        && let Ok(doc) = std::fs::read_to_string(root.join(rel))
            .unwrap_or_default()
            .parse::<toml_edit::DocumentMut>()
    {
        push(rel.to_string(), Kind::Hugo, hugo_texts(&doc, source));
    }
    out
}

/// Write `locale`'s labels; `tr` gives the translation of a source text. Returns the file written
/// (`None`: nothing changed, or the config has no place for this locale yet, after a warning).
pub fn write(
    root: &Path,
    l: &Labels,
    source: &str,
    locale: &str,
    tr: &dyn Fn(&str) -> Option<String>,
    written: &mut Written,
) -> Result<Option<PathBuf>> {
    let last = written
        .entry(locale.into())
        .or_default()
        .entry(l.rel.clone())
        .or_default();
    let map: BTreeMap<String, String> = l
        .texts
        .iter()
        .filter_map(|t| Some((t.clone(), tr(t)?)))
        .collect();
    // A current value glot may replace.
    let ours = |src: &str, cur: Option<&str>, last: &BTreeMap<String, String>| {
        cur.is_none_or(|c| c == src || last.get(src).is_some_and(|w| w == c))
    };
    let path = root.join(&l.rel);
    let (target, text) = match l.kind {
        Kind::VitePress => {
            let dir = Path::new(&l.rel).parent().unwrap_or(Path::new(""));
            let json = root
                .join(dir)
                .join("locales")
                .join(format!("{locale}.json"));
            let old: BTreeMap<String, String> = std::fs::read_to_string(&json)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            let mut new = BTreeMap::new();
            for t in &l.texts {
                let cur = old.get(t).map(String::as_str);
                match map.get(t) {
                    Some(v) if ours(t, cur, last) => {
                        new.insert(t.clone(), v.clone());
                    }
                    _ => {
                        if let Some(c) = cur {
                            new.insert(t.clone(), c.to_string());
                        }
                    }
                }
            }
            (json, serde_json::to_string_pretty(&new)? + "\n")
        }
        Kind::MkDocs => {
            let text = std::fs::read_to_string(&path)?;
            let Some(out) = mkdocs_write(&text, locale, &map, &|s, c| ours(s, c, last)) else {
                anstream::eprintln!(
                    "note: mkdocs.yml has no `- locale: {locale}` under the i18n plugin's `languages`: nav titles not written for {locale}"
                );
                return Ok(None);
            };
            (path, out)
        }
        Kind::Hugo => {
            let text = std::fs::read_to_string(&path)?;
            let mut doc: toml_edit::DocumentMut = text.parse().with_context(|| l.rel.clone())?;
            if !hugo_write(&mut doc, source, locale, &map, &|s, c| ours(s, c, last)) {
                anstream::eprintln!(
                    "note: {} has no [languages.{locale}]: site title and menus not written for {locale}",
                    l.rel
                );
                return Ok(None);
            }
            (path, doc.to_string())
        }
    };
    *last = map;
    if std::fs::read_to_string(&target).ok().as_deref() == Some(text.as_str()) {
        return Ok(None);
    }
    if let Some(d) = target.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&target, text)?;
    Ok(Some(target))
}

// ---------------------------------------------------------------- MkDocs

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if let Some(i) = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        i.replace("\\\"", "\"").replace("\\\\", "\\")
    } else if let Some(i) = s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        i.replace("''", "'")
    } else {
        s.to_string()
    }
}

fn yaml_str(s: &str) -> String {
    let plain = !s.is_empty()
        && !s.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%',
            '@', '`',
        ])
        && !s.contains(": ")
        && !s.contains(" #")
        && !s.ends_with(':')
        && s.trim() == s
        && !["true", "false", "null", "yes", "no", "on", "off", "~"]
            .contains(&s.to_ascii_lowercase().as_str());
    if plain {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The key of a `- Title: …` / `Title:` line (quoted or not).
fn yaml_key(t: &str) -> Option<String> {
    let t = t.trim_end();
    let (k, _) = if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = t[1..].find(q)? + 2;
        t[end..].starts_with(':').then_some((&t[..end], ()))?
    } else {
        let i = t
            .find(": ")
            .or_else(|| t.ends_with(':').then(|| t.len() - 1))?;
        (&t[..i], ())
    };
    Some(unquote(k))
}

/// Titles of the `nav:` tree (`- Title: page.md`, `- Section:`); bare `- page.md` items have none.
fn mkdocs_nav(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.starts_with("nav:")) else {
        return vec![];
    };
    let mut out = vec![];
    for l in &lines[start + 1..] {
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        if indent(l) == 0 {
            break;
        }
        if let Some(item) = l.trim_start().strip_prefix("- ")
            && let Some(k) = yaml_key(item)
        {
            out.push(k);
        }
    }
    out
}

/// `text` with the `nav_translations` of the `- locale: <locale>` language item set from `map`.
fn mkdocs_write(
    text: &str,
    locale: &str,
    map: &BTreeMap<String, String>,
    ours: &dyn Fn(&str, Option<&str>) -> bool,
) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let item = lines.iter().position(|l| {
        let t = l.trim_start().trim_start_matches("- ").trim_end();
        t.strip_prefix("locale:")
            .is_some_and(|v| unquote(v.split(" #").next().unwrap_or(v)) == locale)
    })?;
    let line = lines[item];
    let col = indent(line)
        + if line.trim_start().starts_with("- ") {
            2
        } else {
            0
        };
    let mut end = item + 1;
    let mut last = item;
    while end < lines.len() {
        let l = lines[end];
        if !l.trim().is_empty() {
            if indent(l) < col || (indent(l) == col && l.trim_start().starts_with("- ")) {
                break;
            }
            last = end;
        }
        end += 1;
    }
    let pad = " ".repeat(col);
    let nt = (item + 1..=last).find(|&i| {
        indent(lines[i]) == col && lines[i].trim_start().starts_with("nav_translations:")
    });
    // Current entries (kept in their order), then new ones (sorted).
    let (from, to, mut entries, inner) = match nt {
        Some(i) => {
            let mut j = i + 1;
            let mut cur = vec![];
            let mut inner = None;
            while j <= last && (lines[j].trim().is_empty() || indent(lines[j]) > col) {
                let t = lines[j].trim();
                if let (Some(k), Some((_, v))) = (yaml_key(t), t.split_once(": ")) {
                    inner.get_or_insert(indent(lines[j]));
                    cur.push((k, unquote(v)));
                }
                j += 1;
            }
            (i, j, cur, inner)
        }
        None => (last + 1, last + 1, vec![], None),
    };
    for (k, v) in map {
        match entries.iter_mut().find(|(ck, _)| ck == k) {
            Some(e) if ours(k, Some(&e.1)) => e.1 = v.clone(),
            Some(_) => {}
            None => entries.push((k.clone(), v.clone())),
        }
    }
    if entries.is_empty() {
        return Some(text.to_string());
    }
    let inner = " ".repeat(inner.unwrap_or(col + 2));
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut block = format!("{pad}nav_translations:{nl}");
    for (k, v) in &entries {
        block += &format!("{inner}{}: {}{nl}", yaml_str(k), yaml_str(v));
    }
    let mut out: String = lines[..from].concat();
    if !out.is_empty() && !out.ends_with('\n') {
        out += nl;
    }
    out += &block;
    out += &lines[to..].concat();
    Some(out)
}

// ---------------------------------------------------------------- VitePress

/// `themeConfig` texts shown to readers (and the site `title` / `description`), outside the other
/// locales' blocks of `locales` (their `root` block counts, its `label` is a language name).
const VP_KEYS: &[&str] = &[
    "title",
    "description",
    "text",
    "outlineTitle",
    "lastUpdatedText",
    "darkModeSwitchLabel",
    "lightModeSwitchTitle",
    "darkModeSwitchTitle",
    "sidebarMenuLabel",
    "returnToTopLabel",
    "langMenuLabel",
    "skipToContentLabel",
    "prev",
    "next",
    "message",
    "copyright",
    "label",
    "placeholder",
    "buttonText",
    "buttonAriaLabel",
    "noResultsText",
    "resetButtonTitle",
];

struct VpVisitor {
    path: Vec<String>,
    out: Vec<String>,
}

impl<'a> Visit<'a> for VpVisitor {
    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        let key = it
            .key
            .static_name()
            .map(|k| k.to_string())
            .unwrap_or_default();
        let n = self.path.len();
        // `locales: { root: {...}, fr: {...} }`: only the root locale is the source.
        if n >= 1 && self.path[n - 1] == "locales" && key != "root" {
            return;
        }
        let lang_name = key == "label" && self.path.last().is_some_and(|p| p == "root");
        let text = match &it.value {
            Expression::StringLiteral(s) => Some(s.value.to_string()),
            Expression::TemplateLiteral(t) if t.expressions.is_empty() => t
                .quasis
                .first()
                .and_then(|q| q.value.cooked.as_ref())
                .map(|c| c.to_string()),
            _ => None,
        };
        // `prev`/`next` are also booleans (skipped); `head` meta tags are not shown text.
        if let Some(t) = text
            && VP_KEYS.contains(&key.as_str())
            && !lang_name
            && !self.path.iter().any(|p| p == "head")
        {
            self.out.push(t);
        }
        self.path.push(key);
        walk::walk_object_property(self, it);
        self.path.pop();
    }
}

fn vitepress_texts(src: &str, file: &str) -> Vec<String> {
    let alloc = Allocator::default();
    let Some(program) = crate::setup::refactor::parse(&alloc, file, src) else {
        return vec![];
    };
    let mut v = VpVisitor {
        path: vec![],
        out: vec![],
    };
    v.visit_program(&program);
    v.out
}

// ---------------------------------------------------------------- Hugo

fn str_at<'a>(t: &'a dyn toml_edit::TableLike, k: &str) -> Option<&'a str> {
    t.get(k)?.as_str()
}

/// `(menus key, table of menus)` of the source: `[languages.<src>.menus]`, else the top level.
fn hugo_menus<'a>(
    doc: &'a toml_edit::DocumentMut,
    source: &str,
) -> Option<(&'static str, &'a dyn toml_edit::TableLike)> {
    let lang = doc
        .get("languages")
        .and_then(|l| l.get(source))
        .and_then(|l| l.as_table_like());
    for scope in [lang, Some(doc.as_table() as &dyn toml_edit::TableLike)]
        .into_iter()
        .flatten()
    {
        for k in ["menus", "menu"] {
            if let Some(m) = scope.get(k).and_then(|m| m.as_table_like()) {
                return Some((k, m));
            }
        }
    }
    None
}

fn hugo_title<'a>(doc: &'a toml_edit::DocumentMut, source: &str) -> Option<&'a str> {
    doc.get("languages")
        .and_then(|l| l.get(source))
        .and_then(|l| l.get("title"))
        .and_then(|t| t.as_str())
        .or_else(|| str_at(doc.as_table(), "title"))
}

fn menu_items(m: &toml_edit::Item) -> Vec<&dyn toml_edit::TableLike> {
    if let Some(a) = m.as_array_of_tables() {
        a.iter().map(|t| t as &dyn toml_edit::TableLike).collect()
    } else if let Some(a) = m.as_array() {
        a.iter()
            .filter_map(|v| v.as_inline_table().map(|t| t as &dyn toml_edit::TableLike))
            .collect()
    } else {
        vec![]
    }
}

fn hugo_texts(doc: &toml_edit::DocumentMut, source: &str) -> Vec<String> {
    let mut out: Vec<String> = hugo_title(doc, source)
        .map(str::to_string)
        .into_iter()
        .collect();
    if let Some((_, menus)) = hugo_menus(doc, source) {
        for (_, m) in menus.iter() {
            for it in menu_items(m) {
                out.extend(str_at(it, "name").map(str::to_string));
            }
        }
    }
    out
}

/// Set `[languages.<locale>]` `title` and `menus` from `map`; `false` when that table is missing.
fn hugo_write(
    doc: &mut toml_edit::DocumentMut,
    source: &str,
    locale: &str,
    map: &BTreeMap<String, String>,
    ours: &dyn Fn(&str, Option<&str>) -> bool,
) -> bool {
    let title = hugo_title(doc, source).map(str::to_string);
    // Source menus, copied: (menus key, menu name, entries).
    let menus: Vec<(String, String, Vec<toml_edit::Table>)> = hugo_menus(doc, source)
        .map(|(k, m)| {
            m.iter()
                .map(|(name, item)| {
                    let entries = menu_items(item)
                        .into_iter()
                        .map(|t| {
                            let mut tab = toml_edit::Table::new();
                            for (k, v) in t.iter() {
                                if let Some(v) = v.as_value() {
                                    tab.insert(k, toml_edit::value(v.clone()));
                                }
                            }
                            tab
                        })
                        .collect();
                    (k.to_string(), name.to_string(), entries)
                })
                .collect()
        })
        .unwrap_or_default();
    let Some(lang) = doc
        .get_mut("languages")
        .and_then(|l| l.get_mut(locale))
        .and_then(|l| l.as_table_like_mut())
    else {
        return false;
    };
    let set = |t: &mut dyn toml_edit::TableLike, key: &str, src: &str| {
        let Some(v) = map.get(src) else { return };
        let cur = t.get(key).and_then(|c| c.as_str()).map(str::to_string);
        if ours(src, cur.as_deref()) && cur.as_deref() != Some(v) {
            t.insert(key, toml_edit::value(v.as_str()));
        }
    };
    if let Some(t) = &title {
        set(lang, "title", t);
    }
    for (key, name, entries) in menus {
        let mk = lang.entry(&key).or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        });
        let Some(mt) = mk.as_table_like_mut() else {
            continue;
        };
        if mt.get(&name).is_none() {
            let mut arr = toml_edit::ArrayOfTables::new();
            for e in &entries {
                arr.push(e.clone());
            }
            mt.insert(&name, toml_edit::Item::ArrayOfTables(arr));
        }
        let Some(arr) = mt.get_mut(&name).and_then(|m| m.as_array_of_tables_mut()) else {
            continue;
        };
        if arr.len() != entries.len() {
            // A menu edited by hand (other items): left alone.
            continue;
        }
        for (t, e) in arr.iter_mut().zip(&entries) {
            if let Some(src) = e.get("name").and_then(|n| n.as_str()) {
                set(t, "name", src);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let p = d.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        d
    }

    fn fr(s: &str) -> Option<String> {
        Some(format!("{s} (fr)"))
    }

    const MK: &str = "site_name: Demo\nnav:\n  - Home: index.md\n  - 'User Guide':\n      - Writing: guide/writing.md\n      - guide/other.md\n  - About: https://example.com\nplugins:\n  - search\n  - i18n:\n      docs_structure: suffix\n      languages:\n        - locale: en\n          default: true\n          name: English\n        - locale: fr\n          name: Français\n          nav_translations:\n            Home: Accueil perso\n        - locale: de\n          name: Deutsch\nmarkdown_extensions:\n  - toc\n";

    #[test]
    fn mkdocs_nav_translations() {
        let d = tmp(&[("mkdocs.yml", MK)]);
        let found = find(d.path(), "en");
        assert_eq!(found[0].texts, ["Home", "User Guide", "Writing", "About"]);
        let mut w = Written::new();
        write(d.path(), &found[0], "en", "fr", &fr, &mut w).unwrap();
        write(d.path(), &found[0], "en", "de", &fr, &mut w).unwrap();
        let out = std::fs::read_to_string(d.path().join("mkdocs.yml")).unwrap();
        assert!(
            out.contains("          name: Français\n          nav_translations:\n            Home: Accueil perso\n            About: About (fr)\n            User Guide: User Guide (fr)\n            Writing: Writing (fr)\n        - locale: de\n"),
            "hand-written value kept:\n{out}"
        );
        assert!(
            out.contains("          name: Deutsch\n          nav_translations:\n            About: About (fr)\n"),
            "{out}"
        );
        assert!(out.ends_with("markdown_extensions:\n  - toc\n"), "{out}");
        // Second run: same file.
        write(d.path(), &found[0], "en", "fr", &fr, &mut w).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.path().join("mkdocs.yml")).unwrap(),
            out
        );
        // A changed translation replaces what glot wrote, not the hand edit.
        let fr2 = |s: &str| Some(format!("{s} v2"));
        write(d.path(), &found[0], "en", "fr", &fr2, &mut w).unwrap();
        let out = std::fs::read_to_string(d.path().join("mkdocs.yml")).unwrap();
        assert!(
            out.contains("Home: Accueil perso\n            About: About v2\n"),
            "{out}"
        );
    }

    #[test]
    fn vitepress_theme_config() {
        let cfg = "import { defineConfig } from 'vitepress'\nexport default defineConfig({\n  title: 'Demo',\n  head: [['meta', { name: 'x' }]],\n  locales: {\n    root: { label: 'English', lang: 'en' },\n    fr: { label: 'Français', lang: 'fr', themeConfig: { nav: [{ text: 'Guide FR', link: '/fr/' }] } },\n  },\n  themeConfig: {\n    nav: [{ text: 'Guide', link: '/guide/' }],\n    sidebar: [{ text: `Getting started`, items: [{ text: 'Install', link: '/install' }] }],\n    outlineTitle: 'On this page',\n    docFooter: { prev: 'Previous page', next: false },\n  },\n})\n";
        let d = tmp(&[("docs/.vitepress/config.mts", cfg)]);
        let found = find(d.path(), "en");
        assert_eq!(
            found[0].texts,
            [
                "Demo",
                "Guide",
                "Getting started",
                "Install",
                "On this page",
                "Previous page"
            ]
        );
        let mut w = Written::new();
        write(d.path(), &found[0], "en", "fr", &fr, &mut w).unwrap();
        let json =
            std::fs::read_to_string(d.path().join("docs/.vitepress/locales/fr.json")).unwrap();
        assert!(json.contains("\"Guide\": \"Guide (fr)\""), "{json}");
    }

    #[test]
    fn hugo_title_and_menus() {
        let cfg = "baseURL = 'https://example.org/'\ntitle = 'My site'\n\n[languages.en]\nweight = 1\n\n[languages.fr]\nweight = 2\n\n[[menus.main]]\nname = 'Home'\npageRef = '/'\nweight = 10\n\n[[menus.main]]\nname = 'Blog'\npageRef = '/posts'\nweight = 20\n";
        let d = tmp(&[("hugo.toml", cfg)]);
        let found = find(d.path(), "en");
        assert_eq!(found[0].texts, ["My site", "Home", "Blog"]);
        let mut w = Written::new();
        write(d.path(), &found[0], "en", "fr", &fr, &mut w).unwrap();
        let out = std::fs::read_to_string(d.path().join("hugo.toml")).unwrap();
        let doc: toml::Value = toml::from_str(&out).unwrap();
        let t = &doc["languages"]["fr"];
        assert_eq!(t["title"].as_str(), Some("My site (fr)"));
        assert_eq!(t["menus"]["main"][1]["name"].as_str(), Some("Blog (fr)"));
        assert_eq!(t["menus"]["main"][1]["pageRef"].as_str(), Some("/posts"));
        assert!(
            out.starts_with("baseURL = 'https://example.org/'\ntitle = 'My site'\n"),
            "{out}"
        );
        // No [languages.de]: nothing written.
        assert!(
            write(d.path(), &found[0], "en", "de", &fr, &mut w)
                .unwrap()
                .is_none()
        );
    }
}
