//! `glot labs markets`: which language to add next, with demand (server insights, when `[server]` is set),
//! public market data, the exact catalog cost and the technical effort. `--add` writes it to glot.toml.
use anyhow::{Context, Result, bail};
use glot_core::{
    context::KeyContext,
    markets::{self, Demand},
};
use glot_llm::translate::{Item, Request};
use serde::Deserialize;
use std::path::Path;
use std::process::ExitCode;

use crate::style::{BOLD, DIM, GREEN, YELLOW};

#[derive(clap::Args)]
pub struct MarketsArgs {
    /// Insights window (days) when a glot server is configured.
    #[arg(long, default_value_t = 30)]
    days: i64,
    /// Rows to show.
    #[arg(long, default_value_t = 8)]
    top: usize,
    /// Add this locale to target_locales in glot.toml, then run `glot translate --dry-run` for it.
    #[arg(long)]
    add: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Deserialize)]
struct Overview {
    total_visits: i64,
    demand: Vec<Demand>,
}

/// Demand from the server's insights (`[server]` + token), else None: public data only.
async fn demand(ws: &glot_llm::engine::Workspace, days: i64) -> Option<Overview> {
    let srv = ws.config.server.as_ref()?;
    let token = std::env::var(&srv.token_env).ok()?;
    let url = format!(
        "{}/api/projects/{}/insights?days={days}",
        srv.url.trim_end_matches('/'),
        srv.project
    );
    let res = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .and_then(|r| r.error_for_status());
    match res {
        Ok(r) => r.json().await.ok(),
        Err(e) => {
            anstream::eprintln!(
                "{YELLOW}warning:{YELLOW:#} no visitor demand ({e}); public data only"
            );
            None
        }
    }
}

pub async fn run(a: MarketsArgs) -> Result<ExitCode> {
    if let Some(loc) = a.add {
        return add(&loc).await;
    }
    let ws = crate::open()?;
    let offered: Vec<String> = ws.config.all_locales();
    let source = ws.source();
    let items: Vec<Item> = source
        .entries
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| Item {
            key: k.clone(),
            source: v.clone(),
            context: KeyContext {
                key: k.clone(),
                ..Default::default()
            },
            previous: None,
        })
        .collect();
    let cost = |loc: &str| {
        let req = Request {
            source_locale: ws.config.source_locale.clone(),
            target_locale: loc.into(),
            items: items.clone(),
            glossary: vec![],
            syntax: ws.config.syntax,
            style: ws.config.llm.style.clone(),
        };
        glot_llm::translate::estimate(&ws.config, &req).cost_usd
    };
    let o = demand(&ws, a.days).await;
    let (total, dem) = o.map_or((0, vec![]), |o| (o.total_visits, o.demand));
    let words = markets::words(source.entries.values());
    let mut ranked = markets::rank(&offered, total, &dem, words, cost);
    ranked.truncate(a.top);
    if a.json {
        crate::print_json(&ranked)?;
        return Ok(ExitCode::SUCCESS);
    }
    anstream::println!(
        "{BOLD}Which language next?{BOLD:#}  {DIM}{} keys, {words} words, model {}; {}; public data as of {}{DIM:#}\n",
        items.len(),
        ws.config.llm.model,
        if total > 0 {
            format!("{total} visits in {} days", a.days)
        } else {
            "no visitor data (set [server] + token for insights)".into()
        },
        markets::DATA_AS_OF
    );
    anstream::println!(
        "{BOLD}{:<7} {:<22} {:>5} {:>7} {:>9} {:>8} {:>7}  effort{BOLD:#}",
        "locale",
        "language",
        "score",
        "demand",
        "users",
        "cost",
        "review"
    );
    for o in &ranked {
        anstream::println!(
            "{:<7} {:<22} {:>5} {:>6.1}% {:>8.0}M {:>8} {:>7}  {}",
            o.locale,
            o.name,
            o.score,
            o.share * 100.0,
            o.internet_users_m,
            markets::usd(o.cost_usd),
            markets::duration(o.review_hours),
            if o.effort_notes.is_empty() {
                "none".into()
            } else {
                o.effort_notes.join(", ")
            }
        );
    }
    if let Some(best) = ranked.first() {
        anstream::println!(
            "\n{GREEN}→{GREEN:#} {}\n  {DIM}glot labs markets --add {}{DIM:#}",
            best.summary,
            best.locale
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn add(loc: &str) -> Result<ExitCode> {
    if glot_core::plural::language(loc).len() < 2
        || !loc.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        bail!("invalid locale \"{loc}\" (BCP 47, e.g. pt-BR)");
    }
    let ws = crate::open()?;
    if ws.config.all_locales().iter().any(|l| l == loc) {
        bail!("{loc} is already in glot.toml");
    }
    let path = ws.root.join(glot_core::config::CONFIG_FILE);
    add_target(&path, loc)?;
    anstream::println!(
        "{GREEN}added{GREEN:#} {loc} to target_locales in {}",
        path.display()
    );
    crate::translate(vec![loc.into()], vec![], false, true, false, None, false).await?;
    anstream::println!("{DIM}run `glot translate --locale {loc}` to translate it{DIM:#}");
    Ok(ExitCode::SUCCESS)
}

/// Append `loc` to the `target_locales = [...]` line, keeping the rest of the file (comments) intact.
fn add_target(path: &Path, loc: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let mut done = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            if done || !line.trim_start().starts_with("target_locales") {
                return Ok(line.to_string());
            }
            // ponytail: single-line arrays only (what `glot init` writes); multi-line ones bail.
            let v: toml::Table = toml::from_str(line)
                .context("target_locales spans several lines: add the locale by hand")?;
            let mut list: Vec<String> = v["target_locales"]
                .clone()
                .try_into()
                .context("target_locales is not a list of strings")?;
            list.push(loc.into());
            done = true;
            let tail = line.rfind(']').map_or("", |i| &line[i + 1..]);
            Ok(format!(
                "target_locales = {}{tail}",
                toml::Value::from(list)
            ))
        })
        .collect::<Result<_>>()?;
    if !done {
        bail!("no target_locales line in {}", path.display());
    }
    std::fs::write(path, out.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn add_target_keeps_comments() {
        let dir = std::env::temp_dir().join(format!("glot-markets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("glot.toml");
        std::fs::write(
            &p,
            "# mine\nsource_locale = \"en\"\ntarget_locales = [\"fr\"] # shipped\n[llm]\n",
        )
        .unwrap();
        super::add_target(&p, "pt-BR").unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.starts_with("# mine\n"), "{s}");
        assert!(
            s.contains("target_locales = [\"fr\", \"pt-BR\"] # shipped\n"),
            "{s}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
