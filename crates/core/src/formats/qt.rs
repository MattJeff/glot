//! Qt Linguist `.ts` (`translations/app_fr.ts`, written by `lupdate`, compiled by `lrelease`).
//! Each file holds the source text and one locale's translation, so the source catalog is implied
//! (like gettext without a source `.po`). Key = `context` U+0004 `source` (+ U+0004 `comment`, Qt's
//! disambiguation), or the `id` of id-based files. `numerus="yes"` <=> ICU plural on `count`
//! (`<numerusform>`s in the language's order, CLDR's fraction-only category left out), `%1` / `%n`
//! stay verbatim. `type="unfinished"` with a value => stale; `vanished` / `obsolete` are skipped.
//! `extracomment` / `comment` / `location` => note. Writing patches `<translation>` in place.
use std::collections::BTreeSet;

use super::xml::{self, Elem, Node};
use super::{Parsed, folded_choice, icu_choice, line_end, line_start, po};
use crate::catalog::Catalog;

struct Msg {
    key: String,
    source: String,
    numerus: bool,
    /// The `<message>` element and its `<translation>` (absent in some hand-made files).
    elem: Elem,
    tr: Option<Elem>,
    forms: Vec<String>,
    kind: Option<String>,
    note: String,
}

/// Each `<context>`: its name, the `<context>` element and its messages.
fn contexts(s: &str) -> Result<Vec<(String, Elem, Vec<Msg>)>, String> {
    let ts = s.find("<TS").ok_or("no <TS> element")?;
    let from = ts + s[ts..].find('>').ok_or("unclosed <TS>")? + 1;
    let to = s.rfind("</TS>").unwrap_or(s.len());
    let mut out = vec![];
    for c in elems(s, from, to)?
        .into_iter()
        .filter(|e| e.tag == "context")
    {
        let kids = elems(s, c.open_end, c.close)?;
        let name = kids
            .iter()
            .find(|e| e.tag == "name")
            .map(|e| text(s, e))
            .unwrap_or_default();
        let mut msgs = vec![];
        for m in kids.into_iter().filter(|e| e.tag == "message") {
            let parts = elems(s, m.open_end, m.close)?;
            let get = |t: &str| parts.iter().find(|e| e.tag == t);
            let source = get("source").map(|e| text(s, e)).unwrap_or_default();
            let comment = get("comment").map(|e| text(s, e)).unwrap_or_default();
            let tr = get("translation").cloned();
            let numerus = m.attr("numerus") == Some("yes");
            let forms = match &tr {
                Some(t) if numerus => elems(s, t.open_end, t.close)?
                    .iter()
                    .filter(|e| e.tag == "numerusform")
                    .map(|e| text(s, e))
                    .collect(),
                Some(t) => vec![text(s, t)],
                None => vec![],
            };
            let key = match m.attr("id") {
                Some(id) => id.to_string(),
                None if comment.is_empty() => format!("{name}\u{4}{source}"),
                None => format!("{name}\u{4}{source}\u{4}{comment}"),
            };
            let mut note: Vec<String> = ["extracomment", "comment"]
                .iter()
                .filter_map(|t| get(t).map(|e| text(s, e)))
                .filter(|t| !t.is_empty())
                .collect();
            let refs: Vec<String> = parts
                .iter()
                .filter(|e| e.tag == "location")
                .filter_map(|e| {
                    let f = e.attr("filename")?;
                    Some(e.attr("line").map_or(f.to_string(), |l| format!("{f}:{l}")))
                })
                .collect();
            if !refs.is_empty() {
                note.push(format!("References: {}", refs.join(" ")));
            }
            msgs.push(Msg {
                key,
                source,
                numerus,
                kind: tr.as_ref().and_then(|t| t.attr("type")).map(str::to_string),
                elem: m,
                tr,
                forms,
                note: note.join("\n"),
            });
        }
        out.push((name, c, msgs));
    }
    Ok(out)
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

fn text(s: &str, e: &Elem) -> String {
    xml::unescape(&s[e.open_end..e.close])
}

fn skipped(m: &Msg) -> bool {
    matches!(m.kind.as_deref(), Some("vanished" | "obsolete"))
}

/// `TS language="fr_FR"` (target) or the file's locale.
fn categories(s: &str, locale: &str) -> Vec<&'static str> {
    let lang = s
        .split_once(" language=\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map_or(locale.to_string(), |(l, _)| l.replace('_', "-"));
    po::default_categories(&lang)
}

fn value(m: &Msg, cats: &[&str]) -> String {
    if !m.numerus {
        return m.forms.first().cloned().unwrap_or_default();
    }
    if m.forms.iter().all(String::is_empty) {
        return String::new();
    }
    let forms: Vec<(String, String)> = m
        .forms
        .iter()
        .enumerate()
        .map(|(i, f)| (cats.get(i).unwrap_or(&"other").to_string(), f.clone()))
        .collect();
    icu_choice("count", "plural", &forms)
}

pub fn parse(s: &str, locale: &str) -> Result<Parsed, String> {
    let cats = categories(s, locale);
    let mut p = Parsed::default();
    for m in contexts(s)?.into_iter().flat_map(|c| c.2) {
        if skipped(&m) {
            continue;
        }
        let v = value(&m, &cats);
        if m.kind.as_deref() == Some("unfinished") && !v.is_empty() {
            p.stale.insert(m.key.clone());
        }
        p.implicit.insert(
            m.key.clone(),
            if m.numerus {
                icu_choice(
                    "count",
                    "plural",
                    &[
                        ("one".into(), m.source.clone()),
                        ("other".into(), m.source.clone()),
                    ],
                )
            } else {
                m.source.clone()
            },
        );
        if !m.note.is_empty() {
            p.notes.insert(m.key.clone(), m.note);
        }
        p.entries.insert(m.key, v);
    }
    Ok(p)
}

/// `<translation>` for `v`: numerus forms on their own lines under `ind`.
fn translation(v: &str, numerus: bool, cats: &[&str], ind: &str) -> String {
    if !numerus {
        return format!("<translation>{}</translation>", xml::escape(v));
    }
    let choice = folded_choice(v);
    let mut out = "<translation>".to_string();
    for c in cats {
        let f = choice
            .as_ref()
            .and_then(|ch| ch.form(c).or_else(|| ch.form("other")))
            .unwrap_or(v);
        out += &format!("\n{ind}    <numerusform>{}</numerusform>", xml::escape(f));
    }
    out + &format!("\n{ind}</translation>")
}

fn skeleton(locale: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE TS>\n<TS version=\"2.1\" language=\"{}\">\n</TS>\n",
        locale.replace('-', "_")
    )
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let fresh = skeleton(&cat.locale);
    let s = existing.unwrap_or(&fresh);
    let Ok(ctxs) = contexts(s) else {
        return write(None, cat);
    };
    let cats = categories(s, &cat.locale);
    let (mut out, mut pos) = (String::new(), 0);
    let mut seen = BTreeSet::new();
    let new_msgs = |ctx: &str, seen: &BTreeSet<String>, ind: &str| -> String {
        let mut add = String::new();
        for (k, v) in &cat.entries {
            let mut parts = k.splitn(3, '\u{4}');
            let (Some(c), Some(src)) = (parts.next(), parts.next()) else {
                continue;
            };
            if c != ctx || seen.contains(k) {
                continue;
            }
            let numerus = folded_choice(v).is_some_and(|ch| ch.kind == "plural");
            add += &format!(
                "{ind}<message{}>\n{ind}    <source>{}</source>\n",
                if numerus { " numerus=\"yes\"" } else { "" },
                xml::escape(src)
            );
            if let Some(cm) = parts.next() {
                add += &format!("{ind}    <comment>{}</comment>\n", xml::escape(cm));
            }
            add += &format!(
                "{ind}    {}\n{ind}</message>\n",
                translation(v, numerus, &cats, &format!("{ind}    "))
            );
        }
        add
    };
    for (name, c, msgs) in &ctxs {
        let mut ind = "    ".to_string();
        for m in msgs {
            seen.insert(m.key.clone());
            ind = s[line_start(s, m.elem.start)..m.elem.start].to_string();
            if skipped(m) {
                continue;
            }
            match (cat.entries.get(&m.key), &m.tr) {
                (None, _) => {
                    out.push_str(&s[pos..line_start(s, m.elem.start)]);
                    pos = line_end(s, m.elem.end);
                }
                (Some(v), Some(_)) if *v == value(m, &cats) => {}
                (Some(v), tr) => {
                    let tr_ind = tr
                        .as_ref()
                        .map(|t| s[line_start(s, t.start)..t.start].to_string())
                        .unwrap_or_else(|| format!("{ind}    "));
                    let new = translation(v, m.numerus, &cats, &tr_ind);
                    match tr {
                        Some(t) => {
                            out.push_str(&s[pos..t.start]);
                            out += &new;
                            pos = t.end;
                        }
                        None => {
                            out.push_str(&s[pos..m.elem.close]);
                            out += &format!("{tr_ind}{new}\n{ind}");
                            pos = m.elem.close;
                        }
                    }
                }
            }
        }
        let add = new_msgs(name, &seen, &ind);
        if !add.is_empty() {
            let at = line_start(s, c.close);
            out.push_str(&s[pos..at]);
            out += &add;
            pos = at;
        }
        seen.extend(
            cat.entries
                .keys()
                .filter(|k| k.split('\u{4}').next() == Some(name))
                .cloned(),
        );
    }
    // Contexts the file does not have yet go before `</TS>`.
    let end = s.rfind("</TS>").unwrap_or(s.len());
    let at = line_start(s, end);
    out.push_str(&s[pos..at]);
    let mut done: BTreeSet<&str> = ctxs.iter().map(|c| c.0.as_str()).collect();
    for k in cat.entries.keys() {
        let Some((ctx, _)) = k.split_once('\u{4}') else {
            continue;
        };
        if done.insert(ctx) {
            out += &format!(
                "<context>\n    <name>{}</name>\n{}</context>\n",
                xml::escape(ctx),
                new_msgs(ctx, &seen, "    ")
            );
        }
    }
    out.push_str(&s[at..]);
    out
}
