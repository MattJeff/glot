//! Java / Spring `.properties` (`messages.properties` = source, `messages_fr.properties`).
//! `key=value`, `key: value`, `key value`; `#`/`!` comments (the block right above an entry is its
//! note); `\` line continuations; `\uXXXX`, `\n`, `\t` escapes. Values keep MessageFormat text as
//! written (`{0}`, `''`). ISO-8859-1 files are decoded by `formats::read_text`.
//! Writing patches values in place; a file without raw non-ASCII text (the classic `\u00e9` style,
//! and every new file) gets `\uXXXX` escapes so ISO-8859-1 and UTF-8 readers agree.
use super::Parsed;
use crate::catalog::Catalog;

struct Entry {
    key: String,
    value: String,
    note: Option<String>,
    /// Line start, value start, value end (before the newline), after the newline.
    start: usize,
    vstart: usize,
    vend: usize,
    end: usize,
    /// Separator as written (`=`, ` = `, `: `).
    sep: String,
}

fn ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\x0C')
}

/// Line continuation at `i` (`\` + newline): index after it and the next line's indent.
fn continuation(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'\\') {
        return None;
    }
    let mut j = match (b.get(i + 1), b.get(i + 2)) {
        (Some(b'\r'), Some(b'\n')) => i + 3,
        (Some(b'\n' | b'\r'), _) => i + 2,
        _ => return None,
    };
    while j < b.len() && ws(b[j]) {
        j += 1;
    }
    Some(j)
}

/// Skip whitespace and continuations from `j`.
fn skip(b: &[u8], mut j: usize) -> usize {
    loop {
        match continuation(b, j) {
            Some(n) => j = n,
            None if j < b.len() && ws(b[j]) => j += 1,
            None => return j,
        }
    }
}

fn entries(s: &str) -> Vec<Entry> {
    let b = s.as_bytes();
    let eol = |from: usize| s[from..].find('\n').map_or(s.len(), |n| from + n);
    let (mut out, mut note, mut i) = (vec![], Vec::<String>::new(), 0);
    while i < b.len() {
        let start = i;
        let mut j = i;
        while j < b.len() && ws(b[j]) {
            j += 1;
        }
        if j >= b.len() || matches!(b[j], b'\n' | b'\r') {
            note.clear();
            i = eol(j) + 1;
            continue;
        }
        if matches!(b[j], b'#' | b'!') {
            let e = eol(j);
            note.push(s[j + 1..e].trim().to_string());
            i = e + 1;
            continue;
        }
        // Key: up to an unescaped `=`, `:` or whitespace.
        let kstart = j;
        while j < b.len() && !matches!(b[j], b'=' | b':' | b'\n' | b'\r') && !ws(b[j]) {
            j = if b[j] == b'\\' {
                continuation(b, j).unwrap_or(j + 2)
            } else {
                j + 1
            };
        }
        let kend = j.min(b.len());
        j = skip(b, kend);
        if j < b.len() && matches!(b[j], b'=' | b':') {
            j = skip(b, j + 1);
        }
        let vstart = j.min(b.len());
        while j < b.len() && !matches!(b[j], b'\n' | b'\r') {
            j = if b[j] == b'\\' {
                continuation(b, j).unwrap_or(j + 2)
            } else {
                j + 1
            };
        }
        let vend = j.min(b.len());
        let end = (eol(vend) + 1).min(b.len());
        out.push(Entry {
            key: unescape(&s[kstart..kend]),
            value: unescape(&s[vstart..vend]),
            note: (!note.is_empty()).then(|| note.join("\n")),
            start,
            vstart,
            vend,
            end,
            sep: s[kend..vstart].to_string(),
        });
        note.clear();
        i = end.max(start + 1);
    }
    out
}

fn unescape(raw: &str) -> String {
    let b = raw.as_bytes();
    let mut units: Vec<u16> = vec![];
    let mut buf = [0u16; 2];
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            let c = raw[i..].chars().next().unwrap();
            units.extend_from_slice(c.encode_utf16(&mut buf));
            i += c.len_utf8();
            continue;
        }
        if let Some(n) = continuation(b, i) {
            i = n;
            continue;
        }
        let Some(c) = raw[i + 1..].chars().next() else {
            break;
        };
        i += 1 + c.len_utf8();
        let c = match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'f' => '\x0C',
            'u' => {
                match raw
                    .get(i..i + 4)
                    .and_then(|h| u16::from_str_radix(h, 16).ok())
                {
                    Some(u) => {
                        units.push(u);
                        i += 4;
                    }
                    None => units.push(u16::from(b'u')),
                }
                continue;
            }
            c => c,
        };
        units.extend_from_slice(c.encode_utf16(&mut buf));
    }
    String::from_utf16_lossy(&units)
}

fn escape(v: &str, key: bool, ascii: bool) -> String {
    let mut out = String::with_capacity(v.len());
    for (i, c) in v.char_indices() {
        match c {
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            '\x0C' => out += "\\f",
            ' ' if key || i == 0 => out += "\\ ",
            '=' | ':' if key => {
                out.push('\\');
                out.push(c);
            }
            '#' | '!' if key && i == 0 => {
                out.push('\\');
                out.push(c);
            }
            c if ascii && !c.is_ascii() => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out += &format!("\\u{u:04X}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

pub fn parse(s: &str) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    for e in entries(s) {
        if let Some(n) = e.note {
            p.notes.insert(e.key.clone(), n);
        }
        p.entries.insert(e.key, e.value);
    }
    Ok(p)
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or("");
    let list = entries(s);
    // Raw non-ASCII already in the file: it is UTF-8 (or decoded ISO-8859-1), keep writing raw.
    let ascii = s.is_ascii();
    let (mut out, mut pos) = (String::new(), 0);
    let mut done = std::collections::BTreeSet::new();
    for e in &list {
        done.insert(e.key.clone());
        match cat.entries.get(&e.key) {
            Some(v) if *v == e.value => {}
            Some(v) => {
                out += &s[pos..e.vstart];
                out += &escape(v, false, ascii);
                pos = e.vend;
            }
            None => {
                out += &s[pos..e.start];
                pos = e.end;
            }
        }
    }
    out += &s[pos..];
    let sep = list.first().map_or("=", |e| e.sep.as_str());
    let sep = if sep.contains('\\') || sep.is_empty() {
        "="
    } else {
        sep
    };
    for (k, v) in cat.entries.iter().filter(|(k, _)| !done.contains(*k)) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out += &format!(
            "{}{sep}{}\n",
            escape(k, true, ascii),
            escape(v, false, ascii)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/formats/messages_fr.properties"
    ));

    fn cat(p: &Parsed) -> Catalog {
        Catalog {
            locale: "fr".into(),
            entries: p.entries.clone(),
        }
    }

    #[test]
    fn parse_roundtrip_edit() {
        let p = parse(FIX).unwrap();
        let e = &p.entries;
        assert_eq!(e["app.title"], "Gestion des commandes");
        assert_eq!(e["greeting"], "Bonjour {0}, vous avez {1,number} messages");
        assert_eq!(e["quote"], "L''utilisateur {0} est connecté");
        assert_eq!(
            e["long.text"],
            "Première ligne qui continue sur la suivante"
        );
        assert_eq!(e["colon.key"], "valeur: avec deux-points");
        assert_eq!(e["space key"], "clé avec espace");
        assert_eq!(e["emoji"], "\u{1F600} ok");
        assert_eq!(e["multi"], "a\nb\tc");
        assert_eq!(e["empty"], "");
        assert_eq!(p.notes["app.title"], "Titre de l'application");
        assert!(
            !p.notes.contains_key("greeting"),
            "blank line resets the note"
        );
        assert_eq!(write(Some(FIX), &cat(&p)), FIX, "byte for byte");
        let mut c = cat(&p);
        c.entries.insert("app.title".into(), "Commandes été".into());
        c.entries.remove("empty");
        c.entries
            .insert("new.key".into(), "Nouvelle clé = ok".into());
        let out = write(Some(FIX), &c);
        assert!(
            out.contains("app.title = Commandes \\u00E9t\\u00E9\n"),
            "{out}"
        );
        assert!(!out.contains("empty"));
        assert!(
            out.ends_with("new.key = Nouvelle cl\\u00E9 = ok\n"),
            "{out}"
        );
        assert_eq!(parse(&out).unwrap().entries, c.entries);
    }

    #[test]
    fn latin1_file() {
        use crate::catalog::Format;
        let dir = std::env::temp_dir().join(format!("glot-props-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("messages_fr.properties");
        std::fs::write(&p, b"a=\xe9t\xe9\n").unwrap();
        let mut c = cat(&super::super::read(&p, "fr", Format::Auto).unwrap().unwrap());
        assert_eq!(c.entries["a"], "\u{e9}t\u{e9}");
        c.entries.insert("b".into(), "\u{20ac} \u{e7}a".into());
        super::super::write_path(Format::Properties, &p, &c).unwrap();
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"a=\xe9t\xe9\nb=\\u20AC \xe7a\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn utf8_file_stays_raw_and_new_file() {
        let s = "a=été\n";
        let mut c = cat(&parse(s).unwrap());
        c.entries.insert("b".into(), "ça".into());
        assert_eq!(write(Some(s), &c), "a=été\nb=ça\n");
        let fresh = write(None, &c);
        assert_eq!(fresh, "a=\\u00E9t\\u00E9\nb=\\u00E7a\n");
        assert_eq!(parse(&fresh).unwrap().entries, c.entries);
        let mut d = Catalog::new("fr");
        d.entries.insert("k:x y".into(), " lead\\#".into());
        let w = write(None, &d);
        assert_eq!(parse(&w).unwrap().entries, d.entries, "{w}");
    }
}
