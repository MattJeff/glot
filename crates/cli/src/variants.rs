//! Register variants (`[variants]` in glot.toml, see `glot_core::variants`): after `glot translate`,
//! each `fr-x-informal` catalog is generated from its base `fr` (not the source) so both share wording,
//! validated against the base (same placeholders), and tracked in glot.lock against the base text.
use anyhow::Result;
use std::collections::BTreeSet;

use glot_core::catalog::Catalog;
use glot_core::context::KeyContext;
use glot_core::lock::Origin;
use glot_core::validate::has_errors;
use glot_core::variants;
use glot_llm::provider;
use glot_llm::translate::{Item, Request, Translator};

use crate::style::*;

/// Variants of the requested base locales (all when `locales` is empty); a variant tag selects itself.
pub async fn translate(
    locales: &[String],
    keys: &[String],
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let mut ws = crate::open()?;
    let declared = variants::load(&ws.root)?;
    if declared.is_empty() {
        return Ok(());
    }
    let mut translator = None;
    for (base, tag) in variants::locales(&declared) {
        if !locales.is_empty() && !locales.contains(&base) && !locales.contains(&tag) {
            continue;
        }
        let Some(base_cat) = ws.catalogs.get(&base).cloned() else {
            anstream::eprintln!(
                "{YELLOW}warning:{YELLOW:#} variant {tag}: base locale {base} is not in glot.toml"
            );
            continue;
        };
        let paths = ws.config.catalog_paths(&ws.root, &tag);
        let Some(path) = paths.first().cloned() else {
            continue;
        };
        let target = Catalog::read_all(&paths, &tag, ws.config.format)?;
        let plan = ws.lock.plan(&base_cat, std::slice::from_ref(&target));
        let manual: BTreeSet<_> = plan.manual_edits.iter().map(|(_, k)| k.clone()).collect();
        let todo: Vec<String> = if force {
            base_cat.entries.keys().cloned().collect()
        } else {
            plan.missing
                .iter()
                .chain(&plan.stale)
                .map(|(_, k)| k.clone())
                .filter(|k| !manual.contains(k))
                .collect()
        };
        let todo: Vec<String> = todo
            .into_iter()
            .filter(|k| keys.is_empty() || keys.contains(k))
            .filter(|k| {
                base_cat
                    .entries
                    .get(k)
                    .is_some_and(|v| !v.trim().is_empty())
            })
            .collect();
        if todo.is_empty() {
            continue;
        }
        anstream::println!(
            "Variant {BOLD}{tag}{BOLD:#} ← {base}: {} string(s)",
            todo.len()
        );
        if dry_run {
            continue;
        }
        // Glossary translations of the base apply to its variants.
        let mut glossary = vec![];
        for k in &todo {
            for t in ws.glossary.matches(&base_cat.entries[k]) {
                if !glossary
                    .iter()
                    .any(|g: &glot_core::glossary::Term| g.term == t.term)
                {
                    let mut t = t.clone();
                    if let Some(tr) = t.translations.get(&base).cloned() {
                        t.translations.insert(tag.clone(), tr);
                    }
                    if let Some(f) = t.forbidden.get(&base).cloned() {
                        t.forbidden.insert(tag.clone(), f);
                    }
                    glossary.push(t);
                }
            }
        }
        let variant = variants::split(&tag).map(|(_, v)| v).unwrap_or_default();
        let style = [
            ws.config.llm.style.clone().unwrap_or_default(),
            variants::instruction(&base, variant),
        ]
        .join("\n")
        .trim()
        .to_string();
        let req = Request {
            source_locale: base.clone(),
            target_locale: tag.clone(),
            items: todo
                .iter()
                .map(|k| Item {
                    key: k.clone(),
                    source: base_cat.entries[k].clone(),
                    context: KeyContext {
                        key: k.clone(),
                        description: ws.lock.notes.get(k).cloned(),
                        ..Default::default()
                    },
                    previous: target.entries.get(k).filter(|v| !v.is_empty()).cloned(),
                })
                .collect(),
            glossary,
            syntax: ws.config.syntax,
            style: Some(style),
        };
        if translator.is_none() {
            translator = Some(Translator::new(
                &ws.config,
                provider::from_config(&ws.config.llm, None)?,
            ));
        }
        let (results, _usage) = translator.as_ref().unwrap().translate(req).await?;
        // ponytail: new keys land in the variant's first catalog file; split catalogs keep keys where they already are.
        let mut file = Catalog::read(&path, &tag, ws.config.format)?;
        let mut written = 0;
        for t in &results {
            if has_errors(&t.issues) {
                let why: Vec<&str> = t.issues.iter().map(|i| i.message.as_str()).collect();
                anstream::println!(
                    "  {RED}✗{RED:#} {tag} {}  {DIM}{}{DIM:#}",
                    t.key,
                    why.join("; ")
                );
                continue;
            }
            file.entries.insert(t.key.clone(), t.value.clone());
            ws.lock.record(
                &tag,
                &t.key,
                &base_cat.entries[&t.key],
                &t.value,
                Origin::Machine {
                    model: t.model.clone(),
                },
                t.score,
            );
            written += 1;
        }
        file.write(&path, ws.config.format)?;
        ws.lock.save(&ws.root)?;
        anstream::println!("  {GREEN}✓{GREEN:#} {tag:<6} {written} written");
    }
    Ok(())
}

/// `glot check` findings for variant catalogs (placeholders/plurals/tags vs the base).
pub fn check(ws: &glot_llm::engine::Workspace) -> Vec<glot_llm::engine::Finding> {
    let declared = variants::load(&ws.root).unwrap_or_default();
    let mut out = vec![];
    for (base, tag) in variants::locales(&declared) {
        let Some(base_cat) = ws.catalogs.get(&base) else {
            continue;
        };
        let paths = ws.config.catalog_paths(&ws.root, &tag);
        let Ok(cat) = Catalog::read_all(&paths, &tag, ws.config.format) else {
            continue;
        };
        for (key, issue) in variants::check(base_cat, &cat, ws.config.syntax) {
            out.push(glot_llm::engine::Finding {
                locale: tag.clone(),
                key,
                issue,
            });
        }
    }
    out
}
