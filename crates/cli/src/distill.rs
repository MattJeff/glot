//! `glot labs distill`: dataset export, local+RAG translation, eval local vs cloud.
use anyhow::Result;
use std::path::PathBuf;
use std::process::ExitCode;

use glot_llm::distill::{self, Lane};

use crate::style::*;
use crate::{confirm, open, print_json};

#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DistillArgs {
    #[command(subcommand)]
    cmd: Option<Sub>,
    /// Output directory of train.jsonl / eval.jsonl / pairs.json.
    #[arg(long, default_value = distill::DIR)]
    out: PathBuf,
    /// Share of the pairs held out for `distill eval` (percent).
    #[arg(long, default_value_t = 10)]
    eval_pct: u32,
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct Local {
    /// Local model (Ollama tag, LM Studio / llama.cpp model id).
    #[arg(long, default_value = "qwen3:8b")]
    model: String,
    /// `local` (Ollama default URL), `openai-compatible` (LM Studio, llama.cpp server), `mock` (tests).
    #[arg(long, default_value = "local", value_parser = ["local", "openai-compatible", "mock"])]
    provider: String,
    /// Server URL, e.g. http://localhost:1234/v1 (LM Studio), http://localhost:8080/v1 (llama.cpp).
    #[arg(long)]
    base_url: Option<String>,
    /// Approved examples retrieved per string.
    #[arg(short, long, default_value_t = 5)]
    k: usize,
}

#[derive(clap::Subcommand)]
enum Sub {
    /// Translate missing/stale strings with the local model + retrieved examples (validated, retried).
    Translate {
        #[command(flatten)]
        local: Local,
        /// Escalate what the local model fails (validator) to the project's cloud model.
        #[arg(long)]
        escalate: bool,
        #[arg(long, value_delimiter = ',')]
        locale: Vec<String>,
        /// Only keys of this class: label, sentence, icu (see `distill eval`).
        #[arg(long, value_parser = ["label", "sentence", "icu"])]
        class: Option<String>,
    },
    /// Held-out pairs: local vs local+RAG vs the project's cloud model (judge, validator, cost, latency).
    Eval {
        #[command(flatten)]
        local: Local,
        /// Max eval pairs (cost cap).
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long, default_value_t = 10)]
        eval_pct: u32,
        /// Provider of the cloud lane and judge, overriding glot.toml (`mock`: offline).
        #[arg(long)]
        cloud_provider: Option<String>,
        /// Do not write the proposed routing into glot.toml.
        #[arg(long)]
        no_write: bool,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(a: DistillArgs) -> Result<ExitCode> {
    match a.cmd {
        None => export(a.out, a.eval_pct, a.json),
        Some(Sub::Translate {
            local,
            escalate,
            locale,
            class,
        }) => translate(local, escalate, locale, class).await,
        Some(Sub::Eval {
            local,
            limit,
            eval_pct,
            cloud_provider,
            no_write,
            yes,
            json,
        }) => eval(local, limit, eval_pct, cloud_provider, no_write, yes, json).await,
    }
}

fn export(out: PathBuf, eval_pct: u32, json: bool) -> Result<ExitCode> {
    let ws = open()?;
    let ds = distill::collect(&ws, eval_pct);
    let dir = ws.root.join(&out);
    distill::write(&ws, &ds, &dir)?;
    if json {
        print_json(&serde_json::json!({
            "train": ds.train.len(), "eval": ds.eval.len(), "skipped_secret": ds.skipped_secret,
            "skipped_invalid": ds.skipped_invalid, "duplicates": ds.duplicates, "dir": dir,
        }))?;
        return Ok(ExitCode::SUCCESS);
    }
    anstream::println!(
        "{GREEN}wrote{GREEN:#} {} train + {} eval pairs to {}  {DIM}(chat JSONL; skipped {} with secrets, {} invalid, {} duplicates){DIM:#}",
        ds.train.len(),
        ds.eval.len(),
        out.display(),
        ds.skipped_secret,
        ds.skipped_invalid,
        ds.duplicates
    );
    if ds.train.is_empty() && ds.eval.is_empty() {
        anstream::eprintln!(
            "{YELLOW}no reviewed pairs yet:{YELLOW:#} `glot set`, `glot labs queue approve` and hand edits feed the dataset"
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn translate(
    l: Local,
    escalate: bool,
    locale: Vec<String>,
    class: Option<String>,
) -> Result<ExitCode> {
    let mut ws = open()?;
    let llm = distill::local_config(&ws.config.llm, &l.provider, &l.model, l.base_url, escalate);
    let locales = (!locale.is_empty()).then_some(locale);
    let o = distill::translate(&mut ws, &llm, l.k, locales, class.as_deref()).await?;
    let written: usize = o.report.written.values().map(Vec::len).sum::<usize>()
        + o.report.tm_reused.values().map(Vec::len).sum::<usize>();
    anstream::println!(
        "{GREEN}wrote{GREEN:#} {written}  {DIM}local {} · escalated {} · cost ${:.4}{DIM:#}",
        o.local,
        o.escalated,
        o.usage.cost_usd
    );
    for (loc, t) in &o.rejected {
        let why: Vec<&str> = t.issues.iter().map(|i| i.message.as_str()).collect();
        anstream::eprintln!("{RED}rejected{RED:#} {loc} {}: {}", t.key, why.join("; "));
    }
    if !o.rejected.is_empty() && !escalate {
        anstream::eprintln!("{DIM}--escalate sends these to the cloud model{DIM:#}");
    }
    Ok(if o.rejected.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn eval(
    l: Local,
    limit: usize,
    eval_pct: u32,
    cloud_provider: Option<String>,
    no_write: bool,
    yes: bool,
    json: bool,
) -> Result<ExitCode> {
    let ws = open()?;
    let ds = distill::collect(&ws, eval_pct);
    let mut held = ds.eval.clone();
    held.truncate(limit);
    if held.is_empty() {
        anyhow::bail!(
            "no reviewed pairs to evaluate on (`glot labs distill` shows what the dataset holds)"
        );
    }
    let pool = distill::pool(&ws, &ds.train, &ds.eval);
    let mut cloud = ws.config.llm.clone();
    if let Some(p) = cloud_provider {
        cloud.provider = p;
    }
    // Raw lanes: no escalation, so each model is measured on its own.
    cloud.fallback_model = None;
    let local = distill::local_config(&cloud, &l.provider, &l.model, l.base_url, false);
    let paid = !["mock", "local"].contains(&cloud.provider.as_str());
    if paid
        && !yes
        && !confirm(&format!(
            "eval sends {} pairs to {} (translation + judge). Continue?",
            held.len(),
            cloud.model
        ))?
    {
        return Ok(ExitCode::SUCCESS);
    }
    let lanes = [
        Lane {
            name: "local".into(),
            llm: local.clone(),
            rag: false,
        },
        Lane {
            name: "local+rag".into(),
            llm: local.clone(),
            rag: true,
        },
        Lane {
            name: "cloud".into(),
            llm: cloud.clone(),
            rag: false,
        },
    ];
    let judge_model = cloud
        .judge_model
        .clone()
        .unwrap_or_else(|| cloud.model.clone());
    let judge = glot_llm::provider::from_config(&cloud, None)?;
    let (scores, judge_usage) = distill::eval(
        &ws,
        &held,
        &pool,
        &lanes,
        Some((judge.as_ref(), judge_model.as_str())),
        l.k,
    )
    .await?;
    let (text, block) = distill::recommend(&scores, &local, &cloud);
    let wrote = !no_write && !block.is_empty();
    if wrote {
        distill::write_proposal(&ws.root, &block)?;
    }
    if json {
        print_json(&serde_json::json!({
            "scores": scores, "recommendation": text, "judge_model": judge_model,
            "judge_cost_usd": judge_usage.cost_usd, "proposal": block,
        }))?;
        return Ok(ExitCode::SUCCESS);
    }
    anstream::println!(
        "{BOLD}{} held-out pairs{BOLD:#}, retrieval from {} approved pairs, judge {judge_model}\n",
        held.len(),
        pool.len()
    );
    anstream::println!(
        "{BOLD}{:<10} {:<22} {:>6} {:>7} {:>7} {:>9} {:>7} {:>9} {:>8}{BOLD:#}",
        "lane",
        "model",
        "judge",
        "judged",
        "ref sim",
        "validator",
        "1st try",
        "cost",
        "latency"
    );
    for s in &scores {
        let pct = |v: usize| 100 * v / s.n.max(1);
        anstream::println!(
            "{:<10} {:<22} {:>6} {:>7} {:>7.2} {:>8}% {:>6}% {:>9} {:>7.1}s",
            s.lane,
            s.model,
            s.judge.map_or("-".into(), |j| format!("{j:.1}")),
            format!("{}/{}", s.judged, s.n),
            s.ref_sim,
            pct(s.valid),
            pct(s.first_pass),
            format!("${:.4}", s.cost_usd),
            s.secs
        );
    }
    anstream::println!("\n{BOLD}by class{BOLD:#} (judge / validator)");
    for s in &scores {
        let row: Vec<String> = s
            .classes
            .iter()
            .map(|(c, v)| {
                let j = if v.judged == 0 {
                    "-".into()
                } else {
                    format!("{:.0}", v.judge)
                };
                format!("{c} {j}/{}%", 100 * v.valid / v.n.max(1))
            })
            .collect();
        anstream::println!("  {:<10} {}", s.lane, row.join("  "));
    }
    anstream::println!(
        "\n{GREEN}→{GREEN:#} {text}\n{DIM}judge cost ${:.4}; the judge is {judge_model}: it may favour its own family. `ref sim` compares with your reviewed translations.{DIM:#}",
        judge_usage.cost_usd
    );
    if wrote {
        anstream::println!("{DIM}proposed routing written to glot.toml (commented){DIM:#}");
    }
    Ok(ExitCode::SUCCESS)
}
