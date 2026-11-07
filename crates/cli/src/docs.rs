//! `glot docs`: translate Markdown / MDX pages whole, from the `[[docs]]` entries of glot.toml.
//!
//! ```toml
//! [[docs]]
//! source = "docs/**/*.md*"
//! target = "i18n/{locale}/docusaurus-plugin-content-docs/current/{path}.{ext}"
//! ```
//!
//! Each page is split into segments (`glot_core::markdown`): front matter values, paragraphs,
//! list items, headings, table cells. Code, links, tags, MDX and shortcodes are placeholders the
//! validator checks. Segments go through the regular pipeline (glossary, validator, retries,
//! lockfile, translation memory) in a dedicated project, `.glot-docs/` (`{locale}.json` catalogs
//! keyed `page#hash`, its own `glot.lock`; `cd .glot-docs && glot check` works there), so the app's
//! catalogs stay untouched. Then every page is rendered per locale: an untranslated segment keeps
//! the source text, a page whose content would not change is not rewritten.
//! Pages edited by hand are read back: a changed segment becomes a human translation (protected
//! from retranslation); a page whose structure no longer matches is left alone (`--force`).
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use glot_core::catalog::{Catalog, Format, Layout};
use glot_core::config::{Config, I18nextConfig, Separator};
use glot_core::lock::Lockfile;
use glot_core::markdown::Doc;
use glot_core::validate::Syntax;
use glot_llm::engine::{TranslateOptions, Workspace};

use crate::style::*;

pub const DIR: &str = ".glot-docs";
const RENDERED: &str = "rendered.json";

#[derive(clap::Args)]
pub struct DocsArgs {
    /// Show the plan (pages, segments, estimated cost); write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(short, long)]
    pub yes: bool,
    /// Rewrite translated pages edited by hand whose structure no longer matches their source.
    #[arg(long)]
    pub force: bool,
    /// Only these target locales (comma separated).
    #[arg(long, value_delimiter = ',')]
    pub locale: Vec<String>,
    /// LLM provider for this run (`mock`: offline, free, no API key).
    #[arg(long, value_parser = ["mock", "anthropic", "openai", "openai-compatible"])]
    pub provider: Option<String>,
}

#[derive(Deserialize, Default)]
struct DocsFile {
    #[serde(default)]
    docs: Vec<Entry>,
}

#[derive(Deserialize, Clone)]
struct Entry {
    source: String,
    target: String,
    #[serde(default)]
    exclude: Vec<String>,
}

struct Page {
    /// Source path, relative to the project root.
    rel: String,
    doc: Doc,
    /// Catalog key of each segment.
    keys: Vec<String>,
    /// locale => translated path, relative to the root.
    targets: BTreeMap<String, String>,
}

/// locale => translated page => keys of its segments, as last written by glot.
type Rendered = BTreeMap<String, BTreeMap<String, Vec<String>>>;

/// Static folder of a glob (`docs/**/*.md` => `docs/`).
fn glob_base(glob: &str) -> &str {
    let wild = glob.find(['*', '?', '[']).unwrap_or(glob.len());
    glob[..wild].rfind('/').map_or("", |i| &glob[..=i])
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn pages(root: &Path, entries: &[Entry], config: &Config) -> Result<Vec<Page>> {
    let opts = glob::MatchOptions {
        require_literal_leading_dot: true,
        ..Default::default()
    };
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    for e in entries {
        let base = glob_base(&e.source);
        let excludes: Vec<glob::Pattern> = e
            .exclude
            .iter()
            .filter_map(|x| glob::Pattern::new(x).ok())
            .collect();
        let pattern = root.join(&e.source).to_string_lossy().into_owned();
        for p in glob::glob_with(&pattern, opts)
            .with_context(|| format!("invalid [[docs]] source `{}`", e.source))?
            .flatten()
        {
            let r = rel(root, &p);
            let Some(under) = r.strip_prefix(base) else {
                continue;
            };
            let Some((noext, ext)) = under.rsplit_once('.') else {
                continue;
            };
            if !p.is_file()
                || !matches!(ext, "md" | "mdx" | "markdown")
                || under.split('/').any(|s| s == "node_modules")
                || excludes.iter().any(|x| x.matches(&r))
                // Translations next to their source: `fr/intro.md`, `intro.fr.md`.
                || config.target_locales.iter().any(|l| {
                    under.starts_with(&format!("{l}/")) || noext.ends_with(&format!(".{l}"))
                })
                || !seen.insert(r.clone())
            {
                continue;
            }
            let text = std::fs::read_to_string(&p).with_context(|| r.clone())?;
            let doc = Doc::parse(&text);
            let keys = doc
                .segments()
                .iter()
                .map(|s| format!("{r}#{}", &glot_core::hash::hash(&s.text)[..12]))
                .collect();
            let targets = config
                .target_locales
                .iter()
                .map(|l| {
                    let t = e
                        .target
                        .replace("{locale}", l)
                        .replace("{path}", noext)
                        .replace("{ext}", ext);
                    (l.clone(), t)
                })
                .filter(|(_, t)| *t != r)
                .collect();
            out.push(Page {
                rel: r,
                doc,
                keys,
                targets,
            });
        }
    }
    Ok(out)
}

fn docs_config(main: &Config) -> Config {
    let mut c = main.clone();
    c.files = vec!["{locale}.json".into()];
    c.format = Format::JsonFlat;
    c.syntax = Syntax::Icu;
    c.server = None;
    c.i18next = I18nextConfig {
        key_separator: Some(Separator::Off(false)),
        ns_separator: None,
    };
    // Pages are not code: no code context to extract.
    c.extract = glot_core::config::ExtractConfig {
        include: vec![],
        exclude: vec![],
        functions: vec![],
    };
    c
}

fn layout() -> Layout {
    Layout {
        key_separator: String::new(),
        ..Layout::default()
    }
}

const NOTE: &str = "From a Markdown page. {0}, {1}… stand for code, links, tags or shortcodes: keep every one. Keep the Markdown markup (**bold**, _italic_, [link text]) around the same words.";

pub async fn run(a: DocsArgs) -> Result<ExitCode> {
    let mut main = crate::open()?;
    let saved = docs_config(&main.config);
    if let Some(p) = &a.provider {
        main.config.llm.provider = p.clone();
    }
    let text = std::fs::read_to_string(main.root.join(glot_core::config::CONFIG_FILE))?;
    let entries = toml::from_str::<DocsFile>(&text)
        .context("glot.toml: invalid [[docs]]")?
        .docs;
    if entries.is_empty() {
        bail!(
            "no [[docs]] in glot.toml. Add one per folder of Markdown pages, e.g.\n\n[[docs]]\nsource = \"docs/**/*.md*\"\ntarget = \"i18n/{{locale}}/docusaurus-plugin-content-docs/current/{{path}}.{{ext}}\"\n\n(`{{path}}` = path under the glob's folder without extension; `glot init` writes it for Docusaurus, VitePress, Starlight, MkDocs and Hugo)"
        );
    }
    let root = main.root.clone();
    let mut config = docs_config(&main.config);
    if !a.locale.is_empty() {
        config.target_locales.retain(|l| a.locale.contains(l));
    }
    let pages = pages(&root, &entries, &config)?;
    if pages.is_empty() {
        bail!("[[docs]]: no Markdown page matches the source globs");
    }
    let dir = root.join(DIR);
    // ponytail: created even by --dry-run (empty), the pipeline resolves paths under it.
    std::fs::create_dir_all(&dir)?;
    let src_locale = config.source_locale.clone();
    let mut catalogs = BTreeMap::new();
    for loc in config.all_locales() {
        catalogs.insert(loc.clone(), Catalog::load(&config, &dir, &loc)?);
    }
    let mut ws = Workspace {
        root: dir.clone(),
        config,
        glossary: main.glossary.clone(),
        lock: Lockfile::load(&dir)?,
        catalogs,
        visual: BTreeMap::new(),
        progress: None,
        batch: false,
    };

    // Source segments, keyed by page; translations of segments gone from the source are dropped.
    let mut source = BTreeMap::new();
    let mut seg_of = BTreeMap::new();
    for p in &pages {
        for (i, k) in p.keys.iter().enumerate() {
            source.insert(k.clone(), p.doc.segments()[i].text.clone());
            seg_of.insert(k.clone(), (p, i));
            ws.lock
                .notes
                .entry(k.clone())
                .or_insert_with(|| format!("{NOTE} Page: {}.", p.rel));
        }
    }
    ws.lock.notes.retain(|k, _| source.contains_key(k));
    for (loc, cat) in ws.catalogs.iter_mut() {
        if *loc == src_locale {
            cat.entries = source.clone();
        } else {
            cat.entries.retain(|k, _| source.contains_key(k));
        }
    }
    if !a.dry_run {
        saved.save(&dir)?;
        for cat in ws.catalogs.values() {
            cat.write_with(
                &dir.join(format!("{}.json", cat.locale)),
                Format::JsonFlat,
                &layout(),
            )?;
        }
        ws.lock.save(&dir)?;
    }

    // Pages edited by hand (or translated before glot): read back into the catalogs.
    let rendered_path = dir.join(RENDERED);
    let mut rendered: Rendered = std::fs::read_to_string(&rendered_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut protected: BTreeSet<(String, String)> = BTreeSet::new();
    let mut imported = 0usize;
    for p in &pages {
        for (loc, target) in &p.targets {
            let Ok(text) = std::fs::read_to_string(root.join(target)) else {
                continue;
            };
            let tdoc = Doc::parse(&text);
            let prev = rendered.get(loc).and_then(|m| m.get(target));
            let keys = prev.unwrap_or(&p.keys);
            if tdoc.segments().len() != keys.len() {
                if !a.force {
                    protected.insert((loc.clone(), target.clone()));
                }
                continue;
            }
            for (i, k) in keys.iter().enumerate() {
                let Some((sp, si)) = seg_of.get(k) else {
                    continue;
                };
                let src = &sp.doc.segments()[*si];
                let Some(t) = tdoc.segments()[i].renumbered(src) else {
                    continue;
                };
                let cur = ws.catalogs.get(loc).and_then(|c| c.entries.get(k));
                // Untranslated segments render the source text; unchanged ones match the catalog.
                // A page never written by glot only fills segments the catalog lacks.
                if t == src.text || cur == Some(&t) || (prev.is_none() && cur.is_some()) {
                    continue;
                }
                if a.dry_run {
                    imported += 1;
                    continue;
                }
                let issues = ws.set_translation(loc, k, &t)?;
                if glot_core::validate::has_errors(&issues) {
                    anstream::println!(
                        "{YELLOW}warning:{YELLOW:#} {target}: edited segment {} kept out ({})",
                        i + 1,
                        issues[0].message
                    );
                } else {
                    imported += 1;
                }
            }
        }
    }

    let segs: usize = pages.iter().map(|p| p.keys.len()).sum();
    anstream::println!(
        "{BOLD}{}{BOLD:#} pages, {BOLD}{segs}{BOLD:#} segments ({} unique){}",
        pages.len(),
        source.len(),
        if imported > 0 {
            format!(", {imported} hand edits read back")
        } else {
            String::new()
        }
    );
    for (loc, t) in &protected {
        anstream::println!(
            "{YELLOW}skipped{YELLOW:#} {t} ({loc}): edited by hand and no longer matches its source's structure; fix it or pass --force to regenerate it"
        );
    }

    let opts = TranslateOptions {
        locales: None,
        keys: None,
        force: false,
        dry_run: true,
    };
    let dry = ws.translate(opts.clone()).await?;
    let total: usize = dry.planned.values().map(Vec::len).sum();
    if a.dry_run {
        anstream::println!(
            "{total} segments to translate {DIM}(est. ${:.4}){DIM:#}; nothing written (dry run)",
            dry.estimated_cost_usd
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut rejected = 0;
    if total > 0 {
        let max = dry.estimated_cost_max_usd.max(dry.estimated_cost_usd);
        anstream::println!(
            "Translating {BOLD}{total}{BOLD:#} segments {DIM}(est. ${:.4}, up to ${max:.4}){DIM:#}",
            dry.estimated_cost_usd
        );
        if max > 1.0
            && !a.yes
            && !crate::confirm(&format!("Estimated cost up to ${max:.2}. Continue?"))?
        {
            bail!("aborted");
        }
        let report = ws
            .translate(TranslateOptions {
                dry_run: false,
                ..opts
            })
            .await?;
        rejected = report.rejected.values().map(Vec::len).sum::<usize>();
        for (loc, items) in &report.written {
            anstream::println!("  {GREEN}✓{GREEN:#} {loc:<6} {} written", items.len());
        }
    }

    let mut written = 0;
    for p in &pages {
        for (loc, target) in &p.targets {
            if protected.contains(&(loc.clone(), target.clone())) {
                continue;
            }
            let cat = ws.catalogs.get(loc);
            let tr = |i: usize| cat.and_then(|c| c.entries.get(&p.keys[i]).cloned());
            let out = p.doc.render(&tr, Some((&src_locale, loc)));
            let path = root.join(target);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(out.as_str()) {
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&path, &out)?;
                written += 1;
            }
            rendered
                .entry(loc.clone())
                .or_default()
                .insert(target.clone(), p.keys.clone());
        }
    }
    std::fs::write(
        &rendered_path,
        serde_json::to_string_pretty(&rendered)? + "\n",
    )?;
    anstream::println!(
        "{GREEN}✓{GREEN:#} {written} pages written {DIM}(translations in {DIR}/, `cd {DIR} && glot check` to review){DIM:#}"
    );
    if rejected > 0 {
        anstream::println!(
            "{YELLOW}{rejected} segments rejected by the validator{YELLOW:#}: they keep the source text; run `glot docs` again to retry"
        );
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_bases() {
        assert_eq!(glob_base("docs/**/*.md*"), "docs/");
        assert_eq!(
            glob_base("src/content/docs/en/**/*.md"),
            "src/content/docs/en/"
        );
        assert_eq!(glob_base("**/*.md"), "");
    }

    #[test]
    fn pages_skip_translations_and_map_targets() {
        let d = std::env::temp_dir().join(format!("glot-docs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (p, c) in [
            ("docs/intro.md", "# Intro\n"),
            ("docs/guide/setup.mdx", "Setup\n"),
            ("docs/fr/intro.md", "# Intro FR\n"),
            ("docs/about.fr.md", "x\n"),
            ("docs/.vitepress/x.md", "x\n"),
        ] {
            std::fs::create_dir_all(d.join(p).parent().unwrap()).unwrap();
            std::fs::write(d.join(p), c).unwrap();
        }
        let config: Config = toml::from_str(
            "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"x/{locale}.json\"]\n",
        )
        .unwrap();
        let e = Entry {
            source: "docs/**/*.md*".into(),
            target: "i18n/{locale}/current/{path}.{ext}".into(),
            exclude: vec![],
        };
        let ps = pages(&d, &[e], &config).unwrap();
        let got: Vec<(&str, &str)> = ps
            .iter()
            .map(|p| (p.rel.as_str(), p.targets["fr"].as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("docs/guide/setup.mdx", "i18n/fr/current/guide/setup.mdx"),
                ("docs/intro.md", "i18n/fr/current/intro.md"),
            ]
        );
        assert!(ps[1].keys[0].starts_with("docs/intro.md#"));
        std::fs::remove_dir_all(d).unwrap();
    }
}
