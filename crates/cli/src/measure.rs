//! `glot measure`: render the source locale, record each key's pixel budget in `.glot/layout.json`
//! (box width × lines + font) and save the app's webfonts to `.glot/fonts/`. `glot translate` then
//! rejects translations that would not fit, before writing them.
use anyhow::{Context, Result};
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
    let o = crate::fit::node_tool("visual-qa")
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
        .with_context(|| crate::fit::tool_missing("visual-qa"))?;
    if !o.status.success() {
        return Err(crate::fit::tool_failed("visual-qa", &o.stderr));
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
    // A key rendered in boxes of different sizes keeps the tightest; the detail stays in layout.json.
    for (key, fit) in &measured {
        let mut widths: Vec<f32> = fit.occurrences.iter().map(|o| o.max_width_px).collect();
        widths.sort_by(f32::total_cmp);
        widths.dedup();
        if widths.len() > 1 {
            anstream::println!(
                "  {BOLD}{key}{BOLD:#}  {DIM}{} occurrence(s), budgets {}px → {}px kept{DIM:#}",
                fit.occurrences.len(),
                widths
                    .iter()
                    .map(|w| w.to_string())
                    .collect::<Vec<_>>()
                    .join("/"),
                fit.max_width_px
            );
        }
    }
    // Same source text under several keys, mapped by text: only the first key got a budget.
    let mut by_text: std::collections::BTreeMap<String, Vec<&String>> = Default::default();
    for (k, v) in &source.entries {
        by_text.entry(v.trim().to_lowercase()).or_default().push(k);
    }
    let ambiguous: Vec<String> = measured
        .iter()
        .filter(|(_, f)| f.occurrences.iter().any(|o| o.by == "text"))
        .filter_map(|(k, _)| {
            let same = &by_text[&source.entries.get(k)?.trim().to_lowercase()];
            (same.len() > 1).then(|| {
                same.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" = ")
            })
        })
        .collect();
    if !ambiguous.is_empty() {
        anstream::println!(
            "{YELLOW}ambiguous{YELLOW:#} same text under several keys, mapped to the first: {}. Run the app with GLOT_DEV_KEYS=1 (getGlotMessages from @glot/runtime/dev) for exact keys.",
            ambiguous.join(", ")
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
