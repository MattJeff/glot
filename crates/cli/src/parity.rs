//! `glot parity`: can a user finish the same tasks in every locale? Drives `packages/agent-qa`
//! (Playwright + a Claude vision agent, or a scripted mock) and prints its report.
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::style::*;

#[derive(clap::Args)]
pub struct ParityArgs {
    /// Task file: [{id, goal, start, success: {url_matches|text_visible_key|selector}, script?}].
    #[arg(long, default_value = "glot.tasks.yaml")]
    pub tasks: PathBuf,
    /// Locales to test (comma separated). Default: source + all targets.
    #[arg(long, value_delimiter = ',')]
    pub locales: Vec<String>,
    #[arg(long, default_value = "http://localhost:3000")]
    pub base_url: String,
    /// URL layout; the source locale gets no prefix. E.g. "{route}?lang={locale}".
    #[arg(long, default_value = "/{locale}{route}")]
    pub url_pattern: String,
    /// Cookies (comma separated) set to the locale before each run, so in-app links keep the language.
    /// Default: NEXT_LOCALE,glot-locale (what `glot i18nize` generates); "" sets none.
    #[arg(long)]
    pub locale_cookie: Option<String>,
    /// anthropic (Claude vision agent) | mock (follows each task's `script`, no tokens).
    /// Default: anthropic if glot.toml uses it, else mock.
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub model: String,
    #[arg(long, default_value_t = 15)]
    pub max_steps: u32,
    #[arg(long, default_value = "1280x800")]
    pub viewport: String,
    #[arg(long, default_value = ".glot/parity")]
    pub out: PathBuf,
    /// Exit 1 if a locale's parity (success rate vs source, %) is below this.
    #[arg(long, default_value_t = 0)]
    pub min_parity: u32,
}

pub fn run(a: ParityArgs) -> Result<ExitCode> {
    let ws = crate::open()?;
    let cfg = &ws.config;
    let locales = if a.locales.is_empty() {
        std::iter::once(&cfg.source_locale)
            .chain(&cfg.target_locales)
            .cloned()
            .collect()
    } else {
        a.locales.clone()
    };
    let provider = a.provider.clone().unwrap_or_else(|| {
        if cfg.llm.provider == "anthropic" {
            "anthropic"
        } else {
            "mock"
        }
        .into()
    });
    if provider != "anthropic" && provider != "mock" {
        bail!(
            "--provider must be anthropic or mock (the parity agent drives Claude's vision + tool use)"
        );
    }
    let mut c = crate::fit::node_tool("agent-qa");
    c.current_dir(&ws.root)
        .args(["--locales", &locales.join(",")])
        .args(["--source-locale", &cfg.source_locale])
        .args(["--base-url", &a.base_url])
        .args(["--url-pattern", &a.url_pattern])
        .args(["--catalogs", &cfg.files.join(",")])
        .args(["--provider", &provider])
        .args(["--model", &a.model])
        .args(["--max-steps", &a.max_steps.to_string()])
        .args(["--viewport", &a.viewport])
        .args(["--min-parity", &a.min_parity.to_string()])
        .arg("--tasks")
        .arg(&a.tasks)
        .arg("--out")
        .arg(&a.out);
    if let Some(k) = &a.locale_cookie {
        c.args(["--locale-cookie", k]);
    }
    // Same key as `glot translate`: honour a custom api_key_env.
    if provider == "anthropic"
        && let Some(env) = cfg
            .llm
            .api_key_env
            .as_deref()
            .filter(|e| *e != "ANTHROPIC_API_KEY")
        && let Ok(key) = std::env::var(env)
    {
        c.env("ANTHROPIC_API_KEY", key);
    }
    anstream::eprintln!(
        "{DIM}parity: {} × {} ({provider})…{DIM:#}",
        a.tasks.display(),
        locales.join(",")
    );
    let status = c
        .status()
        .with_context(|| crate::fit::tool_missing("agent-qa"))?;
    // 0: parity ok, 1: below --min-parity, anything else: it could not run (no fresh report).
    let code = match status.code() {
        Some(c @ (0 | 1)) => c as u8,
        _ => bail!("agent-qa failed"),
    };
    let md = ws.root.join(&a.out).join("report.md");
    println!(
        "{}",
        std::fs::read_to_string(&md).context("agent-qa wrote no report.md")?
    );
    Ok(ExitCode::from(code))
}
