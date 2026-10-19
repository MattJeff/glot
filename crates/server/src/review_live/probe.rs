//! Connection probe for the Review page: fetches the project's own site URL (Settings → Your site) server-side and
//! reports why the live frame may not connect — framing headers (X-Frame-Options / CSP frame-ancestors) and
//! whether the review widget script tag is in the HTML, plus how the site switches language (hreflang alternates →
//! a glot URL pattern, so Settings can suggest it). SSRF-safe: only the project's base_url (and up to
//! `MAX_HOPS` redirects), public IPs only (resolved once, then pinned), short timeout, capped body.
use axum::{Json, extract::State};
use reqwest::{Url, header};
use serde::Serialize;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use super::model::Ctx;
use crate::{ApiResult, AppError, AppState};

const TIMEOUT: Duration = Duration::from_secs(6);
const MAX_BODY: usize = 512 * 1024;
const MAX_HOPS: usize = 3;

#[derive(Serialize, Debug, Default)]
pub struct Probe {
    /// Final URL fetched (after redirects).
    pub url: String,
    /// HTTP status, or None when the site could not be fetched.
    pub status: Option<u16>,
    /// `private` (local/private address: can't be checked from glot's servers), `unreachable`, `timeout`.
    pub error: Option<&'static str>,
    pub x_frame_options: Option<String>,
    pub frame_ancestors: Option<String>,
    /// `allowed`, `blocked`, or `unknown` (not fetched).
    pub framing: &'static str,
    /// A `review.global.js` script tag is in the HTML.
    pub widget: bool,
    pub widget_project: Option<String>,
    pub widget_app_origin: Option<String>,
    /// The feedback widget (end-user reports) is there instead: it never answers the Review frame.
    pub feedback_widget: bool,
    /// How the site switches language, from its hreflang alternates (HTML or `Link` header). None when not fetched.
    pub locale_scheme: Option<LocaleScheme>,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct LocaleScheme {
    /// `query` (`?lang=en`), `prefix` (`/en/…`), `subdomain` (`en.example.com`: glot can't build those URLs), `unknown`.
    pub kind: &'static str,
    /// glot URL pattern to use; None for `subdomain` / `unknown`.
    pub pattern: Option<String>,
    /// hreflang values, x-default excluded.
    pub locales: Vec<String>,
    /// The site's apparent default locale: the one without a marker, else x-default's, else `<html lang>`.
    pub default_locale: Option<String>,
    /// The default locale has no `?lang=` / `/xx` marker.
    pub default_unprefixed: bool,
    pub html_lang: Option<String>,
}

pub async fn probe(State(state): State<AppState>, ctx: Ctx) -> ApiResult<Json<Probe>> {
    if ctx.widget {
        return Err(AppError::not_found("route not found"));
    }
    let base: Option<String> = sqlx::query_scalar(
        "SELECT base_url FROM preview_settings WHERE project_id = $1 AND base_url IS NOT NULL",
    )
    .bind(ctx.project.id)
    .fetch_optional(&state.db)
    .await?;
    let base = base.ok_or_else(|| AppError::bad_request("no site URL set"))?;
    let url = Url::parse(&base).map_err(|_| AppError::bad_request("invalid site URL"))?;
    Ok(Json(run(url, super::web_origin()).await))
}

async fn run(mut url: Url, app_origin: &str) -> Probe {
    let mut out = Probe {
        framing: "unknown",
        ..Default::default()
    };
    for _ in 0..=MAX_HOPS {
        out.url = url.to_string();
        let resp = match fetch(&url).await {
            Ok(r) => r,
            Err(e) => {
                out.error = Some(e);
                return out;
            }
        };
        if resp.status().is_redirection()
            && let Some(next) = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .and_then(|l| url.join(l).ok())
        {
            url = next;
            continue;
        }
        out.status = Some(resp.status().as_u16());
        let h = resp.headers();
        let all = |n: &str| -> Vec<String> {
            h.get_all(n)
                .iter()
                .filter_map(|v| v.to_str().ok().map(str::to_string))
                .collect()
        };
        let xfo = all("x-frame-options");
        let fa: Vec<String> = all("content-security-policy")
            .iter()
            .filter_map(|c| frame_ancestors(c))
            .collect();
        out.x_frame_options = (!xfo.is_empty()).then(|| xfo.join(", "));
        out.frame_ancestors = (!fa.is_empty()).then(|| fa.join(" ; "));
        let site = url.origin().ascii_serialization();
        out.framing = if framing_allowed(&xfo, &fa, &site, app_origin) {
            "allowed"
        } else {
            "blocked"
        };
        let links = link_tags(&all("link"));
        let html = body(resp).await;
        out.locale_scheme = Some(locale_scheme(&format!("{links}{html}"), &url));
        let w = widget(&html);
        out.widget = w.found;
        out.widget_project = w.project;
        out.widget_app_origin = w.app_origin;
        out.feedback_widget = html.contains("feedback.global.js");
        return out;
    }
    out.error = Some("unreachable");
    out
}

/// One request, no redirect following, to a host that resolves only to public addresses (pinned: no rebinding).
async fn fetch(url: &Url) -> Result<reqwest::Response, &'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("unreachable");
    }
    let host = url.host_str().ok_or("unreachable")?.to_string();
    let port = url.port_or_known_default().ok_or("unreachable")?;
    let addrs: Vec<SocketAddr> = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::time::timeout(TIMEOUT, tokio::net::lookup_host((host.as_str(), port)))
            .await
            .map_err(|_| "timeout")?
            .map_err(|_| "unreachable")?
            .collect(),
    };
    if addrs.is_empty() {
        return Err("unreachable");
    }
    if addrs.iter().any(|a| !is_public(a.ip())) {
        return Err("private");
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .resolve_to_addrs(&host, &addrs)
        .user_agent("glot-review-probe/1")
        .build()
        .map_err(|_| "unreachable")?;
    client.get(url.clone()).send().await.map_err(|e| {
        if e.is_timeout() {
            "timeout"
        } else {
            "unreachable"
        }
    })
}

async fn body(mut resp: reqwest::Response) -> String {
    let mut buf = Vec::new();
    while let Ok(Some(c)) = tokio::time::timeout(TIMEOUT, resp.chunk())
        .await
        .unwrap_or(Ok(None))
    {
        buf.extend_from_slice(&c);
        if buf.len() >= MAX_BODY {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64/10
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
                || o[0] >= 240)
        }
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return is_public(IpAddr::V4(m));
            }
            let s = v.segments()[0];
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s & 0xfe00) == 0xfc00 // unique local
                || (s & 0xffc0) == 0xfe80 // link local
                || s == 0x2001 && v.segments()[1] == 0xdb8)
        }
    }
}

/// The `frame-ancestors` directive's source list in one CSP header value.
fn frame_ancestors(csp: &str) -> Option<String> {
    csp.split(';').find_map(|d| {
        let d = d.trim();
        let (name, rest) = d.split_once(char::is_whitespace).unwrap_or((d, ""));
        name.eq_ignore_ascii_case("frame-ancestors")
            .then(|| rest.trim().to_string())
    })
}

/// Can `app_origin` frame a page of `site` with these headers? CSP frame-ancestors (every policy must allow)
/// overrides X-Frame-Options, as in browsers.
fn framing_allowed(xfo: &[String], fa: &[String], site: &str, app_origin: &str) -> bool {
    if !fa.is_empty() {
        return fa
            .iter()
            .all(|list| source_list_allows(list, site, app_origin));
    }
    !xfo.iter().any(|v| {
        v.split(',').any(|v| {
            let v = v.trim();
            v.eq_ignore_ascii_case("deny")
                || (v.eq_ignore_ascii_case("sameorigin") && site != app_origin)
        })
    })
}

fn source_list_allows(list: &str, site: &str, app: &str) -> bool {
    let Ok(app_url) = Url::parse(app) else {
        return false;
    };
    let (scheme, host) = (app_url.scheme(), app_url.host_str().unwrap_or(""));
    let port = app_url.port_or_known_default();
    list.split_whitespace().any(|src| {
        let s = src.to_ascii_lowercase();
        match s.as_str() {
            "'none'" => false,
            "'self'" => site == app,
            "*" => true,
            _ if s.ends_with(':') && !s.contains('/') => s.trim_end_matches(':') == scheme,
            _ => {
                let (sch, rest) = match s.split_once("://") {
                    Some((a, b)) => (Some(a), b),
                    None => (None, s.as_str()),
                };
                // Scheme-less sources match the same scheme or its secure upgrade.
                if sch.is_some_and(|x| x != scheme) || (sch.is_none() && scheme != "https") {
                    return false;
                }
                let rest = rest.split('/').next().unwrap_or("");
                let (h, p) = match rest.rsplit_once(':') {
                    Some((h, p)) => (h, Some(p)),
                    None => (rest, None),
                };
                let port_ok = match p {
                    None => true,
                    Some("*") => true,
                    Some(p) => p.parse::<u16>().ok() == port,
                };
                let host_ok = match h.strip_prefix("*.") {
                    Some(suffix) => host.ends_with(&format!(".{suffix}")),
                    None => h == host,
                };
                host_ok && port_ok
            }
        }
    })
}

struct Widget {
    found: bool,
    project: Option<String>,
    app_origin: Option<String>,
}

/// The first `<script … review.global.js …>` tag and its data attributes.
fn widget(html: &str) -> Widget {
    let lower = html.to_ascii_lowercase();
    let Some(at) = lower.find("review.global.js") else {
        return Widget {
            found: false,
            project: None,
            app_origin: None,
        };
    };
    let start = lower[..at].rfind("<script").unwrap_or(at);
    let end = lower[at..].find('>').map_or(lower.len(), |e| at + e);
    let tag = &html[start..end];
    Widget {
        found: true,
        project: attr(tag, "data-project"),
        app_origin: attr(tag, "data-app-origin"),
    }
}

/// An attribute's value in one start tag (`name=` preceded by whitespace; quoted or bare).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pat = format!("{name}=");
    let (i, _) = lower
        .match_indices(&pat)
        .find(|(i, _)| *i > 0 && lower.as_bytes()[i - 1].is_ascii_whitespace())?;
    let v = &tag[i + pat.len()..];
    let q = v.chars().next()?;
    let v = if q == '"' || q == '\'' {
        v[1..].split(q).next()?
    } else {
        v.split(|c: char| c.is_whitespace() || c == '>').next()?
    };
    Some(v.replace("&amp;", "&"))
}

/// `Link: <url>; rel="alternate"; hreflang="de"` headers (next-intl sends these) as `<link>` tags.
fn link_tags(headers: &[String]) -> String {
    headers
        .iter()
        .flat_map(|h| h.split(','))
        .filter_map(|part| {
            let (url, params) = part.trim().strip_prefix('<')?.split_once('>')?;
            Some(format!(
                "<link {} href=\"{url}\">",
                params.replace(';', " ")
            ))
        })
        .collect()
}

/// Infers the site's locale URL scheme from its hreflang alternates and `<html lang>`.
fn locale_scheme(html: &str, page: &Url) -> LocaleScheme {
    let lower = html.to_ascii_lowercase();
    let tag_at = |s: usize| &html[s..lower[s..].find('>').map_or(lower.len(), |e| s + e)];
    let html_lang = lower
        .find("<html")
        .and_then(|s| attr(tag_at(s), "lang"))
        .filter(|l| !l.is_empty());
    let mut alts: Vec<(String, Url)> = Vec::new();
    let mut x_default = None;
    for (s, _) in lower.match_indices("<link") {
        let tag = tag_at(s);
        let alternate = attr(tag, "rel").is_some_and(|r| {
            r.split_whitespace()
                .any(|r| r.eq_ignore_ascii_case("alternate"))
        });
        let (Some(lang), Some(href)) = (attr(tag, "hreflang"), attr(tag, "href")) else {
            continue;
        };
        let Ok(u) = page.join(&href) else { continue };
        if !alternate || lang.is_empty() {
            continue;
        } else if lang.eq_ignore_ascii_case("x-default") {
            x_default.get_or_insert(u);
        } else if !alts.iter().any(|(l, _)| l.eq_ignore_ascii_case(&lang)) {
            alts.push((lang, u));
        }
    }
    // A marker matches the full tag or its primary subtag (hreflang `en-US` on `/en/`).
    let is = |v: &str, l: &str| {
        v.eq_ignore_ascii_case(l)
            || l.split('-')
                .next()
                .is_some_and(|p| v.eq_ignore_ascii_case(p))
    };
    let param = |l: &str, u: &Url| {
        u.query_pairs()
            .find(|(_, v)| is(v, l))
            .map(|(k, _)| k.into_owned())
    };
    let prefix = |l: &str, u: &Url| {
        u.path_segments()
            .and_then(|mut s| s.next())
            .is_some_and(|s| is(s, l))
    };
    let sub = |l: &str, u: &Url| {
        u.host_str()
            .and_then(|h| h.split('.').next())
            .is_some_and(|h| is(h, l))
    };
    let count = |f: &dyn Fn(&str, &Url) -> bool| alts.iter().filter(|(l, u)| f(l, u)).count();
    let counts = [
        ("query", count(&|l, u| param(l, u).is_some())),
        ("prefix", count(&prefix)),
        ("subdomain", count(&sub)),
    ];
    // ponytail: majority vote; a site mixing schemes gets the most common one.
    let (kind, n) = counts
        .iter()
        .fold(("unknown", 0), |a, &b| if b.1 > a.1 { b } else { a });
    let marked = |l: &str, u: &Url| match kind {
        "query" => param(l, u).is_some(),
        "prefix" => prefix(l, u),
        "subdomain" => sub(l, u),
        _ => false,
    };
    let unmarked: Vec<&String> = alts
        .iter()
        .filter(|(l, u)| !marked(l, u))
        .map(|(l, _)| l)
        .collect();
    let pattern = match kind {
        "query" => alts
            .iter()
            .find_map(|(l, u)| param(l, u))
            .map(|k| format!("{{route}}?{k}={{locale}}")),
        "prefix" => Some("/{locale}{route}".to_string()),
        _ => None,
    };
    let default_locale = match unmarked.as_slice() {
        [one] if n > 0 => Some((*one).clone()),
        _ => x_default
            .as_ref()
            .and_then(|x| alts.iter().find(|(_, u)| u == x))
            .map(|(l, _)| l.clone())
            .or_else(|| html_lang.clone()),
    };
    let default_unprefixed = n > 0 && !unmarked.is_empty();
    LocaleScheme {
        kind,
        pattern,
        locales: alts.into_iter().map(|(l, _)| l).collect(),
        default_locale,
        default_unprefixed,
        html_lang,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "https://app.glotlang.com";
    const SITE: &str = "https://cliponit.app";
    fn ok(xfo: &[&str], csp: &[&str]) -> bool {
        let xfo: Vec<String> = xfo.iter().map(|s| s.to_string()).collect();
        let fa: Vec<String> = csp.iter().filter_map(|c| frame_ancestors(c)).collect();
        framing_allowed(&xfo, &fa, SITE, APP)
    }

    #[test]
    fn framing_rules() {
        assert!(ok(&[], &[]));
        assert!(!ok(&["DENY"], &[]));
        assert!(!ok(&["SAMEORIGIN"], &[]));
        // The founder's fix: CSP overrides XFO.
        assert!(ok(
            &["DENY"],
            &["default-src 'self'; frame-ancestors 'self' https://app.glotlang.com"]
        ));
        assert!(!ok(&[], &["frame-ancestors 'self'"]));
        assert!(!ok(&[], &["frame-ancestors 'none'"]));
        assert!(ok(&[], &["frame-ancestors *"]));
        assert!(ok(&[], &["frame-ancestors https:"]));
        assert!(ok(&[], &["frame-ancestors https://*.glotlang.com"]));
        assert!(ok(&[], &["frame-ancestors app.glotlang.com"]));
        assert!(!ok(&[], &["frame-ancestors https://glotlang.com"]));
        assert!(!ok(&[], &["frame-ancestors http://app.glotlang.com"]));
        assert!(!ok(&[], &["frame-ancestors https://app.glotlang.com:8443"]));
        // Two policies: both must allow.
        assert!(!ok(&[], &["frame-ancestors *", "frame-ancestors 'self'"]));
        assert_eq!(frame_ancestors("script-src 'self'"), None);
    }

    #[test]
    fn widget_tag() {
        let w = widget(
            r#"<head><script src="https://app.glotlang.com/widget/review.global.js" data-project="clip" data-endpoint="https://app.glotlang.com" data-app-origin='https://app.glotlang.com' async></script>"#,
        );
        assert!(w.found);
        assert_eq!(w.project.as_deref(), Some("clip"));
        assert_eq!(w.app_origin.as_deref(), Some("https://app.glotlang.com"));
        let w = widget(r#"<script src="/feedback.global.js" data-key="k"></script>"#);
        assert!(!w.found);
    }

    fn scheme(html: &str, page: &str) -> LocaleScheme {
        locale_scheme(html, &Url::parse(page).unwrap())
    }

    #[test]
    fn locale_query_param() {
        // cliponit.app: French is bare and canonical, English is `?lang=en`.
        let s = scheme(
            r#"<!doctype html><html lang="fr"><head>
            <link rel="canonical" href="https://cliponit.app/">
            <link rel="alternate" hreflang="fr" href="https://cliponit.app/">
            <link rel="alternate" hreflang="en" href="https://cliponit.app/?lang=en">
            <link rel="alternate" hreflang="x-default" href="https://cliponit.app/">"#,
            "https://cliponit.app/",
        );
        assert_eq!(s.kind, "query");
        assert_eq!(s.pattern.as_deref(), Some("{route}?lang={locale}"));
        assert_eq!(s.locales, ["fr", "en"]);
        assert_eq!(s.default_locale.as_deref(), Some("fr"));
        assert!(s.default_unprefixed);
        assert_eq!(s.html_lang.as_deref(), Some("fr"));
        // Any param name, `&amp;` in the href.
        let s = scheme(
            r#"<link href="/?ref=x&amp;hl=de-DE" hreflang="de-DE" rel="alternate"><link rel=alternate hreflang=en href=/>"#,
            "https://example.com/",
        );
        assert_eq!(s.pattern.as_deref(), Some("{route}?hl={locale}"));
        assert_eq!(s.default_locale.as_deref(), Some("en"));
    }

    #[test]
    fn locale_path_prefix() {
        // next-intl `as-needed`: alternates arrive as a `Link` header, the default locale is unprefixed.
        let links = link_tags(&[r#"<https://example.com/>; rel="alternate"; hreflang="en", <https://example.com/de>; rel="alternate"; hreflang="de", <https://example.com/>; rel="alternate"; hreflang="x-default""#.to_string()]);
        let s = scheme(
            &format!(r#"{links}<html lang="en">"#),
            "https://example.com/",
        );
        assert_eq!(s.kind, "prefix");
        assert_eq!(s.pattern.as_deref(), Some("/{locale}{route}"));
        assert_eq!(s.locales, ["en", "de"]);
        assert_eq!(s.default_locale.as_deref(), Some("en"));
        assert!(s.default_unprefixed);
        // `always`: every locale prefixed; the default comes from x-default.
        let s = scheme(
            r#"<html lang="de"><link rel="alternate" hreflang="en-US" href="/en/pricing"><link rel="alternate" hreflang="fr" href="/fr/pricing"><link rel="alternate" hreflang="x-default" href="/fr/pricing">"#,
            "https://example.com/de/pricing",
        );
        assert_eq!(s.kind, "prefix");
        assert!(!s.default_unprefixed);
        assert_eq!(s.default_locale.as_deref(), Some("fr"));
    }

    #[test]
    fn locale_subdomain_and_none() {
        let s = scheme(
            r#"<html lang="en"><link rel="alternate" hreflang="en" href="https://en.example.com/"><link rel="alternate" hreflang="de" href="https://de.example.com/">"#,
            "https://en.example.com/",
        );
        assert_eq!(s.kind, "subdomain");
        assert_eq!(s.pattern, None);
        assert_eq!(s.default_locale.as_deref(), Some("en"));
        let s = scheme(
            r#"<html lang="fr-FR"><head><title>x</title>"#,
            "https://example.com/",
        );
        assert_eq!(s.kind, "unknown");
        assert_eq!(s.pattern, None);
        assert!(s.locales.is_empty());
        assert_eq!(s.default_locale.as_deref(), Some("fr-FR"));
    }

    #[test]
    fn public_ips() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn refuses_private_hosts() {
        let p = run(Url::parse("http://127.0.0.1:1/").unwrap(), APP).await;
        assert_eq!(p.error, Some("private"));
        assert_eq!(p.framing, "unknown");
    }
}
