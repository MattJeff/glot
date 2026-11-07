//! Markdown / MDX documents as translatable segments (`glot docs`).
//!
//! A document is split into raw parts (kept byte for byte) and segments: paragraphs, list items,
//! headings, table cells, blockquote lines, admonition titles and the front matter values people
//! read (`title`, `description`, `sidebar_label`…). Fenced and indented code, math blocks, HTML
//! comments, MDX `import`/`export` and lines with no words are raw.
//! Inside a segment, what must survive translation is masked as ICU `{0}`, `{1}`…: inline code,
//! `{…}` (MDX expressions, Hugo shortcodes, Liquid, heading ids), HTML/JSX tags, link targets
//! `](url "title")` / `][ref]`, footnote refs and bare URLs. Apostrophes are doubled so the
//! segment is valid ICU and the validator checks every placeholder came back.
//! Rendering with no translation gives the original text back exactly.

/// How a front matter value is written back.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Quote {
    /// Body text.
    Body,
    YamlPlain,
    YamlDouble,
    YamlSingle,
    TomlDouble,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Masked text sent for translation (`{0}`, `{1}`…, `''` for `'`).
    pub text: String,
    /// The protected spans behind `{0}`, `{1}`…
    pub spans: Vec<String>,
    quote: Quote,
    /// Original bytes of the segment (rendered when it has no translation).
    orig: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Raw(String),
    Seg(usize),
    /// Front matter `lang: en`: the value, rewritten to the target locale when it is the source's.
    Lang(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    parts: Vec<Part>,
    segments: Vec<Segment>,
}

const FRONT_MATTER_KEYS: &[&str] = &[
    "title",
    "description",
    "sidebar_label",
    "sidebar_title",
    "summary",
    "subtitle",
    "linkTitle",
    "excerpt",
    "label",
    "tagline",
];
const LANG_KEYS: &[&str] = &["lang", "locale", "language"];

impl Doc {
    pub fn parse(text: &str) -> Self {
        let mut d = Doc::default();
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let body = d.front_matter(&lines);
        d.body(&lines[body..]);
        d
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The document with `tr(i)` (masked text, as produced for `segments()[i]`) in place of each
    /// translated segment; `lang` = (source, target) rewrites a front matter `lang: source`.
    pub fn render(
        &self,
        tr: &dyn Fn(usize) -> Option<String>,
        lang: Option<(&str, &str)>,
    ) -> String {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Raw(s) => out.push_str(s),
                Part::Lang(v) => match lang {
                    Some((src, tgt)) if v == src => out.push_str(tgt),
                    _ => out.push_str(v),
                },
                Part::Seg(i) => {
                    let s = &self.segments[*i];
                    match tr(*i) {
                        Some(t) => out.push_str(&requote(&unmask(&t, &s.spans), s.quote)),
                        None => out.push_str(&s.orig),
                    }
                }
            }
        }
        out
    }

    fn raw(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if let Some(Part::Raw(r)) = self.parts.last_mut() {
            r.push_str(s);
        } else {
            self.parts.push(Part::Raw(s.to_string()));
        }
    }

    /// `prefix` raw, `content` as a segment (raw when it has no words), `suffix` raw.
    fn seg(&mut self, prefix: &str, content: &str, orig: &str, suffix: &str, quote: Quote) {
        self.raw(prefix);
        let (text, spans) = mask(content);
        if words(&text) {
            self.parts.push(Part::Seg(self.segments.len()));
            self.segments.push(Segment {
                text,
                spans,
                quote,
                orig: orig.to_string(),
            });
        } else {
            self.raw(orig);
        }
        self.raw(suffix);
    }

    /// YAML (`---`) or TOML (`+++`) front matter; returns the index of the first body line.
    fn front_matter(&mut self, lines: &[&str]) -> usize {
        let Some(fence) = lines
            .first()
            .map(|l| l.trim_end())
            .filter(|l| *l == "---" || *l == "+++")
        else {
            return 0;
        };
        let Some(end) = lines[1..].iter().position(|l| l.trim_end() == fence) else {
            return 0;
        };
        self.raw(lines[0]);
        for line in &lines[1..=end] {
            self.fm_line(line, fence == "+++");
        }
        self.raw(lines[end + 1]);
        end + 2
    }

    fn fm_line(&mut self, line: &str, toml: bool) {
        let (body, nl) = split_nl(line);
        let sep = if toml { '=' } else { ':' };
        let Some((k, v)) = body.split_once(sep) else {
            return self.raw(line);
        };
        let key = k.trim();
        if k.starts_with(char::is_whitespace)
            || !(FRONT_MATTER_KEYS.contains(&key) || LANG_KEYS.contains(&key))
        {
            return self.raw(line);
        }
        let lead = v.len() - v.trim_start().len();
        let v = v[lead..].trim_end();
        let prefix = &body[..k.len() + 1 + lead];
        let trail = &body[prefix.len() + v.len()..];
        let (inner, quote, q) =
            if let Some(s) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                (
                    s.replace("\\\"", "\"").replace("\\\\", "\\"),
                    if toml {
                        Quote::TomlDouble
                    } else {
                        Quote::YamlDouble
                    },
                    "\"",
                )
            } else if let Some(s) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                (s.replace("''", "'"), Quote::YamlSingle, "'")
            } else if toml || v.is_empty() || v.starts_with(['[', '{', '|', '>', '&', '*', '!']) {
                return self.raw(line);
            } else {
                (v.to_string(), Quote::YamlPlain, "")
            };
        if LANG_KEYS.contains(&key) {
            self.raw(&format!("{prefix}{q}"));
            self.parts.push(Part::Lang(inner));
            return self.raw(&format!("{q}{trail}{nl}"));
        }
        // The quotes are re-chosen on render: the segment covers them.
        self.seg(prefix, &inner, v, &format!("{trail}{nl}"), quote);
    }

    fn body(&mut self, lines: &[&str]) {
        let mut para: Option<Para> = None;
        let mut fence: Option<(char, usize)> = None;
        let mut until: Option<&str> = None;
        let mut esm = false;
        let mut in_list = false;
        let mut blank_before = true;
        for &line in lines {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            let blank = line.trim().is_empty();
            if let Some((c, n)) = fence {
                self.raw(line);
                let t = trimmed.trim_end();
                if t.len() >= n && t.chars().all(|x| x == c) {
                    fence = None;
                }
                continue;
            }
            if let Some(end) = until {
                self.raw(line);
                if line.contains(end) {
                    until = None;
                }
                continue;
            }
            if esm {
                self.raw(line);
                esm = !blank;
                continue;
            }
            let ticks = fence_len(trimmed);
            if ticks.is_some() || trimmed.starts_with("$$") || trimmed.starts_with("<!--") || blank
            {
                self.flush(&mut para);
                self.raw(line);
                if let Some(f) = ticks {
                    fence = Some(f);
                } else if let Some(rest) = trimmed.strip_prefix("$$") {
                    if !rest.contains("$$") {
                        until = Some("$$");
                    }
                } else if trimmed.starts_with("<!--") && !trimmed.contains("-->") {
                    until = Some("-->");
                }
                blank_before = blank;
                continue;
            }
            if link_definition(trimmed) {
                self.flush(&mut para);
                self.raw(line);
                continue;
            }
            if indent == 0 && (trimmed.starts_with("import ") || trimmed.starts_with("export ")) {
                self.flush(&mut para);
                self.raw(line);
                esm = true;
                continue;
            }
            // Indented code: 4+ spaces after a blank line, outside a list.
            if indent >= 4 && para.is_none() && blank_before && !in_list {
                self.raw(line);
                continue;
            }
            blank_before = false;
            let (body, nl) = split_nl(line);
            let (container, content) = container(body);
            // A line of tags / shortcodes only (`<Tabs>`, `{{% notice %}}`) stands alone.
            if !words(&mask(content).0) {
                self.flush(&mut para);
                self.raw(line);
                continue;
            }
            let marker = list_marker(content);
            if let Some(m) = marker {
                in_list = true;
                self.flush(&mut para);
                let prefix = &body[..container.len() + m];
                para = Some(Para::new(prefix, &body[prefix.len()..], line, nl));
                continue;
            }
            let hashes = content.chars().take_while(|c| *c == '#').count();
            let heading = (1..=6).contains(&hashes) && content[hashes..].starts_with([' ', '\t']);
            if heading || content.starts_with('|') || content.starts_with(":::") {
                self.flush(&mut para);
                if content.starts_with('|') {
                    self.raw(container);
                    self.table_row(content);
                    self.raw(nl);
                } else {
                    let mut cut = container.len() + hashes;
                    if !heading {
                        cut += content.chars().take_while(|c| *c == ':').count();
                        cut += body[cut..]
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '-')
                            .count();
                    }
                    let rest = &body[cut..];
                    let lead = rest.len() - rest.trim_start().len();
                    let (p, c) = body.split_at(cut + lead);
                    self.seg(p, c, c, nl, Quote::Body);
                }
                continue;
            }
            if indent == 0 && container.is_empty() && para.is_none() {
                in_list = false;
            }
            match &mut para {
                Some(p) if p.depth == depth(container) => {
                    p.push(content, line, nl);
                }
                _ => {
                    self.flush(&mut para);
                    para = Some(Para::new(&body[..container.len()], content, line, nl));
                }
            }
            if body.ends_with("  ") || body.ends_with('\\') {
                self.flush(&mut para);
            }
        }
        self.flush(&mut para);
    }

    fn flush(&mut self, para: &mut Option<Para>) {
        let Some(p) = para.take() else { return };
        let (text, brk) = match p.text.strip_suffix('\\') {
            Some(t) => (t.to_string(), "\\"),
            None => {
                let t = p.text.trim_end();
                (t.to_string(), &p.text[t.len()..])
            }
        };
        // Untranslated: the original lines (wrapping included); translated: one line.
        let orig = &p.orig[p.prefix.len()..p.orig.len() - p.nl.len()];
        let orig = &orig[..orig.len() - brk.len()];
        let prefix = p.prefix.clone();
        self.seg(&prefix, &text, orig, &format!("{brk}{}", p.nl), Quote::Body);
    }

    fn table_row(&mut self, row: &str) {
        let mut cells = vec![];
        let (mut start, mut code, mut esc) = (0, false, false);
        for (i, c) in row.char_indices() {
            match c {
                _ if esc => esc = false,
                '\\' => esc = true,
                '`' => code = !code,
                '|' if !code => {
                    cells.push(&row[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        cells.push(&row[start..]);
        for (n, cell) in cells.iter().enumerate() {
            if n > 0 {
                self.raw("|");
            }
            let t = cell.trim();
            let lead = cell.len() - cell.trim_start().len();
            self.seg(&cell[..lead], t, t, &cell[lead + t.len()..], Quote::Body);
        }
    }
}

struct Para {
    prefix: String,
    /// Blockquote depth (`>` count): a lazy continuation line joins at the same depth.
    depth: usize,
    text: String,
    orig: String,
    nl: String,
}

impl Para {
    fn new(prefix: &str, content: &str, line: &str, nl: &str) -> Self {
        let (container, _) = container(prefix);
        Para {
            prefix: prefix.to_string(),
            depth: depth(container),
            text: content.to_string(),
            orig: line.to_string(),
            nl: nl.to_string(),
        }
    }

    fn push(&mut self, content: &str, line: &str, nl: &str) {
        self.text = format!("{} {}", self.text.trim_end(), content.trim());
        self.orig.push_str(line);
        self.nl = nl.to_string();
    }
}

/// `(line without its newline, newline)`.
fn split_nl(line: &str) -> (&str, &str) {
    let body = line.trim_end_matches(['\n', '\r']);
    (body, &line[body.len()..])
}

/// Leading indentation and blockquote markers (`  > > `), and the rest.
fn container(line: &str) -> (&str, &str) {
    let mut i = line.len() - line.trim_start().len();
    while line[i..].starts_with('>') {
        i += 1;
        i += line[i..].len() - line[i..].trim_start().len();
    }
    line.split_at(i)
}

fn depth(container: &str) -> usize {
    container.matches('>').count()
}

/// `[id]: url` link reference definition (`[^1]: …` footnotes are text).
fn link_definition(s: &str) -> bool {
    s.starts_with('[')
        && !s.starts_with("[^")
        && s.find("]:").is_some_and(|i| !s[1..i].contains(']'))
}

/// Length of a list marker (`- `, `* `, `+ `, `1. `, `2) `, task `- [ ] `).
fn list_marker(s: &str) -> Option<usize> {
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    let n = if digits > 0 && digits < 10 && s[digits..].starts_with(['.', ')']) {
        digits + 1
    } else if s.starts_with(['-', '*', '+']) {
        1
    } else {
        return None;
    };
    let rest = &s[n..];
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    // `---` / `***` rules have no space after the first char; `- ` followed by nothing is a list.
    let mut n = n + rest.len() - rest.trim_start().len();
    for task in ["[ ] ", "[x] ", "[X] "] {
        if s[n..].starts_with(task) {
            n += task.len();
        }
    }
    Some(n)
}

/// ``` / ~~~ fence: (char, length).
fn fence_len(s: &str) -> Option<(char, usize)> {
    let c = s.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = s.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

/// Has a word outside placeholders.
fn words(masked: &str) -> bool {
    let mut depth = 0;
    masked.chars().any(|c| {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => return depth == 0 && c.is_alphabetic(),
        }
        false
    })
}

/// Index of the char closing `open` at byte `i` (nesting counted), if any.
fn closing(s: &str, i: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (j, c) in s[i..].char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + j);
            }
        }
    }
    None
}

/// Protected spans => `{n}`; `'` => `''`.
pub fn mask(s: &str) -> (String, Vec<String>) {
    let (mut out, mut spans) = (String::new(), Vec::<String>::new());
    let mut push = |out: &mut String, span: &str| {
        let n = spans.iter().position(|x| x == span).unwrap_or_else(|| {
            spans.push(span.to_string());
            spans.len() - 1
        });
        out.push_str(&format!("{{{n}}}"));
    };
    let b = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().expect("in bounds");
        let end = match c {
            '`' => {
                let n = rest.bytes().take_while(|x| *x == b'`').count();
                let fence = &rest[..n];
                rest[n..]
                    .find(fence)
                    .filter(|j| !rest[n + j + n..].starts_with('`'))
                    .map(|j| i + n + j + n)
                    .or(Some(i + n))
            }
            '{' => closing(s, i, '{', '}').map(|j| j + 1).or(Some(i + 1)),
            '}' => Some(i + 1),
            '<' => {
                let next = rest[1..].chars().next();
                if next.is_some_and(|x| x.is_ascii_alphabetic() || x == '/' || x == '!') {
                    rest.find('>').map(|j| i + j + 1).or(Some(i + 1))
                } else {
                    Some(i + 1)
                }
            }
            ']' if rest[1..].starts_with('(') => closing(s, i + 1, '(', ')').map(|j| j + 1),
            ']' if rest[1..].starts_with('[') => closing(s, i + 1, '[', ']').map(|j| j + 1),
            '[' if rest.starts_with("[^") => rest.find(']').map(|j| i + j + 1),
            'h' if (rest.starts_with("https://") || rest.starts_with("http://"))
                && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) =>
            {
                let len = rest
                    .find(|x: char| x.is_whitespace() || x == '<' || x == '>')
                    .unwrap_or(rest.len());
                let url = rest[..len].trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
                Some(i + url.len())
            }
            _ => None,
        };
        match end {
            // `]` stays text: only the link target after it is protected.
            Some(e) if c == ']' => {
                out.push(']');
                push(&mut out, &s[i + 1..e]);
                i = e;
            }
            Some(e) => {
                push(&mut out, &s[i..e]);
                i = e;
            }
            None => {
                if c == '\'' {
                    out.push_str("''");
                } else {
                    out.push(c);
                }
                i += c.len_utf8();
            }
        }
    }
    (out, spans)
}

/// Masked text back to Markdown: `{n}` => span, `''` => `'`.
pub fn unmask(t: &str, spans: &[String]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < t.len() {
        let rest = &t[i..];
        if rest.starts_with("''") {
            out.push('\'');
            i += 2;
            continue;
        }
        if rest.starts_with('{')
            && let Some(j) = rest.find('}')
            && let Ok(n) = rest[1..j].parse::<usize>()
            && let Some(s) = spans.get(n)
        {
            out.push_str(s);
            i += j + 1;
            continue;
        }
        let c = rest.chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

impl Segment {
    /// This segment's text with its spans numbered as in `src` (a translated document read back:
    /// the same code, links and tags in another order). `None` if a span is not in `src`.
    pub fn renumbered(&self, src: &Segment) -> Option<String> {
        let map: Vec<usize> = self
            .spans
            .iter()
            .map(|s| src.spans.iter().position(|x| x == s))
            .collect::<Option<_>>()?;
        let mut out = String::new();
        let mut rest = self.text.as_str();
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let j = rest[i..].find('}').map(|j| i + j)?;
            let n: usize = rest[i + 1..j].parse().ok()?;
            out.push_str(&format!("{{{}}}", map.get(n)?));
            rest = &rest[j + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

fn requote(s: &str, q: Quote) -> String {
    match q {
        Quote::Body => s.to_string(),
        Quote::YamlDouble | Quote::TomlDouble => {
            format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
        }
        Quote::YamlSingle => format!("'{}'", s.replace('\'', "''")),
        Quote::YamlPlain => {
            let needs = s.starts_with([
                '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"',
                '%', '@', '`',
            ]) || s.contains(": ")
                || s.contains(" #")
                || s.ends_with(':')
                || s.trim() != s;
            if needs {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                s.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"---
id: intro
title: Getting started
description: "Install it: in 5 minutes"
sidebar_position: 1
lang: en
---

import Tabs from '@theme/Tabs';

# Welcome to `glot` {#welcome}

glot translates your **docs**, see [the guide](https://example.com/guide "Guide")
and the [API][api]. It's fast.

- First item with `code`
- [ ] Second item
  continued here

1. Step one: run https://example.com/install.sh.

> A quote
> on two lines

:::tip Pro tip
Use <kbd>Ctrl</kbd> + {props.key}.
:::

| Name | Description |
| ---- | ----------- |
| `a`  | The first   |

```js
const x = "not translated";
```

    indented code

<Tabs groupId="x">
{{< figure src="a.png" >}}

Line with hard break\
next line.

[api]: https://example.com/api
"#;

    fn texts(d: &Doc) -> Vec<&str> {
        d.segments().iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn untranslated_render_is_identical() {
        let d = Doc::parse(DOC);
        assert_eq!(
            d.render(&|_| None, Some(("en", "fr")))
                .replace("lang: fr", "lang: en"),
            DOC
        );
        let plain = "Just one line without newline";
        assert_eq!(Doc::parse(plain).render(&|_| None, None), plain);
    }

    #[test]
    fn segments_and_masks() {
        let d = Doc::parse(DOC);
        let t = texts(&d);
        assert_eq!(
            t,
            [
                "Getting started",
                "Install it: in 5 minutes",
                "Welcome to {0} {1}",
                "glot translates your **docs**, see [the guide]{0} and the [API]{1}. It''s fast.",
                "First item with {0}",
                "Second item continued here",
                "Step one: run {0}.",
                "A quote on two lines",
                "Pro tip",
                "Use {0}Ctrl{1} + {2}.",
                "Name",
                "Description",
                "The first",
                "Line with hard break",
                "next line.",
            ]
        );
        assert_eq!(d.segments()[2].spans, ["`glot`", "{#welcome}"]);
        assert_eq!(
            d.segments()[3].spans,
            ["(https://example.com/guide \"Guide\")", "[api]"]
        );
    }

    #[test]
    fn translated_render() {
        let d = Doc::parse(DOC);
        let fr = |i: usize| -> Option<String> {
            Some(match i {
                0 => "Bien démarrer".into(),
                1 => "Installez-le : en 5 minutes".into(),
                2 => "Bienvenue dans {0} {1}".into(),
                3 => "glot traduit vos **docs**, voir [le guide]{0} et l''[API]{1}.".into(),
                5 => "Deuxième élément".into(),
                9 => "Utilisez {2} avec {0}Ctrl{1}.".into(),
                11 => "Description".into(),
                _ => return None,
            })
        };
        let out = d.render(&fr, Some(("en", "fr")));
        assert!(out.contains("title: Bien démarrer\n"), "{out}");
        assert!(
            out.contains("description: \"Installez-le : en 5 minutes\"\n"),
            "{out}"
        );
        assert!(out.contains("lang: fr\n"), "{out}");
        assert!(
            out.contains("# Bienvenue dans `glot` {#welcome}\n"),
            "{out}"
        );
        assert!(
            out.contains("voir [le guide](https://example.com/guide \"Guide\") et l'[API][api].\n"),
            "{out}"
        );
        assert!(out.contains("- [ ] Deuxième élément\n\n"), "{out}");
        assert!(
            out.contains("Utilisez {props.key} avec <kbd>Ctrl</kbd>.\n:::"),
            "{out}"
        );
        assert!(out.contains("const x = \"not translated\";"), "{out}");
        assert!(out.contains("| Name | Description |\n| ---- |"), "{out}");
        // A document read back aligns with its source.
        let back = Doc::parse(&out);
        assert_eq!(back.segments().len(), d.segments().len());
        assert_eq!(
            back.segments()[9].renumbered(&d.segments()[9]).as_deref(),
            Some("Utilisez {2} avec {0}Ctrl{1}.")
        );
    }

    #[test]
    fn toml_front_matter_and_plain_yaml_quoting() {
        let d = Doc::parse("+++\ntitle = \"Hello\"\ndraft = true\n+++\nBody\n");
        assert_eq!(texts(&d), ["Hello", "Body"]);
        let out = d.render(&|i| (i == 0).then(|| "Bonjour \"monde\"".into()), None);
        assert!(
            out.starts_with("+++\ntitle = \"Bonjour \\\"monde\\\"\"\n"),
            "{out}"
        );
        let y = Doc::parse("---\ntitle: Hello\n---\n");
        assert_eq!(
            y.render(&|_| Some("Note : ceci".into()), None),
            "---\ntitle: \"Note : ceci\"\n---\n"
        );
    }
}
