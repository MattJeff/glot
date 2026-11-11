//! glot Edge and SPA frameworks: hydration-safe payloads, the client patcher tag, the "empty shell" heuristic.
//! Same rules as `packages/edge/src/hydration.ts` and `packages/renderer/src/skeleton.ts`; shared fixtures in
//! `crates/core/tests/fixtures/edge/hydration` (real framework pages, byte-identical output on both sides).
//!
//! [`translate_payloads`]: frameworks hydrate from a serialized payload; if the HTML is translated but the payload
//! is not, the client renders the source text over it (React: error #418 + client re-render = flash). Strings of
//! the payload that are exactly the source of a translated segment are translated too:
//! - Next.js Pages `<script id="__NEXT_DATA__" type="application/json">`, Nuxt `<script id="__NUXT_DATA__">`
//!   (devalue array), Next.js App Router flight `self.__next_f.push([1,"…"])` (JSON rows; `T` text rows untouched).
//! - SvelteKit 2 `kit.start(app, element, { data: [JS literal] })`, SolidStart seroval `self.$R[…]` scripts (JS
//!   literals: [`scan`]ned, then every edited span confirmed as a string literal by oxc), React Router 7 / Remix
//!   `streamController.enqueue("<turbo-stream>")` (first line), Astro `<astro-island props="…">` (type-0 values).
//! - Never a flight reference (`$…`), an object key, a computed member name, or the value of an identifier-like
//!   key (`id`, `href`, `type`, `variant`…): app logic comparing such values keeps working. Strings are edited
//!   in place: key order, numbers and the rest of the script stay byte-identical.
//! - Vue pages (Nuxt, Astro islands) get `<html data-allow-mismatch="text">`: Vue still replaces a hard-coded
//!   text it finds translated (the patcher re-translates it in the same task) without logging a hydration error.
//!
//! Text hard-coded in client bundles is not in these payloads: the Edge localizes the served chunks themselves
//! (`glot_extract::bundle`, server `edge/bundle.rs`); what remains (ambiguous texts, no Referer in worker mode)
//! the [`patcher_tag`] script re-applies after the framework re-renders (MutationObserver, same task → no painted
//! flash). See docs/edge.md, "Single-page apps and hydration".
// ponytail: string scanning + serde_json, no HTML parser: these scripts have a fixed shape written by frameworks.
use serde_json::Value;

const ID_KEYS: &[&str] = &[
    "id",
    "_id",
    "key",
    "slug",
    "href",
    "url",
    "src",
    "srcset",
    "as",
    "type",
    "kind",
    "variant",
    "status",
    "state",
    "plan",
    "tier",
    "mode",
    "lang",
    "locale",
    "code",
    "name",
    "classname",
    "class",
    "style",
    "role",
    "rel",
    "target",
    "method",
    "action",
    "icon",
    "color",
    "size",
    "format",
    "currency",
    "path",
    "route",
    "pathname",
    "query",
    "hash",
];
/// Identifier-like key (`id`, `type`, `variant`…): its string value is app logic, never translated.
pub fn id_key(k: &str) -> bool {
    ID_KEYS.iter().any(|x| x.eq_ignore_ascii_case(k))
}
fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// JSON made safe inside `<script>` (Next's htmlEscapeJsonString).
fn esc(json: &str) -> String {
    let mut o = String::with_capacity(json.len());
    for c in json.chars() {
        match c {
            '<' => o.push_str("\\u003c"),
            '>' => o.push_str("\\u003e"),
            '&' => o.push_str("\\u0026"),
            '\u{2028}' => o.push_str("\\u2028"),
            '\u{2029}' => o.push_str("\\u2029"),
            c => o.push(c),
        }
    }
    o
}

type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn tr(s: &str, lookup: Lookup) -> Option<String> {
    if s.starts_with('$') || !s.chars().any(char::is_alphabetic) {
        return None;
    }
    lookup(&fold(s)).filter(|t| t != s)
}

/// Byte range of the body of the `<script …id="{id}"…>` element.
fn script_body(html: &str, id: &str) -> Option<std::ops::Range<usize>> {
    let at = html.find(&format!("id=\"{id}\""))?;
    let open = html[..at].rfind("<script")?;
    let start = open + html[open..].find('>')? + 1;
    let end = start + html[start..].find("</script>")?;
    (at < start).then_some(start..end)
}

// ------------------------------------------------------------ data-literal scanner (= hydration.ts `scan`)

/// A string value of a data literal: `a..b` = the literal with its quotes (byte offsets), `s` = decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Lit {
    pub a: usize,
    pub b: usize,
    pub s: String,
    /// Key the value belongs to (inherited through arrays: `title: [0, "…"]`).
    pub key: Option<String>,
    /// Index when directly in a top-level array.
    pub top: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum K {
    Obj,
    Arr,
    Paren,
    Member,
}
struct Frame {
    k: K,
    key: Option<String>,
    pend: Option<String>,
    i: usize,
}

fn hex_at(src: &str, j: usize, n: usize) -> Option<u32> {
    let h = src.get(j..j + n)?;
    if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

/// JS (and JSON) string literal at `src[i]` → (decoded, end) or `None` (unterminated, raw line break).
fn read_str(src: &str, i: usize, to: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let q = b[i];
    let mut s = String::new();
    let mut j = i + 1;
    while j < to {
        let c = src[j..].chars().next()?;
        if c as u32 == q as u32 {
            return Some((s, j + 1));
        }
        if c == '\n' || c == '\r' {
            return None;
        }
        if c != '\\' {
            s.push(c);
            j += c.len_utf8();
            continue;
        }
        let e = src.get(j + 1..)?.chars().next()?;
        j += 1 + e.len_utf8();
        match e {
            'n' => s.push('\n'),
            'r' => s.push('\r'),
            't' => s.push('\t'),
            'b' => s.push('\u{8}'),
            'f' => s.push('\u{c}'),
            'v' => s.push('\u{b}'),
            '0' => {
                if b.get(j).is_some_and(u8::is_ascii_digit) {
                    return None;
                }
                s.push('\0');
            }
            'x' => {
                s.push(char::from_u32(hex_at(src, j, 2)?)?);
                j += 2;
            }
            'u' if b.get(j) == Some(&b'{') => {
                let k = j + src[j..].find('}')?;
                let h = &src[j + 1..k];
                if k - j > 7 || h.is_empty() || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return None;
                }
                s.push(char::from_u32(u32::from_str_radix(h, 16).ok()?)?);
                j = k + 1;
            }
            'u' => {
                let mut cp = hex_at(src, j, 4)?;
                j += 4;
                if (0xd800..0xdc00).contains(&cp) && src.get(j..j + 2) == Some("\\u") {
                    // surrogate pair (JS strings are UTF-16)
                    let lo = hex_at(src, j + 2, 4)?;
                    if (0xdc00..0xe000).contains(&lo) {
                        cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                        j += 6;
                    }
                }
                s.push(char::from_u32(cp)?); // lone surrogate: not representable, leave the script as is
            }
            '\r' => {
                if b.get(j) == Some(&b'\n') {
                    j += 1;
                }
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            '1'..='9' => return None, // legacy octal: never in generated data
            e => s.push(e),
        }
    }
    None
}

/// String values of a JSON text or a JS *data literal* (objects, arrays, calls, arrows, identifiers, numbers) in
/// `src[from..to]`. `one`: `src[from]` is a bracket, stop when it closes. Object keys, computed member names
/// (`x["k"]`) and strings outside any bracket are not returned. `None` = something a generated data literal
/// never contains (template literal, slash = regex/comment, `?`, unbalanced brackets): leave that script as is.
pub fn scan(src: &str, from: usize, to: usize, one: bool) -> Option<Vec<Lit>> {
    let b = src.as_bytes();
    let mut out = vec![];
    let mut st: Vec<Frame> = vec![];
    let mut prev = ' ';
    let next = |mut j: usize| {
        while j < to {
            let c = src[j..].chars().next().unwrap_or(' ');
            if !c.is_whitespace() {
                return Some(c);
            }
            j += c.len_utf8();
        }
        None
    };
    let id0 = |c: char| c.is_ascii_alphabetic() || c == '_' || c == '$';
    let idc = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut i = from;
    while i < to {
        let c = src[i..].chars().next()?;
        let depth = st.len();
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if c == '"' || c == '\'' {
            let (s, e) = read_str(src, i, to)?;
            match st.last_mut() {
                Some(f) if f.k == K::Obj && next(e) == Some(':') => f.pend = Some(s),
                Some(f) if f.k != K::Member => {
                    let key = match f.k {
                        K::Obj => f.pend.take(),
                        K::Arr => f.key.clone(),
                        _ => None,
                    };
                    let top = (depth == 1 && f.k == K::Arr).then_some(f.i);
                    out.push(Lit {
                        a: i,
                        b: e,
                        s,
                        key,
                        top,
                    });
                }
                _ => {}
            }
            prev = 'v';
            i = e;
            continue;
        }
        if id0(c) || c.is_ascii_digit() {
            let mut j = i + 1;
            while j < to && (idc(b[j]) || (!id0(c) && b[j] == b'.')) {
                j += 1;
            }
            if id0(c)
                && let Some(f) = st.last_mut()
                && f.k == K::Obj
                && next(j) == Some(':')
            {
                f.pend = Some(src[i..j].to_string());
            }
            prev = 'v';
            i = j;
            continue;
        }
        match c {
            '`' | '/' | '?' => return None,
            '{' | '(' => st.push(Frame {
                k: if c == '{' { K::Obj } else { K::Paren },
                key: None,
                pend: None,
                i: 0,
            }),
            '[' if prev == 'v' || prev == ')' => st.push(Frame {
                k: K::Member,
                key: None,
                pend: None,
                i: 0,
            }),
            '[' => {
                let key = match st.last_mut() {
                    Some(f) if f.k == K::Obj => f.pend.take(),
                    Some(f) if f.k == K::Arr => f.key.clone(),
                    _ => None,
                };
                st.push(Frame {
                    k: K::Arr,
                    key,
                    pend: None,
                    i: 0,
                });
            }
            '}' | ']' | ')' => {
                let open = st.pop()?.k;
                let ok = match c {
                    '}' => open == K::Obj,
                    ')' => open == K::Paren,
                    _ => open == K::Arr || open == K::Member,
                };
                if !ok {
                    return None;
                }
                if one && st.is_empty() {
                    return Some(out);
                }
                prev = ')';
                i += 1;
                continue;
            }
            ',' => {
                if let Some(f) = st.last_mut() {
                    f.i += 1;
                    if f.k == K::Obj {
                        f.pend = None;
                    }
                }
            }
            _ => {}
        }
        prev = c;
        i += c.len_utf8();
    }
    (!one && st.is_empty()).then_some(out)
}

type Edit = (usize, usize, String);

fn splice(s: &str, edits: &[Edit]) -> String {
    let mut o = String::with_capacity(s.len() + edits.len() * 16);
    let mut at = 0;
    for (a, b, v) in edits {
        o.push_str(&s[at..*a]);
        o.push_str(v);
        at = *b;
    }
    o.push_str(&s[at..]);
    o
}

fn js_str(v: &str) -> String {
    Value::String(v.to_string()).to_string()
}

/// Edits for the translatable values of `lits`; `skip(l)` = never this one. `enc`: how a value is written.
fn edits(
    lits: &[Lit],
    lookup: Lookup,
    skip: impl Fn(&Lit) -> bool,
    enc: impl Fn(&str) -> String,
) -> Vec<Edit> {
    lits.iter()
        .filter(|l| !l.key.as_deref().is_some_and(id_key) && !skip(l))
        .filter_map(|l| tr(&l.s, lookup).map(|t| (l.a, l.b, enc(&t))))
        .collect()
}
fn script_enc(v: &str) -> String {
    esc(&js_str(v))
}

/// Each inline `<script>` body (no `src`) of `html` through `f` (new body or `None`).
fn scripts(html: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(html.len());
    let (mut at, mut from) = (0, 0);
    while let Some(p) = html[from..].find("<script").map(|p| from + p) {
        from = p + 7;
        if html
            .as_bytes()
            .get(p + 7)
            .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
        {
            continue;
        }
        let Some(gt) = html[p..].find('>').map(|g| p + g) else {
            break;
        };
        let Some(end) = html[gt + 1..].find("</script>").map(|e| gt + 1 + e) else {
            break;
        };
        from = end + 9;
        if has_src(&html[p..gt]) {
            continue;
        }
        if let Some(b) = f(&html[gt + 1..end]) {
            out.push_str(&html[at..gt + 1]);
            out.push_str(&b);
            at = end;
        }
    }
    out.push_str(&html[at..]);
    out
}
/// `/\ssrc\s*=/` on an open tag.
fn has_src(tag: &str) -> bool {
    tag.match_indices("src").any(|(i, _)| {
        tag[..i]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
            && tag[i + 3..].trim_start().starts_with('=')
    })
}

/// `edits` applied when every edited span is a string literal of the script for oxc too (a JS data literal is
/// real JavaScript: the parser confirms the scanner never took a comment, regex or template part for a string).
fn apply_js(body: &str, e: Vec<Edit>, n: &mut usize) -> Option<String> {
    use oxc_ast_visit::Visit;
    struct Spans(std::collections::HashSet<(usize, usize)>);
    impl<'a> Visit<'a> for Spans {
        fn visit_string_literal(&mut self, it: &oxc_ast::ast::StringLiteral<'a>) {
            self.0
                .insert((it.span.start as usize, it.span.end as usize));
        }
    }
    if e.is_empty() {
        return None;
    }
    let alloc = oxc_allocator::Allocator::default();
    let parsed = [oxc_span::SourceType::mjs(), oxc_span::SourceType::cjs()]
        .into_iter()
        .map(|t| oxc_parser::Parser::new(&alloc, body, t).parse())
        .find(|p| !p.diagnostics.has_errors())?;
    let mut v = Spans(Default::default());
    v.visit_program(&parsed.program);
    if !e.iter().all(|(a, b, _)| v.0.contains(&(*a, *b))) {
        return None;
    }
    *n += e.len();
    Some(splice(body, &e))
}
fn apply(body: &str, e: Vec<Edit>, n: &mut usize) -> Option<String> {
    if e.is_empty() {
        return None;
    }
    *n += e.len();
    Some(splice(body, &e))
}

/// Index-addressed flat arrays (devalue, turbo-stream): the top-level entries never translated.
fn flat_skip(a: &[Value], turbo: bool) -> std::collections::HashSet<usize> {
    let mut skip = std::collections::HashSet::new();
    let idx = |k: &str| {
        k.strip_prefix('_')
            .filter(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
            .and_then(|d| d.parse::<usize>().ok())
    };
    for x in a {
        match x {
            Value::Object(m) => {
                for (k, i) in m {
                    // devalue: {"id": 7}; turbo-stream: {"_3": 7} where entry 3 is the key "id" (never text)
                    let key = if turbo {
                        idx(k).and_then(|j| {
                            skip.insert(j);
                            a.get(j)?.as_str()
                        })
                    } else {
                        Some(k.as_str())
                    };
                    if let (Some(key), Some(i)) = (key, i.as_u64())
                        && id_key(key)
                    {
                        skip.insert(i as usize);
                    }
                }
            }
            // ["Reactive", 3]: the leading string is a type tag
            Value::Array(t) if !turbo && t.first().is_some_and(Value::is_string) => {
                skip.extend(
                    t.iter()
                        .skip(1)
                        .filter_map(Value::as_u64)
                        .map(|i| i as usize),
                );
            }
            _ => {}
        }
    }
    skip
}
fn flat(
    json: &str,
    lookup: Lookup,
    turbo: bool,
    enc: impl Fn(&str) -> String,
) -> Option<Vec<Edit>> {
    let Ok(Value::Array(a)) = serde_json::from_str::<Value>(json) else {
        return None;
    };
    let lits = scan(json, 0, json.len(), false)?;
    let skip = flat_skip(&a, turbo);
    Some(edits(
        &lits,
        lookup,
        |l| l.top.is_none_or(|t| skip.contains(&t)),
        enc,
    ))
}

fn json_script(html: &str, id: &str, f: impl FnOnce(&str) -> Option<String>) -> Option<String> {
    let r = script_body(html, id)?;
    let b = f(&html[r.clone()])?;
    let mut out = html.to_string();
    out.replace_range(r, &b);
    Some(out)
}

/// React Router 7 / Remix: `…streamController.enqueue("<turbo-stream>")`; its first line is the flat array.
fn react_router(body: &str, lookup: Lookup, n: &mut usize) -> Option<String> {
    const CALL: &str = "streamController.enqueue(";
    let mut out = String::new();
    let (mut at, mut from) = (0, 0);
    while let Some(p) = body[from..].find(CALL).map(|p| from + p + CALL.len()) {
        from = p;
        let bs = body.as_bytes();
        if bs.get(p) != Some(&b'"') {
            continue;
        }
        let mut e = p + 1;
        while e < bs.len() && bs[e] != b'"' && bs[e] != b'\n' {
            e += if bs[e] == b'\\' { 2 } else { 1 };
        }
        if bs.get(e) != Some(&b'"') || bs.get(e + 1) != Some(&b')') {
            continue;
        }
        from = e + 1;
        let Ok(stream) = serde_json::from_str::<String>(&body[p..=e]) else {
            continue;
        };
        let line = &stream[..stream.find('\n').unwrap_or(stream.len())];
        if !line.starts_with('[') {
            continue; // P/E rows (deferred promises): left to the patcher
        }
        let Some(ed) = flat(line, lookup, true, js_str).filter(|e| !e.is_empty()) else {
            continue;
        };
        *n += ed.len();
        out.push_str(&body[at..p]);
        out.push_str(&esc(&js_str(&(splice(line, &ed) + &stream[line.len()..]))));
        at = e + 1;
    }
    (at > 0).then(|| out + &body[at..])
}

/// Attribute value as the browser reads it; `None` for a named entity outside the five basic ones.
fn unattr(v: &str) -> Option<String> {
    let mut s = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(p) = rest.find('&') {
        s.push_str(&rest[..p]);
        rest = &rest[p..];
        let body = &rest[1..];
        let len = if let Some(h) = body.strip_prefix("#x") {
            h.bytes().take_while(u8::is_ascii_hexdigit).count() + 2
        } else if let Some(d) = body.strip_prefix('#') {
            d.bytes().take_while(u8::is_ascii_digit).count() + 1
        } else {
            body.bytes().take_while(u8::is_ascii_alphabetic).count()
        };
        let ent = &body[..len];
        if body.as_bytes().get(len) != Some(&b';')
            || ent
                .trim_start_matches("#x")
                .trim_start_matches('#')
                .is_empty()
        {
            s.push('&'); // not an entity reference: as written
            rest = body;
            continue;
        }
        let ch = match ent {
            "quot" => '"',
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "apos" => '\'',
            e if e.starts_with("#x") => char::from_u32(u32::from_str_radix(&e[2..], 16).ok()?)?,
            e if e.starts_with('#') => char::from_u32(e[1..].parse().ok()?)?,
            _ => return None,
        };
        s.push(ch);
        rest = &body[len + 1..];
    }
    s.push_str(rest);
    Some(s)
}
fn reattr(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}
/// `before` ends with `[0,` (Astro's "plain value" tag), whitespace allowed.
fn plain(before: &str) -> bool {
    before
        .trim_end()
        .strip_suffix(',')
        .and_then(|t| t.trim_end().strip_suffix('0'))
        .is_some_and(|t| t.trim_end().ends_with('['))
}

/// Astro islands: `props` is JSON of `[type, value]` pairs; only type-0 (plain) strings are text.
fn astro(html: &str, lookup: Lookup, n: &mut usize) -> String {
    const TAG: &str = "<astro-island";
    let mut out = String::with_capacity(html.len());
    let (mut at, mut from) = (0, 0);
    while let Some(p) = html[from..].find(TAG).map(|p| from + p) {
        from = p + TAG.len();
        if html
            .as_bytes()
            .get(from)
            .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
        {
            continue;
        }
        let Some(gt) = html[p..].find('>').map(|g| p + g) else {
            break;
        };
        from = gt;
        let tag = &html[p..gt];
        let Some(v0) = tag
            .match_indices("props=\"")
            .find(|(i, _)| {
                tag[..*i]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            })
            .map(|(i, m)| p + i + m.len())
        else {
            continue;
        };
        let Some(v1) = html[v0..gt].find('"').map(|e| v0 + e) else {
            continue;
        };
        let Some(json) = unattr(&html[v0..v1]) else {
            continue;
        };
        let Some(lits) = scan(&json, 0, json.len(), false) else {
            continue;
        };
        let e = edits(&lits, lookup, |l| !plain(&json[..l.a]), js_str);
        if e.is_empty() {
            continue;
        }
        *n += e.len();
        out.push_str(&html[at..v0]);
        out.push_str(&reattr(&splice(&json, &e)));
        at = v1;
    }
    out.push_str(&html[at..]);
    out
}

/// Vue (Nuxt, Astro islands): hard-coded text Vue finds translated is replaced without a console error.
fn vue_allow_mismatch(html: String) -> String {
    let vue = html.contains("__NUXT_DATA__")
        || html.contains("id=\"__nuxt\"")
        || html.match_indices("<astro-island").any(|(i, m)| {
            !html
                .as_bytes()
                .get(i + m.len())
                .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
        });
    if !vue {
        return html;
    }
    let lower = html.to_ascii_lowercase();
    let Some(h) = lower.match_indices("<html").map(|(h, _)| h).find(|&h| {
        lower
            .as_bytes()
            .get(h + 5)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b'>')
    }) else {
        return html;
    };
    let Some(end) = lower[h..].find('>').map(|e| h + e) else {
        return html;
    };
    let tag = &lower[h..end];
    if tag.match_indices("data-allow-mismatch").any(|(i, m)| {
        tag[..i]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
            && !tag
                .as_bytes()
                .get(i + m.len())
                .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
    }) {
        return html;
    }
    format!(
        "{} data-allow-mismatch=\"text\"{}",
        &html[..h + 5],
        &html[h + 5..]
    )
}

/// Edits `(start, end, json string)` on the concatenated flight stream (byte offsets).
fn flight_edits(stream: &str, lookup: Lookup) -> Vec<(usize, usize, String)> {
    let b = stream.as_bytes();
    let mut edits = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\n' {
            i += 1;
            continue;
        }
        let id_end = i + b[i..]
            .iter()
            .take(24)
            .position(|&c| c == b':')
            .unwrap_or(usize::MAX - i);
        if id_end >= b.len() || !b[i..id_end].iter().all(u8::is_ascii_hexdigit) {
            break; // not a row start: stop rather than guess
        }
        let j = id_end + 1;
        if b.get(j) == Some(&b'T') {
            let Some(comma) = stream[j..].find(',').map(|c| j + c) else {
                break;
            };
            let Ok(len) = usize::from_str_radix(&stream[j + 1..comma], 16) else {
                break;
            };
            i = comma + 1 + len; // T rows (long text, length-delimited) stay untranslated
            continue;
        }
        let end = stream[j..].find('\n').map_or(b.len(), |e| j + e);
        if matches!(b.get(j), Some(b'[' | b'{')) {
            let mut k = j;
            let mut key = String::new();
            while k < end {
                if b[k] != b'"' {
                    k += 1;
                    continue;
                }
                let mut e = k + 1;
                while e < end && b[e] != b'"' {
                    e += if b[e] == b'\\' { 2 } else { 1 };
                }
                if e >= end {
                    break;
                }
                let tok = &stream[k..=e];
                let rest = stream[e + 1..end].trim_start();
                let s: String = serde_json::from_str(tok).unwrap_or_default();
                if rest.starts_with(':') {
                    key = s;
                } else {
                    if !id_key(&key)
                        && let Some(t) = tr(&s, lookup)
                    {
                        edits.push((k, e + 1, Value::String(t).to_string()));
                    }
                    key.clear();
                }
                k = e + 1;
            }
        }
        i = end + 1;
    }
    edits
}

const PUSH: &str = "self.__next_f.push(";

fn flight(html: &str, lookup: Lookup, n: &mut usize) -> Option<String> {
    // (range of the `[1,"…"]` literal in html, decoded chunk)
    let mut pushes: Vec<(std::ops::Range<usize>, String)> = vec![];
    let mut at = 0;
    while let Some(p) = html[at..].find(PUSH) {
        let s = at + p + PUSH.len();
        let mut it = serde_json::Deserializer::from_str(&html[s..]).into_iter::<Value>();
        let v = it.next();
        let e = s + it.byte_offset();
        at = e;
        if let Some(Ok(Value::Array(a))) = v
            && a.first().and_then(Value::as_u64) == Some(1)
            && let Some(Value::String(c)) = a.get(1)
        {
            pushes.push((s..e, c.clone()));
        }
    }
    if pushes.is_empty() {
        return None;
    }
    let mut starts = vec![];
    let mut stream = String::new();
    for (_, c) in &pushes {
        starts.push(stream.len());
        stream.push_str(c);
    }
    let mut out: Vec<String> = pushes.iter().map(|(_, c)| c.clone()).collect();
    for (a, b, v) in flight_edits(&stream, lookup).into_iter().rev() {
        let k = starts.iter().rposition(|&s| s <= a)?;
        if b > starts[k] + pushes[k].1.len() {
            continue; // straddles two pushes
        }
        out[k].replace_range(a - starts[k]..b - starts[k], &v);
        *n += 1;
    }
    let mut html = html.to_string();
    for (k, (r, c)) in pushes.iter().enumerate().rev() {
        if &out[k] != c {
            html.replace_range(r.clone(), &esc(&serde_json::json!([1, out[k]]).to_string()));
        }
    }
    Some(html)
}

/// Translate the framework payloads of `html` with the page's lookup (normalized source text → translation).
/// Returns the new HTML and how many strings changed.
pub fn translate_payloads(html: &str, lookup: Lookup) -> (String, usize) {
    let mut n = 0;
    let mut out = html.to_string();
    if out.contains("__NEXT_DATA__")
        && let Some(h) = json_script(&out, "__NEXT_DATA__", |body| {
            let l = scan(body, 0, body.len(), false)?;
            apply(body, edits(&l, lookup, |_| false, script_enc), &mut n)
        })
    {
        out = h;
    }
    if out.contains("__NUXT_DATA__")
        && let Some(h) = json_script(&out, "__NUXT_DATA__", |body| {
            apply(body, flat(body, lookup, false, script_enc)?, &mut n)
        })
    {
        out = h;
    }
    if out.contains(PUSH)
        && let Some(h) = flight(&out, lookup, &mut n)
    {
        out = h;
    }
    // SvelteKit: the arguments of `kit.start(app, element, { node_ids, data, … })` (devalue.uneval JS literal)
    if out.contains("kit.start(") {
        out = scripts(&out, |body| {
            let at = body.find("kit.start(")? + "kit.start".len();
            let l = scan(body, at, body.len(), true)?;
            apply_js(body, edits(&l, lookup, |_| false, script_enc), &mut n)
        });
    }
    // SolidStart: seroval's `self.$R[…]` scripts (data in object/array literals and resolver calls)
    if out.contains("$R[") {
        out = scripts(&out, |body| {
            if !body.contains("$R[") {
                return None;
            }
            let l = scan(body, 0, body.len(), false)?;
            apply_js(body, edits(&l, lookup, |_| false, script_enc), &mut n)
        });
    }
    if out.contains("streamController.enqueue(") {
        out = scripts(&out, |body| react_router(body, lookup, &mut n));
    }
    if out.contains("<astro-island") {
        out = astro(&out, lookup, &mut n);
    }
    (vue_allow_mismatch(out), n)
}

/// `<script async>` loading the client patcher (`/widget/edge-patcher.global.js`, ~9 kB gz, never render-blocking),
/// placed right after the page's `<meta name="glot-tm">` (or `<head>`). Text the app renders before it runs is
/// caught up when its TM arrives; an inline `<script type="application/json" id="glot-tm">` TM removes that window.
/// `missing`: the runtime's discovery endpoint (`/__glot/missing`) for text the app renders on the client only.
pub fn patcher_tag(
    script_url: &str,
    tm_url: &str,
    source_locale: &str,
    missing: Option<&str>,
) -> String {
    let a = |s: &str| {
        s.replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    };
    let missing = missing
        .map(|m| format!(" data-missing=\"{}\"", a(m)))
        .unwrap_or_default();
    format!(
        "<script async src=\"{}\" data-tm=\"{}\" data-source=\"{}\"{missing}></script>",
        a(script_url),
        a(tm_url),
        a(source_locale)
    )
}

/// Largest TM inlined in the page (bytes of JSON); a larger one is fetched by the patcher (`data-tm`).
pub const TM_INLINE_MAX: usize = 32_768;

/// `<script type="application/json" id="glot-tm">` carrying the page's TM when its JSON is at most
/// [`TM_INLINE_MAX`] bytes: the patcher translates text the app renders right after load (after-mount,
/// client-only) without waiting for a request, so a cold visit paints no source text. Empty when larger; the
/// serialization stops at the limit (bounded cost for a big TM). Same as `tmInline` in hydration.ts.
pub fn tm_inline(tm: &impl serde::Serialize) -> String {
    struct Capped(Vec<u8>);
    impl std::io::Write for Capped {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.0.len() + b.len() > TM_INLINE_MAX {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            self.0.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut w = Capped(vec![]);
    match serde_json::to_writer(&mut w, tm).map(|_| String::from_utf8(w.0)) {
        Ok(Ok(json)) => format!(
            r#"<script type="application/json" id="glot-tm">{}</script>"#,
            esc(&json)
        ),
        _ => String::new(),
    }
}

/// Insert `tag` after `<meta name="glot-tm" …>` if present, else right after `<head>`; unchanged without `<head>`.
pub fn inject_head(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let at = lower
        .find("<meta name=\"glot-tm\"")
        .and_then(|m| lower[m..].find('>').map(|e| m + e + 1))
        .or_else(|| {
            let (h, _) = lower.match_indices("<head").find(|(h, _)| {
                matches!(
                    lower.as_bytes().get(h + 5),
                    Some(b'>' | b' ' | b'\t' | b'\n' | b'\r')
                )
            })?;
            lower[h..].find('>').map(|e| h + e + 1)
        });
    match at {
        Some(i) => format!("{}{tag}{}", &html[..i], &html[i..]),
        None => html.to_string(),
    }
}

/// Minimum visible text of a real page; below it (or without any h1–h3/p/li) the HTML is an empty SPA shell and
/// `rendering: auto` uses the renderer's snapshot.
pub const MIN_TEXT: usize = 200;

/// The `rendering: auto` heuristic (same as `packages/renderer/src/skeleton.ts`). One pass over the bytes, no
/// allocation: comments and raw/opaque elements (script, style, template, noscript, svg) are skipped, tags count
/// as a space, visible text is measured with whitespace folded.
pub fn is_skeleton(html: &str) -> bool {
    let b = html.as_bytes();
    let (start, end) = match (find_ci(b, 0, b"<body"), rfind_ci(b, b"</body>")) {
        (Some(s), Some(e)) if s < e => (
            b[s..e]
                .iter()
                .position(|&c| c == b'>')
                .map_or(s, |g| s + g + 1),
            e,
        ),
        _ => (0, b.len()),
    };
    let word_end = |i: usize| !b.get(i).is_some_and(u8::is_ascii_alphanumeric);
    let (mut has_content, mut text, mut space) = (false, 0usize, false);
    let mut i = start;
    while i < end {
        if b[i] == b'<' {
            space = true;
            if starts_ci(b, i, b"<!--") {
                i = find_ci(&b[..end], i + 4, b"-->").map_or(end, |e| e + 3);
                continue;
            }
            if let Some(t) = [&b"script"[..], b"style", b"template", b"noscript", b"svg"]
                .into_iter()
                .find(|t| starts_ci(b, i + 1, t) && word_end(i + 1 + t.len()))
            {
                // `</tag>` (case-insensitive), searched without building the string
                let mut j = i + 1 + t.len();
                i = loop {
                    match find_ci(&b[..end], j, b"</") {
                        Some(k)
                            if starts_ci(b, k + 2, t) && b.get(k + 2 + t.len()) == Some(&b'>') =>
                        {
                            break k + 3 + t.len();
                        }
                        Some(k) => j = k + 2,
                        None => break end,
                    }
                };
                continue;
            }
            has_content = has_content
                || [&b"h1"[..], b"h2", b"h3", b"p", b"li"]
                    .into_iter()
                    .any(|t| starts_ci(b, i + 1, t) && word_end(i + 1 + t.len()));
            i = b[i..end]
                .iter()
                .position(|&c| c == b'>')
                .map_or(end, |g| i + g + 1);
            continue;
        }
        let c = b[i];
        if c.is_ascii_whitespace() {
            space = true;
        } else if c & 0xC0 != 0x80 {
            // one char (UTF-8 lead byte), plus the folded space before it
            text += 1 + usize::from(space && text > 0);
            space = false;
            if has_content && text >= MIN_TEXT {
                return false;
            }
        }
        i += 1;
    }
    text < MIN_TEXT || !has_content
}

fn starts_ci(b: &[u8], i: usize, pat: &[u8]) -> bool {
    b.get(i..i + pat.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(pat))
}

fn find_ci(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    (from..b.len().saturating_sub(pat.len() - 1)).find(|&i| starts_ci(b, i, pat))
}

fn rfind_ci(b: &[u8], pat: &[u8]) -> Option<usize> {
    (0..b.len().saturating_sub(pat.len() - 1))
        .rev()
        .find(|&i| starts_ci(b, i, pat))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn es(s: &str) -> Option<String> {
        match s {
            "Simple pricing for teams" => Some("Precios simples para equipos".into()),
            "Add to cart" => Some("Añadir al carrito".into()),
            "Pro" => Some("Profesional".into()),
            "Long story" => Some("Historia <larga>".into()),
            _ => None,
        }
    }

    #[test]
    fn next_data_values_not_ids() {
        let h = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"title":"Simple pricing for teams","plan":"Pro","list":["Add to cart"]}},"page":"/p"}</script>"#;
        let (o, n) = translate_payloads(h, &es);
        assert_eq!(n, 2);
        assert!(o.contains(r#""title":"Precios simples para equipos""#));
        assert!(o.contains(r#""plan":"Pro""#), "identifier-like key kept");
        assert!(o.contains(r#"["Añadir al carrito"]"#));
    }

    #[test]
    fn nuxt_devalue() {
        let h = r#"<script type="application/json" id="__NUXT_DATA__" data-ssr="true">[["ShallowReactive",1],{"data":2,"state":4},{"title":3,"variant":5},"Simple pricing for teams",{},"Pro"]</script>"#;
        let (o, n) = translate_payloads(h, &es);
        assert_eq!(n, 1);
        assert!(o.contains(r#""Precios simples para equipos""#) && o.contains(r#","Pro"]"#));
    }

    #[test]
    fn flight_rows_and_escaping() {
        let chunk = "1:\"$Sreact.fragment\"\n7:[\"$\",\"h1\",null,{\"children\":\"Simple pricing for teams\"}]\n8:T6,Add to\n9:[\"$\",\"$L9\",null,{\"label\":\"Long story\",\"href\":\"Add to cart\"}]\n";
        let lit = serde_json::json!([1, chunk]).to_string();
        let h = format!(
            "<script>self.__next_f.push([0])</script><script>self.__next_f.push({lit})</script>"
        );
        let (o, n) = translate_payloads(&h, &es);
        assert_eq!(n, 2, "{o}");
        assert!(o.contains("Precios simples para equipos"));
        assert!(
            o.contains(r#"Historia \\u003clarga\\u003e"#)
                || o.contains("Historia \\u003clarga\\u003e"),
            "{o}"
        );
        assert!(o.contains("8:T6,Add to"), "T row untouched");
        assert!(o.contains(r#"\"href\":\"Add to cart\""#), "href kept: {o}");
        // still parseable, and the decoded stream carries the translation
        let s = o.find("push([1").unwrap() + 5;
        let v: Value = serde_json::Deserializer::from_str(&o[s..])
            .into_iter::<Value>()
            .next()
            .unwrap()
            .unwrap();
        assert!(
            v[1].as_str()
                .unwrap()
                .contains("{\"children\":\"Precios simples para equipos\"}")
        );
    }

    #[test]
    fn patcher_tag_after_version_meta() {
        let t = patcher_tag(
            "https://g/widget/edge-patcher.global.js",
            "https://g/edge/v1/p/es/tm?a=1&b=2",
            "fr",
            None,
        );
        assert!(t.contains("a=1&amp;b=2"));
        assert!(
            patcher_tag("/p.js", "/tm.json", "fr", Some("/__glot/missing"))
                .ends_with(r#"data-source="fr" data-missing="/__glot/missing"></script>"#)
        );
        let h = inject_head(
            r#"<html><head><meta name="glot-tm" content="es@3"><title>x</title></head></html>"#,
            &t,
        );
        assert!(h.contains(r#"content="es@3"><script async src="#));
        assert_eq!(inject_head("<p>no head</p>", &t), "<p>no head</p>");
        assert!(
            inject_head("<html><header>x</header><head></head>", "<s>")
                .contains("<head><s></head>")
        );
    }

    #[test]
    fn inline_tm_capped() {
        let t = tm_inline(&serde_json::json!({"t": {"a": "</script>"}}));
        assert_eq!(
            t,
            r#"<script type="application/json" id="glot-tm">{"t":{"a":"\u003c/script\u003e"}}</script>"#
        );
        assert_eq!(
            tm_inline(&"é".repeat(TM_INLINE_MAX / 2)),
            "",
            "bytes, not characters"
        );
    }

    #[test]
    fn skeleton() {
        assert!(is_skeleton(
            r#"<html><body><div id="root"></div><script>render()</script></body></html>"#
        ));
        let real = format!(
            "<body><h1>Hi</h1><p>{}</p><script>{}</script></body>",
            "word ".repeat(60),
            "x".repeat(5000)
        );
        assert!(!is_skeleton(&real));
        assert!(is_skeleton(&format!(
            "<body><div>{}</div></body>",
            "word ".repeat(60)
        )));
        assert!(is_skeleton(&format!(
            "<body><h1>x</h1><!-- {} --></body>",
            "word ".repeat(60)
        )));
        assert!(is_skeleton("<BODY><H1>x</H1><SCRIPT>a</b></SCRIPT></BODY>"));
        assert!(!is_skeleton(&format!(
            "<BODY><SCRIPT>{}</SCRIPT><H2>Ñandú</H2><P>{}</P></BODY>",
            "</scriptx>".repeat(9),
            "é ".repeat(120)
        )));
        // linear: a 1 MB page in well under the old per-byte-allocating time
        let big = format!(
            "<body><div>{}</div></body>",
            "<span>a</span> ".repeat(70_000)
        );
        let t = std::time::Instant::now();
        assert!(is_skeleton(&big));
        assert!(
            t.elapsed() < std::time::Duration::from_millis(200),
            "{:?}",
            t.elapsed()
        );
    }
}
