//! `glot localize`: init (if needed) → i18nize → translate → check → status, in one idempotent step.
//! `glot agent-prompt`: a ready-to-paste i18n prompt for any coding agent / app builder.
use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::style::*;

#[derive(clap::Args)]
pub struct LocalizeArgs {
    /// Target locales for a new project (comma separated), passed to `glot init`.
    #[arg(long, value_delimiter = ',')]
    pub targets: Vec<String>,
    /// Show what each step would do (i18nize diff, translation plan and cost); write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(short, long)]
    pub yes: bool,
    /// LLM provider for this run (`mock`: offline, free, no API key).
    #[arg(long, value_parser = ["mock", "anthropic", "openai", "openai-compatible"])]
    pub provider: Option<String>,
    /// Skip the code rewrite (strings already in catalogs).
    #[arg(long)]
    pub no_i18nize: bool,
    /// Print the agent prompt for this project and exit (like i18next-cli `--print-agent-prompt`).
    #[arg(long)]
    pub print_agent_prompt: bool,
}

#[derive(clap::Args)]
pub struct AgentPromptArgs {
    /// Who reads it: `agent` (Claude Code, Codex, Cursor…) or an app builder (`lovable`, `bolt`, `v0`).
    #[arg(long = "for", default_value = "agent", value_parser = ["agent", "lovable", "bolt", "v0"])]
    pub target: String,
}

/// One step = this same binary with other arguments, output streamed as is.
fn step(label: &str, args: &[String]) -> Result<bool> {
    anstream::eprintln!(
        "\n{BOLD}▸ {label}{BOLD:#} {DIM}glot {}{DIM:#}",
        args.join(" ")
    );
    let status = Command::new(std::env::current_exe()?)
        .args(args)
        .status()
        .with_context(|| format!("running glot {label}"))?;
    Ok(status.success())
}

pub fn run(a: LocalizeArgs) -> Result<ExitCode> {
    let root = std::env::current_dir()?;
    if a.print_agent_prompt {
        print!("{}", agent_prompt(&root, "agent"));
        return Ok(ExitCode::SUCCESS);
    }
    let s = |x: &str| x.to_string();
    let mut flags: Vec<String> = vec![];
    if a.yes {
        flags.push(s("--yes"));
    }
    let provider: Vec<String> = a
        .provider
        .iter()
        .flat_map(|p| [s("--provider"), p.clone()])
        .collect();

    if !root.join("glot.toml").exists() {
        let mut args = vec![s("init"), s("--yes")];
        if !a.targets.is_empty() {
            args.extend([s("--targets"), a.targets.join(",")]);
        }
        args.extend(provider.iter().cloned());
        if a.dry_run {
            anstream::println!(
                "{YELLOW}no glot.toml{YELLOW:#}: would run {BOLD}glot {}{BOLD:#}, then i18nize → translate → check. Run without --dry-run to start.",
                args.join(" ")
            );
            return Ok(ExitCode::SUCCESS);
        }
        if !step("init", &args)? {
            return Ok(ExitCode::from(2));
        }
    }

    let dry: Vec<String> = if a.dry_run {
        vec![s("--dry-run")]
    } else {
        vec![]
    };
    if !a.no_i18nize {
        let mut args = vec![s("i18nize")];
        args.extend(dry.iter().chain(&flags).cloned());
        // ponytail: unsupported framework / nothing to rewrite is not fatal, catalogs may already be complete.
        if !step("i18nize", &args)? {
            anstream::eprintln!(
                "{YELLOW}i18nize skipped{YELLOW:#} {DIM}(see above; continuing with the catalogs as they are){DIM:#}"
            );
        }
    }

    let mut args = vec![s("translate")];
    args.extend(dry.iter().chain(&flags).chain(&provider).cloned());
    if !step("translate", &args)? {
        return Ok(ExitCode::from(2));
    }
    if a.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    let ok = step("check", &[s("check")])?;
    step("status", &[s("status")])?;
    Ok(if ok {
        anstream::println!(
            "\n{GREEN}✓ localized{GREEN:#} {DIM}rerun `glot localize --yes` after any UI change{DIM:#}"
        );
        ExitCode::SUCCESS
    } else {
        anstream::println!(
            "\n{RED}✗ glot check failed{RED:#}: fix with `glot set <locale> <key> \"<value>\"` or `glot translate --key <key> --force`"
        );
        ExitCode::FAILURE
    })
}

pub fn agent_prompt_cmd(a: AgentPromptArgs) -> Result<ExitCode> {
    print!("{}", agent_prompt(&std::env::current_dir()?, &a.target));
    Ok(ExitCode::SUCCESS)
}

/// Stack-specific i18n rules + the glot commands, from glot.toml when present, else detection.
pub fn agent_prompt(root: &Path, target: &str) -> String {
    let d = crate::init::detect(root);
    let (source, targets, files) = match glot_core::config::Config::load(root) {
        Ok(c) => (c.source_locale, c.target_locales.join(","), c.files),
        Err(_) => ("en".into(), "<locales>".into(), d.files.clone()),
    };
    let listed = targets.replace(',', ", ");
    let catalog = files
        .first()
        .map(|f| f.replace("{locale}", &source))
        .unwrap_or_default();
    let usage = match d.stack {
        "next-intl" => format!(
            "This is a Next.js app using next-intl. Use `const t = useTranslations(\"<section>\")` in client components and `const t = await getTranslations(\"<section>\")` in server components. Catalog: {catalog}. Plurals: ICU, e.g. \"{{count, plural, one {{# invoice}} other {{# invoices}}}}\" with t(\"invoices.count\", {{ count }})."
        ),
        "i18next" | "react-i18next" => format!(
            "This app uses react-i18next. Use `const {{ t }} = useTranslation()` and `<Trans>` for markup. Catalog: {catalog}. Placeholders are {{{{name}}}}; plurals use _one/_other key suffixes with t(\"key\", {{ count }})."
        ),
        "react-intl" => format!(
            "This app uses react-intl (FormatJS). Use `useIntl().formatMessage({{ id }})` or `<FormattedMessage id=… />`. Catalog: {catalog}. ICU placeholders and plurals."
        ),
        "vue-i18n" => format!(
            "This app uses vue-i18n. Use `$t(\"key\")` / `useI18n().t`. Catalog: {catalog}."
        ),
        "lingui" => {
            format!("This app uses Lingui. Use the `t` macro / `<Trans>`. Catalog: {catalog}.")
        }
        _ => format!(
            "No i18n library yet. Next.js App Router: install next-intl (catalogs messages/<locale>.json). Vite/React: react-i18next + i18next, initialized in src/i18n.ts imported first by the entry file (catalogs src/locales/<locale>.json). Current catalog pattern: {}.",
            files.join(", ")
        ),
    };
    let run = if target == "agent" {
        format!(
            "When asked to translate the app or add a language, run `npx -y glot-i18n localize --yes --targets {targets}` (init → i18nize → translate → check; idempotent). It needs ANTHROPIC_API_KEY or OPENAI_API_KEY. `glot check` must exit 0 before you finish; commit glot.lock with the catalogs."
        )
    } else {
        // App builders (Lovable, Bolt, v0) usually cannot run a CLI: keep the code translation-ready.
        format!(
            "Translation itself happens outside this builder: the repository runs `npx -y glot-i18n localize --yes --targets {targets}` (or its GitHub workflow). If you can run commands, run it; otherwise just keep every string in the {source} catalog."
        )
    };
    format!(
        "Internationalization rules (this app is translated with glot, https://github.com/MattJeff/glot):\n\
         - {usage}\n\
         - Never hardcode user-facing text: headings, buttons, labels, placeholders, aria-labels, titles, toasts, error and empty states all go through the i18n library.\n\
         - Add new strings to the `{source}` catalog only, with nested keys named by screen and role (e.g. \"billing.plan_card.upgrade_button\"). Never create or edit other locales' files ({listed}); glot generates them.\n\
         - Placeholders and ICU plurals, never string concatenation. Format dates, numbers and currencies with Intl using the active locale.\n\
         - Leave room for text 30-40% longer than {source}: no fixed widths on buttons, tabs or badges.\n\
         - Brand and product names stay untranslated.\n\
         - {run}\n"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_adapts_to_stack() {
        let dir = std::env::temp_dir().join(format!("glot-agent-prompt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dependencies":{"next-intl":"4"}}"#,
        )
        .unwrap();
        let p = super::agent_prompt(&dir, "agent");
        assert!(p.contains("useTranslations") && p.contains("glot-i18n localize"));
        std::fs::write(
            dir.join("package.json"),
            r#"{"dependencies":{"react-i18next":"15"}}"#,
        )
        .unwrap();
        let p = super::agent_prompt(&dir, "lovable");
        assert!(p.contains("useTranslation()") && p.contains("outside this builder"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
