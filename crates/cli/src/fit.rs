//! `glot fit` (layouts that repair themselves) and the capture behind `glot translate --visual`.
//! Both drive `packages/visual-qa` (Playwright) and read its report.json.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use glot_core::Key;
use glot_llm::engine::Workspace;
use glot_llm::visual::{Shorten, parse_shortened, png_b64};

use crate::style::*;

#[derive(clap::Args, Clone)]
pub struct QaArgs {
    /// Running app to capture.
    #[arg(long, default_value = "http://localhost:3000")]
    pub base_url: String,
    /// Routes to capture (comma separated).
    #[arg(long, value_delimiter = ',', default_value = "/")]
    pub routes: Vec<String>,
    /// URL layout; the source locale gets no prefix.
    #[arg(long, default_value = "/{locale}{route}")]
    pub url_pattern: String,
    #[arg(long, value_delimiter = ',', default_value = "1280x800")]
    pub viewports: Vec<String>,
}

#[derive(clap::Args)]
pub struct FitArgs {
    #[command(flatten)]
    pub qa: QaArgs,
    /// Locales to fix (comma separated). Default: all targets.
    #[arg(long, value_delimiter = ',')]
    pub locales: Vec<String>,
    #[arg(long, default_value_t = 3)]
    pub max_iterations: u32,
    /// Write without asking.
    #[arg(short, long)]
    pub yes: bool,
    /// Rebuild command run after each write (prod builds); dev servers just hot-reload.
    #[arg(long)]
    pub build_cmd: Option<String>,
    /// Wait after writing before re-rendering (HMR / rebuild settle time).
    #[arg(long, default_value_t = 1500)]
    pub reload_ms: u64,
}

/// npm scope the Node helpers are published under (`scripts/publish-packages.sh`). `@glot` on npm
/// belongs to someone else; a build can switch with `GLOT_NPM_SCOPE=@glot cargo build`.
const NPM_SCOPE: &str = match option_env!("GLOT_NPM_SCOPE") {
    Some(s) => s,
    None => "@glot-i18n",
};

/// A Node helper (`visual-qa`, `agent-qa`): `$GLOT_VISUAL_QA` / `$GLOT_AGENT_QA` (a cli.js, or any
/// executable), else (debug builds only) this repo's sources, else `glot-<name>` on PATH, else
/// `npx -y <scope>/<name>@<major.minor of this glot>`.
pub(crate) fn node_tool(name: &str) -> Command {
    let node = |p: PathBuf| {
        let mut c = Command::new("node");
        c.arg(p);
        c
    };
    if let Some(p) = std::env::var_os(tool_env(name)).filter(|v| !v.is_empty()) {
        let p = PathBuf::from(p);
        return match p.extension().and_then(|e| e.to_str()) {
            Some("js" | "mjs" | "cjs" | "ts") => node(p),
            _ => Command::new(p),
        };
    }
    #[cfg(debug_assertions)]
    {
        let p =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../packages/{name}/src/cli.ts"));
        if p.exists() {
            return node(p);
        }
    }
    let bin = format!("glot-{name}");
    let exe = if cfg!(windows) {
        format!("{bin}.cmd")
    } else {
        bin.clone()
    };
    if let Some(found) = std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
    {
        return Command::new(found);
    }
    let mut c = Command::new(if cfg!(windows) { "npx.cmd" } else { "npx" });
    c.args([
        "-y",
        &format!("--package={}", tool_package(name)),
        "--",
        &bin,
    ]);
    c
}

fn tool_env(name: &str) -> String {
    format!("GLOT_{}", name.to_uppercase().replace('-', "_"))
}

fn tool_package(name: &str) -> String {
    format!(
        "{NPM_SCOPE}/{name}@{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR")
    )
}

/// A helper that ran and failed; npx not finding the package means it must be installed by hand.
pub(crate) fn tool_failed(name: &str, stderr: &[u8]) -> anyhow::Error {
    let err = String::from_utf8_lossy(stderr);
    if err.contains("npm error") || err.contains("npm ERR!") {
        anyhow::anyhow!("{}\n{err}", tool_missing(name))
    } else {
        anyhow::anyhow!("{name} failed: {err}")
    }
}

/// What to install when a Node helper cannot be started at all.
pub(crate) fn tool_missing(name: &str) -> String {
    format!(
        "cannot run glot-{name}: install Node.js >= 20 (glot runs it with `npx {pkg}`), or \
         `npm i -g {pkg} && npx playwright install chromium`, or set {env}=/path/to/cli.js",
        pkg = tool_package(name),
        env = tool_env(name),
    )
}

fn run_qa(
    ws: &Workspace,
    qa: &QaArgs,
    locales: &[String],
    out: &Path,
    map: Option<&str>,
) -> Result<Value> {
    let mut c = node_tool("visual-qa");
    c.current_dir(&ws.root).args([
        "--base-url",
        &qa.base_url,
        "--routes",
        &qa.routes.join(","),
        "--locales",
        &locales.join(","),
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
    ]);
    c.arg("--out").arg(out);
    if let Some(m) = map {
        c.args(["--map", m]);
    }
    let o = c.output().with_context(|| tool_missing("visual-qa"))?;
    if !o.status.success() {
        return Err(tool_failed("visual-qa", &o.stderr));
    }
    let report = std::fs::read_to_string(out.join("report.json"))
        .context("visual-qa wrote no report.json")?;
    Ok(serde_json::from_str(&report)?)
}

/// `glot translate --visual`: crop every planned key where it renders in the source locale.
pub fn capture(ws: &Workspace, qa: &QaArgs, keys: &[Key]) -> Result<BTreeMap<Key, String>> {
    let mut keys = keys.to_vec();
    keys.sort();
    keys.dedup();
    anstream::eprintln!(
        "{DIM}Capturing {} for visual context…{DIM:#}",
        qa.routes.join(", ")
    );
    let out = ws.root.join(".glot/visual");
    // ponytail: past a few hundred keys the argv gets long; map everything and filter here.
    let map = if keys.len() > 300 {
        "*".to_string()
    } else {
        keys.join(",")
    };
    let report = run_qa(ws, qa, &[], &out, Some(&map))?;
    let mut crops = BTreeMap::new();
    for m in report["keys"].as_array().into_iter().flatten() {
        let (Some(key), Some(crop)) = (m["key"].as_str(), m["crop"].as_str()) else {
            continue;
        };
        if m["locale"] == ws.config.source_locale.as_str() && keys.iter().any(|k| k == key) {
            crops.insert(key.to_string(), png_b64(&out.join(crop))?);
        }
    }
    anstream::eprintln!(
        "{DIM}{}/{} keys seen on screen{DIM:#}",
        crops.len(),
        keys.len()
    );
    Ok(crops)
}

#[derive(Default)]
struct Track {
    before_value: String,
    before_crop: String,
    after_value: Option<String>,
    after_crop: Option<String>,
    rejected: Vec<(String, String)>,
    fixed_at: Option<u32>,
}

fn n(v: &Value) -> u32 {
    v.as_f64().unwrap_or(0.0).round() as u32
}

pub async fn fit(a: FitArgs) -> Result<ExitCode> {
    let mut ws = crate::open()?;
    let locales: Vec<String> = if a.locales.is_empty() {
        ws.config.target_locales.clone()
    } else {
        a.locales.clone()
    };
    let provider = glot_llm::provider::from_config(&ws.config.llm, None)?;
    let model = ws.config.llm.model.clone();
    let dir = ws.root.join(".glot/fit");
    let _ = std::fs::remove_dir_all(&dir);
    let mut tracks: BTreeMap<(String, Key), Track> = BTreeMap::new();
    let mut cost = 0.0;
    let mut iter = 0;

    loop {
        iter += 1;
        let run = format!("run-{iter}");
        let out = dir.join(&run);
        let map = (!tracks.is_empty()).then(|| {
            tracks
                .keys()
                .map(|(_, k)| k.as_str())
                .collect::<Vec<_>>()
                .join(",")
        });
        anstream::eprintln!(
            "{DIM}[{iter}] rendering {} × {}…{DIM:#}",
            locales.join(","),
            a.qa.routes.join(",")
        );
        let report = run_qa(&ws, &a.qa, &locales, &out, map.as_deref())?;

        // Worst overflow per (locale, key) across routes/viewports.
        let mut issues: BTreeMap<(String, Key), &Value> = BTreeMap::new();
        for i in report["issues"].as_array().into_iter().flatten() {
            let (Some(loc), Some(key)) = (i["locale"].as_str(), i["key"].as_str()) else {
                continue;
            };
            if !matches!(i["kind"].as_str(), Some("overflow" | "clamp"))
                || i["fit"].is_null()
                || !ws.source().entries.contains_key(key)
            {
                continue;
            }
            let worse =
                |v: &Value| n(&v["fit"]["need_width"]) * 1000 / n(&v["fit"]["width"]).max(1);
            let slot = issues
                .entry((loc.to_string(), key.to_string()))
                .or_insert(i);
            if worse(i) > worse(slot) {
                *slot = i;
            }
        }
        let mapped = |loc: &str, key: &str| {
            report["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|m| m["locale"] == loc && m["key"] == key)
                .and_then(|m| m["crop"].as_str())
                .map(|c| format!("{run}/{c}"))
        };
        for (id, t) in tracks.iter_mut().filter(|(_, t)| t.fixed_at.is_none()) {
            match issues.get(id) {
                None => {
                    t.fixed_at = Some(iter - 1);
                    t.after_crop = mapped(&id.0, &id.1);
                }
                Some(i) => {
                    t.after_crop = i["crop"].as_str().map(|c| format!("{run}/{c}"));
                    if let Some(v) = t
                        .after_value
                        .as_ref()
                        .filter(|v| !t.rejected.iter().any(|r| &r.0 == *v))
                    {
                        let f = &i["fit"];
                        t.rejected.push((
                            v.clone(),
                            format!(
                                "still overflows: needs {}px, box is {}px",
                                n(&f["need_width"]),
                                n(&f["width"])
                            ),
                        ));
                    }
                }
            }
        }
        if issues.is_empty() || iter > a.max_iterations {
            break;
        }

        let mut proposals = vec![];
        for ((loc, key), i) in &issues {
            let current = ws
                .catalogs
                .get(loc)
                .and_then(|c| c.entries.get(key))
                .cloned()
                .unwrap_or_default();
            let source = ws.source().entries[key].clone();
            let t = tracks
                .entry((loc.clone(), key.clone()))
                .or_insert_with(|| Track {
                    before_value: current.clone(),
                    before_crop: i["crop"]
                        .as_str()
                        .map(|c| format!("{run}/{c}"))
                        .unwrap_or_default(),
                    ..Default::default()
                });
            let images = ["crop", "context_crop"]
                .iter()
                .filter_map(|f| i[*f].as_str())
                .map(|c| png_b64(&out.join(c)))
                .collect::<std::io::Result<Vec<_>>>()?;
            let f = &i["fit"];
            let req = Shorten {
                locale: loc.clone(),
                key: key.clone(),
                glossary: ws.glossary.matches(&source).into_iter().cloned().collect(),
                source,
                current: current.clone(),
                width_px: n(&f["width"]),
                need_width_px: n(&f["need_width"]),
                height_px: n(&f["height"]),
                need_height_px: n(&f["need_height"]),
                font: f["font"].as_str().unwrap_or_default().into(),
                rejected: t.rejected.clone(),
                images,
            };
            let c = provider.complete(req.request(&model)).await?;
            cost += glot_llm::pricing::cost_usd(&c.model, c.input_tokens, c.output_tokens);
            let reject = match parse_shortened(&c.text) {
                Some(v) if v != current => {
                    proposals.push((loc.clone(), key.clone(), current, v));
                    continue;
                }
                Some(v) => (v, "model returned the same text".to_string()),
                None => (
                    c.text.chars().take(80).collect(),
                    "unparseable answer".to_string(),
                ),
            };
            if !t.rejected.contains(&reject) {
                t.rejected.push(reject);
            }
        }
        for (loc, key, old, new) in &proposals {
            anstream::println!(
                "  {loc:<6} {BOLD}{key}{BOLD:#}  {RED}{old:?}{RED:#} → {GREEN}{new:?}{GREEN:#}"
            );
        }
        if !proposals.is_empty()
            && !a.yes
            && !crate::confirm(&format!("Write {} shorter variant(s)?", proposals.len()))?
        {
            bail!("aborted");
        }
        let mut written = 0;
        for (loc, key, _, new) in proposals {
            let t = tracks
                .get_mut(&(loc.clone(), key.clone()))
                .expect("tracked");
            let found = ws.set_translation(&loc, &key, &new)?;
            if glot_core::validate::has_errors(&found) {
                let why = found
                    .iter()
                    .map(|i| i.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                anstream::println!("  {RED}✗{RED:#} {loc} {key} rejected: {why}");
                t.rejected.push((new, why));
            } else {
                t.after_value = Some(new);
                written += 1;
            }
        }
        // Nothing changed on disk: re-rendering would show the same overflows.
        if written == 0 {
            break;
        }
        if let Some(cmd) = &a.build_cmd {
            anstream::eprintln!("{DIM}$ {cmd}{DIM:#}");
            if !Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(&ws.root)
                .status()?
                .success()
            {
                bail!("--build-cmd failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(a.reload_ms)).await;
    }

    let md = summary(&tracks, iter, cost);
    std::fs::write(dir.join("report.md"), &md)?;
    let fixed = tracks.values().filter(|t| t.fixed_at.is_some()).count();
    let (c, word) = if fixed == tracks.len() {
        (GREEN, "fits")
    } else {
        (YELLOW, "partial")
    };
    anstream::println!(
        "{c}{word}{c:#}: {fixed}/{} overflowing string(s) fixed  {DIM}${cost:.4} — {}{DIM:#}",
        tracks.len(),
        dir.join("report.md").display()
    );
    Ok(if fixed == tracks.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn summary(tracks: &BTreeMap<(String, Key), Track>, runs: u32, cost: f64) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    let mut md = format!(
        "## glot fit\n\n{} string(s), {runs} render(s), LLM cost ${cost:.4}\n\n",
        tracks.len()
    );
    md += "| Locale | Key | Before | After | Result |\n|---|---|---|---|---|\n";
    for ((loc, key), t) in tracks {
        let after = t.after_value.as_deref().unwrap_or(&t.before_value);
        let img = |p: &Option<String>| {
            p.as_ref()
                .map(|p| format!("![]({p})<br>"))
                .unwrap_or_default()
        };
        let result = match (t.fixed_at, &t.after_value) {
            (Some(i), _) => format!("✓ fits after {i} iteration(s)"),
            (None, None) => "✗ no valid variant".to_string(),
            (None, Some(_)) => "✗ still overflows".to_string(),
        };
        md += &format!(
            "| `{loc}` | `{key}` | {}`{}` | {}`{}` | {result} |\n",
            img(&Some(t.before_crop.clone())),
            cell(&t.before_value),
            img(&t.after_crop),
            cell(after)
        );
    }
    for ((loc, key), t) in tracks.iter().filter(|(_, t)| !t.rejected.is_empty()) {
        md += &format!("\n<details><summary>{loc} {key}: rejected attempts</summary>\n\n");
        for (v, why) in &t.rejected {
            md += &format!("- `{}` — {}\n", cell(v), cell(why));
        }
        md += "\n</details>\n";
    }
    md
}
