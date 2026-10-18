//! `glot i18nize`: hardcoded copy -> catalog keys + rewritten source.
//! Detection/rewrite is deterministic (`glot_extract::i18nize`); the LLM only names keys
//! and writes translator notes (one JSON call per file), with a deterministic fallback.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use futures::stream::{self, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};

use glot_core::validate::{self, Syntax};
pub use glot_extract::i18nize::Lib;
use glot_extract::i18nize::{self as ex, Candidate, FileAnalysis, Skipped};

use crate::{
    Error, Result,
    engine::Workspace,
    provider::{self, CompletionRequest, Provider},
    translate::{self, Usage},
};

#[derive(Debug, Clone, Serialize)]
pub struct NewKey {
    pub key: String,
    pub value: String,
    pub note: Option<String>,
    /// Existing key reused (same text and role): nothing to write.
    pub reused: bool,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilePlan {
    pub path: String,
    pub diff: String,
    pub converted: usize,
    pub keys: Vec<NewKey>,
    pub skipped: Vec<Skipped>,
    pub error: Option<String>,
    #[serde(skip)]
    pub old: String,
    #[serde(skip)]
    pub new: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub lib: Lib,
    /// Provider used to name keys, or `deterministic`.
    pub namer: String,
    pub files: Vec<FilePlan>,
    pub usage: Usage,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn converted(&self) -> usize {
        self.files.iter().map(|f| f.converted).sum()
    }
    pub fn changed_files(&self) -> impl Iterator<Item = &FilePlan> {
        self.files
            .iter()
            .filter(|f| f.error.is_none() && f.new != f.old)
    }
}

/// `next-intl` / `react-i18next` from package.json (a bare Next.js app defaults to next-intl).
pub fn detect_lib(root: &Path) -> Option<Lib> {
    let pkg = std::fs::read_to_string(root.join("package.json")).ok()?;
    let has = |d: &str| pkg.contains(&format!("\"{d}\""));
    if has("next-intl") {
        Some(Lib::NextIntl)
    } else if has("react-i18next") {
        Some(Lib::ReactI18next)
    } else if has("next") {
        Some(Lib::NextIntl)
    } else {
        None
    }
}

const EXTS: &[&str] = &["ts", "tsx", "js", "jsx"];

/// Files to process: explicit files/dirs (relative to the project root), or `[extract]` globs.
pub fn list_files(ws: &Workspace, paths: &[String]) -> Result<Vec<String>> {
    if paths.is_empty() {
        return ex::source_files(&ws.root, &ws.config.extract)
            .map_err(|e| Error::Other(e.to_string()));
    }
    let mut out = BTreeSet::new();
    for p in paths {
        let abs = ws.root.join(p.trim_start_matches("./"));
        let abs = abs
            .canonicalize()
            .map_err(|e| Error::Other(format!("{p}: {e}")))?;
        let root = ws
            .root
            .canonicalize()
            .map_err(|e| Error::Other(e.to_string()))?;
        let rel = abs
            .strip_prefix(&root)
            .map_err(|_| Error::Other(format!("{p} is outside the project root")))?
            .to_string_lossy()
            .replace('\\', "/");
        if abs.is_dir() {
            let dir = if rel.is_empty() {
                String::new()
            } else {
                format!("{rel}/")
            };
            let cfg = glot_core::config::ExtractConfig {
                include: EXTS.iter().map(|e| format!("{dir}**/*.{e}")).collect(),
                exclude: ws.config.extract.exclude.clone(),
                functions: vec![],
            };
            out.extend(ex::source_files(&ws.root, &cfg).map_err(|e| Error::Other(e.to_string()))?);
        } else {
            out.insert(rel);
        }
    }
    Ok(out.into_iter().collect())
}

struct Named {
    leaf: String,
    note: Option<String>,
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_string()
}

/// LLM key -> `a.b_c` relative to `ns`, or None if unusable.
fn clean_leaf(key: &str, ns: &str) -> Option<String> {
    let key = key
        .trim()
        .strip_prefix(&format!("{ns}."))
        .unwrap_or(key.trim());
    let segs: Vec<String> = key.split('.').map(snake).collect();
    (!segs.is_empty()
        && segs.len() <= 4
        && segs
            .iter()
            .all(|s| !s.is_empty() && !s.starts_with(|c: char| c.is_ascii_digit())))
    .then(|| segs.join("."))
}

fn leaf_of(c: &Candidate) -> String {
    c.suggested_key
        .rsplit_once('.')
        .map_or(c.suggested_key.clone(), |(_, l)| l.to_string())
}

fn default_note(c: &Candidate) -> String {
    let mut n = c.role.clone();
    if let Some(e) = &c.element {
        n.push_str(&format!(" in <{e}>"));
    }
    if let Some(comp) = &c.component {
        n.push_str(&format!(" of {comp}"));
    }
    n
}

const SYSTEM: &str = "You are a senior i18n engineer. You name translation keys for a SaaS codebase and write short notes for translators. Answer with one JSON object only.";

fn prompt(fa: &FileAnalysis, cands: &[Candidate], existing: &[String]) -> String {
    let strings: Vec<Value> = cands
        .iter()
        .map(|c| json!({"id": c.id, "text": c.text, "role": c.role, "component": c.component, "element": c.element, "line": c.line, "suggested": leaf_of(c)}))
        .collect();
    let payload = json!({"task": "keys", "file": fa.path, "namespace": fa.namespace, "existing_keys": existing, "strings": strings});
    format!(
        "Name a translation key for each UI string of `{}`. Keys are RELATIVE to the namespace `{}` (do not repeat it): \
lowercase snake_case segments joined by dots, at most 3 segments, grouped by UI area and stable \
(e.g. `empty_state.title`, `table.columns.amount`, `actions.delete_button`, `filters.search_placeholder`). \
Same text and role => same key. Never reuse an existing key for a different text.\n\
For each string also write a one-line note for translators: where it appears, what it does, constraints (e.g. short button label).\n\
Answer exactly: {{\"keys\": {{\"<id>\": {{\"key\": \"...\", \"note\": \"...\"}}}}}}\n\n{}",
        fa.path,
        fa.namespace,
        translate::payload_block(&payload)
    )
}

/// Mock provider answer: the deterministic suggestion + a role note.
pub(crate) fn mock_keys(p: &Value) -> Value {
    let keys: serde_json::Map<String, Value> = p["strings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_u64()?.to_string();
            let note = format!(
                "{} in {}",
                s["role"].as_str().unwrap_or("text"),
                s["component"].as_str().unwrap_or("the page")
            );
            Some((id, json!({"key": s["suggested"], "note": note})))
        })
        .collect();
    json!({ "keys": keys })
}

async fn name_file(
    provider: &dyn Provider,
    model: &str,
    fa: &FileAnalysis,
    cands: Vec<Candidate>,
    existing: Vec<String>,
) -> std::result::Result<(HashMap<usize, Named>, Usage), String> {
    let req = CompletionRequest {
        model: model.into(),
        system: SYSTEM.into(),
        user: prompt(fa, &cands, &existing),
        max_tokens: 4096,
        json: true,
        images: vec![],
    };
    let c = provider.complete(req).await.map_err(|e| e.to_string())?;
    let usage = Usage::of_completion(&c);
    let v = translate::parse_json(&c.text).ok_or("LLM answer is not JSON")?;
    let mut out = HashMap::new();
    for cand in &cands {
        let item = &v["keys"][cand.id.to_string()];
        if let Some(leaf) = item["key"]
            .as_str()
            .and_then(|k| clean_leaf(k, &fa.namespace))
        {
            let note = item["note"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(String::from);
            out.insert(cand.id, Named { leaf, note });
        }
    }
    Ok((out, usage))
}

/// Keys already used (exact or as a prefix/child: nested JSON can't hold both `a.b` and `a.b.c`).
struct Keyspace {
    values: BTreeMap<String, String>,
}

impl Keyspace {
    fn free(&self, key: &str, value: &str) -> bool {
        if let Some(v) = self.values.get(key) {
            return v == value;
        }
        let child = format!("{key}.");
        let has_child = self
            .values
            .range(child.clone()..)
            .next()
            .is_some_and(|(k, _)| k.starts_with(&child));
        let has_parent = key
            .match_indices('.')
            .any(|(i, _)| self.values.contains_key(&key[..i]));
        !has_child && !has_parent
    }

    /// First free variant of `base` for these entries (`base`, `base_2`, ...).
    fn claim(&mut self, base: &str, entries: &[(String, String)]) -> String {
        // An existing leaf `a.b` can't get children: `a.b.c` -> `a.b_c`.
        let mut base = base.to_string();
        while let Some(i) = base
            .match_indices('.')
            .map(|(i, _)| i)
            .find(|&i| self.values.contains_key(&base[..i]))
        {
            base.replace_range(i..i + 1, "_");
        }
        let base = base.as_str();
        let mut key = base.to_string();
        let mut i = 2;
        while !entries
            .iter()
            .all(|(suf, v)| self.free(&format!("{key}{suf}"), v))
        {
            key = format!("{base}_{i}");
            i += 1;
        }
        for (suf, v) in entries {
            self.values.insert(format!("{key}{suf}"), v.clone());
        }
        key
    }
}

pub async fn plan(ws: &Workspace, files: &[String], lib: Lib) -> Result<Plan> {
    let syntax = ws.config.syntax;
    if syntax == Syntax::Mf2 {
        return Err(Error::Other(
            "i18nize supports syntax = \"icu\" or \"i18next\", not mf2".into(),
        ));
    }
    let mut warnings = vec![];
    let mut analyses: Vec<(String, FileAnalysis)> = vec![];
    for f in files {
        let src = std::fs::read_to_string(ws.root.join(f))
            .map_err(|e| Error::Other(format!("{f}: {e}")))?;
        let mut fa = ex::analyze(f, &src, lib);
        // Drop messages the project's validator would reject.
        let (ok, bad): (Vec<_>, Vec<_>) =
            std::mem::take(&mut fa.candidates)
                .into_iter()
                .partition(|c| {
                    ex::render_message(&c.parts, syntax)
                        .iter()
                        .all(|(_, m)| validate::placeholders(m, syntax).is_ok())
                });
        fa.skipped.extend(bad.into_iter().map(|c| Skipped {
            line: c.line,
            text: c.text,
            reason: format!("generated message is not valid {syntax:?}"),
        }));
        fa.candidates = ok;
        analyses.push((src, fa));
    }

    // Existing catalog: text -> keys, with the UI role seen in code (if any).
    let source = ws.source();
    let roles: HashMap<String, String> = ws
        .extract()
        .map(|r| {
            r.usages
                .into_iter()
                .filter_map(|(k, u)| Some((k, u.into_iter().find_map(|u| u.ui_role)?)))
                .collect()
        })
        .unwrap_or_default();
    let mut by_text: HashMap<&str, Vec<&String>> = HashMap::new();
    for (k, v) in &source.entries {
        by_text.entry(v.as_str()).or_default().push(k);
    }
    let reuse = |c: &Candidate| -> Option<String> {
        if c.parts.iter().any(|p| matches!(p, ex::Part::Plural { .. })) && syntax == Syntax::I18next
        {
            return None;
        }
        by_text
            .get(c.text.as_str())?
            .iter()
            .find(|k| roles.get(**k).is_none_or(|r| *r == c.role))
            .map(|k| k.to_string())
    };
    // Same text + role in several files => `common.*`.
    let mut seen_in: HashMap<(String, String), BTreeSet<usize>> = HashMap::new();
    for (i, (_, fa)) in analyses.iter().enumerate() {
        for c in &fa.candidates {
            seen_in
                .entry((c.text.clone(), c.role.clone()))
                .or_default()
                .insert(i);
        }
    }
    let shared = |c: &Candidate| {
        seen_in
            .get(&(c.text.clone(), c.role.clone()))
            .is_some_and(|s| s.len() > 1)
    };

    // One LLM call per file for the strings that need a fresh name.
    let (provider, namer) = match provider::from_config(&ws.config.llm, None) {
        Ok(p) => {
            let n = p.name().to_string();
            (Some(p), n)
        }
        Err(e) => {
            warnings.push(format!(
                "no LLM available ({e}); using deterministic key names"
            ));
            (None, "deterministic".into())
        }
    };
    let model = ws.config.llm.model.clone();
    let jobs: Vec<(usize, Vec<Candidate>, Vec<String>)> = analyses
        .iter()
        .enumerate()
        .filter_map(|(i, (_, fa))| {
            let cands: Vec<Candidate> = fa
                .candidates
                .iter()
                .filter(|c| reuse(c).is_none() && !shared(c))
                .cloned()
                .collect();
            let prefix = format!("{}.", fa.namespace);
            let existing: Vec<String> = source
                .entries
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .take(50)
                .cloned()
                .collect();
            (!cands.is_empty()).then_some((i, cands, existing))
        })
        .collect();
    let mut usage = Usage::default();
    let mut names: HashMap<(usize, usize), Named> = HashMap::new();
    if let Some(p) = provider.as_deref() {
        let results: Vec<_> = stream::iter(jobs)
            .map(|(i, cands, existing)| {
                let fa = &analyses[i].1;
                let model = model.clone();
                async move { (i, name_file(p, &model, fa, cands, existing).await) }
            })
            .buffered(4)
            .collect()
            .await;
        for (i, r) in results {
            match r {
                Ok((named, u)) => {
                    usage.add(&u);
                    names.extend(named.into_iter().map(|(id, n)| ((i, id), n)));
                }
                Err(e) => warnings.push(format!(
                    "{}: key naming failed ({e}); deterministic names used",
                    analyses[i].1.path
                )),
            }
        }
    }

    let mut space = Keyspace {
        values: source
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    let mut assigned: HashMap<(String, String), String> = HashMap::new();
    let mut out = vec![];
    for (i, (src, fa)) in analyses.iter().enumerate() {
        let mut keys: BTreeMap<usize, String> = BTreeMap::new();
        let mut new_keys = vec![];
        for c in &fa.candidates {
            let entries = ex::render_message(&c.parts, syntax);
            let ident = (c.text.clone(), c.role.clone());
            if let Some(k) = reuse(c) {
                new_keys.push(NewKey {
                    key: k.clone(),
                    value: c.text.clone(),
                    note: None,
                    reused: true,
                    line: c.line,
                });
                keys.insert(c.id, k);
                continue;
            }
            if let Some(k) = assigned.get(&ident) {
                keys.insert(c.id, k.clone());
                continue;
            }
            let named = names.remove(&(i, c.id));
            let (base, note) = if shared(c) {
                (
                    format!("common.{}", leaf_of(c)),
                    format!("shared {}", c.role),
                )
            } else {
                let note = named
                    .as_ref()
                    .and_then(|n| n.note.clone())
                    .unwrap_or_else(|| default_note(c));
                (
                    format!(
                        "{}.{}",
                        fa.namespace,
                        named.map_or_else(|| leaf_of(c), |n| n.leaf)
                    ),
                    note,
                )
            };
            let key = space.claim(&base, &entries);
            assigned.insert(ident, key.clone());
            for (suf, v) in entries {
                new_keys.push(NewKey {
                    key: format!("{key}{suf}"),
                    value: v,
                    note: Some(note.clone()),
                    reused: false,
                    line: c.line,
                });
            }
            keys.insert(c.id, key);
        }
        let mut fp = FilePlan {
            path: fa.path.clone(),
            diff: String::new(),
            converted: 0,
            keys: vec![],
            skipped: fa.skipped.clone(),
            error: fa.error.clone(),
            old: src.clone(),
            new: src.clone(),
        };
        if fa.error.is_none() && !keys.is_empty() {
            match ex::rewrite(fa, src, &keys, lib) {
                Ok(new) => {
                    fp.diff = ex::unified_diff(&fa.path, src, &new);
                    fp.new = new;
                    fp.converted = keys.len();
                    fp.keys = new_keys;
                }
                Err(e) => fp.error = Some(e),
            }
        }
        out.push(fp);
    }
    Ok(Plan {
        lib,
        namer,
        files: out,
        usage,
        warnings,
    })
}

/// Write rewritten files, then the new source strings (with notes) to the catalog.
pub fn apply(ws: &mut Workspace, plan: &Plan) -> Result<()> {
    for f in plan.changed_files() {
        let path = ws.root.join(&f.path);
        let disk =
            std::fs::read_to_string(&path).map_err(|e| Error::Other(format!("{}: {e}", f.path)))?;
        if disk != f.old {
            return Err(Error::Other(format!(
                "{} changed since the plan was made; re-run i18nize",
                f.path
            )));
        }
        ex::parses(&f.path, &f.new).map_err(|e| Error::Other(format!("{}: {e}", f.path)))?;
    }
    let mut written = BTreeSet::new();
    for f in plan.changed_files() {
        std::fs::write(ws.root.join(&f.path), &f.new)
            .map_err(|e| Error::Other(format!("{}: {e}", f.path)))?;
        for k in f.keys.iter().filter(|k| !k.reused) {
            if written.insert(k.key.clone()) {
                ws.add_source(&k.key, &k.value, k.note.as_deref())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyspace_avoids_collisions_and_prefixes() {
        let mut s = Keyspace {
            values: [("a.b".to_string(), "X".to_string())].into(),
        };
        let e = |v: &str| vec![(String::new(), v.to_string())];
        assert_eq!(s.claim("a.b", &e("X")), "a.b");
        assert_eq!(s.claim("a.b", &e("Y")), "a.b_2");
        assert_eq!(s.claim("a.b.c", &e("Z")), "a.b_c");
        assert_eq!(s.claim("a", &e("W")), "a_2");
        assert_eq!(
            clean_leaf("invoices.Empty State.title", "invoices").as_deref(),
            Some("empty_state.title")
        );
        assert_eq!(clean_leaf("", "x"), None);
    }

    #[tokio::test]
    async fn plan_and_apply_with_mock_and_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("glot.toml"), "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n[extract]\ninclude = [\"app/**/*.tsx\"]\n").unwrap();
        std::fs::create_dir_all(root.join("messages")).unwrap();
        std::fs::write(
            root.join("messages/en.json"),
            r#"{"common": {"save": "Save"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("app/a")).unwrap();
        std::fs::create_dir_all(root.join("app/b")).unwrap();
        std::fs::write(root.join("app/a/page.tsx"), "export default function A() {\n  return <div><button>Save</button><p>Delete invoice</p></div>;\n}\n").unwrap();
        std::fs::write(
            root.join("app/b/page.tsx"),
            "export default function B() {\n  return <p>Delete invoice</p>;\n}\n",
        )
        .unwrap();
        let mut ws = Workspace::open(root).unwrap();
        let files = list_files(&ws, &[]).unwrap();
        assert_eq!(files, ["app/a/page.tsx", "app/b/page.tsx"]);
        let plan = plan(&ws, &files, Lib::NextIntl).await.unwrap();
        assert_eq!(plan.namer, "mock");
        let a = &plan.files[0];
        let keys: Vec<(&str, bool)> = a.keys.iter().map(|k| (k.key.as_str(), k.reused)).collect();
        assert_eq!(
            keys,
            [("common.save", true), ("common.delete_invoice", false)]
        );
        assert!(a.new.contains("const t = useTranslations();"), "{}", a.new);
        assert!(
            a.new.contains(r#"<button>{t("common.save")}</button>"#),
            "{}",
            a.new
        );
        apply(&mut ws, &plan).unwrap();
        ws.reload().unwrap();
        assert_eq!(
            ws.source().entries["common.delete_invoice"],
            "Delete invoice"
        );
        assert_eq!(ws.lock.notes["common.delete_invoice"], "shared text");
        let b = std::fs::read_to_string(root.join("app/b/page.tsx")).unwrap();
        assert!(b.contains(r#"t("common.delete_invoice")"#), "{b}");
    }
}
