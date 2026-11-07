//! vue-i18n single-file components: the `<i18n>` custom blocks of `.vue` files
//! (`@intlify/unplugin-vue-i18n`). A block holds every locale (`{"en": {…}, "ja": {…}}`, or
//! `en:` / `ja:` sections with `lang="yaml"`), or one locale with `<i18n locale="ja">`.
//! `src="…"` blocks point at a normal catalog and are skipped; JSON5 (`lang="json5"`, comments,
//! unquoted keys) goes through the JSON5 patcher, so its comments survive.
//!
//! Writing patches only the locale's part of its block: the rest of the component (template,
//! script, style, the other locales) keeps its bytes. A locale the file lacks becomes a new
//! section of the multi-locale block, or a new `<i18n locale="…">` block after the last one.
//! glot keys are prefixed by the component path (`src/{ns}.vue` => `components/Hello:title`).
use std::path::Path;

use super::{Parsed, json5, yaml};
use crate::catalog::{Catalog, Format, Layout};

struct Block {
    /// Byte range of the content between `<i18n …>` and `</i18n>`.
    body: (usize, usize),
    /// After `</i18n>`.
    end: usize,
    locale: Option<String>,
    yaml: bool,
    /// The `lang` attribute as written (copied to a new per-locale block).
    lang: Option<String>,
}

fn same_locale(a: &str, b: &str) -> bool {
    a.replace('_', "-")
        .eq_ignore_ascii_case(&b.replace('_', "-"))
}

/// Top-level `<i18n>` blocks (at the start of a line).
fn blocks(s: &str) -> Vec<Block> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(off) = s[i..].find("<i18n") {
        let at = i + off;
        i = at + 5;
        let top = s[..at]
            .rsplit('\n')
            .next()
            .is_some_and(|l| l.trim().is_empty());
        if !top || !s[i..].starts_with(|c: char| c == '>' || c.is_whitespace()) {
            continue;
        }
        let Some(gt) = s[i..].find('>') else { break };
        let head = &s[i..i + gt];
        let start = i + gt + 1;
        if head.trim_end().ends_with('/') {
            i = start;
            continue;
        }
        let Some(close) = s[start..].find("</i18n>") else {
            break;
        };
        let attr = |name: &str| {
            head.split_whitespace().find_map(|t| {
                let (k, v) = t.split_once('=')?;
                (k == name).then(|| v.trim_matches(['"', '\'']).to_string())
            })
        };
        i = start + close + 7;
        if attr("src").is_some() {
            continue;
        }
        let lang = attr("lang");
        out.push(Block {
            body: (start, start + close),
            end: i,
            locale: attr("locale"),
            yaml: lang.as_deref().is_some_and(|l| l == "yaml" || l == "yml"),
            lang,
        });
    }
    out
}

/// Index after the JSON string starting at `b[i]` (a `"`).
/// Index after the string starting at `b[i]` (`"` or JSON5 `'`).
fn skip_string(b: &[u8], mut i: usize) -> usize {
    let q = b[i];
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == q => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// Index after whitespace, commas and JSON5 comments from `b[i]`.
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        match (b.get(i), b.get(i + 1)) {
            (Some(b'/'), Some(b'/')) => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            (Some(b'/'), Some(b'*')) => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            _ => return i.min(b.len()),
        }
    }
}

/// Index after the JSON value starting at `b[i]`.
fn skip_value(b: &[u8], mut i: usize) -> usize {
    match b.get(i) {
        Some(b'"' | b'\'') => skip_string(b, i),
        Some(b'{' | b'[') => {
            let mut depth = 0;
            while i < b.len() {
                match b[i] {
                    b'"' | b'\'' => {
                        i = skip_string(b, i);
                        continue;
                    }
                    b'/' if matches!(b.get(i + 1), Some(b'/' | b'*')) => {
                        i = skip_ws(b, i);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            b.len()
        }
        _ => {
            while i < b.len() && !matches!(b[i], b',' | b'}' | b']') && !b[i].is_ascii_whitespace()
            {
                i += 1;
            }
            i
        }
    }
}

/// The JSON object of `s`: `(open, close)` brace indices and each member `(key, value span)`.
type Members = (usize, usize, Vec<(String, usize, usize)>);

fn members(s: &str) -> Option<Members> {
    let b = s.as_bytes();
    let open = s.find('{')?;
    let mut i = open + 1;
    let mut out = vec![];
    loop {
        i = skip_ws(b, i);
        let (key, ke) = match b.get(i)? {
            b'}' => return Some((open, i, out)),
            b'"' => {
                let ke = skip_string(b, i);
                (serde_json::from_str(&s[i..ke]).ok()?, ke)
            }
            // JSON5: `'ja'` and bare `ja`.
            b'\'' => {
                let ke = skip_string(b, i);
                (s[i + 1..ke - 1].to_string(), ke)
            }
            _ => {
                let n =
                    s[i..].find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '$')));
                let ke = i + n.filter(|&n| n > 0)?;
                (s[i..ke].to_string(), ke)
            }
        };
        i = ke;
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b':') {
            i += 1;
        }
        let ve = skip_value(b, i);
        out.push((key, i, ve));
        i = ve;
    }
}

/// Leading whitespace of the line holding byte `i`.
fn indent_at(s: &str, i: usize) -> &str {
    let ls = s[..i].rfind('\n').map_or(0, |n| n + 1);
    let line = &s[ls..];
    &line[..line.len() - line.trim_start().len()]
}

/// Lines after the first lose (`add = false`) or gain `pad`.
fn shift(s: &str, pad: &str, add: bool) -> String {
    let mut out = String::new();
    for (n, line) in s.split_inclusive('\n').enumerate() {
        if n == 0 || line.trim().is_empty() {
            out += line;
        } else if add {
            out += pad;
            out += line;
        } else {
            out += line.strip_prefix(pad).unwrap_or(line);
        }
    }
    out
}

/// `cat` merged into the JSON object `obj` (written as found in the file, at indent `pad`).
fn merge_json(obj: &str, pad: &str, cat: &Catalog) -> Option<String> {
    let ded = shift(obj, pad, false);
    if serde_json::from_str::<serde_json::Value>(&ded).is_err() {
        let new = json5::write(Some(&ded), cat);
        let new = new.trim_end();
        return (new != ded.trim_end()).then(|| shift(new, pad, true));
    }
    let fmt = match Catalog::detect_format(&ded) {
        Format::Auto => Format::JsonNested,
        f => f,
    };
    let merged = cat.merge_into(&ded, fmt, &Layout::default())?;
    let same = serde_json::from_str::<serde_json::Value>(&ded).ok()
        == serde_json::from_str::<serde_json::Value>(&merged).ok();
    (!same).then(|| shift(merged.trim_end(), pad, true))
}

fn json_entries(obj: &str, locale: &str) -> Result<Parsed, String> {
    if serde_json::from_str::<serde_json::Value>(obj).is_err() {
        return json5::parse(obj, locale);
    }
    Catalog::parse(obj, locale)
        .map(|c| Parsed {
            entries: c.entries,
            ..Parsed::default()
        })
        .map_err(|e| e.to_string())
}

/// Smallest indent of the non-blank lines.
fn min_indent(s: &str) -> usize {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0)
}

fn dedent(s: &str, n: usize) -> String {
    s.split_inclusive('\n')
        .map(|l| if l.trim().is_empty() { l } else { &l[n..] })
        .collect()
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.split_inclusive('\n')
        .map(|l| {
            if l.trim().is_empty() {
                l.to_string()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect()
}

/// Line range of the top-level `locale:` section of a multi-locale YAML block.
fn yaml_section(lines: &[&str], locale: &str) -> Option<(usize, usize)> {
    let top = |l: &str| !l.starts_with([' ', '\t', '#', '\n', '\r']) && !l.trim().is_empty();
    let a = lines.iter().position(|l| {
        top(l)
            && l.split_once(':')
                .is_some_and(|(k, _)| same_locale(k.trim().trim_matches(['"', '\'']), locale))
    })?;
    let b = (a + 1..lines.len())
        .find(|&n| top(lines[n]))
        .unwrap_or(lines.len());
    Some((a, b))
}

const YAML: &str = "i18n.yaml";

pub fn parse(s: &str, locale: &str) -> Result<Parsed, String> {
    let mut out = Parsed::default();
    for b in blocks(s) {
        let body = &s[b.body.0..b.body.1];
        let mine = b.locale.as_deref().map(|l| same_locale(l, locale));
        let p = match (b.yaml, mine) {
            (_, Some(false)) => continue,
            (false, Some(true)) => json_entries(body, locale)?,
            (false, None) => {
                let Some((_, _, ms)) = members(body) else {
                    continue;
                };
                match ms.iter().find(|m| same_locale(&m.0, locale)) {
                    Some((_, a, z)) => json_entries(&body[*a..*z], locale)?,
                    None => continue,
                }
            }
            (true, Some(true)) => yaml::parse(&dedent(body, min_indent(body)), locale)?,
            (true, None) => {
                let ded = dedent(body, min_indent(body));
                let lines: Vec<&str> = ded.split_inclusive('\n').collect();
                match yaml_section(&lines, locale) {
                    Some((a, z)) => yaml::parse(&lines[a..z].concat(), locale)?,
                    None => continue,
                }
            }
        };
        out.entries.extend(p.entries);
        out.notes.extend(p.notes);
    }
    Ok(out)
}

/// The locales the `<i18n>` blocks of a component hold (`glot init`).
pub fn locales(s: &str) -> Vec<String> {
    let mut out = vec![];
    for b in blocks(s) {
        let body = &s[b.body.0..b.body.1];
        if let Some(l) = b.locale {
            out.push(l);
        } else if b.yaml {
            let ded = dedent(body, min_indent(body));
            out.extend(ded.lines().filter_map(|l| {
                let (k, _) = l.split_once(':')?;
                (!l.starts_with([' ', '\t', '#']) && !k.is_empty())
                    .then(|| k.trim().trim_matches(['"', '\'']).to_string())
            }));
        } else if let Some((_, _, ms)) = members(body) {
            out.extend(ms.into_iter().map(|m| m.0));
        }
    }
    out
}

/// New text of a block body for `cat`, `None` when unchanged.
fn patch(body: &str, b: &Block, cat: &Catalog) -> Option<String> {
    let loc = &cat.locale;
    if b.yaml {
        let m = min_indent(body);
        let ded = dedent(body, m);
        let new = if b.locale.is_some() {
            yaml::write(Some(&ded), cat, Path::new(YAML))
        } else {
            let lines: Vec<&str> = ded.split_inclusive('\n').collect();
            match yaml_section(&lines, loc) {
                Some((a, z)) => {
                    let sec = lines[a..z].concat();
                    let w = yaml::write(Some(&sec), cat, Path::new(YAML));
                    let w = if sec.ends_with('\n') || w.ends_with('\n') {
                        w
                    } else {
                        w + "\n"
                    };
                    [lines[..a].concat(), w, lines[z..].concat()].concat()
                }
                None if cat.entries.is_empty() => return None,
                None => {
                    let w = yaml::write(Some(&format!("{loc}:\n")), cat, Path::new(YAML));
                    let sep = if ded.ends_with('\n') || ded.is_empty() {
                        ""
                    } else {
                        "\n"
                    };
                    format!("{ded}{sep}{w}")
                }
            }
        };
        return (new != ded).then(|| indent(&new, m));
    }
    let (open, close, ms) = members(body)?;
    if b.locale.is_some() {
        let new = merge_json(&body[open..=close], indent_at(body, close), cat)?;
        return Some(format!("{}{new}{}", &body[..open], &body[close + 1..]));
    }
    if let Some((_, a, z)) = ms.iter().find(|m| same_locale(&m.0, loc)) {
        let new = merge_json(&body[*a..*z], indent_at(body, *a), cat)?;
        return Some(format!("{}{new}{}", &body[..*a], &body[*z..]));
    }
    if cat.entries.is_empty() {
        return None;
    }
    // A new locale member after the last one, indented like the others.
    let pad = ms
        .first()
        .map_or("  ".to_string(), |m| indent_at(body, m.1).to_string());
    let obj = merge_json("{}", &pad, cat)?;
    let key = serde_json::to_string(loc).ok()?;
    Some(match ms.last() {
        Some((_, _, z)) => format!("{},\n{pad}{key}: {obj}{}", &body[..*z], &body[*z..]),
        None => format!("{}\n{pad}{key}: {obj}\n{}", &body[..=open], &body[close..]),
    })
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let s = existing.unwrap_or_default();
    let bs = blocks(s);
    let target = bs
        .iter()
        .find(|b| {
            b.locale
                .as_deref()
                .is_some_and(|l| same_locale(l, &cat.locale))
        })
        .or_else(|| bs.iter().find(|b| b.locale.is_none()));
    if let Some(b) = target {
        return match patch(&s[b.body.0..b.body.1], b, cat) {
            Some(new) => format!("{}{new}{}", &s[..b.body.0], &s[b.body.1..]),
            None => s.to_string(),
        };
    }
    if cat.entries.is_empty() {
        return s.to_string();
    }
    // Only other locales' `<i18n locale>` blocks (or none): a new block after the last one.
    let last = bs.last();
    let yaml = last.is_some_and(|b| b.yaml);
    let lang = last
        .and_then(|b| b.lang.as_deref())
        .map(|l| format!(" lang=\"{l}\""))
        .unwrap_or_default();
    let (head, body) = if last.is_some() {
        let body = if yaml {
            yaml::write(Some(""), cat, Path::new(YAML))
        } else {
            merge_json("{}", "", cat).unwrap_or_default() + "\n"
        };
        (format!("<i18n locale=\"{}\"{lang}>", cat.locale), body)
    } else {
        let key = serde_json::to_string(&cat.locale).unwrap_or_default();
        let obj = merge_json("{}", "  ", cat).unwrap_or_default();
        ("<i18n>".to_string(), format!("{{\n  {key}: {obj}\n}}\n"))
    };
    let at = last.map_or(s.len(), |b| b.end);
    let sep = if at == 0 || s[..at].ends_with("\n\n") {
        ""
    } else if s[..at].ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let tail = if last.is_some() { "" } else { "\n" };
    format!("{}{sep}{head}\n{body}</i18n>{tail}{}", &s[..at], &s[at..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(locale: &str, kv: &[(&str, &str)]) -> Catalog {
        Catalog {
            locale: locale.into(),
            entries: kv
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    const JSON: &str = r#"<template>
  <p>{{ t('hello', { name }) }}</p>
</template>

<script setup>
const { t } = useI18n({ useScope: 'local' })
</script>

<i18n>
{
  "en": {
    "hello": "Hello {name}!",
    "nav": { "home": "Home" },
    "cars": "car | cars"
  },
  "ja": {
    "hello": "こんにちは {name}!"
  }
}
</i18n>

<style scoped>
p { color: red; }
</style>
"#;

    #[test]
    fn multi_locale_json_block() {
        let en = parse(JSON, "en").unwrap().entries;
        assert_eq!(en["hello"], "Hello {name}!");
        assert_eq!(en["nav.home"], "Home");
        assert_eq!(en["cars"], "car | cars");
        assert_eq!(parse(JSON, "ja").unwrap().entries.len(), 1);
        assert!(parse(JSON, "fr").unwrap().entries.is_empty());
        assert_eq!(locales(JSON), ["en", "ja"]);
        // Byte-identical round trip, for every locale.
        for l in ["en", "ja"] {
            let c = Catalog {
                locale: l.into(),
                entries: parse(JSON, l).unwrap().entries,
            };
            assert_eq!(write(Some(JSON), &c), JSON);
        }
        // A changed locale: only its object moves, the rest of the file keeps its bytes.
        let ja = cat("ja", &[("hello", "こんにちは {name}!"), ("cars", "車")]);
        let out = write(Some(JSON), &ja);
        assert!(out.contains("  \"ja\": {\n    \"hello\": \"こんにちは {name}!\",\n    \"cars\": \"車\"\n  }\n}\n</i18n>"), "{out}");
        assert_eq!(out.replace(",\n    \"cars\": \"車\"", ""), JSON);
        // A new locale: a member after the last one.
        let fr = cat(
            "fr",
            &[("hello", "Bonjour {name} !"), ("nav.home", "Accueil")],
        );
        let out = write(Some(JSON), &fr);
        assert!(out.contains("  },\n  \"fr\": {\n    \"hello\": \"Bonjour {name} !\",\n    \"nav\": {\n      \"home\": \"Accueil\"\n    }\n  }\n}\n</i18n>\n\n<style scoped>"), "{out}");
        assert_eq!(parse(&out, "fr").unwrap().entries.len(), 2);
        assert_eq!(parse(&out, "en").unwrap().entries, en);
    }

    #[test]
    fn yaml_and_per_locale_blocks() {
        let yaml = "<script setup lang=\"ts\">\nconst { t } = useI18n()\n</script>\n\n<i18n lang=\"yaml\">\nen:\n  # Greeting\n  hello: Hello {name}\n  bye: Bye\nja:\n  hello: こんにちは\n</i18n>\n";
        assert_eq!(parse(yaml, "en").unwrap().entries["bye"], "Bye");
        assert_eq!(parse(yaml, "en").unwrap().notes["hello"], "Greeting");
        let en = Catalog {
            locale: "en".into(),
            entries: parse(yaml, "en").unwrap().entries,
        };
        assert_eq!(write(Some(yaml), &en), yaml);
        let ja = cat("ja", &[("hello", "こんにちは"), ("bye", "さようなら")]);
        assert_eq!(
            write(Some(yaml), &ja),
            yaml.replace(
                "ja:\n  hello: こんにちは\n",
                "ja:\n  hello: こんにちは\n  bye: さようなら\n"
            )
        );
        let fr = cat("fr", &[("hello", "Bonjour {name}")]);
        let out = write(Some(yaml), &fr);
        assert!(
            out.ends_with("ja:\n  hello: こんにちは\nfr:\n  hello: Bonjour {name}\n</i18n>\n"),
            "{out}"
        );
        assert_eq!(
            parse(&out, "fr").unwrap().entries["hello"],
            "Bonjour {name}"
        );

        // One block per locale: a new locale gets its own block, same `lang`.
        let per = "<template><p>{{ t('a') }}</p></template>\n\n<i18n locale=\"en\" lang=\"json\">\n{\n  \"a\": \"A\"\n}\n</i18n>\n";
        assert_eq!(parse(per, "en").unwrap().entries["a"], "A");
        assert!(parse(per, "de").unwrap().entries.is_empty());
        let de = cat("de", &[("a", "Ä")]);
        let out = write(Some(per), &de);
        assert_eq!(
            out,
            format!("{per}\n<i18n locale=\"de\" lang=\"json\">\n{{\n  \"a\": \"Ä\"\n}}\n</i18n>\n")
        );
        assert_eq!(parse(&out, "de").unwrap().entries["a"], "Ä");
        assert_eq!(write(Some(&out), &cat("de", &[("a", "Ä")])), out);
        // JSON5 blocks: comments, single quotes and bare keys survive a patch.
        let j5 = "<i18n lang=\"json5\">\n{\n  // English\n  en: { hi: 'Hi', /* {x} */ bye: 'Bye' },\n  'ja': {\n    hi: 'やあ', // casual\n  },\n}\n</i18n>\n";
        assert_eq!(parse(j5, "en").unwrap().entries["bye"], "Bye");
        assert_eq!(parse(j5, "ja").unwrap().entries["hi"], "やあ");
        let en = Catalog {
            locale: "en".into(),
            entries: parse(j5, "en").unwrap().entries,
        };
        assert_eq!(write(Some(j5), &en), j5);
        let out = write(Some(j5), &cat("ja", &[("hi", "やあ"), ("bye", "じゃね")]));
        assert!(out.contains("// casual"), "{out}");
        assert!(out.starts_with("<i18n lang=\"json5\">\n{\n  // English\n  en: { hi: 'Hi', /* {x} */ bye: 'Bye' },\n"), "{out}");
        assert_eq!(parse(&out, "ja").unwrap().entries["bye"], "じゃね");
        assert_eq!(parse(&out, "en").unwrap().entries, en.entries);
        // `src` blocks are other catalogs.
        assert!(
            parse(
                "<i18n src=\"./en.json\"></i18n>\n<i18n src=\"./x.json\" />\n",
                "en"
            )
            .unwrap()
            .entries
            .is_empty()
        );
    }
}
