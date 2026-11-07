//! Angular XMB / XTB (`ng extract-i18n --format xmb`): the source `messages.xmb`
//! (`<msg id desc meaning><source>file:line</source>text</msg>`) and one `messages.{locale}.xtb`
//! per target (`<translation id>text</translation>`).
//!
//! Key = `id`; `desc` / `meaning` are translator notes, `<source>` locations are skipped.
//! Placeholders `<ph name="INTERPOLATION"><ex>…</ex>…</ph>` (XMB) / `<ph name="INTERPOLATION"/>`
//! (XTB) read as `{INTERPOLATION}`; writing restores the unit's own markup, and a token the unit
//! lacks becomes `<ph name="…"/>`. ICU text (`{VAR_PLURAL, plural, …}`) is plain text and stays ICU.
use std::path::Path;

use super::xml::{self, Elem, Node};
use super::{Parsed, line_end, line_start};
use crate::catalog::Catalog;

struct Unit {
    key: String,
    elem: Elem,
    /// Where the message text starts (after XMB `<source>` locations).
    from: usize,
    text: String,
    /// `{NAME}` => the markup it was read from.
    map: Vec<(String, String)>,
}

fn elems(s: &str, from: usize, to: usize) -> Result<Vec<Elem>, String> {
    Ok(xml::children(s, from, to)?
        .into_iter()
        .filter_map(|n| match n {
            Node::Elem(e) => Some(e),
            Node::Comment(_) => None,
        })
        .collect())
}

/// The bundle element and its units.
fn doc(s: &str) -> Result<(Elem, Vec<Unit>), String> {
    let root = elems(s, 0, s.len())?
        .into_iter()
        .find(|e| e.tag == "messagebundle" || e.tag == "translationbundle")
        .ok_or("no <messagebundle> / <translationbundle> element")?;
    let mut units = vec![];
    for e in elems(s, root.open_end, root.close)? {
        if !matches!(e.tag.as_str(), "msg" | "translation") {
            continue;
        }
        let Some(key) = e.attr("id").map(str::to_string) else {
            continue;
        };
        let kids = elems(s, e.open_end, e.close)?;
        let from = kids
            .iter()
            .filter(|k| k.tag == "source")
            .map(|k| k.end)
            .max()
            .unwrap_or(e.open_end);
        let (mut text, mut map, mut i) = (String::new(), vec![], from);
        for k in kids.iter().filter(|k| k.start >= from) {
            text += &xml::unescape(&s[i..k.start]);
            i = k.end;
            let markup = s[k.start..k.end].to_string();
            let tok = match k.attr("name") {
                Some(n) if k.tag == "ph" && is_name(n) => format!("{{{n}}}"),
                _ => markup.clone(),
            };
            if !map.iter().any(|(t, _)| *t == tok) {
                map.push((tok.clone(), markup));
            }
            text += &tok;
        }
        text += &xml::unescape(&s[i..e.close]);
        units.push(Unit {
            key,
            elem: e,
            from,
            text,
            map,
        });
    }
    Ok((root, units))
}

/// Angular placeholder names: `INTERPOLATION`, `START_BOLD_TEXT`, `ICU_1`.
fn is_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

pub fn parse(s: &str, _locale: &str) -> Result<Parsed, String> {
    let (_, units) = doc(s)?;
    let mut p = Parsed::default();
    for u in units {
        let note: Vec<String> = [("desc", ""), ("meaning", "meaning: ")]
            .iter()
            .filter_map(|(a, pre)| u.elem.attr(a).map(|v| format!("{pre}{v}")))
            .collect();
        if !note.is_empty() {
            p.notes.insert(u.key.clone(), note.join("\n"));
        }
        p.entries.insert(u.key, u.text);
    }
    Ok(p)
}

/// Text back to XML: `{NAME}` tokens become the unit's markup (`<ph name="NAME"/>` when it has
/// none), the rest is escaped.
// ponytail: an all-caps `{WORD}` written as text in a message becomes a placeholder too.
fn from_text(v: &str, map: &[(String, String)]) -> String {
    let mut out = String::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        let rest = &v[i..];
        if let Some((t, m)) = map
            .iter()
            .filter(|(t, _)| rest.starts_with(t.as_str()))
            .max_by_key(|(t, _)| t.len())
        {
            out += m;
            i += t.len();
            continue;
        }
        if let Some(name) = rest
            .strip_prefix('{')
            .and_then(|r| r.split_once('}'))
            .map(|(n, _)| n)
            .filter(|n| is_name(n))
        {
            out += &format!("<ph name=\"{name}\"/>");
            i += name.len() + 2;
            continue;
        }
        let c = rest.chars().next().unwrap();
        match c {
            '&' => out += "&amp;",
            '<' => out += "&lt;",
            '>' => out += "&gt;",
            c => out.push(c),
        }
        i += c.len_utf8();
    }
    out
}

fn attr_escape(s: &str) -> String {
    xml::escape(s).replace('"', "&quot;")
}

pub fn write(existing: Option<&str>, cat: &Catalog, path: &Path) -> String {
    let xmb = path.extension().is_some_and(|e| e == "xmb");
    let fresh = if xmb {
        "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<messagebundle>\n</messagebundle>\n"
            .to_string()
    } else {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE translationbundle>\n<translationbundle lang=\"{}\">\n</translationbundle>\n",
            attr_escape(&cat.locale)
        )
    };
    let s = existing.unwrap_or(&fresh);
    let Ok((root, units)) = doc(s) else {
        return s.to_string();
    };
    let mut edits: Vec<(usize, usize, String)> = vec![];
    for u in &units {
        match cat.entries.get(&u.key) {
            None => edits.push((
                line_start(s, u.elem.start),
                line_end(s, u.elem.end),
                String::new(),
            )),
            Some(v) if *v == u.text => {}
            Some(v) => edits.push((u.from, u.elem.close, from_text(v, &u.map))),
        }
    }
    // New units before the bundle's end tag, indented like the others.
    let pad = units
        .first()
        .map(|u| {
            let ls = line_start(s, u.elem.start);
            if ls < u.elem.start {
                s[ls..u.elem.start].to_string()
            } else {
                String::new()
            }
        })
        .unwrap_or_else(|| "  ".into());
    let (tag, at) = (
        if xmb { "msg" } else { "translation" },
        line_start(s, root.close),
    );
    let mut add = String::new();
    for (k, v) in &cat.entries {
        if !units.iter().any(|u| u.key == *k) {
            add += &format!(
                "{pad}<{tag} id=\"{}\">{}</{tag}>\n",
                attr_escape(k),
                from_text(v, &[])
            );
        }
    }
    if !add.is_empty() {
        if !s[..at].ends_with('\n') {
            add.insert(0, '\n');
        }
        edits.push((at, at, add));
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = s.to_string();
    for (a, b, t) in edits {
        out.replace_range(a..b, &t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const XMB: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<!DOCTYPE messagebundle [
<!ELEMENT messagebundle (msg)*>
<!ATTLIST messagebundle class CDATA #IMPLIED>
<!ELEMENT msg (#PCDATA|ph|source)*>
<!ATTLIST msg id CDATA #IMPLIED>
<!ELEMENT ph (#PCDATA|ex)*>
<!ATTLIST ph name CDATA #REQUIRED>
<!ELEMENT ex (#PCDATA)>
<!ELEMENT source (#PCDATA)>
]>
<messagebundle handler="angular">
  <msg id="introductionHeader" desc="An introduction header for this sample" meaning="User welcome"><source>src/app/app.component.html:1,3</source>
  Hello i18n!
</msg>
  <msg id="4835017162512815845"><source>src/app/app.component.html:5</source>Hi <ph name="INTERPOLATION"><ex>{{ name }}</ex>{{ name }}</ph>, <ph name="START_BOLD_TEXT"><ex>&lt;b&gt;</ex>&lt;b&gt;</ph>welcome<ph name="CLOSE_BOLD_TEXT"><ex>&lt;/b&gt;</ex>&lt;/b&gt;</ph> &amp; enjoy</msg>
  <msg id="5115473329279458000"><source>src/app/app.component.html:9</source>{VAR_PLURAL, plural, =0 {just now} =1 {one minute ago} other {<ph name="INTERPOLATION"><ex>{{minutes}}</ex>{{minutes}}</ph> minutes ago} }</msg>
</messagebundle>
"#;

    #[test]
    fn xmb_source_and_xtb_targets() {
        let p = parse(XMB, "en").unwrap();
        assert_eq!(p.entries["introductionHeader"], "\n  Hello i18n!\n");
        assert_eq!(
            p.entries["4835017162512815845"],
            "Hi {INTERPOLATION}, {START_BOLD_TEXT}welcome{CLOSE_BOLD_TEXT} & enjoy"
        );
        assert_eq!(
            p.entries["5115473329279458000"],
            "{VAR_PLURAL, plural, =0 {just now} =1 {one minute ago} other {{INTERPOLATION} minutes ago} }"
        );
        assert_eq!(
            p.notes["introductionHeader"],
            "An introduction header for this sample\nmeaning: User welcome"
        );
        let en = Catalog {
            locale: "en".into(),
            entries: p.entries.clone(),
        };
        let path = Path::new("src/locale/messages.xmb");
        assert_eq!(write(Some(XMB), &en, path), XMB);
        // An edited source message keeps its `<source>` and placeholder markup.
        let mut e2 = en.clone();
        e2.entries.insert(
            "4835017162512815845".into(),
            "Hey {INTERPOLATION}, {START_BOLD_TEXT}welcome{CLOSE_BOLD_TEXT}".into(),
        );
        assert!(write(Some(XMB), &e2, path).contains("<source>src/app/app.component.html:5</source>Hey <ph name=\"INTERPOLATION\"><ex>{{ name }}</ex>{{ name }}</ph>, <ph name=\"START_BOLD_TEXT\">"));

        // A new XTB target, then patched in place.
        let mut fr = Catalog::new("fr");
        fr.entries
            .insert("introductionHeader".into(), "Bonjour i18n !".into());
        fr.entries.insert(
            "4835017162512815845".into(),
            "Salut {INTERPOLATION}, {START_BOLD_TEXT}bienvenue{CLOSE_BOLD_TEXT} & profitez".into(),
        );
        let xtb_path = Path::new("src/locale/messages.fr.xtb");
        let xtb = write(None, &fr, xtb_path);
        assert_eq!(
            xtb,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE translationbundle>\n<translationbundle lang=\"fr\">\n  <translation id=\"4835017162512815845\">Salut <ph name=\"INTERPOLATION\"/>, <ph name=\"START_BOLD_TEXT\"/>bienvenue<ph name=\"CLOSE_BOLD_TEXT\"/> &amp; profitez</translation>\n  <translation id=\"introductionHeader\">Bonjour i18n !</translation>\n</translationbundle>\n"
        );
        assert_eq!(parse(&xtb, "fr").unwrap().entries, fr.entries);
        assert_eq!(write(Some(&xtb), &fr, xtb_path), xtb);
        // Changed, removed and added units.
        let mut fr2 = fr.clone();
        fr2.entries.remove("introductionHeader");
        fr2.entries.insert(
            "4835017162512815845".into(),
            "Coucou {INTERPOLATION}".into(),
        );
        fr2.entries.insert(
            "5115473329279458000".into(),
            "{VAR_PLURAL, plural, other {{INTERPOLATION} min} }".into(),
        );
        let out = write(Some(&xtb), &fr2, xtb_path);
        assert_eq!(parse(&out, "fr").unwrap().entries, fr2.entries);
        assert!(!out.contains("introductionHeader"));
        assert!(out.contains("<translation id=\"5115473329279458000\">{VAR_PLURAL, plural, other {<ph name=\"INTERPOLATION\"/> min} }</translation>\n</translationbundle>"));
    }
}
