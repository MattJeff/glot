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
    /// Do not run the package manager (passed to `glot i18nize`); the install command is printed instead.
    #[arg(long)]
    pub no_install: bool,
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
        if a.no_install {
            args.push(s("--no-install"));
        }
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
/// Also the body of the block `glot init` writes to CLAUDE.md / AGENTS.md / skills / Cursor rules.
pub fn agent_prompt(root: &Path, target: &str) -> String {
    let d = crate::init::detect(root);
    let (source, targets, files) = match glot_core::config::Config::load(root) {
        Ok(c) => (c.source_locale, c.target_locales.join(","), c.files),
        Err(_) => ("en".into(), "<locales>".into(), d.files.clone()),
    };
    let listed = targets.replace(',', ", ");
    let cats = files
        .iter()
        .map(|f| format!("`{f}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let source_file = files
        .first()
        .map(|f| f.replace("{locale}", &source))
        .unwrap_or_default();
    let agent = target == "agent";
    let copy = "Every user-facing string (headings, buttons, labels, placeholders, aria-labels, titles, toasts, errors, empty states, emails)";
    let keys = "Keys are nested and named `<screen>.<component>.<role>`, lowercase snake_case: `billing.plan_card.upgrade_button`, `auth.login_form.email_label`. Reuse `common.*` for generic actions (`common.cancel`, `common.save`).";
    let localize = format!(
        "`npx -y glot-i18n localize --yes --targets {targets}` (add the new locale to the list)"
    );
    let own_lib = !["i18next", "next-intl", "react-intl", "lingui", "vue-i18n"].contains(&d.stack)
        && !d.stack.contains("no i18n library");
    let custom = own_lib.then(|| crate::init::custom_runtime(root)).flatten();
    let own = custom.is_some() || d.stack.starts_with("custom") || d.stack == "TS/JS dictionaries";
    let mut rules: Vec<String> = vec![];
    let (plural, switcher) = if own {
        // A hand-rolled runtime: keep it, never a second one (i18next / next-intl).
        let hook = custom.as_ref().map_or_else(
            || "the app's own `t()`".to_string(),
            |c| format!("`const {{ t }} = {}()` from `{}`", c.hook, c.module),
        );
        let style = std::fs::read_to_string(root.join(&source_file))
            .ok()
            .and_then(|t| {
                glot_core::formats::parse(
                    glot_core::catalog::Format::Js,
                    Path::new(&source_file),
                    &t,
                    &source,
                )
                .ok()
            })
            .and_then(|p| p.entries.keys().next().cloned())
            .map_or_else(
                || "the existing key style".to_string(),
                |k| format!("the existing key style (`{k}`)"),
            );
        rules.push(format!(
            "This app has its own i18n runtime: {hook}, dictionary {cats}. Keep using it: do NOT install i18next or next-intl, do NOT create a second dictionary. {copy} goes through `t('<key>')`. Add keys to the `{source}` object only, in {style}; glot fills the other locales in place."
        ));
        rules.push("Replace any `language === 'xx' ? … : …` you touch with `t('<key>')`.".into());
        (
            "No placeholders or plurals unless the app's `t()` handles them: one full sentence per key, never built by concatenation. Dates, numbers, currencies: `Intl` with the active locale.".to_string(),
            "Language switcher: the existing context's setter (no new state, no new context).".to_string(),
        )
    } else {
        let (usage, plural, switcher) = match d.stack {
            "next-intl" | "next.js (no i18n library yet)" => (
                format!(
                    "{copy} goes through `const t = useTranslations(\"<screen>\")` (client) or `const t = await getTranslations(\"<screen>\")` (server)"
                ),
                "ICU: `{count, plural, one {# invoice} other {# invoices}}` with `t(\"invoices.count\", { count })`. Placeholders `{name}`.",
                "Language switcher: the `LocaleSwitcher` from `glot i18nize --switcher` (next-intl routing).",
            ),
            "i18next" | "react-i18next" | "react (no i18n library yet)" => (
                format!(
                    "{copy} goes through `const {{ t }} = useTranslation()` (`<Trans>` for markup)"
                ),
                "Placeholders `{{name}}`, plurals with `_one`/`_other` suffixes and `t(\"key\", { count })`.",
                "Language switcher: call `setLocale(locale)` from `src/i18n.ts`.",
            ),
            "react-intl" => (
                format!(
                    "{copy} goes through `useIntl().formatMessage({{ id }})` or `<FormattedMessage id=… />`"
                ),
                "ICU placeholders `{name}` and plurals `{count, plural, one {#} other {#}}`.",
                "",
            ),
            "vue-i18n" | "nuxt-i18n" => (
                format!("{copy} goes through `$t(\"key\")` / `useI18n().t`"),
                "Placeholders `{name}`, plurals `one | many` with `$t(\"key\", count)`.",
                "",
            ),
            "lingui" => (
                format!("{copy} goes through the `t` macro / `<Trans>`"),
                "ICU placeholders and plurals.",
                "",
            ),
            _ => (
                format!("{copy} goes through the i18n library ({})", d.stack),
                "Placeholders and plurals in the catalog syntax.",
                "",
            ),
        };
        let mut usage = usage;
        if d.stack.contains("no i18n library") {
            usage +=
                ". No library is wired yet: run `glot i18nize` first (it installs and wires it)";
        }
        rules.push(format!(
            "{usage}. Never hardcode text in components, never `lang === 'fr' ? … : …`."
        ));
        rules.push(keys.into());
        (
            format!(
                "{plural} Never build sentences by concatenation. Dates, numbers, currencies: `Intl` with the active locale."
            ),
            switcher.to_string(),
        )
    };
    rules.push(if agent {
        format!("Add a string: MCP tool `propose_string` (server `glot`), then `add_string`. Without MCP: `glot add <key> \"<text>\" --note \"<where it shows, max length>\"`. Write only the `{source}` catalog; never edit {listed}: glot translates them.")
    } else {
        format!("Write only the `{source}` catalog ({source_file}); never create or edit {listed}: glot generates them.")
    });
    rules.push(plural);
    rules.push(format!("Leave room for text 30-40% longer than `{source}`: no fixed widths on buttons, tabs, badges. Brand and product names stay untranslated."));
    rules.push(format!(
        "\"Make the app multilingual\" / \"add a language\": do NOT create a LanguageContext, a `translations` object or a new i18n library. {}{}",
        if agent {
            format!("Run {localize}.")
        } else {
            format!("Translation itself happens outside this builder: the repository runs {localize} (or its GitHub workflow). If you can run commands, run it; otherwise just keep every string in the `{source}` catalog.")
        },
        if switcher.is_empty() { String::new() } else { format!(" {switcher}") }
    ));
    rules.push(if agent {
        "Before finishing: `glot check` must exit 0. Commit `glot.lock` with the catalogs.".into()
    } else {
        "The repository's `glot check` must pass.".into()
    });
    format!(
        "This app is localized with glot (https://github.com/MattJeff/glot). Source locale: `{source}`. Targets: {listed}. Catalogs: {cats} (source: `{source_file}`).\n\n{}",
        rules.iter().map(|r| format!("- {r}\n")).collect::<String>()
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
