//! `glot seo`: multilingual SEO audit of the running site, Next.js App Router fixes
//! (hreflang/canonical, sitemap, robots) and search-localized metadata.
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::collections::BTreeMap;
use std::process::ExitCode;

use crate::style::*;
use crate::{confirm, open, print_json};

mod audit;
mod fix;

#[derive(Args)]
pub struct SeoArgs {
    #[command(subcommand)]
    cmd: SeoCmd,
}

#[derive(Subcommand)]
enum SeoCmd {
    /// Crawl every locale: hreflang (reciprocal, x-default), canonical, <html lang>, translated
    /// title/description and their SERP length, sitemap, untranslated indexable pages. Exit 1 on errors.
    Audit(AuditArgs),
    /// Next.js App Router: canonical + hreflang in generateMetadata, app/sitemap.ts, app/robots.ts.
    Fix(FixArgs),
    /// Localize page titles/descriptions for search (what natives type) instead of literally.
    Keywords(KeywordsArgs),
}

#[derive(Args)]
struct AuditArgs {
    /// Site origin, e.g. http://localhost:3000
    #[arg(long)]
    base_url: String,
    /// Routes in the source locale (comma separated). Default: `/`, or the sitemap with `--sitemap`.
    #[arg(long, value_delimiter = ',')]
    routes: Vec<String>,
    /// Take the routes from /sitemap.xml.
    #[arg(long)]
    sitemap: bool,
    /// URL of a route in another locale.
    #[arg(long, default_value = "/{locale}{route}")]
    url_pattern: String,
    /// The source locale is prefixed too (default: unprefixed, `/pricing`).
    #[arg(long)]
    prefix_default: bool,
    /// Locales, source first (default: glot.toml).
    #[arg(long, value_delimiter = ',')]
    locales: Vec<String>,
    /// JSON report instead of markdown.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct FixArgs {
    /// Show the diff; write nothing.
    #[arg(long)]
    dry_run: bool,
    #[arg(short, long)]
    yes: bool,
}

#[derive(Args)]
struct KeywordsArgs {
    /// Locales (comma separated). Default: all targets.
    #[arg(long, value_delimiter = ',')]
    locale: Vec<String>,
    /// Only these keys. Default: the keys used in generateMetadata (title/description).
    #[arg(long, value_delimiter = ',')]
    keys: Vec<String>,
    /// Write the best variant that passes validation (glossary, placeholders, length).
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    json: bool,
}

pub async fn run(a: SeoArgs) -> Result<ExitCode> {
    match a.cmd {
        SeoCmd::Audit(a) => run_audit(a).await,
        SeoCmd::Fix(a) => run_fix(a),
        SeoCmd::Keywords(a) => keywords(a).await,
    }
}

async fn fetch(c: &reqwest::Client, url: &str) -> (u16, String, String) {
    match c.get(url).send().await {
        Ok(r) => {
            let (status, final_url) = (r.status().as_u16(), r.url().to_string());
            (status, final_url, r.text().await.unwrap_or_default())
        }
        Err(_) => (0, url.into(), String::new()),
    }
}

fn locale_url(a: &AuditArgs, base: &str, src: &str, locale: &str, route: &str) -> String {
    if locale == src && !a.prefix_default {
        return format!("{base}{route}");
    }
    let r = if route == "/" { "" } else { route };
    let p = a
        .url_pattern
        .replace("{locale}", locale)
        .replace("{route}", r);
    format!("{base}{}", if p.is_empty() { "/" } else { &p })
}

async fn run_audit(a: AuditArgs) -> Result<ExitCode> {
    let ws = open().ok();
    let locales = if a.locales.is_empty() {
        ws.as_ref()
            .map(|w| w.config.all_locales())
            .context("no glot.toml here: pass --locales en,fr,...")?
    } else {
        a.locales.clone()
    };
    let base = a.base_url.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .user_agent("glot-seo-audit")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let (status, _, xml) = fetch(&client, &format!("{base}/sitemap.xml")).await;
    let sitemap = if status == 200 {
        let mut locs = audit::sitemap_locs(&xml);
        if xml.contains("<sitemapindex") {
            let children = std::mem::take(&mut locs);
            for c in children {
                locs.extend(audit::sitemap_locs(&fetch(&client, &c).await.2));
            }
        }
        Some(locs)
    } else {
        None
    };
    let (status, _, robots) = fetch(&client, &format!("{base}/robots.txt")).await;
    let robots = (status == 200).then_some(robots);
    let mut routes = a.routes.clone();
    if routes.is_empty() && a.sitemap {
        let sm = sitemap.as_ref().context("--sitemap: no /sitemap.xml")?;
        for loc in sm {
            let path = reqwest::Url::parse(loc)
                .map(|u| u.path().to_string())
                .unwrap_or_default();
            let first = path.split('/').nth(1).unwrap_or("");
            let route = if locales.iter().any(|l| l == first) {
                match &path[first.len() + 1..] {
                    "" => "/".to_string(),
                    r => r.to_string(),
                }
            } else {
                path
            };
            if !routes.contains(&route) {
                routes.push(route);
            }
        }
    }
    if routes.is_empty() {
        routes.push("/".into());
    }
    let mut urls = BTreeMap::new();
    let mut pages = std::collections::HashMap::new();
    for route in &routes {
        for l in &locales {
            let u = locale_url(&a, &base, &locales[0], l, route);
            let (status, final_url, html) = fetch(&client, &u).await;
            let p = audit::parse(&final_url, status, &html);
            pages.insert(audit::norm(&final_url), p.clone());
            pages.insert(audit::norm(&u), p);
            urls.insert((route.clone(), l.clone()), u);
        }
    }
    // hreflang targets outside the audited URLs (reciprocity / broken links).
    let extra: Vec<String> = pages
        .values()
        .flat_map(|p| p.hreflang.iter().map(|(_, h)| audit::norm(h)))
        .filter(|h| !pages.contains_key(h))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for u in extra {
        let (status, final_url, html) = fetch(&client, &u).await;
        pages.insert(u, audit::parse(&final_url, status, &html));
    }
    let catalogs = ws
        .as_ref()
        .map(|w| {
            w.catalogs
                .iter()
                .map(|(l, c)| (l.clone(), c.entries.clone()))
                .collect()
        })
        .unwrap_or_default();
    let site = audit::Site {
        locales,
        routes,
        urls,
        pages,
        sitemap,
        robots,
        catalogs,
    };
    let findings = audit::audit(&site);
    let n = site.urls.len();
    let errors = findings
        .iter()
        .filter(|f| f.severity == audit::Severity::Error)
        .count();
    if a.json {
        print_json(&serde_json::json!({
            "pages": n, "errors": errors, "warnings": findings.len() - errors, "findings": findings,
        }))?;
    } else {
        print!("{}", audit::markdown(&findings, n));
    }
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn run_fix(a: FixArgs) -> Result<ExitCode> {
    let ws = open()?;
    let project = crate::setup::detect(&ws.root).context("no package.json framework detected")?;
    let plan = fix::plan(&ws.root, &project).map_err(anyhow::Error::msg)?;
    if plan.changes.is_empty() {
        anstream::println!("{DIM}SEO setup already in place.{DIM:#}");
        return Ok(ExitCode::SUCCESS);
    }
    crate::i18nize::print_setup(&plan, a.dry_run || !a.yes);
    if a.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    if !a.yes && !confirm("Apply these SEO changes?")? {
        return Ok(ExitCode::SUCCESS);
    }
    crate::setup::write(&ws.root, &plan)?;
    anstream::println!(
        "{GREEN}✓ done.{GREEN:#} Next: build, then {BOLD}glot seo audit --base-url …{BOLD:#}"
    );
    Ok(ExitCode::SUCCESS)
}

async fn keywords(a: KeywordsArgs) -> Result<ExitCode> {
    use glot_llm::seo;
    let mut ws = open()?;
    let locales = (!a.locale.is_empty()).then_some(a.locale.as_slice());
    let keys = (!a.keys.is_empty()).then_some(a.keys.as_slice());
    let (suggestions, usage) = seo::localize_workspace(&ws, locales, keys).await?;
    if suggestions.is_empty() {
        anstream::println!(
            "{YELLOW}no metadata keys{YELLOW:#} (keys used in generateMetadata, or pass --keys)"
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut applied = vec![];
    for s in &suggestions {
        if !s.search_terms.is_empty() {
            let note = seo::note_with_terms(
                ws.lock.notes.get(&s.key).map(String::as_str),
                &s.locale,
                &s.search_terms,
            );
            ws.lock.notes.insert(s.key.clone(), note);
        }
    }
    ws.lock.save(&ws.root)?;
    if a.apply {
        for s in &suggestions {
            for v in &s.variants {
                let issues = ws.set_translation(&s.locale, &s.key, v)?;
                if !glot_core::validate::has_errors(&issues) {
                    applied.push((s.locale.clone(), s.key.clone(), v.clone()));
                    break;
                }
            }
        }
    }
    if a.json {
        print_json(&serde_json::json!({ "suggestions": suggestions, "applied": applied.len() }))?;
    } else {
        for s in &suggestions {
            anstream::println!(
                "{BOLD}{}{BOLD:#} [{}] {DIM}searched as:{DIM:#} {}",
                s.key,
                s.locale,
                s.search_terms.join(", ")
            );
            for (i, v) in s.variants.iter().enumerate() {
                let mark = if applied
                    .iter()
                    .any(|(l, k, x)| *l == s.locale && *k == s.key && x == v)
                {
                    "✓"
                } else {
                    " "
                };
                anstream::println!(
                    "  {GREEN}{mark}{GREEN:#} {}. {v} {DIM}({}/{}){DIM:#}",
                    i + 1,
                    seo::serp_width(v),
                    s.kind.max()
                );
            }
        }
        anstream::eprintln!(
            "{DIM}search terms stored in glot.lock notes · LLM cost ${:.4}{DIM:#}",
            usage.cost_usd
        );
        if a.apply {
            anstream::println!(
                "{GREEN}✓{GREEN:#} {} translations updated (validated)",
                applied.len()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}
