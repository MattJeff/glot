//! Translation memory: every validated translation (`.glot/tm.json`), reused exactly (0 token) or
//! fuzzily (≥ `FUZZY` similarity, shown to the model as an example). Shareable as TMX 1.4.
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{Error, Result};

pub const TM_FILE: &str = ".glot/tm.json";
/// Minimum normalized Levenshtein similarity of a fuzzy match.
pub const FUZZY: f64 = 0.85;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub source_locale: String,
    pub target_locale: String,
    pub source: String,
    pub target: String,
    /// Disambiguating note (glot.lock `notes`): "Save" the verb and "Save" the noun are two units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tm {
    pub units: Vec<Unit>,
}

/// Whitespace-collapsed form used for matching (case and punctuation kept: they are part of the copy).
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Other(format!("{}: {e}", path.display()))
}

// ponytail: linear scans over a JSON file; index by (locales, normalized source) or move to SQLite past ~100k units.
impl Tm {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(TM_FILE);
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| Error::Other(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(io(&path)(e)),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(TM_FILE);
        std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(io(&path))?;
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        std::fs::write(&path, text).map_err(io(&path))
    }

    /// Insert or replace (same locales, normalized source and context).
    pub fn add(&mut self, u: Unit) {
        let n = normalize(&u.source);
        match self.units.iter_mut().find(|x| {
            x.source_locale == u.source_locale
                && x.target_locale == u.target_locale
                && x.context == u.context
                && normalize(&x.source) == n
        }) {
            Some(x) => *x = u,
            None => self.units.push(u),
        }
    }

    /// Exact match: same locales, normalized source and context.
    pub fn exact(
        &self,
        src: &str,
        tgt: &str,
        source: &str,
        context: Option<&str>,
    ) -> Option<&Unit> {
        let n = normalize(source);
        self.units.iter().find(|u| {
            u.source_locale == src
                && u.target_locale == tgt
                && u.context.as_deref() == context
                && normalize(&u.source) == n
        })
    }

    /// Best match ≥ `FUZZY` for the locale pair, with its similarity (context ignored).
    pub fn fuzzy(&self, src: &str, tgt: &str, source: &str) -> Option<(&Unit, f64)> {
        let n = normalize(source);
        self.units
            .iter()
            .filter(|u| u.source_locale == src && u.target_locale == tgt)
            .map(|u| (u, strsim::normalized_levenshtein(&normalize(&u.source), &n)))
            .filter(|(_, s)| *s >= FUZZY)
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Load, add one unit, save (single validated translation: `glot set`, queue approval).
    pub fn remember(root: &Path, u: Unit) -> Result<()> {
        let mut tm = Self::load(root)?;
        tm.add(u);
        tm.save(root)
    }

    /// TMX 1.4: one `<tu>` per unit, the context as `<prop type="x-context">`.
    pub fn to_tmx(&self) -> String {
        let src = self
            .units
            .first()
            .map_or("en", |u| u.source_locale.as_str());
        let mut s = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tmx version=\"1.4\">\n  <header creationtool=\"glot\" creationtoolversion=\"{}\" segtype=\"sentence\" o-tmf=\"glot\" adminlang=\"en\" srclang=\"{}\" datatype=\"plaintext\"/>\n  <body>\n",
            env!("CARGO_PKG_VERSION"),
            esc(src)
        );
        for u in &self.units {
            s += &format!("    <tu srclang=\"{}\">\n", esc(&u.source_locale));
            if let Some(c) = &u.context {
                s += &format!("      <prop type=\"x-context\">{}</prop>\n", esc(c));
            }
            for (l, t) in [(&u.source_locale, &u.source), (&u.target_locale, &u.target)] {
                s += &format!(
                    "      <tuv xml:lang=\"{}\"><seg>{}</seg></tuv>\n",
                    esc(l),
                    esc(t)
                );
            }
            s += "    </tu>\n";
        }
        s + "  </body>\n</tmx>\n"
    }

    /// Units of a TMX file: each `<tu>`'s source variant (its `srclang`, else the header's, else
    /// the first `<tuv>`) paired with every other variant.
    pub fn from_tmx(text: &str) -> Result<Vec<Unit>> {
        let doc =
            roxmltree::Document::parse(text).map_err(|e| Error::Other(format!("TMX: {e}")))?;
        const XML: &str = "http://www.w3.org/XML/1998/namespace";
        let lang = |n: roxmltree::Node| {
            n.attribute((XML, "lang"))
                .or_else(|| n.attribute("lang"))
                .map(String::from)
        };
        let header = doc
            .descendants()
            .find(|n| n.has_tag_name("header"))
            .and_then(|h| h.attribute("srclang"))
            .filter(|l| *l != "*all*");
        let mut out = vec![];
        for tu in doc.descendants().filter(|n| n.has_tag_name("tu")) {
            let context = tu
                .children()
                .find(|n| n.has_tag_name("prop") && n.attribute("type") == Some("x-context"))
                .and_then(|n| n.text())
                .map(String::from);
            let tuvs: Vec<(String, String)> = tu
                .children()
                .filter(|n| n.has_tag_name("tuv"))
                .filter_map(|n| {
                    let seg = n.children().find(|c| c.has_tag_name("seg"))?;
                    // ponytail: inline markup (<ph>, <bpt>…) is flattened to its text.
                    let text: String = seg
                        .descendants()
                        .filter(|d| d.is_text())
                        .filter_map(|d| d.text())
                        .collect();
                    Some((lang(n)?, text))
                })
                .collect();
            let src = tu
                .attribute("srclang")
                .or(header)
                .map(String::from)
                .or_else(|| tuvs.first().map(|t| t.0.clone()));
            let Some(src) = src else { continue };
            let Some((_, source)) = tuvs.iter().find(|(l, _)| l.eq_ignore_ascii_case(&src)) else {
                continue;
            };
            for (l, t) in tuvs.iter().filter(|(l, _)| !l.eq_ignore_ascii_case(&src)) {
                out.push(Unit {
                    source_locale: src.clone(),
                    target_locale: l.clone(),
                    source: source.clone(),
                    target: t.clone(),
                    context: context.clone(),
                });
            }
        }
        Ok(out)
    }
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(s: &str, t: &str) -> Unit {
        Unit {
            source_locale: "en".into(),
            target_locale: "fr".into(),
            source: s.into(),
            target: t.into(),
            context: None,
        }
    }

    #[test]
    fn exact_fuzzy_tmx() {
        let mut tm = Tm::default();
        tm.add(unit("Save  changes", "Enregistrer"));
        tm.add(unit("Save changes", "Enregistrer les modifications"));
        tm.add(Unit {
            context: Some("noun & <b>".into()),
            ..unit("Delete <b>{name}</b>", "Supprimer <b>{name}</b>")
        });
        assert_eq!(tm.units.len(), 2);
        assert_eq!(
            tm.exact("en", "fr", " Save changes ", None).unwrap().target,
            "Enregistrer les modifications"
        );
        assert!(tm.exact("en", "de", "Save changes", None).is_none());
        let (u, s) = tm.fuzzy("en", "fr", "Save change").unwrap();
        assert!(s >= FUZZY && u.source == "Save changes");
        assert!(tm.fuzzy("en", "fr", "Cancel").is_none());

        let back = Tm::from_tmx(&tm.to_tmx()).unwrap();
        assert_eq!(back, tm.units);
        // Multi-variant TU from another tool.
        let other = r#"<tmx version="1.4"><header srclang="en-US"/><body><tu><tuv xml:lang="en-US"><seg>Hi</seg></tuv><tuv xml:lang="fr"><seg>Salut</seg></tuv><tuv xml:lang="de"><seg>Hallo</seg></tuv></tu></body></tmx>"#;
        let u = Tm::from_tmx(other).unwrap();
        assert_eq!(u.len(), 2);
        assert_eq!(
            (u[1].target_locale.as_str(), u[1].target.as_str()),
            ("de", "Hallo")
        );
    }
}
