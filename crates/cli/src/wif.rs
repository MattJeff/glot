//! `glot wif`: set up Anthropic Workload Identity Federation (keyless Claude) from the terminal.
//! Issuer + rule creation through the Admin API (org:admin OAuth token), and a local JWT check.
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use glot_llm::wif::{
    self, DEFAULT_AUDIENCE, Expect, GITHUB_ISSUER, IssuerSpec, Jwks, RuleSpec, github_match,
};

use crate::style::*;

#[derive(Subcommand)]
pub enum WifCmd {
    /// Federation issuers (the OIDC identity provider Anthropic trusts).
    #[command(subcommand)]
    Issuer(IssuerCmd),
    /// Federation rules (which JWTs may act as which service account).
    #[command(subcommand)]
    Rule(RuleCmd),
    /// Decode a JWT locally (never sent) and compare it with an issuer + rule: iss, aud, sub, claims,
    /// exp/iat/lifetime, kid, alg. Exit 1 when a check fails.
    Check(CheckArgs),
}

#[derive(Subcommand)]
pub enum IssuerCmd {
    /// Register an issuer (POST /v1/organizations/federation_issuers).
    Create(IssuerArgs),
}

#[derive(Subcommand)]
pub enum RuleCmd {
    /// Create a rule (POST /v1/organizations/federation_rules).
    Create(RuleArgs),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Preset {
    /// GitHub Actions: https://token.actions.githubusercontent.com, discovery, repo-pinned subject.
    Github,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
pub enum JwksMode {
    /// OIDC discovery (GitHub Actions, EKS, GKE, Entra ID…).
    Discovery,
    /// Fixed JWKS URL (internal discovery, public JWKS).
    ExplicitUrl,
    /// Upload the keys (self-hosted / air-gapped Kubernetes). No auto-refresh.
    Inline,
}

#[derive(Args)]
pub struct Common {
    /// Print the request body instead of calling the API.
    #[arg(long)]
    dry_run: bool,
    /// API base URL (default: ANTHROPIC_BASE_URL, else https://api.anthropic.com).
    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Args)]
pub struct IssuerArgs {
    #[arg(long, value_enum)]
    preset: Option<Preset>,
    /// Slug `^[a-z0-9-]+$` (github preset: github-actions).
    #[arg(long)]
    name: Option<String>,
    /// Exact `iss` claim value.
    #[arg(long)]
    issuer_url: Option<String>,
    /// Inferred from --jwks-url (explicit-url) or --jwks-file / --jwks-from (inline); default discovery.
    #[arg(long, value_enum)]
    jwks: Option<JwksMode>,
    /// Discovery document base, when it differs from the issuer URL.
    #[arg(long)]
    discovery_base: Option<String>,
    /// JWKS endpoint (explicit-url).
    #[arg(long)]
    jwks_url: Option<String>,
    /// Local JWKS file; its `keys` array is uploaded (inline).
    #[arg(long)]
    jwks_file: Option<PathBuf>,
    /// Download this JWKS now and upload its keys (inline).
    #[arg(long)]
    jwks_from: Option<String>,
    /// PEM file of a private CA for the JWKS fetch (discovery / explicit-url).
    #[arg(long)]
    ca_cert: Option<PathBuf>,
    /// Disable jti single-use (replay protection) for every rule on this issuer. Last resort.
    #[arg(long)]
    no_check_jti: bool,
    /// Max JWT lifetime (exp - iat) accepted, seconds (default 3600).
    #[arg(long)]
    max_lifetime: Option<u64>,
    #[command(flatten)]
    common: Common,
}

#[derive(Args)]
pub struct MatchArgs {
    #[arg(long, value_enum)]
    preset: Option<Preset>,
    /// github preset: OWNER/REPO (subject `repo:OWNER/REPO:*` + repository_owner claim).
    #[arg(long)]
    repo: Option<String>,
    /// github preset: pin a git ref (claims.ref), e.g. refs/heads/main. Omit for pull_request runs.
    #[arg(long = "ref")]
    git_ref: Option<String>,
    /// `sub` match: exact, or prefix with a trailing `*`.
    #[arg(long)]
    subject_prefix: Option<String>,
    /// Exact `aud` (github preset: https://api.anthropic.com).
    #[arg(long)]
    audience: Option<String>,
    /// Exact claim match, repeatable: --claim ref=refs/heads/main
    #[arg(long = "claim", value_parser = parse_kv)]
    claims: Vec<(String, String)>,
}

#[derive(Args)]
pub struct RuleArgs {
    /// Slug `^[a-z0-9-]+$`.
    #[arg(long)]
    name: String,
    /// Issuer id (fdis_…).
    #[arg(long)]
    issuer: String,
    /// Target service account (svac_…).
    #[arg(long)]
    service_account: String,
    /// Workspace the minted tokens bill to (wrkspc_…).
    #[arg(long)]
    workspace: Option<String>,
    #[arg(long)]
    all_workspaces: bool,
    #[command(flatten)]
    m: MatchArgs,
    /// CEL condition over `claims`, e.g. 'claims.ref in ["refs/heads/main"]'.
    #[arg(long)]
    condition: Option<String>,
    /// workspace:inference (Messages + Models, enough for glot) or workspace:developer.
    #[arg(long, default_value = "workspace:inference")]
    scope: String,
    /// Access-token lifetime, 60–86400 s (server default 3600).
    #[arg(long)]
    token_lifetime: Option<u64>,
    #[command(flatten)]
    common: Common,
}

#[derive(Args)]
pub struct CheckArgs {
    /// JWT file, or `-` for stdin (default: ANTHROPIC_IDENTITY_TOKEN_FILE).
    jwt: Option<PathBuf>,
    #[command(flatten)]
    m: MatchArgs,
    /// Expected `iss` (github preset: https://token.actions.githubusercontent.com).
    #[arg(long)]
    issuer_url: Option<String>,
    /// Issuer max JWT lifetime, seconds (default 3600).
    #[arg(long)]
    max_lifetime: Option<u64>,
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("{s:?}: expected KEY=VALUE"))
}

/// (subject_prefix, audience, claims) after applying the preset.
type Match = (Option<String>, Option<String>, BTreeMap<String, String>);

fn resolve_match(m: &MatchArgs) -> Result<Match> {
    let mut claims: BTreeMap<String, String> = m.claims.iter().cloned().collect();
    let mut prefix = m.subject_prefix.clone();
    let mut audience = m.audience.clone();
    if let Some(Preset::Github) = m.preset {
        let repo = m
            .repo
            .as_deref()
            .context("--preset github needs --repo OWNER/REPO")?;
        let (p, c) = github_match(repo, m.git_ref.as_deref()).map_err(anyhow::Error::msg)?;
        prefix.get_or_insert(p);
        for (k, v) in c {
            claims.entry(k).or_insert(v);
        }
        audience.get_or_insert_with(|| DEFAULT_AUDIENCE.into());
    } else if m.repo.is_some() || m.git_ref.is_some() {
        bail!("--repo / --ref need --preset github");
    }
    Ok((prefix, audience, claims))
}

fn base_url(c: &Common) -> String {
    c.base_url
        .clone()
        .or_else(|| {
            std::env::var("ANTHROPIC_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "https://api.anthropic.com".into())
}

async fn create(
    c: &Common,
    collection: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    if c.dry_run {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(serde_json::Value::Null);
    }
    let base = base_url(c);
    let token = wif::admin_token(&base, |k| std::env::var(k).ok()).await?;
    Ok(wif::admin_create(&base, &token, collection, &body).await?)
}

pub async fn run(cmd: WifCmd) -> Result<ExitCode> {
    match cmd {
        WifCmd::Issuer(IssuerCmd::Create(a)) => issuer(a).await,
        WifCmd::Rule(RuleCmd::Create(a)) => rule(a).await,
        WifCmd::Check(a) => check(a),
    }
}

async fn issuer(a: IssuerArgs) -> Result<ExitCode> {
    let github = matches!(a.preset, Some(Preset::Github));
    let issuer_url = a
        .issuer_url
        .clone()
        .or_else(|| github.then(|| GITHUB_ISSUER.to_string()))
        .context("--issuer-url is required (or --preset github)")?;
    let name = a
        .name
        .clone()
        .or_else(|| github.then(|| "github-actions".to_string()))
        .context("--name is required")?;
    let mode = a.jwks.unwrap_or(if a.jwks_url.is_some() {
        JwksMode::ExplicitUrl
    } else if a.jwks_file.is_some() || a.jwks_from.is_some() {
        JwksMode::Inline
    } else {
        JwksMode::Discovery
    });
    let ca_cert_pem = match &a.ca_cert {
        Some(p) => {
            Some(std::fs::read_to_string(p).with_context(|| format!("--ca-cert {}", p.display()))?)
        }
        None => None,
    };
    let jwks = match mode {
        JwksMode::Discovery => Jwks::Discovery {
            discovery_base: a.discovery_base.clone(),
            ca_cert_pem,
        },
        JwksMode::ExplicitUrl => Jwks::ExplicitUrl {
            url: a
                .jwks_url
                .clone()
                .context("--jwks explicit-url needs --jwks-url")?,
            ca_cert_pem,
        },
        JwksMode::Inline => {
            if ca_cert_pem.is_some() {
                bail!("--ca-cert applies to discovery / explicit-url (inline fetches nothing)");
            }
            let text = match (&a.jwks_file, &a.jwks_from) {
                (Some(p), None) => std::fs::read_to_string(p)
                    .with_context(|| format!("--jwks-file {}", p.display()))?,
                (None, Some(u)) => {
                    reqwest::get(u)
                        .await
                        .and_then(|r| r.error_for_status())
                        .with_context(|| format!("--jwks-from {u}"))?
                        .text()
                        .await?
                }
                _ => bail!("--jwks inline needs exactly one of --jwks-file / --jwks-from"),
            };
            Jwks::Inline {
                keys: wif::jwks_keys(&text).map_err(anyhow::Error::msg)?,
            }
        }
    };
    let inline = matches!(jwks, Jwks::Inline { .. });
    let body = IssuerSpec {
        name,
        issuer_url,
        jwks,
        check_jti: !a.no_check_jti,
        max_jwt_lifetime_seconds: a.max_lifetime,
    }
    .body()
    .map_err(anyhow::Error::msg)?;
    let v = create(&a.common, "federation_issuers", body).await?;
    if a.common.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    anstream::println!(
        "{GREEN}✓{GREEN:#} issuer {BOLD}{}{BOLD:#} ({})",
        v["id"].as_str().unwrap_or("?"),
        v["name"]
    );
    if inline {
        anstream::println!(
            "{YELLOW}!{YELLOW:#} inline keys are never refreshed: rerun with the new JWKS on every key rotation"
        );
    }
    anstream::println!(
        "{DIM}next: glot wif rule create --issuer {} --service-account svac_… --workspace wrkspc_… …{DIM:#}",
        v["id"].as_str().unwrap_or("fdis_…")
    );
    Ok(ExitCode::SUCCESS)
}

async fn rule(a: RuleArgs) -> Result<ExitCode> {
    let (subject_prefix, audience, claims) = resolve_match(&a.m)?;
    let body = RuleSpec {
        name: a.name,
        issuer_id: a.issuer,
        service_account_id: a.service_account.clone(),
        workspace_id: a.workspace.clone(),
        all_workspaces: a.all_workspaces,
        subject_prefix,
        audience,
        claims,
        condition: a.condition,
        oauth_scope: a.scope,
        token_lifetime_seconds: a.token_lifetime,
    }
    .body()
    .map_err(anyhow::Error::msg)?;
    let v = create(&a.common, "federation_rules", body).await?;
    if a.common.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    let id = v["id"].as_str().unwrap_or("fdrl_…");
    anstream::println!("{GREEN}✓{GREEN:#} rule {BOLD}{id}{BOLD:#} ({})", v["name"]);
    anstream::println!(
        "\nSet on the workload (GitHub Action inputs: federation-rule-id, organization-id, service-account-id, workspace-id):"
    );
    println!("  ANTHROPIC_FEDERATION_RULE_ID={id}");
    println!("  ANTHROPIC_ORGANIZATION_ID=<your organization UUID>");
    println!("  ANTHROPIC_SERVICE_ACCOUNT_ID={}", a.service_account);
    if a.all_workspaces {
        println!(
            "  ANTHROPIC_WORKSPACE_ID=wrkspc_…   # required: the rule spans several workspaces"
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn check(a: CheckArgs) -> Result<ExitCode> {
    let path = a
        .jwt
        .clone()
        .or_else(|| std::env::var_os("ANTHROPIC_IDENTITY_TOKEN_FILE").map(PathBuf::from))
        .context("pass a JWT file (or `-` for stdin)")?;
    let jwt = if path.as_os_str() == "-" {
        std::io::read_to_string(std::io::stdin())?
    } else {
        std::fs::read_to_string(&path).with_context(|| path.display().to_string())?
    };
    let (subject_prefix, audience, claims) = resolve_match(&a.m)?;
    let issuer_url = a
        .issuer_url
        .clone()
        .or_else(|| matches!(a.m.preset, Some(Preset::Github)).then(|| GITHUB_ISSUER.to_string()));
    let expect = Expect {
        issuer_url,
        audience,
        subject_prefix,
        claims,
        max_lifetime: a.max_lifetime,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let lines = wif::check_jwt(&jwt, &expect, now).map_err(anyhow::Error::msg)?;
    let mut failed = 0;
    for (ok, label, detail) in &lines {
        if *ok {
            anstream::println!("{GREEN}✓{GREEN:#} {label:<10} {detail}");
        } else {
            failed += 1;
            anstream::println!("{RED}✗{RED:#} {label:<10} {detail}");
        }
    }
    anstream::println!(
        "{DIM}local check only: the signature is verified by Anthropic against the issuer's JWKS{DIM:#}"
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_preset_fills_match() {
        let m = MatchArgs {
            preset: Some(Preset::Github),
            repo: Some("acme/web".into()),
            git_ref: None,
            subject_prefix: None,
            audience: None,
            claims: vec![("environment".into(), "prod".into())],
        };
        let (p, a, c) = resolve_match(&m).unwrap();
        assert_eq!(p.as_deref(), Some("repo:acme/web:*"));
        assert_eq!(a.as_deref(), Some(DEFAULT_AUDIENCE));
        assert_eq!(c["repository_owner"], "acme");
        assert_eq!(c["environment"], "prod");
        assert!(!c.contains_key("ref"));
        assert!(parse_kv("noequals").is_err());
    }
}
