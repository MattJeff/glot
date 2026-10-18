//! `glot coverage`: branch coverage of plural/select/selectordinal messages, per locale and CLDR category.
//!
//! Every combination of selector cases (`glot_core::branches`) is rendered as text and measured against
//! its pixel budget (`.glot/layout.json`, no browser). `--render` mounts every branch on the running app's
//! page (its CSS and webfonts) through visual-qa's harness mode — no app instrumentation — and records
//! the result per branch in `.glot/coverage.json`, so the report can say which branches were never seen.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::ExitCode;

use glot_core::branches::{self, Branch};
use glot_core::catalog::Catalog;
use glot_core::hash::hash;
use glot_core::measure::{self, load_layout};
use glot_core::validate::{Severity, Syntax};

use crate::style::*;

pub const COVERAGE_FILE: &str = ".glot/coverage.json";

#[derive(clap::Args)]
pub struct CoverageArgs {
    /// Locales (comma separated). Default: source, targets and register variants.
    #[arg(long, value_delimiter = ',')]
    pub locale: Vec<String>,
    /// Only these keys.
    #[arg(long, value_delimiter = ',')]
    pub key: Vec<String>,
    /// Mount every branch on the app page (`--base-url`, first `--routes`) and check overflow/truncation.
    #[arg(long)]
    pub render: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub qa: crate::fit::QaArgs,
}

/// Last browser check of a branch; only valid while the branch text hashes the same.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Seen {
    text_hash: String,
    /// `null` = rendered without issue, else the visual-qa kind (`overflow`, `clamp`).
    issue: Option<String>,
}

/// locale -> key -> branch id -> last check.
type Store = BTreeMap<String, BTreeMap<String, BTreeMap<String, Seen>>>;

#[derive(Serialize)]
struct Row {
    locale: String,
    key: String,
    #[serde(flatten)]
    branch: Branch,
    /// `fits` / `too_wide` against `.glot/layout.json`; `null` when the key has no budget.
    measured: Option<&'static str>,
    /// `ok` / visual-qa kind when rendered in a browser for this exact text; `null` = never seen.
    visual: Option<String>,
}

#[derive(Serialize, Default)]
struct Tally {
    branches: usize,
    visual: usize,
    failing: usize,
    too_wide: usize,
}

pub fn run(a: CoverageArgs) -> Result<ExitCode> {
    let ws = crate::open()?;
    let syntax = ws.config.syntax;
    if syntax == Syntax::I18next {
        // ponytail: i18next plurals are `_one`/`_few` sibling keys, already one message per category.
        bail!("glot coverage needs ICU or MF2 messages (i18next plurals are separate `_few` keys)");
    }
    let layout = load_layout(&ws.root)?;
    let mut locales = ws.config.all_locales();
    let declared = glot_core::variants::load(&ws.root)?;
    locales.extend(
        glot_core::variants::locales(&declared)
            .into_iter()
            .map(|(_, t)| t),
    );
    locales.retain(|l| a.locale.is_empty() || a.locale.contains(l));

    let mut rows = vec![];
    for loc in &locales {
        let cat = match ws.catalogs.get(loc) {
            Some(c) => c.clone(),
            None => Catalog::load(&ws.config, &ws.root, loc)?,
        };
        for (key, msg) in &cat.entries {
            if !a.key.is_empty() && !a.key.contains(key) {
                continue;
            }
            // Broken messages are `glot check`'s job.
            let Ok(list) = branches::enumerate(msg, loc, syntax) else {
                continue;
            };
            for b in list.into_iter().filter(|b| !b.cases.is_empty()) {
                let measured =
                    layout
                        .get(key)
                        .map(|fit| match measure::check(&b.text, Syntax::Icu, fit) {
                            Some(i) if i.severity == Severity::Error => "too_wide",
                            _ => "fits",
                        });
                rows.push(Row {
                    locale: loc.clone(),
                    key: key.clone(),
                    branch: b,
                    measured,
                    visual: None,
                });
            }
        }
    }

    let store_path = ws.root.join(COVERAGE_FILE);
    let mut store: Store = std::fs::read_to_string(&store_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if a.render {
        render(&ws, &a.qa, &rows, &layout, &mut store)?;
        std::fs::create_dir_all(store_path.parent().unwrap())?;
        std::fs::write(&store_path, serde_json::to_string_pretty(&store)? + "\n")?;
    }
    for r in &mut rows {
        r.visual = store
            .get(&r.locale)
            .and_then(|k| k.get(&r.key))
            .and_then(|b| b.get(&r.branch.id))
            .filter(|s| s.text_hash == hash(&r.branch.text))
            .map(|s| s.issue.clone().unwrap_or_else(|| "ok".into()));
    }

    // locale -> category -> tally
    let mut tallies: BTreeMap<&str, BTreeMap<&str, Tally>> = BTreeMap::new();
    for r in &rows {
        let t = tallies
            .entry(&r.locale)
            .or_default()
            .entry(&r.branch.category)
            .or_default();
        t.branches += 1;
        t.visual += r.visual.is_some() as usize;
        t.failing += r.visual.as_deref().is_some_and(|v| v != "ok") as usize;
        t.too_wide += (r.measured == Some("too_wide")) as usize;
    }
    let failed = rows
        .iter()
        .any(|r| r.measured == Some("too_wide") || r.visual.as_deref().is_some_and(|v| v != "ok"));
    if a.json {
        crate::print_json(&serde_json::json!({ "summary": tallies, "branches": rows }))?;
        return Ok(if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        });
    }
    if rows.is_empty() {
        anstream::println!("No plural/select messages: nothing to cover.");
        return Ok(ExitCode::SUCCESS);
    }
    anstream::println!(
        "{BOLD}{:<14} {:<10} {:>8} {:>16} {:>8} {:>9}{BOLD:#}",
        "locale",
        "branch",
        "count",
        "seen in browser",
        "failing",
        "too wide"
    );
    for (loc, cats) in &tallies {
        for (cat, t) in cats {
            let pct = 100 * t.visual / t.branches.max(1);
            let color = if t.failing + t.too_wide > 0 {
                RED
            } else if pct < 100 {
                YELLOW
            } else {
                GREEN
            };
            anstream::println!(
                "{:<14} {:<10} {:>8} {color}{:>9} ({pct:>3} %){color:#} {:>8} {:>9}",
                loc,
                cat,
                t.branches,
                t.visual,
                t.failing,
                t.too_wide
            );
        }
    }
    for r in rows.iter().filter(|r| {
        r.measured == Some("too_wide") || r.visual.as_deref().is_some_and(|v| v != "ok")
    }) {
        let why = r
            .visual
            .as_deref()
            .filter(|v| *v != "ok")
            .unwrap_or("too_wide");
        anstream::println!(
            "  {RED}✗{RED:#} {} {} [{}] {why}: {:?}",
            r.locale,
            r.key,
            r.branch.id,
            r.branch.text
        );
    }
    let unseen = rows.iter().filter(|r| r.visual.is_none()).count();
    if unseen > 0 {
        let unbudgeted = rows.iter().filter(|r| r.measured.is_none()).count();
        anstream::println!(
            "{DIM}{unseen} branch(es) never rendered in a browser: `glot coverage --render --base-url …`{}{DIM:#}",
            if unbudgeted > 0 {
                format!(" ({unbudgeted} without a pixel budget: run `glot measure` first)")
            } else {
                String::new()
            }
        );
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One box per branch with the key's measured budget (width, lines, font), on the app's page.
fn harness(rows: &[Row], layout: &measure::Layout) -> BTreeMap<String, String> {
    let mut pages: BTreeMap<String, String> = BTreeMap::new();
    for r in rows {
        let Some(fit) = layout.get(&r.key) else {
            continue;
        };
        let f = &fit.font;
        let lines = match fit.max_lines {
            Some(1) => "white-space:nowrap;overflow:hidden;".to_string(),
            Some(n) => format!(
                "display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:{n};overflow:hidden;"
            ),
            None => "overflow:hidden;".to_string(),
        };
        let style = format!(
            "box-sizing:content-box;padding:0;margin:0 0 8px;width:{}px;font-family:'{}',sans-serif;font-size:{}px;font-weight:{};letter-spacing:{}px;text-transform:{};{lines}",
            fit.max_width_px,
            f.family.replace('\'', ""),
            f.size_px,
            f.weight,
            f.letter_spacing_px,
            f.text_transform.as_deref().unwrap_or("none"),
        );
        pages
            .entry(r.locale.clone())
            .or_default()
            .push_str(&format!(
                "<div data-glot-key=\"{}\" style=\"{}\">{}</div>\n",
                esc(&format!("{}#{}", r.key, r.branch.id)),
                esc(&style),
                esc(&r.branch.text)
            ));
    }
    pages
}

fn render(
    ws: &glot_llm::engine::Workspace,
    qa: &crate::fit::QaArgs,
    rows: &[Row],
    layout: &measure::Layout,
    store: &mut Store,
) -> Result<()> {
    let pages = harness(rows, layout);
    if pages.is_empty() {
        bail!("no branch has a pixel budget: run `glot measure --base-url …` first");
    }
    let out = ws.root.join(".glot/coverage");
    std::fs::create_dir_all(&out)?;
    let file = out.join("harness.json");
    std::fs::write(&file, serde_json::to_string(&pages)?)?;
    let n: usize = pages.values().map(|p| p.matches("<div").count()).sum();
    anstream::eprintln!("{DIM}Rendering {n} branch(es) on {}…{DIM:#}", qa.base_url);
    let o = crate::fit::node_tool("visual-qa")
        .current_dir(&ws.root)
        .args([
            "--base-url",
            &qa.base_url,
            "--routes",
            &qa.routes.join(","),
            "--source-locale",
            &ws.config.source_locale,
            "--url-pattern",
            &qa.url_pattern,
            "--viewports",
            &qa.viewports.join(","),
            "--fail-on",
            "none",
        ])
        .arg("--harness")
        .arg(&file)
        .arg("--out")
        .arg(out.join("visual"))
        .output()
        .with_context(|| crate::fit::tool_missing("visual-qa"))?;
    if !o.status.success() {
        return Err(crate::fit::tool_failed("visual-qa", &o.stderr));
    }
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("visual/report.json"))
            .context("visual-qa wrote no report.json")?,
    )?;
    let mut issues: BTreeMap<(String, String), String> = BTreeMap::new();
    for i in report["issues"].as_array().into_iter().flatten() {
        if let (Some(l), Some(k), Some(kind)) =
            (i["locale"].as_str(), i["key"].as_str(), i["kind"].as_str())
            && matches!(kind, "overflow" | "clamp")
        {
            issues.insert((l.into(), k.into()), kind.into());
        }
    }
    for r in rows.iter().filter(|r| layout.contains_key(&r.key)) {
        let id = format!("{}#{}", r.key, r.branch.id);
        store
            .entry(r.locale.clone())
            .or_default()
            .entry(r.key.clone())
            .or_default()
            .insert(
                r.branch.id.clone(),
                Seen {
                    text_hash: hash(&r.branch.text),
                    issue: issues.get(&(r.locale.clone(), id)).cloned(),
                },
            );
    }
    Ok(())
}
