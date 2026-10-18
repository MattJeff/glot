//! `glot i18nize`: turn hardcoded UI copy into catalog keys and rewrite the code.
use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use std::collections::BTreeMap;

use glot_llm::i18nize::{self, Lib, Plan};

use crate::style::*;

#[derive(Clone, Copy, ValueEnum)]
pub enum LibArg {
    Auto,
    NextIntl,
    ReactI18next,
}

#[derive(Args)]
pub struct I18nizeArgs {
    /// Files or directories to convert (default: every file matched by `[extract].include`).
    pub paths: Vec<String>,
    /// Show the diff and the keys that would be created; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(short, long)]
    pub yes: bool,
    /// i18n library the rewritten code targets.
    #[arg(long, value_enum, default_value_t = LibArg::Auto)]
    pub lib: LibArg,
}

pub async fn run(args: I18nizeArgs) -> Result<()> {
    let mut ws = crate::open()?;
    let lib = match args.lib {
        LibArg::NextIntl => Lib::NextIntl,
        LibArg::ReactI18next => Lib::ReactI18next,
        LibArg::Auto => match i18nize::detect_lib(&ws.root) {
            Some(l) => l,
            None => bail!(
                "cannot detect the i18n library from package.json; pass --lib next-intl|react-i18next"
            ),
        },
    };
    let cwd = std::env::current_dir()?;
    let paths: Vec<String> = args
        .paths
        .iter()
        .map(|p| cwd.join(p).to_string_lossy().into_owned())
        .collect();
    let files = i18nize::list_files(&ws, &paths)?;
    if files.is_empty() {
        bail!("no source files to convert (check the paths or [extract].include in glot.toml)");
    }
    anstream::eprintln!(
        "{DIM}Scanning {} files, naming keys with {}…{DIM:#}",
        files.len(),
        ws.config.llm.provider
    );
    let plan = i18nize::plan(&ws, &files, lib).await?;
    for w in &plan.warnings {
        anstream::eprintln!("{YELLOW}warning:{YELLOW:#} {w}");
    }
    if args.dry_run {
        for f in plan.changed_files() {
            print_diff(&f.diff);
            print_keys(f);
            println!();
        }
    }
    summary(&plan);
    if args.dry_run || plan.converted() == 0 {
        return Ok(());
    }
    if !args.yes
        && !crate::confirm(&format!(
            "Rewrite {} files and add the keys to the source catalog?",
            plan.changed_files().count()
        ))?
    {
        anstream::println!("Aborted, nothing written.");
        return Ok(());
    }
    i18nize::apply(&mut ws, &plan)?;
    anstream::println!(
        "{GREEN}✓ done.{GREEN:#} Next: {BOLD}glot translate{BOLD:#} to fill {}",
        ws.config.target_locales.join(", ")
    );
    if lib == Lib::NextIntl
        && !std::fs::read_to_string(ws.root.join("package.json"))
            .unwrap_or_default()
            .contains("\"next-intl\"")
    {
        anstream::println!(
            "{YELLOW}note:{YELLOW:#} add and configure next-intl (`pnpm add next-intl`, i18n/request.ts, NextIntlClientProvider)."
        );
    }
    Ok(())
}

fn print_diff(diff: &str) {
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            anstream::println!("{BOLD}{line}{BOLD:#}");
        } else if line.starts_with("@@") {
            anstream::println!("{DIM}{line}{DIM:#}");
        } else if line.starts_with('+') {
            anstream::println!("{GREEN}{line}{GREEN:#}");
        } else if line.starts_with('-') {
            anstream::println!("{RED}{line}{RED:#}");
        } else {
            println!("{line}");
        }
    }
}

fn print_keys(f: &i18nize::FilePlan) {
    for k in &f.keys {
        if k.reused {
            anstream::println!("  {DIM}= {} (reused){DIM:#}", k.key);
        } else {
            let note = k
                .note
                .as_deref()
                .map(|n| format!("  {n}"))
                .unwrap_or_default();
            anstream::println!(
                "  {GREEN}+{GREEN:#} {BOLD}{}{BOLD:#} = {:?}{DIM}{note}{DIM:#}",
                k.key,
                k.value
            );
        }
    }
}

fn summary(plan: &Plan) {
    let changed = plan.changed_files().count();
    let new = plan
        .files
        .iter()
        .flat_map(|f| &f.keys)
        .filter(|k| !k.reused)
        .count();
    let reused = plan
        .files
        .iter()
        .flat_map(|f| &f.keys)
        .filter(|k| k.reused)
        .count();
    let mut skipped: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for f in &plan.files {
        for s in &f.skipped {
            skipped
                .entry(&s.reason)
                .or_default()
                .push(format!("{}:{}  {:?}", f.path, s.line, s.text));
        }
    }
    let errors: Vec<_> = plan
        .files
        .iter()
        .filter_map(|f| Some((&f.path, f.error.as_ref()?)))
        .collect();
    anstream::println!(
        "{BOLD}{changed} files{BOLD:#} to modify, {BOLD}{}{BOLD:#} strings converted ({new} new keys, {reused} reused){}",
        plan.converted(),
        if plan.usage.cost_usd > 0.0 {
            format!(", key naming cost ${:.4}", plan.usage.cost_usd)
        } else {
            String::new()
        }
    );
    if !plan.files.is_empty() && !skipped.is_empty() {
        let n: usize = skipped.values().map(Vec::len).sum();
        anstream::println!("{YELLOW}{n} strings left as is:{YELLOW:#}");
        for (reason, items) in &skipped {
            anstream::println!("  {BOLD}{reason}{BOLD:#} ({})", items.len());
            for i in items.iter().take(5) {
                anstream::println!("    {DIM}{i}{DIM:#}");
            }
            if items.len() > 5 {
                anstream::println!("    {DIM}… {} more{DIM:#}", items.len() - 5);
            }
        }
    }
    for (path, e) in errors {
        anstream::println!("{RED}✗{RED:#} {path}: {e}");
    }
}
