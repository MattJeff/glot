//! `glot i18nize` on an iOS / macOS project. SwiftUI literals are already localized at runtime
//! (`Text("Hello")` looks up `"Hello"`), so the code is left alone: their keys are added to the
//! source catalog, in their table. Plain `String`s shown to the user (UIKit `.text = "…"`) are
//! listed, and wrapped in `String(localized: …)` with `--rewrite`.
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::PathBuf;

use glot_core::catalog::Catalog;
use glot_extract::swift;
use glot_llm::engine::Workspace;

use super::{I18nizeArgs, print_diff};
use crate::style::*;

/// Native Apple catalogs or Swift sources configured.
pub fn is_swift(ws: &Workspace) -> bool {
    ws.config
        .files
        .iter()
        .any(|f| f.ends_with(".xcstrings") || f.ends_with(".strings"))
        || ws
            .config
            .extract
            .include
            .iter()
            .any(|g| g.ends_with(".swift"))
}

/// Source-locale catalog file for `table` (`Auth` -> `en.lproj/Auth.strings`); the first file
/// (`Localizable.*`, or the String Catalog) otherwise.
fn table_path(paths: &[PathBuf], table: Option<&str>) -> Option<PathBuf> {
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned());
    let table = table.unwrap_or("Localizable");
    paths
        .iter()
        .filter(|p| !p.to_string_lossy().ends_with(".stringsdict"))
        .find(|p| stem(p).as_deref() == Some(table))
        .or_else(|| paths.first())
        .cloned()
}

/// Identifier-style key (`login.title`): its value cannot be guessed from the code.
fn id_key(k: &str) -> bool {
    !k.contains(char::is_whitespace)
        && (k.contains('.') || k.contains('_'))
        && k.chars().all(|c| c.is_alphanumeric() || "._-".contains(c))
        && k.split('.').all(|p| !p.is_empty())
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
        .filter(|f| f.ends_with(".swift"))
        .collect();
    if files.is_empty() {
        anyhow::bail!(
            "no .swift files to scan: set [extract] include = [\"**/*.swift\"] in glot.toml (or pass paths)"
        );
    }
    let sources: Vec<(String, String)> = files
        .into_iter()
        .filter_map(|f| Some((f.clone(), std::fs::read_to_string(ws.root.join(&f)).ok()?)))
        .collect();
    let types = swift::project_types(sources.iter().map(|(_, s)| s.as_str()));
    let src_locale = ws.config.source_locale.clone();
    let cat_paths = ws.config.catalog_paths(&ws.root, &src_locale);
    let have = ws.source().entries.clone();

    // key -> (file, value, note, where)
    let mut add: BTreeMap<String, (PathBuf, String, Option<String>, String)> = BTreeMap::new();
    let mut by_hand: Vec<String> = vec![];
    let mut rewrites: Vec<(String, String, String)> = vec![];
    let mut plain = 0;
    let (mut warnings, mut errors) = (0, vec![]);
    for (path, src) in &sources {
        let s = swift::scan(path, src, &ws.config.extract.functions, &types);
        if let Some(e) = s.error {
            errors.push(format!("{path}: {e}"));
            continue;
        }
        for (line, w) in &s.warnings {
            warnings += 1;
            anstream::eprintln!("{YELLOW}warning:{YELLOW:#} {path}:{line}: {w}");
        }
        let unlocalized: Vec<&swift::SwiftString> =
            s.strings.iter().filter(|x| !x.localized).collect();
        plain += unlocalized.len();
        for x in &unlocalized {
            if !args.rewrite {
                anstream::println!(
                    "  {YELLOW}~{YELLOW:#} {path}:{}  {:?}  {DIM}{} takes a plain String: String(localized: …) with --rewrite{DIM:#}",
                    x.line,
                    x.key,
                    x.api
                );
            }
        }
        if args.rewrite && !unlocalized.is_empty() {
            match swift::rewrite(src, &unlocalized) {
                Some(new) => rewrites.push((path.clone(), src.clone(), new)),
                None => errors.push(format!("{path}: rewrite would not re-parse, left as is")),
            }
        }
        for x in &s.strings {
            if (!x.localized && !args.rewrite) || have.contains_key(&x.key) {
                continue;
            }
            let at = format!("{path}:{}", x.line);
            let value = x.value.clone().unwrap_or_else(|| x.key.clone());
            if x.value.is_none() && id_key(&x.key) {
                by_hand.push(format!("{}  {DIM}{at}{DIM:#}", x.key));
                continue;
            }
            let Some(file) = table_path(&cat_paths, x.table.as_deref()) else {
                continue;
            };
            add.entry(x.key.clone())
                .or_insert((file, value, x.comment.clone(), at));
        }
    }

    for (path, old, new) in &rewrites {
        if args.dry_run {
            print_diff(&glot_extract::i18nize::unified_diff(path, old, new));
        }
    }
    for (k, (file, v, _, at)) in &add {
        let table = file.file_name().unwrap_or_default().to_string_lossy();
        let shown = if v == k {
            String::new()
        } else {
            format!(" = {v:?}")
        };
        anstream::println!("  {GREEN}+{GREEN:#} {k:?}{shown}  {DIM}{table}  {at}{DIM:#}");
    }
    by_hand.sort();
    by_hand.dedup();
    if !by_hand.is_empty() {
        anstream::println!(
            "{YELLOW}{} keys used in code but missing, with no value to infer:{YELLOW:#} add them with `{} add <key> <value>`",
            by_hand.len(),
            crate::init::cli_name()
        );
        for k in &by_hand {
            anstream::println!("    {k}");
        }
    }
    for e in &errors {
        anstream::println!("{RED}✗{RED:#} {e}");
    }
    anstream::println!(
        "{BOLD}{}{BOLD:#} Swift files, {BOLD}{}{BOLD:#} new keys for the catalog, {BOLD}{plain}{BOLD:#} plain Strings{}{}",
        sources.len(),
        add.len(),
        if args.rewrite {
            format!(" ({} files rewritten)", rewrites.len())
        } else {
            String::new()
        },
        if warnings > 0 {
            format!(", {warnings} interpolations typed %@ by default")
        } else {
            String::new()
        }
    );
    if args.dry_run || (add.is_empty() && rewrites.is_empty()) {
        return Ok(());
    }
    if !args.yes
        && !crate::confirm(&format!(
            "Add {} keys to the source catalog{}?",
            add.len(),
            if rewrites.is_empty() {
                String::new()
            } else {
                format!(" and rewrite {} files", rewrites.len())
            }
        ))?
    {
        anstream::println!("Aborted, nothing written.");
        return Ok(());
    }
    for (path, _, new) in &rewrites {
        std::fs::write(ws.root.join(path), new)?;
    }
    // Written per table file (not `add_source`: SwiftUI keys are sentences, `...` included).
    let mut per_file: BTreeMap<&PathBuf, Vec<(&String, &String)>> = BTreeMap::new();
    for (k, (file, v, note, _)) in &add {
        per_file.entry(file).or_default().push((k, v));
        if let Some(n) = note.as_deref().filter(|n| !n.trim().is_empty()) {
            ws.lock.notes.insert(k.clone(), n.to_string());
        }
    }
    for (file, entries) in per_file {
        let mut cat = Catalog::read(file, &src_locale, ws.config.format)?;
        for (k, v) in entries {
            cat.entries.insert(k.clone(), v.clone());
        }
        cat.write(file, ws.config.format)?;
    }
    ws.lock.save(&ws.root)?;
    anstream::println!(
        "{GREEN}✓ done.{GREEN:#} Next: {BOLD}{} translate{BOLD:#} to fill {}",
        crate::init::cli_name(),
        ws.config.target_locales.join(", ")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_and_id_keys() {
        let ps: Vec<PathBuf> = [
            "en.lproj/Localizable.strings",
            "en.lproj/Auth.strings",
            "en.lproj/Localizable.stringsdict",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(table_path(&ps, Some("Auth")).unwrap(), ps[1]);
        assert_eq!(table_path(&ps, None).unwrap(), ps[0]);
        assert_eq!(table_path(&ps, Some("Nope")).unwrap(), ps[0]);
        assert!(id_key("login.title") && !id_key("Loading...") && !id_key("Hello world"));
    }
}
