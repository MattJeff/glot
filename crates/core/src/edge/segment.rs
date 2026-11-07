//! glot Edge segmentation: rendered HTML → translatable segments, and back.
//!
//! A segment is the text run of a block element (inline children as numbered placeholders: `<0>…</0>` pairs,
//! `<1/>` for voids, icons, code and user content) or a translatable attribute. Rules, identical in
//! `packages/edge/src/segment.ts` (shared fixtures in `crates/core/tests/fixtures/edge`):
//! 1. Runs end at block boundaries; an inline tag whose partner is outside the run (`<a><div>…</div></a>`)
//!    is a boundary too.
//! 2. Edges are trimmed: whitespace, comments, void/opaque placeholders (`<svg/> Save` → `Save`).
//! 3. A run fully wrapped by one inline element is unwrapped (`<a>Pricing</a>` → `Pricing`, role `a`).
//! 4. A run without loose text is split into one segment per element when they are all links/buttons/labels
//!    (`<a>Home</a> <a>Pricing</a>`) or glued without whitespace (`<span>€19</span><span>/mo</span>`).
//! 5. Text is entity-decoded, values masked as `{n}` ([`protect::mask`]), HTML whitespace collapsed, trimmed.
//!    No letter left (only numbers, placeholders or do-not-translate terms) → no segment.
//! 6. Attributes: `title alt aria-label aria-description placeholder`, `value` of `input[type=button|submit|reset]`,
//!    `content` of `meta[name|property=description|og:title|og:description|twitter:title|twitter:description]`.
//!
//! Hash = [`tm::text_hash`]`(source locale, text)`; context role = carrying tag or ARIA role (`button`, `a`,
//! `h1`, `title`), `tag:attr` for attributes (`img:alt`, `meta:og:title`).
use super::protect::{self, Selector, Skip, Value};
use super::tm;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct SegmentOpts {
    pub source_locale: String,
    /// Project selectors whose subtree is never segmented (`.user-name`, `#account`).
    pub skip_selectors: Vec<Selector>,
    /// Do-not-translate glossary terms: a segment made only of them is not a segment.
    pub dnt: Vec<String>,
    /// Personal-looking values that are public (company email/phone): masked but discoverable.
    pub pii_allow: Vec<String>,
}

impl Default for SegmentOpts {
    fn default() -> Self {
        Self::new("en")
    }
}

impl SegmentOpts {
    pub fn new(source_locale: &str) -> Self {
        Self {
            source_locale: source_locale.into(),
            skip_selectors: Vec::new(),
            dnt: Vec::new(),
            pii_allow: Vec::new(),
        }
    }

    /// Adds selectors; invalid ones (combinators) are ignored.
    pub fn skip(mut self, selectors: &[&str]) -> Self {
        self.skip_selectors
            .extend(selectors.iter().filter_map(|s| Selector::parse(s)));
        self
    }
}

/// Where the segment sits: the `role` half of [`tm::context`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentCtx {
    pub role: String,
}

/// Inline element behind `<k>…</k>` / `<k/>`: source byte ranges, re-emitted verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placeholder {
    Pair {
        name: String,
        open: Range<usize>,
        close: Range<usize>,
    },
    Void {
        name: String,
        range: Range<usize>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub hash: String,
    /// Normalized text with placeholders: `Read our <0>terms</0>, {0} %`.
    pub text: String,
    #[serde(flatten)]
    pub ctx: SegmentCtx,
    /// `data-glot-key` / `data-glot` of the carrying element: the catalog key wins over the TM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// False when personal data was detected: never reported to glot.
    pub discoverable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
    #[serde(skip)]
    pub tags: Vec<Placeholder>,
    /// Source bytes replaced on translation (the whole `name=value` for an attribute).
    #[serde(skip)]
    pub span: Range<usize>,
    #[serde(skip)]
    pub attr: Option<String>,
}

// ---------------------------------------------------------------------------------------------- tokenizer

struct Attr {
    name: String,
    value: String,
    range: Range<usize>,
}

enum Tok {
    Start {
        name: String,
        attrs: Vec<Attr>,
        self_closing: bool,
        range: Range<usize>,
    },
    End {
        name: String,
        range: Range<usize>,
    },
    Text(Range<usize>),
    Comment(Range<usize>),
}

const RAW_TEXT: &[&str] = &[
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes",
];

fn tokenize(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let lower = s.to_ascii_lowercase();
    let n = b.len();
    let mut toks = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    let find = |from: usize, pat: &str| s[from..].find(pat).map(|j| from + j);
    let name_end = |from: usize| {
        (from..n)
            .find(|&j| b[j].is_ascii_whitespace() || b[j] == b'/' || b[j] == b'>')
            .unwrap_or(n)
    };
    macro_rules! flush_text {
        () => {
            if text_start < i {
                toks.push(Tok::Text(text_start..i));
            }
        };
    }
    while i < n {
        if b[i] != b'<' || i + 1 >= n {
            i += 1;
            continue;
        }
        let c = b[i + 1];
        if s[i..].starts_with("<!--") {
            flush_text!();
            let end = find(i + 4, "-->").map_or(n, |j| j + 3);
            toks.push(Tok::Comment(i..end));
            i = end;
            text_start = i;
        } else if c == b'!' || c == b'?' {
            flush_text!();
            let end = find(i, ">").map_or(n, |j| j + 1);
            toks.push(Tok::Comment(i..end));
            i = end;
            text_start = i;
        } else if c == b'/' && i + 2 < n && b[i + 2].is_ascii_alphabetic() {
            flush_text!();
            let ne = name_end(i + 2);
            let end = find(ne, ">").map_or(n, |j| j + 1);
            toks.push(Tok::End {
                name: lower[i + 2..ne].to_string(),
                range: i..end,
            });
            i = end;
            text_start = i;
        } else if c.is_ascii_alphabetic() {
            flush_text!();
            let ne = name_end(i + 1);
            let name = lower[i + 1..ne].to_string();
            let mut j = ne;
            let mut attrs = Vec::new();
            let mut self_closing = false;
            loop {
                while j < n && (b[j].is_ascii_whitespace() || b[j] == b'/') {
                    self_closing = b[j] == b'/';
                    j += 1;
                }
                if j >= n || b[j] == b'>' {
                    break;
                }
                self_closing = false;
                let ns = j;
                j += 1;
                while j < n && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/') {
                    j += 1;
                }
                let aname = lower[ns..j].to_string();
                let mut k = j;
                while k < n && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                let mut value = String::new();
                if k < n && b[k] == b'=' {
                    k += 1;
                    while k < n && b[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k < n && (b[k] == b'"' || b[k] == b'\'') {
                        let q = b[k];
                        let vs = k + 1;
                        let ve = (vs..n).find(|&x| b[x] == q).unwrap_or(n);
                        value = s[vs..ve].to_string();
                        j = (ve + 1).min(n);
                    } else {
                        let vs = k;
                        while k < n && !b[k].is_ascii_whitespace() && b[k] != b'>' {
                            k += 1;
                        }
                        value = s[vs..k].to_string();
                        j = k;
                    }
                }
                attrs.push(Attr {
                    name: aname,
                    value: protect::decode_entities(&value).into_owned(),
                    range: ns..j,
                });
            }
            let end = (j + 1).min(n);
            toks.push(Tok::Start {
                name: name.clone(),
                attrs,
                self_closing,
                range: i..end,
            });
            i = end;
            text_start = i;
            if RAW_TEXT.contains(&name.as_str()) && !self_closing {
                let pat = format!("</{name}");
                let mut from = i;
                let close = loop {
                    match lower[from..].find(&pat) {
                        None => break n,
                        Some(x) => {
                            let at = from + x;
                            let after = at + pat.len();
                            if after >= n
                                || b[after].is_ascii_whitespace()
                                || matches!(b[after], b'/' | b'>')
                            {
                                break at;
                            }
                            from = after;
                        }
                    }
                };
                if i < close {
                    toks.push(Tok::Text(i..close));
                }
                i = close;
                text_start = i;
            }
        } else {
            i += 1;
        }
    }
    flush_text!();
    toks
}

// ---------------------------------------------------------------------------------------------- segmenter

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Phrasing elements: placeholders inside a run. Everything else (and unknown/custom elements) is a block.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "big", "br", "button", "cite", "code", "data", "del", "dfn",
    "em", "font", "i", "img", "input", "ins", "kbd", "label", "link", "mark", "meta", "meter",
    "output", "picture", "progress", "q", "ruby", "rp", "rt", "s", "samp", "select", "small",
    "span", "strike", "strong", "sub", "sup", "textarea", "time", "tt", "u", "var", "wbr", "svg",
    "math", "video", "audio", "canvas", "iframe", "object", "embed",
];

/// No text inside, but their attributes (`alt`, `title`) are translated.
const MEDIA: &[&str] = &[
    "img", "picture", "video", "audio", "canvas", "iframe", "object", "embed",
];

/// Inline elements that carry meaning: unwrapping them changes the segment role.
const ROLE_INLINE: &[&str] = &["a", "button", "label"];

const META_NAMES: &[&str] = &[
    "description",
    "og:title",
    "og:description",
    "twitter:title",
    "twitter:description",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Normal,
    /// Media: no text collected, attributes translated.
    Opaque,
    /// Nothing collected.
    Skip,
}

enum Kind {
    Block {
        role: String,
        key: Option<String>,
        has_child_block: bool,
    },
    Inline {
        id: usize,
    },
    /// Muted inline element in a run: a `<k/>` placeholder whose end is set on close.
    VoidEl {
        item: usize,
    },
    Muted,
}

struct Frame {
    name: String,
    level: Level,
    kind: Kind,
}

#[derive(Clone)]
enum Item {
    Text {
        range: Range<usize>,
        text: String,
    },
    Open {
        id: usize,
        name: String,
        role: Option<String>,
        key: Option<String>,
        range: Range<usize>,
    },
    Close {
        id: usize,
        range: Range<usize>,
    },
    Void {
        name: String,
        range: Range<usize>,
    },
    Comment(Range<usize>),
}

impl Item {
    fn range(&self) -> Range<usize> {
        match self {
            Item::Text { range, .. }
            | Item::Open { range, .. }
            | Item::Close { range, .. }
            | Item::Void { range, .. }
            | Item::Comment(range) => range.clone(),
        }
    }
    fn trimmable(&self) -> bool {
        match self {
            Item::Text { text, .. } => text.chars().all(protect::is_html_ws),
            Item::Void { .. } | Item::Comment(_) => true,
            _ => false,
        }
    }
}

fn get<'a>(attrs: &'a [(String, String)], n: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str())
}

fn role_of(tag: &str, attrs: &[(String, String)]) -> String {
    get(attrs, "role")
        .map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| tag.to_string())
}

fn key_of(attrs: &[(String, String)]) -> Option<String> {
    get(attrs, "data-glot-key")
        .or_else(|| get(attrs, "data-glot"))
        .map(str::to_string)
}

fn implied_close(open: &str, new: &str) -> bool {
    const P_CLOSERS: &[&str] = &[
        "address",
        "article",
        "aside",
        "blockquote",
        "details",
        "dialog",
        "div",
        "dl",
        "dd",
        "dt",
        "fieldset",
        "figcaption",
        "figure",
        "footer",
        "form",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hgroup",
        "hr",
        "li",
        "main",
        "menu",
        "nav",
        "ol",
        "p",
        "pre",
        "section",
        "table",
        "ul",
    ];
    match open {
        "p" => P_CLOSERS.contains(&new),
        "li" => new == "li",
        "dt" | "dd" => matches!(new, "dt" | "dd"),
        "td" | "th" => matches!(new, "td" | "th" | "tr" | "tbody" | "thead" | "tfoot"),
        "tr" => matches!(new, "tr" | "tbody" | "thead" | "tfoot"),
        "thead" | "tbody" | "tfoot" => matches!(new, "tbody" | "thead" | "tfoot"),
        _ => false,
    }
}

struct Segmenter<'a> {
    opts: &'a SegmentOpts,
    stack: Vec<Frame>,
    run: Vec<Item>,
    next_id: usize,
    out: Vec<Segment>,
}

impl<'a> Segmenter<'a> {
    fn level(&self) -> Level {
        self.stack.last().map_or(Level::Normal, |f| f.level)
    }

    fn start(&mut self, name: String, attrs: &[Attr], self_closing: bool, range: Range<usize>) {
        let kv: Vec<(String, String)> = attrs
            .iter()
            .map(|a| (a.name.clone(), a.value.clone()))
            .collect();
        if self.level() == Level::Normal {
            // `<p>a<div>` closes the p, `<li>a<li>` the first li.
            while let Some(i) = self
                .stack
                .iter()
                .rposition(|f| matches!(f.kind, Kind::Block { .. }))
            {
                let only_inline_above = self.stack[i + 1..]
                    .iter()
                    .all(|f| matches!(f.kind, Kind::Inline { .. }));
                if only_inline_above && implied_close(&self.stack[i].name, &name) {
                    self.close_to(i, range.start..range.start);
                } else {
                    break;
                }
            }
        }
        let cur = self.level();
        let skip = if cur == Level::Skip {
            None
        } else {
            protect::skip(&name, &kv, &self.opts.skip_selectors)
        };
        if cur != Level::Skip && !matches!(skip, Some(Skip::Code | Skip::Opaque)) {
            self.attr_segments(&name, attrs, &kv);
        }
        let void = VOID.contains(&name.as_str())
            || (self_closing && (cur == Level::Skip || matches!(name.as_str(), "svg" | "math")));
        let own = match skip {
            Some(_) => Level::Skip,
            None if MEDIA.contains(&name.as_str()) => Level::Opaque,
            None => Level::Normal,
        };
        let level = cur.max(own);
        let inline = INLINE.contains(&name.as_str());
        let kind = if cur != Level::Normal {
            Kind::Muted
        } else if level == Level::Normal && inline {
            if void {
                self.run.push(Item::Void { name, range });
                return;
            }
            let id = self.next_id;
            self.next_id += 1;
            self.run.push(Item::Open {
                id,
                name: name.clone(),
                role: (ROLE_INLINE.contains(&name.as_str()) || get(&kv, "role").is_some())
                    .then(|| role_of(&name, &kv)),
                key: key_of(&kv),
                range,
            });
            Kind::Inline { id }
        } else if inline {
            self.run.push(Item::Void {
                name: name.clone(),
                range,
            });
            Kind::VoidEl {
                item: self.run.len() - 1,
            }
        } else {
            if let Some(Frame {
                kind: Kind::Block {
                    has_child_block, ..
                },
                ..
            }) = self
                .stack
                .iter_mut()
                .rev()
                .find(|f| matches!(f.kind, Kind::Block { .. }))
            {
                *has_child_block = true;
            }
            self.flush();
            if level == Level::Normal {
                Kind::Block {
                    role: role_of(&name, &kv),
                    key: key_of(&kv),
                    has_child_block: false,
                }
            } else {
                Kind::Muted
            }
        };
        if !void {
            self.stack.push(Frame { name, level, kind });
        }
    }

    fn end(&mut self, name: &str, range: Range<usize>) {
        if let Some(i) = self.stack.iter().rposition(|f| f.name == name) {
            self.close_to(i, range);
        }
    }

    /// Close frames down to `i` included; implicitly closed ones end where the closer starts.
    fn close_to(&mut self, i: usize, range: Range<usize>) {
        while self.stack.len() > i {
            let r = if self.stack.len() == i + 1 {
                range.clone()
            } else {
                range.start..range.start
            };
            match self.stack.last().map(|f| &f.kind) {
                Some(Kind::Inline { id }) => {
                    let id = *id;
                    self.run.push(Item::Close { id, range: r });
                }
                Some(Kind::VoidEl { item }) => {
                    let item = *item;
                    if let Some(Item::Void { range, .. }) = self.run.get_mut(item) {
                        range.end = r.end;
                    }
                }
                Some(Kind::Block { .. }) => self.flush(),
                _ => {}
            }
            self.stack.pop();
        }
    }

    fn attr_segments(&mut self, tag: &str, attrs: &[Attr], kv: &[(String, String)]) {
        for a in attrs {
            let role = match a.name.as_str() {
                "title" | "alt" | "aria-label" | "aria-description" | "placeholder" => {
                    format!("{tag}:{}", a.name)
                }
                "value"
                    if tag == "input"
                        && get(kv, "type").is_some_and(|t| {
                            matches!(
                                t.to_ascii_lowercase().as_str(),
                                "button" | "submit" | "reset"
                            )
                        }) =>
                {
                    "input:value".into()
                }
                "content" if tag == "meta" => {
                    match get(kv, "name").or_else(|| get(kv, "property")) {
                        Some(n) if META_NAMES.contains(&n.to_ascii_lowercase().as_str()) => {
                            format!("meta:{}", n.to_ascii_lowercase())
                        }
                        _ => continue,
                    }
                }
                _ => continue,
            };
            let mut values = Vec::new();
            let text =
                protect::collapse_ws(&protect::mask(&a.value, &mut values, &self.opts.pii_allow));
            if let Some(mut seg) = self.make(text, values, &a.value, role, None) {
                seg.span = a.range.clone();
                seg.attr = Some(a.name.clone());
                self.out.push(seg);
            }
        }
    }

    fn make(
        &self,
        text: String,
        values: Vec<Value>,
        raw: &str,
        role: String,
        key: Option<String>,
    ) -> Option<Segment> {
        let mut plain = strip_placeholders(&text);
        for t in &self.opts.dnt {
            if !t.is_empty() {
                plain = plain.replace(t.as_str(), " ");
            }
        }
        if !protect::has_letter(&plain) {
            return None;
        }
        let discoverable =
            !values.iter().any(|v| v.kind.is_pii()) && !protect::looks_like_address(raw);
        Some(Segment {
            hash: tm::text_hash(&self.opts.source_locale, &text),
            text,
            ctx: SegmentCtx { role },
            key,
            discoverable,
            values,
            ..Default::default()
        })
    }

    fn carrier(&self) -> (String, Option<String>) {
        self.stack
            .iter()
            .rev()
            .find_map(|f| match &f.kind {
                Kind::Block {
                    role,
                    key,
                    has_child_block,
                } => Some((role.clone(), key.clone().filter(|_| !has_child_block))),
                _ => None,
            })
            .unwrap_or_else(|| ("body".into(), None))
    }

    fn flush(&mut self) {
        if self.run.is_empty() {
            return;
        }
        // Empty inline elements (`<i class="icon"></i>`) are voids: trimmed at edges, `<k/>` inside.
        let mut items: Vec<Item> = Vec::new();
        for it in std::mem::take(&mut self.run) {
            if let Item::Close { id, range } = &it {
                let ws_between = matches!(items.last(), Some(Item::Text { text, .. }) if text.chars().all(protect::is_html_ws));
                let at = items.len() - usize::from(ws_between);
                if let Some(Item::Open {
                    id: o,
                    name,
                    range: open,
                    ..
                }) = at.checked_sub(1).map(|i| &items[i])
                    && o == id
                {
                    let void = Item::Void {
                        name: name.clone(),
                        range: open.start..range.end,
                    };
                    items.truncate(at - 1);
                    items.push(void);
                    continue;
                }
            }
            items.push(it);
        }
        let (role, key) = self.carrier();
        // Inline tags whose partner is outside the run are boundaries.
        let mut opened = HashMap::new();
        let mut matched = std::collections::HashSet::new();
        for it in &items {
            match it {
                Item::Open { id, .. } => {
                    opened.insert(*id, ());
                }
                Item::Close { id, .. } if opened.contains_key(id) => {
                    matched.insert(*id);
                }
                _ => {}
            }
        }
        let mut start = 0;
        for (i, it) in items.iter().enumerate() {
            let boundary = match it {
                Item::Open { id, .. } | Item::Close { id, .. } => !matched.contains(id),
                _ => false,
            };
            if boundary {
                self.piece(&items[start..i], role.clone(), key.clone());
                start = i + 1;
            }
        }
        self.piece(&items[start..], role, key);
    }

    fn piece(&mut self, mut s: &[Item], mut role: String, mut key: Option<String>) {
        loop {
            while s.first().is_some_and(Item::trimmable) {
                s = &s[1..];
            }
            while s.last().is_some_and(Item::trimmable) {
                s = &s[..s.len() - 1];
            }
            let wrapped = match (s.first(), s.last()) {
                (
                    Some(Item::Open {
                        id,
                        role: r,
                        key: k,
                        ..
                    }),
                    Some(Item::Close { id: c, .. }),
                ) if id == c && s.len() >= 2 => Some((r.clone(), k.clone())),
                _ => None,
            };
            let Some((r, k)) = wrapped else { break };
            if let Some(r) = r {
                role = r;
            }
            if k.is_some() {
                key = k;
            }
            s = &s[1..s.len() - 1];
        }
        if s.is_empty() {
            return;
        }
        let mut depth = 0usize;
        let mut loose = false;
        let mut spaced = false;
        let mut all_roles = true;
        let mut groups = Vec::new();
        let mut gstart = 0;
        for (i, it) in s.iter().enumerate() {
            match it {
                Item::Open { role, .. } => {
                    if depth == 0 {
                        gstart = i;
                        all_roles &= role.is_some();
                    }
                    depth += 1;
                }
                Item::Close { .. } => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        groups.push(gstart..i + 1);
                    }
                }
                Item::Text { text, .. } if depth == 0 => {
                    let ws = text.chars().all(protect::is_html_ws);
                    loose |= !ws;
                    spaced |= ws;
                }
                _ => {}
            }
        }
        // No loose text: links/buttons side by side, or elements glued without a space, are separate items;
        // `<b>Deeply</b> <u>nested</u>` is one sentence.
        if !loose && (all_roles || !spaced) {
            for g in groups {
                self.piece(&s[g], role.clone(), None);
            }
            return;
        }
        self.emit(s, role, key);
    }

    fn emit(&mut self, s: &[Item], role: String, key: Option<String>) {
        let mut text = String::new();
        let mut raw = String::new();
        let mut values = Vec::new();
        let mut tags: Vec<Placeholder> = Vec::new();
        let mut ids = HashMap::new();
        for it in s {
            match it {
                Item::Text { text: t, .. } => {
                    raw.push_str(t);
                    text.push_str(&protect::mask(t, &mut values, &self.opts.pii_allow));
                }
                Item::Open {
                    id, name, range, ..
                } => {
                    ids.insert(*id, tags.len());
                    text.push_str(&format!("<{}>", tags.len()));
                    tags.push(Placeholder::Pair {
                        name: name.clone(),
                        open: range.clone(),
                        close: 0..0,
                    });
                }
                Item::Close { id, range } => {
                    let k = ids[id];
                    if let Placeholder::Pair { close, .. } = &mut tags[k] {
                        *close = range.clone();
                    }
                    text.push_str(&format!("</{k}>"));
                }
                Item::Void { name, range } => {
                    text.push_str(&format!("<{}/>", tags.len()));
                    tags.push(Placeholder::Void {
                        name: name.clone(),
                        range: range.clone(),
                    });
                }
                Item::Comment(_) => {}
            }
            raw.push(' ');
        }
        let text = protect::collapse_ws(&text);
        if let Some(mut seg) = self.make(text, values, &raw, role, key) {
            seg.tags = tags;
            seg.span = s[0].range().start..s[s.len() - 1].range().end;
            self.out.push(seg);
        }
    }
}

/// Text without `<k>`, `</k>`, `<k/>`, `{k}` placeholders.
fn strip_placeholders(t: &str) -> String {
    parse_template(t, usize::MAX, usize::MAX)
        .into_iter()
        .filter_map(|p| match p {
            Part::Text(x) => Some(x),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, PartialEq)]
enum Part<'a> {
    Text(&'a str),
    Open(usize),
    Close(usize),
    Void(usize),
    Val(usize),
}

/// Split a template into text and placeholders; indices out of range (`< ntags`, `< nvalues`) stay text.
fn parse_template(t: &str, ntags: usize, nvalues: usize) -> Vec<Part<'_>> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut text = 0;
    let mut i = 0;
    while i < b.len() {
        let hit = match b[i] {
            b'<' => {
                let close = b.get(i + 1) == Some(&b'/');
                let ds = i + 1 + usize::from(close);
                let de = (ds..b.len())
                    .find(|&j| !b[j].is_ascii_digit())
                    .unwrap_or(b.len());
                let n = t[ds..de]
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n < ntags && de > ds);
                match (n, &t[de..]) {
                    (Some(n), r) if !close && r.starts_with("/>") => Some((Part::Void(n), de + 2)),
                    (Some(n), r) if r.starts_with('>') => {
                        Some((if close { Part::Close(n) } else { Part::Open(n) }, de + 1))
                    }
                    _ => None,
                }
            }
            b'{' => {
                let de = (i + 1..b.len())
                    .find(|&j| !b[j].is_ascii_digit())
                    .unwrap_or(b.len());
                t[i + 1..de]
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n < nvalues && b.get(de) == Some(&b'}'))
                    .map(|n| (Part::Val(n), de + 1))
            }
            _ => None,
        };
        match hit {
            Some((p, end)) => {
                if text < i {
                    out.push(Part::Text(&t[text..i]));
                }
                out.push(p);
                i = end;
                text = i;
            }
            None => i += 1,
        }
    }
    if text < b.len() {
        out.push(Part::Text(&t[text..]));
    }
    out
}

/// All segments of `html`, in document order (by span start).
pub fn segment(html: &str, opts: &SegmentOpts) -> Vec<Segment> {
    let mut sg = Segmenter {
        opts,
        stack: Vec::new(),
        run: Vec::new(),
        next_id: 0,
        out: Vec::new(),
    };
    for t in tokenize(html) {
        match t {
            Tok::Start {
                name,
                attrs,
                self_closing,
                range,
            } => sg.start(name, &attrs, self_closing, range),
            Tok::End { name, range } => sg.end(&name, range),
            Tok::Text(r) => {
                if sg.level() == Level::Normal {
                    let text = protect::decode_entities(&html[r.clone()]).into_owned();
                    sg.run.push(Item::Text { range: r, text });
                }
            }
            Tok::Comment(r) => {
                if sg.level() == Level::Normal {
                    sg.run.push(Item::Comment(r));
                }
            }
        }
    }
    sg.close_to(0, html.len()..html.len());
    sg.flush();
    let mut out = sg.out;
    out.sort_by_key(|s| s.span.start);
    out
}

// ---------------------------------------------------------------------------------------------- apply

/// Checks `translation` against the segment's placeholders: every tag exactly once and well nested, every
/// value exactly once, no tag in an attribute. `None` = invalid, the source is kept.
fn check<'t>(seg: &Segment, translation: &'t str) -> Option<Vec<Part<'t>>> {
    // Placeholder-looking text out of range (`<5>` for a 2-tag segment) is a translator error too.
    let any = parse_template(translation, usize::MAX, usize::MAX);
    if any.iter().any(|p| match *p {
        Part::Open(k) | Part::Close(k) | Part::Void(k) => k >= seg.tags.len(),
        Part::Val(k) => k >= seg.values.len(),
        Part::Text(_) => false,
    }) {
        return None;
    }
    let parts = any;
    let mut seen_tag = vec![0u8; seg.tags.len()];
    let mut seen_val = vec![0u8; seg.values.len()];
    let mut stack = Vec::new();
    for p in &parts {
        match *p {
            Part::Open(k) => {
                if !matches!(seg.tags[k], Placeholder::Pair { .. }) {
                    return None;
                }
                seen_tag[k] += 1;
                stack.push(k);
            }
            Part::Close(k) => {
                if stack.pop() != Some(k) {
                    return None;
                }
            }
            Part::Void(k) => {
                if !matches!(seg.tags[k], Placeholder::Void { .. }) {
                    return None;
                }
                seen_tag[k] += 1;
            }
            Part::Val(k) => seen_val[k] += 1,
            Part::Text(_) => {}
        }
    }
    (stack.is_empty()
        && seen_tag.iter().all(|&n| n == 1)
        && seen_val.iter().all(|&n| n == 1)
        && (seg.attr.is_none() || seg.tags.is_empty()))
    .then_some(parts)
}

/// Source `range` with the `edits` that lie inside it applied (edits sorted, disjoint).
fn splice(src: &str, range: Range<usize>, edits: &[(Range<usize>, String)]) -> String {
    let mut out = String::new();
    let mut at = range.start;
    for (r, v) in edits {
        if r.start >= at && r.end <= range.end {
            out.push_str(&src[at..r.start]);
            out.push_str(v);
            at = r.end;
        }
    }
    out.push_str(&src[at..range.end]);
    out
}

/// Translate `html`: each segment whose `lookup` returns a valid translation (same placeholders) is replaced;
/// everything else, including untranslated segments, is left byte for byte.
pub fn translate_html(
    html: &str,
    opts: &SegmentOpts,
    mut lookup: impl FnMut(&Segment) -> Option<String>,
) -> String {
    let segs = segment(html, opts);
    let mut attr_edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut text_tr = Vec::new();
    let valid: Vec<(&Segment, String)> = segs
        .iter()
        .filter_map(|seg| {
            lookup(seg)
                .filter(|t| check(seg, t).is_some())
                .map(|t| (seg, t))
        })
        .collect();
    for (seg, t) in &valid {
        let parts = check(seg, t).unwrap_or_default();
        match &seg.attr {
            Some(name) => {
                let v: String = parts
                    .iter()
                    .map(|p| match *p {
                        Part::Val(k) => seg.values[k].value.as_str(),
                        Part::Text(x) => x,
                        _ => "",
                    })
                    .collect();
                attr_edits.push((
                    seg.span.clone(),
                    format!("{name}=\"{}\"", protect::escape_attr(&v)),
                ));
            }
            None => text_tr.push((*seg, parts)),
        }
    }
    attr_edits.sort_by_key(|e| e.0.start);
    let mut edits: Vec<(Range<usize>, String)> = text_tr
        .into_iter()
        .map(|(seg, parts)| {
            let mut out = String::new();
            for p in parts {
                match p {
                    Part::Text(x) => out.push_str(&protect::escape_text(x)),
                    Part::Val(k) => out.push_str(&protect::escape_text(&seg.values[k].value)),
                    Part::Open(k) | Part::Close(k) | Part::Void(k) => {
                        let r = match (&seg.tags[k], &p) {
                            (Placeholder::Pair { close, .. }, Part::Close(_)) => close.clone(),
                            (Placeholder::Pair { open, .. }, _) => open.clone(),
                            (Placeholder::Void { range, .. }, _) => range.clone(),
                        };
                        out.push_str(&splice(html, r, &attr_edits));
                    }
                }
            }
            (seg.span.clone(), out)
        })
        .collect();
    let covered: Vec<Range<usize>> = edits.iter().map(|e| e.0.clone()).collect();
    edits.extend(
        attr_edits
            .into_iter()
            .filter(|(r, _)| !covered.iter().any(|c| c.start <= r.start && r.end <= c.end)),
    );
    edits.sort_by_key(|e| e.0.start);
    splice(html, 0..html.len(), &edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(html: &str) -> Vec<(String, String)> {
        segment(html, &SegmentOpts::default())
            .into_iter()
            .map(|s| (s.ctx.role, s.text))
            .collect()
    }

    fn t(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn blocks_inline_and_attributes() {
        assert_eq!(
            texts(
                r#"<html><head><title>Clip &amp; go</title><meta name="description" content="Make clips"></head>
            <body><h1>Hello <b>world</b></h1><p>Read our <a href="/t">terms</a> today.</p>
            <nav><a href="/">Home</a> <a href="/p"><svg><path/></svg> Pricing</a></nav>
            <button><svg viewBox="0 0 1 1"></svg> Save</button><img src=x alt="A cat">
            <script>var a = "no";</script><pre>no</pre><p>Save 20 %</p><p>42</p></body></html>"#
            ),
            t(&[
                ("title", "Clip & go"),
                ("meta:description", "Make clips"),
                ("h1", "Hello <0>world</0>"),
                ("p", "Read our <0>terms</0> today."),
                ("a", "Home"),
                ("a", "Pricing"),
                ("button", "Save"),
                ("img:alt", "A cat"),
                ("p", "Save {0} %"),
            ])
        );
    }

    #[test]
    fn protection() {
        let html = r#"<p>Hello <span data-glot-user>Jean Dupont</span>, welcome</p>
            <div class="notranslate">Brand</div><p translate="no">Nope</p><textarea placeholder="Your message">typed</textarea>
            <input type="text" value="user value"><input type="submit" value="Send">
            <p>Mail me at jean@gmail.com</p><div contenteditable>draft</div><p class="acct">Account 1</p>"#;
        let segs = segment(html, &SegmentOpts::default().skip(&[".acct"]));
        let got: Vec<(&str, bool)> = segs
            .iter()
            .map(|s| (s.text.as_str(), s.discoverable))
            .collect();
        assert_eq!(
            got,
            [
                ("Hello <0/>, welcome", true),
                ("Your message", true),
                ("Send", true),
                ("Mail me at {0}", false),
            ]
        );
    }

    #[test]
    fn implied_closes_and_inline_blocks() {
        assert_eq!(
            texts("<ul><li>One<li>Two</ul><p>A<div>B</div><a href=/><div>Card</div>more</a>"),
            t(&[
                ("li", "One"),
                ("li", "Two"),
                ("p", "A"),
                ("div", "B"),
                ("div", "Card"),
                ("body", "more")
            ])
        );
    }

    #[test]
    fn translate_round_trip_and_validation() {
        let html = r#"<p class="x">Read our <a href="/t" title="Terms page">terms</a>, save 20&nbsp;%.</p><p>Keep</p>"#;
        let out = translate_html(html, &SegmentOpts::default(), |s| match s.text.as_str() {
            "Read our <0>terms</0>, save {0}\u{a0}%." => {
                Some("Lee nuestros <0>términos</0>, ahorra {0}\u{a0}% & más.".into())
            }
            "Terms page" => Some("Página \"términos\"".into()),
            "Keep" => Some("<0>broken</0>".into()), // invalid: stays source
            _ => None,
        });
        assert_eq!(
            out,
            "<p class=\"x\">Lee nuestros <a href=\"/t\" title=\"Página &quot;términos&quot;\">términos</a>, ahorra 20\u{a0}% &amp; más.</p><p>Keep</p>"
        );
        // Identity translation keeps the document.
        let same = translate_html(html, &SegmentOpts::default(), |s| Some(s.text.clone()));
        assert_eq!(same, html.replace("&nbsp;", "\u{a0}"));
    }
}
