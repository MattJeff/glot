//! The pure rules of Local SEO: what a market is, the length budget of its search engine, the validators every
//! proposal must pass, and which override wins for a page. No I/O: everything here is unit-tested below.
use glot_llm::seo::serp_width;
use serde::{Deserialize, Serialize};

// ---------- markets ----------

/// A market is a locale, not a language: `es-ES` and `es-MX` search differently and get separate proposals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Market {
    pub locale: String,
    pub language: String,
    /// ISO 3166-1 alpha-2, uppercase. `None` for a bare language (`es`) or a region group (`es-419`).
    pub country: Option<String>,
    /// The search engine whose limits apply: google | baidu | yandex | naver.
    pub engine: &'static str,
    pub limits: Limits,
}

/// Widths in "Latin characters" ([`serp_width`]: CJK / Hangul count double).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Limits {
    pub title: usize,
    pub description: usize,
}

pub fn market(locale: &str) -> Market {
    let mut parts = locale.split(['-', '_']);
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    let country = parts
        .rfind(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic()))
        .map(str::to_ascii_uppercase);
    // Dominant engine of the market. Guidelines, not guarantees: every engine truncates by pixels.
    let (engine, limits) = match (language.as_str(), country.as_deref()) {
        ("zh", Some("CN")) => (
            "baidu",
            Limits {
                title: 60,
                description: 156,
            },
        ),
        ("ru", None) | (_, Some("RU")) => (
            "yandex",
            Limits {
                title: 70,
                description: 160,
            },
        ),
        ("ko", None) | (_, Some("KR")) => (
            "naver",
            Limits {
                title: 80,
                description: 160,
            },
        ),
        _ => (
            "google",
            Limits {
                title: 60,
                description: 160,
            },
        ),
    };
    Market {
        locale: locale.replace('_', "-"),
        language,
        country,
        engine,
        limits,
    }
}

/// `/pricing/?utm=1#top` → `/pricing`; `""` → `/`. Overrides are keyed by this.
pub fn norm_path(path: &str) -> String {
    let p = path.split(['?', '#']).next().unwrap_or_default().trim();
    let p = p.trim_end_matches('/');
    if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    }
}

// ---------- content ----------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Faq {
    pub question: String,
    pub answer: String,
}

/// The search-facing content of one page in one market. An absent field keeps the faithful translation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Content {
    pub title: Option<String>,
    pub description: Option<String>,
    pub h1: Option<String>,
    pub h2: Vec<String>,
    /// A suggestion only: URLs are changed by glot Edge's translated slugs, never by this module.
    pub slug: Option<String>,
    pub faq: Vec<Faq>,
}

impl Content {
    /// Trimmed, empty strings dropped.
    pub fn clean(mut self) -> Self {
        let opt = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        self.title = opt(self.title);
        self.description = opt(self.description);
        self.h1 = opt(self.h1);
        self.slug = opt(self.slug);
        self.h2 = self
            .h2
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.faq = self
            .faq
            .into_iter()
            .map(|f| Faq {
                question: f.question.trim().into(),
                answer: f.answer.trim().into(),
            })
            .collect();
        self
    }

    fn texts(&self) -> Vec<(&'static str, &str)> {
        let mut out = vec![];
        for (f, v) in [
            ("title", &self.title),
            ("description", &self.description),
            ("h1", &self.h1),
        ] {
            if let Some(v) = v {
                out.push((f, v.as_str()));
            }
        }
        out.extend(self.h2.iter().map(|h| ("h2", h.as_str())));
        for f in &self.faq {
            out.push(("faq", f.question.as_str()));
            out.push(("faq", f.answer.as_str()));
        }
        out
    }
}

/// The source page as scanned (source language).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub title: String,
    pub description: String,
    pub h1: String,
    pub h2: Vec<String>,
    /// Visible text excerpt: the evidence a claim must be found in.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub code: String,
    pub severity: Severity,
    pub field: String,
    pub message: String,
}

fn issue(code: &str, severity: Severity, field: &str, message: String) -> Issue {
    Issue {
        code: code.into(),
        severity,
        field: field.into(),
        message,
    }
}

pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Error)
}

// ---------- validators ----------

/// Another page of the same market: its path, what it shows now and what is proposed / served for it.
pub struct Sibling {
    pub path: String,
    pub current: Content,
    pub effective: Content,
}

pub struct Ctx<'a> {
    pub market: &'a Market,
    pub source_locale: &'a str,
    pub page: &'a Page,
    /// The faithful translation shown today (also evidence for claims).
    pub current: &'a Content,
    /// Demand terms of the market, best first.
    pub terms: &'a [String],
    /// Brand / do-not-translate terms, exact spelling.
    pub brands: &'a [String],
    pub siblings: &'a [Sibling],
    /// `(locale, title, top demand term)` of the same page in the other regions of the same language.
    pub regions: &'a [(String, String, String)],
}

const H_MAX: usize = 70;
const SLUG_MAX: usize = 60;
const FAQ_MAX: usize = 5;

/// Words that promise something. A proposal may use a concept only when the page states it (in any language).
// ponytail: lexicon + numbers, not entailment. It catches the claims that get sites in trouble (free, best,
// guaranteed, prices, figures); add an LLM entailment judge if subtler inventions show up in review.
const CLAIMS: &[(&str, &[&str])] = &[
    (
        "free",
        &[
            "free",
            "gratis",
            "gratuit",
            "gratuite",
            "gratuitement",
            "gratuito",
            "gratuita",
            "grátis",
            "kostenlos",
            "gratuitamente",
            "бесплатно",
            "無料",
            "免费",
            "免費",
            "무료",
            "مجاني",
            "مجانا",
        ],
    ),
    (
        "best",
        &[
            "best",
            "mejor",
            "mejores",
            "meilleur",
            "meilleure",
            "meilleurs",
            "beste",
            "bester",
            "bestes",
            "melhor",
            "migliore",
            "#1",
            "n°1",
            "nº 1",
            "nº1",
            "number one",
            "número uno",
            "numéro un",
            "лучший",
            "最高",
            "最好",
            "최고",
            "أفضل",
        ],
    ),
    (
        "cheap",
        &[
            "cheap",
            "cheapest",
            "barato",
            "barata",
            "baratos",
            "baratas",
            "pas cher",
            "moins cher",
            "günstig",
            "billig",
            "economico",
            "econômico",
            "económico",
            "дешево",
            "激安",
            "最安",
            "便宜",
            "رخيص",
        ],
    ),
    (
        "guarantee",
        &[
            "guarantee",
            "guaranteed",
            "garantía",
            "garantizado",
            "garantizada",
            "garantie",
            "garanti",
            "garantiert",
            "garantia",
            "garantito",
            "гарантия",
            "保証",
            "保证",
            "보장",
            "ضمان",
        ],
    ),
    (
        "official",
        &[
            "official",
            "oficial",
            "officiel",
            "officielle",
            "offiziell",
            "ufficiale",
            "официальный",
            "公式",
            "官方",
            "공식",
            "رسمي",
        ],
    ),
    (
        "discount",
        &[
            "discount",
            "descuento",
            "descuentos",
            "réduction",
            "remise",
            "rabatt",
            "sconto",
            "desconto",
            "oferta",
            "ofertas",
            "promo",
            "скидка",
            "割引",
            "折扣",
            "할인",
            "خصم",
        ],
    ),
    (
        "certified",
        &[
            "certified",
            "certificado",
            "certificada",
            "certifié",
            "certifiée",
            "zertifiziert",
            "certificato",
            "認定",
            "认证",
        ],
    ),
];

fn fold(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '#' && c != '°')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whole-word match for space-separated scripts, substring for the others (CJK has no word boundaries).
fn mentions(text: &str, word: &str) -> bool {
    let text = text.to_lowercase();
    if word
        .chars()
        .all(|c| c.is_alphabetic() && (c as u32) < 0x2E80)
    {
        words(&text).iter().any(|w| w == word)
    } else {
        text.contains(word)
    }
}

/// Digit runs of a text, separators dropped: "1 299,00 €" → ["1", "29900"] is avoided by keeping `.,` inside a run.
fn numbers(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let inner_sep = matches!(c, '.' | ',')
            && !cur.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_ascii_digit() {
            cur.push(*c);
        } else if !inner_sep && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `term` in `text` as a whole word (not inside a longer one).
fn has_word(text: &str, term: &str, case_sensitive: bool) -> bool {
    let (text, term) = if case_sensitive {
        (text.to_string(), term.to_string())
    } else {
        (text.to_lowercase(), term.to_lowercase())
    };
    if term.is_empty() {
        return false;
    }
    text.match_indices(&term).any(|(i, m)| {
        let before = text[..i].chars().next_back();
        let after = text[i + m.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn count(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        0
    } else {
        haystack.matches(needle).count()
    }
}

/// Every rule a proposal (from the LLM or from a human edit) must pass. Errors block approval and auto-apply;
/// warnings are shown and block auto-apply only.
pub fn validate(c: &Content, ctx: &Ctx) -> Vec<Issue> {
    use Severity::{Error, Warning};
    let mut out = vec![];
    let lim = ctx.market.limits;
    let engine = ctx.market.engine;

    // 1. Length budget of the market's search engine.
    if let Some(t) = &c.title {
        let w = serp_width(t);
        if w > lim.title {
            out.push(issue(
                "title_too_long",
                Error,
                "title",
                format!(
                    "The title is {w} wide; {engine} cuts titles around {}.",
                    lim.title
                ),
            ));
        } else if w < 15 {
            out.push(issue(
                "title_too_short",
                Warning,
                "title",
                format!("The title is only {w} wide: room is left for what people search."),
            ));
        }
    }
    if let Some(d) = &c.description {
        let w = serp_width(d);
        if w > lim.description {
            out.push(issue(
                "description_too_long",
                Error,
                "description",
                format!(
                    "The description is {w} wide; {engine} cuts descriptions around {}.",
                    lim.description
                ),
            ));
        } else if w < 50 {
            out.push(issue(
                "description_too_short",
                Warning,
                "description",
                format!("The description is only {w} wide."),
            ));
        }
    }
    for (field, h) in
        c.h1.iter()
            .map(|h| ("h1", h))
            .chain(c.h2.iter().map(|h| ("h2", h)))
    {
        if serp_width(h) > H_MAX {
            out.push(issue(
                "heading_too_long",
                Warning,
                field,
                format!("“{h}” is longer than {H_MAX}: a heading, not a sentence."),
            ));
        }
    }

    // 2. Keyword stuffing.
    for (field, text, phrase_max, word_max) in [
        ("title", c.title.as_deref(), 1, 2),
        ("h1", c.h1.as_deref(), 1, 2),
        ("description", c.description.as_deref(), 2, 3),
    ] {
        let Some(text) = text else { continue };
        let folded = fold(text);
        if let Some(t) = ctx
            .terms
            .iter()
            .find(|t| count(&folded, &fold(t)) > phrase_max)
        {
            out.push(issue(
                "keyword_stuffing",
                Error,
                field,
                format!("“{t}” is repeated in the {field}: search engines read that as stuffing."),
            ));
        }
        let ws = words(text);
        if let Some(w) = ws.iter().find(|w| {
            w.chars().count() >= 4
                && !ctx.brands.iter().any(|b| b.to_lowercase() == **w)
                && ws.iter().filter(|x| x == w).count() > word_max
        }) {
            out.push(issue(
                "keyword_stuffing",
                Error,
                field,
                format!("“{w}” appears more than {word_max} times in the {field}."),
            ));
        }
    }
    if let Some(t) = &c.title
        && t.split(['|', ',', '·', '•'])
            .flat_map(|p| p.split(" - "))
            .filter(|p| !p.trim().is_empty())
            .count()
            >= 4
    {
        out.push(issue(
            "keyword_list",
            Error,
            "title",
            "The title is a list of keywords, not a title.".into(),
        ));
    }

    // 3. No claim the page does not make.
    let mut evidence = format!(
        "{} {} {} {} {}",
        ctx.page.title,
        ctx.page.description,
        ctx.page.h1,
        ctx.page.h2.join(" "),
        ctx.page.text
    );
    for (_, t) in ctx.current.texts() {
        evidence.push(' ');
        evidence.push_str(t);
    }
    let known_numbers = numbers(&evidence);
    for (field, text) in c.texts() {
        for (concept, lexicon) in CLAIMS {
            if let Some(w) = lexicon.iter().find(|w| mentions(text, w))
                && !lexicon.iter().any(|w| mentions(&evidence, w))
            {
                out.push(issue(
                    "unsupported_claim",
                    Error,
                    field,
                    format!("“{w}” ({concept}) is not stated anywhere on the page."),
                ));
            }
        }
        if let Some(n) = numbers(text)
            .into_iter()
            .find(|n| !known_numbers.contains(n))
        {
            out.push(issue(
                "unsupported_number",
                Error,
                field,
                format!("The figure “{n}” is not on the page."),
            ));
        }
    }

    // 4. Brand terms intact.
    for b in ctx.brands {
        for (field, source, proposed, severity) in [
            ("title", &ctx.page.title, &c.title, Error),
            ("h1", &ctx.page.h1, &c.h1, Error),
            (
                "description",
                &ctx.page.description,
                &c.description,
                Warning,
            ),
        ] {
            let Some(p) = proposed else { continue };
            // Only where the page itself names the brand: "Pro" the plan is not "pro" the word.
            if !has_word(source, b, true) || has_word(p, b, true) {
                continue;
            }
            if has_word(p, b, false) {
                out.push(issue(
                    "brand_altered",
                    Error,
                    field,
                    format!("“{b}” must keep its exact spelling."),
                ));
            } else {
                out.push(issue(
                    "brand_missing",
                    severity,
                    field,
                    format!("The page's {field} names “{b}”; the proposal dropped it."),
                ));
            }
        }
    }

    // 5. Unique across the site's pages of this market.
    for s in ctx.siblings {
        for (field, mine, theirs) in [
            ("title", &c.title, [&s.effective.title, &s.current.title]),
            (
                "description",
                &c.description,
                [&s.effective.description, &s.current.description],
            ),
            ("slug", &c.slug, [&s.effective.slug, &s.current.slug]),
        ] {
            if let Some(m) = mine
                && theirs
                    .iter()
                    .any(|t| t.as_ref().is_some_and(|t| fold(t) == fold(m)))
            {
                out.push(issue(&format!("duplicate_{field}"), Error, field, format!("{} already uses this {field}: two pages with the same {field} compete with each other.", s.path)));
            }
        }
    }

    // 6. Really localized.
    if let Some(t) = &c.title
        && ctx.market.language != glot_core::plural::language(ctx.source_locale)
        && fold(t) == fold(&ctx.page.title)
        && !ctx.brands.iter().any(|b| fold(b) == fold(t))
    {
        out.push(issue(
            "untranslated",
            Warning,
            "title",
            "The title is identical to the source language title.".into(),
        ));
    }
    if let Some(t) = &c.title {
        for (locale, title, top) in ctx.regions {
            if fold(title) == fold(t)
                && ctx
                    .terms
                    .first()
                    .is_some_and(|mine| fold(mine) != fold(top))
            {
                out.push(issue(
                    "same_as_other_region",
                    Warning,
                    "title",
                    format!("Same title as {locale}, although people there search “{top}”."),
                ));
            }
        }
    }

    // 7. Slug suggestion and FAQ shape.
    if let Some(s) = &c.slug {
        let ok = !s.is_empty()
            && !s.starts_with('-')
            && !s.ends_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            out.push(issue(
                "slug_invalid",
                Error,
                "slug",
                "A slug is lowercase letters, digits and hyphens.".into(),
            ));
        } else if s.len() > SLUG_MAX {
            out.push(issue(
                "slug_too_long",
                Warning,
                "slug",
                format!("The slug is longer than {SLUG_MAX} characters."),
            ));
        }
    }
    if c.faq.len() > FAQ_MAX {
        out.push(issue(
            "faq_too_many",
            Warning,
            "faq",
            format!("More than {FAQ_MAX} questions: keep the ones people ask."),
        ));
    }
    if c.faq
        .iter()
        .any(|f| f.question.is_empty() || f.answer.is_empty())
    {
        out.push(issue(
            "faq_incomplete",
            Error,
            "faq",
            "Every question needs an answer.".into(),
        ));
    }
    out
}

// ---------- resolution ----------

/// Statuses a visitor or a crawler can see. Everything else is a draft.
pub fn served(status: &str) -> bool {
    matches!(status, "approved" | "auto")
}

fn same_locale(a: &str, b: &str) -> bool {
    a.replace('_', "-")
        .eq_ignore_ascii_case(&b.replace('_', "-"))
}

/// Which override applies to `locale` among the rows `(locale, status)` of one page. Order:
/// 1. the market's own row when it is served;
/// 2. nothing when the market's own row was rejected (a human said no for this market);
/// 3. the bare-language row (`es` for `es-MX`) when it is served;
/// 4. nothing: the faithful translation stays. A sibling region (`es-ES` for `es-MX`) is never used.
pub fn resolve<'a, T>(rows: &'a [(String, String, T)], locale: &str) -> Option<&'a T> {
    let own = rows.iter().find(|(l, _, _)| same_locale(l, locale));
    match own {
        Some((_, s, c)) if served(s) => return Some(c),
        Some((_, s, _)) if s == "rejected" => return None,
        _ => {}
    }
    let lang = glot_core::plural::language(locale);
    if same_locale(&lang, locale) {
        return None;
    }
    rows.iter()
        .find(|(l, s, _)| same_locale(l, &lang) && served(s))
        .map(|(_, _, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        Page {
            title: "Recadrez vos vidéos | ClipOnIt".into(),
            description: "Recadrez vos vidéos en un clic, à partir de 9 € par mois.".into(),
            h1: "Recadrez vos vidéos".into(),
            h2: vec![],
            text: "Importez une vidéo, choisissez un format, exportez.".into(),
        }
    }

    fn check(c: Content, terms: &[&str], siblings: &[Sibling]) -> Vec<String> {
        let (m, p, cur) = (market("es-MX"), page(), Content::default());
        let terms: Vec<String> = terms.iter().map(|t| t.to_string()).collect();
        let brands = vec!["ClipOnIt".to_string()];
        let ctx = Ctx {
            market: &m,
            source_locale: "fr",
            page: &p,
            current: &cur,
            terms: &terms,
            brands: &brands,
            siblings,
            regions: &[],
        };
        validate(&c, &ctx).into_iter().map(|i| i.code).collect()
    }

    fn title(t: &str) -> Content {
        Content {
            title: Some(t.into()),
            ..Default::default()
        }
    }

    #[test]
    fn markets_are_locales_not_languages() {
        let (es, mx) = (market("es-ES"), market("es-MX"));
        assert_eq!(es.language, mx.language);
        assert_ne!(es.country, mx.country);
        assert_eq!(market("es").country, None);
        assert_eq!(market("es-419").country, None);
        assert_eq!(market("zh-Hant-TW").country.as_deref(), Some("TW"));
        assert_eq!(market("zh-CN").engine, "baidu");
        assert_eq!(market("ru").engine, "yandex");
        assert_eq!(market("ko-KR").engine, "naver");
        assert_eq!(market("fr-CA").engine, "google");
    }

    #[test]
    fn length_limits_follow_the_engine() {
        let long = "Cortar video en línea para computadora y celular | ClipOnIt MX";
        assert_eq!(serp_width(long), 62);
        assert_eq!(check(title(long), &[], &[]), ["title_too_long"]);
        // The same title fits Yandex (70).
        let (m, p, cur) = (market("es-RU"), page(), Content::default());
        let brands = vec!["ClipOnIt".to_string()];
        let ctx = Ctx {
            market: &m,
            source_locale: "fr",
            page: &p,
            current: &cur,
            terms: &[],
            brands: &brands,
            siblings: &[],
            regions: &[],
        };
        assert!(validate(&title(long), &ctx).is_empty());
        // CJK counts double: 31 ideographs = 62.
        assert_eq!(
            check(title(&format!("{} ClipOnIt", "视".repeat(26))), &[], &[]),
            ["title_too_long"]
        );
        let d = Content {
            description: Some("x".repeat(161)),
            ..Default::default()
        };
        assert_eq!(check(d, &[], &[]), ["description_too_long"]);
        assert_eq!(check(title("ClipOnIt"), &[], &[]), ["title_too_short"]);
    }

    #[test]
    fn keyword_stuffing_is_refused() {
        let terms = ["cortar video"];
        assert_eq!(
            check(
                title("Cortar video: cortar video en línea | ClipOnIt"),
                &terms,
                &[]
            ),
            ["keyword_stuffing"]
        );
        assert_eq!(
            check(title("Video, video online y video HD | ClipOnIt"), &[], &[]),
            ["keyword_stuffing"]
        );
        assert_eq!(
            check(
                title("Cortar video | Editor | Recortar | ClipOnIt"),
                &terms,
                &[]
            ),
            ["keyword_list"]
        );
        assert!(check(title("Cortar video en línea | ClipOnIt"), &terms, &[]).is_empty());
    }

    #[test]
    fn claims_must_be_on_the_page() {
        // "gratis" is what people search, but the page never says it is free.
        assert_eq!(
            check(
                title("Cortar video gratis | ClipOnIt"),
                &["cortar video gratis"],
                &[]
            ),
            ["unsupported_claim"]
        );
        assert_eq!(
            check(title("El mejor editor de video | ClipOnIt"), &[], &[]),
            ["unsupported_claim"]
        );
        // Figures: 9 is on the page, 5 is not.
        let d = |s: &str| Content {
            description: Some(format!(
                "Corta tus videos en un clic con ClipOnIt, {s} y listo para publicar."
            )),
            ..Default::default()
        };
        assert!(check(d("desde 9 € al mes"), &[], &[]).is_empty());
        assert_eq!(
            check(d("desde 5 € al mes"), &[], &[]),
            ["unsupported_number"]
        );
        // FAQ answers are held to the same rule.
        let faq = Content {
            faq: vec![Faq {
                question: "¿Cómo cortar un video?".into(),
                answer: "Con garantía de 30 días.".into(),
            }],
            ..Default::default()
        };
        assert_eq!(
            check(faq, &[], &[]),
            ["unsupported_claim", "unsupported_number"]
        );
        // A claim the page makes (in the source language) is allowed in the market's language.
        let mut p = page();
        p.text.push_str(" Essai gratuit.");
        let (m, cur) = (market("es-MX"), Content::default());
        let ctx = Ctx {
            market: &m,
            source_locale: "fr",
            page: &p,
            current: &cur,
            terms: &[],
            brands: &[],
            siblings: &[],
            regions: &[],
        };
        assert!(validate(&title("Cortar video gratis en línea"), &ctx).is_empty());
    }

    #[test]
    fn brand_terms_stay_intact() {
        assert_eq!(
            check(title("Cortar video en línea"), &[], &[]),
            ["brand_missing"]
        );
        assert_eq!(
            check(title("Cortar video en línea | Cliponit"), &[], &[]),
            ["brand_altered"]
        );
        assert!(check(title("Cortar video en línea | ClipOnIt"), &[], &[]).is_empty());
        // A short brand ("Pro", a plan name) is a word, not a substring: "producto" neither alters nor replaces it.
        let (m, cur) = (market("es-MX"), Content::default());
        let p = Page {
            title: "Offre Pro | ClipOnIt".into(),
            ..page()
        };
        let brands = vec!["Pro".to_string()];
        let ctx = Ctx {
            market: &m,
            source_locale: "fr",
            page: &p,
            current: &cur,
            terms: &[],
            brands: &brands,
            siblings: &[],
            regions: &[],
        };
        let codes = |t: &str| {
            validate(&title(t), &ctx)
                .into_iter()
                .map(|i| i.code)
                .collect::<Vec<_>>()
        };
        assert_eq!(codes("Un producto para cortar video"), ["brand_missing"]);
        assert_eq!(codes("Plan pro para cortar video"), ["brand_altered"]);
        assert!(codes("Plan Pro para cortar video").is_empty());
        let other = Page {
            title: "Recadrez vos produits".into(),
            ..page()
        };
        let ctx = Ctx {
            page: &other,
            ..ctx
        };
        assert!(validate(&title("Un producto para cortar video"), &ctx).is_empty());
    }

    #[test]
    fn titles_are_unique_across_the_site() {
        let other = Sibling {
            path: "/pricing".into(),
            current: title("Precios | ClipOnIt"),
            effective: title("Cortar video en línea | ClipOnIt"),
        };
        assert_eq!(
            check(
                title("cortar video en línea | ClipOnIt"),
                &[],
                std::slice::from_ref(&other)
            ),
            ["duplicate_title"]
        );
        // Also against what the other page shows today.
        assert_eq!(
            check(title("Precios | ClipOnIt"), &[], &[other]),
            ["duplicate_title"]
        );
    }

    #[test]
    fn regions_of_a_language_are_compared() {
        let (m, p, cur) = (market("es-MX"), page(), Content::default());
        let terms = vec!["cortar video en línea".to_string()];
        let regions = vec![(
            "es-ES".to_string(),
            "Recortar vídeo online | ClipOnIt".to_string(),
            "recortar vídeo online".to_string(),
        )];
        let ctx = Ctx {
            market: &m,
            source_locale: "fr",
            page: &p,
            current: &cur,
            terms: &terms,
            brands: &[],
            siblings: &[],
            regions: &regions,
        };
        let codes = |t: &str| {
            validate(&title(t), &ctx)
                .into_iter()
                .map(|i| i.code)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            codes("Recortar vídeo online | ClipOnIt"),
            ["same_as_other_region"]
        );
        assert!(codes("Cortar video en línea | ClipOnIt").is_empty());
    }

    #[test]
    fn slug_and_faq_shape() {
        let slug = |s: &str| Content {
            slug: Some(s.into()),
            ..Default::default()
        };
        assert_eq!(check(slug("Cortar Video"), &[], &[]), ["slug_invalid"]);
        assert!(check(slug("cortar-video-en-linea"), &[], &[]).is_empty());
        let faq = Content {
            faq: vec![Faq {
                question: "¿Y?".into(),
                answer: String::new(),
            }],
            ..Default::default()
        };
        assert_eq!(check(faq, &[], &[]), ["faq_incomplete"]);
    }

    #[test]
    fn resolution_order() {
        let rows = |v: &[(&str, &str)]| -> Vec<(String, String, String)> {
            v.iter()
                .map(|(l, s)| (l.to_string(), s.to_string(), l.to_string()))
                .collect()
        };
        let r = |v: &[(&str, &str)], locale: &str| resolve(&rows(v), locale).cloned();
        // 1. The market's own served row wins over the language row.
        assert_eq!(
            r(&[("es", "approved"), ("es-MX", "approved")], "es-MX").as_deref(),
            Some("es-MX")
        );
        assert_eq!(
            r(&[("es", "approved"), ("es-MX", "auto")], "es_mx").as_deref(),
            Some("es-MX")
        );
        // 2. A rejected market row blocks the fallback.
        assert_eq!(
            r(&[("es", "approved"), ("es-MX", "rejected")], "es-MX"),
            None
        );
        // 3. A draft / stale market row falls back to the served language row.
        for s in ["proposed", "stale", "needs_demand"] {
            assert_eq!(
                r(&[("es", "approved"), ("es-MX", s)], "es-MX").as_deref(),
                Some("es")
            );
        }
        assert_eq!(r(&[("es", "approved")], "es-MX").as_deref(), Some("es"));
        // 4. Never a sibling region, never a draft.
        assert_eq!(r(&[("es-ES", "approved")], "es-MX"), None);
        assert_eq!(r(&[("es-MX", "proposed")], "es-MX"), None);
        assert_eq!(r(&[("es", "proposed")], "es-MX"), None);
        assert_eq!(r(&[("es-MX", "approved")], "es"), None);
    }

    #[test]
    fn paths_are_normalized() {
        assert_eq!(norm_path("/pricing/?utm=1#top"), "/pricing");
        assert_eq!(norm_path("/"), "/");
        assert_eq!(norm_path(""), "/");
        assert_eq!(norm_path("a/b"), "/a/b");
    }
}
