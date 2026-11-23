//! Segmented copy experiments in the Edge proxy: a Rust port of `decide()` (packages/edge/src/experiments.ts,
//! contract §5–7), limited to experiments whose keys are all Edge segments (`edge:<hash>`, contract §4.1): those
//! are the only texts the proxy can substitute. Same steps as the JS: GPC/DNT → official text, `?glot_xp=` preview,
//! holdout, then one arm per experiment (sticky through the `glot_xp` cookie), `x-glot-xp` for the origin.
//! Differences, on purpose: bots (`ingest::is_bot`) get the official text and no assignment (SEO unchanged); no
//! `unit` (the proxy has no stable visitor id, draws are random per document, like the JS default); no consent
//! signal, so no `glot_seen`; the cookie carries `Domain=` the origin's domain so its server can read it.
use axum::http::{HeaderMap, header};
use base64::Engine;
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use crate::experiments::{EDGE_KEY, Exposure, SegmentRule};

#[derive(serde::Deserialize)]
pub struct Config {
    pub v: u32,
    #[serde(default)]
    pub max_stale: u64,
    #[serde(default)]
    persistence: String,
    holdout: Option<Holdout>,
    #[serde(default)]
    segments: Vec<Seg>,
    #[serde(default)]
    experiments: Vec<Xp>,
}

#[derive(serde::Deserialize)]
struct Holdout {
    id: String,
    pct: f64,
}

#[derive(serde::Deserialize)]
struct Seg {
    id: String,
    rule: SegmentRule,
}

#[derive(serde::Deserialize)]
struct Xp {
    id: String,
    status: String,
    traffic_pct: f64,
    locales: Vec<String>,
    #[serde(default)]
    keys: Vec<String>,
    cells: Vec<Cell>,
    arms: Vec<Arm>,
}

#[derive(serde::Deserialize)]
struct Cell {
    id: String,
    weights: Vec<f64>,
    count: bool,
}

#[derive(serde::Deserialize)]
struct Arm {
    arm: i32,
    #[serde(default)]
    values: BTreeMap<String, BTreeMap<String, String>>,
}

impl Xp {
    fn edge(&self) -> bool {
        !self.keys.is_empty() && self.keys.iter().all(|k| k.starts_with(EDGE_KEY))
    }
}

#[derive(Default, Debug)]
pub struct Decision {
    /// Segment hash → arm text for the served locale (arm 0 never appears).
    pub overlay: BTreeMap<String, String>,
    /// What to count (`ingest::record`): empty for prefetches, previews, personalized cells.
    pub exposures: Vec<Exposure>,
    /// `x-glot-xp` request header for the origin.
    pub header: Option<String>,
    pub cookie: Option<String>,
    /// An experiment of this locale decided the page: never shared through a cache.
    pub assigned: bool,
}

#[derive(serde::Serialize, Clone)]
struct A {
    id: String,
    cell: String,
    arm: i32,
}

static PREFETCH: LazyLock<Regex> = LazyLock::new(|| Regex::new("prefetch|prerender").unwrap());
static PAID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("cpc|ppc|paid|display|cpm|banner|ads?$").unwrap());
static SEARCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(^|\.)(google|bing|yahoo|duckduckgo|baidu|yandex|ecosia|qwant|startpage|naver|seznam)\.",
    )
    .unwrap()
});
static SOCIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|\.)(facebook|fb|instagram|t|twitter|x|linkedin|lnkd|reddit|pinterest|tiktok|youtube|threads|bsky|mastodon)\.|^(t\.co|lnkd\.in)$").unwrap()
});
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|\.)(mail|outlook|gmail)\.").unwrap());
static TABLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)ipad|tablet|kindle|silk").unwrap());
static MOBILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)mobi|iphone|ipod|android").unwrap());

fn get<'a>(h: &'a HeaderMap, k: &str) -> Option<&'a str> {
    h.get(k).and_then(|v| v.to_str().ok())
}

pub fn cookie<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|c| c.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(name)?.strip_prefix('='))
}

/// `glot_xp` cookie value → (experiment id, arm, cell): the stored assignments (contract §7).
pub fn parse_cookie(v: &str) -> Vec<(String, i32, String)> {
    v.split('~')
        .filter_map(|part| {
            let mut it = part.split('.');
            let (id, arm, cell) = (it.next()?, it.next()?, it.next()?);
            let ok = !id.is_empty() && !cell.is_empty() && arm.len() == 1;
            Some((
                id.to_string(),
                arm.parse().ok().filter(|_| ok)?,
                cell.to_string(),
            ))
        })
        .collect()
}

/// A real document navigation (counted once): not a prefetch, not a Next RSC / client-navigation fetch.
fn is_document(h: &HeaderMap, q: &[(String, String)]) -> bool {
    let purpose = ["sec-purpose", "purpose", "x-purpose"]
        .map(|k| get(h, k).unwrap_or(""))
        .concat();
    if PREFETCH.is_match(&purpose)
        || ["next-router-prefetch", "rsc", "next-router-state-tree"]
            .iter()
            .any(|k| h.contains_key(*k))
        || q.iter().any(|(k, _)| k == "_rsc")
    {
        return false;
    }
    match get(h, "sec-fetch-dest") {
        Some(d) => d == "document",
        None => get(h, "accept").unwrap_or("").contains("text/html"),
    }
}

/// Visitor context (contract §3) without app attributes; `visitor` is `unknown` (no consent signal here).
fn context(
    h: &HeaderMap,
    locale: &str,
    host: &str,
    q: &[(String, String)],
) -> BTreeMap<String, String> {
    let mut ctx = BTreeMap::from([("locale".to_string(), locale.to_string())]);
    let country = [
        "x-vercel-ip-country",
        "cf-ipcountry",
        "cloudfront-viewer-country",
        "fastly-geo-country",
    ]
    .iter()
    .find_map(|k| get(h, k))
    .unwrap_or("")
    .to_ascii_uppercase();
    if country.len() == 2 && country.bytes().all(|b| b.is_ascii_uppercase()) && country != "XX" {
        ctx.insert("country".into(), country);
    }
    if let Some(r) = [
        "x-vercel-ip-country-region",
        "cf-region-code",
        "cloudfront-viewer-country-region",
    ]
    .iter()
    .find_map(|k| get(h, k))
    {
        ctx.insert("region".into(), r.to_ascii_uppercase());
    }
    let ua = get(h, "user-agent").unwrap_or("");
    let lower = ua.to_ascii_lowercase();
    // JS `android(?!.*mobile)`: the last "android" not followed by "mobile".
    let android_tablet = lower
        .rfind("android")
        .is_some_and(|i| !lower[i..].contains("mobile"));
    let device = if TABLET.is_match(ua) || android_tablet {
        "tablet"
    } else if get(h, "sec-ch-ua-mobile") == Some("?1") || MOBILE.is_match(ua) {
        "mobile"
    } else {
        "desktop"
    };
    ctx.insert("device".into(), device.into());
    ctx.insert("source".into(), source(host, q, get(h, "referer")).into());
    ctx.insert("visitor".into(), "unknown".into());
    ctx
}

fn source(host: &str, q: &[(String, String)], referer: Option<&str>) -> &'static str {
    let param = |k: &str| q.iter().find(|p| p.0 == k).map(|p| p.1.as_str());
    let medium = param("utm_medium").unwrap_or("").to_ascii_lowercase();
    if PAID.is_match(&medium)
        || ["gclid", "fbclid", "msclkid"]
            .iter()
            .any(|k| param(k).is_some())
    {
        return "paid";
    }
    if medium.contains("email") || medium == "newsletter" {
        return "email";
    }
    if medium.contains("social") {
        return "social";
    }
    if medium == "organic" {
        return "search";
    }
    let mut from = param("utm_source").unwrap_or("").to_ascii_lowercase();
    if !from.is_empty() {
        from.push('.'); // `utm_source=google` → matches like a host
    } else if let Some(r) = referer {
        from = reqwest::Url::parse(r)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|h| h.trim_start_matches("www.").to_string())
            })
            .unwrap_or_default();
        if from == host.trim_start_matches("www.") {
            return "direct"; // internal navigation
        }
    }
    if from.is_empty() {
        "direct"
    } else if SEARCH.is_match(&from) {
        "search"
    } else if SOCIAL.is_match(&from) {
        "social"
    } else if EMAIL.is_match(&from) {
        "email"
    } else {
        "referral"
    }
}

/// Index into `weights` for u ∈ [0, 1); None if every weight is 0.
fn pick(weights: &[f64], u: f64) -> Option<usize> {
    let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
    if total <= 0.0 {
        return None;
    }
    let (mut acc, mut last) = (0.0, None);
    for (i, w) in weights.iter().enumerate().filter(|(_, w)| **w > 0.0) {
        last = Some(i);
        acc += w;
        if u * total < acc {
            return Some(i);
        }
    }
    last // float rounding at u ≈ 1
}

/// `es.example.com` served for the origin `https://www.example.com` → `example.com`: the cookie reaches the
/// origin (its server forwards it with server-side conversions). None (host-only) when the Edge host is not under
/// the origin's domain.
pub fn cookie_domain(host: &str, origin_url: &str) -> Option<String> {
    let o = reqwest::Url::parse(origin_url).ok()?;
    let o = o.host_str()?.trim_start_matches("www.");
    (o.contains('.') && host.ends_with(&format!(".{o}"))).then(|| o.to_string())
}

pub fn decide(
    h: &HeaderMap,
    query: &str,
    host: &str,
    cfg: &Config,
    locale: &str,
    domain: Option<&str>,
) -> Decision {
    if cfg.v != 1 || get(h, "sec-gpc") == Some("1") || get(h, "dnt") == Some("1") {
        return Decision::default(); // opt-out: official text, nothing counted
    }
    // Bots and crawlers: the official text (no cloaking of the indexed copy), never counted.
    if crate::experiments::ingest::is_bot(h) {
        return Decision::default();
    }
    let mine = |x: &&Xp| x.edge() && x.locales.iter().any(|l| l == locale);
    if !cfg.experiments.iter().any(|x| mine(&x)) {
        return Decision::default(); // nothing the proxy serves here (the holdout included)
    }
    let q: Vec<(String, String)> = reqwest::Url::parse(&format!("http://h/?{query}"))
        .map(|u| {
            u.query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default();
    let rand = rand::random::<f64>;
    let session = cfg.persistence == "session";
    let theirs = cookie(h, "glot_xp");
    let saved: HashMap<String, (i32, String)> = if session {
        parse_cookie(theirs.unwrap_or(""))
            .into_iter()
            .map(|(id, arm, cell)| (id, (arm, cell)))
            .collect()
    } else {
        HashMap::new()
    };
    let mut assignments: Vec<A> = vec![];
    let mut counted: Vec<A> = vec![];
    let mut overlay: BTreeMap<String, String> = BTreeMap::new();
    let mut k: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let apply = |overlay: &mut BTreeMap<String, String>, arm: &Arm| {
        if let Some(v) = arm.values.get(locale) {
            overlay.extend(v.iter().map(|(a, b)| (a.clone(), b.clone())));
        }
    };

    // Preview `?glot_xp=<id>.<arm>`: that arm only, nothing counted, nothing stored.
    if let Some((_, forced)) = q.iter().find(|p| p.0 == "glot_xp") {
        let (id, arm) = forced.split_once('.').unwrap_or((forced, ""));
        let Some((xp, a)) = cfg.experiments.iter().filter(|x| x.edge()).find_map(|x| {
            (x.id == id).then_some(())?;
            Some((x, x.arms.iter().find(|a| arm.parse() == Ok(a.arm))?))
        }) else {
            return Decision::default();
        };
        apply(&mut overlay, a);
        let a = vec![A {
            id: xp.id.clone(),
            cell: "*".into(),
            arm: a.arm,
        }];
        return finish(locale, a, overlay, vec![], None, k);
    }

    let mut in_holdout = false;
    if let Some(ho) = &cfg.holdout {
        let prev = saved.get(&ho.id).filter(|p| p.1 == "*");
        let arm = match prev {
            Some(p) => p.0,
            None if rand() * 100.0 < ho.pct => 0,
            None => 1,
        };
        in_holdout = arm == 0;
        let a = A {
            id: ho.id.clone(),
            cell: "*".into(),
            arm,
        };
        if prev.is_none_or(|p| p.0 != arm) {
            counted.push(a.clone());
        }
        assignments.push(a);
    }

    if !in_holdout {
        let ctx = context(h, locale, host, &q);
        for xp in cfg.experiments.iter().filter(mine) {
            let prev = saved.get(&xp.id);
            let kept = prev.and_then(|(arm, cell)| {
                let c = xp.cells.iter().find(|c| c.id == *cell)?;
                let i = xp.arms.iter().position(|a| a.arm == *arm)?;
                (c.weights.get(i).copied().unwrap_or(0.0) > 0.0).then_some((c, i))
            });
            let (cell, i) = match kept {
                Some(x) => x,
                None => {
                    // ponytail: out-of-traffic visitors aren't stored, so in `session` mode they're re-drawn per page.
                    if rand() * 100.0 >= xp.traffic_pct {
                        continue;
                    }
                    let Some(c) = xp.cells.iter().find(|c| {
                        c.id == "*"
                            || cfg
                                .segments
                                .iter()
                                .any(|s| s.id == c.id && s.rule.matches(&ctx))
                    }) else {
                        continue;
                    };
                    let Some(i) = pick(&c.weights, rand()) else {
                        continue;
                    };
                    (c, i)
                }
            };
            let Some(arm) = xp.arms.get(i) else { continue };
            let a = A {
                id: xp.id.clone(),
                cell: cell.id.clone(),
                arm: arm.arm,
            };
            // `session`: once per experiment and session (an arm already in the cookie was counted on its first page).
            let live = cell.count && xp.status == "running";
            if live && prev.is_none_or(|p| p.0 != a.arm || p.1 != a.cell) {
                counted.push(a.clone());
            }
            if live {
                k.insert(xp.id.clone(), xp.keys.clone());
            }
            assignments.push(a);
            apply(&mut overlay, arm);
        }
    }

    let mut set = None;
    if session && !assignments.is_empty() {
        let mut kept = assignments.clone();
        // Arms of experiments not decided here (another locale, a catalog experiment) that are still live.
        for (id, (arm, cell)) in &saved {
            if !kept.iter().any(|a| a.id == *id) && cfg.experiments.iter().any(|x| x.id == *id) {
                kept.push(A {
                    id: id.clone(),
                    arm: *arm,
                    cell: cell.clone(),
                });
            }
        }
        let value: Vec<String> = kept
            .iter()
            .map(|a| format!("{}.{}.{}", a.id, a.arm, a.cell))
            .collect();
        let value = value.join("~");
        if Some(value.as_str()) != theirs {
            let domain = domain.map(|d| format!("; Domain={d}")).unwrap_or_default();
            set = Some(format!(
                "glot_xp={value}; Path=/; SameSite=Lax; Secure{domain}"
            ));
        }
    }
    let exposures = if is_document(h, &q) { counted } else { vec![] };
    finish(locale, assignments, overlay, exposures, set, k)
}

fn finish(
    locale: &str,
    a: Vec<A>,
    o: BTreeMap<String, String>,
    exposures: Vec<A>,
    cookie: Option<String>,
    k: BTreeMap<String, Vec<String>>,
) -> Decision {
    let mut x = serde_json::json!({"v": 1, "locale": locale, "a": a, "o": o});
    if !k.is_empty() {
        x["k"] = serde_json::json!(k);
    }
    Decision {
        overlay: o
            .iter()
            .filter_map(|(key, v)| Some((key.strip_prefix(EDGE_KEY)?.to_string(), v.clone())))
            .collect(),
        exposures: exposures
            .into_iter()
            .filter_map(|e| {
                Some(Exposure {
                    id: e.id.parse().ok()?,
                    cell: e.cell,
                    arm: e.arm,
                    seen: false,
                    visible_ms: None,
                })
            })
            .collect(),
        header: (!a.is_empty())
            .then(|| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(x.to_string())),
        cookie,
        assigned: !a.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts() {
        assert_eq!(pick(&[0.0, 1.0], 0.3), Some(1));
        assert_eq!(pick(&[0.5, 0.5], 0.49), Some(0));
        assert_eq!(pick(&[0.5, 0.5], 0.99), Some(1));
        assert_eq!(pick(&[0.0, 0.0], 0.5), None);
        assert_eq!(
            parse_cookie("a.1.*~b.x.*~c.12.*~d.0.seg"),
            vec![("a".into(), 1, "*".into()), ("d".into(), 0, "seg".into())]
        );
        assert_eq!(
            cookie_domain("es.gta6clipmaker.com", "https://gta6clipmaker.com"),
            Some("gta6clipmaker.com".into())
        );
        assert_eq!(
            cookie_domain("es.example.com", "https://www.example.com/"),
            Some("example.com".into())
        );
        assert_eq!(
            cookie_domain("es.other.com", "https://app.vercel.app"),
            None
        );
        assert_eq!(
            source("x.com", &[], Some("https://www.google.com/")),
            "search"
        );
        assert_eq!(
            source("x.com", &[("utm_source".into(), "t".into())], None),
            "social"
        );
        assert_eq!(source("x.com", &[], Some("https://x.com/a")), "direct");
    }
}
