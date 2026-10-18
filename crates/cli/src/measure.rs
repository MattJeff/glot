//! `glot measure`: render the source locale, record each key's pixel budget in `.glot/layout.json`
//! (box width × lines + font) and save the app's webfonts to `.glot/fonts/`. `glot translate` then
//! rejects translations that would not fit, before writing them.
use anyhow::{Context, Result, bail};
use std::process::ExitCode;

use glot_core::measure::{FONTS_DIR, Fit, LAYOUT_FILE, load_layout, measure_text, save_layout};

use crate::style::*;

#[derive(clap::Args)]
pub struct MeasureArgs {
    #[command(flatten)]
    pub qa: crate::fit::QaArgs,
}

pub fn run(a: MeasureArgs) -> Result<ExitCode> {
    let ws = crate::open()?;
    let out = ws.root.join(".glot/measure");
    let qa = &a.qa;
    anstream::eprintln!(
        "{DIM}Measuring {} × {} ({})…{DIM:#}",
        ws.config.source_locale,
        qa.routes.join(","),
        qa.viewports.join(",")
    );
    let o = crate::fit::qa_command()
        .current_dir(&ws.root)
        .args([
            "--base-url",
            &qa.base_url,
            "--routes",
            &qa.routes.join(","),
            "--locales",
            "",
            "--source-locale",
            &ws.config.source_locale,
            "--url-pattern",
            &qa.url_pattern,
            "--viewports",
            &qa.viewports.join(","),
            "--catalogs",
            &ws.config.files.join(","),
            "--fail-on",
            "none",
            "--measure",
        ])
        .arg("--fonts-dir")
        .arg(ws.root.join(FONTS_DIR))
        .arg("--out")
        .arg(&out)
        .output()
        .context("cannot run visual-qa (node >= 25 and packages/visual-qa deps installed?)")?;
    if !o.status.success() {
        bail!("visual-qa failed: {}", String::from_utf8_lossy(&o.stderr));
    }
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("report.json"))
            .context("visual-qa wrote no report.json")?,
    )?;
    let measured: std::collections::BTreeMap<String, Fit> =
        serde_json::from_value(report["layout"].clone()).unwrap_or_default();

    // Keys measured now replace their old budget; keys on routes not rendered this time are kept.
    let mut layout = load_layout(&ws.root)?;
    let source = ws.source();
    let mut drift = vec![];
    for (key, fit) in &measured {
        if !source.entries.contains_key(key) {
            continue;
        }
        // Our shaping vs the browser on the source itself: a large gap means a font we could not load.
        if let (Some(t), Some(px)) = (&fit.text, fit.text_px) {
            let ours = measure_text(t, &fit.font);
            if px > 0.0 && ((ours - px) / px).abs() > 0.05 {
                drift.push(format!(
                    "{key} ({}: {ours:.0}px here vs {px:.0}px in the browser)",
                    fit.font.family
                ));
            }
        }
        layout.insert(key.clone(), fit.clone());
    }
    save_layout(&ws.root, &layout)?;

    let mut fonts: Vec<String> = measured
        .values()
        .map(|f| format!("{} {}px/{}", f.font.family, f.font.size_px, f.font.weight))
        .collect();
    fonts.sort();
    fonts.dedup();
    anstream::println!(
        "{GREEN}measured{GREEN:#} {} key(s) → {LAYOUT_FILE}  {DIM}({} total, {} font(s): {}){DIM:#}",
        measured.len(),
        layout.len(),
        fonts.len(),
        fonts.join(", ")
    );
    for (key, fit) in measured
        .iter()
        .filter(|(_, f)| f.max_lines == Some(1))
        .take(8)
    {
        anstream::println!(
            "  {BOLD}{key}{BOLD:#}  {DIM}{}{DIM:#}",
            glot_core::measure::hint(fit)
        );
    }
    if !drift.is_empty() {
        anstream::println!(
            "{YELLOW}calibrated{YELLOW:#} {} key(s) whose font metrics differ from the browser by > 5 %: {}",
            drift.len(),
            drift.join(", ")
        );
    }
    Ok(ExitCode::SUCCESS)
}
