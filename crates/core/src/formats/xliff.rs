//! XLIFF 1.2 (`<trans-unit>`) and 2.0 (`<unit><segment>`): Angular `messages.xlf` /
//! `messages.fr.xlf`, Symfony `translations/messages.fr.xlf`, TMS exports.
//!
//! Key = `resname` (Symfony) or `id`. The value is `<target>`; a source-language file (no
//! `target-language`, or the same as `source-language`) reads `<target>` when present (Symfony
//! `messages.en.xlf`) else `<source>` (Angular). `<note>`s are translator notes (locations
//! skipped); target `state` `new`/`needs-*`/`initial` => stale, `final`/`signed-off`/`reviewed` =>
//! reviewed in the lockfile.
//!
//! Inline codes become tokens the validator checks like any placeholder or tag:
//! `<x id="INTERPOLATION" equiv-text="{{ name }}"/>` => `{INTERPOLATION}`, 2.0 `<ph id="0" …/>` =>
//! `{0}`, `<bpt id="1">…</bpt>` => `{bpt_1}`, paired `<g id="1">…</g>` / `<pc id="1">…</pc>` =>
//! `<g1>…</g1>`. Writing maps tokens back to the exact markup of the unit's `<source>`, so
//! attributes (`equiv-text`, `disp`, `ctype`) survive. Angular ICU text (`{VAR_PLURAL, plural, …}`)
//! is plain text in the file and stays ICU.
//!
//! A new target file starts from the source-language sibling (`messages.xlf` or the Symfony
//! `messages.en.xlf`), so every unit keeps its `<source>`; without one glot writes a source file.
use std::collections::BTreeSet;
use std::path::Path;

use super::xml::{self, Elem, Node};
use super::{Parsed, line_end, line_start};
use crate::catalog::Catalog;

const STALE: &[&str] = &[
    "new",
    "initial",
    "needs-translation",
    "needs-adaptation",
    "needs-l10n",
    "needs-review-translation",
    "needs-review-adaptation",
    "needs-review-l10n",
];
const REVIEWED: &[&str] = &["final", "signed-off", "reviewed"];
const PAIRED: &[&str] = &["g", "pc", "mrk"];

struct Unit {
    key: String,
    elem: Elem,
    /// `<trans-unit>` (1.2) or `<segment>` (2.0): holds source/target and, in 2.0, `state`.
    seg: Option<Elem>,
    source: Option<Elem>,
    target: Option<Elem>,
    notes: Vec<String>,
}

struct Doc {
    v2: bool,
    src_lang: Option<String>,
    trg_lang: Option<String>,
    /// Element carrying the language attributes (`<file>` 1.2, `<xliff>` 2.0).
    lang_elem: Elem,
    units: Vec<Unit>,
    /// `<` of the `</body>` (1.2) / `</file>` (2.0) new units go before.
    append_at: usize,
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

fn same_lang(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('_', "-").to_ascii_lowercase();
    let (a, b) = (n(a), n(b));
    a == b || a.starts_with(&format!("{b}-")) || b.starts_with(&format!("{a}-"))
}

fn doc(s: &str) -> Result<Doc, String> {
    let root = elems(s, 0, s.len())?
        .into_iter()
        .find(|e| e.tag == "xliff")
        .ok_or("no <xliff> element")?;
    let v2 = root.attr("version").is_some_and(|v| v.starts_with('2'));
    let files: Vec<Elem> = elems(s, root.open_end, root.close)?
        .into_iter()
        .filter(|e| e.tag == "file")
        .collect();
    let last = files.last().ok_or("no <file> element")?;
    let (lang_elem, src_lang, trg_lang) = if v2 {
        (root.clone(), root.attr("srcLang"), root.attr("trgLang"))
    } else {
        let f = &files[0];
        (
            f.clone(),
            f.attr("source-language"),
            f.attr("target-language"),
        )
    };
    let (src_lang, trg_lang) = (src_lang.map(str::to_string), trg_lang.map(str::to_string));
    let append_at = if v2 {
        last.close
    } else {
        elems(s, last.open_end, last.close)?
            .into_iter()
            .find(|e| e.tag == "body")
            .map_or(last.close, |b| b.close)
    };
    let mut units = vec![];
    for f in &files {
        walk(s, f, v2, &mut units)?;
    }
    Ok(Doc {
        v2,
        src_lang,
        trg_lang,
        lang_elem,
        units,
        append_at,
    })
}

fn walk(s: &str, parent: &Elem, v2: bool, out: &mut Vec<Unit>) -> Result<(), String> {
    for e in elems(s, parent.open_end, parent.close)? {
        match e.tag.as_str() {
            "trans-unit" if !v2 => out.push(unit(s, e, v2)?),
            "unit" if v2 => out.push(unit(s, e, v2)?),
            "body" | "group" => walk(s, &e, v2, out)?,
            _ => {}
        }
    }
    Ok(())
}

fn unit(s: &str, e: Elem, v2: bool) -> Result<Unit, String> {
    let key = e
        .attr("resname")
        .or(e.attr("id"))
        .unwrap_or_default()
        .to_string();
    let mut notes = vec![];
    let mut seg = (!v2).then(|| e.clone());
    let kids = elems(s, e.open_end, e.close)?;
    let note_elems: Vec<Elem> = if v2 {
        match kids.iter().find(|k| k.tag == "notes") {
            Some(n) => elems(s, n.open_end, n.close)?,
            None => vec![],
        }
    } else {
        kids.clone()
    };
    for n in note_elems.iter().filter(|n| n.tag == "note") {
        let kind = n.attr("category").or(n.attr("from")).unwrap_or_default();
        let t = xml::unescape(&s[n.open_end..n.close]).trim().to_string();
        if kind != "location" && !t.is_empty() {
            notes.push(t);
        }
    }
    // ponytail: 2.0 units with several segments: only the first is read and written.
    if v2 {
        seg = kids.into_iter().find(|k| k.tag == "segment");
    }
    let (mut source, mut target) = (None, None);
    if let Some(sg) = &seg {
        for c in elems(s, sg.open_end, sg.close)? {
            match c.tag.as_str() {
                "source" if source.is_none() => source = Some(c),
                "target" if target.is_none() => target = Some(c),
                _ => {}
            }
        }
    }
    Ok(Unit {
        key,
        elem: e,
        seg,
        source,
        target,
        notes,
    })
}

/// Inline content of `e` as text + the (token, markup) pairs it used.
fn to_text(s: &str, e: &Elem, map: &mut Vec<(String, String)>) -> String {
    inline(s, e.open_end, e.close, map)
}

fn inline(s: &str, from: usize, to: usize, map: &mut Vec<(String, String)>) -> String {
    let raw = &s[from..to];
    if let Some(c) = raw
        .trim()
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
    {
        return c.to_string();
    }
    let Ok(kids) = elems(s, from, to) else {
        return xml::unescape(raw);
    };
    let add = |tok: String, markup: &str, map: &mut Vec<(String, String)>| {
        if !map.iter().any(|(t, _)| *t == tok) {
            map.push((tok.clone(), markup.to_string()));
        }
        tok
    };
    let (mut out, mut i) = (String::new(), from);
    for k in kids {
        out += &xml::unescape(&s[i..k.start]);
        i = k.end;
        let id = k.attr("id").or(k.attr("startRef")).unwrap_or_default();
        let name_ok = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
        let paired = PAIRED.contains(&k.tag.as_str()) && k.close > k.open_end;
        if paired && name_ok {
            let t = format!("{}{id}", k.tag);
            out += &add(format!("<{t}>"), &s[k.start..k.open_end], map);
            out += &inline(s, k.open_end, k.close, map);
            out += &add(format!("</{t}>"), &s[k.close..k.end], map);
        } else if name_ok && !paired {
            let tok = if matches!(k.tag.as_str(), "x" | "ph") {
                format!("{{{id}}}")
            } else {
                format!("{{{}_{id}}}", k.tag)
            };
            out += &add(tok, &s[k.start..k.end], map);
        } else {
            let m = &s[k.start..k.end];
            out += &add(m.to_string(), m, map);
        }
    }
    out + &xml::unescape(&s[i..to])
}

/// Text back to XML: tokens from `map` (longest first) become their markup, the rest is escaped.
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

fn is_source(d: &Doc, locale: &str) -> bool {
    match (&d.src_lang, &d.trg_lang) {
        (_, None) => true,
        (Some(s), Some(t)) => same_lang(s, t) || (same_lang(s, locale) && !same_lang(t, locale)),
        (None, Some(t)) => !same_lang(t, locale),
    }
}

/// (value element, current text, token map) of a unit as seen by `locale`'s catalog.
fn current(
    s: &str,
    u: &Unit,
    source: bool,
) -> (Option<Elem>, Option<String>, Vec<(String, String)>) {
    let mut map = vec![];
    let src = u.source.as_ref().map(|e| to_text(s, e, &mut map));
    let tgt = u.target.as_ref().map(|e| to_text(s, e, &mut map));
    match (source, tgt) {
        (_, Some(t)) => (u.target.clone(), Some(t), map),
        (true, None) => (u.source.clone(), src, map),
        (false, None) => (None, None, map),
    }
}

fn state(u: &Unit, v2: bool) -> Option<&str> {
    let e = if v2 {
        u.seg.as_ref()
    } else {
        u.target.as_ref()
    };
    e.and_then(|e| e.attr("state"))
}

pub fn parse(s: &str, locale: &str) -> Result<Parsed, String> {
    let d = doc(s)?;
    let source = is_source(&d, locale);
    let mut p = Parsed::default();
    for u in &d.units {
        if u.key.is_empty() {
            continue;
        }
        if !u.notes.is_empty() {
            p.notes.insert(u.key.clone(), u.notes.join("\n"));
        }
        let (_, val, _) = current(s, u, source);
        if !source && let Some(src) = &u.source {
            p.implicit
                .insert(u.key.clone(), to_text(s, src, &mut vec![]));
        }
        let Some(val) = val else { continue };
        if !source && let Some(st) = state(u, d.v2) {
            if STALE.contains(&st) {
                p.stale.insert(u.key.clone());
            } else if REVIEWED.contains(&st) {
                p.reviewed.insert(u.key.clone());
            }
        }
        p.entries.insert(u.key.clone(), val);
    }
    Ok(p)
}

/// `name="value"` set on the start tag of `e` (edit on `s`).
fn set_attr(s: &str, e: &Elem, name: &str, value: &str) -> (usize, usize, String) {
    let head = &s[e.start..e.open_end];
    let pat = format!(" {name}=");
    if let Some(p) = head.find(&pat) {
        let vs = e.start + p + pat.len();
        let q = &s[vs..vs + 1];
        let ve = vs + 1 + s[vs + 1..].find(q).unwrap_or(0);
        return (vs + 1, ve, value.to_string());
    }
    let at = e.start + 1 + e.tag.len();
    (at, at, format!(" {name}=\"{value}\""))
}

fn apply(s: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = s.to_string();
    for (a, b, t) in edits {
        out.replace_range(a..b, &t);
    }
    out
}

/// The source-language sibling of a target file: `messages.xlf` / `messages.en.xlf` next to
/// `messages.fr.xlf`.
fn template(path: &Path) -> Option<String> {
    let (dir, name) = (path.parent()?, path.file_name()?.to_str()?);
    let ext = path.extension()?.to_str()?;
    let base = name.strip_suffix(ext)?.trim_end_matches('.');
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    let mut cands: Vec<_> = std::fs::read_dir(if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    })
    .ok()?
    .flatten()
    .map(|e| e.path())
    .filter(|p| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            n != name
                && n.ends_with(&format!(".{ext}"))
                && (n == format!("{stem}.{ext}") || n.starts_with(&format!("{stem}.")))
        })
    })
    .collect();
    cands.sort_by_key(|p| p.file_name().map(|n| n.len()));
    cands.into_iter().find_map(|p| {
        let t = std::fs::read_to_string(p).ok()?;
        let d = doc(&t).ok()?;
        (d.trg_lang.is_none()
            || d.src_lang
                .as_deref()
                .zip(d.trg_lang.as_deref())
                .is_some_and(|(a, b)| same_lang(a, b)))
        .then_some(t)
    })
}

fn skeleton(locale: &str, v2: bool) -> String {
    if v2 {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xliff version=\"2.0\" xmlns=\"urn:oasis:names:tc:xliff:document:2.0\" srcLang=\"{locale}\">\n  <file id=\"ngi18n\">\n  </file>\n</xliff>\n"
        )
    } else {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xliff version=\"1.2\" xmlns=\"urn:oasis:names:tc:xliff:document:1.2\">\n  <file source-language=\"{locale}\" datatype=\"plaintext\" original=\"messages\">\n    <body>\n    </body>\n  </file>\n</xliff>\n"
        )
    }
}

pub fn write(existing: Option<&str>, cat: &Catalog, path: &Path) -> String {
    let tpl = template(path);
    let s: String = match (existing, &tpl) {
        (Some(s), _) => s.to_string(),
        (None, Some(t)) => match doc(t) {
            Ok(d) => {
                let name = if d.v2 { "trgLang" } else { "target-language" };
                apply(t, vec![set_attr(t, &d.lang_elem, name, &cat.locale)])
            }
            Err(_) => skeleton(&cat.locale, false),
        },
        (None, None) => skeleton(&cat.locale, false),
    };
    let Ok(d) = doc(&s) else {
        // ponytail: unparseable existing file => left as is (parse errors surface on read first).
        return s;
    };
    let source = is_source(&d, &cat.locale);
    let tpl_doc = tpl.as_deref().and_then(|t| doc(t).ok().map(|d| (t, d)));
    let mut edits = vec![];
    let mut done = BTreeSet::new();
    for u in &d.units {
        done.insert(u.key.clone());
        let (el, cur, map) = current(&s, u, source);
        match cat.entries.get(&u.key) {
            None if el.is_none() => {}
            None => edits.push((
                line_start(&s, u.elem.start),
                line_end(&s, u.elem.end),
                String::new(),
            )),
            Some(v) if Some(v) == cur.as_ref() => {}
            Some(v) => {
                let inner = from_text(v, &map);
                match &el {
                    Some(e) if s[e.start..e.open_end].ends_with("/>") => {
                        let tag = &e.tag;
                        let head = s[e.start..e.open_end - 2].trim_end();
                        edits.push((e.start, e.end, format!("{head}>{inner}</{tag}>")));
                    }
                    Some(e) => edits.push((e.open_end, e.close, inner)),
                    None => {
                        let Some(src) = &u.source else { continue };
                        let ind = &s[line_start(&s, src.start)..src.start];
                        let ind = if ind.trim().is_empty() { ind } else { "" };
                        let st = if d.v2 { "" } else { " state=\"translated\"" };
                        edits.push((
                            src.end,
                            src.end,
                            format!("\n{ind}<target{st}>{inner}</target>"),
                        ));
                    }
                }
                if !source {
                    let st_elem = if d.v2 { u.seg.as_ref() } else { el.as_ref() };
                    if let Some(e) =
                        st_elem.filter(|e| e.attr("state").is_some_and(|x| x != "translated"))
                    {
                        edits.push(set_attr(&s, e, "state", "translated"));
                    }
                }
            }
        }
    }
    // New units before `</body>` / `</file>`.
    let at = line_start(&s, d.append_at);
    let ind = d
        .units
        .first()
        .map(|u| &s[line_start(&s, u.elem.start)..u.elem.start])
        .filter(|i| i.trim().is_empty() && !i.is_empty())
        .map_or_else(
            || {
                if d.v2 {
                    "    ".to_string()
                } else {
                    "      ".to_string()
                }
            },
            str::to_string,
        );
    let step = if ind.starts_with('\t') { "\t" } else { "  " };
    let resname = d.units.iter().any(|u| u.elem.attr("resname").is_some());
    let mut add = String::new();
    for (k, v) in cat.entries.iter().filter(|(k, _)| !done.contains(*k)) {
        // Target file: the source text (and inline markup) comes from the source-language sibling.
        let from_tpl = tpl_doc.as_ref().filter(|_| !source).and_then(|(t, td)| {
            td.units
                .iter()
                .find(|u| u.key == *k)
                .and_then(|u| u.source.as_ref())
                .map(|e| (*t, e.clone()))
        });
        let (src_xml, tgt_xml) = match &from_tpl {
            Some((t, e)) => {
                let mut map = vec![];
                to_text(t, e, &mut map);
                (t[e.open_end..e.close].to_string(), Some(from_text(v, &map)))
            }
            None if source => (from_text(v, &[]), None),
            None => (from_text(v, &[]), Some(from_text(v, &[]))),
        };
        let id = super::xml::escape(k).replace('"', "&quot;");
        let i2 = format!("{ind}{step}");
        if d.v2 {
            let i3 = format!("{i2}{step}");
            let st = if tgt_xml.is_some() {
                " state=\"translated\""
            } else {
                ""
            };
            add += &format!(
                "{ind}<unit id=\"{id}\">\n{i2}<segment{st}>\n{i3}<source>{src_xml}</source>\n"
            );
            if let Some(t) = &tgt_xml {
                add += &format!("{i3}<target>{t}</target>\n");
            }
            add += &format!("{i2}</segment>\n{ind}</unit>\n");
        } else {
            let rn = if resname {
                format!(" resname=\"{id}\"")
            } else {
                String::new()
            };
            add += &format!("{ind}<trans-unit id=\"{id}\"{rn}>\n{i2}<source>{src_xml}</source>\n");
            if let Some(t) = &tgt_xml {
                add += &format!("{i2}<target state=\"translated\">{t}</target>\n");
            }
            add += &format!("{ind}</trans-unit>\n");
        }
    }
    if !add.is_empty() {
        if at < d.append_at {
            edits.push((at, at, add));
        } else {
            edits.push((d.append_at, d.append_at, format!("\n{add}")));
        }
    }
    apply(&s, edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{IssueCode, Syntax, validate};

    const NG12: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/messages.fr.xlf"
    ));
    const NG12_SRC: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/messages.xlf"
    ));
    const NG20: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/messages.de.xlf"
    ));
    const SF: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/validators.fr.xlf"
    ));

    fn cat(locale: &str, p: &Parsed) -> Catalog {
        Catalog {
            locale: locale.into(),
            entries: p.entries.clone(),
        }
    }

    #[test]
    fn roundtrip_byte_for_byte() {
        for (s, loc) in [(NG12, "fr"), (NG12_SRC, "en"), (NG20, "de"), (SF, "fr")] {
            let p = parse(s, loc).unwrap();
            assert!(!p.entries.is_empty());
            assert_eq!(
                write(Some(s), &cat(loc, &p), Path::new("x/none.xlf")),
                s,
                "{loc}"
            );
        }
    }

    #[test]
    fn angular_12_target() {
        let p = parse(NG12, "fr").unwrap();
        let e = &p.entries;
        assert_eq!(e["greeting"], "Bonjour {INTERPOLATION} !");
        assert_eq!(e["bold"], "Cliquez {START_BOLD_TEXT}ici{CLOSE_BOLD_TEXT}");
        assert_eq!(
            e["4374883465412154"],
            "{VAR_PLURAL, plural, =0 {aucun} =1 {un élément} other {{INTERPOLATION} éléments}}"
        );
        assert_eq!(e["amp"], "Prix & <taxes>");
        assert!(!e.contains_key("untranslated"));
        assert_eq!(p.implicit["untranslated"], "Not yet");
        assert_eq!(p.notes["greeting"], "Accueil de l'utilisateur\nheader");
        assert!(p.stale.contains("bold"));
        assert!(p.reviewed.contains("greeting"));
        // Placeholders are validated against the source.
        let src = parse(NG12_SRC, "en").unwrap().entries;
        assert_eq!(src["greeting"], "Hello {INTERPOLATION}!");
        assert!(
            validate(
                &src["greeting"],
                &e["greeting"],
                "fr",
                Syntax::Icu,
                &Default::default()
            )
            .is_empty()
        );
        let bad = validate(
            &src["greeting"],
            "Bonjour !",
            "fr",
            Syntax::Icu,
            &Default::default(),
        );
        assert!(
            bad.iter().any(|i| i.code == IssueCode::PlaceholderMissing),
            "{bad:?}"
        );

        let mut c = cat("fr", &p);
        c.entries.insert(
            "greeting".into(),
            "Salut {INTERPOLATION} & bienvenue".into(),
        );
        c.entries.insert(
            "bold".into(),
            "{START_BOLD_TEXT}Ici{CLOSE_BOLD_TEXT} !".into(),
        );
        c.entries.insert("untranslated".into(), "Pas encore".into());
        c.entries.remove("amp");
        let out = write(Some(NG12), &c, Path::new("x/none.xlf"));
        assert!(out.contains("<target state=\"translated\">Salut <x id=\"INTERPOLATION\" equiv-text=\"{{ name }}\"/> &amp; bienvenue</target>"), "{out}");
        assert!(out.contains("<target state=\"translated\"><x id=\"START_BOLD_TEXT\" ctype=\"x-b\" equiv-text=\"&lt;b&gt;\"/>Ici"), "{out}");
        assert!(
            out.contains(
                "<source>Not yet</source>\n        <target state=\"translated\">Pas encore</target>"
            ),
            "{out}"
        );
        assert!(!out.contains("Prix"));
        let back = parse(&out, "fr").unwrap();
        assert_eq!(back.entries, c.entries);
        assert!(
            !back.stale.contains("bold"),
            "edited target is no longer stale"
        );
    }

    #[test]
    fn angular_20_and_symfony() {
        let p = parse(NG20, "de").unwrap();
        assert_eq!(p.entries["welcome"], "Willkommen {0}, <pc0>jetzt</pc0>");
        assert_eq!(p.notes["welcome"], "Begrüßung");
        assert!(p.stale.contains("welcome"));
        let mut c = cat("de", &p);
        c.entries
            .insert("welcome".into(), "Hallo {0}, <pc0>los</pc0>".into());
        c.entries.insert("added".into(), "Neu".into());
        let out = write(Some(NG20), &c, Path::new("x/none.xlf"));
        assert!(out.contains("<segment state=\"translated\">"), "{out}");
        assert!(out.contains("<target>Hallo <ph id=\"0\" equiv=\"INTERPOLATION\" disp=\"{{ name }}\"/>, <pc id=\"0\" equivStart=\"START_BOLD_TEXT\" equivEnd=\"CLOSE_BOLD_TEXT\" type=\"fmt\">los</pc></target>"), "{out}");
        assert!(out.contains("<unit id=\"added\">"), "{out}");
        assert_eq!(parse(&out, "de").unwrap().entries, c.entries);

        // Symfony: resname is the key; the same-language file reads <target>.
        let p = parse(SF, "fr").unwrap();
        assert_eq!(
            p.entries["form.name.blank"],
            "Le nom ne peut pas être vide."
        );
        let mut c = cat("fr", &p);
        c.entries.insert("form.new".into(), "Nouveau".into());
        let out = write(Some(SF), &c, Path::new("x/none.xlf"));
        assert!(
            out.contains("<trans-unit id=\"form.new\" resname=\"form.new\">"),
            "{out}"
        );
        assert_eq!(parse(&out, "fr").unwrap().entries, c.entries);
    }

    #[test]
    fn new_target_from_source_sibling() {
        let dir = std::env::temp_dir().join(format!("glot-xliff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("messages.xlf"), NG12_SRC).unwrap();
        let mut c = Catalog::new("es");
        c.entries
            .insert("greeting".into(), "Hola {INTERPOLATION}!".into());
        let out = write(None, &c, &dir.join("messages.es.xlf"));
        assert!(out.contains("target-language=\"es\""), "{out}");
        assert!(
            out.contains(
                "<source>Hello <x id=\"INTERPOLATION\" equiv-text=\"{{ name }}\"/>!</source>"
            ),
            "{out}"
        );
        assert!(out.contains("<target state=\"translated\">Hola <x id=\"INTERPOLATION\" equiv-text=\"{{ name }}\"/>!</target>"), "{out}");
        assert_eq!(parse(&out, "es").unwrap().entries, c.entries);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
