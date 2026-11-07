//! glot Edge and SPA frameworks: hydration-safe payloads, the client patcher tag, the "empty shell" heuristic.
//! Same rules as `packages/edge/src/hydration.ts` and `packages/renderer/src/skeleton.ts`.
//!
//! [`translate_payloads`]: frameworks hydrate from a serialized payload; if the HTML is translated but the payload
//! is not, the client renders the source text over it (React: error #418 + client re-render = flash). Strings of
//! the payload that are exactly the source of a translated segment are translated too:
//! - Next.js Pages `<script id="__NEXT_DATA__" type="application/json">`, Nuxt `<script id="__NUXT_DATA__">`
//!   (devalue array), Next.js App Router flight `self.__next_f.push([1,"…"])` (JSON rows; `T` text rows untouched).
//! - Never a flight reference (`$…`), an object key, or the value of an identifier-like key (`id`, `href`, `type`,
//!   `variant`…): app logic comparing such values keeps working.
//!
//! Text hard-coded in client bundles cannot be reached here: the [`patcher_tag`] script re-applies the TM after
//! the framework re-renders (MutationObserver, same task → no painted flash). See docs/edge.md, "Frameworks".
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
fn id_key(k: &str) -> bool {
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

fn walk(v: &mut Value, lookup: Lookup, key: &str, n: &mut usize) {
    match v {
        Value::String(s) if !id_key(key) => {
            if let Some(t) = tr(s, lookup) {
                *s = t;
                *n += 1;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| walk(x, lookup, key, n)),
        Value::Object(m) => m.iter_mut().for_each(|(k, x)| walk(x, lookup, k, n)),
        _ => {}
    }
}

/// Byte range of the body of the `<script …id="{id}"…>` element.
fn script_body(html: &str, id: &str) -> Option<std::ops::Range<usize>> {
    let at = html.find(&format!("id=\"{id}\""))?;
    let open = html[..at].rfind("<script")?;
    let start = open + html[open..].find('>')? + 1;
    let end = start + html[start..].find("</script>")?;
    (at < start).then_some(start..end)
}

fn json_script(html: &mut String, id: &str, f: impl FnOnce(&mut Value) -> bool) {
    let Some(r) = script_body(html, id) else {
        return;
    };
    let Ok(mut v) = serde_json::from_str::<Value>(&html[r.clone()]) else {
        return;
    };
    if f(&mut v) {
        html.replace_range(r, &esc(&v.to_string()));
    }
}

/// devalue's flat array: values are top-level entries, objects/arrays hold indexes.
fn nuxt(v: &mut Value, lookup: Lookup, n: &mut usize) -> bool {
    let Value::Array(a) = v else { return false };
    let mut skip = std::collections::HashSet::new();
    for x in a.iter() {
        match x {
            Value::Object(m) => {
                for (k, i) in m {
                    if let (true, Some(i)) = (id_key(k), i.as_u64()) {
                        skip.insert(i as usize);
                    }
                }
            }
            // ["Reactive", 3]: the leading string is a type tag
            Value::Array(t) if t.first().is_some_and(Value::is_string) => {
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
    let before = *n;
    for (i, x) in a.iter_mut().enumerate() {
        if let (false, Value::String(s)) = (skip.contains(&i), &mut *x)
            && let Some(t) = tr(s, lookup)
        {
            *s = t;
            *n += 1;
        }
    }
    *n > before
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
    if out.contains("__NEXT_DATA__") {
        json_script(&mut out, "__NEXT_DATA__", |v| {
            let b = n;
            walk(v, lookup, "", &mut n);
            n > b
        });
    }
    if out.contains("__NUXT_DATA__") {
        json_script(&mut out, "__NUXT_DATA__", |v| nuxt(v, lookup, &mut n));
    }
    if out.contains(PUSH)
        && let Some(h) = flight(&out, lookup, &mut n)
    {
        out = h;
    }
    (out, n)
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
