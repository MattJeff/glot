//! `glot review` (back-translation, semantic drift) and `glot glossary mine`.
use anyhow::Result;
use std::process::ExitCode;

use glot_llm::review::{self, Fidelity};
use glot_llm::{mine, translate::Usage};

use crate::style::*;
use crate::{confirm, open, print_json};

#[derive(clap::Args)]
pub struct ReviewArgs {
    /// Locales to review (comma separated). Default: all targets.
    #[arg(long, value_delimiter = ',')]
    locale: Vec<String>,
    /// Only these keys (comma separated or repeated).
    #[arg(long, value_delimiter = ',')]
    keys: Vec<String>,
    #[arg(long)]
    json: bool,
    /// Ignore the cache (.glot/review.json) and ask the LLM again.
    #[arg(long)]
    refresh: bool,
    /// Write the suggested translation for every `different` / `nuance` row (validated).
    #[arg(long)]
    fix: bool,
    /// Stale strings: keep (source change is cosmetic) or retranslate, per key.
    #[arg(long)]
    stale: bool,
    /// Apply without asking.
    #[arg(short, long)]
    yes: bool,
}

#[derive(clap::Args)]
pub struct MineArgs {
    /// Write the terms to glossary.toml and retranslate the inconsistent keys.
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    json: bool,
    #[arg(short, long)]
    yes: bool,
}

fn cost(u: &Usage) {
    anstream::eprintln!(
        "{DIM}LLM cost ${:.4} ({} in / {} out tokens){DIM:#}",
        u.cost_usd,
        u.input_tokens,
        u.output_tokens
    );
}

fn verdict(v: Fidelity) -> String {
    match v {
        Fidelity::Same => format!("{GREEN}same{GREEN:#}"),
        Fidelity::Nuance => format!("{YELLOW}nuance{YELLOW:#}"),
        Fidelity::Different => format!("{RED}DIFFERENT{RED:#}"),
    }
}

pub async fn run(a: ReviewArgs) -> Result<ExitCode> {
    if a.stale {
        return stale(a.json, a.yes).await;
    }
    let mut ws = open()?;
    let locales = (!a.locale.is_empty()).then_some(a.locale.as_slice());
    let keys = (!a.keys.is_empty()).then_some(a.keys.as_slice());
    let (rows, usage) = review::review_workspace(&ws, locales, keys, a.refresh).await?;
    if a.json {
        print_json(&rows)?;
    } else {
        for r in &rows {
            anstream::println!(
                "{BOLD}{}{BOLD:#} [{}] {}",
                r.key,
                r.locale,
                verdict(r.verdict)
            );
            anstream::println!("  source  {:?}", r.source);
            anstream::println!("  target  {:?}", r.target);
            anstream::println!("  back    {:?}", r.back_translation);
            anstream::println!("  {DIM}{}{DIM:#}", r.explanation);
            if let Some(s) = &r.suggestion {
                anstream::println!("  {YELLOW}suggest{YELLOW:#} {s:?}");
            }
        }
        let bad = rows
            .iter()
            .filter(|r| r.verdict == Fidelity::Different)
            .count();
        let nuance = rows
            .iter()
            .filter(|r| r.verdict == Fidelity::Nuance)
            .count();
        anstream::println!(
            "{} reviewed: {RED}{bad} different{RED:#}, {YELLOW}{nuance} nuance{YELLOW:#}",
            rows.len()
        );
    }
    cost(&usage);
    if a.fix {
        let (ok, rejected) = review::apply_suggestions(&mut ws, &rows)?;
        anstream::println!("{GREEN}fixed{GREEN:#} {ok}");
        for (k, l, why) in &rejected {
            anstream::println!("  {RED}✗{RED:#} {l} {k}  {DIM}{why}{DIM:#}");
        }
    }
    let bad = rows.iter().any(|r| r.verdict == Fidelity::Different);
    Ok(if bad && !a.fix {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

async fn stale(json: bool, yes: bool) -> Result<ExitCode> {
    let mut ws = open()?;
    let (drifts, usage) = review::stale(&ws).await?;
    if json {
        print_json(&drifts)?;
    } else {
        for d in &drifts {
            let action = if d.keep {
                format!("{GREEN}keep{GREEN:#}")
            } else {
                format!("{YELLOW}retranslate{YELLOW:#}")
            };
            anstream::println!(
                "{BOLD}{}{BOLD:#} [{}] {action} ({})",
                d.key,
                d.locale,
                verdict(d.verdict)
            );
            anstream::println!(
                "  new source {:?}\n  target     {:?}\n  back       {:?}",
                d.source,
                d.target,
                d.back_translation
            );
            anstream::println!("  {DIM}{}{DIM:#}", d.explanation);
        }
        let keep = drifts.iter().filter(|d| d.keep).count();
        anstream::println!(
            "{} stale: {keep} keep, {} retranslate",
            drifts.len(),
            drifts.len() - keep
        );
    }
    cost(&usage);
    if drifts.is_empty()
        || !(yes || confirm("Apply (keep = refresh glot.lock, else retranslate)?")?)
    {
        return Ok(ExitCode::SUCCESS);
    }
    let u = review::apply_drift(&mut ws, &drifts).await?;
    anstream::println!("{GREEN}applied{GREEN:#}");
    cost(&u);
    Ok(ExitCode::SUCCESS)
}

pub async fn mine(a: MineArgs) -> Result<()> {
    let mut ws = open()?;
    let (mined, usage) = mine::mine(&ws).await?;
    if a.json {
        print_json(&mined)?;
    } else {
        for m in &mined {
            let tr = if m.term.do_not_translate {
                format!("{YELLOW}do-not-translate{YELLOW:#}")
            } else {
                m.term
                    .translations
                    .iter()
                    .map(|(l, v)| format!("{l}={v}"))
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            anstream::println!(
                "{BOLD}{}{BOLD:#} {DIM}×{}{DIM:#}  {tr}",
                m.term.term,
                m.occurrences
            );
            for (l, ks) in &m.inconsistent {
                anstream::println!("  {RED}inconsistent{RED:#} {l}: {}", ks.join(", "));
            }
        }
        if mined.is_empty() {
            anstream::println!("{GREEN}no new terms{GREEN:#}");
        }
    }
    cost(&usage);
    if a.apply
        && !mined.is_empty()
        && (a.yes || confirm("Write glossary.toml and retranslate inconsistent keys?")?)
    {
        let u = mine::apply(&mut ws, &mined).await?;
        anstream::println!("{GREEN}glossary updated{GREEN:#}, inconsistent keys retranslated");
        cost(&u);
    }
    Ok(())
}
