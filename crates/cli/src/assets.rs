//! `glot assets`: localize beyond the UI (emails, SVG illustrations, OG images) with the same
//! guarantees: keys in the catalog, validator + glossary + lockfile through the regular translate
//! pipeline, pixel budgets in `.glot/layout.json` (too-wide translations are retried before writing),
//! then per-locale renders and asset checks. Idempotent: a second run changes nothing.
use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

use glot_core::measure::{self, Fit, Font};
use glot_core::validate::{Issue, IssueCode, Severity};
use glot_extract::{email, svg};
use glot_llm::engine::{TranslateOptions, Workspace};
use glot_llm::i18nize::{self, Lib};
use resvg::{tiny_skia, usvg};

use crate::style::*;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Step {
    /// Turn the copy of templates/images into catalog keys (+ pixel budgets).
    I18nize,
    /// Translate the asset keys (validator, glossary, budgets, retries).
    Translate,
    /// Render every locale: `.glot/emails/`, `public/img/{locale}/`, `public/og/{locale}/`.
    Preview,
    /// Asset checks: links, subject/preheader length, RTL, rendered width. Exit 1 on errors.
    Check,
}

#[derive(Args)]
pub struct AssetsArgs {
    /// One step only (default: i18nize, translate, preview, check).
    #[arg(value_enum)]
    pub step: Option<Step>,
    /// Emails: React Email (`.tsx`/`.jsx`), HTML and MJML templates.
    #[arg(long)]
    pub emails: bool,
    /// SVG illustrations with `<text>`.
    #[arg(long)]
    pub svg: bool,
    /// OG images written in JSX (@vercel/og / satori).
    #[arg(long)]
    pub og: bool,
    #[arg(long, default_value = "emails")]
    pub emails_dir: String,
    #[arg(long, default_value = "assets")]
    pub svg_dir: String,
    #[arg(long, default_value = "og")]
    pub og_dir: String,
    /// Do not ask for confirmation.
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    EmailJsx,
    EmailMarkup,
    Svg,
    Og,
}

struct Asset {
    kind: Kind,
    path: String,
    name: String,
}

/// What an asset needs translated, and the budgets of its keys.
struct Inv {
    asset: Asset,
    keys: BTreeSet<String>,
    fits: Vec<(String, Fit)>,
}

fn find(root: &Path, dir: &str, exts: &[&str]) -> Result<Vec<String>> {
    let cfg = glot_core::config::ExtractConfig {
        include: exts.iter().map(|e| format!("{dir}/**/*.{e}")).collect(),
        exclude: vec![],
        functions: vec![],
    };
    if !root.join(dir).is_dir() {
        return Ok(vec![]);
    }
    Ok(glot_extract::i18nize::source_files(root, &cfg)?)
}

fn discover(root: &Path, a: &AssetsArgs) -> Result<Vec<Asset>> {
    let all = !(a.emails || a.svg || a.og);
    let mut out = vec![];
    let mut add = |kind, dir: &str, exts: &[&str]| -> Result<()> {
        for path in find(root, dir, exts)? {
            let name = email::template_name(&path);
            out.push(Asset { kind, path, name });
        }
        Ok(())
    };
    if all || a.emails {
        add(Kind::EmailJsx, &a.emails_dir, &["tsx", "jsx"])?;
        add(Kind::EmailMarkup, &a.emails_dir, &["html", "mjml"])?;
    }
    if all || a.svg {
        add(Kind::Svg, &a.svg_dir, &["svg"])?;
    }
    if all || a.og {
        add(Kind::Og, &a.og_dir, &["tsx", "jsx"])?;
    }
    Ok(out)
}

pub async fn run(a: AssetsArgs) -> Result<ExitCode> {
    // usvg logs every per-char font fallback (CJK, Arabic) at warn level.
    log::set_max_level(log::LevelFilter::Error);
    let mut ws = crate::open()?;
    let assets = discover(&ws.root, &a)?;
    if assets.is_empty() {
        bail!(
            "no assets found (looked in {}/ for .tsx/.jsx/.html/.mjml, {}/ for .svg, {}/ for .tsx/.jsx)",
            a.emails_dir,
            a.svg_dir,
            a.og_dir
        );
    }
    let steps: Vec<Step> = match a.step {
        Some(s) => vec![s],
        None => vec![Step::I18nize, Step::Translate, Step::Preview, Step::Check],
    };
    if steps.contains(&Step::I18nize) {
        i18nize_assets(&mut ws, &assets).await?;
    }
    let inv = inventory(&ws, assets)?;
    let budgets = save_budgets(&ws, &inv)?;
    if steps.contains(&Step::I18nize) {
        let keys: usize = inv.iter().map(|i| i.keys.len()).sum();
        anstream::println!(
            "{BOLD}{}{BOLD:#} assets, {BOLD}{keys}{BOLD:#} keys, {budgets} pixel budgets updated in {}",
            inv.len(),
            measure::LAYOUT_FILE
        );
    }
    if steps.contains(&Step::Translate) {
        translate(&mut ws, &inv, a.yes).await?;
    }
    if steps.contains(&Step::Preview) || steps.contains(&Step::Check) {
        let (written, issues) = render(&ws, &inv, steps.contains(&Step::Preview))?;
        if steps.contains(&Step::Preview) {
            anstream::println!(
                "{GREEN}✓{GREEN:#} {} renders for {} ({written} changed)",
                inv.len() * ws.config.all_locales().len(),
                ws.config.all_locales().join(", ")
            );
        }
        if steps.contains(&Step::Check) {
            return Ok(check(&ws, &inv, issues));
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- i18nize

async fn i18nize_assets(ws: &mut Workspace, assets: &[Asset]) -> Result<()> {
    // JSX (React Email, OG): the regular i18nize, keys under the file's namespace (`emails.welcome`).
    let jsx: Vec<String> = assets
        .iter()
        .filter(|a| matches!(a.kind, Kind::EmailJsx | Kind::Og))
        .map(|a| a.path.clone())
        .collect();
    if !jsx.is_empty() {
        let lib = i18nize::detect_lib(&ws.root).unwrap_or(Lib::NextIntl);
        let plan = i18nize::plan(ws, &jsx, lib).await?;
        for f in &plan.files {
            if let Some(e) = &f.error {
                anstream::println!("{RED}✗{RED:#} {}: {e}", f.path);
            }
        }
        if plan.converted() > 0 {
            i18nize::apply(ws, &plan)?;
            for f in plan.changed_files() {
                anstream::println!("  {GREEN}~{GREEN:#} {} ({} strings)", f.path, f.converted);
            }
        }
    }
    for a in assets {
        let abs = ws.root.join(&a.path);
        let src = std::fs::read_to_string(&abs).with_context(|| a.path.clone())?;
        let (prefix, found): (String, Vec<(String, String, String)>) = match a.kind {
            Kind::EmailJsx => {
                let key = format!("emails.{}.subject", a.name);
                if let Some((new, text)) = email::i18nize_subject(&a.path, &src, &key) {
                    std::fs::write(&abs, new)?;
                    ws.add_source(
                        &key,
                        &text,
                        Some("Email subject line (inbox preview, keep it short)"),
                    )?;
                    anstream::println!("  {GREEN}~{GREEN:#} {} (subject)", a.path);
                }
                continue;
            }
            Kind::Og => continue,
            Kind::EmailMarkup => (
                format!("emails.{}.", a.name),
                email::segments(&a.path, &src)
                    .into_iter()
                    .map(|(k, s)| {
                        let note = match k.rsplit('.').next() {
                            Some("subject") => {
                                "Email subject line (inbox preview, keep it short)".to_string()
                            }
                            Some("preheader") => "Email preheader (inbox preview text)".to_string(),
                            _ => format!("Text of the email template {}", a.path),
                        };
                        (k, s.text, note)
                    })
                    .collect(),
            ),
            Kind::Svg => (
                format!("svg.{}.", a.name),
                svg::extract(&a.path, &src)
                    .into_iter()
                    .map(|t| {
                        let note = format!(
                            "Text drawn in the SVG image {} ({})",
                            a.path,
                            if t.fit.is_some() {
                                "single line, fixed box"
                            } else {
                                "accessible title"
                            }
                        );
                        (t.key, t.seg.text, note)
                    })
                    .collect(),
            ),
        };
        // The file is the source of truth for its namespace: add/update, drop keys no longer in it.
        let mut keep = BTreeSet::new();
        for (k, text, note) in found {
            if ws.source().entries.get(&k) != Some(&text) {
                ws.add_source(&k, &text, Some(&note))?;
            }
            keep.insert(k);
        }
        let stale: Vec<String> = ws
            .source()
            .entries
            .keys()
            .filter(|k| k.starts_with(&prefix) && !keep.contains(*k))
            .cloned()
            .collect();
        if !stale.is_empty() {
            ws.remove_keys(&stale)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- inventory & budgets

/// One line of `n` average Latin chars at a typical inbox font (CJK counts double, like the inbox).
fn chars_fit(n: usize) -> Fit {
    let font = Font {
        family: "Helvetica".into(),
        postscript: None,
        size_px: 14.0,
        weight: 400,
        letter_spacing_px: 0.0,
        text_transform: None,
    };
    let sample = "the quick brown fox jumps over the lazy dog";
    let avg = measure::measure_text(sample, &font) / sample.chars().count() as f32;
    Fit {
        // +0.5 char: `approx_chars` floors to exactly `n`.
        max_width_px: (avg * (n as f32 + 0.5)).round(),
        max_lines: Some(1),
        font,
        text: None,
        text_px: None,
    }
}

fn lookup_in<'w>(ws: &'w Workspace, locale: &str) -> impl Fn(&str) -> Option<String> + 'w {
    let source = ws.source();
    let target = ws.catalogs.get(locale);
    move |k: &str| {
        target
            .and_then(|c| c.entries.get(k))
            .filter(|v| !v.is_empty())
            .or_else(|| source.entries.get(k))
            .cloned()
    }
}

fn inventory(ws: &Workspace, assets: Vec<Asset>) -> Result<Vec<Inv>> {
    let src_loc = ws.config.source_locale.clone();
    let source = lookup_in(ws, &src_loc);
    let mut out = vec![];
    for asset in assets {
        let text = std::fs::read_to_string(ws.root.join(&asset.path))
            .with_context(|| asset.path.clone())?;
        let (keys, fits): (BTreeSet<String>, Vec<(String, Fit)>) = match asset.kind {
            Kind::EmailJsx => {
                let r =
                    email::render_jsx(&asset.path, &text, &source).map_err(anyhow::Error::msg)?;
                let mut fits = vec![];
                if let Some(k) = &r.subject_key {
                    fits.push((k.clone(), chars_fit(email::SUBJECT_MAX)));
                }
                if let Some(k) = preheader_key(&r.root) {
                    fits.push((k, chars_fit(email::PREHEADER_MAX)));
                }
                (r.keys.into_iter().collect(), fits)
            }
            Kind::EmailMarkup => {
                let segs = email::segments(&asset.path, &text);
                let fits = segs
                    .iter()
                    .filter_map(|(k, _)| match k.rsplit('.').next() {
                        Some("subject") => Some((k.clone(), chars_fit(email::SUBJECT_MAX))),
                        Some("preheader") => Some((k.clone(), chars_fit(email::PREHEADER_MAX))),
                        _ => None,
                    })
                    .collect();
                (segs.into_iter().map(|(k, _)| k).collect(), fits)
            }
            Kind::Svg => {
                let t = svg::extract(&asset.path, &text);
                let fits = t
                    .iter()
                    .filter_map(|t| Some((t.key.clone(), t.fit.clone()?)))
                    .collect();
                (t.into_iter().map(|t| t.key).collect(), fits)
            }
            Kind::Og => {
                let r =
                    email::render_jsx(&asset.path, &text, &source).map_err(anyhow::Error::msg)?;
                let og = svg::og_svg(&r.root, &src_loc);
                (r.keys.into_iter().collect(), og.fits)
            }
        };
        out.push(Inv { asset, keys, fits });
    }
    Ok(out)
}

fn preheader_key(root: &email::Node) -> Option<String> {
    let mut out = None;
    root.walk(&mut |n| {
        if n.tag() == Some("Preview") {
            n.walk(&mut |m| {
                if let email::Node::Msg { key, .. } = m
                    && out.is_none()
                {
                    out = Some(key.clone());
                }
            });
        }
    });
    out
}

/// Merge the assets' budgets into `.glot/layout.json`; returns how many changed.
fn save_budgets(ws: &Workspace, inv: &[Inv]) -> Result<usize> {
    let mut layout = measure::load_layout(&ws.root)?;
    let mut changed = 0;
    for (k, f) in inv.iter().flat_map(|i| &i.fits) {
        if layout.get(k) != Some(f) {
            layout.insert(k.clone(), f.clone());
            changed += 1;
        }
    }
    if changed > 0 {
        measure::save_layout(&ws.root, &layout)?;
    }
    Ok(changed)
}

// ---------------------------------------------------------------- translate

async fn translate(ws: &mut Workspace, inv: &[Inv], yes: bool) -> Result<()> {
    let keys: Vec<String> = inv
        .iter()
        .flat_map(|i| &i.keys)
        .filter(|k| ws.source().entries.contains_key(*k))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    let opts = TranslateOptions {
        locales: None,
        keys: Some(keys),
        force: false,
        dry_run: true,
    };
    let dry = ws.translate(opts.clone()).await?;
    let total: usize = dry.planned.values().map(Vec::len).sum();
    if total == 0 {
        anstream::println!("{GREEN}Asset translations up to date.{GREEN:#}");
        return Ok(());
    }
    let max = dry.estimated_cost_max_usd.max(dry.estimated_cost_usd);
    anstream::println!(
        "Translating {BOLD}{total}{BOLD:#} asset strings {DIM}(est. ${:.4}, up to ${max:.4}){DIM:#}",
        dry.estimated_cost_usd
    );
    if max > 1.0 && !yes && !crate::confirm(&format!("Estimated cost up to ${max:.2}. Continue?"))?
    {
        bail!("aborted");
    }
    let report = ws
        .translate(TranslateOptions {
            dry_run: false,
            ..opts
        })
        .await?;
    for (loc, items) in &report.written {
        let retried = items.iter().filter(|t| t.attempts > 1).count();
        anstream::println!(
            "  {GREEN}✓{GREEN:#} {loc:<6} {} written{}",
            items.len(),
            if retried > 0 {
                format!(" {DIM}({retried} retried to fit their budget or fix issues){DIM:#}")
            } else {
                String::new()
            }
        );
    }
    for (loc, items) in &report.rejected {
        for t in items {
            crate::print_issues(loc, &t.key, &t.issues);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- render & check

fn fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.load_fonts_dir(measure::FONTS_DIR);
        // macOS' placeholder font would win the per-char fallback and draw boxes.
        let last: Vec<_> = db
            .faces()
            .filter(|f| f.families.iter().any(|(n, _)| n.starts_with(".LastResort")))
            .map(|f| f.id)
            .collect();
        for id in last {
            db.remove_face(id);
        }
        db.set_sans_serif_family("Helvetica");
        Arc::new(db)
    })
    .clone()
}

fn svg_tree(svg: &str) -> Result<usvg::Tree> {
    let opt = usvg::Options {
        fontdb: fontdb(),
        font_family: "Helvetica".into(),
        ..Default::default()
    };
    usvg::Tree::from_str(svg, &opt).context("invalid SVG")
}

fn png(tree: &usvg::Tree) -> Result<Vec<u8>> {
    let size = tree.size().to_int_size();
    let mut pix = tiny_skia::Pixmap::new(size.width(), size.height()).context("empty SVG")?;
    resvg::render(tree, tiny_skia::Transform::default(), &mut pix.as_mut());
    pix.encode_png().context("png encoding")
}

/// Rendered width of every `<text>` with an id (user units).
fn text_widths(tree: &usvg::Tree) -> BTreeMap<String, f32> {
    fn walk(g: &usvg::Group, out: &mut BTreeMap<String, f32>) {
        for n in g.children() {
            match n {
                usvg::Node::Group(g) => walk(g, out),
                usvg::Node::Text(t) if !t.id().is_empty() => {
                    out.insert(t.id().to_string(), t.bounding_box().width());
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(tree.root(), &mut out);
    out
}

fn write_if_changed(root: &Path, rel: &str, bytes: &[u8], n: &mut usize) -> Result<()> {
    let p = root.join(rel);
    if std::fs::read(&p).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, bytes)?;
    *n += 1;
    Ok(())
}

type Found = (String, String, Issue);

fn issue(severity: Severity, code: IssueCode, message: String) -> Issue {
    Issue {
        code,
        severity,
        message,
    }
}

/// Render every asset in every locale (writing files when `write`) and collect asset issues
/// as (locale, key or path, issue).
fn render(ws: &Workspace, inv: &[Inv], write: bool) -> Result<(usize, Vec<Found>)> {
    let root = &ws.root;
    let src_loc = ws.config.source_locale.clone();
    let mut written = 0;
    let mut out: Vec<Found> = vec![];
    for i in inv {
        let a = &i.asset;
        let text = std::fs::read_to_string(root.join(&a.path))?;
        let mut source_links: Option<Vec<String>> = None;
        for loc in ws.config.all_locales() {
            let lookup = lookup_in(ws, &loc);
            let mut err = |key: &str, sev, code, msg: String| {
                out.push((loc.clone(), key.to_string(), issue(sev, code, msg)))
            };
            match a.kind {
                Kind::EmailJsx => {
                    let r =
                        email::render_jsx(&a.path, &text, &lookup).map_err(anyhow::Error::msg)?;
                    for k in &r.missing {
                        err(k, Severity::Error, IssueCode::Empty, "key used by the template is not in the source catalog (run `glot assets i18nize`)".into());
                    }
                    email_checks(
                        &r.subject,
                        &r.preheader(),
                        r.subject_key.as_deref(),
                        &a.path,
                        &mut err,
                    );
                    let links = r.links();
                    match &source_links {
                        None => source_links = Some(links),
                        Some(s) if *s != links => err(
                            &a.path,
                            Severity::Error,
                            IssueCode::TagMismatch,
                            format!("links differ from the source email: {links:?} vs {s:?}"),
                        ),
                        _ => {}
                    }
                    if email::is_rtl(&loc) && !r.has_dir() {
                        err(&a.path, Severity::Warning, IssueCode::TagMismatch, "<Html> has no `dir` prop: this right-to-left email will be sent left-to-right; pass dir=\"rtl\" for RTL locales".into());
                    }
                    if write {
                        let rel = format!(".glot/emails/{}/{loc}.html", a.name);
                        write_if_changed(
                            root,
                            &rel,
                            email::to_html(&r, &loc).as_bytes(),
                            &mut written,
                        )?;
                    }
                }
                Kind::EmailMarkup => {
                    let segs = email::segments(&a.path, &text);
                    let get = |k: &str| lookup(k).unwrap_or_default();
                    let value = |part: &str| {
                        segs.iter()
                            .find(|(k, _)| k.ends_with(part))
                            .map(|(k, _)| get(k))
                    };
                    email_checks(
                        &value(".subject"),
                        &value(".preheader"),
                        None,
                        &a.path,
                        &mut err,
                    );
                    if write {
                        let repl: Vec<_> = segs
                            .iter()
                            .map(|(k, s)| (s.span, s.attr.is_some(), get(k)))
                            .collect();
                        let html = email::set_lang_dir(&email::localize(&text, &repl), &loc);
                        let ext = a.path.rsplit('.').next().unwrap_or("html");
                        let rel = format!(".glot/emails/{}/{loc}.{ext}", a.name);
                        write_if_changed(root, &rel, html.as_bytes(), &mut written)?;
                    }
                }
                Kind::Svg => {
                    let texts = svg::extract(&a.path, &text);
                    let value = |k: &str| lookup(k).unwrap_or_default();
                    let tree = svg_tree(&svg::localize(&text, &texts, &value, true))?;
                    // Real width: the box resvg actually renders, against the budget.
                    let widths = text_widths(&tree);
                    let mut seen = BTreeSet::new();
                    for t in &texts {
                        let (Some(fit), Some(id)) = (&t.fit, svg::text_id(t)) else {
                            continue;
                        };
                        if !seen.insert(id.clone()) {
                            continue;
                        }
                        if let Some(w) = widths.get(&id)
                            && *w > fit.max_width_px * (1.0 + measure::MARGIN)
                        {
                            err(
                                &t.key,
                                Severity::Error,
                                IssueCode::TooWide,
                                format!(
                                    "renders {w:.0}px wide in {}, its box is {:.0}px",
                                    a.path, fit.max_width_px
                                ),
                            );
                        }
                    }
                    if write {
                        let rel = format!(
                            "public/img/{loc}/{}",
                            a.path.rsplit('/').next().unwrap_or(&a.path)
                        );
                        write_if_changed(
                            root,
                            &rel,
                            svg::localize(&text, &texts, &value, false).as_bytes(),
                            &mut written,
                        )?;
                        write_if_changed(
                            root,
                            &format!("public/img/{loc}/{}.png", a.name),
                            &png(&tree)?,
                            &mut written,
                        )?;
                    }
                }
                Kind::Og => {
                    let r =
                        email::render_jsx(&a.path, &text, &lookup).map_err(anyhow::Error::msg)?;
                    let og = svg::og_svg(&r.root, &loc);
                    if og.overflow_px > 0.0 {
                        err(
                            &a.path,
                            Severity::Error,
                            IssueCode::TooWide,
                            format!(
                                "content is {:.0}px taller than the {}x{} image",
                                og.overflow_px, og.width, og.height
                            ),
                        );
                    }
                    if write {
                        let tree = svg_tree(&og.svg)?;
                        write_if_changed(
                            root,
                            &format!("public/og/{loc}/{}.png", a.name),
                            &png(&tree)?,
                            &mut written,
                        )?;
                    }
                }
            }
            if loc != src_loc {
                let target = ws.catalogs.get(&loc);
                for k in &i.keys {
                    if target
                        .and_then(|c| c.entries.get(k))
                        .is_none_or(|v| v.is_empty())
                    {
                        err(
                            k,
                            Severity::Warning,
                            IssueCode::Untranslated,
                            "no translation yet: renders the source text".into(),
                        );
                    }
                }
            }
        }
    }
    Ok((written, out))
}

fn email_checks(
    subject: &Option<String>,
    preheader: &Option<String>,
    subject_key: Option<&str>,
    path: &str,
    err: &mut impl FnMut(&str, Severity, IssueCode, String),
) {
    match subject {
        Some(s) if s.chars().count() > email::SUBJECT_MAX => err(
            subject_key.unwrap_or(path),
            Severity::Error,
            IssueCode::TooLong,
            format!(
                "subject is {} characters, inboxes cut it after {}",
                s.chars().count(),
                email::SUBJECT_MAX
            ),
        ),
        None => err(
            path,
            Severity::Warning,
            IssueCode::Empty,
            "no subject (`export const subject` or <title>)".into(),
        ),
        _ => {}
    }
    if let Some(p) = preheader
        && p.chars().count() > email::PREHEADER_MAX
    {
        err(
            path,
            Severity::Warning,
            IssueCode::TooLong,
            format!(
                "preheader is {} characters, inboxes show about {}",
                p.chars().count(),
                email::PREHEADER_MAX
            ),
        );
    }
}

fn check(ws: &Workspace, inv: &[Inv], mut issues: Vec<Found>) -> ExitCode {
    let keys: BTreeSet<&String> = inv.iter().flat_map(|i| &i.keys).collect();
    // Same validator as `glot check` (placeholders, plurals, glossary, pixel budgets), on asset keys.
    issues.extend(
        ws.check()
            .into_iter()
            .filter(|f| keys.contains(&f.key))
            .map(|f| (f.locale, f.key, f.issue)),
    );
    issues.sort_by(|a, b| (&a.0, &a.1, &a.2.message).cmp(&(&b.0, &b.1, &b.2.message)));
    issues.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1 && a.2.message == b.2.message);
    for (loc, key, i) in &issues {
        crate::print_issues(loc, key, std::slice::from_ref(i));
    }
    let errors = issues
        .iter()
        .filter(|(_, _, i)| i.severity == Severity::Error)
        .count();
    if errors == 0 {
        anstream::println!(
            "{GREEN}✓ assets check passed{GREEN:#} {DIM}({} warnings){DIM:#}",
            issues.len()
        );
        ExitCode::SUCCESS
    } else {
        anstream::println!(
            "{RED}✗ {errors} errors{RED:#}, {} warnings",
            issues.len() - errors
        );
        ExitCode::FAILURE
    }
}
