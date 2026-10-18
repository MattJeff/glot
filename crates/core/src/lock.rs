use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::config::{read_optional, write_file};
use crate::{Error, Key, Locale, Result, catalog::Catalog, hash::hash};

pub const LOCK_FILE: &str = "glot.lock";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default)]
    pub notes: BTreeMap<Key, String>,
    #[serde(default)]
    pub entries: BTreeMap<Locale, BTreeMap<Key, LockEntry>>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            version: 1,
            notes: BTreeMap::new(),
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockEntry {
    pub source_hash: String,
    pub target_hash: String,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<u8>,
    #[serde(default)]
    pub reviewed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Origin {
    Machine { model: String },
    Human,
    Imported,
}

/// What needs doing, per (locale, key). Sorted, deterministic.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Key in source, absent (or empty) in target.
    pub missing: Vec<(Locale, Key)>,
    /// Source changed since the target was produced (source_hash mismatch).
    pub stale: Vec<(Locale, Key)>,
    /// Present in target but not in source.
    pub orphaned: Vec<(Locale, Key)>,
    /// Target edited outside glot (target_hash mismatch) — keep it, record as Human.
    pub manual_edits: Vec<(Locale, Key)>,
    /// Present in target but no lock entry (pre-existing translation) — record as Imported.
    pub untracked: Vec<(Locale, Key)>,
}

impl Lockfile {
    /// Missing file => default.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(LOCK_FILE);
        match read_optional(&path)? {
            Some(text) => serde_json::from_str(&text).map_err(|e| Error::Parse {
                path,
                message: e.to_string(),
            }),
            None => Ok(Self::default()),
        }
    }

    /// Pretty JSON, sorted keys, trailing newline.
    pub fn save(&self, root: &Path) -> Result<()> {
        let mut text =
            serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        text.push('\n');
        write_file(&root.join(LOCK_FILE), &text)
    }

    /// Each (locale, key) lands in at most one list. A manual edit wins over staleness so a
    /// human translation is never queued for overwrite; once recorded as Human, a later
    /// source change shows up as stale. Source keys with an empty value are ignored.
    pub fn plan(&self, source: &Catalog, targets: &[Catalog]) -> Plan {
        let mut plan = Plan::default();
        for target in targets {
            let loc = &target.locale;
            let locked = self.entries.get(loc);
            for (key, src) in &source.entries {
                if src.trim().is_empty() {
                    continue;
                }
                let item = (loc.clone(), key.clone());
                match (
                    target.entries.get(key).filter(|t| !t.trim().is_empty()),
                    locked.and_then(|l| l.get(key)),
                ) {
                    (None, _) => plan.missing.push(item),
                    (Some(_), None) => plan.untracked.push(item),
                    (Some(t), Some(e)) if e.target_hash != hash(t) => plan.manual_edits.push(item),
                    (Some(_), Some(e)) if e.source_hash != hash(src) => plan.stale.push(item),
                    _ => {}
                }
            }
            for key in target
                .entries
                .keys()
                .filter(|k| !source.entries.contains_key(*k))
            {
                plan.orphaned.push((loc.clone(), key.clone()));
            }
        }
        for list in [
            &mut plan.missing,
            &mut plan.stale,
            &mut plan.orphaned,
            &mut plan.manual_edits,
            &mut plan.untracked,
        ] {
            list.sort();
            list.dedup();
        }
        plan
    }

    /// Human-origin entries are recorded as reviewed.
    pub fn record(
        &mut self,
        locale: &str,
        key: &str,
        source: &str,
        target: &str,
        origin: Origin,
        score: Option<u8>,
    ) {
        let reviewed = origin == Origin::Human;
        let entry = LockEntry {
            source_hash: hash(source),
            target_hash: hash(target),
            origin,
            score,
            reviewed,
        };
        self.entries
            .entry(locale.to_string())
            .or_default()
            .insert(key.to_string(), entry);
    }

    pub fn remove(&mut self, locale: &str, key: &str) {
        if let Some(map) = self.entries.get_mut(locale) {
            map.remove(key);
            if map.is_empty() {
                self.entries.remove(locale);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(locale: &str, kv: &[(&str, &str)]) -> Catalog {
        let mut c = Catalog::new(locale);
        c.entries
            .extend(kv.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        c
    }

    #[test]
    fn plan_all_buckets() {
        let source = cat(
            "en",
            &[
                ("a", "Hello"),
                ("b", "Bye"),
                ("c", "New"),
                ("d", "Edited"),
                ("e", "Old"),
                ("f", "Empty"),
                ("g", ""),
            ],
        );
        let fr = cat(
            "fr",
            &[
                ("a", "Bonjour"),
                ("b", "Salut"),
                ("d", "Modifié à la main"),
                ("e", "Vieux"),
                ("f", " "),
                ("zz", "Orphelin"),
            ],
        );
        let ja = cat("ja", &[]);
        let mut lock = Lockfile::default();
        let m = || Origin::Machine { model: "m".into() };
        lock.record("fr", "a", "Hello", "Bonjour", m(), Some(90));
        lock.record("fr", "b", "Bye (old)", "Salut", m(), None);
        lock.record("fr", "d", "Edited", "Modifié", m(), None);
        lock.record("fr", "zz", "x", "Orphelin", m(), None);
        let p = lock.plan(&source, &[ja, fr]);
        let v = |l: &[(&str, &str)]| {
            l.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            p.missing,
            v(&[
                ("fr", "c"),
                ("fr", "f"),
                ("ja", "a"),
                ("ja", "b"),
                ("ja", "c"),
                ("ja", "d"),
                ("ja", "e"),
                ("ja", "f")
            ])
        );
        assert_eq!(p.stale, v(&[("fr", "b")]));
        assert_eq!(p.manual_edits, v(&[("fr", "d")]));
        assert_eq!(p.untracked, v(&[("fr", "e")]));
        assert_eq!(p.orphaned, v(&[("fr", "zz")]));
    }

    #[test]
    fn record_remove_save_load() {
        let mut lock = Lockfile::default();
        lock.notes.insert("a".into(), "note".into());
        lock.record("fr", "a", "Hello", "Bonjour", Origin::Human, None);
        assert!(lock.entries["fr"]["a"].reviewed);
        let dir = std::env::temp_dir().join(format!("glot-lock-{}", std::process::id()));
        assert_eq!(Lockfile::load(&dir).unwrap(), Lockfile::default());
        lock.save(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
        assert!(text.ends_with("}\n") && text.contains(r#""kind": "human""#));
        assert_eq!(Lockfile::load(&dir).unwrap(), lock);
        lock.remove("fr", "a");
        assert!(lock.entries.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
