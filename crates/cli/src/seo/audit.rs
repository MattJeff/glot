//! `glot seo audit`: what a crawler sees in each locale's server HTML (hreflang, canonical,
//! `<html lang>`, title/description, sitemap, untranslated indexable pages). Pure over fetched
//! pages so the rules are unit-tested on static HTML.
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

use glot_llm::seo::{DESCRIPTION_MAX, TITLE_MAX, serp_width};

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub status: u16,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub canonical: Option<String>,
    /// (hreflang, absolute href)
    pub hreflang: Vec<(String, String)>,
    pub noindex: bool,
    /// Visible text segments (between tags, outside script/style).
    pub text: Vec<String>,
}

fn decode(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// `name="v"` / `name='v'` / `name=v` / `name`, names lowercased.
fn attrs(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let name = tag[start..i].to_ascii_lowercase();
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let end = tag[i + 1..].find(q as char).map_or(b.len(), |e| i + 1 + e);
                value = decode(&tag[i + 1..end]);
                i = end + 1;
            } else {
                let s = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = decode(&tag[s..i]);
            }
        }
        if !name.is_empty() {
            out.insert(name, value);
        }
    }
    out
}

/// Minimal tag scanner: enough for the head tags crawlers read and the visible text.
pub fn parse(url: &str, status: u16, html: &str) -> Page {
    let mut p = Page {
        status,
        ..Default::default()
    };
    let base = reqwest::Url::parse(url).ok();
    let abs = |h: &str| {
        base.as_ref()
            .and_then(|b| b.join(h).ok())
            .map_or(h.to_string(), |u| u.to_string())
    };
    let mut i = 0;
    while let Some(rel) = html[i..].find('<') {
        let text = decode(html[i..i + rel].trim());
        if text.chars().any(char::is_alphabetic) {
            p.text.push(text);
        }
        let open = i + rel;
        if html[open..].starts_with("<!--") {
            i = html[open..]
                .find("-->")
                .map_or(html.len(), |e| open + e + 3);
            continue;
        }
        let Some(close) = html[open..].find('>').map(|e| open + e) else {
            break;
        };
        let tag = &html[open + 1..close];
        let name_end = tag
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(tag.len());
        let name = tag[..name_end].to_ascii_lowercase();
        let a = attrs(&tag[name_end..]);
        i = close + 1;
        match name.as_str() {
            "html" => p.lang = a.get("lang").cloned(),
            "title" | "script" | "style" => {
                let end = html[i..]
                    .to_ascii_lowercase()
                    .find(&format!("</{name}"))
                    .map_or(html.len(), |e| i + e);
                if name == "title" && p.title.is_none() {
                    p.title = Some(decode(html[i..end].trim()));
                }
                i = end;
            }
            "meta" => match a.get("name").map(|n| n.to_ascii_lowercase()).as_deref() {
                Some("description") => p.description = a.get("content").cloned(),
                Some("robots") | Some("googlebot") => {
                    p.noindex |= a
                        .get("content")
                        .is_some_and(|c| c.to_ascii_lowercase().contains("noindex"))
                }
                _ => {}
            },
            "link" => {
                let rel = a.get("rel").map(|r| r.to_ascii_lowercase());
                let href = a.get("href").map(|h| abs(h));
                match (rel.as_deref(), href, a.get("hreflang")) {
                    (Some("canonical"), Some(h), _) => p.canonical = Some(h),
                    (Some("alternate"), Some(h), Some(l)) => p.hreflang.push((l.clone(), h)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    p
}

/// `<loc>` URLs of a sitemap (or sitemap index).
pub fn sitemap_locs(xml: &str) -> Vec<String> {
    xml.split("<loc>")
        .skip(1)
        .filter_map(|s| Some(decode(s.split("</loc>").next()?.trim())))
        .collect()
}

pub fn norm(u: &str) -> String {
    let u = u.split('#').next().unwrap_or(u);
    match u.strip_suffix('/') {
        Some(s) if s.matches('/').count() > 2 => s.to_string(),
        _ => u.to_string(),
    }
}

fn primary(lang: &str) -> String {
    lang.split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: &'static str,
    pub locale: String,
    pub route: String,
    pub url: String,
    pub message: String,
    /// Catalog key of the offending text (title / description).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

pub struct Site {
    /// Source first.
    pub locales: Vec<String>,
    pub routes: Vec<String>,
    /// (route, locale) -> URL audited.
    pub urls: BTreeMap<(String, String), String>,
    /// Every fetched page by normalized URL (locale pages + hreflang targets).
    pub pages: HashMap<String, Page>,
    /// `None`: no sitemap.xml.
    pub sitemap: Option<Vec<String>>,
    pub robots: Option<String>,
    /// locale -> key -> value (to attach findings to catalog keys).
    pub catalogs: BTreeMap<String, BTreeMap<String, String>>,
}

impl Site {
    fn key_of(&self, locale: &str, text: &str) -> Option<String> {
        let t = text.trim();
        self.catalogs
            .get(locale)?
            .iter()
            .find(|(_, v)| v.trim() == t)
            .map(|(k, _)| k.clone())
    }
}

/// Valid BCP 47 shape and none of the classic mistakes (`jp`, `en-UK`, `en_US`).
fn bad_hreflang(l: &str) -> bool {
    if l == "x-default" {
        return false;
    }
    let mut parts = l.split('-');
    let lang = parts.next().unwrap_or("");
    let region = parts.next();
    !(2..=3).contains(&lang.len())
        || !lang.chars().all(|c| c.is_ascii_lowercase())
        || ["jp", "cn", "kr", "dk", "se", "gr", "cz", "ua"].contains(&lang)
        || region.is_some_and(|r| r.eq_ignore_ascii_case("uk") || r.is_empty())
        || l.contains('_')
}

pub fn audit(site: &Site) -> Vec<Finding> {
    let mut out = vec![];
    let src = &site.locales[0];
    let sitemap: Option<HashSet<String>> = site
        .sitemap
        .as_ref()
        .map(|s| s.iter().map(|u| norm(u)).collect());
    let mut push = |sev, code, locale: &str, route: &str, url: &str, message: String, key| {
        out.push(Finding {
            severity: sev,
            code,
            locale: locale.into(),
            route: route.into(),
            url: url.into(),
            message,
            key,
        })
    };
    use Severity::*;
    if site.sitemap.is_none() {
        push(
            Warning,
            "sitemap_missing",
            "",
            "",
            "",
            "no /sitemap.xml: list every locale URL with hreflang alternates".into(),
            None,
        );
    }
    if site
        .robots
        .as_deref()
        .is_some_and(|r| !r.to_ascii_lowercase().contains("sitemap:"))
    {
        push(
            Warning,
            "robots_no_sitemap",
            "",
            "",
            "",
            "robots.txt does not reference the sitemap".into(),
            None,
        );
    }
    for route in &site.routes {
        if route
            .chars()
            .any(|c| c.is_ascii_uppercase() || c == '_' || c == ' ')
        {
            push(
                Warning,
                "slug_format",
                "",
                route,
                "",
                "slugs should be lowercase words separated by hyphens".into(),
                None,
            );
        }
        let url = |l: &str| {
            site.urls
                .get(&(route.clone(), l.to_string()))
                .cloned()
                .unwrap_or_default()
        };
        let page = |l: &str| site.pages.get(&norm(&url(l)));
        let source = page(src);
        for loc in &site.locales {
            let u = url(loc);
            let Some(p) = page(loc) else { continue };
            let f = |sev, code, msg: String, key| {
                (
                    sev,
                    code,
                    loc.as_str(),
                    route.as_str(),
                    u.as_str(),
                    msg,
                    key,
                )
            };
            let mut emit = |t: (
                Severity,
                &'static str,
                &str,
                &str,
                &str,
                String,
                Option<String>,
            )| { push(t.0, t.1, t.2, t.3, t.4, t.5, t.6) };
            if p.status != 200 {
                emit(f(Error, "http_status", format!("HTTP {}", p.status), None));
                continue;
            }
            match p.lang.as_deref() {
                None => emit(f(
                    Error,
                    "html_lang_missing",
                    "<html> has no lang attribute".into(),
                    None,
                )),
                Some(l) if primary(l) != primary(loc) => emit(f(
                    Error,
                    "html_lang",
                    format!("<html lang=\"{l}\"> on the {loc} page"),
                    None,
                )),
                _ => {}
            }
            if p.noindex {
                continue; // not indexed: no duplicate/untranslated risk
            }
            if let Some(sm) = &sitemap
                && !sm.contains(&norm(&u))
            {
                emit(f(
                    Warning,
                    "sitemap_missing_url",
                    "indexable page not listed in sitemap.xml".into(),
                    None,
                ));
            }
            // canonical
            let variants: Vec<(String, String)> = site
                .locales
                .iter()
                .map(|l| (l.clone(), norm(&url(l))))
                .collect();
            match &p.canonical {
                None => emit(f(
                    Error,
                    "canonical_missing",
                    "no <link rel=\"canonical\">".into(),
                    None,
                )),
                Some(c) if norm(c) == norm(&u) => {}
                Some(c) => match variants.iter().find(|(l, v)| l != loc && *v == norm(c)) {
                    Some((l, _)) => emit(f(
                        Error,
                        "canonical_cross_locale",
                        format!("canonical points to the {l} page ({c}): Google drops this locale"),
                        None,
                    )),
                    None => emit(f(
                        Warning,
                        "canonical_other",
                        format!("canonical is another URL: {c}"),
                        None,
                    )),
                },
            }
            // hreflang
            if site.locales.len() > 1 {
                if p.hreflang.is_empty() {
                    emit(f(
                        Error,
                        "hreflang_missing",
                        "no <link rel=\"alternate\" hreflang> tags".into(),
                        None,
                    ));
                } else {
                    for (l, _) in p.hreflang.iter().filter(|(l, _)| bad_hreflang(l)) {
                        emit(f(
                            Error,
                            "hreflang_invalid",
                            format!(
                                "invalid hreflang code \"{l}\" (ISO 639-1 language, optional ISO 3166 region: fr, fr-CA, ja, en-GB)"
                            ),
                            None,
                        ));
                    }
                    for l in &site.locales {
                        if !p
                            .hreflang
                            .iter()
                            .any(|(h, _)| h.eq_ignore_ascii_case(l) || primary(h) == *l)
                        {
                            emit(f(
                                Error,
                                "hreflang_incomplete",
                                format!("no hreflang for {l}"),
                                None,
                            ));
                        }
                    }
                    let me = [
                        norm(&u),
                        p.canonical.as_deref().map(norm).unwrap_or_default(),
                    ];
                    if !p
                        .hreflang
                        .iter()
                        .any(|(h, href)| primary(h) == primary(loc) && me.contains(&norm(href)))
                    {
                        emit(f(
                            Error,
                            "hreflang_self",
                            format!("no self-referencing hreflang=\"{loc}\" pointing to this URL"),
                            None,
                        ));
                    }
                    if !p.hreflang.iter().any(|(h, _)| h == "x-default") {
                        emit(f(
                            Warning,
                            "hreflang_x_default",
                            "no hreflang=\"x-default\" fallback".into(),
                            None,
                        ));
                    }
                    for (h, href) in p
                        .hreflang
                        .iter()
                        .filter(|(h, href)| h != "x-default" && !me.contains(&norm(href)))
                    {
                        match site.pages.get(&norm(href)) {
                            Some(t) if t.status == 200 => {
                                if !t.hreflang.iter().any(|(_, back)| me.contains(&norm(back))) {
                                    emit(f(
                                        Error,
                                        "hreflang_not_reciprocal",
                                        format!(
                                            "{href} (hreflang {h}) does not link back to this page"
                                        ),
                                        None,
                                    ));
                                }
                                if let Some(tl) = &t.lang
                                    && primary(tl) != primary(h)
                                {
                                    emit(f(
                                        Error,
                                        "hreflang_lang_mismatch",
                                        format!(
                                            "hreflang {h} points to a page in <html lang=\"{tl}\">: {href}"
                                        ),
                                        None,
                                    ));
                                }
                            }
                            Some(t) => emit(f(
                                Error,
                                "hreflang_broken",
                                format!("hreflang {h} -> HTTP {}: {href}", t.status),
                                None,
                            )),
                            None => emit(f(
                                Error,
                                "hreflang_broken",
                                format!("hreflang {h} target not reachable: {href}"),
                                None,
                            )),
                        }
                    }
                }
            }
            // title / description
            for (what, val, src_val, max, missing) in [
                (
                    "title",
                    &p.title,
                    source.and_then(|s| s.title.as_ref()),
                    TITLE_MAX,
                    Error,
                ),
                (
                    "description",
                    &p.description,
                    source.and_then(|s| s.description.as_ref()),
                    DESCRIPTION_MAX,
                    Warning,
                ),
            ] {
                let Some(v) = val.as_ref().filter(|v| !v.trim().is_empty()) else {
                    emit(f(
                        missing,
                        if what == "title" {
                            "title_missing"
                        } else {
                            "description_missing"
                        },
                        format!("no {what}"),
                        None,
                    ));
                    continue;
                };
                if loc != src && src_val.is_some_and(|s| s.trim() == v.trim()) {
                    let code = if what == "title" {
                        "title_untranslated"
                    } else {
                        "description_untranslated"
                    };
                    emit(f(
                        Error,
                        code,
                        format!("{what} identical to the {src} page: \"{v}\""),
                        site.key_of(src, v),
                    ));
                }
                let w = serp_width(v);
                if w > max {
                    let code = if what == "title" {
                        "title_too_long"
                    } else {
                        "description_too_long"
                    };
                    emit(f(
                        Warning,
                        code,
                        format!(
                            "{what} is {w} wide (> {max}, CJK counts double): truncated in results"
                        ),
                        site.key_of(loc, v),
                    ));
                }
            }
            // untranslated indexable content (Google "scaled content" / duplicate risk)
            if loc != src
                && let Some(s) = source
            {
                let src_text: HashSet<&str> = s.text.iter().map(String::as_str).collect();
                let segs: Vec<&String> = p
                    .text
                    .iter()
                    .filter(|t| t.chars().filter(|c| c.is_alphabetic()).count() >= 4)
                    .collect();
                let same = segs
                    .iter()
                    .filter(|t| src_text.contains(t.as_str()))
                    .count();
                if segs.len() >= 3 {
                    let r = same as f64 / segs.len() as f64;
                    let msg = format!(
                        "{:.0} % of the visible text is identical to the {src} page",
                        r * 100.0
                    );
                    if r >= 0.9 {
                        emit(f(
                            Error,
                            "untranslated_page",
                            format!(
                                "{msg}: indexable duplicate (translate it, or noindex/drop its hreflang)"
                            ),
                            None,
                        ));
                    } else if r >= 0.5 {
                        emit(f(Warning, "partially_untranslated", msg, None));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        (a.severity, &a.route, &a.locale, a.code).cmp(&(b.severity, &b.route, &b.locale, b.code))
    });
    out
}

pub fn markdown(findings: &[Finding], pages: usize) -> String {
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let mut s = format!(
        "# SEO audit\n\n{pages} pages, **{errors} errors**, {} warnings.\n",
        findings.len() - errors
    );
    if findings.is_empty() {
        return s;
    }
    s.push_str(
        "\n| severity | code | locale | route | key | message |\n|---|---|---|---|---|---|\n",
    );
    for f in findings {
        s.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} |\n",
            if f.severity == Severity::Error {
                "error"
            } else {
                "warning"
            },
            f.code,
            f.locale,
            f.route,
            f.key
                .as_deref()
                .map(|k| format!("`{k}`"))
                .unwrap_or_default(),
            f.message.replace('|', "\\|")
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "https://x.test";

    fn html(
        lang: &str,
        title: &str,
        desc: &str,
        links: &[(&str, &str)],
        canonical: &str,
        body: &str,
    ) -> String {
        let links: String = links
            .iter()
            .map(|(l, h)| format!("<link rel=\"alternate\" hrefLang=\"{l}\" href=\"{h}\"/>"))
            .collect();
        format!(
            "<!DOCTYPE html><html lang=\"{lang}\"><head><title>{title}</title><meta name=\"description\" content=\"{desc}\"/><link rel=\"canonical\" href=\"{canonical}\"/>{links}<script>var a = \"<p>x</p>\";</script></head><body><h1>{body}</h1><p>Second paragraph here</p><p>Third paragraph text</p><!-- c --></body></html>"
        )
    }

    fn site(pages: Vec<(&str, String)>) -> Site {
        let urls = [("en", "/"), ("fr", "/fr")]
            .iter()
            .map(|(l, p)| (("/".to_string(), l.to_string()), format!("{B}{p}")))
            .collect();
        Site {
            locales: vec!["en".into(), "fr".into()],
            routes: vec!["/".into()],
            urls,
            pages: pages
                .into_iter()
                .map(|(p, h)| (norm(&format!("{B}{p}")), parse(&format!("{B}{p}"), 200, &h)))
                .collect(),
            sitemap: Some(vec![format!("{B}/"), format!("{B}/fr")]),
            robots: None,
            catalogs: [
                (
                    "en".to_string(),
                    [("home.title".to_string(), "Acme — Invoices".to_string())].into(),
                ),
                (
                    "fr".to_string(),
                    [("home.desc".to_string(), "é".repeat(170))].into(),
                ),
            ]
            .into(),
        }
    }

    fn codes(s: &Site) -> Vec<(&'static str, String, Option<String>)> {
        audit(s)
            .into_iter()
            .map(|f| (f.code, f.locale, f.key))
            .collect()
    }

    const ALL: &[(&str, &str)] = &[
        ("en", "https://x.test/"),
        ("fr", "https://x.test/fr"),
        ("x-default", "https://x.test/"),
    ];

    #[test]
    fn parses_head_and_text() {
        let p = parse(
            &format!("{B}/fr"),
            200,
            &html("fr", "A &amp; B", "d", &[("fr", "/fr")], "/fr", "Bonjour"),
        );
        assert_eq!(p.lang.as_deref(), Some("fr"));
        assert_eq!(p.title.as_deref(), Some("A & B"));
        assert_eq!(p.canonical.as_deref(), Some("https://x.test/fr"));
        assert_eq!(
            p.hreflang,
            [("fr".to_string(), "https://x.test/fr".to_string())]
        );
        assert_eq!(
            p.text,
            ["Bonjour", "Second paragraph here", "Third paragraph text"]
        );
    }

    #[test]
    fn clean_site_has_no_findings() {
        let s = site(vec![
            (
                "/",
                html(
                    "en",
                    "Acme — Invoices",
                    "Send invoices",
                    ALL,
                    "https://x.test/",
                    "Hello world",
                ),
            ),
            (
                "/fr",
                html(
                    "fr",
                    "Acme — Factures",
                    "Envoyez vos factures",
                    ALL,
                    "https://x.test/fr",
                    "Bonjour le monde",
                ),
            ),
        ]);
        let mut s = s;
        for p in s.pages.values_mut() {
            p.text.truncate(1); // distinct text only
        }
        assert_eq!(codes(&s), vec![]);
    }

    #[test]
    fn hreflang_missing_and_not_reciprocal() {
        let s = site(vec![
            (
                "/",
                html(
                    "en",
                    "Acme — Invoices",
                    "Send",
                    ALL,
                    "https://x.test/",
                    "Hello world",
                ),
            ),
            (
                "/fr",
                html(
                    "fr",
                    "Acme — Factures",
                    "Envoyez",
                    &[],
                    "https://x.test/fr",
                    "Bonjour le monde",
                ),
            ),
        ]);
        let c = codes(&s);
        assert!(
            c.contains(&("hreflang_missing", "fr".into(), None)),
            "{c:?}"
        );
        assert!(
            c.contains(&("hreflang_not_reciprocal", "en".into(), None)),
            "{c:?}"
        );
    }

    #[test]
    fn untranslated_title_long_description_cross_canonical() {
        let long = "é".repeat(170);
        let s = site(vec![
            (
                "/",
                html(
                    "en",
                    "Acme — Invoices",
                    "Send",
                    ALL,
                    "https://x.test/",
                    "Hello world",
                ),
            ),
            (
                "/fr",
                html(
                    "en",
                    "Acme — Invoices",
                    &long,
                    &ALL[..2],
                    "https://x.test/",
                    "Hello world",
                ),
            ),
        ]);
        let c = codes(&s);
        for want in [
            (
                "title_untranslated",
                "fr".to_string(),
                Some("home.title".to_string()),
            ),
            (
                "description_too_long",
                "fr".into(),
                Some("home.desc".into()),
            ),
            ("canonical_cross_locale", "fr".into(), None),
            ("html_lang", "fr".into(), None),
            ("untranslated_page", "fr".into(), None),
            ("hreflang_x_default", "fr".into(), None),
        ] {
            assert!(c.contains(&want), "missing {want:?} in {c:?}");
        }
    }

    #[test]
    fn hreflang_codes_and_sitemap() {
        assert!(
            bad_hreflang("jp")
                && bad_hreflang("en-UK")
                && bad_hreflang("en_US")
                && bad_hreflang("FR")
        );
        assert!(
            !bad_hreflang("fr")
                && !bad_hreflang("pt-BR")
                && !bad_hreflang("x-default")
                && !bad_hreflang("zh-Hant")
        );
        assert_eq!(
            sitemap_locs(
                "<urlset><url><loc>https://x.test/?a=1&amp;b=2</loc></url><url><loc> https://x.test/fr </loc></url></urlset>"
            ),
            ["https://x.test/?a=1&b=2", "https://x.test/fr"]
        );
    }
}
