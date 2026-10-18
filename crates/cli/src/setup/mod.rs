//! `glot i18nize` runtime wiring: detect the framework, then add the i18n library, its config,
//! the provider and a locale switcher. Never overwrites: existing files are edited with span
//! edits (AST) or left alone with a note. Every change is a (path, old, new) shown as a diff.
use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::Path;

use glot_extract::i18nize::Lib;

mod next;
mod react;
pub mod refactor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Routing {
    /// Non-default locales always live under `/fr/...` (`as-needed`: the default has no prefix).
    Prefix,
    /// `/fr/...` selects the locale and stores it in a cookie; unprefixed URLs keep it.
    Cookie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framework {
    NextApp,
    NextPages,
    Vite,
    Remix,
    Astro,
}

impl Framework {
    pub fn lib(self) -> Lib {
        match self {
            Framework::NextApp | Framework::NextPages => Lib::NextIntl,
            _ => Lib::ReactI18next,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Framework::NextApp => "Next.js App Router",
            Framework::NextPages => "Next.js Pages Router",
            Framework::Vite => "Vite + React",
            Framework::Remix => "Remix / React Router",
            Framework::Astro => "Astro (React islands)",
        }
    }
}

pub struct Project {
    pub framework: Framework,
    /// `src/` prefix when the app lives in `src/app`, `src/pages`.
    pub base: &'static str,
    pub pm: &'static str,
    pub next_major: u32,
    /// `react-router` major (Remix / React Router projects), 0 when absent.
    pub rr_major: u32,
    pub deps: BTreeSet<String>,
}

pub fn deps(root: &Path) -> (BTreeSet<String>, serde_json::Value) {
    let pkg: serde_json::Value = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let deps = ["dependencies", "devDependencies"]
        .iter()
        .filter_map(|k| pkg.get(k)?.as_object())
        .flat_map(|o| o.keys().cloned())
        .collect();
    (deps, pkg)
}

pub(crate) fn exists_any(root: &Path, base: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .map(|n| format!("{base}{n}"))
        .find(|p| root.join(p).is_file())
}

pub fn detect(root: &Path) -> Option<Project> {
    let (deps, pkg) = deps(root);
    let has = |d: &str| deps.contains(d);
    let layout = |b: &str| {
        exists_any(
            root,
            b,
            &[
                "app/layout.tsx",
                "app/layout.jsx",
                "app/layout.js",
                "app/layout.ts",
            ],
        )
        .is_some()
    };
    let (framework, base) = if has("next") {
        if layout("") {
            (Framework::NextApp, "")
        } else if layout("src/") {
            (Framework::NextApp, "src/")
        } else if root.join("src/pages").is_dir() {
            (Framework::NextPages, "src/")
        } else {
            (Framework::NextPages, "")
        }
    } else if has("astro") {
        (Framework::Astro, "src/")
    } else if has("@remix-run/react")
        || has("react-router")
            && (root.join("app/root.tsx").is_file() || root.join("app/root.jsx").is_file())
    {
        (Framework::Remix, "app/")
    } else if has("react") {
        (Framework::Vite, "src/")
    } else {
        return None;
    };
    let pm = if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bun"
    } else if root.join("package-lock.json").is_file() {
        "npm"
    } else {
        pkg["packageManager"]
            .as_str()
            .and_then(|p| {
                ["pnpm", "yarn", "bun"]
                    .into_iter()
                    .find(|m| p.starts_with(m))
            })
            .unwrap_or("npm")
    };
    // `*` / `latest` / workspace specs count as the latest major.
    let major = |dep: &str, latest: u32| {
        ["dependencies", "devDependencies"]
            .iter()
            .find_map(|k| pkg[k][dep].as_str())
            .map(|v| {
                v.trim_start_matches(['^', '~', '>', '=', ' '])
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .parse()
                    .unwrap_or(latest)
            })
    };
    let next_major = major("next", 16).unwrap_or(16);
    let rr_major = major("react-router", 8).unwrap_or(0);
    Some(Project {
        framework,
        base,
        pm,
        next_major,
        rr_major,
        deps,
    })
}

pub struct Opts {
    pub routing: Routing,
    pub switcher: bool,
    /// Source first.
    pub locales: Vec<String>,
    /// `locales/{locale}.json`
    pub catalog: String,
}

pub struct Change {
    pub path: String,
    pub old: Option<String>,
    pub new: String,
}

#[derive(Default)]
pub struct SetupPlan {
    pub changes: Vec<Change>,
    /// Packages to add with the project's package manager.
    pub packages: Vec<String>,
    pub pm: String,
    pub notes: Vec<String>,
    /// Source strings the generated code needs (locale switcher label).
    pub keys: Vec<(String, String, String)>,
}

impl SetupPlan {
    pub(crate) fn create(&mut self, root: &Path, rel: &str, content: String) {
        if let Ok(cur) = std::fs::read_to_string(root.join(rel)) {
            if cur != content {
                self.notes.push(format!("{rel} exists, left as is"));
            }
        } else if root.join(rel).exists() {
            self.notes.push(format!("{rel} exists, left as is"));
        } else {
            self.changes.push(Change {
                path: rel.into(),
                old: None,
                new: content,
            });
        }
    }

    /// Edit an existing file; `f` returns None when it is already set up, Err(note) when it can't.
    /// A file edited twice in one plan gets one change: `f` sees the previous edit's result.
    pub(crate) fn edit(
        &mut self,
        root: &Path,
        rel: &str,
        f: impl FnOnce(&str) -> std::result::Result<Option<String>, String>,
    ) {
        let pending = self.changes.iter().position(|c| c.path == rel);
        let cur = match pending {
            Some(i) => self.changes[i].new.clone(),
            None => match std::fs::read_to_string(root.join(rel)) {
                Ok(s) => s,
                Err(_) => return,
            },
        };
        match f(&cur) {
            Ok(Some(new)) if new != cur => match pending {
                Some(i) => self.changes[i].new = new,
                None => self.changes.push(Change {
                    path: rel.into(),
                    old: Some(cur),
                    new,
                }),
            },
            Ok(_) => {}
            Err(note) => self.notes.push(format!("{rel}: {note}")),
        }
    }

    pub fn install_cmd(&self) -> Option<String> {
        if self.packages.is_empty() {
            return None;
        }
        let add = if self.pm == "npm" { "install" } else { "add" };
        Some(format!("{} {add} {}", self.pm, self.packages.join(" ")))
    }
}

/// `from_dir` -> `to` as an import path (`../locales/en.json`).
pub fn rel_import(from_dir: &str, to: &str) -> String {
    let a: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let up = a.len() - common;
    let rest = b[common..].join("/");
    if up == 0 {
        format!("./{rest}")
    } else {
        format!("{}{rest}", "../".repeat(up))
    }
}

/// Edit inserting an import after the last import (or the directives, or at the top).
pub fn import_edit(path: &str, src: &str, line: &str) -> Option<(u32, u32, String)> {
    let alloc = oxc_allocator::Allocator::default();
    let program = refactor::parse(&alloc, path, src)?;
    let last_import = program
        .body
        .iter()
        .filter(|s| matches!(s, oxc_ast::ast::Statement::ImportDeclaration(_)))
        .map(|s| oxc_span::GetSpan::span(s).end)
        .next_back();
    Some(match (last_import, program.directives.last()) {
        (Some(p), _) => (p, p, format!("\n{line}")),
        (None, Some(d)) => (d.span.end, d.span.end, format!("\n\n{line}")),
        (None, None) => (0, 0, format!("{line}\n")),
    })
}

pub fn add_import(path: &str, src: &str, line: &str) -> Option<String> {
    refactor::apply_edits(src, vec![import_edit(path, src, line)?])
}

pub fn plan(root: &Path, p: &Project, o: &Opts) -> SetupPlan {
    let mut plan = SetupPlan {
        pm: p.pm.into(),
        ..Default::default()
    };
    let need = |plan: &mut SetupPlan, pkg: &str, spec: &str| {
        if !p.deps.contains(pkg) {
            plan.packages.push(spec.into());
        }
    };
    match p.framework {
        Framework::NextApp => {
            need(&mut plan, "next-intl", "next-intl@^4");
            next::app(root, p, o, &mut plan);
        }
        Framework::NextPages => {
            need(&mut plan, "next-intl", "next-intl@^4");
            next::pages(root, p, o, &mut plan);
        }
        _ => {
            need(&mut plan, "i18next", "i18next@^26");
            need(&mut plan, "react-i18next", "react-i18next@^17");
            react::setup(root, p, o, &mut plan);
        }
    }
    if o.switcher {
        plan.keys.push((
            "locale_switcher.label".into(),
            "Language".into(),
            "aria-label of the language picker".into(),
        ));
    }
    plan
}

/// Static imports need every catalog to exist: create missing ones as `{}`.
fn empty_catalogs(root: &Path, o: &Opts, plan: &mut SetupPlan) {
    for l in &o.locales {
        let rel = o.catalog.replace("{locale}", l);
        if !root.join(&rel).exists() {
            plan.changes.push(Change {
                path: rel,
                old: None,
                new: "{}\n".into(),
            });
        }
    }
}

pub fn write(root: &Path, plan: &SetupPlan) -> Result<()> {
    for c in &plan.changes {
        let path = root.join(&c.path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if let Some(old) = &c.old
            && std::fs::read_to_string(&path).ok().as_ref() != Some(old)
        {
            anyhow::bail!(
                "{} changed while planning the setup; re-run i18nize",
                c.path
            );
        }
        std::fs::write(&path, &c.new).with_context(|| format!("writing {}", c.path))?;
    }
    Ok(())
}

pub fn install(root: &Path, plan: &SetupPlan) -> Result<()> {
    let Some(cmd) = plan.install_cmd() else {
        return Ok(());
    };
    let mut parts = cmd.split(' ');
    let out = std::process::Command::new(parts.next().unwrap())
        .args(parts)
        .current_dir(root)
        .output()
        .with_context(|| format!("running `{cmd}`"))?;
    anyhow::ensure!(
        out.status.success(),
        "`{cmd}` failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}

#[cfg(test)]
mod tests;
