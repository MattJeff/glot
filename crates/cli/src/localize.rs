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
    /// Directory holding glot's previous output for this repository (the open pull request's
    /// branch): its keys and still-valid translations are reused, so an unchanged string costs
    /// nothing and produces no diff; keys it added that the code no longer uses are dropped.
    #[arg(long, value_name = "DIR")]
    pub carry_from: Option<std::path::PathBuf>,
}

#[derive(clap::Args)]
pub struct AgentPromptArgs {
    /// Who reads it: `agent` (Claude Code, Codex, Cursor…) or an app builder (`lovable`, `bolt`, `v0`).
    #[arg(long = "for", default_value = "agent", value_parser = ["agent", "lovable", "bolt", "v0"])]
    pub target: String,
}

/// Extra variable for the steps (`GLOT_CATALOGS_FROM_GLOT`, see `run`).
static CHILD_ENV: std::sync::OnceLock<(&str, &str)> = std::sync::OnceLock::new();

/// One step = this same binary with other arguments, output streamed as is.
fn step(label: &str, args: &[String]) -> Result<bool> {
    anstream::eprintln!(
        "\n{BOLD}▸ {label}{BOLD:#} {DIM}glot {}{DIM:#}",
        args.join(" ")
    );
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.args(args);
    if let Some((k, v)) = CHILD_ENV.get() {
        cmd.env(k, v);
    }
    let status = cmd
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

    // A glot.toml written by the developer (not by this run) decides the scope.
    let scope = configured_scope(&root);
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

    // Incremental runs: start from the previous output (ignored when unreadable: a full run then).
    let carried: Vec<String> = match (&a.carry_from, a.dry_run) {
        (Some(prev), false) => {
            // No catalog on the base yet: the ones carried are glot's own output, not an i18n
            // runtime the app already had (`glot i18nize` reads this).
            if glot_llm::engine::Workspace::open(&root)
                .is_ok_and(|ws| ws.source().entries.is_empty())
            {
                let _ = CHILD_ENV.set(("GLOT_CATALOGS_FROM_GLOT", "1"));
            }
            match glot_llm::engine::Workspace::open(&root).and_then(|mut ws| ws.carry_from(prev)) {
                Ok(keys) => keys,
                Err(e) => {
                    anstream::eprintln!(
                        "{YELLOW}previous output ignored{YELLOW:#} {DIM}({e}){DIM:#}"
                    );
                    vec![]
                }
            }
        }
        _ => vec![],
    };

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
        if let Some(dir) = &scope {
            anstream::eprintln!(
                "{DIM}scope: {dir} (the catalogs of glot.toml live there and it sets no [extract] include){DIM:#}"
            );
            args.push(dir.clone());
        }
        // ponytail: unsupported framework / nothing to rewrite is not fatal, catalogs may already be complete.
        if !step("i18nize", &args)? {
            anstream::eprintln!(
                "{YELLOW}i18nize skipped{YELLOW:#} {DIM}(see above; continuing with the catalogs as they are){DIM:#}"
            );
        }
    }

    if !a.dry_run {
        match tidy(&root, &carried) {
            Ok((dropped, renamed, pruned)) if dropped + renamed + pruned > 0 => {
                anstream::eprintln!(
                    "{DIM}catalogs tidied: {renamed} renamed keys kept their translations, {} deleted strings removed{DIM:#}",
                    dropped + pruned
                )
            }
            Ok(_) => {}
            Err(e) => {
                anstream::eprintln!("{YELLOW}catalogs not tidied{YELLOW:#} {DIM}({e}){DIM:#}")
            }
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

/// The code a hand-written glot.toml is about, when it does not say (`[extract]` absent): the
/// feature folder holding its catalogs. `files = ["src/app/visa-api/landing/messages/{locale}.json"]`
/// => `src/app/visa-api/landing`. Without it, the default include (`src/**`) would move every
/// string of the repository into the landing page's catalogs. `None`: the whole default scope.
fn configured_scope(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("glot.toml")).ok()?;
    if text
        .lines()
        .any(|l| l.trim_start().starts_with("[extract]"))
    {
        return None;
    }
    let c = glot_core::config::Config::load(root).ok()?;
    let dirs: std::collections::BTreeSet<String> = c
        .files
        .iter()
        .map(|f| {
            let dir = &f[..f.find(['{', '*']).unwrap_or(f.len())];
            let dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
            // `…/messages`, `…/locales`, `…/i18n`: the feature is the folder above.
            match dir.rsplit_once('/') {
                Some((up, leaf))
                    if [
                        "messages",
                        "locales",
                        "locale",
                        "i18n",
                        "lang",
                        "translations",
                    ]
                    .contains(&leaf) =>
                {
                    up.to_string()
                }
                _ => dir.to_string(),
            }
        })
        .collect();
    if dirs.len() != 1 {
        return None;
    }
    let dir = dirs.into_iter().next()?;
    // Only a nested feature folder narrows anything (`src`, `app`, the root: same as the default).
    (dir.matches('/').count() >= 2 && root.join(&dir).is_dir() && !dir.contains(".."))
        .then_some(dir)
}

/// Before translating: keys the previous run added that the code no longer uses are dropped,
/// renamed keys keep their translations, target entries of deleted source keys are removed.
/// Returns (dropped, renamed, pruned).
fn tidy(root: &Path, carried: &[String]) -> Result<(usize, usize, usize)> {
    let mut ws = glot_llm::engine::Workspace::open(root)?;
    crate::forced_llm(&mut ws.config.llm);
    // Placeholder values of the offline mock (`⟦fr⟧ Text`) left by an earlier run: with a real
    // provider they are translated again instead of shipped.
    if ws.config.llm.provider != "mock" {
        let purged = ws.drop_mock_values()?;
        if purged > 0 {
            anstream::eprintln!(
                "{YELLOW}{purged} placeholder translations (`⟦xx⟧ …`) from the mock provider will be translated again{YELLOW:#}"
            );
        }
    }
    let mut dropped = 0;
    if !carried.is_empty() {
        let report = ws.extract()?;
        // Nothing seen in the code at all: the extractor does not know this stack, drop nothing.
        if !report.usages.is_empty() {
            let unused = glot_extract::unused_keys(&report, ws.source());
            let gone: Vec<String> = carried
                .iter()
                .filter(|k| unused.contains(k))
                .cloned()
                .collect();
            dropped = gone.len();
            if !gone.is_empty() {
                ws.remove_keys(&gone)?;
            }
        }
    }
    let (renamed, pruned) = ws.reconcile()?;
    // Last check of the rewrite: a key called in the code must exist in the source catalog.
    let ws = glot_llm::engine::Workspace::open(root)?;
    let report = ws.extract()?;
    let missing = glot_extract::missing_keys(&report, ws.source());
    // Most usages "missing": the catalog is keyed another way (source text as key), not a bug.
    if !missing.is_empty() && missing.len() * 2 < report.usages.len() {
        anstream::println!(
            "warning: {} key{} used in the code {} missing from the source catalog: {}",
            missing.len(),
            if missing.len() == 1 { "" } else { "s" },
            if missing.len() == 1 { "is" } else { "are" },
            missing
                .iter()
                .take(8)
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok((dropped, renamed, pruned))
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
            |c| {
                if c.hook.is_empty() {
                    format!("`import {{ t }}` from `{}`", c.module)
                } else {
                    format!("`const {{ t }} = {}()` from `{}`", c.hook, c.module)
                }
            },
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

    #[test]
    fn hand_written_config_scopes_the_rewrite() {
        let dir = std::env::temp_dir().join(format!("glot-scope-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/app/visa-api/landing/messages")).unwrap();
        let toml = "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"src/app/visa-api/landing/messages/{locale}.json\"]\n";
        std::fs::write(dir.join("glot.toml"), toml).unwrap();
        assert_eq!(
            super::configured_scope(&dir).as_deref(),
            Some("src/app/visa-api/landing")
        );
        // An explicit [extract] include is the developer's choice.
        std::fs::write(
            dir.join("glot.toml"),
            format!("{toml}[extract]\ninclude = [\"src/**/*.tsx\"]\n"),
        )
        .unwrap();
        assert_eq!(super::configured_scope(&dir), None);
        // Root-level catalogs: nothing to narrow.
        std::fs::write(
            dir.join("glot.toml"),
            toml.replace("src/app/visa-api/landing/messages/", "messages/"),
        )
        .unwrap();
        assert_eq!(super::configured_scope(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
