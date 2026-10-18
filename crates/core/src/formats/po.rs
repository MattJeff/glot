//! gettext PO/POT. Key = `msgid` (`msgctxt` + U+0004 + `msgid` with a context, gettext's own
//! convention). `msgid_plural`/`msgstr[n]` <=> ICU plural on `count`, indexes mapped to CLDR
//! categories through `Plural-Forms: nplurals`. `#.` => note, `#:` => references, `fuzzy` => stale.
use super::{Parsed, folded_choice, icu_choice};
use crate::catalog::Catalog;
use crate::plural::{PluralCategory, categories, language};

#[derive(Default)]
struct Entry {
    ctx: Option<String>,
    id: String,
    id_plural: Option<String>,
    strs: Vec<String>,
    extracted: Vec<String>,
    refs: Vec<String>,
    fuzzy: bool,
    obsolete: bool,
    /// Byte span of the whole block, of its `#,` line and of its msgstr lines.
    span: (usize, usize),
    flags: Option<(usize, usize)>,
    msgstr: (usize, usize),
}

impl Entry {
    fn key(&self) -> String {
        match &self.ctx {
            Some(c) => format!("{c}\u{4}{}", self.id),
            None => self.id.clone(),
        }
    }
}

fn entries(s: &str) -> Result<Vec<Entry>, String> {
    let mut out = vec![];
    let mut cur: Option<Entry> = None;
    // Field receiving continuation lines: 0 ctx, 1 id, 2 id_plural, 3+n msgstr[n].
    let mut field = 0usize;
    let mut pos = 0;
    for line in s.split_inclusive('\n') {
        let (start, end) = (pos, pos + line.len());
        pos = end;
        let t = line.trim();
        if t.is_empty() {
            out.extend(cur.take());
            continue;
        }
        let e = cur.get_or_insert_with(|| Entry {
            span: (start, end),
            msgstr: (end, end),
            ..Default::default()
        });
        if !t.starts_with("msgstr") && !t.starts_with('"') && e.msgstr.0 < e.msgstr.1 {
            // A new entry without a blank line in between.
            out.extend(cur.take());
            cur = Some(Entry {
                span: (start, end),
                msgstr: (end, end),
                ..Default::default()
            });
        }
        let e = cur.as_mut().unwrap();
        e.span.1 = end;
        if t.starts_with("#~") {
            e.obsolete = true;
        } else if let Some(c) = t.strip_prefix("#.") {
            e.extracted.push(c.trim().to_string());
        } else if let Some(c) = t.strip_prefix("#:") {
            e.refs.extend(c.split_whitespace().map(str::to_string));
        } else if let Some(c) = t.strip_prefix("#,") {
            e.fuzzy = c.split(',').any(|f| f.trim() == "fuzzy");
            e.flags = Some((start, end));
        } else if t.starts_with('#') {
            // Translator comment / previous msgid: kept verbatim by the writer.
        } else if let Some(r) = t.strip_prefix("msgctxt") {
            e.ctx = Some(quoted(r)?);
            field = 0;
        } else if let Some(r) = t.strip_prefix("msgid_plural") {
            e.id_plural = Some(quoted(r)?);
            field = 2;
        } else if let Some(r) = t.strip_prefix("msgid") {
            e.id = quoted(r)?;
            field = 1;
        } else if let Some(r) = t.strip_prefix("msgstr") {
            let (n, r) = match r.strip_prefix('[') {
                Some(r) => {
                    let (n, r) = r.split_once(']').ok_or("bad msgstr[n]")?;
                    (n.trim().parse::<usize>().map_err(|e| e.to_string())?, r)
                }
                None => (0, r),
            };
            if e.strs.len() <= n {
                e.strs.resize(n + 1, String::new());
            }
            e.strs[n] = quoted(r)?;
            field = 3 + n;
            if e.msgstr.0 == e.msgstr.1 {
                e.msgstr.0 = start;
            }
            e.msgstr.1 = end;
        } else if t.starts_with('"') {
            let more = quoted(t)?;
            match field {
                0 => e.ctx.get_or_insert_default().push_str(&more),
                1 => e.id.push_str(&more),
                2 => e.id_plural.get_or_insert_default().push_str(&more),
                n => e.strs[n - 3].push_str(&more),
            }
            if field >= 3 {
                e.msgstr.1 = end;
            }
        } else {
            return Err(format!("unexpected line: {t}"));
        }
    }
    out.extend(cur);
    Ok(out.into_iter().filter(|e| !e.obsolete).collect())
}

fn quoted(r: &str) -> Result<String, String> {
    let r = r.trim();
    let inner = r
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .ok_or_else(|| format!("expected a quoted string, got `{r}`"))?;
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    Ok(out)
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

fn nplurals(list: &[Entry]) -> Option<usize> {
    let header = list.iter().find(|e| e.id.is_empty() && e.ctx.is_none())?;
    let h = header.strs.first()?;
    let at = h.find("nplurals=")? + "nplurals=".len();
    h[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// CLDR category of each `msgstr[n]`. gettext omits CLDR's fraction-only category: `many` for
/// Romance/Czech-like languages, `other` for East Slavic/Polish.
fn po_categories(locale: &str, n: Option<usize>) -> Vec<&'static str> {
    let mut cats = categories(locale);
    let n = n.unwrap_or(cats.len()).max(1);
    if cats.len() > n {
        let drop = match language(locale).as_str() {
            "ru" | "uk" | "be" | "pl" => PluralCategory::Other,
            _ => PluralCategory::Many,
        };
        cats.retain(|c| *c != drop);
    }
    cats.truncate(n);
    cats.iter().map(PluralCategory::as_str).collect()
}

fn value(e: &Entry, cats: &[&str]) -> String {
    if e.id_plural.is_none() {
        return e.strs.first().cloned().unwrap_or_default();
    }
    if e.strs.iter().all(String::is_empty) {
        return String::new();
    }
    let forms: Vec<(String, String)> = e
        .strs
        .iter()
        .enumerate()
        .map(|(i, s)| (cats.get(i).unwrap_or(&"other").to_string(), s.clone()))
        .collect();
    icu_choice("count", "plural", &forms)
}

pub fn parse(s: &str, locale: &str) -> Result<Parsed, String> {
    let list = entries(s)?;
    let cats = po_categories(locale, nplurals(&list));
    let mut p = Parsed::default();
    for e in list
        .iter()
        .filter(|e| !(e.id.is_empty() && e.ctx.is_none()))
    {
        let key = e.key();
        p.entries.insert(key.clone(), value(e, &cats));
        // Lingui: `#. js-lingui-explicit-id` = the msgid is a key, not source text.
        let lingui = |c: &&String| c.starts_with("js-lingui-");
        if !e.extracted.iter().any(|c| c == "js-lingui-explicit-id") {
            p.implicit.insert(
                key.clone(),
                match &e.id_plural {
                    Some(pl) => icu_choice(
                        "count",
                        "plural",
                        &[("one".into(), e.id.clone()), ("other".into(), pl.clone())],
                    ),
                    None => e.id.clone(),
                },
            );
        }
        let mut note = e
            .extracted
            .iter()
            .filter(|c| !lingui(c))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if !e.refs.is_empty() {
            if !note.is_empty() {
                note.push('\n');
            }
            note += &format!("References: {}", e.refs.join(" "));
        }
        if !note.is_empty() {
            p.notes.insert(key.clone(), note);
        }
        if e.fuzzy {
            p.stale.insert(key);
        }
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let header = header(&cat.locale);
    let s = existing.unwrap_or(&header);
    let Ok(list) = entries(s) else {
        return write(None, cat);
    };
    let cats = po_categories(&cat.locale, nplurals(&list));
    let (mut out, mut pos) = (String::new(), 0);
    let mut seen = std::collections::BTreeSet::new();
    for e in list
        .iter()
        .filter(|e| !(e.id.is_empty() && e.ctx.is_none()))
    {
        let key = e.key();
        seen.insert(key.clone());
        match cat.entries.get(&key) {
            None => {
                // Drop the block and the blank line after it.
                out.push_str(&s[pos..e.span.0]);
                pos = e.span.1;
                if s[pos..].starts_with('\n') {
                    pos += 1;
                }
            }
            Some(v) if *v == value(e, &cats) => {}
            Some(v) => {
                if let Some((a, b)) = e.flags {
                    out.push_str(&s[pos..a]);
                    out += &unfuzzy(&s[a..b]);
                    pos = b;
                }
                out.push_str(&s[pos..e.msgstr.0]);
                out += &msgstr(v, e.id_plural.is_some(), &cats);
                pos = e.msgstr.1;
            }
        }
    }
    out.push_str(&s[pos..]);
    for (k, v) in &cat.entries {
        if seen.contains(k) {
            continue;
        }
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        let (ctx, id) = match k.split_once('\u{4}') {
            Some((c, i)) => (Some(c), i),
            None => (None, k.as_str()),
        };
        if let Some(c) = ctx {
            out += &format!("msgctxt {}\n", lines(c));
        }
        out += &format!("msgid {}\n", lines(id));
        let plural = folded_choice(v).is_some_and(|c| c.kind == "plural");
        if plural {
            // ponytail: no source text here; the plural msgid repeats the singular.
            out += &format!("msgid_plural {}\n", lines(id));
        }
        out += &msgstr(v, plural, &cats);
    }
    out
}

/// `#, fuzzy, c-format` => `#, c-format`; a lone fuzzy flag line disappears.
fn unfuzzy(line: &str) -> String {
    let flags: Vec<&str> = line.trim()[2..]
        .split(',')
        .map(str::trim)
        .filter(|f| *f != "fuzzy" && !f.is_empty())
        .collect();
    if flags.is_empty() {
        String::new()
    } else {
        format!("#, {}\n", flags.join(", "))
    }
}

fn msgstr(v: &str, plural: bool, cats: &[&str]) -> String {
    if !plural {
        return format!("msgstr {}\n", lines(v));
    }
    let choice = folded_choice(v);
    cats.iter()
        .enumerate()
        .map(|(i, c)| {
            let text = choice
                .as_ref()
                .and_then(|ch| ch.form(c).or_else(|| ch.form("other")))
                .unwrap_or(v);
            format!("msgstr[{i}] {}\n", lines(text))
        })
        .collect()
}

/// A quoted PO string; multi-line values use gettext's `""` + one line per `\n` layout.
fn lines(v: &str) -> String {
    let body = v.strip_suffix('\n').unwrap_or(v);
    if !body.contains('\n') {
        return format!("\"{}\"", escape(v));
    }
    let mut out = "\"\"".to_string();
    for l in v.split_inclusive('\n') {
        out += &format!("\n\"{}\"", escape(l));
    }
    out
}

fn header(locale: &str) -> String {
    // ponytail: formulas for the common languages; others get no Plural-Forms (msginit adds it).
    let forms = match language(locale).as_str() {
        "ja" | "zh" | "ko" | "vi" | "th" | "id" | "ms" => Some("nplurals=1; plural=0;"),
        "fr" => Some("nplurals=2; plural=(n > 1);"),
        "pt" if locale.contains("BR") => Some("nplurals=2; plural=(n > 1);"),
        "ru" | "uk" | "be" => Some(
            "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        ),
        "pl" => Some(
            "nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        ),
        "cs" | "sk" => Some("nplurals=3; plural=(n==1) ? 0 : (n>=2 && n<=4) ? 1 : 2;"),
        "ar" => Some(
            "nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);",
        ),
        "en" | "de" | "nl" | "sv" | "da" | "nb" | "no" | "fi" | "it" | "es" | "pt" | "el"
        | "hu" | "tr" | "ca" | "et" | "bg" => Some("nplurals=2; plural=(n != 1);"),
        _ => None,
    };
    let mut h = format!(
        "msgid \"\"\nmsgstr \"\"\n\"Language: {locale}\\n\"\n\"MIME-Version: 1.0\\n\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Content-Transfer-Encoding: 8bit\\n\"\n"
    );
    if let Some(f) = forms {
        h += &format!("\"Plural-Forms: {f}\\n\"\n");
    }
    h
}
