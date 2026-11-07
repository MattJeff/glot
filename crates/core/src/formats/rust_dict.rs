//! Rust string tables: hand-rolled i18n in Leptos / Yew / Dioxus apps (cliponit's `web/src/i18n.rs`).
//!
//! ```text
//! // (key, fr, en)
//! static DICT: &[(&str, &str, &str)] = &[
//!     ("login_title", "Connexion", "Log in"),
//! ];
//! ```
//!
//! Every `static` / `const` slice of `&str` tuples is a table: column 0 is the key, the others are
//! locales, named by a comment above the table (`// (clé, fr, en)`, `/// Libellés (FR, EN)`) or else
//! by a tuple pattern of the lookup code (`|(_, fr, en)|`, `for (key, fr, en) in DICT`). Every
//! locale lives in the same file (no `{locale}` in the glot.toml path).
//! Writing patches string literals in place; a key new to the file is appended as a tuple to the
//! first table (other columns `""`); a key gone from a locale empties its cell, and a row whose
//! locale cells are all empty is removed. Columns are never added (the lookup code destructures
//! fixed-size tuples): a locale without a column lives in a flat JSON catalog next to the crate,
//! `{crate}/locales/{locale}.json` (`{"key": "text"}`), that the lookup code loads as a fallback
//! (`include_str!("../locales/de.json")` + `serde_json`, or `glot.loadCatalog`).
// ponytail: fixed-size arrays (`[(&str, &str); 6]`) are not tables: appending would break the length.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::Parsed;
use crate::catalog::Catalog;

/// ISO 639-1 codes: a tuple pattern `(_, fr, en)` names columns only with real language codes.
const ISO639: &str = "aa ab ae af ak am an ar as av ay az ba be bg bh bi bm bn bo br bs ca ce ch co cr cs cu cv cy da de dv dz ee el en eo es et eu fa ff fi fj fo fr fy ga gd gl gn gu gv ha he hi ho hr ht hu hy hz ia id ie ig ii ik io is it iu ja jv ka kg ki kj kk kl km kn ko kr ks ku kv kw ky la lb lg li ln lo lt lu lv mg mh mi mk ml mn mr ms mt my na nb nd ne ng nl nn no nr nv ny oc oj om or os pa pi pl ps pt qu rm rn ro ru rw sa sc sd se sg si sk sl sm sn so sq sr ss st su sv sw ta te tg th ti tk tl tn to tr ts tt tw ty ug uk ur uz ve vi vo wa wo xh yi yo za zh zu";

#[derive(Debug, Clone)]
struct Lit {
    start: usize,
    end: usize,
    value: String,
    /// `r#"…"#`: the number of `#`.
    raw: Option<usize>,
}

#[derive(Debug, Clone)]
struct Row {
    /// `(` .. past `)` (and its `,` when present).
    start: usize,
    end: usize,
    close: usize,
    comma: bool,
    cells: Vec<Lit>,
}

/// One string table of a Rust file.
#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    /// Locale of columns 1.., BCP 47 (`pt_br` => `pt-BR`); empty when the file does not say.
    pub locales: Vec<String>,
    /// Tuple arity (key + locale columns).
    pub arity: usize,
    rows: Vec<Row>,
    /// Index of the closing `]`.
    close: usize,
}

impl Table {
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|r| r.cells[0].value.as_str())
    }

    /// Keys look like keys (`login_title`, `nav.home`), not prose: a lookup table, not a
    /// `("Personnages", "People")` mapping from source text (never written).
    pub fn is_keyed(&self) -> bool {
        self.keys().all(|k| {
            !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':'))
        })
    }

    fn column(&self, locale: &str) -> Option<usize> {
        let norm = |l: &str| l.replace('_', "-").to_ascii_lowercase();
        self.locales
            .iter()
            .position(|l| norm(l) == norm(locale))
            .map(|i| i + 1)
    }
}

struct Cur<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cur<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    /// Whitespace and comments.
    fn ws(&mut self) {
        loop {
            let r = self.rest();
            let t = r.trim_start();
            self.i += r.len() - t.len();
            if t.starts_with("//") {
                self.i += t.find('\n').unwrap_or(t.len());
            } else if t.starts_with("/*") {
                self.i += t.find("*/").map_or(t.len(), |n| n + 2);
            } else {
                return;
            }
        }
    }

    fn eat(&mut self, tok: &str) -> bool {
        self.ws();
        let ok = self.rest().starts_with(tok);
        if ok {
            self.i += tok.len();
        }
        ok
    }

    fn ident(&mut self) -> Option<String> {
        self.ws();
        let n = self
            .rest()
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(self.rest().len());
        (n > 0).then(|| {
            let id = self.rest()[..n].to_string();
            self.i += n;
            id
        })
    }

    /// `&str` / `&'static str`.
    fn str_ty(&mut self) -> bool {
        self.eat("&") && {
            if self.eat("'") {
                self.ident();
            }
            self.ident().as_deref() == Some("str")
        }
    }

    /// A string literal at the cursor.
    fn lit(&mut self) -> Option<Lit> {
        self.ws();
        let start = self.i;
        let r = self.rest();
        if let Some(raw) = r.strip_prefix('r') {
            let hashes = raw.len() - raw.trim_start_matches('#').len();
            let body = raw[hashes..].strip_prefix('"')?;
            let close = format!("\"{}", "#".repeat(hashes));
            let n = body.find(&close)?;
            self.i += 1 + hashes + 1 + n + close.len();
            return Some(Lit {
                start,
                end: self.i,
                value: body[..n].to_string(),
                raw: Some(hashes),
            });
        }
        let body = r.strip_prefix('"')?;
        let (mut out, mut chars) = (String::new(), body.char_indices());
        while let Some((k, c)) = chars.next() {
            match c {
                '"' => {
                    self.i += 1 + k + 1;
                    return Some(Lit {
                        start,
                        end: self.i,
                        value: out,
                        raw: None,
                    });
                }
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    'x' => {
                        let h: String = (0..2).filter_map(|_| chars.next().map(|c| c.1)).collect();
                        out.push(char::from(u8::from_str_radix(&h, 16).ok()?));
                    }
                    'u' => {
                        let mut h = String::new();
                        for (_, c) in chars.by_ref() {
                            match c {
                                '{' => {}
                                '}' => break,
                                c => h.push(c),
                            }
                        }
                        out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
                    }
                    // Line continuation: the newline and the next line's indentation are skipped.
                    '\n' | '\r' => {
                        while chars.clone().next().is_some_and(|(_, c)| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    c => out.push(c),
                },
                c => out.push(c),
            }
        }
        None
    }
}

/// `pt_br` => `pt-BR`, `zh_hans` => `zh-Hans`, `fr` => `fr`; `None` unless a language code.
fn locale_ident(id: &str) -> Option<String> {
    let id = id.to_ascii_lowercase();
    let (lang, region) = id
        .split_once(['_', '-'])
        .map_or((id.as_str(), None), |(l, r)| (l, Some(r)));
    if lang.len() != 2 || !ISO639.split(' ').any(|c| c == lang) {
        return None;
    }
    let region = match region {
        None => String::new(),
        Some(r) if r.len() == 2 && r.chars().all(|c| c.is_ascii_alphabetic()) => {
            format!("-{}", r.to_ascii_uppercase())
        }
        Some(r) if r.len() == 3 && r.chars().all(|c| c.is_ascii_digit()) => format!("-{r}"),
        Some(r) if r.len() == 4 && r.chars().all(|c| c.is_ascii_alphabetic()) => {
            format!("-{}{}", r[..1].to_ascii_uppercase(), &r[1..])
        }
        Some(_) => return None,
    };
    Some(format!("{lang}{region}"))
}

/// Locale names in `(a, b, c)` groups of `text` for a table of `arity` columns (key first).
fn names_in(text: &str, arity: usize) -> Option<Vec<String>> {
    let mut rest = text;
    while let Some(o) = rest.find('(') {
        let after = &rest[o + 1..];
        let Some(c) = after.find(')') else { break };
        let items: Vec<&str> = after[..c].split([',', '/']).map(str::trim).collect();
        let locs = match items.len() {
            n if n == arity => &items[1..],
            n if n + 1 == arity => &items[..],
            _ => &[][..],
        };
        if locs.len() + 1 == arity
            && let Some(v) = locs
                .iter()
                .map(|l| locale_ident(l))
                .collect::<Option<Vec<_>>>()
        {
            return Some(v);
        }
        rest = after;
    }
    None
}

/// Past the `)` closing the tuple opened at `open` (strings and nested parentheses skipped).
fn skip_tuple(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut chars = s[open..].char_indices();
    while let Some((k, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + k + 1);
                }
            }
            '"' => {
                while let Some((_, c)) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Every string table of a Rust source file.
pub fn tables(s: &str) -> Result<Vec<Table>, String> {
    let mut out = vec![];
    let mut line_start = 0;
    for line in s.split_inclusive('\n') {
        let at = line_start;
        line_start += line.len();
        let t = line.trim_start();
        let t = t
            .strip_prefix("pub(crate) ")
            .or_else(|| t.strip_prefix("pub(super) "))
            .or_else(|| t.strip_prefix("pub "))
            .unwrap_or(t);
        let Some(decl) = t
            .strip_prefix("static ")
            .or_else(|| t.strip_prefix("const "))
        else {
            continue;
        };
        let mut c = Cur {
            s,
            i: at + line.len() - decl.len(),
        };
        let Some(name) = c.ident() else { continue };
        if !(c.eat(":") && c.eat("&") && c.eat("[") && c.eat("(")) {
            continue;
        }
        let mut arity = 0;
        while c.str_ty() {
            arity += 1;
            if !c.eat(",") {
                break;
            }
        }
        if arity < 2 || !(c.eat(")") && c.eat("]") && c.eat("=") && c.eat("&") && c.eat("[")) {
            continue;
        }
        let mut rows = vec![];
        let close = loop {
            c.ws();
            if c.rest().starts_with(']') {
                break c.i;
            }
            let start = c.i;
            if !c.eat("(") {
                return Err(format!(
                    "table `{name}`: expected a `(\"key\", …)` tuple at byte {}",
                    c.i
                ));
            }
            let mut cells = vec![];
            for _ in 0..arity {
                let Some(lit) = c.lit() else { break };
                cells.push(lit);
                c.eat(",");
            }
            // `(MINE, "My elements")`: not literals, left as is and not translated.
            if cells.len() < arity {
                c.i = skip_tuple(s, start)
                    .ok_or_else(|| format!("table `{name}`: unclosed tuple at byte {start}"))?;
                c.eat(",");
                continue;
            }
            if !c.eat(")") {
                return Err(format!("table `{name}`: unclosed tuple at byte {start}"));
            }
            let close = c.i;
            let comma = c.eat(",");
            rows.push(Row {
                start,
                end: c.i,
                close,
                comma,
                cells,
            });
        };
        // Column names: the comment block above the declaration, else the lookup code's patterns.
        let above: String = s[..at]
            .lines()
            .rev()
            .map(str::trim)
            .take_while(|l| l.starts_with("//") || l.starts_with("#["))
            .collect::<Vec<_>>()
            .join("\n");
        let locales = names_in(&above, arity)
            .or_else(|| names_in(s, arity))
            .unwrap_or_default();
        out.push(Table {
            name,
            locales,
            arity,
            rows,
            close,
        });
    }
    Ok(out)
}

pub fn parse(s: &str, locale: &str) -> Result<Parsed, String> {
    let ts = tables(s)?;
    if ts.is_empty() {
        return Err("no string table (`static DICT: &[(&str, &str, &str)] = &[(\"key\", \"fr\", \"en\"), …];`) in this file".into());
    }
    let mut p = Parsed::default();
    let mut found = false;
    for t in ts.iter().filter(|t| t.is_keyed()) {
        let Some(col) = t.column(locale) else {
            continue;
        };
        found = true;
        for r in &t.rows {
            let v = &r.cells[col].value;
            if !v.is_empty() {
                p.entries
                    .entry(r.cells[0].value.clone())
                    .or_insert_with(|| v.clone());
            }
        }
    }
    if !found {
        let Some(t) = ts.iter().find(|t| t.is_keyed()) else {
            return Err("no keyed string table in this file (its first column is text, not `snake_case` keys)".into());
        };
        return Err(if t.locales.is_empty() {
            format!(
                "table `{}`: which column is which locale? Add a comment right above it naming the columns, e.g. `// (key, fr, en)`",
                t.name
            )
        } else {
            format!(
                "table `{}` has no `{locale}` column (columns: {}); add it to every tuple, the table type and the lookup code, or remove `{locale}` from target_locales",
                t.name,
                t.locales.join(", ")
            )
        });
    }
    Ok(p)
}

/// `{crate}/locales/{locale}/{file}.json` when the file's tables name their columns and none is
/// `locale`.
pub fn sidecar(path: &Path, text: &str, locale: &str) -> Option<PathBuf> {
    let ts = tables(text).ok()?;
    let keyed: Vec<&Table> = ts.iter().filter(|t| t.is_keyed()).collect();
    if keyed.is_empty()
        || keyed
            .iter()
            .any(|t| t.locales.is_empty() || t.column(locale).is_some())
    {
        return None;
    }
    let dir = path.parent()?;
    let base = dir
        .ancestors()
        .find(|d| d.join("Cargo.toml").is_file())
        .unwrap_or(dir);
    // One catalog per Rust file: two tables may use the same key (`a1` in landing and tools).
    // ponytail: by file stem; two `mod.rs` tables of one crate would share a file.
    let stem = path.file_stem()?.to_string_lossy();
    Some(
        base.join("locales")
            .join(locale)
            .join(format!("{stem}.json")),
    )
}

fn read_sidecar(json: &Path) -> Result<BTreeMap<String, String>, String> {
    match std::fs::read_to_string(json) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| {
            format!(
                "{}: expected a flat {{\"key\": \"text\"}} object: {e}",
                json.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("{}: {e}", json.display())),
    }
}

pub fn parse_sidecar(json: &Path) -> Result<Parsed, String> {
    Ok(Parsed {
        entries: read_sidecar(json)?
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .collect(),
        ..Default::default()
    })
}

/// The sidecar catalog for `cat` (sorted keys), `None` when it would not change.
pub fn write_sidecar(json: &Path, cat: &Catalog) -> Result<Option<String>, String> {
    let new: BTreeMap<&String, &String> =
        cat.entries.iter().filter(|(_, v)| !v.is_empty()).collect();
    let old = read_sidecar(json)?;
    if json.exists() && new.len() == old.len() && new.iter().all(|(k, v)| old.get(*k) == Some(*v)) {
        return Ok(None);
    }
    Ok(Some(
        serde_json::to_string_pretty(&new).map_err(|e| e.to_string())? + "\n",
    ))
}

fn render(v: &str, raw: Option<usize>) -> String {
    if let Some(h) = raw
        && !v.contains(&format!("\"{}", "#".repeat(h)))
    {
        let h = "#".repeat(h);
        return format!("r{h}\"{v}\"{h}");
    }
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '\\' => out += "\\\\",
            '"' => out += "\\\"",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            '\0' => out += "\\0",
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let Some(s) = existing else {
        let mut out = format!(
            "// (key, {})\npub static DICT: &[(&str, &str)] = &[\n",
            cat.locale
        );
        for (k, v) in &cat.entries {
            out += &format!("    ({}, {}),\n", render(k, None), render(v, None));
        }
        return out + "];\n";
    };
    let Ok(ts) = tables(s) else {
        return s.to_string();
    };
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let mut seen = std::collections::BTreeSet::new();
    let mut home = None;
    for t in ts.iter().filter(|t| t.is_keyed()) {
        let Some(col) = t.column(&cat.locale) else {
            continue;
        };
        home.get_or_insert((t, col));
        for r in &t.rows {
            let key = &r.cells[0].value;
            let cell = &r.cells[col];
            // A later duplicate is dead code (the lookup finds the first row): left as is.
            if !seen.insert(key.clone()) {
                continue;
            }
            match cat.entries.get(key) {
                Some(v) if *v == cell.value => {}
                Some(v) => edits.push((cell.start, cell.end, render(v, cell.raw))),
                None if cell.value.is_empty() => {}
                None if r.cells[1..]
                    .iter()
                    .enumerate()
                    .all(|(i, c)| i + 1 == col || c.value.is_empty()) =>
                {
                    // The whole line when the row is alone on it.
                    let ls = s[..r.start].rfind('\n').map_or(0, |n| n + 1);
                    let le = s[r.end..].find('\n').map_or(s.len(), |n| r.end + n + 1);
                    if s[ls..r.start].trim().is_empty() && s[r.end..le].trim().is_empty() {
                        edits.push((ls, le, String::new()));
                    } else {
                        let sp = s[r.end..].len() - s[r.end..].trim_start_matches(' ').len();
                        edits.push((r.start, r.end + sp, String::new()));
                    }
                }
                None => edits.push((cell.start, cell.end, "\"\"".into())),
            }
        }
    }
    if let Some((t, col)) = home {
        let fresh: Vec<String> = cat
            .entries
            .iter()
            .filter(|(k, _)| !seen.contains(*k))
            .map(|(k, v)| {
                let cells: Vec<String> = (0..t.arity)
                    .map(|i| match i {
                        0 => render(k, None),
                        i if i == col => render(v, None),
                        _ => "\"\"".into(),
                    })
                    .collect();
                format!("({}),", cells.join(", "))
            })
            .collect();
        if !fresh.is_empty() {
            if let Some(last) = t.rows.last().filter(|r| !r.comma) {
                edits.push((last.close, last.close, ",".into()));
            }
            let ls = s[..t.close].rfind('\n').map_or(0, |n| n + 1);
            let indent = t
                .rows
                .last()
                .map(|r| {
                    let l = s[..r.start].rfind('\n').map_or(0, |n| n + 1);
                    s[l..r.start].to_string()
                })
                .filter(|i| i.trim().is_empty())
                .unwrap_or_else(|| "    ".into());
            let text: String = fresh.iter().map(|r| format!("{indent}{r}\n")).collect();
            if s[ls..t.close].trim().is_empty() {
                edits.push((ls, ls, text));
            } else {
                edits.push((t.close, t.close, format!("\n{text}")));
            }
        }
    }
    edits.sort_by_key(|e| (e.0, e.1));
    let (mut out, mut pos) = (String::new(), 0);
    for (a, b, text) in edits {
        if a < pos {
            continue;
        }
        out += &s[pos..a];
        out += &text;
        pos = b;
    }
    out + &s[pos..]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r##"//! Dictionnaire FR/EN.
use leptos::prelude::*;

// (clé, fr, en)
static DICT: &[(&str, &str, &str)] = &[
    // auth
    ("login_title", "Connexion", "Log in"),
    ("quote", "Tape \"oui\"\npour confirmer", "Type \"yes\"\nto confirm"),
    ("raw", r#"Le "vrai" {n}"#, r#"The "real" {n}"#),
    ("a", "A fr", "A en"), ("b", "B fr", "B en"),
    ("cont", "une \
              ligne", "one line"),
    ("login_title", "Connexion", "Log in"),
];

/// Libellés (FR, EN) par élément.
const LABELS: &[(&'static str, &'static str, &'static str)] = &[("cut", "Coupe", "Cut"), (MINE, "Mes", "Mine")];

const STYLES: [(&str, &str, &str); 1] = [("x", "y", "z")];

const FAMILIES: &[(&str, &str)] = &[("Personnages", "People"), ("Documents & lieux", "Documents & places")];

pub fn tl(key: &str, en: bool) -> String {
    DICT.iter().find(|(k, _, _)| *k == key).map(|(_, fr, en_txt)| if en { *en_txt } else { *fr }).unwrap_or(key).to_string()
}
"##;

    fn cat(locale: &str, p: &Parsed) -> Catalog {
        Catalog {
            locale: locale.into(),
            entries: p.entries.clone(),
        }
    }

    #[test]
    fn reads_columns() {
        let ts = tables(SRC).unwrap();
        assert_eq!(ts.len(), 3, "fixed-size arrays are not tables");
        assert!(!ts[2].is_keyed(), "source-text mapping");
        assert_eq!(ts[0].locales, ["fr", "en"]);
        assert_eq!(ts[1].locales, ["fr", "en"]);
        let fr = parse(SRC, "fr").unwrap();
        assert_eq!(fr.entries["login_title"], "Connexion");
        assert_eq!(fr.entries["quote"], "Tape \"oui\"\npour confirmer");
        assert_eq!(fr.entries["raw"], "Le \"vrai\" {n}");
        assert_eq!(fr.entries["cont"], "une ligne");
        assert_eq!(fr.entries["cut"], "Coupe");
        assert_eq!(fr.entries.len(), 7);
        let en = parse(SRC, "en").unwrap();
        assert_eq!(en.entries["b"], "B en");
        let err = parse(SRC, "de").unwrap_err();
        assert!(err.contains("no `de` column"), "{err}");
        // Columns named only by the lookup code.
        let bare = "static D: &[(&str, &str, &str)] = &[(\"k\", \"a\", \"b\")];\nfn f() { D.iter().map(|(_, de, pt_br)| 1); }\n";
        assert_eq!(tables(bare).unwrap()[0].locales, ["de", "pt-BR"]);
        let unknown = "static D: &[(&str, &str, &str)] = &[(\"k\", \"a\", \"b\")];\n";
        assert!(
            parse(unknown, "fr")
                .unwrap_err()
                .contains("// (key, fr, en)")
        );
    }

    #[test]
    fn roundtrip_and_edits() {
        for l in ["fr", "en"] {
            let p = parse(SRC, l).unwrap();
            assert_eq!(write(Some(SRC), &cat(l, &p)), SRC, "{l} byte for byte");
        }
        let mut c = cat("en", &parse(SRC, "en").unwrap());
        c.entries.insert("login_title".into(), "Sign in".into());
        c.entries.insert("raw".into(), "The \"new\" {n}".into());
        c.entries.insert("quote".into(), "Say \"yes\"\\ok".into());
        c.entries.remove("a");
        c.entries.insert("new_key".into(), "New".into());
        let out = write(Some(SRC), &c);
        assert!(
            out.contains("(\"login_title\", \"Connexion\", \"Sign in\"),"),
            "{out}"
        );
        assert!(
            out.contains("\n    (\"login_title\", \"Connexion\", \"Log in\"),\n    (\"new_key\""),
            "a later duplicate is dead code, untouched: {out}"
        );
        assert!(out.contains("r#\"The \"new\" {n}\"#"), "{out}");
        assert!(out.contains("\"Say \\\"yes\\\"\\\\ok\""), "{out}");
        assert!(out.contains("(\"a\", \"A fr\", \"\"), (\"b\""), "{out}");
        assert!(
            out.contains("\"Log in\"),\n    (\"new_key\", \"\", \"New\"),\n];"),
            "{out}"
        );
        assert_eq!(parse(&out, "en").unwrap().entries, c.entries);
        assert_eq!(parse(&out, "fr").unwrap().entries.len(), 7, "fr untouched");
        // Emptying the last locale cell drops the row.
        let mut f = cat("fr", &parse(&out, "fr").unwrap());
        f.entries.remove("a");
        let out2 = write(Some(&out), &f);
        assert!(!out2.contains("(\"a\""), "{out2}");
        assert!(
            out2.contains("    (\"b\", \"B fr\", \"B en\"),\n"),
            "{out2}"
        );
    }

    #[test]
    fn extra_locale_goes_to_sidecar_json() {
        let d = std::env::temp_dir().join(format!("glot-rust-sidecar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("web/src/pages")).unwrap();
        std::fs::write(d.join("web/Cargo.toml"), "[package]\n").unwrap();
        let rs = d.join("web/src/i18n.rs");
        std::fs::write(&rs, SRC).unwrap();
        // `a` again: the page has its own table (and lookup) for it.
        let page = d.join("web/src/pages/p.rs");
        let page_src = "// (key, fr, en)\nstatic P: &[(&str, &str, &str)] = &[(\"p_k\", \"P\", \"P\"), (\"a\", \"x\", \"y\")];\n";
        std::fs::write(&page, page_src).unwrap();
        let json = d.join("web/locales/de/i18n.json");
        assert_eq!(sidecar(&rs, SRC, "de"), Some(json.clone()));
        assert_eq!(sidecar(&rs, SRC, "en"), None);
        let read = |p: &Path| {
            crate::formats::read(p, "de", crate::catalog::Format::Auto)
                .unwrap()
                .unwrap()
                .entries
        };
        assert!(read(&rs).is_empty(), "no file yet: nothing translated");
        let write = |p: &Path, c: &Catalog| {
            crate::formats::write_path(crate::catalog::Format::RustDict, p, c).unwrap()
        };
        let mut c = Catalog::new("de");
        c.entries.insert("login_title".into(), "Anmelden".into());
        c.entries.insert("a".into(), "A de".into());
        write(&rs, &c);
        assert_eq!(
            std::fs::read_to_string(&rs).unwrap(),
            SRC,
            "Rust code untouched"
        );
        assert_eq!(
            std::fs::read_to_string(&json).unwrap(),
            "{\n  \"a\": \"A de\",\n  \"login_title\": \"Anmelden\"\n}\n"
        );
        let mut pc = Catalog::new("de");
        pc.entries.insert("a".into(), "Y de".into());
        write(&page, &pc);
        assert_eq!(read(&rs), c.entries);
        assert_eq!(read(&page), pc.entries, "same key, own file");
        assert!(d.join("web/locales/de/p.json").is_file());
        c.entries.remove("a");
        write(&rs, &c);
        assert_eq!(read(&rs), c.entries);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn appends_to_single_line_table() {
        let s = "static T: &[(&str, &str, &str)] = &[(\"k\", \"x\", \"y\")];\n// (key, fr, en)\n";
        let mut c = cat("fr", &parse(s, "fr").unwrap());
        c.entries.insert("n".into(), "N".into());
        let out = write(Some(s), &c);
        assert_eq!(
            out,
            "static T: &[(&str, &str, &str)] = &[(\"k\", \"x\", \"y\"),\n    (\"n\", \"N\", \"\"),\n];\n// (key, fr, en)\n"
        );
    }
}
