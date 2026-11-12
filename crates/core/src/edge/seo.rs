//! glot Edge — international SEO rewrite, run in the same lol_html pass as the segment translation.
//!
//! On a page served in `locale`: `<html lang dir>`, one canonical (the origin's canonical, localized), reciprocal
//! hreflang alternates for every published locale + x-default, `og:locale` / `og:url`, JSON-LD strings translated
//! through the TM and its URLs localized, internal links rewritten to the localized URL. Text segments (title,
//! meta description, OG/Twitter text) are the segmenter's job (`translatable attributes` contract), not this one.
//!
//! URLs are built from one base per locale (`https://cliponit.app/es` in subdirectory mode,
//! `https://es.cliponit.app` in subdomain mode): localized URL = base(locale) + path of the source URL.
use lol_html::html_content::ContentType;
use lol_html::{ElementContentHandlers, Selector, element, text};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::RefCell;

/// Everything the SEO rewrite needs for one served page.
pub struct SeoCtx<'a> {
    pub source_locale: String,
    /// The locale this response is served in (may be the source: a proxied source page gets the alternates too).
    pub locale: String,
    /// `(locale, base URL without trailing slash)` for every published locale, the source first.
    /// A locale missing here (not ready) gets no hreflang and no localized links.
    pub bases: Vec<(String, String)>,
    /// Path + query of the page in source terms (`/pricing?x=1`), as fetched from the origin.
    pub path: String,
    /// Translates one free text (JSON-LD values); `None` keeps the source.
    pub tr: &'a dyn Fn(&str) -> Option<String>,
    /// Target locales whose version of THIS page is complete enough for search engines; `None` = every
    /// published locale. A page whose own locale is missing here is a duplicate of the source for now:
    /// canonical to the source URL, no alternates (and no other page lists it).
    pub page_locales: Option<Vec<String>>,
    /// Translated URL paths (opt-in); `None` = every locale keeps the source paths.
    pub slugs: Option<&'a Slugs>,
}

/// Translated URL paths: locale → source path (no query, no trailing slash but `/`) → localized path.
pub type Slugs = std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>;

/// `path` (source terms, may carry `?query` / `#fragment` / a trailing slash) with its translated slug in `locale`.
pub fn localized_path(slugs: Option<&Slugs>, locale: &str, path: &str) -> String {
    let cut = path.find(['?', '#']).unwrap_or(path.len());
    let (p, rest) = path.split_at(cut);
    let key = if p.len() > 1 {
        p.trim_end_matches('/')
    } else {
        p
    };
    match slugs.and_then(|s| s.get(locale)?.get(key)) {
        Some(slug) => format!("{slug}{}{rest}", &p[key.len()..]),
        None => path.to_string(),
    }
}

/// Open Graph wants `language_TERRITORY`: `es` → `es_ES`, `pt-BR` → `pt_BR`.
pub fn og_locale(locale: &str) -> String {
    let (lang, region) = match locale.split_once(['-', '_']) {
        Some((l, r)) => (l, Some(r)),
        None => (locale, None),
    };
    let lang = lang.to_ascii_lowercase();
    let region = match region {
        Some(r) => r.to_ascii_uppercase(),
        None => match lang.as_str() {
            "en" => "US",
            "ja" => "JP",
            "ko" => "KR",
            "zh" => "CN",
            "ar" => "AR",
            "he" => "IL",
            "el" => "GR",
            "da" => "DK",
            "sv" => "SE",
            "cs" => "CZ",
            "uk" => "UA",
            "hi" => "IN",
            "vi" => "VN",
            "fa" => "IR",
            "nb" | "no" => "NO",
            "pt" => "PT",
            _ => return format!("{lang}_{}", lang.to_ascii_uppercase()),
        }
        .to_string(),
    };
    format!("{lang}_{region}")
}

impl SeoCtx<'_> {
    fn base(&self, locale: &str) -> Option<&str> {
        self.bases
            .iter()
            .find(|(l, _)| l == locale)
            .map(|(_, b)| b.trim_end_matches('/'))
    }

    fn source_base(&self) -> &str {
        self.base(&self.source_locale).unwrap_or("")
    }

    /// Absolute URL of `path` (source terms) in `locale`.
    pub fn url_for(&self, locale: &str, path: &str) -> Option<String> {
        let path = if path.is_empty() { "/" } else { path };
        let base = self.base(locale)?;
        Some(format!(
            "{base}{}",
            localized_path(self.slugs, locale, path)
        ))
    }
    /// `locale`'s version of this page may be shown to search engines (the source always is).
    pub fn advertised(&self, locale: &str) -> bool {
        locale == self.source_locale
            || (self.base(locale).is_some()
                && self
                    .page_locales
                    .as_ref()
                    .is_none_or(|l| l.iter().any(|x| x == locale)))
    }

    /// `(hreflang, href)` alternates: every published locale, then x-default (the source URL).
    pub fn alternates(&self, path: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .bases
            .iter()
            .filter(|(l, _)| self.advertised(l))
            .filter_map(|(l, _)| Some((l.clone(), self.url_for(l, path)?)))
            .collect();
        if let Some(x) = self.url_for(&self.source_locale, path) {
            out.push(("x-default".into(), x));
        }
        out
    }

    /// Source-terms path of an absolute or root-relative URL that belongs to the site (any locale's base),
    /// `None` for external links, protocol-relative, fragments, `mailto:`… and static assets.
    pub fn site_path(&self, href: &str) -> Option<String> {
        let href = href.trim();
        let path = if href.starts_with('/') && !href.starts_with("//") {
            // Root-relative: on a subdirectory page it may already carry a locale prefix.
            let mut p = href.to_string();
            for (_, b) in &self.bases {
                if let Some(prefix) = url_path(b).filter(|x| !x.is_empty())
                    && let Some(rest) = strip_dir(href, prefix)
                {
                    p = rest.to_string();
                }
            }
            p
        } else {
            // Longest base first: `https://x.app/es` before `https://x.app`.
            let mut bases: Vec<&str> = self.bases.iter().map(|(_, b)| b.as_str()).collect();
            bases.sort_by_key(|b| std::cmp::Reverse(b.len()));
            let b = bases.into_iter().find_map(|b| strip_dir(href, b))?;
            b.to_string()
        };
        let path = if path.is_empty() { "/".into() } else { path };
        (!is_asset(&path)).then_some(path)
    }

    /// Internal link rewritten for the served locale: root-relative stays root-relative on the same host
    /// (subdomain mode) or gains the locale prefix (subdirectory mode); absolute links to the site switch base.
    pub fn localize_href(&self, href: &str) -> Option<String> {
        let path = localized_path(self.slugs, &self.locale, &self.site_path(href)?);
        let base = self.base(&self.locale)?;
        if href.trim().starts_with('/') {
            // Same host as the page being served: keep it relative.
            let prefix = url_path(base).unwrap_or("");
            let page_host_prefix = url_path(self.source_base()).unwrap_or("");
            let same_host = host_of(base) == host_of(self.source_base());
            return Some(if same_host {
                format!(
                    "{prefix}{}",
                    strip_dir(&path, page_host_prefix).unwrap_or(&path)
                )
            } else {
                path
            });
        }
        Some(format!("{base}{path}"))
    }
}

/// `<html lang dir>` direction.
pub fn dir(locale: &str) -> &'static str {
    let base = locale.split(['-', '_']).next().unwrap_or(locale);
    if matches!(
        base,
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ckb"
    ) {
        "rtl"
    } else {
        "ltr"
    }
}

/// Response headers the edge sets on a translated page (`Content-Language`, `Link` alternates for non-HTML
/// consumers are not needed: the head carries them). The proxy also drops any origin `Content-Language`.
pub fn headers(ctx: &SeoCtx) -> Vec<(&'static str, String)> {
    vec![("content-language", ctx.locale.clone())]
}

/// The `<link>` tags glot adds to `<head>` (canonical first); also offered as a snippet for origins that are not
/// proxied (subdomain mode: the source domain must list the translated pages too, or reciprocity fails).
pub fn head_tags(ctx: &SeoCtx, canonical_path: &str) -> String {
    let mut s = String::new();
    if !ctx.advertised(&ctx.locale) {
        // Not complete in this language yet: a duplicate of the source page, said so, and out of the cluster.
        if let Some(c) = ctx.url_for(&ctx.source_locale, canonical_path) {
            s.push_str(&format!(r#"<link rel="canonical" href="{}">"#, esc(&c)));
        }
        return s;
    }
    if let Some(c) = ctx.url_for(&ctx.locale, canonical_path) {
        s.push_str(&format!(r#"<link rel="canonical" href="{}">"#, esc(&c)));
    }
    for (l, u) in ctx.alternates(canonical_path) {
        s.push_str(&format!(
            r#"<link rel="alternate" hreflang="{}" href="{}">"#,
            esc(&l),
            esc(&u)
        ));
    }
    s
}

type Handler<'h> = (Cow<'static, Selector>, ElementContentHandlers<'h>);

/// Per-document state of the SEO handlers (one per response).
#[derive(Default)]
pub struct SeoState {
    /// The origin canonical, in source terms; decides which URL canonical + alternates describe.
    canonical: RefCell<Option<String>>,
    json: RefCell<String>,
}

/// Placeholder appended at the end of `<head>`; [`finish`] swaps it for the tags once the origin canonical
/// (which may come after other head children) is known. lol_html end-tag handlers must be `'static`, hence this.
pub const HEAD_MARKER: &str = "<!--glot-seo-head-->";

/// lol_html handlers for the SEO rewrite; append them to the translation pass's settings, then run [`finish`]
/// on the output (when streaming: on the chunk holding `</head>`, the marker sits right before it).
pub fn handlers<'h>(ctx: &'h SeoCtx<'h>, st: &'h SeoState) -> Vec<Handler<'h>> {
    let managed = |l: &str| {
        l.eq_ignore_ascii_case("x-default")
            || ctx.bases.iter().any(|(b, _)| b.eq_ignore_ascii_case(l))
    };
    vec![
        element!("html", move |el| {
            el.set_attribute("lang", &ctx.locale)?;
            el.set_attribute("dir", dir(&ctx.locale))?;
            Ok(())
        }),
        element!("head", move |el| {
            el.append(HEAD_MARKER, ContentType::Html);
            Ok(())
        }),
        element!("link[rel]", move |el| {
            let rel = el
                .get_attribute("rel")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let rels: Vec<&str> = rel.split_whitespace().collect();
            if rels.contains(&"canonical") {
                if let Some(h) = el.get_attribute("href") {
                    st.canonical
                        .borrow_mut()
                        .get_or_insert(ctx.site_path(&h).unwrap_or_else(|| ctx.path.clone()));
                }
                el.remove();
            } else if rels.contains(&"alternate")
                && let Some(lang) = el.get_attribute("hreflang")
            {
                if managed(&lang) {
                    el.remove();
                } else if let Some(h) = el.get_attribute("href")
                    && h.starts_with('/')
                    && !h.starts_with("//")
                {
                    // An origin-managed language (`?lang=en`): absolute, so it still points at the source host.
                    el.set_attribute("href", &format!("{}{h}", origin_of(ctx.source_base())))?;
                }
            }
            Ok(())
        }),
        element!("meta[property]", move |el| {
            match el.get_attribute("property").as_deref() {
                Some("og:locale") => el.set_attribute("content", &og_locale(&ctx.locale))?,
                Some("og:url") => {
                    if let Some(u) = el
                        .get_attribute("content")
                        .and_then(|c| ctx.site_path(&c))
                        .and_then(|p| ctx.url_for(&ctx.locale, &p))
                    {
                        el.set_attribute("content", &u)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }),
        element!("a[href]", move |el| {
            if el.has_attribute("hreflang") || el.has_attribute("data-glot-skip") {
                return Ok(()); // language switcher links point to another locale on purpose
            }
            if let Some(h) = el.get_attribute("href").and_then(|h| ctx.localize_href(&h)) {
                el.set_attribute("href", &h)?;
            }
            Ok(())
        }),
        text!(r#"script[type="application/ld+json"]"#, move |t| {
            st.json.borrow_mut().push_str(t.as_str());
            if t.last_in_text_node() {
                let src = std::mem::take(&mut *st.json.borrow_mut());
                let out = jsonld(&src, ctx).unwrap_or(src);
                t.replace(&out, ContentType::Html);
            } else {
                t.remove();
            }
            Ok(())
        }),
    ]
}

/// Replaces [`HEAD_MARKER`] with canonical + alternates.
pub fn finish(html: &str, ctx: &SeoCtx, st: &SeoState) -> String {
    let c = st.canonical.borrow();
    html.replacen(
        HEAD_MARKER,
        &head_tags(ctx, c.as_deref().unwrap_or(&ctx.path)),
        1,
    )
}

/// One-shot SEO rewrite of a whole document (tests, prerender, snapshots); the origin HTML on error.
pub fn rewrite(html: &str, ctx: &SeoCtx) -> String {
    let st = SeoState::default();
    let settings = handlers(ctx, &st)
        .into_iter()
        .fold(lol_html::RewriteStrSettings::new(), |s, h| {
            s.append_element_content_handler(h)
        });
    match lol_html::rewrite_str(html, settings) {
        Ok(out) => finish(&out, ctx, &st),
        Err(_) => html.to_string(),
    }
}

/// What a crawler sees of a page's international SEO (raw HTML, no JavaScript) — the SEO report's input.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
pub struct PageSeo {
    pub lang: Option<String>,
    pub dir: Option<String>,
    /// Every `<link rel=canonical>` href (more than one is an error).
    pub canonicals: Vec<String>,
    /// `(hreflang, href)` of `<link rel=alternate hreflang>`.
    pub alternates: Vec<(String, String)>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub og_title: Option<String>,
    pub og_locale: Option<String>,
    /// `<meta name=robots|googlebot>` contents.
    pub robots: Vec<String>,
    pub jsonld: usize,
    /// `inLanguage` values found in JSON-LD.
    pub jsonld_languages: Vec<String>,
}

/// Parses the SEO-relevant head of a page.
pub fn inspect(html: &str) -> PageSeo {
    let out = RefCell::new(PageSeo::default());
    let title = RefCell::new(String::new());
    let json = RefCell::new(String::new());
    let settings = lol_html::RewriteStrSettings::new()
        .append_element_content_handler(element!("html", |el| {
            let mut o = out.borrow_mut();
            o.lang = el.get_attribute("lang");
            o.dir = el.get_attribute("dir");
            Ok(())
        }))
        .append_element_content_handler(element!("link[rel][href]", |el| {
            let rel = el
                .get_attribute("rel")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let href = el.get_attribute("href").unwrap_or_default();
            let mut o = out.borrow_mut();
            if rel.split_whitespace().any(|r| r == "canonical") {
                o.canonicals.push(href);
            } else if rel.split_whitespace().any(|r| r == "alternate")
                && let Some(l) = el.get_attribute("hreflang")
            {
                o.alternates.push((l, href));
            }
            Ok(())
        }))
        .append_element_content_handler(element!("meta[content]", |el| {
            let c = el.get_attribute("content").unwrap_or_default();
            let name = el
                .get_attribute("name")
                .or_else(|| el.get_attribute("property"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let mut o = out.borrow_mut();
            match name.as_str() {
                "description" => o.description = Some(c),
                "og:title" => o.og_title = Some(c),
                "og:locale" => o.og_locale = Some(c),
                "robots" | "googlebot" => o.robots.push(c.to_ascii_lowercase()),
                _ => {}
            }
            Ok(())
        }))
        .append_element_content_handler(text!("title", |t| {
            title.borrow_mut().push_str(t.as_str());
            Ok(())
        }))
        .append_element_content_handler(text!(r#"script[type="application/ld+json"]"#, |t| {
            json.borrow_mut().push_str(t.as_str());
            if t.last_in_text_node() {
                let src = std::mem::take(&mut *json.borrow_mut());
                let mut o = out.borrow_mut();
                o.jsonld += 1;
                if let Ok(v) = serde_json::from_str::<Value>(src.trim()) {
                    collect_languages(&v, &mut o.jsonld_languages);
                }
            }
            Ok(())
        }));
    let _ = lol_html::rewrite_str(html, settings);
    let mut o = out.into_inner();
    let t = crate::edge::tm::normalize(&title.into_inner());
    o.title = (!t.is_empty()).then(|| decode(&t));
    o
}

/// The texts of a page's JSON-LD that are worth translating on their own (sentences: descriptions, headlines,
/// FAQ answers), normalized. Short values (names, brands) are only translated when the page shows the same text.
pub fn jsonld_texts(html: &str) -> Vec<String> {
    let out = RefCell::new(Vec::new());
    let collect = |t: &str| {
        let n = crate::edge::tm::normalize(t);
        if jsonld_sentence(&n) && !out.borrow().contains(&n) {
            out.borrow_mut().push(n);
        }
        None
    };
    let ctx = SeoCtx {
        source_locale: String::new(),
        locale: String::new(),
        bases: vec![],
        path: "/".into(),
        tr: &collect,
        page_locales: None,
        slugs: None,
    };
    rewrite(html, &ctx);
    out.into_inner()
}

/// A JSON-LD value glot may learn by itself: at least three words, of a reasonable size.
pub fn jsonld_sentence(normalized: &str) -> bool {
    normalized.len() <= 2000 && normalized.split_whitespace().count() >= 3
}

/// Every `<a href>` of a page, as written (the pre-warm follows the internal ones).
pub fn links(html: &str) -> Vec<String> {
    let out = RefCell::new(Vec::new());
    let settings = lol_html::RewriteStrSettings::new().append_element_content_handler(element!(
        "a[href]",
        |el| {
            if !el
                .get_attribute("rel")
                .is_some_and(|r| r.to_ascii_lowercase().contains("nofollow"))
            {
                out.borrow_mut()
                    .push(el.get_attribute("href").unwrap_or_default());
            }
            Ok(())
        }
    ));
    let _ = lol_html::rewrite_str(html, settings);
    out.into_inner()
}

fn collect_languages(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(l)) = m.get("inLanguage")
                && !out.contains(l)
            {
                out.push(l.clone());
            }
            m.values().for_each(|v| collect_languages(v, out));
        }
        Value::Array(a) => a.iter().for_each(|v| collect_languages(v, out)),
        _ => {}
    }
}

/// Minimal entity decoding of a title's raw text.
fn decode(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Translates JSON-LD string values (`name`, `description`…), localizes its site URLs, sets `inLanguage`.
fn jsonld(src: &str, ctx: &SeoCtx) -> Option<String> {
    let mut v: Value = serde_json::from_str(src.trim()).ok()?;
    walk(&mut v, ctx, None);
    // `</script>` inside a string would end the element.
    Some(serde_json::to_string(&v).ok()?.replace("</", "<\\/"))
}

const LD_TEXT: &[&str] = &[
    "name",
    "alternateName",
    "description",
    "headline",
    "alternativeHeadline",
    "abstract",
    "text",
    "caption",
    "slogan",
    "disambiguatingDescription",
    "articleBody",
    "reviewBody",
    "keywords",
];
const LD_URL: &[&str] = &["url", "@id", "mainEntityOfPage", "item", "target", "sameAs"];

fn walk(v: &mut Value, ctx: &SeoCtx, key: Option<&str>) {
    match v {
        Value::Object(m) => {
            if m.contains_key("inLanguage") {
                m.insert("inLanguage".into(), Value::String(ctx.locale.clone()));
            }
            for (k, v) in m.iter_mut() {
                if k != "inLanguage" {
                    walk(v, ctx, Some(k));
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| walk(v, ctx, key)),
        Value::String(s) => match key {
            Some(k) if LD_TEXT.contains(&k) => {
                if let Some(t) = (ctx.tr)(s) {
                    *s = t;
                }
            }
            Some(k) if LD_URL.contains(&k) => {
                if let Some(u) = ctx.site_path(s).and_then(|p| ctx.url_for(&ctx.locale, &p)) {
                    *s = u;
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// `https://x.app/es` → `/es`; `https://x.app` → ``; None if not absolute.
fn url_path(base: &str) -> Option<&str> {
    let rest = base.split_once("://")?.1;
    Some(
        rest.find('/')
            .map_or("", |i| &rest[i..])
            .trim_end_matches('/'),
    )
}

fn host_of(base: &str) -> &str {
    let rest = base.split_once("://").map_or(base, |x| x.1);
    rest.split('/').next().unwrap_or("")
}

fn origin_of(base: &str) -> &str {
    let p = url_path(base).unwrap_or("");
    &base[..base.len() - p.len()]
}

/// `href` minus the directory prefix `dir`, only at a path boundary (`/es` matches `/es`, `/es/x`, `/es?q`, not `/estate`).
fn strip_dir<'s>(href: &'s str, dir: &str) -> Option<&'s str> {
    let rest = href.strip_prefix(dir.trim_end_matches('/'))?;
    (rest.is_empty() || rest.starts_with(['/', '?', '#'])).then_some(rest)
}

/// Static files keep their URL (`/logo.png`, `/_next/…`): only pages are localized.
// ponytail: extension heuristic; a configured exclude list if a site serves pages with odd extensions.
fn is_asset(path: &str) -> bool {
    let p = path.split(['?', '#']).next().unwrap_or("");
    if p.starts_with("/_next/") || p.starts_with("/api/") || p.starts_with("/cdn-cgi/") {
        return true;
    }
    let last = p.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, ext)) => !matches!(
            ext.to_ascii_lowercase().as_str(),
            "html" | "htm" | "php" | "aspx"
        ),
        None => false,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(s: &str) -> Option<String> {
        match s {
            "Clip videos" => Some("Recorta vídeos".into()),
            "Pricing" => Some("Precios".into()),
            _ => None,
        }
    }

    fn ctx<'a>(locale: &str, bases: &[(&str, &str)], path: &str) -> SeoCtx<'a> {
        SeoCtx {
            source_locale: "fr".into(),
            locale: locale.into(),
            bases: bases
                .iter()
                .map(|(l, b)| (l.to_string(), b.to_string()))
                .collect(),
            path: path.into(),
            tr: &tr,
            page_locales: None,
            slugs: None,
        }
    }

    const SUB: &[(&str, &str)] = &[
        ("fr", "https://cliponit.app"),
        ("es", "https://es.cliponit.app"),
    ];
    const DIR: &[(&str, &str)] = &[
        ("fr", "https://cliponit.app"),
        ("es", "https://cliponit.app/es"),
        ("ar", "https://cliponit.app/ar"),
    ];

    const PAGE: &str = r##"<!doctype html><html lang="fr"><head><title>Tarifs</title>
<link rel="canonical" href="https://cliponit.app/pricing?utm=x">
<link rel="alternate" hreflang="fr" href="https://cliponit.app/pricing">
<link rel="alternate" hreflang="en" href="/pricing?lang=en">
<link rel="alternate" hreflang="x-default" href="https://cliponit.app/pricing">
<meta property="og:locale" content="fr_FR"><meta property="og:url" content="https://cliponit.app/pricing">
<script type="application/ld+json">{"@type":"Product","name":"Clip videos","url":"https://cliponit.app/pricing","inLanguage":"fr","brand":"ClipOnIt"}</script>
</head><body><a href="/pricing">Tarifs</a> <a href="https://cliponit.app/">Accueil</a> <a href="/logo.png">l</a>
<a href="https://other.io/x">x</a> <a href="#top">t</a> <a href="/?lang=en" hreflang="en">EN</a> <a href="/estate">e</a></body></html>"##;

    #[test]
    fn subdomain_page() {
        let c = ctx("es", SUB, "/pricing");
        let out = rewrite(PAGE, &c);
        assert!(out.contains(r#"<html lang="es" dir="ltr">"#), "{out}");
        // One canonical: the origin's, localized and without the tracking query… as the origin wrote it.
        assert_eq!(out.matches(r#"rel="canonical""#).count(), 1, "{out}");
        assert!(
            out.contains(r#"<link rel="canonical" href="https://es.cliponit.app/pricing?utm=x">"#),
            "{out}"
        );
        assert!(out.contains(
            r#"<link rel="alternate" hreflang="fr" href="https://cliponit.app/pricing?utm=x">"#
        ));
        assert!(out.contains(
            r#"<link rel="alternate" hreflang="es" href="https://es.cliponit.app/pricing?utm=x">"#
        ));
        assert!(out.contains(r#"hreflang="x-default" href="https://cliponit.app/pricing?utm=x""#));
        assert_eq!(out.matches(r#"hreflang="x-default""#).count(), 1);
        // Origin-managed English kept, absolute so it still points at the source host.
        assert!(
            out.contains(r#"hreflang="en" href="https://cliponit.app/pricing?lang=en""#),
            "{out}"
        );
        assert!(out.contains(r#"<meta property="og:locale" content="es_ES">"#));
        assert!(
            out.contains(r#"<meta property="og:url" content="https://es.cliponit.app/pricing">"#)
        );
        // JSON-LD: text via the TM, URL localized, inLanguage, brand untouched.
        assert!(out.contains(r#""name":"Recorta vídeos""#), "{out}");
        assert!(out.contains(r#""url":"https://es.cliponit.app/pricing""#));
        assert!(out.contains(r#""inLanguage":"es""#));
        assert!(out.contains(r#""brand":"ClipOnIt""#));
        // Links: same host stays relative, absolute site links switch host, assets/external/fragments untouched.
        assert!(out.contains(r#"<a href="/pricing">"#));
        assert!(out.contains(r#"<a href="https://es.cliponit.app/">"#));
        assert!(out.contains(r#"<a href="/logo.png">"#));
        assert!(out.contains(r#"<a href="https://other.io/x">"#));
        assert!(out.contains(r##"<a href="#top">"##));
        assert!(out.contains(r#"<a href="/?lang=en" hreflang="en">"#));
        // Canonical + alternates sit in <head>.
        let head_end = out.find("</head>").unwrap();
        assert!(out.find(r#"rel="canonical""#).unwrap() < head_end);
    }

    #[test]
    fn subdirectory_page_and_rtl() {
        let c = ctx("ar", DIR, "/pricing");
        let out = rewrite(PAGE, &c);
        assert!(out.contains(r#"<html lang="ar" dir="rtl">"#));
        assert!(out.contains(r#"<a href="/ar/pricing">"#), "{out}");
        assert!(
            out.contains(r#"<a href="https://cliponit.app/ar/">"#),
            "{out}"
        );
        assert!(out.contains(r#"<a href="/ar/estate">"#), "{out}");
        assert!(out.contains(r#"hreflang="es" href="https://cliponit.app/es/pricing?utm=x""#));
        assert!(out.contains(r#"rel="canonical" href="https://cliponit.app/ar/pricing?utm=x""#));
        // Already-localized links are not prefixed twice.
        let c = ctx("es", DIR, "/");
        assert_eq!(
            c.localize_href("/es/pricing").as_deref(),
            Some("/es/pricing")
        );
        assert_eq!(
            c.localize_href("/ar/pricing").as_deref(),
            Some("/es/pricing")
        );
        assert_eq!(
            c.localize_href("https://cliponit.app/es").as_deref(),
            Some("https://cliponit.app/es/")
        );
        assert_eq!(c.site_path("/estate").as_deref(), Some("/estate"));
        assert_eq!(c.localize_href("mailto:a@b.c"), None);
        assert_eq!(c.localize_href("//cdn.x/y"), None);
    }

    #[test]
    fn no_canonical_self_reference_and_source_page() {
        let html = r#"<html><head><title>x</title></head><body></body></html>"#;
        let out = rewrite(html, &ctx("es", SUB, "/a"));
        assert!(
            out.contains(r#"<link rel="canonical" href="https://es.cliponit.app/a">"#),
            "{out}"
        );
        // The source page through the proxy (subdirectory mode) lists the translations too: reciprocity.
        let out = rewrite(html, &ctx("fr", DIR, "/a"));
        assert!(
            out.contains(r#"<link rel="canonical" href="https://cliponit.app/a">"#),
            "{out}"
        );
        assert!(out.contains(r#"hreflang="ar" href="https://cliponit.app/ar/a""#));
        assert!(!out.contains(HEAD_MARKER));
        // Broken JSON-LD is left as is.
        let out = rewrite(
            r#"<script type="application/ld+json">{oops</script>"#,
            &ctx("es", SUB, "/"),
        );
        assert!(out.contains("{oops"));
        assert_eq!(
            headers(&ctx("es", SUB, "/")),
            [("content-language", "es".to_string())]
        );
    }

    #[test]
    fn inspect_reads_what_rewrite_wrote() {
        let out = rewrite(PAGE, &ctx("es", SUB, "/pricing"));
        let s = inspect(&out);
        assert_eq!(s.lang.as_deref(), Some("es"));
        assert_eq!(s.dir.as_deref(), Some("ltr"));
        assert_eq!(s.canonicals, ["https://es.cliponit.app/pricing?utm=x"]);
        assert_eq!(s.title.as_deref(), Some("Tarifs"));
        assert_eq!(s.og_locale.as_deref(), Some("es_ES"));
        assert_eq!(s.jsonld, 1);
        assert_eq!(s.jsonld_languages, ["es"]);
        let langs: Vec<&str> = s.alternates.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(langs, ["en", "fr", "es", "x-default"]);
        let s =
            inspect(r#"<meta name="robots" content="NOINDEX, follow"><title>A &amp; B</title>"#);
        assert_eq!(s.robots, ["noindex, follow"]);
        assert_eq!(s.title.as_deref(), Some("A & B"));
    }

    #[test]
    fn incomplete_page_is_a_duplicate_of_the_source() {
        let html = r#"<html><head><link rel="canonical" href="/a"></head><body></body></html>"#;
        // es page not complete: canonical to the source, out of the cluster.
        let mut c = ctx("es", DIR, "/a");
        c.page_locales = Some(vec!["ar".into()]);
        let out = rewrite(html, &c);
        assert!(
            out.contains(r#"<link rel="canonical" href="https://cliponit.app/a">"#),
            "{out}"
        );
        assert!(!out.contains("hreflang"), "{out}");
        // The complete ar page does not list es.
        c.locale = "ar".into();
        let out = rewrite(html, &c);
        assert!(out.contains(r#"rel="canonical" href="https://cliponit.app/ar/a""#));
        assert!(out.contains(r#"hreflang="ar""#) && out.contains(r#"hreflang="fr""#));
        assert!(!out.contains(r#"hreflang="es""#), "{out}");
        // A locale that is not published at all: same duplicate statement (it used to get no canonical).
        let out = rewrite(html, &ctx("de", DIR, "/a"));
        assert!(
            out.contains(r#"<link rel="canonical" href="https://cliponit.app/a">"#),
            "{out}"
        );
    }

    #[test]
    fn translated_slugs() {
        let mut slugs = Slugs::new();
        slugs
            .entry("es".into())
            .or_default()
            .insert("/pricing".into(), "/precios".into());
        let mut c = ctx("es", SUB, "/pricing");
        c.slugs = Some(&slugs);
        let out = rewrite(PAGE, &c);
        assert!(
            out.contains(r#"rel="canonical" href="https://es.cliponit.app/precios?utm=x""#),
            "{out}"
        );
        // Reciprocal: the source keeps its path, es gets its slug, on every page of the cluster.
        assert!(out.contains(r#"hreflang="es" href="https://es.cliponit.app/precios?utm=x""#));
        assert!(out.contains(r#"hreflang="fr" href="https://cliponit.app/pricing?utm=x""#));
        assert!(out.contains(r#"<a href="/precios">"#), "{out}");
        assert!(out.contains(r#""url":"https://es.cliponit.app/precios""#));
        assert!(out.contains(r#"property="og:url" content="https://es.cliponit.app/precios""#));
        c.locale = "fr".into();
        assert!(
            rewrite(PAGE, &c).contains(r#"hreflang="es" href="https://es.cliponit.app/precios"#)
        );
        assert_eq!(
            localized_path(Some(&slugs), "es", "/pricing/#a"),
            "/precios/#a"
        );
        assert_eq!(
            localized_path(Some(&slugs), "es", "/pricing-x"),
            "/pricing-x"
        );
        assert_eq!(localized_path(Some(&slugs), "de", "/pricing"), "/pricing");
        assert_eq!(
            [
                og_locale("es"),
                og_locale("pt-br"),
                og_locale("en"),
                og_locale("ja")
            ],
            ["es_ES", "pt_BR", "en_US", "ja_JP"]
        );
    }
}
