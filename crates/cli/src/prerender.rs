//! `glot prerender`: snapshots of a 100 % JavaScript app rendered in the CI (headless shell of `packages/renderer`,
//! run as `glot-renderer prerender`), optionally uploaded to glot Edge (`POST /edge/v1/{project}/snapshots`) so
//! the glot server needs no browser. Edge translates them at serve time like the renderer service's snapshots.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};

use crate::style::*;

#[derive(clap::Args)]
pub struct PrerenderArgs {
    /// The built app, served by the CI (e.g. `vite preview`, `npx serve dist`).
    #[arg(long)]
    pub base_url: String,
    /// Routes to render (comma separated): paths on the origin, e.g. `/,/pricing`.
    #[arg(long, value_delimiter = ',', required = true)]
    pub routes: Vec<String>,
    /// Output directory: `<route>/index.html` + `manifest.json`.
    #[arg(long, default_value = ".glot/prerender")]
    pub out: PathBuf,
    /// Apps that append to `<body>` instead of replacing a root (Leptos `mount_to_body`, Svelte `target: body`).
    #[arg(long)]
    pub append: bool,
    /// Upload the snapshots to glot Edge (`[server]` of glot.toml, or `--server` + `--project`; token in GLOT_TOKEN).
    #[arg(long)]
    pub upload: bool,
    #[arg(long)]
    pub server: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    snapshots: Vec<Snap>,
    #[serde(default)]
    failed: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Snap {
    path: String,
    file: String,
    #[serde(rename = "renderedAt")]
    rendered_at: i64,
}

/// Requests stay well under the server's 64 MiB body limit.
const BATCH_BYTES: usize = 16 * 1024 * 1024;

pub async fn run(a: PrerenderArgs) -> Result<ExitCode> {
    let mut c = crate::fit::node_tool("renderer");
    c.arg("prerender")
        .arg("--base-url")
        .arg(&a.base_url)
        .arg("--routes")
        .arg(a.routes.join(","))
        .arg("--out")
        .arg(&a.out);
    if a.append {
        c.args(["--mount", "append"]);
    }
    let out = c
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| anyhow::anyhow!(crate::fit::tool_missing("renderer")))?;
    let manifest = std::fs::read_to_string(a.out.join("manifest.json"))
        .ok()
        .and_then(|m| serde_json::from_str::<Manifest>(&m).ok());
    let Some(m) = manifest else {
        return Err(crate::fit::tool_failed("renderer", &out.stderr));
    };
    anstream::eprint!("{}", String::from_utf8_lossy(&out.stderr));
    anstream::println!(
        "{GREEN}rendered{GREEN:#} {} page(s) into {}{}",
        m.snapshots.len(),
        a.out.display(),
        if m.failed.is_empty() {
            String::new()
        } else {
            format!(", {RED}{} failed{RED:#}", m.failed.len())
        }
    );
    if a.upload && !m.snapshots.is_empty() {
        upload(&a, &m).await?;
    }
    Ok(if m.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

async fn upload(a: &PrerenderArgs, m: &Manifest) -> Result<()> {
    let srv = crate::open().ok().and_then(|ws| ws.config.server);
    let (url, project, token_env) = match (&a.server, &a.project, &srv) {
        (Some(u), Some(p), _) => (u.clone(), p.clone(), "GLOT_TOKEN".to_string()),
        (_, _, Some(s)) => (
            a.server.clone().unwrap_or_else(|| s.url.clone()),
            a.project.clone().unwrap_or_else(|| s.project.clone()),
            s.token_env.clone(),
        ),
        _ => bail!("--upload needs [server] in glot.toml, or --server and --project"),
    };
    let token = std::env::var(&token_env)
        .with_context(|| format!("set {token_env} to an API token (see the dashboard)"))?;
    let endpoint = format!("{}/edge/v1/{project}/snapshots", url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let (mut imported, mut batch, mut size) = (0u64, Vec::new(), 0usize);
    for (i, s) in m.snapshots.iter().enumerate() {
        let html = std::fs::read_to_string(a.out.join(&s.file))
            .with_context(|| format!("reading {}", s.file))?;
        size += html.len();
        batch.push(serde_json::json!({"path": s.path, "html": html, "rendered_at": s.rendered_at}));
        if size >= BATCH_BYTES || batch.len() == 100 || i + 1 == m.snapshots.len() {
            let res = client
                .post(&endpoint)
                .bearer_auth(&token)
                .json(&serde_json::json!({ "snapshots": batch }))
                .send()
                .await
                .context("upload failed")?;
            let status = res.status();
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            if !status.is_success() {
                bail!(
                    "server returned {status}: {}",
                    body["error"]["message"].as_str().unwrap_or("")
                );
            }
            imported += body["imported"].as_u64().unwrap_or(0);
            for r in body["rejected"].as_array().into_iter().flatten() {
                anstream::eprintln!(
                    "  {YELLOW}rejected{YELLOW:#} {} {}",
                    r["path"].as_str().unwrap_or(""),
                    r["error"].as_str().unwrap_or("")
                );
            }
            (batch, size) = (Vec::new(), 0);
        }
    }
    anstream::println!("{GREEN}uploaded{GREEN:#} {imported} snapshot(s) to {endpoint}");
    Ok(())
}
