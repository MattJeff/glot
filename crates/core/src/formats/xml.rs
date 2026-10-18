//! Minimal XML scanner with byte offsets (patching needs spans, not a DOM).

#[derive(Debug, Clone)]
pub(crate) struct Elem {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    /// `<` of the start tag.
    pub start: usize,
    /// After the start tag's `>`.
    pub open_end: usize,
    /// `<` of the end tag (`open_end` when self-closing).
    pub close: usize,
    /// After the end tag's `>`.
    pub end: usize,
}

impl Elem {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Comment(String),
    Elem(Elem),
}

/// Child nodes of `s[from..to]`, skipping text, `<?..?>`, `<!DOCTYPE>`, CDATA.
pub(crate) fn children(s: &str, from: usize, to: usize) -> Result<Vec<Node>, String> {
    let mut out = vec![];
    let mut i = from;
    while let Some(off) = s[i..to].find('<') {
        let at = i + off;
        let rest = &s[at..to];
        if rest.starts_with("<!--") {
            let e = rest.find("-->").ok_or("unclosed comment")?;
            out.push(Node::Comment(rest[4..e].trim().to_string()));
            i = at + e + 3;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = at + rest.find("]]>").ok_or("unclosed CDATA")? + 3;
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") || rest.starts_with("</") {
            i = at + rest.find('>').ok_or("unclosed tag")? + 1;
            continue;
        }
        let gt = tag_end(rest).ok_or("unclosed start tag")?;
        let head = &rest[1..gt];
        let self_closing = head.ends_with('/');
        let head = head.trim_end_matches('/');
        let name_len = head.find(|c: char| c.is_whitespace()).unwrap_or(head.len());
        let tag = head[..name_len].to_string();
        let attrs = parse_attrs(&head[name_len..]);
        let open_end = at + gt + 1;
        let (close, end) = if self_closing {
            (open_end, open_end)
        } else {
            let close = find_close(s, open_end, to, &tag)
                .ok_or_else(|| format!("<{tag}> is never closed"))?;
            (close, close + s[close..].find('>').unwrap() + 1)
        };
        out.push(Node::Elem(Elem {
            tag,
            attrs,
            start: at,
            open_end,
            close,
            end,
        }));
        i = end;
    }
    Ok(out)
}

/// Index of the `>` ending the tag starting at `s[0]`, quotes respected.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_attrs(mut s: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    while let Some(eq) = s.find('=') {
        let name = s[..eq].trim().to_string();
        let rest = s[eq + 1..].trim_start();
        let Some(q) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
            break;
        };
        let Some(e) = rest[1..].find(q) else { break };
        out.push((name, unescape(&rest[1..1 + e])));
        s = &rest[e + 2..];
    }
    out
}

/// `<` of the `</tag>` closing an element whose content starts at `from` (nesting aware).
fn find_close(s: &str, from: usize, to: usize, tag: &str) -> Option<usize> {
    let mut depth = 0;
    let mut i = from;
    let boundary = |r: &str, n: usize| {
        r[n..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c == '/' || c.is_whitespace())
    };
    while let Some(off) = s[i..to].find('<') {
        let at = i + off;
        let r = &s[at..to];
        if r.starts_with("<![CDATA[") {
            i = at + r.find("]]>")? + 3;
        } else if r.starts_with("<!--") {
            i = at + r.find("-->")? + 3;
        } else if r.starts_with("</") && r[2..].starts_with(tag) && boundary(r, 2 + tag.len()) {
            if depth == 0 {
                return Some(at);
            }
            depth -= 1;
            i = at + 1;
        } else if r[1..].starts_with(tag) && boundary(r, 1 + tag.len()) {
            if !r[..tag_end(r)?].ends_with('/') {
                depth += 1;
            }
            i = at + 1;
        } else {
            i = at + 1;
        }
    }
    None
}

/// Decode the five predefined entities and numeric references.
pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').and_then(|e| {
            let ent = &rest[1..e];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent
                    .strip_prefix("#x")
                    .or_else(|| ent.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((c, e + 1))
        });
        match decoded {
            Some((c, n)) => {
                out.push(c);
                rest = &rest[n..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escape text content (`&`, `<`, `>`).
pub(crate) fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
