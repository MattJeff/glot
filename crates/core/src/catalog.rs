use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
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
    /// TS/JS dictionary modules (`locales/fr.ts`: `export const fr = { ... }`), by extension.
    Js,
    /// XLIFF 1.2 / 2.0 (`messages.fr.xlf`: Angular, Symfony), by extension `.xlf` / `.xliff`.
    Xliff,
    /// Java / Spring `messages_fr.properties`.
    Properties,
    /// .NET `Resources.fr.resx`.
    Resx,
    /// Laravel `lang/{locale}/*.php` arrays.
    Php,
    /// JSON message objects: FormatJS extracted `{"id": {"defaultMessage", "description"}}`,
    /// WebExtensions `_locales/{locale}/messages.json` (by path, also under `auto`).
    JsonMessage,
}

/// How one catalog file maps to glot keys (from `glot.toml`, see [`crate::config::Config::catalog_files`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Nested-key separator (i18next `keySeparator`); empty = keys are never split (`keySeparator: false`).
    pub key_separator: String,
    /// i18next v4: `key_one`/`key_few`/`key_other` (`_zero`, `_ordinal_*`) are one message, an ICU
    /// `{count, plural, …}` in memory, re-split into the target locale's CLDR suffixes on write.
    pub i18next_plurals: bool,
    /// Namespace prefix of this file's keys (`home:` for `public/locales/{locale}/home.json`).
    pub ns: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            key_separator: ".".into(),
            i18next_plurals: false,
            ns: String::new(),
        }
    }
}

/// One locale's messages, keys flattened to dot paths. Deterministic (sorted) order.
/// Array items are `key[0]`, `key[1]`; numbers, booleans and `null` are not messages (kept as is on write).
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
    pub fn read(path: &Path, locale: &str, format: Format) -> Result<Self> {
        Self::read_with(path, locale, format, &Layout::default())
    }

    /// [`Catalog::read`] with the file's [`Layout`] (namespace prefix, i18next plurals, separator).
    pub fn read_with(path: &Path, locale: &str, format: Format, layout: &Layout) -> Result<Self> {
        let mut cat = if let Some(p) = crate::formats::read(path, locale, format)? {
            Self {
                locale: locale.into(),
                entries: p.entries,
            }
        } else if crate::formats::native(format, path).is_some() {
            Self::new(locale)
        } else {
            match read_optional(path)? {
                Some(json) => parse_at(&json, locale, path, layout)?,
                None => Self::new(locale),
            }
        };
        if !layout.ns.is_empty() {
            cat.entries = std::mem::take(&mut cat.entries)
                .into_iter()
                .map(|(k, v)| (format!("{}{k}", layout.ns), v))
                .collect();
        }
        Ok(cat)
    }

    /// Merge several files of the same locale (config.files may list many).
    pub fn read_all(paths: &[PathBuf], locale: &str, format: Format) -> Result<Self> {
        let mut cat = Self::new(locale);
        for p in paths {
            cat.entries.extend(Self::read(p, locale, format)?.entries);
        }
        Ok(cat)
    }

    /// Every catalog file of `locale` in `config`, each read with its layout.
    pub fn load(config: &crate::config::Config, root: &Path, locale: &str) -> Result<Self> {
        let mut cat = Self::new(locale);
        for (p, layout) in config.catalog_files(root, locale) {
            cat.entries
                .extend(Self::read_with(&p, locale, config.format, &layout)?.entries);
        }
        Ok(cat)
    }

    pub fn parse(json: &str, locale: &str) -> Result<Self> {
        parse_at(json, locale, Path::new("<inline>"), &Layout::default())
    }

    /// Serialize (pretty, 2 spaces, trailing newline). Nested unless format == JsonFlat.
    pub fn to_json(&self, format: Format) -> String {
        self.merge_into("{}", format, &Layout::default())
            .expect("an empty object always merges")
    }

    /// `Format::Auto` keeps the format of the existing file (nested for new files).
    /// An existing file keeps its key order (new keys go last in their parent object);
    /// files glot creates are sorted.
    pub fn write(&self, path: &Path, format: Format) -> Result<()> {
        self.write_with(path, format, &Layout::default())
    }

    /// [`Catalog::write`] with the file's [`Layout`]: keys outside its namespace prefix are written
    /// as is, inside it without the prefix. An existing file whose content would not change is
    /// left untouched (same bytes).
    pub fn write_with(&self, path: &Path, format: Format, layout: &Layout) -> Result<()> {
        let own;
        let cat = if layout.ns.is_empty() {
            self
        } else {
            own = Self {
                locale: self.locale.clone(),
                entries: self
                    .entries
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.strip_prefix(&layout.ns).unwrap_or(k).to_string(),
                            v.clone(),
                        )
                    })
                    .collect(),
            };
            &own
        };
        if let Some(f) = crate::formats::native(format, path) {
            return crate::formats::write_path(f, path, cat);
        }
        let existing = read_optional(path)?;
        let format = match format {
            Format::Auto => existing
                .as_deref()
                .map_or(Format::JsonNested, Self::detect_format),
            f => f,
        };
        if let Some(text) = existing.as_deref()
            && let Some(merged) = cat.merge_into(text, format, layout)
        {
            let same = serde_json::from_str::<Value>(text).ok()
                == serde_json::from_str::<Value>(&merged).ok();
            return if same {
                Ok(())
            } else {
                write_file(path, &merged)
            };
        }
        let out = cat
            .merge_into("{}", format, layout)
            .expect("an empty object always merges");
        write_file(path, &out)
    }

    /// Update `text` (an existing catalog of the same `format`) in place: same order and
    /// indentation, changed values, removed keys dropped, new keys appended, non-message values
    /// (numbers, booleans, `null`) kept. `None` if it can't be reused as is.
    fn merge_into(&self, text: &str, format: Format, layout: &Layout) -> Option<String> {
        let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else {
            return None;
        };
        if Self::detect_format(text) != format && !root.is_empty() {
            return None;
        }
        let ordered: Vec<(Key, String)> = if layout.i18next_plurals {
            unfold_plurals(&self.entries)
        } else {
            // easy_localization / slang / i18n-js: `{_count, plural, …}` => `{"one": …, "other": …}`.
            crate::formats::json::unfold_plurals(
                self.entries
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                &layout.key_separator,
            )
        };
        let entries: BTreeMap<&str, &str> = ordered
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let sep = layout.key_separator.as_str();
        let mut seen = HashSet::new();
        let mut root = Value::Object(root);
        update(&mut root, "", sep, &entries, &mut seen);
        let Value::Object(map) = &mut root else {
            unreachable!()
        };
        let split = if format == Format::JsonFlat { "" } else { sep };
        for (k, v) in ordered.iter().filter(|(k, _)| !seen.contains(k.as_str())) {
            append(map, k, v, split);
        }
        let indent = text
            .lines()
            .find_map(|l| {
                let ws = &l[..l.len() - l.trim_start().len()];
                (!ws.is_empty() && l.trim_start().starts_with('"')).then_some(ws)
            })
            .unwrap_or("  ");
        let mut out = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
        root.serialize(&mut serde_json::Serializer::with_formatter(&mut out, fmt))
            .ok()?;
        let mut out = String::from_utf8(out).ok()?;
        out.push('\n');
        Some(out)
    }

    /// Detect the format of an existing file content.
    /// Any nested object => nested; dotted top-level keys => flat; otherwise nested (equivalent).
    /// Unparseable content => Auto.
    pub fn detect_format(json: &str) -> Format {
        let Ok(Value::Object(map)) = serde_json::from_str(json) else {
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

fn join(prefix: &str, k: &str, sep: &str) -> String {
    match (prefix, sep) {
        ("", _) => k.to_string(),
        // `keySeparator: false` files with objects anyway: show them dotted.
        (p, "") => format!("{p}.{k}"),
        (p, s) => format!("{p}{s}{k}"),
    }
}

/// Write `entries` into `v` (the file value at `key`); `false` = drop `v`.
fn update(
    v: &mut Value,
    key: &str,
    sep: &str,
    entries: &BTreeMap<&str, &str>,
    seen: &mut HashSet<String>,
) -> bool {
    match v {
        Value::Object(map) => {
            let was_empty = map.is_empty();
            map.retain(|k, c| update(c, &join(key, k, sep), sep, entries, seen));
            was_empty || !map.is_empty()
        }
        Value::Array(items) => {
            let was_empty = items.is_empty();
            let mut i = 0;
            items.retain_mut(|c| {
                i += 1;
                update(c, &format!("{key}[{}]", i - 1), sep, entries, seen)
            });
            was_empty || !items.is_empty()
        }
        Value::String(s) => match entries.get(key) {
            Some(new) => {
                *s = new.to_string();
                seen.insert(key.to_string());
                true
            }
            None => key == SCHEMA,
        },
        // Numbers, booleans, null: not messages, kept verbatim.
        _ => true,
    }
}

/// Insert a new key, creating its parent objects (and arrays for `name[0]` segments).
fn append(map: &mut Map<String, Value>, key: &str, value: &str, sep: &str) {
    if !place(map, key, value, sep) {
        // Same conflict rule as before: "a" is a leaf, keep "a.b" dotted here.
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn place(map: &mut Map<String, Value>, key: &str, value: &str, sep: &str) -> bool {
    let (seg, rest) = match split_key(key, sep) {
        Some((h, r)) => (h, Some(r)),
        None => (key, None),
    };
    let Some(slot) = slot(map, seg) else {
        return false;
    };
    match rest {
        None => {
            *slot = Value::String(value.to_string());
            true
        }
        Some(rest) => {
            if slot.is_null() {
                *slot = Value::Object(Map::new());
            }
            match slot {
                Value::Object(child) => {
                    append(child, rest, value, sep);
                    true
                }
                _ => false,
            }
        }
    }
}

/// The value for one key segment (`name`, `name[0]`, `name[1][0]`), created (`null`) if missing.
fn slot<'a>(map: &'a mut Map<String, Value>, seg: &str) -> Option<&'a mut Value> {
    let Some((name, idx)) = array_seg(seg) else {
        return Some(map.entry(seg).or_insert(Value::Null));
    };
    if map.get(name).is_some_and(|v| !v.is_array()) {
        return None;
    }
    let mut v = map.entry(name).or_insert_with(|| Value::Array(vec![]));
    for i in idx {
        if v.is_null() {
            *v = Value::Array(vec![]);
        }
        let Value::Array(a) = v else { return None };
        if a.len() <= i {
            a.resize(i + 1, Value::Null);
        }
        v = &mut a[i];
    }
    Some(v)
}

/// `list[0]` => (`list`, [0]); `None` for a plain segment.
fn array_seg(seg: &str) -> Option<(&str, Vec<usize>)> {
    let open = seg.find('[').filter(|&i| i > 0)?;
    let (name, mut rest) = seg.split_at(open);
    let mut idx = vec![];
    while let Some(r) = rest.strip_prefix('[') {
        let close = r.find(']')?;
        idx.push(r[..close].parse().ok()?);
        rest = &r[close + 1..];
    }
    rest.is_empty().then_some((name, idx))
}

/// Split at the first separator that joins two key segments. Natural-language keys
/// (`keySeparator: false` style) are never cut: a key ending with the separator
/// (`"Welcome back."`) or a separator next to whitespace (`"Done. Next"`) stays part of the key.
fn split_key<'k>(key: &'k str, sep: &str) -> Option<(&'k str, &'k str)> {
    if sep.is_empty() || key.ends_with(sep) {
        return None;
    }
    key.match_indices(sep).find_map(|(i, _)| {
        let (head, rest) = (&key[..i], &key[i + sep.len()..]);
        let ws = |c: Option<char>| c.is_none_or(char::is_whitespace);
        (!ws(head.chars().last()) && !ws(rest.chars().next())).then_some((head, rest))
    })
}

fn parse_at(json: &str, locale: &str, path: &Path, layout: &Layout) -> Result<Catalog> {
    let err = |message: String| Error::Parse {
        path: path.to_path_buf(),
        message,
    };
    let value: Value = serde_json::from_str(json).map_err(|e| err(e.to_string()))?;
    if !value.is_object() {
        return Err(err("catalog root must be a JSON object".into()));
    }
    let mut cat = Catalog::new(locale);
    flatten(&value, "", &layout.key_separator, &mut cat.entries);
    if layout.i18next_plurals {
        fold_plurals(&mut cat.entries, locale);
    } else {
        crate::formats::json::fold_plurals(&mut cat.entries, &layout.key_separator, locale);
    }
    Ok(cat)
}

const SCHEMA: &str = "$schema";

fn flatten(v: &Value, key: &str, sep: &str, out: &mut BTreeMap<Key, String>) {
    match v {
        // `"$schema"` (inlang message format, editors): metadata, not a message.
        Value::String(_) if key == SCHEMA => {}
        Value::String(s) => {
            out.insert(key.to_string(), s.clone());
        }
        Value::Object(map) => {
            for (k, c) in map {
                flatten(c, &join(key, k, sep), sep, out);
            }
        }
        Value::Array(items) => {
            for (i, c) in items.iter().enumerate() {
                flatten(c, &format!("{key}[{i}]"), sep, out);
            }
        }
        // Numbers, booleans, null (i18next `returnObjects` data, flags): not messages.
        _ => {}
    }
}

// ---------------------------------------------------------------- i18next v4 plurals

const CATS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// `item_one` => (`item`, cardinal, `one`); `place_ordinal_two` => (`place`, ordinal, `two`).
fn plural_member(key: &str) -> Option<(&str, bool, &str)> {
    let (base, cat) = key.rsplit_once('_')?;
    if !CATS.contains(&cat) {
        return None;
    }
    let (base, ordinal) = match base.strip_suffix("_ordinal") {
        Some(b) => (b, true),
        None => (base, false),
    };
    (!base.is_empty()).then_some((base, ordinal, cat))
}

/// Fold `base_<category>` siblings into `base` = `{count, plural, one {…} other {…}}`
/// (`selectordinal` for `_ordinal_*`). `_zero` is i18next's "count is 0" form: `=0`, or the CLDR
/// `zero` category in locales that have one. Folded when the family has `_other` plus another
/// form (or a lone `_other` using `{{count}}`, e.g. ja), `base` itself is free and the texts
/// survive the round trip.
fn fold_plurals(entries: &mut BTreeMap<Key, String>, locale: &str) {
    let mut fams: BTreeMap<(String, bool), Vec<(&'static str, Key)>> = BTreeMap::new();
    for k in entries.keys() {
        if let Some((base, ordinal, cat)) = plural_member(k) {
            let cat = CATS.into_iter().find(|c| *c == cat).expect("listed");
            fams.entry((base.to_string(), ordinal))
                .or_default()
                .push((cat, k.clone()));
        }
    }
    let cldr_zero =
        crate::plural::categories(locale).contains(&crate::plural::PluralCategory::Zero);
    // Cardinal before ordinal: a base with both keeps its `_ordinal_*` keys as they are.
    for ((base, ordinal), mut members) in fams {
        let Some((_, other)) = members.iter().find(|(c, _)| *c == "other") else {
            continue;
        };
        if entries.contains_key(&base) || (members.len() < 2 && !entries[other].contains("{{count"))
        {
            continue;
        }
        members.sort_by_key(|(c, _)| CATS.iter().position(|x| x == c));
        let forms: Vec<(String, String)> = members
            .iter()
            .map(|(c, k)| {
                let sel = if *c == "zero" && !cldr_zero { "=0" } else { c };
                (sel.to_string(), entries[k].clone())
            })
            .collect();
        let kind = if ordinal { "selectordinal" } else { "plural" };
        let folded = crate::formats::icu_choice("count", kind, &forms);
        if crate::formats::whole_choice(&folded).map(|c| c.forms) != Some(forms) {
            continue;
        }
        for (_, k) in &members {
            entries.remove(k);
        }
        entries.insert(base, folded);
    }
}

/// Inverse of [`fold_plurals`], in write order: each plural message becomes `base_<category>`
/// keys (CLDR order, `=0`/`zero` => `_zero`); other entries unchanged.
fn unfold_plurals(entries: &BTreeMap<Key, String>) -> Vec<(Key, String)> {
    let mut out: Vec<(Key, String)> = vec![];
    for (k, v) in entries {
        let choice = crate::formats::folded_choice(v).filter(|c| {
            c.var == "count"
                && matches!(c.kind.as_str(), "plural" | "selectordinal")
                && c.forms
                    .iter()
                    .all(|(f, _)| f == "=0" || CATS.contains(&f.as_str()))
        });
        let Some(c) = choice else {
            out.push((k.clone(), v.clone()));
            continue;
        };
        let mid = if c.kind == "selectordinal" {
            "_ordinal"
        } else {
            ""
        };
        let mut forms = c.forms.clone();
        // `=0` first so a CLDR `zero` form (ar, lv) wins the `_zero` key.
        forms.sort_by_key(|(f, _)| CATS.iter().position(|x| x == f).map_or(0, |p| p + 1));
        let mut keys: BTreeMap<String, String> = BTreeMap::new();
        let mut order = vec![];
        for (f, text) in forms {
            let cat = if f == "=0" { "zero" } else { f.as_str() };
            let key = format!("{k}{mid}_{cat}");
            if keys.insert(key.clone(), text).is_none() {
                order.push(key);
            }
        }
        out.extend(order.into_iter().map(|key| {
            let text = keys.remove(&key).expect("inserted");
            (key, text)
        }));
    }
    out
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
        assert!(Catalog::parse("[]", "en").is_err());
        assert!(Catalog::parse("{", "en").is_err());
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

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glot-catalog-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn schema_is_metadata() {
        let src = r#"{"$schema":"https://inlang.com/schema/inlang-message-format","hi":"Hi"}"#;
        let c = Catalog::parse(src, "en").unwrap();
        assert_eq!(c.entries.keys().collect::<Vec<_>>(), ["hi"]);
        let p = tmp("schema").join("en.json");
        std::fs::write(&p, src).unwrap();
        let mut t = c.clone();
        t.entries.insert("hi".into(), "Hello".into());
        t.write(&p, Format::Auto).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"$schema":"https://inlang.com/schema/inlang-message-format","hi":"Hello"})
        );
    }

    #[test]
    fn non_string_values_are_kept_and_arrays_translated() {
        // i18next `returnObjects`, ngx-translate / vue data: arrays, numbers, booleans, null, objects in arrays.
        let src = r#"{"list":["a","b"],"n":1,"flag":true,"none":null,"cards":[{"title":"T","w":2}],"ok":"Ok"}"#;
        let c = Catalog::parse(src, "en").unwrap();
        assert_eq!(
            c.entries.keys().collect::<Vec<_>>(),
            ["cards[0].title", "list[0]", "list[1]", "ok"]
        );
        let dir = tmp("arr");
        let p = dir.join("en.json");
        std::fs::write(&p, src).unwrap();
        // Unchanged content: the file keeps its bytes (compact, no trailing newline).
        Catalog::read(&p, "en", Format::Auto)
            .unwrap()
            .write(&p, Format::Auto)
            .unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), src);
        // Element-wise update; non-message values stay.
        let mut t = c.clone();
        t.entries.insert("list[1]".into(), "B".into());
        t.write(&p, Format::Auto).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"list":["a","B"],"n":1,"flag":true,"none":null,"cards":[{"title":"T","w":2}],"ok":"Ok"})
        );
        // A new target file gets real arrays.
        let q = dir.join("fr.json");
        let mut fr = Catalog::new("fr");
        for (k, v) in [
            ("list[0]", "x"),
            ("list[1]", "y"),
            ("cards[0].title", "Titre"),
        ] {
            fr.entries.insert(k.into(), v.into());
        }
        fr.write(&q, Format::Auto).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&q).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"cards":[{"title":"Titre"}],"list":["x","y"]})
        );
        assert_eq!(Catalog::read(&q, "fr", Format::Auto).unwrap(), fr);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn natural_language_keys_are_never_split() {
        // `keySeparator: false` projects: the source text is the key.
        let dir = tmp("nat");
        let p = dir.join("fr.json");
        let mut c = Catalog::new("fr");
        for k in [
            "Welcome back.",
            "Save",
            "Done. Next",
            "menu.a",
            "menu.Open file",
        ] {
            c.entries.insert(k.into(), format!("<{k}>"));
        }
        c.write(&p, Format::JsonNested).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"Done. Next":"<Done. Next>","Save":"<Save>","Welcome back.":"<Welcome back.>","menu":{"Open file":"<menu.Open file>","a":"<menu.a>"}})
        );
        assert_eq!(Catalog::read(&p, "fr", Format::Auto).unwrap(), c);
        // keySeparator: false => nothing is split, not even `menu.a`.
        let flat = Layout {
            key_separator: String::new(),
            ..Layout::default()
        };
        std::fs::remove_file(&p).unwrap();
        c.write_with(&p, Format::Auto, &flat).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.get("menu.a").is_some(), "{v}");
        assert_eq!(
            Catalog::read_with(&p, "fr", Format::Auto, &flat).unwrap(),
            c
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn i18next_plural_families() {
        let l = Layout {
            i18next_plurals: true,
            ..Layout::default()
        };
        let src = r#"{"item_one":"{{count}} item","item_other":"{{count}} items","friend_male":"He","friend_female":"She","place_ordinal_one":"{{count}}st","place_ordinal_two":"{{count}}nd","place_ordinal_few":"{{count}}rd","place_ordinal_other":"{{count}}th","msg_zero":"None","msg_other":"{{count}} msgs","show_other":"Show"}"#;
        let c = parse_at(src, "en", Path::new("x"), &l).unwrap();
        assert_eq!(
            c.entries["item"],
            "{count, plural, one {{{count}} item} other {{{count}} items}}"
        );
        assert_eq!(
            c.entries["place"],
            "{count, selectordinal, one {{{count}}st} two {{{count}}nd} few {{{count}}rd} other {{{count}}th}}"
        );
        assert_eq!(
            c.entries["msg"],
            "{count, plural, =0 {None} other {{{count}} msgs}}"
        );
        // Context keys stay; a lone `_other` without {{count}} is a plain key.
        assert_eq!(c.entries["friend_male"], "He");
        assert_eq!(c.entries["show_other"], "Show");

        // pl target: exactly pl's categories, written as suffixed keys; ja: `_other` only.
        let dir = tmp("plural");
        let p = dir.join("pl.json");
        std::fs::write(
            &p,
            "{\n  \"item_one\": \"old\",\n  \"item_other\": \"old\",\n  \"item_two\": \"x\"\n}\n",
        )
        .unwrap();
        let mut pl = Catalog::new("pl");
        pl.entries.insert(
            "item".into(),
            "{count, plural, one {1 rzecz} few {{{count}} rzeczy} many {{{count}} rzeczy} other {{{count}} rzeczy}}".into(),
        );
        pl.entries.insert(
            "msg".into(),
            "{count, plural, =0 {Brak} one {a} few {b} many {c} other {d}}".into(),
        );
        pl.write_with(&p, Format::Auto, &l).unwrap();
        let keys: Vec<String> =
            match serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap() {
                Value::Object(m) => m.keys().cloned().collect(),
                _ => unreachable!(),
            };
        assert_eq!(
            keys,
            [
                "item_one",
                "item_other",
                "item_few",
                "item_many",
                "msg_zero",
                "msg_one",
                "msg_few",
                "msg_many",
                "msg_other"
            ]
        );
        assert_eq!(Catalog::read_with(&p, "pl", Format::Auto, &l).unwrap(), pl);
        // Unchanged => untouched bytes.
        let before = std::fs::read(&p).unwrap();
        pl.write_with(&p, Format::Auto, &l).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), before);
        // Without the i18next layout an ICU plural stays one key (next-intl).
        let q = dir.join("icu.json");
        pl.write(&q, Format::Auto).unwrap();
        assert_eq!(Catalog::read(&q, "pl", Format::Auto).unwrap(), pl);
        // A lone translated `_other` with {{count}} (ja) folds.
        let ja = parse_at(r#"{"item_other":"{{count}} 個"}"#, "ja", Path::new("x"), &l).unwrap();
        assert_eq!(ja.entries["item"], "{count, plural, other {{{count}} 個}}");
        // Arabic has a CLDR `zero`: `_zero` is that category.
        let ar = parse_at(
            r#"{"n_zero":"a","n_one":"b","n_other":"c"}"#,
            "ar",
            Path::new("x"),
            &l,
        )
        .unwrap();
        assert_eq!(
            ar.entries["n"],
            "{count, plural, zero {a} one {b} other {c}}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn namespace_prefix() {
        let dir = tmp("ns");
        let p = dir.join("home.json");
        std::fs::write(&p, r#"{"title":"Welcome home"}"#).unwrap();
        let l = Layout {
            ns: "home:".into(),
            ..Layout::default()
        };
        let mut c = Catalog::read_with(&p, "en", Format::Auto, &l).unwrap();
        assert_eq!(c.entries.keys().collect::<Vec<_>>(), ["home:title"]);
        c.entries.insert("home:sub.x".into(), "X".into());
        c.write_with(&p, Format::Auto, &l).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{\n  \"title\": \"Welcome home\",\n  \"sub\": {\n    \"x\": \"X\"\n  }\n}\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
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
                Value::Object(m) => m.keys().cloned().collect(),
                _ => unreachable!(),
            };
        assert_eq!(keys, ["b.y", "a.x", "a.a"]);
        // 4-space files stay 4-space.
        let r = dir.join("four.json");
        std::fs::write(&r, "{\n    \"a\": \"A\"\n}\n").unwrap();
        let mut g = Catalog::read(&r, "fr", Format::Auto).unwrap();
        g.entries.insert("b".into(), "B".into());
        g.write(&r, Format::Auto).unwrap();
        assert_eq!(
            std::fs::read_to_string(&r).unwrap(),
            "{\n    \"a\": \"A\",\n    \"b\": \"B\"\n}\n"
        );
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
