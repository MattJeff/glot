//! `glot push` / `glot pull` against `/api/projects/{slug}/sync/*`.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::ExitCode;

use glot_core::{
    Key, Locale,
    catalog::Catalog,
    glossary::{Glossary, Term},
    lock::Lockfile,
};
use glot_llm::engine::Workspace;

use crate::style::{BOLD, GREEN, YELLOW};

struct Remote {
    base: String,
    token: String,
    client: reqwest::Client,
}

fn remote(ws: &Workspace) -> Result<Remote> {
    let srv = ws
        .config
        .server
        .as_ref()
        .context("no [server] section in glot.toml")?;
    let token = std::env::var(&srv.token_env)
        .with_context(|| format!("set {} to an API token (see the dashboard)", srv.token_env))?;
    Ok(Remote {
        base: format!(
            "{}/api/projects/{}/sync",
            srv.url.trim_end_matches('/'),
            srv.project
        ),
        token,
        client: reqwest::Client::new(),
    })
}

async fn parse<T: serde::de::DeserializeOwned>(res: reqwest::Response) -> Result<T> {
    let status = res.status();
    let body = res.text().await?;
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(body);
        bail!("server returned {status}: {msg}");
    }
    serde_json::from_str(&body).context("invalid server response")
}

#[derive(Deserialize)]
struct PushResponse {
    created: usize,
    updated: usize,
    #[serde(default)]
    conflicts: Vec<Conflict>,
}

#[derive(Deserialize)]
struct Conflict {
    locale: Locale,
    key: Key,
    server: Option<String>,
    local: Option<String>,
}

pub async fn push() -> Result<ExitCode> {
    let ws = crate::open()?;
    let r = remote(&ws)?;
    let catalogs: BTreeMap<&Locale, &BTreeMap<Key, String>> =
        ws.catalogs.iter().map(|(l, c)| (l, &c.entries)).collect();
    let context = push_context(&ws);
    let body = serde_json::json!({
        "catalogs": catalogs, "lock": ws.lock, "glossary": ws.glossary.terms, "context": context,
    });
    let res = r
        .client
        .post(format!("{}/push", r.base))
        .bearer_auth(&r.token)
        .json(&body)
        .send()
        .await
        .context("push failed")?;
    let out: PushResponse = parse(res).await?;
    anstream::println!(
        "{GREEN}pushed{GREEN:#} {} created, {} updated",
        out.created,
        out.updated
    );
    for c in &out.conflicts {
        anstream::println!(
            "  {YELLOW}conflict{YELLOW:#} {} {}  server={:?} local={:?}",
            c.locale,
            c.key,
            c.server.as_deref().unwrap_or(""),
            c.local.as_deref().unwrap_or("")
        );
    }
    Ok(if out.conflicts.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Code context (refs, ui_role, component, note) per source key, sent so server-side jobs translate with it.
fn push_context(ws: &Workspace) -> BTreeMap<Key, serde_json::Value> {
    let report = match ws.extract() {
        Ok(r) => r,
        Err(e) => {
            anstream::eprintln!(
                "{YELLOW}warning{YELLOW:#} extraction failed, pushing without code context: {e}"
            );
            return BTreeMap::new();
        }
    };
    let source = ws.source();
    ws.contexts(&report)
        .into_iter()
        .filter(|(k, _)| source.entries.contains_key(k))
        .map(|(k, c)| {
            let v = serde_json::json!({
                "refs": c.refs, "ui_role": c.ui_role, "component": c.component, "description": c.description,
            });
            (k, v)
        })
        .collect()
}

#[derive(Deserialize)]
struct PullResponse {
    catalogs: BTreeMap<Locale, BTreeMap<Key, String>>,
    lock: Lockfile,
    #[serde(default)]
    glossary: Vec<Term>,
}

pub async fn pull() -> Result<ExitCode> {
    let ws = crate::open()?;
    let r = remote(&ws)?;
    let res = r
        .client
        .get(format!("{}/pull", r.base))
        .bearer_auth(&r.token)
        .send()
        .await
        .context("pull failed")?;
    let out: PullResponse = parse(res).await?;
    for (locale, entries) in out.catalogs {
        let paths = ws.config.catalog_paths(&ws.root, &locale);
        // ponytail: single file per locale; splitting keys back into namespaces needs a key->file map.
        let [path] = paths.as_slice() else {
            bail!(
                "pull supports exactly one catalog file per locale ({locale}: {})",
                paths.len()
            )
        };
        Catalog {
            locale: locale.clone(),
            entries,
        }
        .write(path, ws.config.format)?;
        anstream::println!(
            "{GREEN}pulled{GREEN:#} {}",
            path.strip_prefix(&ws.root).unwrap_or(path).display()
        );
    }
    out.lock.save(&ws.root)?;
    Glossary {
        terms: out.glossary,
    }
    .save(&ws.root.join(glot_core::glossary::GLOSSARY_FILE))?;
    anstream::println!("{BOLD}glot.lock{BOLD:#} and {BOLD}glossary.toml{BOLD:#} updated");
    Ok(ExitCode::SUCCESS)
}
