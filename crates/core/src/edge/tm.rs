//! glot Edge translation memory: what the proxy, the worker and the client patcher look segments up in.
//!
//! A segment is identified by `hash` (normalized source text) and `context` (`<role>|<page template>`):
//! "Save" on a button and "Save" in a price row are two entries. Lookup: exact context, then the
//! catalog default (`*`, a key whose source is exactly this text), and when every known translation of
//! the text agrees the entry is a single string valid in any context. Otherwise the text is ambiguous in
//! a new context: no translation, the segment is reported missing with its context.
//!
//! Wire format (`GET /edge/v1/{project}/{locale}/tm`): `{"v": 7, "locale": "es", "t": {"<hash>": "valor",
//! "<hash2>": {"button|/pricing": "Guardar", "td|/pricing": "Ahorra", "*": "Guardar"}}}`.
//! One snapshot = one version: a page is never translated with two versions mixed.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Context of catalog translations (keys whose source is exactly the segment text): used for any context.
pub const ANY: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Entry {
    /// Every known translation of the text agrees: valid in any context.
    One(String),
    /// Context-specific translations (`*` = catalog default).
    Ctx(BTreeMap<String, String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tm {
    /// `edge_catalog_version` of (project, locale) this snapshot was read at.
    pub v: i64,
    pub locale: String,
    pub t: BTreeMap<String, Entry>,
    /// Hashes with a translation for a context seen in a rendered page (not only a catalog key): what bundle
    /// localization may rewrite. Not on the wire (the patcher and the worker don't need it).
    #[serde(skip)]
    pub dom: BTreeSet<String>,
}

impl Tm {
    /// Translation of segment `hash` rendered in `context` (see [`context`]).
    pub fn lookup(&self, hash: &str, context: &str) -> Option<&str> {
        match self.t.get(hash)? {
            Entry::One(v) => Some(v),
            Entry::Ctx(m) => m.get(context).or_else(|| m.get(ANY)).map(String::as_str),
        }
    }

    /// Translation valid in any context of a segment seen in a rendered page (bundle localization: a string
    /// literal has no element context, so only unambiguous texts).
    pub fn lookup_rendered(&self, hash: &str) -> Option<&str> {
        match (self.dom.contains(hash), self.t.get(hash)?) {
            (true, Entry::One(v)) => Some(v),
            _ => None,
        }
    }

    /// Build a snapshot from `(hash, context, value)` rows; texts whose translations all agree collapse to one string.
    pub fn from_rows(
        locale: &str,
        v: i64,
        rows: impl IntoIterator<Item = (String, String, String)>,
    ) -> Self {
        let mut by_hash: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut dom = BTreeSet::new();
        for (h, c, value) in rows {
            if c != ANY {
                dom.insert(h.clone());
            }
            by_hash.entry(h).or_default().insert(c, value);
        }
        let t = by_hash
            .into_iter()
            .map(|(h, m)| {
                let mut values = m.values();
                let first = values.next().cloned().unwrap_or_default();
                let entry = if values.all(|x| *x == first) {
                    Entry::One(first)
                } else {
                    Entry::Ctx(m)
                };
                (h, entry)
            })
            .collect();
        Tm {
            v,
            locale: locale.into(),
            t,
            dom,
        }
    }
}

/// HTML whitespace collapsed and trimmed, NBSP kept (the segmenter's normalization).
pub fn normalize(text: &str) -> String {
    super::protect::collapse_ws(text)
}

/// Segment hash: `sha256(source_locale + "\0" + normalized text)`, first 16 bytes in hex (32 chars).
pub fn text_hash(source_locale: &str, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(source_locale.as_bytes());
    h.update([0u8]);
    h.update(text.as_bytes());
    h.finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `<role>|<page template>`: role = tag or ARIA role of the carrying element (`button`, `a`, `h1`, `title`,
/// `meta:description`…), template = [`template`] of the page path.
pub fn context(role: &str, url: &str, locales: &[String]) -> String {
    let role = role.trim().to_ascii_lowercase();
    let role = if !role.is_empty()
        && role.len() <= 32
        && role
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_'))
    {
        role
    } else {
        "text".into()
    };
    format!("{role}|{}", template(url, locales))
}

/// Path of a URL without scheme, host, query or fragment (`https://x.io/a?b#c` → `/a`). Never keeps the query:
/// it may carry visitor data.
pub fn path_only(url: &str) -> String {
    let rest = match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            after.find('/').map_or("/", |j| &after[j..])
        }
        None => url,
    };
    let path = rest.split(['?', '#']).next().unwrap_or("");
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Page template of a URL: its path with a leading locale segment removed, ids (`123`, uuids, long hex/tokens)
/// as `:id`, and a slug leaf under a section (`/blog/my-first-post`) as `*`. `/es/blog/42/` → `/blog/:id`.
// ponytail: heuristic templates; a per-project route list (`/blog/:slug`) is the upgrade if it misgroups pages.
pub fn template(url: &str, locales: &[String]) -> String {
    let path = path_only(url);
    let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts
        .first()
        .is_some_and(|f| locales.iter().any(|l| l.eq_ignore_ascii_case(f)))
    {
        parts.remove(0);
    }
    let n = parts.len();
    let out: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let digits = s.bytes().filter(u8::is_ascii_digit).count();
            let hexish = s.len() >= 8 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
            if digits == s.len() || (hexish && digits > 0) || s.len() >= 24 {
                ":id".into()
            } else if n >= 2 && i == n - 1 && s.contains(['-', '_']) {
                "*".into()
            } else {
                s.to_ascii_lowercase()
            }
        })
        .collect();
    format!("/{}", out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tm(rows: &[(&str, &str, &str)]) -> Tm {
        Tm::from_rows(
            "es",
            3,
            rows.iter()
                .map(|(h, c, v)| (h.to_string(), c.to_string(), v.to_string())),
        )
    }

    #[test]
    fn exact_context_then_unique_then_ambiguous() {
        let t = tm(&[
            ("save", "button|/", "Guardar"),
            ("save", "td|/pricing", "Ahorra"),
            ("home", "a|/", "Inicio"),
            ("home", "a|/pricing", "Inicio"),
            ("ok", "button|/", "Vale"),
            ("ok", "td|/", "Correcto"),
            ("ok", ANY, "Aceptar"),
        ]);
        assert_eq!(t.lookup("save", "button|/"), Some("Guardar"));
        assert_eq!(t.lookup("save", "td|/pricing"), Some("Ahorra"));
        // Two different translations, unknown context: ambiguous, reported missing.
        assert_eq!(t.lookup("save", "h1|/"), None);
        // All agree: collapsed, valid anywhere.
        assert_eq!(t.t["home"], Entry::One("Inicio".into()));
        assert_eq!(t.lookup("home", "a|/blog/*"), Some("Inicio"));
        // Catalog default for a new context.
        assert_eq!(t.lookup("ok", "label|/"), Some("Aceptar"));
        assert_eq!(t.lookup("ok", "td|/"), Some("Correcto"));
        assert_eq!(t.lookup("nope", "a|/"), None);
        // Wire format round-trips.
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains(r#""home":"Inicio""#), "{json}");
        assert_eq!(serde_json::from_str::<Tm>(&json).unwrap().t, t.t);
        assert!(!json.contains("dom"), "{json}");
        // Bundle localization: rendered + unambiguous only (never a catalog-only text).
        let mut t = t;
        t.t.insert("cat".into(), Entry::One("Solo catálogo".into()));
        assert_eq!(t.lookup_rendered("home"), Some("Inicio"));
        assert_eq!(t.lookup_rendered("save"), None);
        assert_eq!(t.lookup_rendered("ok"), None);
        assert_eq!(t.lookup_rendered("cat"), None);
    }

    #[test]
    fn hash_and_templates() {
        assert_eq!(text_hash("en", "Save").len(), 32);
        assert_ne!(text_hash("en", "Save"), text_hash("fr", "Save"));
        let l = vec!["es".to_string(), "fr".to_string()];
        assert_eq!(template("https://x.io/es/pricing?ref=me#a", &l), "/pricing");
        assert_eq!(template("/", &l), "/");
        assert_eq!(template("/blog/42/", &l), "/blog/:id");
        assert_eq!(template("/blog/my-first-post", &l), "/blog/*");
        assert_eq!(template("/explainer-video", &l), "/explainer-video");
        assert_eq!(
            template("/u/3f2a9c1e-77aa-4b1e-9d2b-0c1d2e3f4a5b/edit", &l),
            "/u/:id/edit"
        );
        assert_eq!(context("BUTTON", "/fr/pricing", &l), "button|/pricing");
        assert_eq!(context("<script>", "/", &l), "text|/");
        assert_eq!(path_only("https://x.io"), "/");
        assert_eq!(normalize("  a \n b  "), "a b");
        assert_eq!(normalize("20\u{a0}%"), "20\u{a0}%");
    }
}
