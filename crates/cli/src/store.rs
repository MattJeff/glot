//! `glot labs store`: App Store / Google Play listings in fastlane's layout
//! (`fastlane/metadata/<locale>/*.txt`, `fastlane/metadata/android/<locale>/*.txt`).
//! The LLM researches keywords per country and writes the copy; a deterministic validator owns
//! the hard limits (Apple: name 30, subtitle 30, keywords 100 bytes, promo 170, description 4000;
//! Play: title 30, short 80, full 4000, changelog 500) and keyword hygiene. Nothing that fails it is written.
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glot_llm::{provider::Provider, store, translate::Usage};

use crate::print_json;
use crate::style::*;

#[derive(Args)]
pub struct StoreArgs {
    #[command(subcommand)]
    cmd: StoreCmd,
    /// fastlane metadata directory.
    #[arg(long, global = true, default_value = "fastlane/metadata")]
    dir: PathBuf,
    /// Source listing locale (store code).
    #[arg(long, global = true, default_value = "en-US")]
    source: String,
    /// LLM provider for this run (`mock`: offline, nothing sent anywhere).
    #[arg(long, global = true, value_parser = ["mock", "anthropic", "openai", "openai-compatible"])]
    provider: Option<String>,
}

#[derive(Subcommand)]
enum StoreCmd {
    /// Draft the source listing from the app (Info.plist name, UI strings). Never overwrites a file.
    Init {
        /// Also create the Google Play listing (default: when android/ exists).
        #[arg(long)]
        android: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Localize the listing per locale: keyword research per country, copy, validated limits.
    Generate {
        /// Locales (glot, Apple or Play codes). Default: glot.toml targets, else the app's .lproj.
        #[arg(long, value_delimiter = ',')]
        locales: Vec<String>,
        /// Competitor brands never used as keywords.
        #[arg(long, value_delimiter = ',')]
        competitors: Vec<String>,
        /// Regenerate fields that already have content.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// User-facing release notes per locale from CHANGELOG.md or the git log since the last tag.
    Notes {
        /// Git revision to start from (default: last tag).
        #[arg(long)]
        since: Option<String>,
        /// Changelog file (default: CHANGELOG.md when present; its first `## ` section).
        #[arg(long)]
        changelog: Option<PathBuf>,
        #[arg(long, value_delimiter = ',')]
        locales: Vec<String>,
        /// Play `changelogs/<version_code>.txt` (default: `default.txt`).
        #[arg(long)]
        version_code: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Validate every listing on disk: limits, keyword hygiene, keyword field fill. Exit 1 on errors.
    Check {
        #[arg(long, value_delimiter = ',')]
        competitors: Vec<String>,
        #[arg(long)]
        json: bool,
    },
}

// ---------------------------------------------------------------- limits and locales

struct Field {
    name: &'static str,
    max: usize,
    /// Apple counts the keywords field in UTF-8 bytes, everything else in characters.
    bytes: bool,
    required: bool,
}

const fn f(name: &'static str, max: usize, bytes: bool, required: bool) -> Field {
    Field {
        name,
        max,
        bytes,
        required,
    }
}

const IOS: &[Field] = &[
    f("name", 30, false, true),
    f("subtitle", 30, false, false),
    f("keywords", 100, true, false),
    f("promotional_text", 170, false, false),
    f("description", 4000, false, true),
    f("release_notes", 4000, false, false),
];
const ANDROID: &[Field] = &[
    f("title", 30, false, true),
    f("short_description", 80, false, true),
    f("full_description", 4000, false, true),
];
const KEYWORDS_MAX: usize = 100;
const PLAY_CHANGELOG_MAX: usize = 500;

fn len(field: &Field, s: &str) -> usize {
    if field.bytes {
        s.len()
    } else {
        s.chars().count()
    }
}

/// (App Store Connect code, Google Play code, aliases): glot/BCP 47, `.lproj` and both stores' codes.
const LOCALES: &[(&str, &str, &[&str])] = &[
    ("ar-SA", "ar", &["ar"]),
    ("ca", "ca", &[]),
    ("cs", "cs-CZ", &[]),
    ("da", "da-DK", &[]),
    ("de-DE", "de-DE", &["de", "de-at", "de-ch"]),
    ("el", "el-GR", &[]),
    ("en-US", "en-US", &["en"]),
    ("en-GB", "en-GB", &[]),
    ("en-AU", "en-AU", &[]),
    ("en-CA", "en-CA", &[]),
    ("es-ES", "es-ES", &["es"]),
    ("es-MX", "es-419", &["es-us", "es-ar", "es-co", "es-cl"]),
    ("fi", "fi-FI", &[]),
    ("fr-FR", "fr-FR", &["fr", "fr-be", "fr-ch"]),
    ("fr-CA", "fr-CA", &[]),
    ("he", "iw-IL", &["he-il", "iw"]),
    ("hi", "hi-IN", &[]),
    ("hr", "hr", &[]),
    ("hu", "hu-HU", &[]),
    ("id", "id", &["in", "id-id"]),
    ("it", "it-IT", &[]),
    ("ja", "ja-JP", &[]),
    ("ko", "ko-KR", &[]),
    ("ms", "ms", &["ms-my"]),
    ("nl-NL", "nl-NL", &["nl"]),
    ("no", "no-NO", &["nb", "nb-no"]),
    ("pl", "pl-PL", &[]),
    // ponytail: bare `pt` goes to Brazil (the larger store); pass pt-PT for Portugal.
    ("pt-BR", "pt-BR", &["pt"]),
    ("pt-PT", "pt-PT", &[]),
    ("ro", "ro", &[]),
    ("ru", "ru-RU", &[]),
    ("sk", "sk", &[]),
    ("sv", "sv-SE", &[]),
    ("th", "th", &[]),
    ("tr", "tr-TR", &[]),
    ("uk", "uk", &["uk-ua"]),
    ("vi", "vi", &[]),
    ("zh-Hans", "zh-CN", &["zh", "zh-sg", "zh-hans-cn"]),
    ("zh-Hant", "zh-TW", &["zh-hk", "zh-hant-tw", "zh-hant-hk"]),
];

/// (App Store code, Play code) of any spelling: `fr`, `fr_FR`, `fr-FR`, `zh-Hans`, `es-419`, `ja-JP`…
fn codes(locale: &str) -> Option<(&'static str, &'static str)> {
    let l = locale.trim().replace('_', "-").to_lowercase();
    LOCALES
        .iter()
        .find(|(a, p, alias)| {
            a.to_lowercase() == l || p.to_lowercase() == l || alias.contains(&l.as_str())
        })
        .map(|(a, p, _)| (*a, *p))
}

// ---------------------------------------------------------------- keyword hygiene

/// Lowercased words (letters/digits runs). A CJK term without spaces stays one word.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Singular form for duplicate detection.
/// ponytail: English/Romance `-s`/`-ies` rule only; per-language stemmer if German/Slavic plurals slip through.
fn norm(w: &str) -> String {
    let w = w.to_lowercase();
    let n = w.chars().count();
    if n > 4 && w.ends_with("ies") {
        format!("{}y", &w[..w.len() - 3])
    } else if n > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_string()
    } else {
        w
    }
}

/// What the store already indexes (name + subtitle) and the brands to avoid.
struct Hygiene {
    indexed_text: String,
    indexed: BTreeSet<String>,
    brands: Vec<String>,
}

impl Hygiene {
    fn new(name: &str, subtitle: &str, brands: &[String]) -> Self {
        let text = format!("{name} {subtitle}");
        Self {
            indexed: words(&text).iter().map(|w| norm(w)).collect(),
            indexed_text: text.to_lowercase(),
            brands: brands
                .iter()
                .map(|b| b.trim().to_lowercase())
                .filter(|b| !b.is_empty())
                .collect(),
        }
    }

    /// Why `term` must not be in the keywords field, given the terms `kept` before it.
    fn reject(&self, term: &str, kept: &BTreeSet<String>) -> Option<String> {
        let t = term.trim().to_lowercase();
        if t.is_empty() {
            return Some("empty".into());
        }
        if let Some(b) = self
            .brands
            .iter()
            .find(|b| t == **b || t.contains(b.as_str()))
        {
            return Some(format!("competitor brand ({b})"));
        }
        let ws = words(&t);
        if ws.iter().all(|w| self.indexed.contains(&norm(w)))
            || (!t.is_ascii() && self.indexed_text.contains(&t))
        {
            return Some("already indexed from name/subtitle".into());
        }
        if kept.contains(&norm(&t)) {
            return Some("duplicate or plural of an earlier keyword".into());
        }
        None
    }
}

#[derive(Debug, Default, Serialize)]
struct Packed {
    /// `a,b,c`, at most 100 bytes.
    field: String,
    kept: Vec<(String, String)>,
    dropped: Vec<(String, String)>,
}

/// Greedy fill in relevance order: phrases are split into words (Apple combines words across
/// name, subtitle and keywords itself), rejected words are reported with their reason.
fn pack(candidates: &[(String, String)], h: &Hygiene) -> Packed {
    let mut p = Packed::default();
    let mut seen = BTreeSet::new();
    for (term, why) in candidates {
        let t = term.trim().to_lowercase().replace(',', " ");
        let parts: Vec<String> = if t.contains(char::is_whitespace) {
            t.split_whitespace().map(String::from).collect()
        } else {
            vec![t]
        };
        for w in parts {
            if p.dropped.iter().any(|(d, _)| *d == w) {
                continue;
            }
            if let Some(r) = h.reject(&w, &seen) {
                if r != "empty" {
                    p.dropped.push((w, r));
                }
                continue;
            }
            let after = p.field.len() + usize::from(!p.field.is_empty()) + w.len();
            if after > KEYWORDS_MAX {
                p.dropped
                    .push((w, format!("no room ({after}/{KEYWORDS_MAX} bytes)")));
                continue;
            }
            if !p.field.is_empty() {
                p.field.push(',');
            }
            p.field.push_str(&w);
            seen.insert(norm(&w));
            p.kept.push((w, why.clone()));
        }
    }
    p
}

// ---------------------------------------------------------------- validator

#[derive(Debug, Clone, Serialize)]
struct Issue {
    locale: String,
    platform: &'static str,
    field: String,
    error: bool,
    message: String,
}

type Fields = BTreeMap<String, String>;

fn spec(android: bool) -> &'static [Field] {
    if android { ANDROID } else { IOS }
}

/// Every hard limit and keyword rule of one listing. `pending`: fields not on disk yet are fine.
fn validate(locale: &str, android: bool, fields: &Fields, brands: &[String]) -> Vec<Issue> {
    let platform = if android { "play" } else { "app_store" };
    let mut out = vec![];
    let mut push = |field: &str, error: bool, message: String| {
        out.push(Issue {
            locale: locale.into(),
            platform,
            field: field.into(),
            error,
            message,
        })
    };
    for f in spec(android) {
        let v = fields.get(f.name).map(String::as_str).unwrap_or_default();
        if v.trim().is_empty() {
            if f.required {
                push(f.name, true, "missing (required by the store)".into());
            }
            continue;
        }
        let n = len(f, v);
        if n > f.max {
            let unit = if f.bytes { "bytes" } else { "characters" };
            push(f.name, true, format!("{n}/{} {unit}", f.max));
        }
    }
    if android {
        for (k, v) in fields.iter().filter(|(k, _)| k.starts_with("changelogs/")) {
            let n = v.chars().count();
            if n > PLAY_CHANGELOG_MAX {
                push(k, true, format!("{n}/{PLAY_CHANGELOG_MAX} characters"));
            }
        }
        return out;
    }
    let kw = fields
        .get("keywords")
        .map(String::as_str)
        .unwrap_or_default();
    if kw.is_empty() {
        push(
            "keywords",
            false,
            "empty: the field the store searches most".into(),
        );
        return out;
    }
    if kw.split(',').any(|t| t != t.trim()) {
        push(
            "keywords",
            true,
            "spaces around commas waste bytes (`a,b`, not `a, b`)".into(),
        );
    }
    let h = Hygiene::new(
        fields.get("name").map(String::as_str).unwrap_or_default(),
        fields
            .get("subtitle")
            .map(String::as_str)
            .unwrap_or_default(),
        brands,
    );
    let mut seen = BTreeSet::new();
    for t in kw.split(',') {
        if let Some(r) = h.reject(t, &seen) {
            push("keywords", true, format!("`{}`: {r}", t.trim()));
        }
        seen.insert(norm(t.trim()));
    }
    if kw.len() < 90 && kw.len() <= KEYWORDS_MAX {
        push(
            "keywords",
            false,
            format!("{} bytes unused", KEYWORDS_MAX - kw.len()),
        );
    }
    out
}

// ---------------------------------------------------------------- files

fn listing_dir(dir: &Path, android: bool, code: &str) -> PathBuf {
    if android {
        dir.join("android").join(code)
    } else {
        dir.join(code)
    }
}

fn read_fields(path: &Path, android: bool) -> Fields {
    let mut out: Fields = spec(android)
        .iter()
        .filter_map(|f| {
            let v = std::fs::read_to_string(path.join(format!("{}.txt", f.name))).ok()?;
            Some((f.name.to_string(), v.trim().to_string()))
        })
        .collect();
    if android && let Ok(rd) = std::fs::read_dir(path.join("changelogs")) {
        for e in rd.flatten() {
            if let Ok(v) = std::fs::read_to_string(e.path()) {
                out.insert(
                    format!("changelogs/{}", e.file_name().to_string_lossy()),
                    v.trim().to_string(),
                );
            }
        }
    }
    out
}

fn write_field(path: &Path, name: &str, value: &str) -> Result<()> {
    let p = path.join(format!("{name}.txt"));
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, value).with_context(|| format!("writing {}", p.display()))
}

/// Directories under `root` (depth ≤ 4), skipping build output, dependencies and VCS.
fn walk(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            || [
                "build",
                "DerivedData",
                "Pods",
                "node_modules",
                "fastlane",
                "target",
            ]
            .contains(&name.as_str())
        {
            continue;
        }
        out.push(p.clone());
        if depth > 0 && p.is_dir() {
            walk(&p, depth - 1, out);
        }
    }
}

/// App display name and category from the first Info.plist that has one.
fn plist_info(paths: &[PathBuf]) -> (Option<String>, Option<String>) {
    let mut plists: Vec<&PathBuf> = paths
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "Info.plist"))
        .collect();
    plists.sort_by_key(|p| p.components().count());
    for p in plists {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let Ok(doc) = roxmltree::Document::parse_with_options(
            &text,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        ) else {
            continue;
        };
        let get = |key: &str| {
            let k = doc
                .descendants()
                .find(|n| n.has_tag_name("key") && n.text() == Some(key))?;
            let v = k.next_sibling_element()?.text()?.trim();
            (!v.is_empty() && !v.starts_with("$(")).then(|| v.to_string())
        };
        if let Some(name) = get("CFBundleDisplayName").or_else(|| get("CFBundleName")) {
            return (Some(name), get("LSApplicationCategoryType"));
        }
    }
    (None, None)
}

/// Source UI strings (en/Base `.lproj`, `.xcstrings`, Android `values/strings.xml`), code-free ones only.
fn app_strings(paths: &[PathBuf]) -> Vec<String> {
    use glot_core::catalog::Format;
    let mut out = vec![];
    for p in paths {
        let s = p.to_string_lossy();
        let source = s.ends_with("en.lproj/Localizable.strings")
            || s.ends_with("Base.lproj/Localizable.strings")
            || s.ends_with(".xcstrings")
            || s.ends_with("res/values/strings.xml");
        let Ok(Some(parsed)) = source
            .then(|| glot_core::formats::read(p, "en", Format::Auto))
            .transpose()
            .map(Option::flatten)
        else {
            continue;
        };
        for v in parsed.entries.into_values() {
            let v = v.trim().to_string();
            let n = v.chars().count();
            if (4..=300).contains(&n)
                && v.chars().filter(|c| c.is_alphabetic()).count() * 2 > n
                && !v.contains(['\\', '%', '{', '$'])
                && !out.contains(&v)
            {
                out.push(v);
            }
        }
    }
    out.truncate(300);
    out
}

// ---------------------------------------------------------------- LLM

fn provider(p: &Option<String>) -> Result<(Box<dyn Provider>, String)> {
    let mut cfg = glot_llm::engine::Workspace::open(&std::env::current_dir()?)
        .map(|w| w.config.llm)
        .unwrap_or_default();
    if let Some(p) = p {
        cfg.provider = p.clone();
    }
    Ok((glot_llm::provider::from_config(&cfg, None)?, cfg.model))
}

/// Ask, then ask once more with the violated `limits` (deterministic count) for those fields only.
async fn ask_valid(
    p: &dyn Provider,
    model: &str,
    mut payload: Value,
    usage: &mut Usage,
) -> Result<Value> {
    let over = |v: &Value| -> Vec<Value> {
        payload["limits"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, max)| {
                let n = v[k].as_str()?.chars().count();
                let max = max.as_u64()? as usize;
                (n > max).then(|| json!({"field": k, "max": max, "length": n}))
            })
            .collect()
    };
    let (mut v, u) = store::ask(p, model, &payload).await?;
    usage.add(&u);
    let violations = over(&v);
    if !violations.is_empty() {
        payload["violations"] = json!(violations);
        payload["previous"] = v.clone();
        let (v2, u) = store::ask(p, model, &payload).await?;
        usage.add(&u);
        for x in &violations {
            let k = x["field"].as_str().unwrap_or_default();
            if let Some(s) = v2[k].as_str() {
                v[k] = json!(s);
            }
        }
    }
    Ok(v)
}

fn keyword_candidates(v: &Value) -> Vec<(String, String)> {
    v["keywords"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| match k {
            Value::String(s) => Some((s.clone(), String::new())),
            _ => Some((
                k["term"].as_str()?.to_string(),
                k["why"].as_str().unwrap_or_default().to_string(),
            )),
        })
        .collect()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default().trim()
}

const LISTING_LIMITS: &str = r#"{"name": 30, "subtitle": 30, "promotional_text": 170, "description": 4000, "short_description": 80}"#;

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
struct LocaleReport {
    locale: String,
    keywords_bytes: usize,
    keywords: Vec<(String, String)>,
    dropped: Vec<(String, String)>,
    issues: Vec<Issue>,
    written: Vec<String>,
}

pub async fn run(a: StoreArgs) -> Result<ExitCode> {
    let root = std::env::current_dir()?;
    let dir = root.join(&a.dir);
    let (src_apple, src_play) =
        codes(&a.source).with_context(|| format!("unknown store locale {}", a.source))?;
    match a.cmd {
        StoreCmd::Init { android, dry_run } => {
            init(
                &root,
                &dir,
                src_apple,
                src_play,
                android,
                dry_run,
                &a.provider,
            )
            .await
        }
        StoreCmd::Check { competitors, json } => check(&dir, &competitors, json),
        StoreCmd::Generate {
            locales,
            competitors,
            force,
            dry_run,
            json,
        } => {
            let targets = targets(&root, &dir, src_apple, &locales)?;
            let g = Gen {
                dir,
                src: (src_apple, src_play),
                competitors,
                force,
                dry_run,
            };
            generate(&g, &targets, json, &a.provider).await
        }
        StoreCmd::Notes {
            since,
            changelog,
            locales,
            version_code,
            dry_run,
            json,
        } => {
            let changes = changes(&root, since.as_deref(), changelog.as_deref())?;
            let mut all = vec![(src_apple, src_play)];
            all.extend(targets(&root, &dir, src_apple, &locales)?);
            let file = format!(
                "changelogs/{}",
                version_code.as_deref().unwrap_or("default")
            );
            notes(&dir, &all, &changes, &file, dry_run, json, &a.provider).await
        }
    }
}

/// Which stores have a source listing.
fn platforms(dir: &Path, src: (&str, &str)) -> (bool, bool) {
    (
        listing_dir(dir, false, src.0).is_dir(),
        listing_dir(dir, true, src.1).is_dir(),
    )
}

fn targets(
    root: &Path,
    dir: &Path,
    src: &str,
    explicit: &[String],
) -> Result<Vec<(&'static str, &'static str)>> {
    let mut raw: Vec<String> = explicit.to_vec();
    if raw.is_empty()
        && let Ok(ws) = glot_llm::engine::Workspace::open(root)
    {
        raw = ws.config.target_locales.clone();
    }
    if raw.is_empty() {
        let mut paths = vec![];
        walk(root, 3, &mut paths);
        raw = paths
            .iter()
            .filter_map(|p| {
                p.file_name()?
                    .to_str()?
                    .strip_suffix(".lproj")
                    .map(String::from)
            })
            .filter(|l| l != "Base")
            .collect();
    }
    if raw.is_empty() {
        raw = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|l| codes(l).is_some())
            .collect();
    }
    let mut out: Vec<(&str, &str)> = vec![];
    for l in &raw {
        let c = codes(l).with_context(|| format!("no App Store / Play locale for `{l}`"))?;
        if c.0 != src && !out.contains(&c) {
            out.push(c);
        }
    }
    Ok(out)
}

async fn init(
    root: &Path,
    dir: &Path,
    apple: &str,
    play: &str,
    android: bool,
    dry_run: bool,
    prov: &Option<String>,
) -> Result<ExitCode> {
    let mut paths = vec![];
    walk(root, 4, &mut paths);
    let (name, category) = plist_info(&paths);
    let name = name.unwrap_or_else(|| {
        root.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let strings = app_strings(&paths);
    let android = android || root.join("android").is_dir();
    let (p, model) = provider(prov)?;
    let mut usage = Usage::default();
    let payload = json!({
        "task": "store_draft", "locale": apple, "app_name": name, "category": category,
        "app_strings": strings, "limits": serde_json::from_str::<Value>(LISTING_LIMITS)?,
    });
    let v = ask_valid(p.as_ref(), &model, payload, &mut usage).await?;
    let packed = pack(
        &keyword_candidates(&v),
        &Hygiene::new(&name, s(&v, "subtitle"), &[]),
    );
    let mut lists: Vec<(bool, &str, Fields)> = vec![(
        false,
        apple,
        [
            ("name", name.as_str()),
            ("subtitle", s(&v, "subtitle")),
            ("keywords", packed.field.as_str()),
            ("promotional_text", s(&v, "promotional_text")),
            ("description", s(&v, "description")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    )];
    if android {
        lists.push((
            true,
            play,
            [
                ("title", name.as_str()),
                ("short_description", s(&v, "short_description")),
                ("full_description", s(&v, "description")),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ));
    }
    anstream::println!(
        "{BOLD}{name}{BOLD:#} {DIM}({} UI strings read, category {}){DIM:#}",
        strings.len(),
        category.as_deref().unwrap_or("?")
    );
    let mut errors = false;
    for (is_android, code, fields) in &lists {
        let path = listing_dir(dir, *is_android, code);
        let existing = read_fields(&path, *is_android);
        for (k, v) in fields {
            let rel = path.join(format!("{k}.txt"));
            let rel = rel.strip_prefix(root).unwrap_or(&rel).display();
            if existing.get(k).is_some_and(|e| !e.is_empty()) {
                anstream::println!("  {DIM}exists{DIM:#}  {rel}");
            } else if v.is_empty() {
                anstream::println!(
                    "  {YELLOW}empty{YELLOW:#}   {rel} {DIM}(write it by hand){DIM:#}"
                );
            } else {
                if !dry_run {
                    write_field(&path, k, v)?;
                }
                let verb = if dry_run { "would create" } else { "created" };
                anstream::println!("  {GREEN}{verb}{GREEN:#} {rel}");
            }
        }
        let on_disk = if dry_run {
            let mut m = fields.clone();
            m.extend(existing.into_iter().filter(|(_, v)| !v.is_empty()));
            m
        } else {
            read_fields(&path, *is_android)
        };
        let issues = validate(code, *is_android, &on_disk, &[]);
        errors |= issues.iter().any(|i| i.error);
        print_issues(&issues);
    }
    print_keywords(&packed);
    anstream::println!(
        "{DIM}draft: review it before `glot labs store generate` · LLM cost ${:.4}{DIM:#}",
        usage.cost_usd
    );
    Ok(if errors {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

struct Gen {
    dir: PathBuf,
    src: (&'static str, &'static str),
    competitors: Vec<String>,
    force: bool,
    dry_run: bool,
}

fn source_listing(dir: &Path, src: (&str, &str)) -> Result<Value> {
    let (ios, android) = platforms(dir, src);
    if !ios && !android {
        bail!(
            "no source listing in {}/{}: run `glot labs store init`",
            dir.display(),
            src.0
        );
    }
    let i = read_fields(&listing_dir(dir, false, src.0), false);
    let a = read_fields(&listing_dir(dir, true, src.1), true);
    let g = |m: &Fields, k: &str| m.get(k).cloned().unwrap_or_default();
    let or = |x: String, y: String| if x.is_empty() { y } else { x };
    Ok(json!({
        "name": or(g(&i, "name"), g(&a, "title")),
        "subtitle": g(&i, "subtitle"),
        "keywords": g(&i, "keywords"),
        "promotional_text": g(&i, "promotional_text"),
        "description": or(g(&i, "description"), g(&a, "full_description")),
        "short_description": or(g(&a, "short_description"), g(&i, "subtitle")),
    }))
}

async fn generate(
    g: &Gen,
    targets: &[(&str, &str)],
    json_out: bool,
    prov: &Option<String>,
) -> Result<ExitCode> {
    let source = source_listing(&g.dir, g.src)?;
    let (ios, android) = platforms(&g.dir, g.src);
    let (p, model) = provider(prov)?;
    let mut usage = Usage::default();
    let mut reports = vec![];
    for &(apple, play) in targets {
        let ios_path = listing_dir(&g.dir, false, apple);
        let and_path = listing_dir(&g.dir, true, play);
        let (old_i, old_a) = (read_fields(&ios_path, false), read_fields(&and_path, true));
        let has = |m: &Fields, k: &str| m.get(k).is_some_and(|v| !v.is_empty());
        let wanted_i = [
            "name",
            "subtitle",
            "keywords",
            "promotional_text",
            "description",
        ]
        .into_iter()
        .filter(|k| ios && (!s(&source, k).is_empty() || *k == "keywords"));
        let wanted_a = ["title", "short_description", "full_description"]
            .into_iter()
            .filter(|_| android);
        let missing =
            wanted_i.clone().any(|k| !has(&old_i, k)) || wanted_a.clone().any(|k| !has(&old_a, k));
        if !g.force && !missing {
            reports.push(LocaleReport {
                locale: apple.into(),
                keywords_bytes: old_i.get("keywords").map_or(0, String::len),
                keywords: vec![],
                dropped: vec![],
                issues: [
                    (ios, validate(apple, false, &old_i, &g.competitors)),
                    (android, validate(play, true, &old_a, &g.competitors)),
                ]
                .into_iter()
                .filter(|(on, _)| *on)
                .flat_map(|(_, i)| i)
                .collect(),
                written: vec![],
            });
            continue;
        }
        let mut limits: Value = serde_json::from_str(LISTING_LIMITS)?;
        limits["keywords_bytes"] = json!(KEYWORDS_MAX);
        let stores: Vec<&str> = [ios.then_some("app_store"), android.then_some("google_play")]
            .into_iter()
            .flatten()
            .collect();
        let payload = json!({
            "task": "store_listing", "source_locale": g.src.0, "target_locale": apple,
            "stores": stores,
            "source": source, "limits": limits, "avoid_brands": g.competitors,
        });
        let v = ask_valid(p.as_ref(), &model, payload, &mut usage).await?;
        let keep = |old: &Fields, k: &str, new: &str| -> String {
            match old.get(k) {
                Some(o) if !o.is_empty() && !g.force => o.clone(),
                _ => new.to_string(),
            }
        };
        let name = keep(&old_i, "name", s(&v, "name"));
        let subtitle = keep(&old_i, "subtitle", s(&v, "subtitle"));
        let packed = pack(
            &keyword_candidates(&v),
            &Hygiene::new(&name, &subtitle, &g.competitors),
        );
        let mut new_i = Fields::new();
        for k in wanted_i {
            let fresh = match k {
                "keywords" => packed.field.as_str(),
                "name" => name.as_str(),
                "subtitle" => subtitle.as_str(),
                _ => s(&v, k),
            };
            new_i.insert(k.into(), keep(&old_i, k, fresh));
        }
        let mut new_a = Fields::new();
        for k in wanted_a {
            let fresh = match k {
                "title" => name.as_str(),
                "short_description" => s(&v, "short_description"),
                _ => s(&v, "description"),
            };
            new_a.insert(k.into(), keep(&old_a, k, fresh));
        }
        let mut issues = vec![];
        let mut written = vec![];
        for (is_android, path, old, new, code) in [
            (false, &ios_path, &old_i, new_i, apple),
            (true, &and_path, &old_a, new_a, play),
        ] {
            if new.is_empty() {
                continue;
            }
            let mut merged = old.clone();
            merged.extend(new.clone());
            let found = validate(code, is_android, &merged, &g.competitors);
            for (k, val) in &new {
                let bad = found.iter().any(|i| i.error && i.field == *k);
                if bad || val.is_empty() || old.get(k) == Some(val) {
                    continue; // ponytail: an invalid field is never written; the old file stays.
                }
                if !g.dry_run {
                    write_field(path, k, val)?;
                }
                written.push(format!(
                    "{}{code}/{k}",
                    if is_android { "android/" } else { "" }
                ));
            }
            issues.extend(found);
        }
        reports.push(LocaleReport {
            locale: apple.into(),
            keywords_bytes: packed.field.len(),
            keywords: packed.kept,
            dropped: packed.dropped,
            issues,
            written,
        });
    }
    let errors = reports.iter().flat_map(|r| &r.issues).any(|i| i.error);
    if json_out {
        print_json(
            &json!({ "locales": reports, "cost_usd": usage.cost_usd, "dry_run": g.dry_run }),
        )?;
    } else {
        for r in &reports {
            let fill = if ios {
                format!("keywords {}/{KEYWORDS_MAX} bytes", r.keywords_bytes)
            } else {
                String::new()
            };
            let what = if r.written.is_empty() && r.keywords.is_empty() {
                "up to date (--force to regenerate)".to_string()
            } else if g.dry_run {
                format!("{} fields would be written", r.written.len())
            } else {
                format!("{} fields written", r.written.len())
            };
            anstream::println!("{BOLD}{:<8}{BOLD:#} {fill}  {DIM}{what}{DIM:#}", r.locale);
            if !r.keywords.is_empty() {
                print_keywords(&Packed {
                    field: String::new(),
                    kept: r.keywords.clone(),
                    dropped: r.dropped.clone(),
                });
            }
            print_issues(&r.issues);
        }
        anstream::eprintln!("{DIM}LLM cost ${:.4}{DIM:#}", usage.cost_usd);
    }
    Ok(if errors {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn print_keywords(p: &Packed) {
    for (i, (w, why)) in p.kept.iter().enumerate() {
        anstream::println!("    {}. {w} {DIM}{why}{DIM:#}", i + 1);
    }
    if !p.dropped.is_empty() {
        let (room, rest): (Vec<_>, Vec<_>) = p
            .dropped
            .iter()
            .partition(|(_, r)| r.starts_with("no room"));
        let mut d: Vec<String> = rest.iter().map(|(w, r)| format!("{w} ({r})")).collect();
        if !room.is_empty() {
            d.push(format!("{} more that did not fit", room.len()));
        }
        anstream::println!("    {DIM}dropped: {}{DIM:#}", d.join(", "));
    }
}

fn print_issues(issues: &[Issue]) {
    for i in issues {
        let (c, tag) = if i.error {
            (RED, "error")
        } else {
            (YELLOW, "warning")
        };
        anstream::println!(
            "    {c}{tag}{c:#} {} {}/{}: {}",
            i.platform,
            i.locale,
            i.field,
            i.message
        );
    }
}

fn check(dir: &Path, brands: &[String], json_out: bool) -> Result<ExitCode> {
    let mut rows = vec![];
    for android in [false, true] {
        let base = if android {
            dir.join("android")
        } else {
            dir.to_path_buf()
        };
        let mut locs: Vec<String> = std::fs::read_dir(&base)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|l| codes(l).is_some())
            .collect();
        locs.sort();
        for l in locs {
            let fields = read_fields(&base.join(&l), android);
            let issues = validate(&l, android, &fields, brands);
            let kw = fields.get("keywords").map_or(0, String::len);
            rows.push((l, android, kw, issues));
        }
    }
    if rows.is_empty() {
        bail!(
            "no listing in {}: run `glot labs store init`",
            dir.display()
        );
    }
    let errors = rows.iter().flat_map(|r| &r.3).filter(|i| i.error).count();
    if json_out {
        let v: Vec<Value> = rows
            .iter()
            .map(|(l, a, kw, i)| json!({"locale": l, "store": if *a {"play"} else {"app_store"}, "keywords_bytes": kw, "issues": i}))
            .collect();
        print_json(&json!({ "listings": v, "errors": errors }))?;
    } else {
        for (l, android, kw, issues) in &rows {
            let label = if *android {
                format!("play {l}")
            } else {
                format!("{l:<8} keywords {kw}/{KEYWORDS_MAX} bytes")
            };
            anstream::println!("{BOLD}{label}{BOLD:#}");
            print_issues(issues);
        }
        anstream::println!("{} listings, {errors} errors", rows.len());
    }
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

// ---------------------------------------------------------------- release notes

/// User-facing changes: the first `## ` section of the changelog, else commit subjects since
/// `since` / the last tag, without chores (conventional-commit prefixes stripped).
fn changes(root: &Path, since: Option<&str>, changelog: Option<&Path>) -> Result<Vec<String>> {
    let cl = changelog
        .map(|p| root.join(p))
        .or_else(|| Some(root.join("CHANGELOG.md")).filter(|p| p.is_file()));
    let lines: Vec<String> = if let Some(p) = cl {
        let text = std::fs::read_to_string(&p).with_context(|| format!("{}", p.display()))?;
        changelog_section(&text)
    } else {
        let git = |args: &[&str]| -> Option<String> {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .ok()?;
            o.status
                .success()
                .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        let from = since
            .map(String::from)
            .or_else(|| git(&["describe", "--tags", "--abbrev=0"]));
        let range = from.map_or("HEAD".to_string(), |f| format!("{f}..HEAD"));
        git(&["log", "--no-merges", "-n", "50", "--pretty=%s", &range])
            .context("no CHANGELOG.md and no git history: pass --changelog")?
            .lines()
            .map(String::from)
            .collect()
    };
    let out = user_facing(&lines);
    if out.is_empty() {
        bail!("no user-facing change found (chores, CI, docs and merges are skipped)");
    }
    Ok(out)
}

fn changelog_section(text: &str) -> Vec<String> {
    let mut sections: Vec<Vec<String>> = vec![];
    for l in text.lines() {
        if l.starts_with("## ") {
            sections.push(vec![]);
        } else if let Some(cur) = sections.last_mut() {
            let t = l.trim().trim_start_matches(['-', '*', '+']).trim();
            if !t.is_empty() && !t.starts_with('#') {
                cur.push(t.to_string());
            }
        }
    }
    sections
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

fn user_facing(lines: &[String]) -> Vec<String> {
    const SKIP: &[&str] = &[
        "chore",
        "ci",
        "test",
        "build",
        "style",
        "docs",
        "refactor",
        "bump",
        "merge",
        "wip",
        "initial commit",
        "release",
    ];
    lines
        .iter()
        .filter_map(|l| {
            let t = l.trim();
            let lower = t.to_lowercase();
            if t.is_empty() || SKIP.iter().any(|s| lower.starts_with(s)) {
                return None;
            }
            // `feat(scope)!: x` -> `x`
            let body = match t.split_once(": ") {
                Some((head, rest))
                    if !head.contains(' ')
                        && head.chars().next().is_some_and(char::is_lowercase) =>
                {
                    rest
                }
                _ => t,
            };
            Some(body.to_string())
        })
        .collect()
}

async fn notes(
    dir: &Path,
    locales: &[(&str, &str)],
    changes: &[String],
    play_file: &str,
    dry_run: bool,
    json_out: bool,
    prov: &Option<String>,
) -> Result<ExitCode> {
    let (ios, android) = platforms(dir, locales[0]);
    if !ios && !android {
        bail!(
            "no source listing in {}: run `glot labs store init`",
            dir.display()
        );
    }
    let max = if android { PLAY_CHANGELOG_MAX } else { 4000 };
    let (p, model) = provider(prov)?;
    let mut usage = Usage::default();
    let mut out = vec![];
    let mut errors = 0;
    for &(apple, play) in locales {
        let payload = json!({
            "task": "store_release_notes", "target_locale": apple, "changes": changes,
            "max_chars": max, "limits": {"notes": max},
        });
        let v = ask_valid(p.as_ref(), &model, payload, &mut usage).await?;
        let text = s(&v, "notes").to_string();
        let n = text.chars().count();
        let ok = !text.is_empty() && n <= max;
        errors += usize::from(!ok);
        if ok && !dry_run {
            if ios {
                write_field(&listing_dir(dir, false, apple), "release_notes", &text)?;
            }
            if android {
                write_field(&listing_dir(dir, true, play), play_file, &text)?;
            }
        }
        out.push(json!({"locale": apple, "notes": text, "chars": n, "max": max, "ok": ok}));
    }
    if json_out {
        print_json(&json!({ "changes": changes, "locales": out, "cost_usd": usage.cost_usd }))?;
    } else {
        anstream::println!("{DIM}{} user-facing changes{DIM:#}", changes.len());
        for o in &out {
            let tag = if o["ok"] == true {
                format!("{GREEN}✓{GREEN:#}")
            } else {
                format!("{RED}✗ not written{RED:#}")
            };
            anstream::println!(
                "{BOLD}{}{BOLD:#} {tag} {DIM}{}/{max}{DIM:#}",
                o["locale"].as_str().unwrap_or_default(),
                o["chars"]
            );
            for l in o["notes"].as_str().unwrap_or_default().lines() {
                anstream::println!("    {l}");
            }
        }
        anstream::eprintln!("{DIM}LLM cost ${:.4}{DIM:#}", usage.cost_usd);
    }
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_codes() {
        assert_eq!(codes("fr"), Some(("fr-FR", "fr-FR")));
        assert_eq!(codes("pt_BR"), Some(("pt-BR", "pt-BR")));
        assert_eq!(codes("es-MX"), Some(("es-MX", "es-419")));
        assert_eq!(codes("es-419"), Some(("es-MX", "es-419")));
        assert_eq!(codes("zh-Hans"), Some(("zh-Hans", "zh-CN")));
        assert_eq!(codes("zh-TW"), Some(("zh-Hant", "zh-TW")));
        assert_eq!(codes("ja"), Some(("ja", "ja-JP")));
        assert_eq!(codes("he"), Some(("he", "iw-IL")));
        assert_eq!(codes("xx"), None);
    }

    #[test]
    fn packs_keywords_under_100_bytes_without_waste() {
        let c = |t: &str| (t.to_string(), String::new());
        let h = Hygiene::new("Orizn Visa", "Passport power", &["VisaHQ".into()]);
        let p = pack(
            &[
                c("travel visa"), // visa: in the name
                c("visas"),       // plural of the name word
                c("visahq"),      // competitor
                c("Countries"),
                c("country"), // plural pair
                c("embassy"),
                c("passports"), // subtitle word, plural
                c(&"x".repeat(95)),
                c("trip"),
            ],
            &h,
        );
        assert_eq!(p.field, "travel,countries,embassy,trip");
        let reasons: Vec<&str> = p.dropped.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(
            reasons,
            [
                "visa",
                "visas",
                "visahq",
                "country",
                "passports",
                &"x".repeat(95)
            ]
        );
        // The validator accepts what pack produced…
        let mut f: Fields = [
            ("name", "Orizn Visa"),
            ("subtitle", "Passport power"),
            ("description", "d"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        f.insert("keywords".into(), p.field.clone());
        let issues = validate("en-US", false, &f, &["VisaHQ".into()]);
        assert!(issues.iter().all(|i| !i.error), "{issues:?}");
        assert!(issues.iter().any(|i| i.message == "71 bytes unused"));
        // …and rejects a hand-written field that breaks every rule.
        f.insert("keywords".into(), "visa, trip,trips,VisaHQ".into());
        f.insert("subtitle".into(), "x".repeat(31));
        let errs: Vec<String> = validate("en-US", false, &f, &["VisaHQ".into()])
            .into_iter()
            .filter(|i| i.error)
            .map(|i| format!("{}: {}", i.field, i.message))
            .collect();
        assert_eq!(
            errs,
            [
                "subtitle: 31/30 characters",
                "keywords: spaces around commas waste bytes (`a,b`, not `a, b`)",
                "keywords: `visa`: already indexed from name/subtitle",
                "keywords: `trips`: duplicate or plural of an earlier keyword",
                "keywords: `VisaHQ`: competitor brand (visahq)",
            ]
        );
    }

    #[test]
    fn keyword_bytes_not_chars() {
        // 10 × 9 bytes (3 CJK chars) + 9 commas = 99 bytes: the 11th does not fit.
        let terms: Vec<(String, String)> = (0..11)
            .map(|i| {
                (
                    format!("ビザ{}", char::from_u32(0x30A2 + i * 2).unwrap()),
                    String::new(),
                )
            })
            .collect();
        let p = pack(&terms, &Hygiene::new("App", "", &[]));
        assert_eq!(p.field.len(), 99);
        assert_eq!(p.kept.len(), 10);
        assert!(p.dropped[0].1.starts_with("no room"));
    }

    #[test]
    fn release_note_sources() {
        let log: Vec<String> = [
            "feat(compare): compare two passports side by side",
            "fix: crash when opening the map offline",
            "chore: bump deps",
            "ci: cache",
            "Merge branch 'x'",
            "Improve onboarding copy",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            user_facing(&log),
            [
                "compare two passports side by side",
                "crash when opening the map offline",
                "Improve onboarding copy"
            ]
        );
        let cl = "# Changelog\n\n## [Unreleased]\n\n## 1.2.0\n- Dark mode\n- Faster map\n\n## 1.1.0\n- Old\n";
        assert_eq!(changelog_section(cl), ["Dark mode", "Faster map"]);
    }
}
