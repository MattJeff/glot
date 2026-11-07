//! JSON5 catalogs (leptos_i18n `locales/{locale}.json5`): comments, trailing commas, unquoted keys
//! and single quotes. A JSON5 document is a JS object literal, so the JS dictionary reader and
//! patcher do the work on `export default <file>`: comments, order and untouched values keep
//! their bytes. A new file is plain JSON (valid JSON5).
use std::path::Path;

use super::{Parsed, js};
use crate::catalog::{Catalog, Format};

const WRAP: &str = "export default ";
/// First property of the wrapped object: a JSON5 file is one locale, never `{ en: {…}, fr: {…} }`
/// (the JS reader would take `{ nav: {…}, app: {…} }` for that).
const PIN: &str = "\"\\u0001\": \"\",";
const PIN_KEY: &str = "\u{1}";

pub fn is_json5(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json5"))
}

/// Byte index of the document's opening `{` (after comments and whitespace).
fn open(text: &str) -> Option<usize> {
    let mut i = 0;
    loop {
        let r = &text[i..];
        let t = r.trim_start();
        i += r.len() - t.len();
        if t.starts_with("//") {
            i += t.find('\n')?;
        } else if t.starts_with("/*") {
            i += t.find("*/")? + 2;
        } else {
            return t.starts_with('{').then_some(i);
        }
    }
}

fn wrap(text: &str) -> Option<(String, usize)> {
    let o = open(text)? + 1;
    Some((format!("{WRAP}{}{PIN}{}", &text[..o], &text[o..]), o))
}

pub fn parse(text: &str, locale: &str) -> Result<Parsed, String> {
    let (w, _) = wrap(text).ok_or("JSON5: expected an object `{ … }`")?;
    let mut p = js::parse(&w, locale)?;
    p.entries.remove(PIN_KEY);
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let fresh = || cat.to_json(Format::JsonNested);
    let Some((w, o)) = existing.and_then(wrap) else {
        return fresh();
    };
    let mut pinned = cat.clone();
    pinned.entries.insert(PIN_KEY.into(), String::new());
    let out = js::write(Some(&w), &pinned);
    match out.strip_prefix(WRAP) {
        Some(body) if body.get(o..).is_some_and(|r| r.starts_with(PIN)) => {
            format!("{}{}", &body[..o], &body[o + PIN.len()..])
        }
        _ => fresh(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "// Accueil\n{\n  hello: 'Bonjour {{ name }}',\n  /* nav */\n  nav: {\n    home: \"Accueil\",\n    count_one: '{{ count }} clic',\n    count_other: '{{ count }} clics',\n  },\n  max: 0x10,\n}\n";

    #[test]
    fn reads_and_patches_in_place() {
        let p = parse(SRC, "fr").unwrap();
        assert_eq!(p.entries["hello"], "Bonjour {{ name }}");
        assert_eq!(p.entries["nav.home"], "Accueil");
        assert_eq!(p.entries["nav.count_one"], "{{ count }} clic");
        assert_eq!(p.entries.len(), 4, "numbers are not messages");
        let same = Catalog {
            locale: "fr".into(),
            entries: p.entries.clone(),
        };
        assert_eq!(write(Some(SRC), &same), SRC, "byte for byte");
        let mut c = same.clone();
        c.entries.insert("nav.home".into(), "Maison".into());
        c.entries.remove("hello");
        c.entries.insert("nav.about".into(), "À propos".into());
        let out = write(Some(SRC), &c);
        assert!(out.starts_with("// Accueil\n{\n  /* nav */"), "{out}");
        assert!(out.contains("home: \"Maison\","), "{out}");
        assert!(out.contains("max: 0x10,"), "{out}");
        assert_eq!(parse(&out, "fr").unwrap().entries, c.entries, "{out}");
        let fresh = write(None, &c);
        assert!(fresh.starts_with("{\n  \"nav\": {"), "{fresh}");
        assert_eq!(parse(&fresh, "fr").unwrap().entries, c.entries);
        // Two-letter root sections are keys, not locales.
        let nav = "{ nav: { home: 'Accueil' }, faq: { q: 'Question' } }";
        assert_eq!(parse(nav, "fr").unwrap().entries.len(), 2);
    }
}
