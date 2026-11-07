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

/// `f` is `t` or a path to it (`crate::i18n::t`).
fn replacement(s: &Spot, f: &str, key: &str) -> String {
    if s.attr {
        format!("move || {f}({key:?})")
    } else {
        format!("{{move || {f}({key:?})}}")
    }
}

/// Rewrite `src`'s `spots` (sorted by start) with calls to `f`.
fn rewrite(src: &str, f: &str, spots: &[(&Spot, String)]) -> String {
    let (mut out, mut pos) = (String::new(), 0);
    for (s, key) in spots {
        out += &src[pos..s.start];
        out += &replacement(s, f, key);
        pos = s.end;
    }
    out + &src[pos..]
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
    let read = |f: &str| std::fs::read_to_string(ws.root.join(f)).ok();
    // The table file defining `t` (`web/src/i18n.rs`).
    let def = ws
        .config
        .files
        .iter()
        .find(|f| f.ends_with(".rs") && read(f).is_some_and(|s| s.contains("pub fn t(")))
        .cloned();
    let have = ws.source().entries.clone();
    let mut taken: BTreeSet<String> = have.keys().cloned().collect();
    // text -> key created in this run.
    let mut made: BTreeMap<String, String> = BTreeMap::new();
    let mut add: Vec<(String, String, String)> = vec![];
    let mut rewrites: Vec<(String, String, String)> = vec![];
    let mut by_hand: Vec<(String, String)> = vec![];
    for path in &files {
        let Some(src) = read(path) else { continue };
        // Keys named from the path inside the crate (`pages/login.rs` => `login_…`).
        let inner = path.rfind("src/").map_or(path.as_str(), |i| &path[i + 4..]);
        let found = rx::spots(inner, &src);
        if found.is_empty() {
            continue;
        }
        // The imported `t`, else its full path (never shadowed by a local, no import to add).
        let module = def.as_deref().and_then(|d| module_of(d, path));
        let f = match &module {
            Some(m) if !has_t(&src) || shadows_t(&src) => Some(format!("{m}::t")),
            _ if has_t(&src) && !shadows_t(&src) => Some("t".to_string()),
            _ => None,
        };
        let callable = f.is_some();
        let mut keyed: Vec<(&Spot, String)> = vec![];
        for s in &found {
            let text = &s.copy.text;
            let key = match made.get(text) {
                Some(k) => k.clone(),
                None => {
                    let base = snake(&s.copy.suggested_key);
                    if have.get(&base) == Some(text) {
                        base
                    } else {
                        std::iter::once(base.clone())
                            .chain((2..).map(|n| format!("{base}_{n}")))
                            .find(|k| !taken.contains(k))
                            .expect("a free key")
                    }
                }
            };
            if s.clean && callable {
                if !made.contains_key(text) && !have.contains_key(&key) {
                    taken.insert(key.clone());
                    add.push((
                        key.clone(),
                        text.clone(),
                        format!("{path}:{}", s.copy.at.line),
                    ));
                }
                made.entry(text.clone()).or_insert_with(|| key.clone());
                keyed.push((s, key));
            } else {
                by_hand.push((
                    format!("{path}:{}", s.copy.at.line),
                    format!("{text:?} -> t({key:?})"),
                ));
            }
        }
        if !keyed.is_empty() {
            let new = rewrite(&src, f.as_deref().unwrap_or("t"), &keyed);
            rewrites.push((path.clone(), src, new));
        }
    }
    if args.dry_run {
        for (path, old, new) in &rewrites {
            print_diff(&glot_extract::i18nize::unified_diff(path, old, new));
        }
    }
    for (k, v, at) in &add {
        anstream::println!("  {GREEN}+{GREEN:#} {k} = {v:?}  {DIM}{at}{DIM:#}");
    }
    if !by_hand.is_empty() {
        anstream::println!(
            "{YELLOW}{} strings to wrap by hand{YELLOW:#} (inside an expression, a component prop, html!/rsx!, or no `t` in scope: rewriting could change a type). Suggested:",
            by_hand.len()
        );
        for (at, hint) in &by_hand {
            anstream::println!("    {DIM}{at}{DIM:#}  {hint}");
        }
    }
    anstream::println!(
        "{BOLD}{}{BOLD:#} Rust files, {BOLD}{}{BOLD:#} rewritten, {BOLD}{}{BOLD:#} new keys, {BOLD}{}{BOLD:#} left to wrap by hand",
        files.len(),
        rewrites.len(),
        add.len(),
        by_hand.len()
    );
    if def.is_none() && !rewrites.is_empty() {
        anstream::println!(
            "{YELLOW}note:{YELLOW:#} no `pub fn t(` in the string-table files: the rewritten code calls `t`, define it"
        );
    }
    if args.dry_run || rewrites.is_empty() {
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
    for (k, v, _) in &add {
        ws.add_source(k, v, None)?;
    }
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
        let src = "view! { <p>\"Bienvenue\"</p> <input placeholder=\"Ton e-mail\" /> <span>{if ok { \"Oui\" } else { \"Non\" }}</span> }";
        let spots = rx::spots("pages/a.rs", src);
        let keyed: Vec<(&Spot, String)> = spots
            .iter()
            .filter(|s| s.clean)
            .map(|s| (s, snake(&s.copy.suggested_key)))
            .collect();
        assert_eq!(
            rewrite(src, "t", &keyed),
            "view! { <p>{move || t(\"a_bienvenue\")}</p> <input placeholder=move || t(\"a_ton_e_mail_placeholder\") /> <span>{if ok { \"Oui\" } else { \"Non\" }}</span> }"
        );
    }
}
