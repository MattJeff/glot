//! `glot queue` (human review of routed translations) and `glot tm` (translation memory as TMX).
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;

use glot_llm::{
    engine::Workspace,
    tm::{Tm, esc},
};

use crate::style::*;

#[derive(Args)]
pub struct QueueArgs {
    #[command(subcommand)]
    cmd: Option<QueueCmd>,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum QueueCmd {
    /// Accept the current translation as is.
    Approve { locale: String, key: String },
    /// Replace the translation (validated) and approve it.
    Edit {
        locale: String,
        key: String,
        value: String,
    },
    /// Export the queue for an external reviewer (stdout without --output).
    Export {
        #[arg(long, value_enum, default_value_t = Fmt::Csv)]
        format: Fmt,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Re-import a reviewed CSV/XLIFF file: unchanged targets are approved, edited ones set and approved.
    Import { file: PathBuf },
}

#[derive(Clone, Copy, ValueEnum)]
enum Fmt {
    Csv,
    Xliff,
}

#[derive(serde::Serialize)]
struct Pending {
    locale: String,
    key: String,
    source: String,
    target: String,
    reason: String,
    score: Option<u8>,
}

fn pending(ws: &Workspace) -> Vec<Pending> {
    let source = &ws.source().entries;
    let mut out = vec![];
    for (loc, entries) in &ws.lock.entries {
        for (key, e) in entries {
            let Some(reason) = &e.needs_review else {
                continue;
            };
            out.push(Pending {
                locale: loc.clone(),
                key: key.clone(),
                source: source.get(key).cloned().unwrap_or_default(),
                target: ws
                    .catalogs
                    .get(loc)
                    .and_then(|c| c.entries.get(key))
                    .cloned()
                    .unwrap_or_default(),
                reason: reason.clone(),
                score: e.score,
            });
        }
    }
    out
}

fn open() -> Result<Workspace> {
    Workspace::open(&std::env::current_dir()?).context("cannot open glot workspace")
}

pub fn run(a: QueueArgs) -> Result<ExitCode> {
    let mut ws = open()?;
    match a.cmd {
        None => {
            let p = pending(&ws);
            if a.json {
                println!("{}", serde_json::to_string_pretty(&p)?);
            } else if p.is_empty() {
                anstream::println!("{GREEN}✓ review queue is empty{GREEN:#}");
            } else {
                for x in &p {
                    anstream::println!(
                        "  {YELLOW}●{YELLOW:#} {:<6} {}  {DIM}{}{DIM:#}\n      {:?}\n    → {:?}",
                        x.locale,
                        x.key,
                        x.reason,
                        x.source,
                        x.target
                    );
                }
                anstream::println!(
                    "{} pending · {DIM}glot queue approve <locale> <key> | edit | export{DIM:#}",
                    p.len()
                );
            }
        }
        Some(QueueCmd::Approve { locale, key }) => {
            ws.approve(&locale, &key)?;
            anstream::println!("{GREEN}✓{GREEN:#} {locale} {key} approved");
        }
        Some(QueueCmd::Edit { locale, key, value }) => {
            if !edit(&mut ws, &locale, &key, &value)? {
                return Ok(ExitCode::FAILURE);
            }
            anstream::println!("{GREEN}✓{GREEN:#} {locale} {key} set and approved");
        }
        Some(QueueCmd::Export { format, output }) => {
            let p = pending(&ws);
            let text = match format {
                Fmt::Csv => to_csv(&p),
                Fmt::Xliff => to_xliff(&ws.config.source_locale, &p),
            };
            match output {
                Some(path) => {
                    std::fs::write(&path, text)?;
                    anstream::eprintln!("{} entries → {}", p.len(), path.display());
                }
                None => print!("{text}"),
            }
        }
        Some(QueueCmd::Import { file }) => {
            let text = std::fs::read_to_string(&file)?;
            let rows = if text.trim_start().starts_with('<') {
                from_xliff(&text)?
            } else {
                from_csv(&text)?
            };
            let (mut ok, mut failed) = (0, 0);
            for (locale, key, target) in rows {
                let current = ws.catalogs.get(&locale).and_then(|c| c.entries.get(&key));
                let done = if current == Some(&target) {
                    ws.approve(&locale, &key).map(|_| true)?
                } else {
                    edit(&mut ws, &locale, &key, &target)?
                };
                if done { ok += 1 } else { failed += 1 }
            }
            anstream::println!("{GREEN}✓{GREEN:#} {ok} approved, {failed} rejected");
            if failed > 0 {
                return Ok(ExitCode::FAILURE);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Validated human edit (recorded as reviewed, out of the queue, into the TM). False if rejected.
fn edit(ws: &mut Workspace, locale: &str, key: &str, value: &str) -> Result<bool> {
    let issues = ws.set_translation(locale, key, value)?;
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == glot_core::validate::Severity::Error)
        .collect();
    for i in &errors {
        anstream::eprintln!("  {RED}✗{RED:#} {locale} {key}: {}", i.message);
    }
    Ok(errors.is_empty())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn to_csv(p: &[Pending]) -> String {
    let mut s = String::from("locale,key,source,target,reason\n");
    for x in p {
        let row = [&x.locale, &x.key, &x.source, &x.target, &x.reason];
        s += &row.map(|f| csv_field(f)).join(",");
        s.push('\n');
    }
    s
}

/// RFC 4180 records (quoted fields may hold commas, quotes and newlines).
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (vec![], vec![], String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// (locale, key, target) rows; columns found by header name.
fn from_csv(text: &str) -> Result<Vec<(String, String, String)>> {
    let mut rows = parse_csv(text).into_iter();
    let header = rows.next().unwrap_or_default();
    let col = |n: &str| header.iter().position(|h| h.trim() == n);
    let (Some(l), Some(k), Some(t)) = (col("locale"), col("key"), col("target")) else {
        bail!("CSV needs locale, key and target columns");
    };
    Ok(rows
        .filter(|r| r.len() > l.max(k).max(t))
        .map(|r| (r[l].clone(), r[k].clone(), r[t].clone()))
        .collect())
}

/// XLIFF 1.2, one `<file>` per target locale; the routing reason as a `<note>`.
fn to_xliff(source_locale: &str, p: &[Pending]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xliff version=\"1.2\" xmlns=\"urn:oasis:names:tc:xliff:document:1.2\">\n",
    );
    let mut locales: Vec<&str> = p.iter().map(|x| x.locale.as_str()).collect();
    locales.dedup();
    for loc in locales {
        s += &format!(
            "  <file original=\"glot\" datatype=\"plaintext\" source-language=\"{}\" target-language=\"{}\">\n    <body>\n",
            esc(source_locale),
            esc(loc)
        );
        for x in p.iter().filter(|x| x.locale == loc) {
            s += &format!(
                "      <trans-unit id=\"{}\">\n        <source>{}</source>\n        <target state=\"needs-review-translation\">{}</target>\n        <note>{}</note>\n      </trans-unit>\n",
                esc(&x.key),
                esc(&x.source),
                esc(&x.target),
                esc(&x.reason)
            );
        }
        s += "    </body>\n  </file>\n";
    }
    s + "</xliff>\n"
}

fn from_xliff(text: &str) -> Result<Vec<(String, String, String)>> {
    let doc = roxmltree::Document::parse(text).context("invalid XLIFF")?;
    let mut out = vec![];
    for file in doc.descendants().filter(|n| n.has_tag_name("file")) {
        let Some(loc) = file.attribute("target-language") else {
            continue;
        };
        for tu in file.descendants().filter(|n| n.has_tag_name("trans-unit")) {
            let (Some(id), Some(target)) = (
                tu.attribute("id"),
                tu.children().find(|n| n.has_tag_name("target")),
            ) else {
                continue;
            };
            let value: String = target
                .descendants()
                .filter(|d| d.is_text())
                .filter_map(|d| d.text())
                .collect();
            out.push((loc.to_string(), id.to_string(), value));
        }
    }
    Ok(out)
}

#[derive(Subcommand)]
pub enum TmCmd {
    /// Write the translation memory as TMX 1.4 (stdout without a file).
    Export { file: Option<PathBuf> },
    /// Merge a TMX file (from another project or tool) into .glot/tm.json.
    Import { file: PathBuf },
}

pub fn tm(cmd: TmCmd) -> Result<ExitCode> {
    let ws = open()?;
    let mut tm = Tm::load(&ws.root)?;
    match cmd {
        TmCmd::Export { file: Some(f) } => {
            std::fs::write(&f, tm.to_tmx())?;
            anstream::eprintln!("{} units → {}", tm.units.len(), f.display());
        }
        TmCmd::Export { file: None } => print!("{}", tm.to_tmx()),
        TmCmd::Import { file } => {
            let units = Tm::from_tmx(&std::fs::read_to_string(&file)?)?;
            let n = units.len();
            for u in units {
                tm.add(u);
            }
            tm.save(&ws.root)?;
            anstream::println!(
                "{GREEN}✓{GREEN:#} {n} units imported ({} in memory)",
                tm.units.len()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_xliff_roundtrip() {
        let p = vec![Pending {
            locale: "fr".into(),
            key: "legal.terms".into(),
            source: "Accept the \"terms\", then <b>{name}</b>\nok".into(),
            target: "Acceptez les « conditions », <b>{name}</b>\nok".into(),
            reason: "risk: legal".into(),
            score: Some(100),
        }];
        let want = vec![("fr".into(), "legal.terms".into(), p[0].target.clone())];
        assert_eq!(from_csv(&to_csv(&p)).unwrap(), want);
        assert_eq!(from_xliff(&to_xliff("en", &p)).unwrap(), want);
    }
}
