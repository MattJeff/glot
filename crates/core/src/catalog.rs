use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{read_optional, write_file};
use crate::{Error, Key, Locale, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// Detect from file content (nested objects => nested), default nested for new files.
    #[default]
    Auto,
    JsonNested,
    JsonFlat,
    /// Apple String Catalog (`Localizable.xcstrings`, every locale in one file).
    Xcstrings,
    /// Apple legacy `.strings` + `.stringsdict` (by extension).
    Strings,
    /// Android `res/values-<locale>/strings.xml`.
    Android,
    /// Flutter ARB (`app_en.arb`).
    Arb,
    /// gettext `.po` / `.pot`.
    Po,
    /// YAML (Rails `config/locales/en.yml`, Symfony).
    Yaml,
}

/// One locale's messages, keys flattened to dot paths. Deterministic (sorted) order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub locale: Locale,
    pub entries: BTreeMap<Key, String>,
}

impl Catalog {
    pub fn new(locale: impl Into<Locale>) -> Self {
        Self {
            locale: locale.into(),
            entries: BTreeMap::new(),
        }
    }

    /// Read a JSON catalog. Missing file => empty catalog.
    pub fn read(path: &Path, locale: &str, _format: Format) -> Result<Self> {
        if let Some(p) = crate::formats::read(path, locale, _format)? {
            return Ok(Self {
                locale: locale.into(),
                entries: p.entries,
            });
        }
        if crate::formats::native(_format, path).is_some() {
            return Ok(Self::new(locale));
        }
        match read_optional(path)? {
            Some(json) => parse_at(&json, locale, path),
            None => Ok(Self::new(locale)),
        }
    }

    /// Merge several files of the same locale (config.files may list many).
    pub fn read_all(paths: &[PathBuf], locale: &str, format: Format) -> Result<Self> {
        let mut cat = Self::new(locale);
        for p in paths {
            cat.entries.extend(Self::read(p, locale, format)?.entries);
        }
        Ok(cat)
    }

    pub fn parse(json: &str, locale: &str) -> Result<Self> {
        parse_at(json, locale, Path::new("<inline>"))
    }

    /// Serialize (pretty, 2 spaces, trailing newline). Nested unless format == JsonFlat.
    pub fn to_json(&self, format: Format) -> String {
        let root = if format == Format::JsonFlat {
            Node::Branch(
                self.entries
                    .iter()
                    .map(|(k, v)| (k.clone(), Node::Leaf(v.clone())))
                    .collect(),
            )
        } else {
            let mut root = BTreeMap::new();
            for (k, v) in &self.entries {
                insert_nested(&mut root, k, v);
            }
            Node::Branch(root)
        };
        let mut out = serde_json::to_string_pretty(&root).expect("strings always serialize");
        out.push('\n');
        out
    }

    /// `Format::Auto` keeps the format of the existing file (nested for new files).
    /// An existing file keeps its key order (new keys go last in their parent object);
    /// files glot creates are sorted.
    pub fn write(&self, path: &Path, format: Format) -> Result<()> {
        let existing = read_optional(path)?;
        if let Some(f) = crate::formats::native(format, path) {
            return write_file(
                path,
                &crate::formats::write(f, path, existing.as_deref(), self),
            );
        }
        let format = match format {
            Format::Auto => existing
                .as_deref()
                .map_or(Format::JsonNested, Self::detect_format),
            f => f,
        };
        let merged = existing
            .as_deref()
            .and_then(|text| self.merge_into(text, format));
        write_file(path, &merged.unwrap_or_else(|| self.to_json(format)))
    }

    /// Update `text` (an existing catalog of the same `format`) in place: same order, changed values,
    /// removed keys dropped, new keys appended. `None` if it can't be reused as is.
    fn merge_into(&self, text: &str, format: Format) -> Option<String> {
        use serde_json::{Map, Value};
        let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(text) else {
            return None;
        };
        if Self::detect_format(text) != format && !root.is_empty() {
            return None;
        }
        fn update(
            map: &mut Map<String, Value>,
            prefix: &str,
            entries: &BTreeMap<Key, String>,
            seen: &mut std::collections::HashSet<Key>,
        ) {
            map.retain(|k, v| {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                match v {
                    Value::Object(child) => {
                        let was_empty = child.is_empty();
                        update(child, &key, entries, seen);
                        was_empty || !child.is_empty()
                    }
                    _ => match entries.get(&key) {
                        Some(new) => {
                            *v = Value::String(new.clone());
                            seen.insert(key);
                            true
                        }
                        None => false,
                    },
                }
            });
        }
        fn append(map: &mut Map<String, Value>, key: &str, value: &str, flat: bool) {
            let split = if flat { None } else { key.split_once('.') };
            match split {
                Some((head, rest)) => {
                    match map.entry(head).or_insert_with(|| Value::Object(Map::new())) {
                        Value::Object(child) => append(child, rest, value, flat),
                        // Same conflict rule as `to_json`: "a" is a leaf, keep "a.b" dotted here.
                        _ => {
                            map.insert(key.to_string(), Value::String(value.to_string()));
                        }
                    }
                }
                None => {
                    map.insert(key.to_string(), Value::String(value.to_string()));
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        update(&mut root, "", &self.entries, &mut seen);
        for (k, v) in self.entries.iter().filter(|(k, _)| !seen.contains(*k)) {
            append(&mut root, k, v, format == Format::JsonFlat);
        }
        let mut out = serde_json::to_string_pretty(&Value::Object(root)).ok()?;
        out.push('\n');
        Some(out)
    }

    /// Detect the format of an existing file content.
    /// Any nested object => nested; dotted top-level keys => flat; otherwise nested (equivalent).
    /// Unparseable content => Auto.
    pub fn detect_format(json: &str) -> Format {
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str(json) else {
            return Format::Auto;
        };
        if map.values().any(|v| v.is_object()) {
            Format::JsonNested
        } else if map.keys().any(|k| k.contains('.')) {
            Format::JsonFlat
        } else {
            Format::JsonNested
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum Node {
    Leaf(String),
    Branch(BTreeMap<String, Node>),
}

fn insert_nested(map: &mut BTreeMap<String, Node>, key: &str, value: &str) {
    match key.split_once('.') {
        None => {
            map.insert(key.to_string(), Node::Leaf(value.to_string()));
        }
        Some((head, rest)) => match map
            .entry(head.to_string())
            .or_insert_with(|| Node::Branch(BTreeMap::new()))
        {
            Node::Branch(child) => insert_nested(child, rest, value),
            // ponytail: "a" and "a.b" both exist; keep the deeper key dotted at this level.
            Node::Leaf(_) => {
                map.insert(key.to_string(), Node::Leaf(value.to_string()));
            }
        },
    }
}

fn parse_at(json: &str, locale: &str, path: &Path) -> Result<Catalog> {
    let err = |message: String| Error::Parse {
        path: path.to_path_buf(),
        message,
    };
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| err(e.to_string()))?;
    let serde_json::Value::Object(map) = value else {
        return Err(err("catalog root must be a JSON object".into()));
    };
    let mut cat = Catalog::new(locale);
    flatten(&map, "", &mut cat.entries).map_err(err)?;
    Ok(cat)
}

fn flatten(
    map: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    out: &mut BTreeMap<Key, String>,
) -> Result<(), String> {
    use serde_json::Value;
    for (k, v) in map {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::String(s) => {
                out.insert(key, s.clone());
            }
            Value::Object(child) => flatten(child, &key, out)?,
            other => {
                let kind = match other {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    _ => "array",
                };
                return Err(format!(
                    "key `{key}` has a {kind} value; catalog values must be strings or nested objects"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_roundtrip_sorted() {
        let c = Catalog::parse(
            r#"{"b": {"y": "Y", "x": "X"}, "a": "A", "c.d": "CD"}"#,
            "en",
        )
        .unwrap();
        assert_eq!(
            c.entries.keys().collect::<Vec<_>>(),
            ["a", "b.x", "b.y", "c.d"]
        );
        assert_eq!(
            c.to_json(Format::JsonNested),
            "{\n  \"a\": \"A\",\n  \"b\": {\n    \"x\": \"X\",\n    \"y\": \"Y\"\n  },\n  \"c\": {\n    \"d\": \"CD\"\n  }\n}\n"
        );
        assert_eq!(
            c.to_json(Format::JsonFlat),
            "{\n  \"a\": \"A\",\n  \"b.x\": \"X\",\n  \"b.y\": \"Y\",\n  \"c.d\": \"CD\"\n}\n"
        );
        assert_eq!(
            Catalog::parse(&c.to_json(Format::JsonNested), "en").unwrap(),
            c
        );
    }

    #[test]
    fn conflict_and_errors() {
        let mut c = Catalog::new("en");
        c.entries.insert("a".into(), "1".into());
        c.entries.insert("a.b".into(), "2".into());
        assert_eq!(
            Catalog::parse(&c.to_json(Format::JsonNested), "en").unwrap(),
            c
        );
        let e = Catalog::parse(r#"{"a": {"n": 3}}"#, "en")
            .unwrap_err()
            .to_string();
        assert!(e.contains("`a.n` has a number"), "{e}");
        assert!(Catalog::parse("[]", "en").is_err());
    }

    #[test]
    fn detect() {
        assert_eq!(
            Catalog::detect_format(r#"{"a": {"b": "c"}}"#),
            Format::JsonNested
        );
        assert_eq!(Catalog::detect_format(r#"{"a.b": "c"}"#), Format::JsonFlat);
        assert_eq!(Catalog::detect_format("nope"), Format::Auto);
    }

    #[test]
    fn write_keeps_existing_order() {
        let dir = std::env::temp_dir().join(format!("glot-catalog-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("fr.json");
        std::fs::write(
            &p,
            "{\n  \"zeta\": \"Z\",\n  \"nav\": {\n    \"home\": \"Accueil\",\n    \"about\": \"A propos\"\n  },\n  \"alpha\": \"A\"\n}\n",
        )
        .unwrap();
        let mut c = Catalog::read(&p, "fr", Format::Auto).unwrap();
        c.entries.insert("nav.contact".into(), "Contact".into());
        c.entries.insert("beta".into(), "B".into());
        c.entries.insert("zeta".into(), "Z2".into());
        c.entries.remove("alpha");
        c.write(&p, Format::Auto).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{\n  \"zeta\": \"Z2\",\n  \"nav\": {\n    \"home\": \"Accueil\",\n    \"about\": \"A propos\",\n    \"contact\": \"Contact\"\n  },\n  \"beta\": \"B\"\n}\n"
        );
        // Flat file: same, dotted keys appended at the top level.
        let q = dir.join("flat.json");
        std::fs::write(&q, r#"{"b.y": "1", "a.x": "2"}"#).unwrap();
        let mut f = Catalog::read(&q, "fr", Format::Auto).unwrap();
        f.entries.insert("a.a".into(), "3".into());
        f.write(&q, Format::Auto).unwrap();
        let keys: Vec<String> =
            match serde_json::from_str(&std::fs::read_to_string(&q).unwrap()).unwrap() {
                serde_json::Value::Object(m) => m.keys().cloned().collect(),
                _ => unreachable!(),
            };
        assert_eq!(keys, ["b.y", "a.x", "a.a"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_write_files() {
        let dir = std::env::temp_dir().join(format!("glot-catalog-{}", std::process::id()));
        let missing = Catalog::read(&dir.join("none.json"), "fr", Format::Auto).unwrap();
        assert!(missing.entries.is_empty());
        let p = dir.join("deep/fr.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"x.y": "old"}"#).unwrap();
        let mut c = Catalog::new("fr");
        c.entries.insert("x.y".into(), "é".into());
        c.write(&p, Format::Auto).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{\n  \"x.y\": \"é\"\n}\n"
        );
        let q = dir.join("other/fr.json");
        let mut d = Catalog::new("fr");
        d.entries.insert("z".into(), "Z".into());
        d.write(&q, Format::Auto).unwrap();
        let all = Catalog::read_all(&[p, q], "fr", Format::Auto).unwrap();
        assert_eq!(all.entries.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
