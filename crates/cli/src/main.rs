//! `glot` CLI: thin layer over `glot_llm::engine::Workspace`.
mod dev;
mod fit;
mod i18nize;
mod init;
mod measure;
mod parity;
mod review;
mod setup;
mod sync;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use glot_core::validate::{Issue, IssueCode, Severity};
use glot_llm::engine::{Finding, TranslateOptions, Workspace};

pub mod style {
    use anstyle::{AnsiColor, Style};
    pub const BOLD: Style = Style::new().bold();
    pub const DIM: Style = Style::new().dimmed();
    pub const GREEN: Style = AnsiColor::Green.on_default();
    pub const YELLOW: Style = AnsiColor::Yellow.on_default();
    pub const RED: Style = AnsiColor::Red.on_default();
}
use style::*;

#[derive(Parser)]
#[command(name = "glot", version, about = "AI-native localization for SaaS apps")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Detect the i18n stack and write glot.toml + AI agent integration.
    Init(init::InitArgs),
    /// Per-locale coverage and translation plan.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Validate every translation (placeholders, plurals, glossary). Exit 1 on errors.
    Check {
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Translate missing and stale strings with the configured LLM.
    Translate {
        /// Locales to translate (comma separated). Default: all targets.
        #[arg(long, value_delimiter = ',')]
        locale: Vec<String>,
        /// Only these keys (comma separated or repeated).
        #[arg(long, value_delimiter = ',')]
        key: Vec<String>,
        /// Retranslate up-to-date and hand-edited entries too.
        #[arg(long)]
        force: bool,
        /// Show the plan and estimated cost only.
        #[arg(long)]
        dry_run: bool,
        /// Do not ask for confirmation.
        #[arg(short, long)]
        yes: bool,
        /// Capture the running app and show the LLM where each key renders (`--base-url`, `--routes`).
        #[arg(long)]
        visual: bool,
        /// First pass through the Anthropic Message Batches API: -50 %, asynchronous (minutes to hours).
        /// Resumable: rerun the same command if interrupted.
        #[arg(long)]
        batch: bool,
        #[command(flatten)]
        qa: fit::QaArgs,
    },
    /// Shorten translations that overflow in the running app, re-render, repeat until they fit.
    Fit(fit::FitArgs),
    /// Record each key's pixel budget (box width, lines, font) from the running app into .glot/layout.json.
    Measure(measure::MeasureArgs),
    /// Hardcoded strings in code, plus missing and unused keys.
    Extract {
        #[arg(long)]
        json: bool,
    },
    /// Source keys never referenced in code.
    Unused {
        /// Delete them from every catalog.
        #[arg(long)]
        prune: bool,
        #[arg(long)]
        json: bool,
    },
    /// Add or replace a source string.
    Add {
        key: String,
        value: String,
        /// Context for translators (stored in glot.lock).
        #[arg(long)]
        note: Option<String>,
    },
    /// Set a translation by hand (validated).
    Set {
        locale: String,
        key: String,
        value: String,
    },
    /// Manage glossary.toml.
    #[command(subcommand)]
    Glossary(GlossaryCmd),
    /// Back-translate translations and explain them; `--stale` sorts cosmetic vs real source changes.
    Review(review::ReviewArgs),
    /// Upload catalogs, lock and glossary to the glot server.
    Push,
    /// Download catalogs, lock and glossary from the glot server.
    Pull,
    /// Run the MCP server on stdio.
    Mcp,
    /// Convert hardcoded UI strings to translation keys and rewrite the code (next-intl / react-i18next).
    I18nize(i18nize::I18nizeArgs),
    /// Local server for the in-app editing overlay (@glot/overlay).
    Dev(dev::DevArgs),
    /// Task parity: an agent runs the same tasks in every locale (glot.tasks.yaml) and compares success.
    Parity(parity::ParityArgs),
}

#[derive(Subcommand)]
enum GlossaryCmd {
    List {
        #[arg(long)]
        json: bool,
    },
    /// `glot glossary add Workspace --fr "Espace de travail" --de Arbeitsbereich [--dnt] [--description ..]`
    Add {
        term: String,
        /// `--<locale> <translation>`, `--dnt`, `--case-sensitive`, `--description <text>`.
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "OPTIONS"
        )]
        rest: Vec<String>,
    },
    /// Frequent capitalized terms translated inconsistently.
    /// Mine glossary terms with the LLM and list inconsistent translations (`--apply` fixes them).
    Mine(review::MineArgs),
    Suggest {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Sarif,
    Github,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(Cli::parse().cmd).await {
        Ok(code) => code,
        Err(e) => {
            anstream::eprintln!("{RED}error:{RED:#} {e:#}");
            ExitCode::from(2)
        }
    }
}

fn open() -> Result<Workspace> {
    let cwd = std::env::current_dir()?;
    Workspace::open(&cwd).context("cannot open glot workspace (run `glot init` first?)")
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

async fn run(cmd: Cmd) -> Result<ExitCode> {
    match cmd {
        Cmd::Init(args) => init::run(args)?,
        Cmd::Status { json } => status(json)?,
        Cmd::Check { format } => return check(format),
        Cmd::Translate {
            locale,
            key,
            force,
            dry_run,
            yes,
            visual,
            batch,
            qa,
        } => {
            translate(
                locale,
                key,
                force,
                dry_run,
                yes,
                visual.then_some(qa),
                batch,
            )
            .await?
        }
        Cmd::Fit(a) => return fit::fit(a).await,
        Cmd::Measure(a) => return measure::run(a),
        Cmd::Extract { json } => extract(json)?,
        Cmd::Unused { prune, json } => unused(prune, json)?,
        Cmd::Add { key, value, note } => {
            let mut ws = open()?;
            let p = ws.propose(&value, Some(&key));
            for (k, v) in p.duplicates.iter().filter(|(k, _)| *k != key) {
                anstream::eprintln!(
                    "{YELLOW}warning:{YELLOW:#} same text already exists as {BOLD}{k}{BOLD:#} = {v:?}"
                );
            }
            ws.add_source(&key, &value, note.as_deref())?;
            anstream::println!(
                "{GREEN}added{GREEN:#} {key} = {value:?}  {DIM}run `glot translate` to fill targets{DIM:#}"
            );
        }
        Cmd::Set { locale, key, value } => {
            let issues = open()?.set_translation(&locale, &key, &value)?;
            print_issues(&locale, &key, &issues);
            if glot_core::validate::has_errors(&issues) {
                anstream::eprintln!("{RED}rejected:{RED:#} nothing written");
                return Ok(ExitCode::FAILURE);
            }
            anstream::println!("{GREEN}set{GREEN:#} {locale} {key} = {value:?}");
        }
        Cmd::Glossary(GlossaryCmd::Mine(a)) => review::mine(a).await?,
        Cmd::Glossary(g) => glossary(g)?,
        Cmd::Review(a) => return review::run(a).await,
        Cmd::Push => return sync::push().await,
        Cmd::Pull => return sync::pull().await,
        Cmd::Mcp => glot_mcp::serve_stdio(std::env::current_dir()?).await?,
        Cmd::I18nize(args) => i18nize::run(args).await?,
        Cmd::Dev(args) => dev::run(args).await?,
        Cmd::Parity(args) => return parity::run(args),
    }
    Ok(ExitCode::SUCCESS)
}

fn status(json: bool) -> Result<()> {
    let st = open()?.status();
    if json {
        return print_json(&st);
    }
    anstream::println!(
        "{BOLD}{} keys{BOLD:#} in source locale {BOLD}{}{BOLD:#}\n",
        st.keys,
        st.source_locale
    );
    anstream::println!(
        "{BOLD}{:<10} {:>10} {:>8} {:>6} {:>9} {:>7} {:>9}{BOLD:#}",
        "locale",
        "translated",
        "missing",
        "stale",
        "reviewed",
        "errors",
        "warnings"
    );
    for (loc, s) in &st.locales {
        let pct = (s.translated * 100).checked_div(s.total).unwrap_or(100);
        let c = if s.errors > 0 {
            RED
        } else if s.missing + s.stale > 0 {
            YELLOW
        } else {
            GREEN
        };
        anstream::println!(
            "{c}{loc:<10}{c:#} {:>10} {:>8} {:>6} {:>9} {:>7} {:>9}",
            format!("{}/{} {pct}%", s.translated, s.total),
            s.missing,
            s.stale,
            s.reviewed,
            s.errors,
            s.warnings
        );
    }
    let p = &st.plan;
    let todo = p.missing.len() + p.stale.len();
    println!();
    if todo == 0 {
        anstream::println!("{GREEN}Up to date.{GREEN:#}");
    } else {
        anstream::println!(
            "Plan: {} missing, {} stale → {BOLD}glot translate{BOLD:#}",
            p.missing.len(),
            p.stale.len()
        );
    }
    for (n, label) in [
        (p.manual_edits.len(), "edited by hand"),
        (p.untracked.len(), "untracked (imported)"),
        (p.orphaned.len(), "orphaned (not in source)"),
    ] {
        if n > 0 {
            anstream::println!("{DIM}{n} {label}{DIM:#}");
        }
    }
    Ok(())
}

fn code_str(code: IssueCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

fn print_issues(locale: &str, key: &str, issues: &[Issue]) {
    for i in issues {
        let (c, sym) = if i.severity == Severity::Error {
            (RED, "✗")
        } else {
            (YELLOW, "!")
        };
        anstream::println!(
            "  {c}{sym}{c:#} {locale:<6} {key}  {DIM}{}{DIM:#} {}",
            code_str(i.code),
            i.message
        );
    }
}

/// Catalog file (relative to root) and 1-based line where `key` is defined, best effort.
fn locate(ws: &Workspace, locale: &str, key: &str) -> (String, Option<usize>) {
    let paths = ws.config.catalog_paths(&ws.root, locale);
    let rel = |p: &std::path::Path| {
        p.strip_prefix(&ws.root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        // Flat file: the dotted key itself.
        if let Some(n) = lines
            .iter()
            .position(|l| l.contains(&format!("\"{key}\":")))
        {
            return (rel(p), Some(n + 1));
        }
        // Nested file: find each segment after its parent's line, so `jobs.empty.title`
        // doesn't land on the first `"title"` of the file.
        let mut from = 0;
        let found = key.split('.').all(|seg| {
            let needle = format!("\"{seg}\":");
            match lines[from..].iter().position(|l| l.contains(&needle)) {
                Some(i) => {
                    from += i;
                    true
                }
                None => false,
            }
        });
        if found {
            return (rel(p), Some(from + 1));
        }
    }
    (paths.first().map(|p| rel(p)).unwrap_or_default(), None)
}

fn gh_escape(s: &str, prop: bool) -> String {
    let s = s
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A");
    if prop {
        s.replace(':', "%3A").replace(',', "%2C")
    } else {
        s
    }
}

fn check(format: Format) -> Result<ExitCode> {
    let ws = open()?;
    let findings = ws.check();
    let errors = findings
        .iter()
        .filter(|f| f.issue.severity == Severity::Error)
        .count();
    match format {
        Format::Json => print_json(&findings)?,
        Format::Text => {
            let mut by_locale: BTreeMap<&str, Vec<&Finding>> = BTreeMap::new();
            for f in &findings {
                by_locale.entry(&f.locale).or_default().push(f);
            }
            for fs in by_locale.values() {
                for f in fs {
                    print_issues(&f.locale, &f.key, std::slice::from_ref(&f.issue));
                }
            }
            let warnings = findings.len() - errors;
            if errors == 0 {
                anstream::println!(
                    "{GREEN}✓ check passed{GREEN:#} {DIM}({warnings} warnings){DIM:#}"
                );
            } else {
                anstream::println!("{RED}✗ {errors} errors{RED:#}, {warnings} warnings");
            }
        }
        Format::Github => {
            for f in &findings {
                let (file, line) = locate(&ws, &f.locale, &f.key);
                let level = if f.issue.severity == Severity::Error {
                    "error"
                } else {
                    "warning"
                };
                let line = line.map(|l| format!(",line={l}")).unwrap_or_default();
                println!(
                    "::{level} file={}{line},title={}::{}",
                    gh_escape(&file, true),
                    gh_escape(&format!("glot {}", code_str(f.issue.code)), true),
                    gh_escape(
                        &format!("[{}] {}: {}", f.locale, f.key, f.issue.message),
                        false
                    )
                );
            }
        }
        Format::Sarif => {
            let results: Vec<_> = findings
                .iter()
                .map(|f| {
                    let (file, line) = locate(&ws, &f.locale, &f.key);
                    let mut loc = serde_json::json!({ "artifactLocation": { "uri": file } });
                    if let Some(l) = line {
                        loc["region"] = serde_json::json!({ "startLine": l });
                    }
                    serde_json::json!({
                        "ruleId": code_str(f.issue.code),
                        "level": if f.issue.severity == Severity::Error { "error" } else { "warning" },
                        "message": { "text": format!("[{}] {}: {}", f.locale, f.key, f.issue.message) },
                        "locations": [{ "physicalLocation": loc }],
                    })
                })
                .collect();
            print_json(&serde_json::json!({
                "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
                "version": "2.1.0",
                "runs": [{ "tool": { "driver": { "name": "glot", "version": env!("CARGO_PKG_VERSION") } }, "results": results }],
            }))?;
        }
    }
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("{question} (not a terminal: re-run with --yes)");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

async fn translate(
    locales: Vec<String>,
    keys: Vec<String>,
    force: bool,
    dry_run: bool,
    yes: bool,
    visual: Option<fit::QaArgs>,
    batch: bool,
) -> Result<()> {
    let mut ws = open()?;
    ws.batch = batch;
    ws.progress = Some(|m| anstream::eprintln!("{DIM}{m}{DIM:#}"));
    let opts = TranslateOptions {
        locales: (!locales.is_empty()).then_some(locales),
        keys: (!keys.is_empty()).then_some(keys),
        force,
        dry_run: true,
    };
    // The dry run is the single source of truth for what will be sent (handles --force, filters, manual edits).
    let dry = ws.translate(opts.clone()).await?;
    let total: usize = dry.planned.values().map(Vec::len).sum();
    if total == 0 {
        anstream::println!("{GREEN}Nothing to translate.{GREEN:#}");
        return Ok(());
    }
    let summary = dry
        .planned
        .iter()
        .filter(|(_, ks)| !ks.is_empty())
        .map(|(l, ks)| format!("{l} ({})", ks.len()))
        .collect::<Vec<_>>()
        .join(", ");
    anstream::println!(
        "Plan: {BOLD}{total}{BOLD:#} strings → {summary}  {DIM}model {}{DIM:#}",
        ws.config.llm.model
    );
    let (estimate, max) = (
        dry.estimated_cost_usd,
        dry.estimated_cost_max_usd.max(dry.estimated_cost_usd),
    );
    anstream::println!(
        "Estimated cost: {BOLD}${estimate:.4}{BOLD:#} {DIM}(up to ${max:.4}; ~{}k in / {}k out tokens){DIM:#}",
        dry.estimated_input_tokens / 1000,
        dry.estimated_output_tokens / 1000
    );
    if !batch && ws.config.llm.provider == "anthropic" {
        anstream::println!(
            "{DIM}With --batch: ~${:.4} (first pass at -50 %, asynchronous){DIM:#}",
            dry.estimated_batch_cost_usd
        );
    }
    if dry_run {
        return Ok(());
    }
    if max > 1.0
        && !yes
        && !confirm(&format!(
            "Estimated cost is ${estimate:.2} (up to ${max:.2}). Continue?"
        ))?
    {
        bail!("aborted");
    }
    if let Some(qa) = &visual {
        let keys: Vec<String> = dry.planned.values().flatten().cloned().collect();
        ws.visual = fit::capture(&ws, qa, &keys)?;
        let e = ws.translate(opts.clone()).await?;
        anstream::println!(
            "Estimated cost with screenshots: {BOLD}${:.4}{BOLD:#} {DIM}(up to ${:.4}){DIM:#}",
            e.estimated_cost_usd,
            e.estimated_cost_max_usd
        );
    }
    anstream::eprintln!("{DIM}Translating…{DIM:#}");
    let report = ws
        .translate(TranslateOptions {
            dry_run: false,
            ..opts
        })
        .await?;
    for (loc, items) in &report.written {
        anstream::println!("  {GREEN}✓{GREEN:#} {loc:<6} {} written", items.len());
    }
    for (loc, items) in &report.rejected {
        anstream::println!("  {RED}✗{RED:#} {loc:<6} {} rejected", items.len());
        for t in items {
            let reasons = t
                .issues
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            anstream::println!("      {}  {DIM}{reasons}{DIM:#}", t.key);
        }
    }
    let u = &report.usage;
    anstream::println!(
        "Cost: {BOLD}${:.4}{BOLD:#} {DIM}({} in + {} cache write + {} cache read / {} out tokens){DIM:#}",
        u.cost_usd,
        u.input_tokens,
        u.cache_creation_tokens,
        u.cache_read_tokens,
        u.output_tokens
    );
    Ok(())
}

fn extract(json: bool) -> Result<()> {
    let ws = open()?;
    let report = ws.extract()?;
    let missing = glot_extract::missing_keys(&report, ws.source());
    let unused = glot_extract::unused_keys(&report, ws.source());
    if json {
        return print_json(&serde_json::json!({
            "files_scanned": report.files_scanned,
            "hardcoded": report.hardcoded,
            "missing": missing,
            "unused": unused,
            "dynamic": report.dynamic,
        }));
    }
    anstream::println!("{DIM}{} files scanned{DIM:#}", report.files_scanned);
    if !report.hardcoded.is_empty() {
        anstream::println!(
            "\n{BOLD}Hardcoded strings ({}){BOLD:#}",
            report.hardcoded.len()
        );
        for h in &report.hardcoded {
            anstream::println!(
                "  {}:{}  {:?}  {DIM}→ {}{DIM:#}",
                h.at.path,
                h.at.line,
                h.text,
                h.suggested_key
            );
        }
    }
    if !missing.is_empty() {
        anstream::println!(
            "\n{RED}{BOLD}Missing from source catalog ({}){BOLD:#}{RED:#}",
            missing.len()
        );
        for k in &missing {
            let at = report
                .usages
                .get(k)
                .and_then(|u| u.first())
                .map(|u| format!("{}:{}", u.at.path, u.at.line));
            anstream::println!("  {k}  {DIM}{}{DIM:#}", at.unwrap_or_default());
        }
    }
    if !unused.is_empty() {
        anstream::println!(
            "\n{YELLOW}{BOLD}Unused keys ({}){BOLD:#}{YELLOW:#}",
            unused.len()
        );
        for k in &unused {
            println!("  {k}");
        }
    }
    if report.hardcoded.is_empty() && missing.is_empty() && unused.is_empty() {
        anstream::println!("{GREEN}✓ nothing to report{GREEN:#}");
    }
    Ok(())
}

fn unused(prune: bool, json: bool) -> Result<()> {
    let mut ws = open()?;
    let report = ws.extract()?;
    let keys = glot_extract::unused_keys(&report, ws.source());
    if json {
        print_json(&keys)?;
    } else {
        for k in &keys {
            println!("{k}");
        }
        if keys.is_empty() {
            anstream::println!("{GREEN}✓ no unused keys{GREEN:#}");
        }
    }
    if prune && !keys.is_empty() {
        // Safety net: extract only sees `[extract] include`; never delete a key whose leaf
        // still appears quoted anywhere in the repo's sources.
        let leaves: Vec<String> = keys
            .iter()
            .map(|k| k.rsplit('.').next().unwrap_or(k).to_string())
            .collect();
        let found = quoted_anywhere(&ws.root, &leaves);
        let (kept, prunable): (Vec<_>, Vec<_>) = keys
            .iter()
            .zip(&leaves)
            .partition(|(_, l)| found.contains(*l));
        for (k, _) in &kept {
            anstream::eprintln!(
                "{YELLOW}kept{YELLOW:#} {k} (its name appears in a source file outside [extract] include)"
            );
        }
        let prunable: Vec<String> = prunable.into_iter().map(|(k, _)| k.clone()).collect();
        if !prunable.is_empty() {
            ws.remove_keys(&prunable)?;
        }
        anstream::eprintln!(
            "{GREEN}pruned{GREEN:#} {} keys from every catalog",
            prunable.len()
        );
    }
    Ok(())
}

/// Leaves found as `"leaf"`, `'leaf'` or `` `leaf` `` in any JS/TS/Vue/Svelte file under `root`.
fn quoted_anywhere(
    root: &std::path::Path,
    leaves: &[String],
) -> std::collections::BTreeSet<String> {
    const SKIP: &[&str] = &["node_modules", ".next", "dist", "build", "target", ".git"];
    const EXTS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "vue", "svelte"];
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|x| EXTS.contains(&x.to_string_lossy().as_ref()))
                && let Ok(src) = std::fs::read_to_string(&path)
            {
                for l in leaves {
                    if [format!("\"{l}\""), format!("'{l}'"), format!("`{l}`")]
                        .iter()
                        .any(|q| src.contains(q.as_str()))
                    {
                        found.insert(l.clone());
                    }
                }
            }
        }
    }
    found
}

fn glossary(cmd: GlossaryCmd) -> Result<()> {
    let mut ws = open()?;
    match cmd {
        GlossaryCmd::List { json } => {
            if json {
                return print_json(&ws.glossary.terms);
            }
            for t in &ws.glossary.terms {
                let tr = t
                    .translations
                    .iter()
                    .map(|(l, v)| format!("{l}={v}"))
                    .collect::<Vec<_>>()
                    .join("  ");
                let dnt = if t.do_not_translate {
                    format!("{YELLOW}do-not-translate{YELLOW:#}")
                } else {
                    String::new()
                };
                anstream::println!(
                    "{BOLD}{}{BOLD:#}  {dnt}{tr}  {DIM}{}{DIM:#}",
                    t.term,
                    t.description.as_deref().unwrap_or("")
                );
            }
            if ws.glossary.terms.is_empty() {
                println!("glossary is empty: `glot glossary add <term> --fr ...`");
            }
        }
        GlossaryCmd::Add { term, rest } => {
            let t = parse_term(term, &rest)?;
            anstream::println!("{GREEN}saved{GREEN:#} {}", t.term);
            ws.glossary.upsert(t);
            ws.save_glossary()?;
        }
        GlossaryCmd::Mine(_) => unreachable!("handled in run"),
        GlossaryCmd::Suggest { json } => {
            let s = suggest(&ws);
            if json {
                return print_json(&s);
            }
            for x in &s {
                anstream::println!(
                    "{BOLD}{}{BOLD:#} {DIM}×{}{DIM:#}  {}",
                    x.term,
                    x.occurrences,
                    x.hint
                );
            }
            if s.is_empty() {
                anstream::println!("{GREEN}no suggestions{GREEN:#}");
            }
        }
    }
    Ok(())
}

fn parse_term(term: String, rest: &[String]) -> Result<glot_core::glossary::Term> {
    let mut t = glot_core::glossary::Term {
        term,
        ..Default::default()
    };
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let Some(name) = arg.strip_prefix("--") else {
            bail!("unexpected argument `{arg}`")
        };
        match name {
            "dnt" | "do-not-translate" => t.do_not_translate = true,
            "case-sensitive" => t.case_sensitive = true,
            _ => {
                let v = it
                    .next()
                    .with_context(|| format!("missing value for --{name}"))?
                    .clone();
                if name == "description" {
                    t.description = Some(v);
                } else {
                    t.translations.insert(name.to_string(), v);
                }
            }
        }
    }
    Ok(t)
}

#[derive(serde::Serialize)]
struct Suggestion {
    term: String,
    occurrences: usize,
    /// Locales where the term is kept verbatim in some targets but not others.
    inconsistent: Vec<String>,
    hint: String,
}

/// ponytail: single capitalized words not at sentence start; multi-word terms need real NLP.
fn suggest(ws: &Workspace) -> Vec<Suggestion> {
    let known: Vec<String> = ws
        .glossary
        .terms
        .iter()
        .map(|t| t.term.to_lowercase())
        .collect();
    let mut counts: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (key, text) in &ws.source().entries {
        let mut sentence_start = true;
        for word in text.split(|c: char| !c.is_alphanumeric() && c != '.' && c != '!' && c != '?') {
            let clean = word.trim_end_matches(['.', '!', '?']);
            if !sentence_start
                && clean.len() > 2
                && clean.starts_with(|c: char| c.is_uppercase())
                && !known.contains(&clean.to_lowercase())
            {
                counts.entry(clean.to_string()).or_default().push(key);
            }
            if !word.is_empty() {
                sentence_start = word.ends_with(['.', '!', '?']);
            }
        }
    }
    let mut out: Vec<Suggestion> = counts
        .into_iter()
        .filter(|(_, keys)| keys.len() >= 2)
        .map(|(term, keys)| {
            let mut inconsistent = vec![];
            let mut verbatim_everywhere = true;
            for (loc, cat) in ws
                .catalogs
                .iter()
                .filter(|(l, _)| **l != ws.config.source_locale)
            {
                let hits: Vec<bool> = keys
                    .iter()
                    .filter_map(|k| cat.entries.get(*k))
                    .map(|v| v.contains(&term))
                    .collect();
                if hits.iter().any(|h| *h) && !hits.iter().all(|h| *h) {
                    inconsistent.push(loc.clone());
                }
                verbatim_everywhere &= !hits.is_empty() && hits.iter().all(|h| *h);
            }
            let hint = if !inconsistent.is_empty() {
                format!(
                    "inconsistent in {}: add a translation",
                    inconsistent.join(", ")
                )
            } else if verbatim_everywhere {
                "always kept verbatim: `--dnt`?".into()
            } else {
                "frequent term".into()
            };
            Suggestion {
                term,
                occurrences: keys.len(),
                inconsistent,
                hint,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.inconsistent
            .len()
            .cmp(&a.inconsistent.len())
            .then(b.occurrences.cmp(&a.occurrences))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_flags() {
        let a: Vec<String> = ["--fr", "Espace", "--dnt", "--description", "x"]
            .map(String::from)
            .into();
        let t = parse_term("Workspace".into(), &a).unwrap();
        assert!(t.do_not_translate);
        assert_eq!(t.translations["fr"], "Espace");
        assert_eq!(t.description.as_deref(), Some("x"));
        assert!(parse_term("x".into(), &["--fr".into()]).is_err());
    }

    #[test]
    fn github_escape() {
        assert_eq!(gh_escape("a:b,c%\n", true), "a%3Ab%2Cc%25%0A");
    }
}
