//! CSV string tables.
//! - Every locale in one file, one column each: Unity Localization String Table export
//!   (`Key,Id,Shared Comments,English(en),English(en) Comments,French(fr),…`) and slang compact CSV
//!   (`key,en,de-DE`). A locale without a column gets one (`French(fr)` in Unity tables).
//! - One file per locale, `key,value` rows without a header (slang `strings_de.i18n.csv`).
//!
//! An empty cell is an untranslated message. `x.one`/`x.other` rows are one plural message (slang).
//! Writing patches cells: untouched rows, quoting and line endings stay byte for byte; a new key is
//! a new row (Unity `Id` `0` = "assign a new id" on import).
use std::collections::BTreeMap;

use super::Parsed;
use super::json::{fold_plurals, unfold_plurals};
use crate::catalog::Catalog;

#[derive(Debug, Clone)]
struct Cell {
    raw: String,
    value: String,
}

#[derive(Debug, Clone)]
struct Row {
    cells: Vec<Cell>,
    /// `\n`, `\r\n` or `` (last line).
    end: String,
}

fn cell(v: &str) -> Cell {
    Cell {
        raw: v.to_string(),
        value: v.to_string(),
    }
}

fn rows(s: &str) -> Vec<Row> {
    let mut out = vec![];
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let mut cells = vec![];
        loop {
            let start = i;
            let mut value = String::new();
            if b.get(i) == Some(&b'"') {
                i += 1;
                let mut from = i;
                while i < b.len() {
                    if b[i] == b'"' {
                        value.push_str(&s[from..i]);
                        if b.get(i + 1) == Some(&b'"') {
                            value.push('"');
                            i += 2;
                            from = i;
                            continue;
                        }
                        i += 1;
                        from = i;
                        break;
                    }
                    i += 1;
                }
                if from < i {
                    value.push_str(&s[from..i]);
                }
            }
            // Unquoted text (or stray text after a closing quote) up to the separator.
            let rest = i;
            while i < b.len() && !matches!(b[i], b',' | b'\n') {
                i += 1;
            }
            let mut stop = i;
            if b.get(i) == Some(&b'\n') && stop > rest && b[stop - 1] == b'\r' {
                stop -= 1;
            }
            value.push_str(&s[rest..stop]);
            cells.push(Cell {
                raw: s[start..stop].to_string(),
                value,
            });
            if b.get(i) == Some(&b',') {
                i += 1;
                continue;
            }
            let end = &s[stop..(i + 1).min(b.len())];
            i = (i + 1).min(b.len());
            out.push(Row {
                cells,
                end: end.to_string(),
            });
            break;
        }
    }
    out
}

fn quote(v: &str, always: bool) -> String {
    if always || v.contains([',', '"', '\n', '\r']) || v.trim() != v {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn render(rows: &[Row]) -> String {
    rows.iter()
        .map(|r| {
            let cells: Vec<&str> = r.cells.iter().map(|c| c.raw.as_str()).collect();
            cells.join(",") + &r.end
        })
        .collect()
}

fn same_locale(a: &str, b: &str) -> bool {
    a.replace('_', "-")
        .eq_ignore_ascii_case(&b.replace('_', "-"))
}

/// A BCP 47-ish tag: `en`, `de-DE`, `zh-Hans`, `pt_BR` (first subtag lowercase).
fn is_tag(s: &str) -> bool {
    let mut parts = s.split(['-', '_']);
    let lang = parts.next().unwrap_or_default();
    (2..=3).contains(&lang.len())
        && lang.chars().all(|c| c.is_ascii_lowercase())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The locale a header names: `fr`, `de-DE` (slang), `French(fr)` (Unity).
fn header_locale(h: &str) -> Option<&str> {
    let h = h.trim();
    if let Some((_, code)) = h.strip_suffix(')').and_then(|x| x.rsplit_once('(')) {
        return is_tag(code).then_some(code);
    }
    is_tag(h).then_some(h)
}

/// Column layout of a table with a header (`None`: `key,value` rows).
struct Header {
    key: usize,
    id: Option<usize>,
    note: Option<usize>,
    locales: Vec<(usize, String)>,
    unity: bool,
}

fn header(rows: &[Row]) -> Option<Header> {
    let first = rows.first()?;
    let names: Vec<&str> = first.cells.iter().map(|c| c.value.trim()).collect();
    let key = names.iter().position(|n| n.eq_ignore_ascii_case("key"))?;
    let locales = names
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != key)
        .filter_map(|(i, n)| Some((i, header_locale(n)?.to_string())))
        .collect();
    Some(Header {
        key,
        id: names.iter().position(|n| *n == "Id"),
        note: names.iter().position(|n| {
            ["shared comments", "comment", "comments", "description"]
                .contains(&n.to_ascii_lowercase().as_str())
        }),
        unity: names.iter().any(|n| n.ends_with(')')),
        locales,
    })
}

pub fn parse(text: &str, locale: &str) -> Result<Parsed, String> {
    let rows = rows(text);
    let mut p = Parsed::default();
    match header(&rows) {
        Some(h) => {
            let Some(col) = h
                .locales
                .iter()
                .find(|(_, l)| same_locale(l, locale))
                .map(|(c, _)| *c)
            else {
                return Ok(p);
            };
            for r in &rows[1..] {
                let get = |i: usize| r.cells.get(i).map(|c| c.value.as_str()).unwrap_or("");
                let k = get(h.key);
                if k.is_empty() {
                    continue;
                }
                if !get(col).is_empty() {
                    p.entries.insert(k.into(), get(col).into());
                }
                if let Some(n) = h.note.map(get).filter(|n| !n.is_empty()) {
                    p.notes.insert(k.into(), n.into());
                }
            }
        }
        None => {
            for r in &rows {
                if let [k, v, ..] = r.cells.as_slice()
                    && !k.value.is_empty()
                {
                    p.entries.insert(k.value.clone(), v.value.clone());
                }
            }
        }
    }
    fold_plurals(&mut p.entries, ".", locale);
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let want: BTreeMap<String, String> =
        unfold_plurals(cat.entries.clone().into_iter().collect(), ".")
            .into_iter()
            .collect();
    let mut rows = rows(existing.unwrap_or_default());
    let nl = rows
        .iter()
        .map(|r| r.end.as_str())
        .find(|e| !e.is_empty())
        .unwrap_or("\n")
        .to_string();
    let mut seen = std::collections::BTreeSet::new();
    let (key, col, width, id) = match header(&rows) {
        Some(h) => {
            let width = rows[0].cells.len();
            let col = match h.locales.iter().find(|(_, l)| same_locale(l, &cat.locale)) {
                Some((c, _)) => *c,
                None => {
                    let name = if h.unity {
                        let lang = cat.locale.split(['-', '_']).next().unwrap_or_default();
                        let name = crate::markets::MARKETS
                            .iter()
                            .find(|m| m.locale == lang)
                            .map_or(cat.locale.as_str(), |m| m.name);
                        format!("{name}({})", cat.locale)
                    } else {
                        cat.locale.clone()
                    };
                    for (i, r) in rows.iter_mut().enumerate() {
                        r.cells.resize_with(width, || cell(""));
                        r.cells.push(cell(&if i == 0 {
                            quote(&name, false)
                        } else {
                            String::new()
                        }));
                        if i == 0 {
                            r.cells.last_mut().unwrap().value = name.clone();
                        }
                    }
                    width
                }
            };
            let width = rows[0].cells.len();
            let others: Vec<usize> = h
                .locales
                .iter()
                .map(|(c, _)| *c)
                .filter(|c| *c != col)
                .collect();
            let mut drop = vec![];
            for (n, r) in rows.iter_mut().enumerate().skip(1) {
                let Some(k) = r
                    .cells
                    .get(h.key)
                    .map(|c| c.value.clone())
                    .filter(|k| !k.is_empty())
                else {
                    continue;
                };
                seen.insert(k.clone());
                let cur = r.cells.get(col).map_or("", |c| c.value.as_str());
                let new = want.get(&k).map_or("", String::as_str);
                if cur == new {
                    continue;
                }
                if new.is_empty()
                    && others
                        .iter()
                        .all(|&c| r.cells.get(c).is_none_or(|c| c.value.is_empty()))
                {
                    drop.push(n);
                    continue;
                }
                if r.cells.len() <= col {
                    r.cells.resize_with(col + 1, || cell(""));
                }
                let quoted = r.cells[col].raw.starts_with('"');
                r.cells[col] = Cell {
                    raw: quote(new, quoted),
                    value: new.into(),
                };
            }
            for n in drop.into_iter().rev() {
                let end = rows.remove(n).end;
                // The removed last line had no newline: neither has the new last line.
                if end.is_empty()
                    && let Some(r) = rows.last_mut()
                {
                    r.end.clear();
                }
            }
            (h.key, col, width, h.id)
        }
        None => {
            let mut keep = vec![];
            for r in rows.drain(..) {
                let k = r.cells.first().map(|c| c.value.clone()).unwrap_or_default();
                if k.is_empty() {
                    keep.push(r);
                    continue;
                }
                let Some(new) = want.get(&k) else { continue };
                seen.insert(k);
                let mut r = r;
                if r.cells.len() < 2 {
                    r.cells.push(cell(""));
                }
                if r.cells[1].value != *new {
                    let quoted = r.cells[1].raw.starts_with('"');
                    r.cells[1] = Cell {
                        raw: quote(new, quoted),
                        value: new.clone(),
                    };
                }
                keep.push(r);
            }
            rows = keep;
            (0, 1, 2, None)
        }
    };
    for (k, v) in &want {
        if seen.contains(k) || v.is_empty() {
            continue;
        }
        let mut cells = vec![cell(""); width];
        cells[key] = cell(&quote(k, false));
        if let Some(i) = id {
            cells[i] = cell("0");
        }
        cells[col] = cell(&quote(v, false));
        // After its last sibling (`apple.few` after `apple.other`), else at the end.
        let at = k
            .rsplit_once('.')
            .and_then(|(parent, _)| {
                let p = format!("{parent}.");
                rows.iter()
                    .rposition(|r| r.cells.get(key).is_some_and(|c| c.value.starts_with(&p)))
            })
            .map_or(rows.len(), |i| i + 1);
        if at == rows.len()
            && let Some(last) = rows.last_mut()
            && last.end.is_empty()
        {
            last.end = nl.clone();
        }
        rows.insert(
            at,
            Row {
                cells,
                end: nl.clone(),
            },
        );
    }
    render(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNITY: &str = "Key,Id,Shared Comments,English(en),English(en) Comments,French(fr),French(fr) Comments\r\n\
EXAMPLE_KEY,1,Example comments.,Example text.,Example comments.,Exemple de texte.,Example comments.\r\n\
GREETING,2,Greeting text.,\"Hello, {0}!\",Greeting text in English.,,\r\n\
FAREWELL,3,,Bye.,,,\r\n";

    fn c(locale: &str, kv: &[(&str, &str)]) -> Catalog {
        Catalog {
            locale: locale.into(),
            entries: kv
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn unity_table_read_write() {
        let en = parse(UNITY, "en").unwrap();
        assert_eq!(en.entries["GREETING"], "Hello, {0}!");
        assert_eq!(en.notes["GREETING"], "Greeting text.");
        let fr = parse(UNITY, "fr").unwrap();
        assert_eq!(fr.entries.len(), 1, "empty cells are untranslated");
        let same = Catalog {
            locale: "fr".into(),
            entries: fr.entries.clone(),
        };
        assert_eq!(write(Some(UNITY), &same), UNITY, "byte for byte");

        let out = write(
            Some(UNITY),
            &c(
                "fr",
                &[
                    ("EXAMPLE_KEY", "Exemple de texte."),
                    ("GREETING", "Bonjour, {0} !"),
                    ("NEW", "Nouveau"),
                ],
            ),
        );
        assert!(out.contains("GREETING,2,Greeting text.,\"Hello, {0}!\",Greeting text in English.,\"Bonjour, {0} !\",\r\n"), "{out}");
        assert!(out.ends_with("NEW,0,,,,Nouveau,\r\n"), "{out}");
        assert!(out.contains("FAREWELL,3,,Bye.,,,\r\n"), "{out}");

        // A new locale gets its column; the source dropping a key removes its row.
        let de = write(Some(UNITY), &c("de", &[("GREETING", "Hallo, {0}!")]));
        assert!(de.starts_with("Key,Id,Shared Comments,English(en),English(en) Comments,French(fr),French(fr) Comments,German(de)\r\n"), "{de}");
        assert!(de.contains("FAREWELL,3,,Bye.,,,,\r\n"), "{de}");
        assert_eq!(parse(&de, "de").unwrap().entries["GREETING"], "Hallo, {0}!");
        let en2 = write(
            Some(UNITY),
            &c(
                "en",
                &[
                    ("GREETING", "Hello, {0}!"),
                    ("EXAMPLE_KEY", "Example text."),
                ],
            ),
        );
        assert!(!en2.contains("FAREWELL"), "{en2}");
    }

    #[test]
    fn slang_compact_and_per_locale() {
        let compact = "key,en,de-DE\nwelcome.title,Welcome $name,Willkommen $name\napple.one,One apple,Ein Apfel\napple.other,$n apples,$n Äpfel";
        let de = parse(compact, "de-DE").unwrap();
        assert_eq!(de.entries["welcome.title"], "Willkommen $name");
        assert_eq!(
            de.entries["apple"],
            "{_count, plural, one {Ein Apfel} other {$n Äpfel}}"
        );
        let same = Catalog {
            locale: "de-DE".into(),
            entries: de.entries.clone(),
        };
        assert_eq!(write(Some(compact), &same), compact);
        let fr = write(
            Some(compact),
            &c(
                "fr",
                &[(
                    "apple",
                    "{_count, plural, one {Une pomme} many {$n de pommes} other {$n pommes}}",
                )],
            ),
        );
        assert!(fr.starts_with("key,en,de-DE,fr\n"), "{fr}");
        assert!(
            fr.contains("apple.one,One apple,Ein Apfel,Une pomme\n"),
            "{fr}"
        );
        assert!(fr.ends_with("apple.many,,,$n de pommes\n"), "{fr}");
        let mid = write(
            Some("key,en\napple.one,a\napple.other,b\nz,c\n"),
            &c(
                "en",
                &[
                    ("apple", "{_count, plural, one {a} few {f} other {b}}"),
                    ("z", "c"),
                ],
            ),
        );
        assert_eq!(
            mid,
            "key,en\napple.one,a\napple.other,b\napple.few,f\nz,c\n"
        );

        let per = "welcome.title,\"Welcome, $name\"\npages.0.title,First Page\n";
        let p = parse(per, "en").unwrap();
        assert_eq!(p.entries["welcome.title"], "Welcome, $name");
        let same = Catalog {
            locale: "en".into(),
            entries: p.entries.clone(),
        };
        assert_eq!(write(Some(per), &same), per);
        assert_eq!(
            write(None, &c("fr", &[("a", "x, y"), ("b", "z")])),
            "a,\"x, y\"\nb,z\n"
        );
    }
}
