//! `glot labs render`: render safety per locale without a browser. Glyph coverage of the
//! webfonts (cmap), bidi bugs in RTL strings, typography (no-break spaces, quotes, digits).
//! `--fix` writes the repairs through the validator (`set_translation`), keeping the provenance.
use anyhow::Result;
use std::collections::BTreeMap;
use std::process::ExitCode;

use glot_core::lock::Origin;
use glot_core::render::{self, Report};
use glot_core::validate::Severity;

use crate::style::*;

#[derive(clap::Args)]
pub struct RenderArgs {
    /// Locales to check (default: every locale, source included).
    #[arg(long, value_delimiter = ',')]
    pub locale: Vec<String>,
    /// Font files, globs or family names to check (default: glot.toml `[measure] fonts`, else every font file in the repo).
    #[arg(long)]
    pub font: Vec<String>,
    /// Apply the fixable findings (FSI/PDI isolation, RLM, no-break spaces, quotes, decimal separator); each goes through the validator.
    #[arg(long)]
    pub fix: bool,
    #[arg(long)]
    pub json: bool,
}

pub fn run(a: RenderArgs) -> Result<ExitCode> {
    let mut ws = crate::open()?;
    let mut fixed = None;
    if a.fix {
        let r = render::report(&ws.root, &ws.config, &ws.catalogs, &a.locale, &a.font);
        fixed = Some(fix(&mut ws, &r)?);
        ws.reload()?;
    }
    let r = render::report(&ws.root, &ws.config, &ws.catalogs, &a.locale, &a.font);
    if a.json {
        let mut v = serde_json::to_value(&r)?;
        if let Some((n, skipped)) = fixed {
            v["fixed"] = n.into();
            v["fix_rejected"] = skipped.into();
        }
        crate::print_json(&v)?;
    } else {
        print(&r, fixed);
    }
    Ok(if r.errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// (written, rejected by the validator). Source-locale findings are reported, never rewritten.
fn fix(ws: &mut glot_llm::engine::Workspace, r: &Report) -> Result<(usize, usize)> {
    let mut keys: Vec<(String, String)> = r
        .findings
        .iter()
        .filter(|f| f.fixable && f.locale != ws.config.source_locale)
        .map(|f| (f.locale.clone(), f.key.clone()))
        .collect();
    keys.sort();
    keys.dedup();
    let (mut n, mut rejected) = (0, 0);
    for (locale, key) in keys {
        let Some(text) = ws.catalogs.get(&locale).and_then(|c| c.entries.get(&key)) else {
            continue;
        };
        let source = ws.source().entries.get(&key).cloned();
        let Some(new) = render::fix_entry(&locale, source.as_deref(), text, ws.config.syntax)
        else {
            continue;
        };
        let prev = ws
            .lock
            .entries
            .get(&locale)
            .and_then(|m| m.get(&key))
            .cloned();
        let issues = ws.set_translation(&locale, &key, &new)?;
        if glot_core::validate::has_errors(&issues) {
            rejected += 1;
            continue;
        }
        n += 1;
        // A typographic repair is not a human review: keep the machine/imported origin.
        if let (Some(p), Some(src)) = (prev, source)
            && p.origin != Origin::Human
        {
            ws.lock.record(&locale, &key, &src, &new, p.origin, p.score);
            ws.lock.save(&ws.root)?;
        }
    }
    Ok((n, rejected))
}

fn print(r: &Report, fixed: Option<(usize, usize)>) {
    if let Some((n, rejected)) = fixed {
        anstream::println!(
            "{GREEN}fixed{GREEN:#} {n} string(s){}\n",
            if rejected > 0 {
                format!(", {rejected} left as is (the validator refused the fix)")
            } else {
                String::new()
            }
        );
    }
    anstream::println!("{BOLD}Fonts{BOLD:#}");
    for f in &r.fonts {
        anstream::println!(
            "  {BOLD}{}{BOLD:#}  {DIM}{} glyphs · {} file(s) · {}{DIM:#}",
            f.family,
            f.glyphs,
            f.files.len(),
            f.origin
        );
    }
    for n in &r.notes {
        anstream::println!("  {DIM}{n}{DIM:#}");
    }
    if !r.glyphs.is_empty() {
        anstream::println!(
            "\n{BOLD}Glyphs{BOLD:#}  {DIM}characters the font has no glyph for: drawn by a fallback font (other look and metrics){DIM:#}"
        );
        for g in &r.glyphs {
            let sample: String = g.missing.chars().take(12).collect();
            anstream::println!(
                "  {YELLOW}{:<6}{YELLOW:#} {}: {BOLD}{} key(s){BOLD:#} fall back to {} · {} char(s) {DIM}({}) {sample}{}{DIM:#}",
                g.locale,
                g.family,
                g.keys,
                g.fallback,
                g.missing.chars().count(),
                g.scripts.join(", "),
                if g.missing.chars().count() > 12 {
                    "…"
                } else {
                    ""
                }
            );
            anstream::println!(
                "         {DIM}add a subset ≈ {} KB woff2, unicode-range: {} · e.g. {}{DIM:#}",
                g.estimated_kb,
                g.unicode_range,
                g.key_sample.join(", ")
            );
        }
    }
    if !r.findings.is_empty() {
        anstream::println!("\n{BOLD}Bidi & typography{BOLD:#}");
        let shown = 60;
        for f in r.findings.iter().take(shown) {
            let (c, label) = match f.severity {
                Severity::Error => (RED, "error"),
                Severity::Warning => (YELLOW, "warn"),
            };
            anstream::println!(
                "  {c}{label:<5}{c:#} {:<6} {:<9} {BOLD}{}{BOLD:#}  {}{}",
                f.locale,
                format!("{:?}", f.check).to_lowercase(),
                f.key.replace('\n', "⏎"),
                f.message,
                if f.fixable {
                    format!(" {GREEN}[fix]{GREEN:#}")
                } else {
                    String::new()
                }
            );
        }
        if r.findings.len() > shown {
            anstream::println!(
                "  {DIM}… {} more (--json, or --locale){DIM:#}",
                r.findings.len() - shown
            );
        }
        // locale → check → (errors, warnings)
        let mut by: BTreeMap<&str, BTreeMap<String, (usize, usize)>> = BTreeMap::new();
        for f in &r.findings {
            let e = by
                .entry(&f.locale)
                .or_default()
                .entry(format!("{:?}", f.check).to_lowercase())
                .or_default();
            match f.severity {
                Severity::Error => e.0 += 1,
                Severity::Warning => e.1 += 1,
            }
        }
        anstream::println!("\n{BOLD}Per locale{BOLD:#}");
        for (l, checks) in by {
            let parts: Vec<String> = checks
                .iter()
                .map(|(c, (e, w))| match (e, w) {
                    (0, w) => format!("{c} {w}"),
                    (e, 0) => format!("{c} {RED}{e}{RED:#}"),
                    (e, w) => format!("{c} {RED}{e}{RED:#}+{w}"),
                })
                .collect();
            anstream::println!("  {l:<6} {}", parts.join(" · "));
        }
    }
    anstream::println!(
        "\n{} error(s), {} warning(s){}",
        r.errors,
        r.warnings,
        if r.fixable > 0 {
            format!(", {} fixable: glot labs render --fix", r.fixable)
        } else {
            String::new()
        }
    );
}
