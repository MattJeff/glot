//! .NET `.resx` (`Resources.resx` = source, `Resources.fr.resx`): `<data name><value>` strings,
//! `<comment>` = translator note. Non-string resources (`type=` / `mimetype=`: images, files) are
//! left alone. Writing patches `<value>` in place; the schema and `<resheader>`s are kept verbatim.
use std::collections::BTreeSet;

use super::xml::{self, Elem, Node};
use super::{Parsed, line_end, line_start};
use crate::catalog::Catalog;

const SKELETON: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<root>
  <resheader name="resmimetype">
    <value>text/microsoft-resx</value>
  </resheader>
  <resheader name="version">
    <value>2.0</value>
  </resheader>
  <resheader name="reader">
    <value>System.Resources.ResXResourceReader, System.Windows.Forms, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089</value>
  </resheader>
  <resheader name="writer">
    <value>System.Resources.ResXResourceWriter, System.Windows.Forms, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089</value>
  </resheader>
</root>
"#;

struct Entry {
    name: String,
    data: Elem,
    value: Option<Elem>,
    note: Option<String>,
}

fn root(s: &str) -> Result<(usize, usize), String> {
    let open = s.find("<root").ok_or("no <root> element")?;
    let start = open + s[open..].find('>').ok_or("unclosed <root>")? + 1;
    let end = s.rfind("</root>").ok_or("no </root>")?;
    Ok((start, end))
}

fn text(s: &str, e: &Elem) -> String {
    let raw = &s[e.open_end..e.close];
    match raw
        .trim()
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
    {
        Some(c) => c.to_string(),
        None => xml::unescape(raw),
    }
}

fn entries(s: &str) -> Result<Vec<Entry>, String> {
    let (from, to) = root(s)?;
    let mut out = vec![];
    for node in xml::children(s, from, to)? {
        let Node::Elem(e) = node else { continue };
        if e.tag != "data" || e.attr("type").is_some() || e.attr("mimetype").is_some() {
            continue;
        }
        let Some(name) = e.attr("name").map(str::to_string) else {
            continue;
        };
        let (mut value, mut note) = (None, None);
        for c in xml::children(s, e.open_end, e.close)? {
            match c {
                Node::Elem(c) if c.tag == "value" => value = Some(c),
                Node::Elem(c) if c.tag == "comment" => {
                    note = Some(text(s, &c)).filter(|n| !n.trim().is_empty())
                }
                _ => {}
            }
        }
        out.push(Entry {
            name,
            data: e,
            value,
            note,
        });
    }
    Ok(out)
}

fn value(s: &str, e: &Entry) -> String {
    e.value.as_ref().map(|v| text(s, v)).unwrap_or_default()
}

pub fn parse(s: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    for e in entries(s)? {
        if let Some(n) = &e.note {
            p.notes.insert(e.name.clone(), n.clone());
        }
        p.entries.insert(e.name.clone(), value(s, &e));
    }
    Ok(p)
}

fn attr_escape(s: &str) -> String {
    xml::escape(s).replace('"', "&quot;")
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or(SKELETON);
    let Ok(list) = entries(s) else {
        // ponytail: unparseable existing file => regenerate (parse errors surface on read first).
        return write(None, cat);
    };
    let (mut out, mut pos, mut done) = (String::new(), 0, BTreeSet::new());
    for e in &list {
        done.insert(e.name.clone());
        match cat.entries.get(&e.name) {
            Some(v) if *v == value(s, e) => {}
            Some(v) => {
                let (a, b, with) = match &e.value {
                    Some(el) if !s[el.start..el.open_end].ends_with("/>") => {
                        (el.open_end, el.close, xml::escape(v))
                    }
                    Some(el) => (
                        el.start,
                        el.end,
                        format!("<value>{}</value>", xml::escape(v)),
                    ),
                    None => (
                        e.data.open_end,
                        e.data.open_end,
                        format!("<value>{}</value>", xml::escape(v)),
                    ),
                };
                out += &s[pos..a];
                out += &with;
                pos = b;
            }
            None => {
                out += &s[pos..line_start(s, e.data.start)];
                pos = line_end(s, e.data.end);
            }
        }
    }
    let end = root(s).map_or(s.len(), |r| r.1);
    let at = line_start(s, end);
    out += &s[pos..at];
    let indent = list
        .first()
        .map(|e| s[line_start(s, e.data.start)..e.data.start].to_string())
        .filter(|i| i.trim().is_empty() && !i.is_empty())
        .unwrap_or_else(|| "  ".into());
    for (k, v) in cat.entries.iter().filter(|(k, _)| !done.contains(*k)) {
        out += &format!(
            "{indent}<data name=\"{}\" xml:space=\"preserve\">\n{indent}{indent}<value>{}</value>\n{indent}</data>\n",
            attr_escape(k),
            xml::escape(v)
        );
    }
    if at < end && !out.ends_with('\n') {
        out.push('\n');
    }
    out += &s[at..];
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/Resources.fr.resx"
    ));

    #[test]
    fn parse_roundtrip_edit() {
        let p = parse(FIX).unwrap();
        let e = &p.entries;
        assert_eq!(e["Welcome"], "Bienvenue, {0} !");
        assert_eq!(e["Total"], "Total : {0:N2} € & taxes <incl.>");
        assert_eq!(e["Empty"], "");
        assert!(!e.contains_key("Logo"), "binary resources are skipped");
        assert_eq!(p.notes["Welcome"], "{0} = prénom");
        let mut c = Catalog {
            locale: "fr".into(),
            entries: p.entries.clone(),
        };
        assert_eq!(write(Some(FIX), &c), FIX, "byte for byte");
        c.entries.insert("Welcome".into(), "Salut {0} & co".into());
        c.entries.insert("Empty".into(), "Rempli".into());
        c.entries.remove("Total");
        c.entries.insert("New.Key".into(), "Nouveau <b>".into());
        let out = write(Some(FIX), &c);
        assert!(out.contains("<value>Salut {0} &amp; co</value>"), "{out}");
        assert!(out.contains("<value>Rempli</value>"), "{out}");
        assert!(!out.contains("Total"));
        assert!(out.contains("  <data name=\"New.Key\" xml:space=\"preserve\">\n    <value>Nouveau &lt;b&gt;</value>\n  </data>\n</root>"), "{out}");
        assert!(out.contains("<xsd:schema"), "schema kept");
        assert_eq!(parse(&out).unwrap().entries, c.entries);
        let fresh = write(None, &c);
        assert!(fresh.contains("resmimetype"));
        assert_eq!(parse(&fresh).unwrap().entries, c.entries);
    }
}
