//! Application paths (`edge_settings.app_paths`): pages that must stay on the origin's own domain (login,
//! account, payment, OAuth callbacks, API…). The session cookie is host-only on the origin and OAuth redirect
//! URIs are registered there, so these paths are never translated, cached or discovered by Edge.
//!
//! One rule per line: `/login` (default action), `redirect /dashboard`, `proxy /api/`.
//! - `/explainer` = `/explainer` and `/explainer/…`, never `/explainer-video` (segment match);
//! - `/live/` = children only (`/live/abc`), not `/live`;
//! - `*` = any run of characters, whole path (`/x*` = raw prefix).
//!
//! Matching is on the normalized path (unreserved `%XX` decoded, `\` → `/`, `//`, `.`, `..` resolved),
//! case-insensitive. `/sitemap.xml`, `/robots.txt` and `/__glot/…` are never app paths (glot serves them).
//! `redirect`: 302 (307 for non-GET/HEAD: method and body kept) to the domain's own origin + normalized path +
//! original query, the locale carried as the site expects it (`query:lang` → `lang=es` added/replaced,
//! `prefix` → `/es/…`, `none`). Loop guard: a short-lived `glot_app_hop` cookie on the edge host; a request
//! for the same path carrying it (the origin sent the visitor back) is proxied instead, as is a target equal
//! to the request itself. `proxy`: the origin response untouched (cookies included), untranslated, uncached.
//!
//! Same algorithm as `packages/edge/src/app-paths.ts`; shared vectors in
//! `packages/edge/test/fixtures/app-paths.json`.

/// Suggested rules: the `edge_settings.app_paths` column default for new rows (migration 0155).
pub const SUGGESTED: &[&str] = &[
    "/login",
    "/register",
    "/dashboard",
    "/auth/",
    "/oauth/",
    "/settings",
    "/account",
    "/checkout",
    "/billing",
    "proxy /api/",
];

pub const HOP_COOKIE: &str = "glot_app_hop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Redirect,
    Proxy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Proxy,
    Redirect {
        status: u16,
        location: String,
        cookie: String,
    },
}

/// One rule line → (action, lowercase pattern); None for blank / `#` lines. Err = invalid line.
pub fn parse_line(line: &str, default: Action) -> Result<Option<(Action, String)>, String> {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') {
        return Ok(None);
    }
    let mut it = l.split_whitespace();
    let first = it.next().unwrap_or("");
    let (action, pattern) = match first.to_ascii_lowercase().as_str() {
        "redirect" => (Action::Redirect, it.next()),
        "proxy" => (Action::Proxy, it.next()),
        _ => (default, Some(first)),
    };
    let p = pattern.unwrap_or("");
    if !p.starts_with('/') || it.next().is_some() || p.len() > 200 {
        return Err(format!(
            "invalid app path rule '{l}' (one path per line, starting with '/', optionally after 'redirect' or 'proxy')"
        ));
    }
    Ok(Some((action, p.to_ascii_lowercase())))
}

fn unreserved(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~')
}

/// Normalized path: always starts with `/`, ASCII only, no `//`, `.` or `..` segments.
pub fn normalize(path: &str) -> String {
    let b = path.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut s = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|&x| hex(x)),
                b.get(i + 2).and_then(|&x| hex(x)),
            )
        {
            let v = h * 16 + l;
            if unreserved(v) {
                s.push(v as char);
            } else {
                s.push_str(&format!("%{v:02X}"));
            }
            i += 3;
            continue;
        }
        if c == b'\\' {
            s.push('/');
        } else if unreserved(c) || b"/!$&'()*+,;=:@".contains(&c) {
            s.push(c as char);
        } else {
            s.push_str(&format!("%{c:02X}"));
        }
        i += 1;
    }
    let trailing = s.ends_with('/') || s.ends_with("/.") || s.ends_with("/..");
    let mut segs: Vec<&str> = vec![];
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            x => segs.push(x),
        }
    }
    let mut out = format!("/{}", segs.join("/"));
    if trailing && !segs.is_empty() {
        out.push('/');
    }
    out
}

/// `*` = any run of bytes, the whole of `s`.
fn wild(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if pi < p.len() && p[pi] == s[si] {
            pi += 1;
            si += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// `path`: normalized, lowercase.
fn matches(pattern: &str, path: &str) -> bool {
    if pattern == "/" {
        true
    } else if pattern.contains('*') {
        wild(pattern.as_bytes(), path.as_bytes())
    } else if pattern.ends_with('/') {
        path.len() > pattern.len() && path.starts_with(pattern)
    } else {
        path == pattern
            || path
                .strip_prefix(pattern)
                .is_some_and(|r| r.starts_with('/'))
    }
}

/// The first rule matching `path` (raw request path, no query).
pub fn classify(rules: &[String], default: Action, path: &str) -> Option<Action> {
    let p = normalize(path).to_ascii_lowercase();
    if p == "/sitemap.xml" || p == "/robots.txt" || p.starts_with("/__glot/") {
        return None;
    }
    rules
        .iter()
        .filter_map(|l| parse_line(l, default).ok().flatten())
        .find(|(_, pat)| matches(pat, &p))
        .map(|(a, _)| a)
}

/// How the site carries the locale, from the project's preview `url_pattern`: `{route}?lang={locale}` →
/// `query:lang`, `/{locale}{route}` → `prefix`, anything else (or no preview settings) → `none`.
pub fn locale_style(url_pattern: Option<&str>) -> String {
    let Some(p) = url_pattern else {
        return "none".into();
    };
    if let Some((_, q)) = p.split_once('?') {
        for kv in q.split('&') {
            if let Some((k, "{locale}")) = kv.split_once('=')
                && !k.is_empty()
                && k.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return format!("query:{k}");
            }
        }
    }
    if p.starts_with("/{locale}{route}") || p.starts_with("/{locale}/") {
        "prefix".into()
    } else {
        "none".into()
    }
}

fn hop_value(norm: &str) -> String {
    norm.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "/._~%-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Everything a decision depends on. `path` = the origin path (locale prefix removed), `req_path` = the
/// public path of this request (equal in subdomain mode), `origin` = the domain's own verified origin.
pub struct Input<'a> {
    pub rules: &'a [String],
    pub action: Action,
    pub locale_style: &'a str,
    pub locale: &'a str,
    pub origin: &'a str,
    pub host: &'a str,
    pub req_path: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub method: &'a str,
    pub cookie: &'a str,
}

pub fn decide(i: &Input) -> Option<Decision> {
    let action = classify(i.rules, i.action, i.path)?;
    if action == Action::Proxy {
        return Some(Decision::Proxy);
    }
    // The origin's scheme://host[:port] only: never anything from the request.
    let Ok(base) = reqwest::Url::parse(i.origin) else {
        return Some(Decision::Proxy);
    };
    let base = base.origin().ascii_serialization();
    let norm = normalize(i.path);
    let hop = hop_value(&norm);
    let bounced = i.cookie.split(';').any(|kv| {
        kv.trim()
            .split_once('=')
            .is_some_and(|(k, v)| k == HOP_COOKIE && v == hop)
    });
    let target_path = match i.locale_style {
        "prefix" => format!("/{}{norm}", i.locale),
        _ => norm.clone(),
    };
    let mut q: Vec<String> = i
        .query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    if let Some(param) = i.locale_style.strip_prefix("query:") {
        q.retain(|kv| kv.split('=').next() != Some(param));
        q.push(format!("{param}={}", i.locale));
    }
    let host = base.split_once("://").map(|x| x.1).unwrap_or("");
    // Same URL as this request (the edge serves the origin's own host): redirecting would loop.
    if bounced || (host.eq_ignore_ascii_case(i.host) && target_path == normalize(i.req_path)) {
        return Some(Decision::Proxy);
    }
    let location = if q.is_empty() {
        format!("{base}{target_path}")
    } else {
        format!("{base}{target_path}?{}", q.join("&"))
    };
    let get = i.method.eq_ignore_ascii_case("GET") || i.method.eq_ignore_ascii_case("HEAD");
    Some(Decision::Redirect {
        status: if get { 302 } else { 307 },
        location,
        cookie: format!("{HOP_COOKIE}={hop}; Max-Age=30; Path=/; HttpOnly; Secure; SameSite=Lax"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const FIXTURES: &str = include_str!("../../../../packages/edge/test/fixtures/app-paths.json");

    fn rules(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    }
    fn action(v: &Value) -> Action {
        if v.as_str() == Some("proxy") {
            Action::Proxy
        } else {
            Action::Redirect
        }
    }

    #[test]
    fn shared_vectors() {
        let f: Value = serde_json::from_str(FIXTURES).unwrap();
        for c in f["normalize"].as_array().unwrap() {
            assert_eq!(normalize(c[0].as_str().unwrap()), c[1], "{c}");
        }
        for g in f["classify"].as_array().unwrap() {
            let r = rules(&g["rules"]);
            for c in g["cases"].as_array().unwrap() {
                let got =
                    classify(&r, action(&g["action"]), c[0].as_str().unwrap()).map(|a| match a {
                        Action::Redirect => "redirect",
                        Action::Proxy => "proxy",
                    });
                assert_eq!(got, c[1].as_str(), "{c}");
            }
        }
        for c in f["locale_style"].as_array().unwrap() {
            assert_eq!(locale_style(c[0].as_str()), c[1], "{c}");
        }
        for c in f["decide"].as_array().unwrap() {
            let s = |k: &str| c["in"][k].as_str().unwrap_or("");
            let r = rules(&c["in"]["rules"]);
            let got = decide(&Input {
                rules: &r,
                action: action(&c["in"]["action"]),
                locale_style: s("locale_style"),
                locale: s("locale"),
                origin: s("origin"),
                host: s("host"),
                req_path: s("req_path"),
                path: s("path"),
                query: s("query"),
                method: s("method"),
                cookie: s("cookie"),
            });
            let got = match got {
                None => serde_json::json!({"kind": "none"}),
                Some(Decision::Proxy) => serde_json::json!({"kind": "proxy"}),
                Some(Decision::Redirect {
                    status,
                    location,
                    cookie,
                }) => {
                    serde_json::json!({"kind": "redirect", "status": status, "location": location, "cookie": cookie})
                }
            };
            assert_eq!(got, c["out"], "{}", c["name"]);
        }
    }

    #[test]
    fn rule_lines() {
        assert_eq!(
            parse_line("  proxy  /Assets/ ", Action::Redirect),
            Ok(Some((Action::Proxy, "/assets/".into())))
        );
        assert_eq!(parse_line("# comment", Action::Redirect), Ok(None));
        for bad in ["login", "proxy", "/a /b", "redirect x"] {
            assert!(parse_line(bad, Action::Redirect).is_err(), "{bad}");
        }
        for s in SUGGESTED {
            assert!(parse_line(s, Action::Redirect).unwrap().is_some(), "{s}");
        }
    }
}
