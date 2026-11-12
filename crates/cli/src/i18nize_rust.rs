//! `glot i18nize` on a Rust front (Leptos `view!`) with a hand-rolled `t(key)` (cliponit):
//! hardcoded copy whose type cannot change (a text node right between tags, a visible attribute of
//! an HTML element) becomes `{move || t("key")}` / `attr=move || t("key")` (`crate::i18n::t` when
//! `t` is not imported or a local shadows it) and its text a new row of the source string table.
//! Text inside expressions, component props and `html!` / `rsx!` copy are listed with the code to
//! write by hand: rewriting them could change a type.
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

use glot_extract::rust::{self as rx, Spot};
use glot_llm::engine::Workspace;

use super::{I18nizeArgs, print_diff};
use crate::style::*;

/// Rust string tables configured, or only Rust sources scanned (a Tauri app's JS front wins).
pub fn is_rust(ws: &Workspace) -> bool {
    let inc = &ws.config.extract.include;
    ws.config.files.iter().any(|f| f.ends_with(".rs"))
        || (inc.iter().any(|g| g.ends_with(".rs"))
            && !inc.iter().any(|g| {
                [".ts", ".tsx", ".js", ".jsx", ".vue", ".svelte", "}"]
                    .iter()
                    .any(|e| g.ends_with(e))
            }))
}

/// Top-level `use` statements of `src` (at column 0): (start, end past `;`).
fn uses(src: &str) -> Vec<(usize, usize)> {
    src.match_indices("use ")
        .filter(|(i, _)| {
            let ls = src[..*i].rfind('\n').map_or(0, |n| n + 1);
            matches!(&src[ls..*i], "" | "pub " | "pub(crate) ")
        })
        .filter_map(|(i, _)| Some((i, i + src[i..].find(';')? + 1)))
        .collect()
}

/// `t` is callable in `src`: defined there or imported from an `i18n` module.
fn has_t(src: &str) -> bool {
    src.contains("fn t(")
        || uses(src).iter().any(|&(a, b)| {
            let u = &src[a..b];
            u.contains("i18n")
                && u.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|w| w == "t")
        })
}

/// `crate::i18n` for the table file `web/src/i18n.rs`, when `page` is in the same crate.
fn module_of(def: &str, page: &str) -> Option<String> {
    let at = def.rfind("src/")?;
    if !page.starts_with(&def[..at + 4]) {
        return None;
    }
    let rest = def[at + 4..].strip_suffix(".rs")?;
    let rest = rest.strip_suffix("/mod").unwrap_or(rest);
    Some(match rest {
        "lib" | "main" => "crate".into(),
        r => format!("crate::{}", r.replace('/', "::")),
    })
}

/// A local named `t` (`let t = …`, `|t|`, `Some(t)`): a bare `t(…)` call would hit it.
fn shadows_t(src: &str) -> bool {
    let b = src.as_bytes();
    let id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    src.match_indices('t').any(|(i, _)| {
        let prev = i.checked_sub(1).map(|k| b[k]);
        let next = src[i + 1..].trim_start().bytes().next();
        !prev.is_some_and(|c| id(c) || c == b'.' || c == b':' || c == b'"' || c == b'\'')
            && !b.get(i + 1).is_some_and(|&c| id(c))
            && !matches!(next, Some(b'(' | b'!' | b'"'))
            && !uses(src).iter().any(|&(a, e)| (a..e).contains(&i))
    })
}

/// cliponit-style key (`login_title`) from the suggested `pages.login.bienvenue`.
fn snake(k: &str) -> String {
    k.replace(['.', '-'], "_")
}

/// Rewrite `src` with the clean `spots` (calls to `f`) and the edits that go with them.
fn rewrite(
    src: &str,
    f: &str,
    spots: &[(&Spot, String)],
    aux: &[(usize, usize, String)],
) -> String {
    let mut edits: Vec<(usize, usize, String)> = spots
        .iter()
        .map(|(s, key)| (s.start, s.end, s.replacement(f, key)))
        .chain(aux.iter().cloned())
        .collect();
    edits.sort_by_key(|e| e.0);
    let (mut out, mut pos) = (String::new(), 0);
    for (start, end, with) in edits {
        // Overlapping edits cannot happen (leaves of one tree); never apply one twice anyway.
        if start < pos {
            continue;
        }
        out += &src[pos..start];
        out += &with;
        pos = end;
    }
    out + &src[pos..]
}

/// A string glot did not move: where, why, and the code to write.
#[derive(serde::Serialize)]
pub struct Left {
    pub path: String,
    pub line: u32,
    pub text: String,
    pub reason: String,
    pub suggestion: String,
}

/// Machine-readable outcome of `glot i18nize` (`GLOT_REPORT=<file>`): the GitHub App builds the
/// pull-request description from it instead of scraping the log.
#[derive(serde::Serialize, Default)]
pub struct Report {
    pub files_scanned: usize,
    pub files_rewritten: usize,
    pub new_keys: usize,
    /// Translations found in the code and kept (`tr("fr", "en")`, `if en { … }`).
    pub imported_translations: usize,
    pub left: Vec<Left>,
    /// Files restored to their original because the rewritten version failed verification.
    pub reverted: Vec<(String, String)>,
}

impl Report {
    pub fn save(&self) {
        if let Some(path) = std::env::var_os("GLOT_REPORT")
            && let Ok(json) = serde_json::to_string(self)
        {
            let _ = std::fs::write(path, json);
        }
    }
}

struct FilePlan {
    path: String,
    src: String,
    /// `t` or its full path; `None`: no `t` reachable from this file.
    f: Option<String>,
    found: rx::Analysis,
}

pub fn run(mut ws: Workspace, args: &I18nizeArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let paths: Vec<String> = args
        .paths
        .iter()
        .map(|p| cwd.join(p).to_string_lossy().into_owned())
        .collect();
    let files: Vec<String> = glot_llm::i18nize::list_files(&ws, &paths)?
        .into_iter()
        .filter(|f| f.ends_with(".rs"))
        .collect();
    if files.is_empty() {
        anyhow::bail!(
            "no .rs files to scan: set [extract] include = [\"src/**/*.rs\"] in glot.toml (or pass paths)"
        );
    }
    // Inline translations (`if en { … }`, `tr("fr", "en")`) are read against the project locales.
    glot_extract::i18nize::set_locales(&ws.config.source_locale, &ws.config.target_locales);
    let read = |f: &str| std::fs::read_to_string(ws.root.join(f)).ok();
    // The table file defining `t` (`web/src/i18n.rs`).
    let def = ws
        .config
        .files
        .iter()
        .find(|f| f.ends_with(".rs") && read(f).is_some_and(|s| s.contains("pub fn t(")))
        .cloned();
    let inner = |path: &str| -> String {
        path.rfind("src/")
            .map_or(path, |i| &path[i + 4..])
            .to_string()
    };
    for path in &files {
        if let Some(src) = read(path) {
            rx::register_pairs(path, &src);
        }
    }
    let mut plans: Vec<FilePlan> = vec![];
    for path in &files {
        let Some(src) = read(path) else { continue };
        // Keys named from the path inside the crate (`pages/login.rs` => `login_…`).
        let found = rx::analyze(&inner(path), &src);
        if found.spots.is_empty() {
            continue;
        }
        // The imported `t`, else its full path (never shadowed by a local, no import to add).
        let module = def.as_deref().and_then(|d| module_of(d, path));
        let f = match &module {
            Some(m) if !has_t(&src) || shadows_t(&src) => Some(format!("{m}::t")),
            _ if has_t(&src) && !shadows_t(&src) => Some("t".to_string()),
            _ => None,
        };
        plans.push(FilePlan {
            path: path.clone(),
            src,
            f,
            found,
        });
    }
    // A message shown in several files is `common_…`, not named after the first file met.
    let mut seen_in: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for p in &plans {
        for s in &p.found.spots {
            seen_in.entry(&s.copy.text).or_default().insert(&p.path);
        }
    }
    let shared: BTreeSet<String> = seen_in
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(t, _)| t.to_string())
        .collect();
    // key => text, existing and new: the same name with the same text is the same key.
    let mut values: BTreeMap<String, String> = ws.source().entries.clone();
    let have: BTreeSet<String> = values.keys().cloned().collect();
    let mut name = |s: &Spot, path: &str| -> String {
        let base = if shared.contains(&s.copy.text) {
            snake(&glot_extract::rust::suggest_key(
                "",
                &s.copy.text,
                s.copy.ui_role.as_deref().unwrap_or("text"),
            ))
        } else {
            let _ = path;
            snake(&s.copy.suggested_key)
        };
        let key = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base}_{n}")))
            .find(|k| values.get(k).is_none_or(|v| v == &s.copy.text))
            .expect("a free key");
        values.insert(key.clone(), s.copy.text.clone());
        key
    };
    let mut report = Report {
        files_scanned: files.len(),
        ..Default::default()
    };
    struct NewKey {
        key: String,
        text: String,
        at: String,
        note: Option<String>,
        translations: BTreeMap<String, String>,
    }
    let mut add: Vec<NewKey> = vec![];
    let mut added: BTreeSet<String> = BTreeSet::new();
    let mut rewrites: Vec<(String, String, String)> = vec![];
    for p in &plans {
        let mut keyed: Vec<(&Spot, String)> = vec![];
        let mut left = |s: &Spot, key: &str, reason: String| {
            report.left.push(Left {
                path: p.path.clone(),
                line: s.copy.at.line,
                text: s.copy.text.clone(),
                reason,
                suggestion: s.replacement(p.f.as_deref().unwrap_or("t"), key),
            })
        };
        for s in &p.found.spots {
            let key = name(s, &p.path);
            match (&p.f, s.clean) {
                (Some(_), true) => keyed.push((s, key)),
                (None, _) => left(
                    s,
                    &key,
                    "no `t()` is reachable from this file: add the string table to the crate, or import its `t`".into(),
                ),
                (_, false) => left(
                    s,
                    &key,
                    s.copy
                        .reason
                        .clone()
                        .unwrap_or_else(|| "its type is used by other code".into()),
                ),
            }
        }
        if keyed.is_empty() {
            continue;
        }
        let new = rewrite(&p.src, p.f.as_deref().unwrap_or("t"), &keyed, &p.found.aux);
        // Never write a file glot cannot read back: the original stays, the strings are reported.
        if let Err(e) = rx::verify(&p.src, &new) {
            for (s, key) in &keyed {
                left(
                    s,
                    key,
                    format!("the rewritten file did not pass verification ({e}): left untouched"),
                );
            }
            report.reverted.push((p.path.clone(), e));
            continue;
        }
        for (s, key) in &keyed {
            if !have.contains(key) && added.insert(key.clone()) {
                add.push(NewKey {
                    key: key.clone(),
                    text: s.copy.text.clone(),
                    at: format!("{}:{}", p.path, s.copy.at.line),
                    note: s.note.clone(),
                    translations: s.translations.clone(),
                });
            }
        }
        rewrites.push((p.path.clone(), p.src.clone(), new));
    }
    // Stable order of the new rows, whatever the order files were read in.
    add.sort_by(|a, b| a.key.cmp(&b.key));
    report.files_rewritten = rewrites.len();
    report.new_keys = add.len();
    report.imported_translations = add
        .iter()
        .flat_map(|k| k.translations.keys())
        .filter(|l| ws.config.target_locales.contains(l))
        .count();
    if args.dry_run {
        for (path, old, new) in &rewrites {
            print_diff(&glot_extract::i18nize::unified_diff(path, old, new));
        }
    }
    for k in &add {
        anstream::println!(
            "  {GREEN}+{GREEN:#} {} = {:?}  {DIM}{}{DIM:#}",
            k.key,
            k.text,
            k.at
        );
    }
    for (path, why) in &report.reverted {
        anstream::println!(
            "warning: {path} left untouched: the rewritten file did not pass verification ({why})"
        );
    }
    if !report.left.is_empty() {
        anstream::println!(
            "{YELLOW}{} strings need a decision{YELLOW:#} (glot only rewrites what cannot change a type). For each: where, why, the code to write:",
            report.left.len()
        );
        for l in &report.left {
            anstream::println!(
                "    {DIM}{}:{}{DIM:#}  {:?} -> {}  {DIM}({}){DIM:#}",
                l.path,
                l.line,
                l.text,
                l.suggestion,
                l.reason
            );
        }
    }
    anstream::println!(
        "{BOLD}{}{BOLD:#} Rust files, {BOLD}{}{BOLD:#} rewritten, {BOLD}{}{BOLD:#} new keys ({} translations kept from the code), {BOLD}{}{BOLD:#} left with a suggestion",
        files.len(),
        rewrites.len(),
        add.len(),
        report.imported_translations,
        report.left.len()
    );
    if def.is_none() && !plans.is_empty() {
        anstream::println!(
            "{YELLOW}note:{YELLOW:#} no `pub fn t(` in the string-table files: nothing can be rewritten until the crate has one"
        );
    }
    if args.dry_run {
        return Ok(());
    }
    report.save();
    if rewrites.is_empty() {
        return Ok(());
    }
    if !args.yes
        && !crate::confirm(&format!(
            "Rewrite {} files and add {} keys to the source table?",
            rewrites.len(),
            add.len()
        ))?
    {
        anstream::println!("Aborted, nothing written.");
        return Ok(());
    }
    let items: Vec<_> = add
        .iter()
        .map(|k| {
            (
                k.key.clone(),
                k.text.clone(),
                k.note.clone(),
                k.translations.clone(),
            )
        })
        .collect();
    ws.add_sources(&items)?;
    for (path, _, new) in &rewrites {
        std::fs::write(ws.root.join(path), new)?;
    }
    anstream::println!(
        "{GREEN}✓ done.{GREEN:#} Next: {BOLD}cargo check{BOLD:#}, then {BOLD}{} translate{BOLD:#} to fill {}",
        crate::init::cli_name(),
        ws.config.target_locales.join(", ")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_and_modules() {
        assert!(has_t("use crate::i18n::{\n    t,\n    tn,\n};\nfn f() {}"));
        assert!(!has_t("use crate::i18n::{tn, tc};\nfn f() { let t = 1; }"));
        assert_eq!(
            module_of("web/src/i18n.rs", "web/src/pages/a.rs").as_deref(),
            Some("crate::i18n")
        );
        assert_eq!(
            module_of("web/src/i18n/mod.rs", "web/src/a.rs").as_deref(),
            Some("crate::i18n")
        );
        assert_eq!(module_of("web/src/i18n.rs", "api/src/a.rs"), None);
        assert!(shadows_t("fn f() { let t = tab.get(); }"));
        assert!(shadows_t("x.map(|t| t.len())"));
        assert!(!shadows_t(
            "use crate::i18n::{t, tn};\nfn f() { t(\"k\"); let x = \"t\"; tn(\"a\", 1); }"
        ));
    }

    #[test]
    fn rewrites_only_clean_spots() {
        let src = "view! { <p>\"Bienvenue\"</p> <input placeholder=\"Ton e-mail\" /> <Field label=\"Nom\" /> <span>{if ok { \"Oui\" } else { \"\" }}</span> }";
        let found = rx::analyze("pages/a.rs", src);
        let keyed: Vec<(&Spot, String)> = found
            .spots
            .iter()
            .filter(|s| s.clean)
            .map(|s| (s, snake(&s.copy.suggested_key)))
            .collect();
        let new = rewrite(src, "t", &keyed, &found.aux);
        assert_eq!(
            new,
            "view! { <p>{move || t(\"a_bienvenue\")}</p> <input placeholder=move || t(\"a_ton_e_mail_placeholder\") /> <Field label=\"Nom\" /> <span>{if ok { t(\"a_oui\") } else { \"\".to_string() }}</span> }"
        );
        rx::verify(src, &new).unwrap();
        assert!(rx::verify(src, "fn broken( {").is_err());
    }
}
