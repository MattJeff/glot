//! Compiled gettext: `.mo` (what `msgfmt` writes) and, for WordPress, the Jed JSON files of
//! `wp i18n make-json` that `wp_set_script_translations` loads.
//!
//! Like `msgfmt`, untranslated and `fuzzy` entries are left out (the app falls back to the msgid).
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use super::po::{Entry, entries};
use crate::{Error, Result};

/// Entries `msgfmt` keeps: the header, then every translated, non-fuzzy one.
fn kept(list: &[Entry]) -> impl Iterator<Item = &Entry> {
    list.iter().filter(|e| {
        let header = e.id.is_empty() && e.ctx.is_none();
        header || (!e.fuzzy && !e.strs.is_empty() && e.strs.iter().all(|s| !s.is_empty()))
    })
}

/// A `.mo` file: little-endian, no hash table (size 0, allowed by the format), msgids sorted.
pub fn compile(po: &str) -> Result<Vec<u8>, String> {
    let list = entries(po)?;
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = kept(&list)
        .map(|e| {
            let mut id = e.key().into_bytes();
            if let Some(p) = &e.id_plural {
                id.push(0);
                id.extend(p.as_bytes());
            }
            (id, e.strs.join("\0").into_bytes())
        })
        .collect();
    pairs.sort();
    let n = pairs.len() as u32;
    let (orig_at, trans_at) = (28, 28 + 8 * n);
    let mut at = 28 + 16 * n;
    let (mut table, mut data) = (vec![], vec![]);
    for side in [0, 1] {
        for p in &pairs {
            let s = if side == 0 { &p.0 } else { &p.1 };
            table.extend((s.len() as u32).to_le_bytes());
            table.extend(at.to_le_bytes());
            data.extend(s);
            data.push(0);
            at += s.len() as u32 + 1;
        }
    }
    let mut out = vec![];
    for w in [0x9504_12de_u32, 0, n, orig_at, trans_at, 0, 28 + 16 * n] {
        out.extend(w.to_le_bytes());
    }
    out.extend(table);
    out.extend(data);
    Ok(out)
}

fn header_field(list: &[Entry], field: &str) -> Option<String> {
    let h = list.iter().find(|e| e.id.is_empty() && e.ctx.is_none())?;
    h.strs.first()?.lines().find_map(|l| {
        let v = l.strip_prefix(field)?.strip_prefix(':')?.trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// WordPress script translations, as `wp i18n make-json` writes them: one Jed 1.x file per JS file
/// named in the `#:` references, `{domain}-{locale}-{md5(path)}.json` (`.min.js` maps to `.js`).
/// `None` unless the file (or its `.pot`) names a WordPress text domain (`X-Domain`, from `make-pot`).
pub fn wp_json(po: &str, base: &str, pot: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let list = entries(po)?;
    let pot_list = pot.and_then(|t| entries(t).ok()).unwrap_or_default();
    let Some(domain) =
        header_field(&list, "X-Domain").or_else(|| header_field(&pot_list, "X-Domain"))
    else {
        return Ok(vec![]);
    };
    let domain: String = domain
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let base = if base.starts_with(&domain) {
        base.to_string()
    } else {
        format!("{domain}-{base}")
    };
    let mut files: std::collections::BTreeMap<String, serde_json::Map<String, serde_json::Value>> =
        Default::default();
    for e in kept(&list).filter(|e| !e.id.is_empty() || e.ctx.is_some()) {
        for r in &e.refs {
            let file = r.rsplit_once(':').map_or(r.as_str(), |(f, _)| f);
            let Some(stem) = file.strip_suffix(".js") else {
                continue;
            };
            let file = format!("{}.js", stem.strip_suffix(".min").unwrap_or(stem));
            files
                .entry(file)
                .or_default()
                .insert(e.key(), serde_json::json!(e.strs));
        }
    }
    let config = serde_json::json!({
        "domain": "messages",
        "lang": header_field(&list, "Language").unwrap_or_else(|| "en".into()),
        "plural-forms": header_field(&list, "Plural-Forms")
            .unwrap_or_else(|| "nplurals=2; plural=(n != 1);".into()),
    });
    Ok(files
        .into_iter()
        .map(|(file, mut messages)| {
            messages.insert(String::new(), config.clone());
            let json = format!(
                "{{\"translation-revision-date\":{},\"generator\":\"glot\",\"source\":{},\"domain\":\"messages\",\"locale_data\":{{\"messages\":{}}}}}",
                serde_json::json!(header_field(&list, "PO-Revision-Date").unwrap_or_default()),
                serde_json::json!(file),
                serde_json::Value::Object(messages),
            );
            (format!("{base}-{}.json", md5_hex(&file)), json)
        })
        .collect())
}

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Write `{po}.mo` next to `path` (and the WordPress JSON files). Returns the files written.
pub fn compile_path(path: &Path) -> Result<Vec<PathBuf>> {
    let err = |message: String| Error::Parse {
        path: path.to_path_buf(),
        message,
    };
    let io = |p: &Path| {
        let p = p.to_path_buf();
        move |source| Error::Io { path: p, source }
    };
    let text = std::fs::read_to_string(path).map_err(io(path))?;
    let mo = path.with_extension("mo");
    std::fs::write(&mo, compile(&text).map_err(err)?).map_err(io(&mo))?;
    let mut out = vec![mo];
    let base = path.file_stem().unwrap_or_default().to_string_lossy();
    let pot = super::pot_template(path);
    for (name, json) in wp_json(&text, &base, pot.as_deref()).map_err(err)? {
        let p = path.with_file_name(name);
        std::fs::write(&p, json).map_err(io(&p))?;
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PO: &str = r#"msgid ""
msgstr ""
"Language: fr_FR\n"
"Content-Type: text/plain; charset=UTF-8\n"
"Plural-Forms: nplurals=2; plural=(n > 1);\n"
"X-Domain: my-plugin\n"

#: my-plugin.php:7 src/app.min.js:3
msgid "Hello world"
msgstr "Bonjour le monde"

#: src/app.js:9
#, php-format
msgid "%s item"
msgid_plural "%s items"
msgstr[0] "%s élément"
msgstr[1] "%s éléments"

msgctxt "menu"
msgid "File"
msgstr "Fichier"

#, fuzzy
msgid "Draft"
msgstr "Brouillon"

msgid "Untranslated"
msgstr ""
"#;

    fn u32_at(b: &[u8], i: usize) -> usize {
        u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize
    }

    fn string(b: &[u8], table: usize, i: usize) -> &[u8] {
        let (len, at) = (u32_at(b, table + 8 * i), u32_at(b, table + 8 * i + 4));
        assert_eq!(b[at + len], 0);
        &b[at..at + len]
    }

    #[test]
    fn mo_layout() {
        let b = compile(PO).unwrap();
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(u32_at(&b, 0), 0x9504_12de);
        assert_eq!(
            u32_at(&b, 8),
            4,
            "header + 3 translated, fuzzy/empty left out"
        );
        let (o, t) = (u32_at(&b, 12), u32_at(&b, 16));
        let origs: Vec<&[u8]> = (0..4).map(|i| string(&b, o, i)).collect();
        assert_eq!(
            origs,
            [
                &b""[..],
                b"%s item\0%s items",
                b"Hello world",
                b"menu\x04File"
            ]
        );
        assert_eq!(string(&b, t, 1), "%s élément\0%s éléments".as_bytes());
        assert!(string(&b, t, 0).starts_with(b"Language: fr_FR\n"));
    }

    #[test]
    fn wordpress_json() {
        let files = wp_json(PO, "my-plugin-fr_FR", None).unwrap();
        assert_eq!(files.len(), 1, "both references are src/app.js");
        let (name, json) = &files[0];
        assert_eq!(
            *name,
            format!("my-plugin-fr_FR-{}.json", md5_hex("src/app.js"))
        );
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let m = &v["locale_data"]["messages"];
        assert_eq!(v["source"], "src/app.js");
        assert_eq!(m["Hello world"], serde_json::json!(["Bonjour le monde"]));
        assert_eq!(
            m["%s item"],
            serde_json::json!(["%s élément", "%s éléments"])
        );
        assert_eq!(m[""]["plural-forms"], "nplurals=2; plural=(n > 1);");
        assert_eq!(m[""]["lang"], "fr_FR");
        // A theme's `fr_FR.po` gets the domain prefix WordPress looks for.
        let theme = wp_json(PO, "fr_FR", None).unwrap();
        assert!(theme[0].0.starts_with("my-plugin-fr_FR-"));
        // No text domain: not WordPress, no JSON.
        assert!(
            wp_json(&PO.replace("X-Domain", "X-Other"), "fr", None)
                .unwrap()
                .is_empty()
        );
    }
}
