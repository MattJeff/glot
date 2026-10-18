//! `glot annotations`: the live review outside the SaaS (docs/design/live-review-contract.md). Talks to the glot
//! server (`[server]` + `GLOT_TOKEN`) through `glot_mcp::annotations`, the same client the MCP tools use.
use anyhow::{Result, anyhow};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::ExitCode;

use glot_core::validate::{Issue, IssueCode, Severity};
use glot_llm::engine::{Finding, Workspace};
use glot_mcp::annotations::{ACTIVE, Filter, ReviewApi, agent_view, code_refs};

use crate::style::*;

#[derive(Args)]
pub struct AnnotationsArgs {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone, Default)]
struct FilterArgs {
    /// Page route without locale prefix (`/pricing`).
    #[arg(long)]
    route: Option<String>,
    #[arg(long)]
    locale: Option<String>,
    /// Comma separated: open, in_progress, resolved, wontfix (default open,in_progress; `all` for every status).
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    key: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Annotations reviewers pinned on the site (default: open + in progress).
    List(FilterArgs),
    /// One annotation with its thread, code usages and screenshots.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Annotate a key from the terminal.
    Add {
        /// The comment.
        body: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        locale: String,
        /// Page route where the text shows (default `/`).
        #[arg(long, default_value = "/")]
        route: String,
        /// Proposed replacement text.
        #[arg(long)]
        suggest: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Reply in the thread.
    Reply { id: String, body: String },
    /// Mark resolved (`-m`: say what you changed, posted in the thread first).
    Resolve {
        id: String,
        #[arg(short, long)]
        message: Option<String>,
        /// Close as wontfix instead.
        #[arg(long)]
        wontfix: bool,
    },
    /// Reopen.
    Reopen {
        id: String,
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Write the suggestion (or `--value`) as the validated translation, locally and on the server; resolves it.
    Apply {
        id: String,
        #[arg(long)]
        value: Option<String>,
    },
    /// Ask the project LLM for rewordings (every locale, validated); `--pick N` writes proposal N.
    Rewrite {
        id: String,
        #[arg(long)]
        instruction: Option<String>,
        /// shorter | clearer | formal | informal | warmer | direct | no_jargon | brand_tone
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        pick: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    /// Create a shareable review link for a reviewer without an account.
    Link {
        /// Page of the site to open (the link is `<url>?glot_review=<token>`).
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value = "comment", value_parser = ["read", "comment", "edit"])]
        access: String,
        #[arg(long)]
        label: Option<String>,
        /// Days before it expires (0 = never).
        #[arg(long, default_value_t = 14)]
        days: u32,
        #[arg(long)]
        json: bool,
    },
    /// Mirror an annotation to GitHub: an issue in the linked repo, or `--pr` a comment on the open glot PR.
    Github {
        id: String,
        #[arg(long)]
        pr: bool,
    },
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn status_color(s: &str) -> anstyle::Style {
    match s {
        "open" => YELLOW,
        "in_progress" => BOLD,
        "resolved" => GREEN,
        _ => DIM,
    }
}

fn line(a: &Value) {
    let s = a["status"].as_str().unwrap_or_default();
    let c = status_color(s);
    anstream::println!(
        "{BOLD}{}{BOLD:#} {c}{s:<11}{c:#} {:<5} {:<14} {}  {DIM}{} · {}{DIM:#}",
        short(a["id"].as_str().unwrap_or_default()),
        a["locale"].as_str().unwrap_or_default(),
        a["route"].as_str().unwrap_or_default(),
        a["key"].as_str().unwrap_or("(no key)"),
        a["author"]["name"].as_str().unwrap_or_default(),
        a["comment_count"],
    );
    anstream::println!("         {}", a["body"].as_str().unwrap_or_default());
    if let Some(sg) = a["suggestion"].as_str() {
        anstream::println!("         {GREEN}→ {sg}{GREEN:#}");
    }
}

pub async fn run(a: AnnotationsArgs) -> Result<ExitCode> {
    let mut ws = crate::open()?;
    let api = ReviewApi::from_workspace(&ws).map_err(|e| anyhow!(e))?;
    let id = |i: &str| {
        let api = &api;
        let i = i.to_string();
        async move { api.resolve_id(&i).await.map_err(|e| anyhow!(e)) }
    };
    let e = |e: String| anyhow!(e);
    match a.cmd {
        Cmd::List(f) => {
            let status = f.status.map(|s| {
                if s == "all" {
                    "open,in_progress,resolved,wontfix".into()
                } else {
                    s
                }
            });
            let filter = Filter {
                status,
                route: f.route,
                locale: f.locale,
                key: f.key,
                limit: Some(200),
            };
            let page = api.list(&filter).await.map_err(e)?;
            let items = page["items"].as_array().cloned().unwrap_or_default();
            if f.json {
                let refs = code_refs(&ws);
                let v: Vec<Value> = items
                    .iter()
                    .map(|a| agent_view(&api, a, Some(&ws), &refs))
                    .collect();
                crate::print_json(&json!({ "total": page["total"], "annotations": v }))?;
            } else if items.is_empty() {
                anstream::println!("{GREEN}No annotations.{GREEN:#}");
            } else {
                items.iter().for_each(line);
                anstream::println!("{DIM}{} total{DIM:#}", page["total"]);
            }
        }
        Cmd::Show { id: i, json } => {
            let i = id(&i).await?;
            let a = api
                .get(&format!("/annotations/{i}"), &[])
                .await
                .map_err(e)?;
            let v = agent_view(&api, &a, Some(&ws), &code_refs(&ws));
            if json {
                crate::print_json(&v)?;
            } else {
                line(&a);
                for (label, k) in [
                    ("url", "url"),
                    ("text", "rendered_text"),
                    ("source", "source"),
                    ("server", "server_value"),
                    ("local", "local_value"),
                    ("catalog", "catalog_file"),
                ] {
                    if let Some(s) = v[k].as_str() {
                        anstream::println!("  {DIM}{label:<8}{DIM:#} {s}");
                    }
                }
                for r in v["used_in"].as_array().into_iter().flatten() {
                    anstream::println!("  {DIM}used in {DIM:#} {}", r.as_str().unwrap_or_default());
                }
                for s in v["screenshots"].as_array().into_iter().flatten() {
                    anstream::println!("  {DIM}capture {DIM:#} {}", s.as_str().unwrap_or_default());
                }
                for c in a["comments"].as_array().into_iter().flatten() {
                    anstream::println!(
                        "  {BOLD}{}{BOLD:#}: {}",
                        c["author"]["name"].as_str().unwrap_or_default(),
                        c["body"].as_str().unwrap_or_default()
                    );
                }
            }
        }
        Cmd::Add {
            body,
            key,
            locale,
            route,
            suggest,
            json,
        } => {
            let text = ws
                .catalogs
                .get(locale.as_str())
                .and_then(|c| c.entries.get(key.as_str()))
                .cloned()
                .unwrap_or_else(|| key.clone());
            // ponytail: no browser here, so the anchor is the key itself; the widget re-anchors on [data-glot-key].
            let anchor = json!({
                "key": key, "key_source": "manual", "selector": format!("[data-glot-key=\"{key}\"]"),
                "text": text, "url": format!("glot://cli/{locale}{}", if route == "/" { "" } else { &route }),
                "locale": locale, "viewport": { "w": 0, "h": 0 },
            });
            let a = api
                .post(
                    "/annotations",
                    json!({ "anchor": anchor, "body": body, "suggestion": suggest }),
                )
                .await
                .map_err(e)?;
            if json {
                crate::print_json(&a)?;
            } else {
                anstream::print!("{GREEN}added{GREEN:#} ");
                line(&a);
            }
        }
        Cmd::Reply { id: i, body } => {
            let i = id(&i).await?;
            api.post(
                &format!("/annotations/{i}/comments"),
                json!({ "body": body }),
            )
            .await
            .map_err(e)?;
            anstream::println!("{GREEN}replied{GREEN:#} {}", short(&i));
        }
        Cmd::Resolve {
            id: i,
            message,
            wontfix,
        } => {
            let i = id(&i).await?;
            let s = if wontfix { "wontfix" } else { "resolved" };
            api.set_status(&i, s, message.as_deref()).await.map_err(e)?;
            anstream::println!("{GREEN}{s}{GREEN:#} {}", short(&i));
        }
        Cmd::Reopen { id: i, message } => {
            let i = id(&i).await?;
            api.set_status(&i, "open", message.as_deref())
                .await
                .map_err(e)?;
            anstream::println!("{YELLOW}reopened{YELLOW:#} {}", short(&i));
        }
        Cmd::Apply { id: i, value } => {
            let i = id(&i).await?;
            let a = api.apply(Some(&mut ws), &i, value).await.map_err(e)?;
            anstream::println!(
                "{GREEN}applied{GREEN:#} {} {} = {:?}{}",
                a["locale"].as_str().unwrap_or_default(),
                a["key"].as_str().unwrap_or_default(),
                a["translation"]["value"].as_str().unwrap_or_default(),
                if a["local"].is_null() {
                    "  (server only)"
                } else {
                    "  (local catalog + server)"
                }
            );
        }
        Cmd::Rewrite {
            id: i,
            instruction,
            preset,
            pick,
            json,
        } => {
            let i = id(&i).await?;
            let r = api
                .rewrite(&i, instruction, preset, pick)
                .await
                .map_err(e)?;
            if json {
                crate::print_json(&r)?;
            } else if let Some(n) = pick {
                anstream::println!(
                    "{GREEN}written{GREEN:#} proposal {n} in every locale  {DIM}glot pull, then glot annotations resolve {}{DIM:#}",
                    short(&i)
                );
            } else {
                for p in r["proposals"].as_array().into_iter().flatten() {
                    let c = if p["valid"] == true { GREEN } else { RED };
                    anstream::println!(
                        "{c}[{}]{c:#} {}\n    {DIM}{}{DIM:#}",
                        p["id"],
                        p["text"].as_str().unwrap_or_default(),
                        p["explanation"].as_str().unwrap_or_default()
                    );
                }
                anstream::println!(
                    "{DIM}write one: glot annotations rewrite {} --pick <n> (same options){DIM:#}",
                    short(&i)
                );
            }
        }
        Cmd::Link {
            url,
            access,
            label,
            days,
            json,
        } => {
            let r = api
                .post(
                    "/links",
                    json!({ "access": access, "label": label,
                            "expires_in_days": if days == 0 { Value::Null } else { json!(days) } }),
                )
                .await
                .map_err(e)?;
            let token = r["token"].as_str().unwrap_or_default();
            let link = url.map(|u| {
                let sep = if u.contains('?') { '&' } else { '?' };
                format!("{u}{sep}glot_review={token}")
            });
            if json {
                crate::print_json(&json!({ "token": token, "url": link, "link": r["link"] }))?;
            } else {
                match link {
                    Some(l) => anstream::println!("{GREEN}review link{GREEN:#} {l}"),
                    None => anstream::println!(
                        "{GREEN}review token{GREEN:#} {token}\n{DIM}send <your site URL>?glot_review={token} (the page must load @glot/review){DIM:#}"
                    ),
                }
                anstream::println!(
                    "{DIM}access {access}, expires {}{DIM:#}",
                    r["link"]["expires_at"].as_str().unwrap_or("never")
                );
            }
        }
        Cmd::Github { id: i, pr } => {
            let i = id(&i).await?;
            let r = api
                .post(
                    &format!("/annotations/{i}/github"),
                    json!({ "target": if pr { "pr_comment" } else { "issue" } }),
                )
                .await
                .map_err(e)?;
            anstream::println!(
                "{GREEN}mirrored{GREEN:#} {}",
                r["url"].as_str().unwrap_or_default()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `glot check`: open annotations on keys whose local value differs from the server's (you changed them: resolve
/// the annotation if that was the fix). Silent without `[server]`/token; a server error is one warning, never fatal.
pub fn check(ws: &Workspace) -> Vec<Finding> {
    let Ok(api) = ReviewApi::from_workspace(ws) else {
        return vec![];
    };
    let page = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(api.list(&Filter {
            status: Some(ACTIVE.into()),
            limit: Some(200),
            ..Default::default()
        }))
    });
    let page = match page {
        Ok(p) => p,
        Err(err) => {
            anstream::eprintln!("{YELLOW}warning:{YELLOW:#} annotations not checked: {err}");
            return vec![];
        }
    };
    let mut by_key: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for a in page["items"].as_array().into_iter().flatten() {
        let (Some(k), Some(l)) = (a["key"].as_str(), a["locale"].as_str()) else {
            continue;
        };
        let local = ws.catalogs.get(l).and_then(|c| c.entries.get(k));
        if local.is_some_and(|v| Some(v.as_str()) != a["translation"]["value"].as_str()) {
            by_key
                .entry((l.into(), k.into()))
                .or_default()
                .push(short(a["id"].as_str().unwrap_or_default()).into());
        }
    }
    by_key
        .into_iter()
        .map(|((locale, key), ids)| Finding {
            locale: locale.as_str().into(),
            key: key.as_str().into(),
            issue: Issue {
                code: IssueCode::OpenAnnotation,
                severity: Severity::Warning,
                message: format!(
                    "changed while open review annotation(s) {} point at it: if this fixes them, `glot annotations resolve <id> -m \"…\"`",
                    ids.join(", ")
                ),
            },
        })
        .collect()
}
