//! `glot experiments`: segmented copy experiments from the terminal (REST, `[server]` + `GLOT_TOKEN`) and their
//! **definitions** in Git (`glot.experiments.toml`: keys, intents, source variants, segments, goal — never state
//! or results). `validate` runs offline in CI (and inside `glot check`): validator, policy, pixel budgets.
//!
//! ```toml
//! [segments.de-google]
//! all = [{ attr = "country", in = ["DE", "AT", "CH"] }, { attr = "source", eq = "search" }]
//!
//! [[experiments]]
//! name = "hero-de-google"
//! prefix = "home.hero"            # or keys = ["home.hero.title", "home.hero.cta"]
//! segments = ["de-google"]
//! goal = "signup"
//! allocation = "segmented"        # fixed | bandit | segmented
//! facts = ["2,000 teams"]         # the only figures a variant may cite
//!
//! [[experiments.variants]]
//! intent = "social proof"
//! source = { "home.hero.title" = "Join 2,000 teams shipping in 12 languages" }
//! ```
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use glot_core::validate::{Constraints, Issue, IssueCode, Severity, validate};
use glot_core::{measure, policy};
use glot_llm::engine::{Finding, Workspace};

use crate::style::*;

pub const FILE: &str = "glot.experiments.toml";
/// Attributes the edge computes (design §1); `custom.*` are app-declared enums.
const ATTRS: &[&str] = &[
    "country",
    "region",
    "locale",
    "device",
    "source",
    "visitor",
    "plan",
    "logged_in",
];
const MAX_CELLS: usize = 4;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Definitions {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub segments: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub experiments: Vec<Definition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<String>,
    #[serde(default = "default_goal")]
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<String>,
    /// Events that must not go up (e.g. `refund`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guardrails: Vec<String>,
    /// `conversion` (default) | `revenue` (needs `currency`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Default: source + targets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locales: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_pct: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_n: Option<i32>,
    /// Variants wait for approval in the dashboard before they can start.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub intent: String,
    /// key -> text in the source locale (keys left out keep the official text). Empty: the server writes it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source: BTreeMap<String, String>,
}

fn default_goal() -> String {
    "signup".into()
}

pub fn load(root: &Path) -> Result<Definitions> {
    let path = root.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(t) => toml::from_str(&t).with_context(|| format!("invalid {FILE}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Definitions::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {FILE}")),
    }
}

fn save(root: &Path, d: &Definitions) -> Result<()> {
    let text = format!(
        "# glot experiment definitions (state and results live on the server: `glot experiments status`).\n{}",
        toml::to_string_pretty(d)?
    );
    std::fs::write(root.join(FILE), text).with_context(|| format!("cannot write {FILE}"))
}

/// The experiment's keys: `keys`, else every source key under `prefix.`.
pub fn keys_of(d: &Definition, ws: &Workspace) -> Vec<String> {
    let mut keys = d.keys.clone();
    if let Some(p) = &d.prefix {
        let p = format!("{}.", p.trim_end_matches('.'));
        keys.extend(
            ws.source()
                .entries
                .keys()
                .filter(|k| k.starts_with(&p))
                .cloned(),
        );
    }
    keys.sort();
    keys.dedup();
    keys
}

fn issue(code: IssueCode, message: String) -> Issue {
    Issue {
        code,
        severity: Severity::Error,
        message,
    }
}

fn check_rule(rule: &Value) -> Result<(), String> {
    let preds = rule["all"]
        .as_array()
        .filter(|a| (1..=8).contains(&a.len()))
        .ok_or("a segment is `all = [{ attr, eq | in | not_in }, …]` (1 to 8 predicates)")?;
    for p in preds {
        let attr = p["attr"].as_str().ok_or("predicate without `attr`")?;
        if !ATTRS.contains(&attr) && !attr.strip_prefix("custom.").is_some_and(|c| !c.is_empty()) {
            return Err(format!(
                "unknown attribute `{attr}` (allowed: {}, custom.*)",
                ATTRS.join(", ")
            ));
        }
        let list = |v: &Value| {
            v.as_array()
                .filter(|a| a.iter().all(Value::is_string))
                .map(Vec::len)
        };
        let n = match (&p["eq"], &p["in"], &p["not_in"]) {
            (Value::String(_), Value::Null, Value::Null) => Some(1),
            (Value::Null, l, Value::Null) | (Value::Null, Value::Null, l) => list(l),
            _ => None,
        }
        .ok_or_else(|| {
            format!(
                "`{attr}`: exactly one of `eq = \"..\"`, `in = [..]`, `not_in = [..]` (strings)"
            )
        })?;
        if n == 0 || n > 20 {
            return Err(format!("`{attr}`: 1 to 20 values"));
        }
    }
    Ok(())
}

/// Offline validation of the definitions: one finding per problem (locale = source locale, key = the
/// string, or `experiment:<name>` for structural problems).
pub fn validate_defs(ws: &Workspace, d: &Definitions) -> Vec<Finding> {
    let src = &ws.config.source_locale;
    let layout = measure::load_layout(&ws.root).unwrap_or_default();
    let policy = std::fs::read_to_string(ws.root.join(glot_core::config::CONFIG_FILE))
        .ok()
        .and_then(|t| policy::PolicyConfig::from_toml(&t).ok())
        .unwrap_or_default();
    let mut out = vec![];
    let mut push = |key: String, i: Issue| {
        out.push(Finding {
            locale: src.clone(),
            key,
            issue: i,
        })
    };
    for (name, rule) in &d.segments {
        if let Err(m) = check_rule(rule) {
            push(format!("segment:{name}"), issue(IssueCode::SyntaxError, m));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for x in &d.experiments {
        let at = format!("experiment:{}", x.name);
        let bad = |m: String| issue(IssueCode::SyntaxError, m);
        if x.name.is_empty() || !seen.insert(x.name.clone()) {
            push(at.clone(), bad("empty or duplicate experiment name".into()));
        }
        if !(1..=40).contains(&x.goal.len())
            || !x
                .goal
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
        {
            push(
                at.clone(),
                bad(format!("invalid goal event name `{}`", x.goal)),
            );
        }
        if let Some(a) = x
            .allocation
            .as_deref()
            .filter(|a| !["fixed", "bandit", "segmented"].contains(a))
        {
            push(
                at.clone(),
                bad(format!("allocation `{a}`: fixed | bandit | segmented")),
            );
        }
        match x.metric.as_deref() {
            None | Some("conversion") => {}
            Some("revenue") if x.currency.as_ref().is_some_and(|c| c.len() == 3) => {}
            Some(m) => push(
                at.clone(),
                bad(format!(
                    "metric `{m}`: conversion | revenue (with a 3-letter `currency`)"
                )),
            ),
        }
        for l in x
            .locales
            .iter()
            .filter(|l| !ws.config.all_locales().contains(l))
        {
            push(at.clone(), bad(format!("locale `{l}` is not in glot.toml")));
        }
        if x.segments.len() > MAX_CELLS {
            push(
                at.clone(),
                bad(format!("at most {MAX_CELLS} segments per experiment")),
            );
        }
        for s in x
            .segments
            .iter()
            .filter(|s| *s != "*" && !d.segments.contains_key(*s))
        {
            push(
                at.clone(),
                bad(format!("segment `{s}` is not declared under [segments]")),
            );
        }
        let keys = keys_of(x, ws);
        if keys.is_empty() {
            push(
                at.clone(),
                bad("no keys (`keys = [..]` or a `prefix` matching source keys)".into()),
            );
        }
        if keys.len() > 20 {
            push(
                at.clone(),
                bad(format!("{} keys: at most 20 per experiment", keys.len())),
            );
        }
        for k in &keys {
            if !ws.source().entries.contains_key(k) {
                push(
                    k.clone(),
                    bad(format!("{at}: key not in the source catalog")),
                );
            } else if let Some(why) = glot_llm::routing::route(
                &ws.config.qa,
                k,
                ws.lock.notes.get(k).map(String::as_str),
                100,
            ) {
                push(
                    k.clone(),
                    bad(format!("{at}: {why} keys cannot be experimented on")),
                );
            }
        }
        if x.variants.is_empty() {
            push(
                at.clone(),
                bad("at least one variant (`[[experiments.variants]]`)".into()),
            );
        }
        for v in &x.variants {
            if v.intent.trim().is_empty() {
                push(at.clone(), bad("variant without `intent`".into()));
            }
            for (k, text) in &v.source {
                let Some(official) = ws.source().entries.get(k) else {
                    continue;
                };
                if !keys.contains(k) {
                    push(
                        k.clone(),
                        bad(format!(
                            "{at}: variant text for a key outside the experiment"
                        )),
                    );
                    continue;
                }
                let ctx = |m: &str| format!("{at} [{}]: {m}", v.intent);
                for i in validate(
                    official,
                    text,
                    src,
                    ws.config.syntax,
                    &Constraints::default(),
                ) {
                    if i.code != IssueCode::Untranslated {
                        push(
                            k.clone(),
                            Issue {
                                message: ctx(&i.message),
                                ..i
                            },
                        );
                    }
                }
                for f in policy::check(text, src, official, &x.facts, &policy) {
                    let message = ctx(&format!(
                        "policy {}: {} ({:?})",
                        f.rule, f.message, f.matched
                    ));
                    push(
                        k.clone(),
                        Issue {
                            code: IssueCode::SyntaxError,
                            severity: f.severity,
                            message,
                        },
                    );
                }
                if let Some(i) = layout
                    .get(k)
                    .and_then(|f| measure::check(text, ws.config.syntax, f))
                {
                    push(
                        k.clone(),
                        Issue {
                            message: ctx(&i.message),
                            ..i
                        },
                    );
                }
            }
        }
    }
    out
}

/// `glot check` findings for `glot.experiments.toml` (none when the file is absent).
pub fn check(ws: &Workspace) -> Vec<Finding> {
    match load(&ws.root) {
        Ok(d) => validate_defs(ws, &d),
        Err(e) => vec![Finding {
            locale: ws.config.source_locale.clone(),
            key: FILE.into(),
            issue: issue(IssueCode::SyntaxError, format!("{e:#}")),
        }],
    }
}

// ---------- CLI ----------

#[derive(clap::Subcommand)]
pub enum Cmd {
    /// Experiments on the server (live first).
    List {
        #[arg(long)]
        json: bool,
    },
    /// Create a draft (variants written by the server from intents), `--start` to launch it.
    Create(CreateArgs),
    /// One experiment (state per segment, readiness), or all of them.
    Status {
        /// Id or name.
        experiment: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Pause {
        experiment: String,
        #[arg(long)]
        json: bool,
    },
    /// Resume a paused experiment, or start a draft.
    Resume {
        experiment: String,
        #[arg(long)]
        json: bool,
    },
    /// Promote a variant everywhere (official translation) or for its segment (served by the edge), then
    /// offer to test its translation in the other languages (`--propagate` does it in the same command).
    Promote(PromoteArgs),
    /// Per-segment results (P(best), lift, interval, SRM).
    Results {
        experiment: String,
        #[arg(long)]
        json: bool,
    },
    /// Upload glot.experiments.toml (segments + draft experiments) to the server.
    Push {
        #[arg(long)]
        json: bool,
    },
    /// Write the server's definitions into glot.experiments.toml (never state or results).
    Pull {
        #[arg(long)]
        json: bool,
    },
    /// Validate glot.experiments.toml offline (validator, policy, pixel budgets). Exit 1 on errors.
    Validate {
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args)]
pub struct CreateArgs {
    #[arg(long)]
    name: String,
    /// Keys (comma separated).
    #[arg(long, value_delimiter = ',')]
    keys: Vec<String>,
    /// Every key under this prefix (`home.hero`).
    #[arg(long)]
    prefix: Option<String>,
    /// Segment names (declared on the server or in glot.experiments.toml), comma separated, ≤ 4.
    #[arg(long, value_delimiter = ',')]
    segment: Vec<String>,
    /// Intents, one variant each (`--intent "social proof" --intent "direct benefit"`).
    #[arg(long)]
    intent: Vec<String>,
    #[arg(long, default_value = "signup")]
    goal: String,
    #[arg(long, value_parser = ["fixed", "bandit", "segmented"])]
    allocation: Option<String>,
    #[arg(long, value_delimiter = ',')]
    locales: Vec<String>,
    /// Facts a variant may cite (`--fact "2,000 teams"`).
    #[arg(long)]
    fact: Vec<String>,
    /// Start right away (validated server side first).
    #[arg(long)]
    start: bool,
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
pub struct PromoteArgs {
    experiment: String,
    /// Winning arm (default: the winner the server declared).
    #[arg(long)]
    arm: Option<i32>,
    /// `all`: write it as the official translation (history, lockfile on the next `glot pull`);
    /// `segment`: keep serving it to that segment only (permanent personalization).
    #[arg(long, value_parser = ["all", "segment"], default_value = "all")]
    scope: String,
    /// Segment (id or name) for `--scope segment` (default: the one where it won).
    #[arg(long)]
    cell: Option<String>,
    /// Then launch a test of its translation in the other languages against their official translation.
    #[arg(long)]
    propagate: bool,
    /// No confirmation prompt.
    #[arg(short, long)]
    yes: bool,
    #[arg(long)]
    json: bool,
}

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
            "{}/api/projects/{}/xp",
            srv.url.trim_end_matches('/'),
            srv.project
        ),
        token,
        client: reqwest::Client::new(),
    })
}

impl Remote {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.context("glot server unreachable")?;
        let status = res.status();
        let text = res.text().await?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v["error"]["message"]
                .as_str()
                .map(String::from)
                .unwrap_or(text);
            let extra = v
                .get("readiness")
                .map(|r| format!("\n{}", problems(r)))
                .unwrap_or_default();
            bail!("server returned {status}: {msg}{extra}");
        }
        Ok(v)
    }
    async fn get(&self, path: &str) -> Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }
    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }
    /// Id or name -> experiment.
    async fn find(&self, which: &str) -> Result<Value> {
        if uuid_like(which) {
            return self.get(&format!("/{which}")).await;
        }
        let all = self.get("").await?;
        all.as_array()
            .and_then(|a| a.iter().find(|x| x["name"] == which))
            .cloned()
            .with_context(|| format!("no experiment named `{which}` (glot experiments list)"))
    }
    async fn segment_ids(&self, names: &[String]) -> Result<Vec<Value>> {
        let segs = self.get("/segments").await?;
        names
            .iter()
            .map(|n| {
                segs.as_array()
                    .and_then(|a| a.iter().find(|s| s["name"] == n.as_str() || s["id"] == n.as_str()))
                    .map(|s| s["id"].clone())
                    .with_context(|| format!("unknown segment `{n}` (declare it in {FILE}, then `glot experiments push`)"))
            })
            .collect()
    }
}

fn uuid_like(s: &str) -> bool {
    s.len() == 36 && s.bytes().filter(|b| *b == b'-').count() == 4
}

fn problems(readiness: &Value) -> String {
    readiness["problems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(
            readiness["cells"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["ok"] == false)
                .filter_map(|c| c["key"].as_str()),
        )
        .map(|p| format!("  - {p}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn summary(x: &Value) -> String {
    let won: Vec<String> = x["state"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| !c["winner"].is_null())
        .map(|c| {
            format!(
                "arm {} wins in {}",
                c["winner"],
                cell_name(x, c["cell"].as_str().unwrap_or("*"))
            )
        })
        .collect();
    format!(
        "{BOLD}{}{BOLD:#} {DIM}{}{DIM:#}  {}  {} keys × {} locales, {} arms, goal {}{}",
        x["name"].as_str().unwrap_or(""),
        x["id"].as_str().unwrap_or(""),
        x["status"].as_str().unwrap_or(""),
        x["keys"].as_array().map_or(0, Vec::len),
        x["locales"].as_array().map_or(0, Vec::len),
        x["arms"].as_array().map_or(0, Vec::len),
        x["goal"].as_str().unwrap_or(""),
        if won.is_empty() {
            String::new()
        } else {
            format!("  {GREEN}{}{GREEN:#}", won.join(", "))
        }
    )
}

fn cell_name(x: &Value, id: &str) -> String {
    x["cells"]
        .as_array()
        .and_then(|c| c.iter().find(|s| s["id"] == id))
        .and_then(|s| s["name"].as_str())
        .unwrap_or(if id == "*" { "all other traffic" } else { id })
        .to_string()
}

fn out(json: bool, v: &Value, text: impl FnOnce()) -> Result<()> {
    if json {
        crate::print_json(v)
    } else {
        text();
        Ok(())
    }
}

pub async fn run(cmd: Cmd) -> Result<ExitCode> {
    let ws = crate::open()?;
    if let Cmd::Validate { json } = cmd {
        let f = check(&ws);
        let errors = f
            .iter()
            .filter(|f| f.issue.severity == Severity::Error)
            .count();
        out(json, &json!(f), || {
            for x in &f {
                crate::print_issues(&x.locale, &x.key, std::slice::from_ref(&x.issue));
            }
            if errors == 0 {
                anstream::println!("{GREEN}✓ {FILE} valid{GREEN:#}");
            }
        })?;
        return Ok(if errors > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        });
    }
    let r = remote(&ws)?;
    match cmd {
        Cmd::Validate { .. } => unreachable!(),
        Cmd::List { json } => {
            let all = r.get("").await?;
            out(json, &all, || {
                for x in all.as_array().into_iter().flatten() {
                    anstream::println!("{}", summary(x));
                }
                if all.as_array().is_none_or(Vec::is_empty) {
                    println!(
                        "no experiments: `glot experiments create` or `glot experiments push`"
                    );
                }
            })?;
        }
        Cmd::Status {
            experiment: None,
            json,
        } => return Box::pin(run(Cmd::List { json })).await,
        Cmd::Status {
            experiment: Some(w),
            json,
        } => {
            let x = r.find(&w).await?;
            let id = x["id"].as_str().unwrap_or_default().to_string();
            let readiness = if x["status"] == "draft" {
                r.get(&format!("/{id}/check")).await?
            } else {
                Value::Null
            };
            let v = json!({ "experiment": x, "readiness": readiness, "proposals": proposals(&r, &x).await });
            out(json, &v, || {
                anstream::println!("{}", summary(&x));
                for c in x["state"].as_array().into_iter().flatten() {
                    anstream::println!(
                        "  {:<22} weights {}  looks {}{}",
                        cell_name(&x, c["cell"].as_str().unwrap_or("*")),
                        c["weights"],
                        c["looks"],
                        if c["personalized"] == true {
                            "  personalized"
                        } else {
                            ""
                        }
                    );
                }
                if readiness["ready"] == false {
                    anstream::println!(
                        "{YELLOW}not ready to start:{YELLOW:#}\n{}",
                        problems(&readiness)
                    );
                }
                print_proposals(&v["proposals"]);
            })?;
        }
        Cmd::Pause {
            ref experiment,
            json,
        }
        | Cmd::Resume {
            ref experiment,
            json,
        } => {
            let x = r.find(experiment).await?;
            // `resume` on a draft starts it (same validation server side).
            let verb = match (&cmd, x["status"].as_str()) {
                (Cmd::Pause { .. }, _) => "pause",
                (_, Some("draft")) => "start",
                _ => "resume",
            };
            let x = r
                .post(
                    &format!("/{}/{verb}", x["id"].as_str().unwrap_or_default()),
                    json!({}),
                )
                .await?;
            out(json, &x, || anstream::println!("{}", summary(&x)))?;
        }
        Cmd::Results { experiment, json } => {
            let x = r.find(&experiment).await?;
            let res = r
                .get(&format!(
                    "/{}/results",
                    x["id"].as_str().unwrap_or_default()
                ))
                .await?;
            out(json, &res, || {
                anstream::println!("{}", summary(&x));
                let a = &res["latest"];
                if a.is_null() {
                    anstream::println!("{DIM}no analysis yet (not enough traffic){DIM:#}");
                }
                let ids = a["arm_ids"].as_array().cloned().unwrap_or_default();
                for c in std::iter::once(&a["global"])
                    .chain(a["cells"].as_array().into_iter().flatten())
                    .filter(|c| !c.is_null())
                {
                    anstream::println!(
                        "  {BOLD}{}{BOLD:#} {DIM}{} views{DIM:#}",
                        c["name"].as_str().unwrap_or("global"),
                        c["views"]
                    );
                    for (i, r) in c["arms"].as_array().into_iter().flatten().enumerate() {
                        let pct = |v: &Value| {
                            v.as_f64()
                                .map_or("-".into(), |f| format!("{:+.1} %", f * 100.0))
                        };
                        anstream::println!(
                            "    arm {}  P(best) {:.0} %  lift {} [{} ; {}]{}",
                            ids.get(i).cloned().unwrap_or(json!(i)),
                            r["p_best"].as_f64().unwrap_or(0.0) * 100.0,
                            pct(&r["lift"]),
                            pct(&r["lift_lo"]),
                            pct(&r["lift_hi"]),
                            if c["winner"] == i {
                                format!("  {GREEN}winner{GREEN:#}")
                            } else {
                                String::new()
                            }
                        );
                    }
                }
                if a["paused"] == true {
                    anstream::println!("{RED}SRM detected: decisions frozen{RED:#}");
                }
            })?;
        }
        Cmd::Create(a) => {
            if a.keys.is_empty() && a.prefix.is_none() {
                bail!("pass --keys or --prefix");
            }
            let mut body = json!({
                "name": a.name, "keys": a.keys, "prefix": a.prefix, "cells": r.segment_ids(&a.segment).await?,
                "goal": a.goal, "facts": a.fact, "fits": fits(&ws, &a.keys, a.prefix.as_deref()),
            });
            if let Some(al) = a.allocation {
                body["allocation"] = json!(al);
            }
            if !a.locales.is_empty() {
                body["locales"] = json!(a.locales);
            }
            let x = r.post("", body).await?;
            let id = x["id"].as_str().unwrap_or_default().to_string();
            let generated = if a.intent.is_empty() {
                Value::Null
            } else {
                r.post(&format!("/{id}/generate"), json!({ "intents": a.intent }))
                    .await?
            };
            let x = if a.start {
                r.post(&format!("/{id}/start"), json!({})).await?
            } else {
                r.get(&format!("/{id}")).await?
            };
            let v = json!({ "experiment": x, "generated": generated });
            out(a.json, &v, || {
                anstream::println!("{GREEN}created{GREEN:#} {}", summary(&x));
                for rj in generated["rejected"].as_array().into_iter().flatten() {
                    anstream::println!("  {RED}✗ rejected{RED:#} {}", rj);
                }
                if !a.start {
                    anstream::println!(
                        "{DIM}start it: glot experiments resume {}{DIM:#}",
                        x["name"].as_str().unwrap_or("")
                    );
                }
            })?;
        }
        Cmd::Promote(a) => return promote(&r, a).await,
        Cmd::Push { json } => return push(&ws, &r, json).await,
        Cmd::Pull { json } => {
            let d = pull(&ws, &r).await?;
            save(&ws.root, &d)?;
            out(json, &json!(d), || {
                anstream::println!(
                    "{GREEN}pulled{GREEN:#} {} experiments, {} segments → {FILE}",
                    d.experiments.len(),
                    d.segments.len()
                )
            })?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Pixel budgets of the experiment's keys (`.glot/layout.json`), sent so the server rejects overflowing variants.
fn fits(ws: &Workspace, keys: &[String], prefix: Option<&str>) -> Value {
    let layout = measure::load_layout(&ws.root).unwrap_or_default();
    let p = prefix.map(|p| format!("{}.", p.trim_end_matches('.')));
    json!(
        layout
            .into_iter()
            .filter(|(k, _)| keys.contains(k) || p.as_ref().is_some_and(|p| k.starts_with(p)))
            .collect::<BTreeMap<_, _>>()
    )
}

/// Cross-language proposals for an experiment (server: `GET …/xp/{id}/propagate`); empty when none or unavailable.
async fn proposals(r: &Remote, x: &Value) -> Value {
    r.get(&format!(
        "/{}/propagate",
        x["id"].as_str().unwrap_or_default()
    ))
    .await
    .map(|v| v["proposals"].clone())
    .unwrap_or(json!([]))
}

fn print_proposals(p: &Value) {
    for c in p.as_array().into_iter().flatten() {
        anstream::println!(
            "{YELLOW}→{YELLOW:#} arm {} ({}) won in {}: test it in {} → {BOLD}glot experiments promote {} --arm {} --propagate{BOLD:#}",
            c["arm"],
            c["intent"].as_str().unwrap_or(""),
            c["won_in"],
            c["locales"],
            c["experiment"].as_str().unwrap_or(""),
            c["arm"]
        );
    }
}

async fn promote(r: &Remote, a: PromoteArgs) -> Result<ExitCode> {
    let x = r.find(&a.experiment).await?;
    let id = x["id"].as_str().unwrap_or_default().to_string();
    let won: Vec<&Value> = x["state"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["winner"].as_i64().is_some())
        .collect();
    let arm = match a.arm {
        Some(n) => n,
        None => won
            .first()
            .and_then(|c| c["winner"].as_i64())
            .context("no declared winner yet: pass --arm")? as i32,
    };
    let mut body = json!({ "scope": a.scope, "arm": arm });
    if a.scope == "segment" {
        let cell = match a.cell {
            Some(c) if uuid_like(&c) || c == "*" => c,
            Some(c) => r.segment_ids(&[c]).await?[0]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            None => won
                .iter()
                .find(|c| c["winner"] == arm)
                .and_then(|c| c["cell"].as_str())
                .context("arm has not won in any segment: pass --cell")?
                .to_string(),
        };
        body["cell"] = json!(cell);
    }
    let what = if a.scope == "all" {
        format!(
            "write arm {arm} as the official translation of {} keys in every locale",
            x["keys"].as_array().map_or(0, Vec::len)
        )
    } else {
        format!(
            "serve arm {arm} permanently to segment {}",
            cell_name(&x, body["cell"].as_str().unwrap_or("*"))
        )
    };
    if !a.yes
        && !crate::confirm(&format!(
            "Promote `{}`: {what}?",
            x["name"].as_str().unwrap_or("")
        ))?
    {
        bail!("aborted");
    }
    let promoted = r.post(&format!("/{id}/promote"), body).await?;
    let propagated = if a.propagate {
        r.post(
            &format!("/{id}/propagate"),
            json!({ "arm": arm, "start": true }),
        )
        .await?
    } else {
        Value::Null
    };
    let v = json!({ "experiment": promoted, "propagated": propagated, "proposals": if a.propagate { json!([]) } else { proposals(r, &x).await } });
    out(a.json, &v, || {
        anstream::println!("{GREEN}promoted{GREEN:#} {}", summary(&promoted));
        if a.scope == "all" {
            anstream::println!("{DIM}`glot pull` brings it into the catalogs and glot.lock{DIM:#}");
        }
        if !propagated.is_null() {
            anstream::println!(
                "{GREEN}cross-language test{GREEN:#} {}",
                summary(&propagated["experiment"])
            );
            for rj in propagated["rejected"].as_array().into_iter().flatten() {
                anstream::println!(
                    "  {RED}✗ rejected{RED:#} {} {} {}",
                    rj["locale"],
                    rj["key"],
                    rj["issues"]
                );
            }
        }
        print_proposals(&v["proposals"]);
    })?;
    Ok(ExitCode::SUCCESS)
}

fn new_experiment(ws: &Workspace, d: &Definition, cells: Vec<Value>, arms: Vec<Value>) -> Value {
    let mut b = json!({
        "name": d.name, "keys": d.keys, "prefix": d.prefix, "cells": cells, "goal": d.goal, "facts": d.facts,
        "guardrails": d.guardrails, "review": d.review, "fits": fits(ws, &d.keys, d.prefix.as_deref()),
    });
    for (k, v) in [
        ("allocation", json!(d.allocation)),
        ("metric", json!(d.metric)),
        ("currency", json!(d.currency)),
        ("traffic_pct", json!(d.traffic_pct)),
        ("min_n", json!(d.min_n)),
        (
            "locales",
            if d.locales.is_empty() {
                Value::Null
            } else {
                json!(d.locales)
            },
        ),
    ] {
        if !v.is_null() {
            b[k] = v;
        }
    }
    b["arms"] = json!(arms);
    b
}

/// Arms as the server wants them: every (key, locale) filled. Keys a variant leaves out keep the official text;
/// the overridden ones are translated here by the guarded pipeline (validator, glossary, retries) from the
/// source text in Git. An unchanged variant reuses the server's values (no LLM call).
async fn arm_values(
    ws: &Workspace,
    d: &Definition,
    existing: Option<&Value>,
) -> Result<Vec<Value>> {
    let src = ws.config.source_locale.clone();
    let keys = keys_of(d, ws);
    let locales = if d.locales.is_empty() {
        ws.config.all_locales()
    } else {
        d.locales.clone()
    };
    let mut arms: Vec<Value> = vec![];
    let mut todo: Vec<(usize, BTreeMap<String, String>)> = vec![];
    for (i, v) in d.variants.iter().enumerate() {
        if v.source.is_empty() {
            arms.push(json!({ "intent": v.intent })); // written by the server (`generate`)
            continue;
        }
        let full: BTreeMap<String, String> = keys
            .iter()
            .filter_map(|k| {
                Some((
                    k.clone(),
                    v.source.get(k).or(ws.source().entries.get(k))?.clone(),
                ))
            })
            .collect();
        let old = existing.and_then(|e| {
            e["arms"]
                .as_array()?
                .iter()
                .find(|a| a["arm"] == i + 1)
                .cloned()
        });
        let reusable = old.as_ref().is_some_and(|o| {
            o["intent"] == v.intent.as_str()
                && o["values"][&src] == json!(full)
                && locales
                    .iter()
                    .all(|l| keys.iter().all(|k| o["values"][l][k].is_string()))
        });
        match old.filter(|_| reusable) {
            Some(o) => arms.push(json!({ "intent": v.intent, "values": o["values"] })),
            None => {
                arms.push(json!({ "intent": v.intent, "values": { src.clone(): full } }));
                todo.push((arms.len() - 1, v.source.clone()));
            }
        }
    }
    if todo.is_empty() {
        return Ok(arms);
    }
    let translator = glot_llm::translate::Translator::new(
        &ws.config,
        glot_llm::provider::from_config(&ws.config.llm, None)?,
    );
    for l in locales.iter().filter(|l| **l != src) {
        let official = ws.catalogs.get(l);
        let items: Vec<glot_llm::translate::Item> = todo
            .iter()
            .flat_map(|(i, over)| {
                over.iter().map(move |(k, text)| glot_llm::translate::Item {
                    key: format!("{i}|{k}"),
                    source: text.clone(),
                    context: glot_core::context::KeyContext {
                        key: k.clone(),
                        description: Some(format!(
                            "Marketing variant ({}) of `{k}` in an A/B test. Keep its persuasion intent and length; add no claim, number, deadline or superlative.",
                            d.variants.get(*i).map_or("", |v| v.intent.as_str())
                        )),
                        ..Default::default()
                    },
                    previous: None,
                })
            })
            .collect();
        let text: Vec<&str> = items.iter().map(|i| i.source.as_str()).collect();
        let req = glot_llm::translate::Request {
            source_locale: src.clone(),
            target_locale: l.clone(),
            glossary: ws
                .glossary
                .matches(&text.join("\n"))
                .into_iter()
                .cloned()
                .collect(),
            items,
            syntax: ws.config.syntax,
            style: ws.config.llm.style.clone(),
        };
        let (done, _) = translator.translate(req).await?;
        for t in done {
            if glot_core::validate::has_errors(&t.issues) {
                let why: Vec<&str> = t.issues.iter().map(|i| i.message.as_str()).collect();
                bail!(
                    "{}: variant {} could not be translated into {l}: {}",
                    d.name,
                    t.key,
                    why.join("; ")
                );
            }
            let Some((i, k)) = t
                .key
                .split_once('|')
                .and_then(|(i, k)| Some((i.parse::<usize>().ok()?, k)))
            else {
                continue;
            };
            arms[i]["values"][l][k] = json!(t.value);
        }
        // Keys the variant leaves out: the official translation.
        for (i, over) in &todo {
            for k in keys.iter().filter(|k| !over.contains_key(*k)) {
                if let Some(v) = official
                    .and_then(|c| c.entries.get(k))
                    .filter(|v| !v.is_empty())
                {
                    arms[*i]["values"][l][k] = json!(v);
                }
            }
        }
    }
    Ok(arms)
}

async fn push(ws: &Workspace, r: &Remote, json_out: bool) -> Result<ExitCode> {
    let d = load(&ws.root)?;
    let findings = validate_defs(ws, &d);
    if findings.iter().any(|f| f.issue.severity == Severity::Error) {
        for f in &findings {
            crate::print_issues(&f.locale, &f.key, std::slice::from_ref(&f.issue));
        }
        bail!("{FILE} is invalid: nothing pushed (glot experiments validate)");
    }
    let mut report = vec![];
    // Segments by name: create the missing ones, update changed rules.
    let segs = r.get("/segments").await?;
    for (name, rule) in &d.segments {
        let existing = segs
            .as_array()
            .and_then(|a| a.iter().find(|s| s["name"] == name.as_str()));
        let rule = serde_json::to_value(rule)?;
        let action = match existing {
            None => r
                .post("/segments", json!({ "name": name, "rule": rule }))
                .await
                .map(|_| "created"),
            Some(s) if s["rule"] != rule => r
                .call(
                    reqwest::Method::PATCH,
                    &format!("/segments/{}", s["id"].as_str().unwrap_or_default()),
                    Some(json!({ "rule": rule })),
                )
                .await
                .map(|_| "updated"),
            Some(_) => Ok("unchanged"),
        };
        report.push(json!({ "segment": name, "action": action.unwrap_or_else(|e| { anstream::eprintln!("{YELLOW}warning{YELLOW:#} segment {name}: {e:#}"); "failed" }) }));
    }
    let remote_all = r.get("").await?;
    for x in &d.experiments {
        let cells = r
            .segment_ids(
                &x.segments
                    .iter()
                    .filter(|s| *s != "*")
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .await?;
        let existing = remote_all
            .as_array()
            .and_then(|a| a.iter().find(|e| e["name"] == x.name.as_str()));
        let frozen = existing.is_some_and(|e| e["status"] != "draft");
        let arms = if frozen {
            vec![]
        } else {
            arm_values(ws, x, existing).await?
        };
        let body = new_experiment(ws, x, cells, arms);
        let action = match existing {
            None => {
                r.post("", body).await?;
                "created"
            }
            Some(e) if e["status"] == "draft" => {
                let id = e["id"].as_str().unwrap_or_default();
                let mut patch = body.clone();
                let arms = patch
                    .as_object_mut()
                    .and_then(|o| o.remove("arms"))
                    .unwrap_or(json!([]));
                r.call(reqwest::Method::PATCH, &format!("/{id}"), Some(patch))
                    .await?;
                let local = arms.as_array().cloned().unwrap_or_default();
                for (i, arm) in local.iter().enumerate() {
                    let n = i + 1;
                    let mut values = arm["values"].as_object().cloned().unwrap_or_default();
                    // Cells the server has but the file no longer produces: "" deletes them.
                    if let Some(old) = e["arms"]
                        .as_array()
                        .and_then(|a| a.iter().find(|a| a["arm"] == n))
                    {
                        for (l, m) in old["values"].as_object().into_iter().flatten() {
                            for k in m.as_object().into_iter().flatten().map(|(k, _)| k) {
                                let entry = values.entry(l.clone()).or_insert(json!({}));
                                if entry.get(k).is_none() {
                                    entry[k] = json!("");
                                }
                            }
                        }
                    }
                    r.call(
                        reqwest::Method::PUT,
                        &format!("/{id}/arms/{n}"),
                        Some(json!({ "intent": arm["intent"], "values": values })),
                    )
                    .await?;
                }
                for old in e["arms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|a| a["arm"].as_u64().is_some_and(|n| n as usize > local.len()))
                {
                    r.call(
                        reqwest::Method::DELETE,
                        &format!("/{id}/arms/{}", old["arm"]),
                        None,
                    )
                    .await?;
                }
                "updated"
            }
            Some(e) => {
                anstream::eprintln!(
                    "{YELLOW}skipped{YELLOW:#} {}: {} on the server, definitions are frozen once started (stop it, or rename to start a new one)",
                    x.name,
                    e["status"].as_str().unwrap_or("")
                );
                "frozen"
            }
        };
        report.push(json!({ "experiment": x.name, "action": action }));
    }
    let local: Vec<&str> = d.experiments.iter().map(|x| x.name.as_str()).collect();
    let server_only: Vec<&str> = remote_all
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["name"].as_str())
        .filter(|n| !local.contains(n))
        .collect();
    let v = json!({ "pushed": report, "server_only": server_only });
    out(json_out, &v, || {
        for x in &report {
            let what = x["experiment"]
                .as_str()
                .map(|n| format!("experiment {n}"))
                .unwrap_or_else(|| format!("segment {}", x["segment"].as_str().unwrap_or("")));
            anstream::println!(
                "{GREEN}{}{GREEN:#} {what}",
                x["action"].as_str().unwrap_or("")
            );
        }
        if !server_only.is_empty() {
            anstream::println!(
                "{DIM}on the server only: {} (glot experiments pull){DIM:#}",
                server_only.join(", ")
            );
        }
    })?;
    Ok(ExitCode::SUCCESS)
}

/// Server experiments -> definitions (source-locale texts of each variant; state, weights and counts dropped).
async fn pull(ws: &Workspace, r: &Remote) -> Result<Definitions> {
    let src = ws.config.source_locale.clone();
    let mut d = Definitions::default();
    for s in r.get("/segments").await?.as_array().into_iter().flatten() {
        if let Some(n) = s["name"].as_str() {
            d.segments.insert(n.into(), s["rule"].clone());
        }
    }
    for e in r
        .get("")
        .await?
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["kind"] != "holdout")
    {
        let strs = |v: &Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        };
        let all_locales = {
            let mut l = ws.config.all_locales();
            l.sort();
            l
        };
        let mut locales = strs(&e["locales"]);
        locales.sort();
        d.experiments.push(Definition {
            name: e["name"].as_str().unwrap_or_default().into(),
            keys: e["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| k["key"].as_str().map(String::from))
                .collect(),
            prefix: None,
            segments: e["cells"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["name"].as_str().map(String::from))
                .collect(),
            goal: e["goal"].as_str().unwrap_or("signup").into(),
            allocation: e["allocation"]
                .as_str()
                .filter(|a| *a != "bandit")
                .map(String::from),
            facts: strs(&e["facts"]),
            guardrails: strs(&e["guardrails"]),
            metric: e["metric"]
                .as_str()
                .filter(|m| *m != "conversion")
                .map(String::from),
            currency: e["currency"].as_str().map(String::from),
            locales: if locales == all_locales {
                vec![]
            } else {
                locales
            },
            traffic_pct: e["traffic_pct"]
                .as_i64()
                .filter(|n| *n != 100)
                .map(|n| n as i32),
            min_n: e["min_n"].as_i64().filter(|n| *n != 500).map(|n| n as i32),
            review: e["review"] == true,
            variants: e["arms"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|a| a["arm"].as_i64().is_some_and(|n| n >= 1))
                .map(|a| Variant {
                    intent: a["intent"].as_str().unwrap_or_default().into(),
                    source: a["values"][&src]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                        .collect(),
                })
                .collect(),
        });
    }
    Ok(d)
}

/// `glot pull`: the per-segment personalizations promoted on the server, as `.glot/personalizations.json`
/// (the same file the promotion PR writes). Best effort: a server without experiments is not an error.
pub async fn pull_personalizations(ws: &Workspace) -> Result<()> {
    let r = remote(ws)?;
    let Ok(v) = r.get("/personalizations").await else {
        return Ok(());
    };
    let path = ws.root.join(".glot/personalizations.json");
    if v["personalizations"].as_array().is_none_or(Vec::is_empty) && !path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(ws.root.join(".glot"))?;
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&v)?))?;
    anstream::println!("{BOLD}.glot/personalizations.json{BOLD:#} updated");
    Ok(())
}
