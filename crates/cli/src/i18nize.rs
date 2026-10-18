//! `glot i18nize`: turn hardcoded UI copy into catalog keys and rewrite the code.
use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use std::collections::BTreeMap;

use glot_llm::i18nize::{self, Lib, Plan};

use crate::setup::{
    self, Routing,
    refactor::{self, Source},
};
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
    /// Next.js locale URLs: `cookie` (`/fr/...` selects and remembers the locale, no `app/[locale]` move)
    /// or `prefix` (non-default locales always under `/fr/...`).
    #[arg(long, value_enum, default_value_t = Routing::Cookie)]
    pub routing: Routing,
    /// Also generate a ready-to-use `<LocaleSwitcher />` component.
    #[arg(long)]
    pub switcher: bool,
    /// Do not run the package manager; print the install command instead.
    #[arg(long)]
    pub no_install: bool,
    /// Only rewrite the code: do not wire the i18n library (provider, config, proxy).
    #[arg(long)]
    pub no_setup: bool,
}

pub async fn run(args: I18nizeArgs) -> Result<()> {
    let mut ws = crate::open()?;
    let project = setup::detect(&ws.root);
    let lib = match args.lib {
        LibArg::NextIntl => Lib::NextIntl,
        LibArg::ReactI18next => Lib::ReactI18next,
        LibArg::Auto => match project.as_ref().map(|p| p.framework.lib()) {
            Some(l) => l,
            None => match i18nize::detect_lib(&ws.root) {
                Some(l) => l,
                None => bail!(
                    "cannot detect the framework from package.json; pass --lib next-intl|react-i18next"
                ),
            },
        },
    };
    if let Some(p) = &project {
        anstream::eprintln!(
            "{DIM}{} detected, target library: {}{DIM:#}",
            p.framework.label(),
            if lib == Lib::NextIntl {
                "next-intl"
            } else {
                "react-i18next"
            }
        );
    }

    // Project-wide passes: text props of local components, module configs -> hooks.
    let all: Vec<Source> = glot_extract::i18nize::source_files(&ws.root, &ws.config.extract)?
        .into_iter()
        .filter_map(|path| {
            let src = std::fs::read_to_string(ws.root.join(&path)).ok()?;
            Some(Source { path, src })
        })
        .collect();
    glot_extract::i18nize::set_text_props(refactor::text_props(&all));
    let cfg = refactor::config_hooks(&all);
    for w in &cfg.warnings {
        anstream::eprintln!("{YELLOW}warning:{YELLOW:#} {w}");
    }
    if !cfg.files.is_empty() {
        anstream::println!(
            "{BOLD}{}{BOLD:#} module-level configs become hooks so their copy can be translated:",
            cfg.converted.len()
        );
        for c in &cfg.converted {
            anstream::println!("  {GREEN}~{GREEN:#} {c}");
        }
        if args.dry_run {
            for (path, old, new) in &cfg.files {
                print_diff(&glot_extract::i18nize::unified_diff(path, old, new));
            }
            anstream::println!(
                "{DIM}(the i18nize plan below is computed before this refactor){DIM:#}\n"
            );
        } else {
            if !args.yes && !crate::confirm("Apply this refactor?")? {
                anstream::println!("Aborted, nothing written.");
                return Ok(());
            }
            for (path, _, new) in &cfg.files {
                std::fs::write(ws.root.join(path), new)?;
            }
        }
    }

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

    let setup_on = !args.no_setup && project.as_ref().is_some_and(|p| p.framework.lib() == lib);
    let opts = |ws: &glot_llm::engine::Workspace| setup::Opts {
        routing: args.routing,
        switcher: args.switcher,
        locales: std::iter::once(ws.config.source_locale.clone())
            .chain(ws.config.target_locales.iter().cloned())
            .collect(),
        catalog: ws.config.files.first().cloned().unwrap_or_default(),
    };
    if args.dry_run {
        if setup_on && let Some(p) = &project {
            let sp = setup::plan(&ws.root, p, &opts(&ws));
            print_setup(&sp, true);
        }
        return Ok(());
    }
    if plan.converted() > 0 {
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
    }
    if setup_on && let Some(p) = &project {
        // After the rewrite: the layout may have just been converted, and Astro islands need it.
        let sp = setup::plan(&ws.root, p, &opts(&ws));
        setup::write(&ws.root, &sp)?;
        for (k, v, note) in &sp.keys {
            if !ws.source().entries.contains_key(k) {
                ws.add_source(k, v, Some(note))?;
            }
        }
        print_setup(&sp, false);
        match sp.install_cmd() {
            Some(cmd) if args.no_install => {
                anstream::println!("{YELLOW}install:{YELLOW:#} run {BOLD}{cmd}{BOLD:#}")
            }
            Some(cmd) => {
                anstream::eprintln!("{DIM}$ {cmd}{DIM:#}");
                setup::install(&ws.root, &sp)?;
            }
            None => {}
        }
    } else if !args.no_setup {
        anstream::println!(
            "{YELLOW}note:{YELLOW:#} framework not detected: wire {} by hand (provider + config).",
            if lib == Lib::NextIntl {
                "next-intl"
            } else {
                "react-i18next"
            }
        );
    }
    anstream::println!(
        "{GREEN}✓ done.{GREEN:#} Next: {BOLD}{} translate{BOLD:#} to fill {}",
        crate::init::cli_name(),
        ws.config.target_locales.join(", ")
    );
    Ok(())
}

pub(crate) fn print_setup(sp: &setup::SetupPlan, dry_run: bool) {
    if sp.changes.is_empty() && sp.packages.is_empty() && sp.notes.is_empty() && sp.moves.is_empty()
    {
        anstream::println!("{DIM}i18n runtime already wired.{DIM:#}");
        return;
    }
    anstream::println!("{BOLD}i18n runtime setup{BOLD:#}");
    for (from, to) in &sp.moves {
        anstream::println!("  {GREEN}moved  {GREEN:#} {from} -> {to}");
    }
    for c in &sp.changes {
        if dry_run {
            print_diff(&glot_extract::i18nize::unified_diff(
                &c.path,
                c.old.as_deref().unwrap_or(""),
                &c.new,
            ));
        } else {
            let what = if c.old.is_some() {
                "edited "
            } else {
                "created"
            };
            anstream::println!("  {GREEN}{what}{GREEN:#} {}", c.path);
        }
    }
    if dry_run && let Some(cmd) = sp.install_cmd() {
        anstream::println!("  {YELLOW}would run{YELLOW:#} {cmd}");
    }
    for n in &sp.notes {
        anstream::println!("  {YELLOW}note:{YELLOW:#} {n}");
    }
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
