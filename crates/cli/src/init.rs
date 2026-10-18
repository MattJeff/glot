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
    ];
    if !js.iter().any(|n| has(n))
        && let Some(d) = native::detect(root)
    {
        return d;
    }
    let (stack, syntax, default_file) = if has("next-intl") {
        ("next-intl", "icu", "messages/{locale}.json")
    } else if has("react-i18next") || has("i18next") || has("next-i18next") {
        ("i18next", "i18next", "public/locales/{locale}/common.json")
    } else if has("react-intl") {
        ("react-intl", "icu", "lang/{locale}.json")
    } else if has("@lingui") {
        ("lingui", "icu", "src/locales/{locale}.json")
    } else if has("vue-i18n") {
        ("vue-i18n", "icu", "src/locales/{locale}.json")
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
                ("react-i18next", "i18next", file)
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

    let (mut files, locales) = find_catalogs(root);
    if files.is_empty() {
        files.push(default_file.to_string());
    }
    Detected {
        stack,
        syntax,
        files,
        locales,
    }
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
        "src/i18n",
        "lang",
        "src/lang",
        "i18n",
        "translations",
    ];
    for dir in DIRS {
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

    let mut source = args.source.clone().unwrap_or_else(|| {
        if d.locales.contains("en") || d.locales.is_empty() {
            "en".into()
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
            vec!["fr".into(), "es".into(), "de".into()]
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
    let block = agent_block(&source, &targets, &d.files);
    w.block("CLAUDE.md", &block, true)?;
    w.block("AGENTS.md", &block, false)?;
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
    let hardcoded = glot_llm::engine::Workspace::open(&root)
        .and_then(|ws| ws.extract())
        .map_or(0, |r| r.hardcoded.len());
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
    let ex = glot_core::config::ExtractConfig::default();
    let include: Vec<String> = ["src", "app", "components", "pages", "lib"]
        .iter()
        .filter(|d| root.join(d).is_dir())
        .map(|d| format!("{d}/**/*.{{ts,tsx,js,jsx}}"))
        .collect();
    let include = if include.is_empty() {
        ex.include
    } else {
        include
    };
    let mut s = format!(
        "# glot configuration. Docs: `glot --help`.\n\
         source_locale = \"{source}\"\n\
         target_locales = {}\n\
         # Catalog files, `{{locale}}` is replaced by each locale.\n\
         files = {}\n\
         format = \"auto\"   # auto | json-nested | json-flat | xcstrings | strings | android | arb | po | yaml\n\
         syntax = \"{}\"   # icu | i18next | mf2 | printf | rails\n\n\
         [llm]\n\
         provider = \"{}\"   # anthropic | openai | openai-compatible | mock\n\
         model = \"{model}\"\n",
        toml_list(targets),
        toml_list(&d.files),
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
    Ok(s)
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

fn agent_block(source: &str, targets: &[String], files: &[String]) -> String {
    let targets = targets.join(", ");
    let files = files
        .iter()
        .map(|f| format!("`{f}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{MARK_START}\n\
         ## Internationalization (glot)\n\n\
         UI strings are localized with glot. Source locale: `{source}`. Targets: {targets}. Catalogs: {files}.\n\n\
         - Write user-facing text only in the `{source}` catalog, never hardcoded in components.\n\
         - Before adding a key, call the MCP tool `propose_string` (server `glot`): it normalizes the key, finds duplicates and glossary terms. Without MCP: `glot add <key> \"<text>\" --note \"<context>\"`.\n\
         - Never edit target catalogs ({targets}) by hand: run `glot translate` (or `glot set <locale> <key> <value>` for a deliberate human fix).\n\
         - Run `glot check` before finishing; it must pass.\n\
         {MARK_END}\n"
    )
}

const SKILL_HEAD: &str = r#"---
name: glot
description: Add, change or translate user-facing strings in this project (i18n). Use when touching UI text, translation catalogs or glot.toml.
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
        assert_eq!((d.stack, d.syntax), ("react-i18next", "i18next"));
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
}
