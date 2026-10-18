//! `glot labs culture`: cultural adaptation audit, what translation does not fix.
//! 1. Deterministic rules on code and catalogs (hard-coded formats, US-only forms, concatenation…), file:line + fix.
//! 2. Deterministic legal expectations per market (GDPR, prices incl. VAT, Impressum, 特定商取引法).
//! 3. LLM per market (idioms, references, symbols, humour, register), proposals validated (glot_llm::culture).
//! 4. A "ready for <market>" score and a prioritized list; `--apply` writes proposals as suggestions.
use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::process::ExitCode;

use glot_llm::culture::{self, Applied, Severity};
use glot_llm::engine::Workspace;

use crate::style::*;
use crate::{confirm, open, print_json};

#[derive(clap::Args)]
pub struct CultureArgs {
    /// Markets to audit (target locales, comma separated). Default: all targets.
    #[arg(long, value_delimiter = ',')]
    locale: Vec<String>,
    /// Only these keys for the LLM pass (comma separated or repeated).
    #[arg(long, value_delimiter = ',')]
    keys: Vec<String>,
    /// Deterministic rules only (code, catalogs, legal): no LLM call.
    #[arg(long)]
    no_llm: bool,
    /// Ignore the cache (.glot/culture.json) and ask the LLM again.
    #[arg(long)]
    refresh: bool,
    /// Write valid proposals as suggestions (glot.lock origin `transcreation/<model>`, waiting in `glot labs queue`).
    #[arg(long)]
    apply: bool,
    /// Apply without asking.
    #[arg(short, long)]
    yes: bool,
    /// Exit 1 when a market scores below this.
    #[arg(long)]
    min_score: Option<u8>,
    #[arg(long)]
    json: bool,
}

/// A deterministic finding: in code (`file:line`), in the source catalog (`key`) or about a market (`legal`).
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub rule: &'static str,
    pub severity: Severity,
    pub location: String,
    pub snippet: String,
    pub message: &'static str,
    pub fix: &'static str,
    /// A US convention: does not count against an `en-US` market.
    pub us_centric: bool,
}

struct Rule {
    id: &'static str,
    severity: Severity,
    us: bool,
    message: &'static str,
    fix: &'static str,
    /// (line, lowercase line without spaces)
    hit: fn(&str, &str) -> bool,
}

fn any(s: &str, pats: &[&str]) -> bool {
    pats.iter().any(|p| s.contains(p))
}

/// `$` glued to an amount: JSX `${price}`, a template's `$${x}`, `"$29"`, `"$" + x`; `€ £ ¥` touching a digit or `{}`.
fn currency_glued(line: &str) -> bool {
    if line.contains(".replace") {
        return false; // "$1" back-references
    }
    let c: Vec<char> = line.chars().collect();
    let mut ticks = 0;
    let open_template = c.iter().filter(|x| **x == '`').count() % 2 == 1;
    for i in 0..c.len() {
        let next = c.get(i + 1).copied().unwrap_or(' ');
        match c[i] {
            '`' => ticks += 1,
            '$' if next == '{' => {
                if (ticks % 2 == 0 && !open_template) || (i > 0 && c[i - 1] == '$') {
                    return true;
                }
            }
            '$' if next.is_ascii_digit() => return true,
            '€' | '£' | '¥' => {
                let near = |j: Option<usize>| {
                    j.and_then(|j| c.get(j))
                        .is_some_and(|x| x.is_ascii_digit() || *x == '{' || *x == '}')
                };
                if near(Some(i + 1)) || near(i.checked_sub(1)) {
                    return true;
                }
            }
            _ => {}
        }
    }
    let compact = line.replace(' ', "");
    any(&compact, &["\"$\"+", "+\"$\"", "'$'+", "+'$'"])
}

/// A number followed by an imperial unit (`5 mi`, `10 lbs`, `70°F`).
fn imperial(line: &str) -> bool {
    const UNITS: &[&str] = &[
        "mi",
        "mile",
        "miles",
        "mph",
        "lb",
        "lbs",
        "ft",
        "feet",
        "foot",
        "inch",
        "inches",
        "oz",
        "gal",
        "gallon",
        "gallons",
        "yd",
        "yds",
        "yard",
        "yards",
        "°f",
        "fahrenheit",
    ];
    let c: Vec<char> = line.to_lowercase().chars().collect();
    for i in 0..c.len() {
        if !c[i].is_ascii_digit() || c.get(i + 1).is_some_and(|x| x.is_ascii_digit()) {
            continue;
        }
        let start = c[..i]
            .iter()
            .rposition(|x| !x.is_ascii_digit() && *x != '.' && *x != ',');
        if start.is_some_and(|s| c[s].is_alphanumeric()) {
            continue;
        }
        let mut j = i + 1;
        if c.get(j).is_some_and(|x| *x == ' ' || *x == '\u{a0}') {
            j += 1;
        }
        let w: String = c[j..]
            .iter()
            .take_while(|x| x.is_alphabetic() || **x == '°')
            .collect();
        if UNITS.contains(&w.as_str()) {
            return true;
        }
    }
    false
}

/// A numeric date with a 4-digit year whose order is ambiguous or US (`10/31/2026`, `03/04/2026`).
fn numeric_date(line: &str) -> bool {
    let b = line.as_bytes();
    let num = |i: usize| {
        let n = b[i..].iter().take_while(|x| x.is_ascii_digit()).count();
        (n, line[i..i + n].parse::<u32>().unwrap_or(0))
    };
    for i in 0..b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && b[i - 1].is_ascii_digit()) {
            continue;
        }
        let (n1, a) = num(i);
        let j = i + n1;
        if !(1..=2).contains(&n1) || b.get(j) != Some(&b'/') || j + 1 >= b.len() {
            continue;
        }
        let (n2, d) = num(j + 1);
        let k = j + 1 + n2;
        if (1..=2).contains(&n2) && b.get(k) == Some(&b'/') && k + 1 < b.len() {
            let (n3, _) = num(k + 1);
            if n3 == 4 && (1..=12).contains(&a) && (1..=31).contains(&d) {
                return true;
            }
        }
    }
    false
}

/// A literal with words joined with `+` to something else (`"Hello " + name`, `t("a") + x`).
fn concatenation(line: &str, functions: &[&str]) -> bool {
    let compact = line.replace(' ', "");
    if any(
        &compact.to_lowercase(),
        &[
            "console.",
            "thrownew",
            "error(",
            "classname",
            "import",
            "require(",
            "http",
            "class=",
            "bearer",
            "authorization",
        ],
    ) {
        return false;
    }
    // A translated fragment glued to another one.
    let glued_t = functions.iter().any(|f| {
        let call = format!("{f}(");
        compact.match_indices(&call).any(|(i, _)| {
            let before = compact[..i].chars().last();
            let ident = before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
            !ident && (compact[..i].ends_with('+') || compact[i..].contains(")+"))
                || compact[..i].ends_with("${") && compact.matches("${").count() > 1
        })
    });
    if glued_t {
        return true;
    }
    // A worded literal (letters + a space) right next to a `+`.
    for q in ['"', '\''] {
        let parts: Vec<&str> = line.split(q).collect();
        for (n, lit) in parts.iter().enumerate().skip(1).step_by(2) {
            if n + 1 >= parts.len() {
                break;
            }
            let worded = lit.chars().any(char::is_alphabetic)
                && lit.contains(' ')
                && !lit.contains('/')
                && !lit.contains('<');
            let (before, after) = (parts[n - 1].trim_end(), parts[n + 1].trim_start());
            if worded && (before.ends_with('+') || after.starts_with('+')) {
                return true;
            }
        }
    }
    false
}

const RULES: &[Rule] = &[
    Rule {
        id: "hardcoded_locale",
        severity: Severity::High,
        us: true,
        message: "date/number formatting pinned to one locale: every market sees its format",
        fix: "pass the active locale: new Intl.DateTimeFormat(locale, …) / Intl.NumberFormat(locale, …) (next-intl: useFormatter())",
        hit: |_, c| {
            [
                "tolocaledatestring(",
                "tolocaletimestring(",
                "tolocalestring(",
                "intl.datetimeformat(",
                "intl.numberformat(",
                "intl.relativetimeformat(",
                "intl.pluralrules(",
            ]
            .iter()
            .any(|f| {
                c.match_indices(f).any(|(i, _)| {
                    let rest = &c[i + f.len()..];
                    let Some(q) = rest
                        .chars()
                        .next()
                        .filter(|q| matches!(q, '"' | '\'' | '`'))
                    else {
                        return false;
                    };
                    let lit: String = rest[1..].chars().take_while(|x| *x != q).collect();
                    (2..=10).contains(&lit.len())
                        && lit.chars().all(|x| x.is_ascii_alphabetic() || x == '-')
                })
            })
        },
    },
    Rule {
        id: "runtime_locale",
        severity: Severity::Low,
        us: false,
        message: "formats with the runtime's default locale (the server's during SSR), not the user's",
        fix: "pass the user's locale explicitly: d.toLocaleDateString(locale) / new Intl.NumberFormat(locale)",
        hit: |_, c| {
            any(
                c,
                &[
                    "tolocaledatestring()",
                    "tolocaletimestring()",
                    "tolocalestring()",
                    "intl.datetimeformat()",
                    "intl.numberformat()",
                    "(undefined,",
                ],
            ) && c.contains("locale")
                || any(
                    c,
                    &[
                        "tolocaledatestring()",
                        "tolocaletimestring()",
                        "intl.datetimeformat()",
                        "intl.numberformat()",
                    ],
                )
        },
    },
    Rule {
        id: "us_date_format",
        severity: Severity::High,
        us: true,
        message: "date written in US month/day order",
        fix: "new Intl.DateTimeFormat(locale, { dateStyle: \"medium\" }).format(d) (date-fns: format(d, \"P\", { locale }))",
        hit: |l, c| {
            any(
                l,
                &[
                    "MM/DD",
                    "MM/dd",
                    "mm/dd",
                    "M/D/Y",
                    "M/d/y",
                    "MMM D, Y",
                    "MMM d, y",
                    "MMMM D, Y",
                    "MMMM d, y",
                    "MMM Do",
                ],
            ) || numeric_date(l)
                || c.contains("getmonth()+1") && c.contains("\"/\"")
        },
    },
    Rule {
        id: "currency_symbol",
        severity: Severity::High,
        us: true,
        message: "currency symbol glued to the amount: symbol, position, spacing and decimals differ per market (9,99 €, CHF 9.99)",
        fix: "new Intl.NumberFormat(locale, { style: \"currency\", currency }).format(amount), or {amount, number, ::currency/EUR} in the message",
        hit: |l, _| currency_glued(l),
    },
    Rule {
        id: "fixed_currency",
        severity: Severity::Low,
        us: true,
        message: "currency fixed to USD: other markets expect their currency or an explicit mention",
        fix: "derive the currency from the market (or the account), and say \"USD\" explicitly where you cannot",
        hit: |_, c| {
            any(
                c,
                &[
                    "currency=\"usd\"",
                    "currency:\"usd\"",
                    "currency='usd'",
                    "currency:'usd'",
                ],
            )
        },
    },
    Rule {
        id: "unformatted_number",
        severity: Severity::Low,
        us: false,
        message: "number rendered with toFixed/toPrecision: always a '.' decimal point and no grouping (1,234.5 vs 1 234,5)",
        fix: "new Intl.NumberFormat(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(n)",
        hit: |_, c| any(c, &[".tofixed(", ".toprecision("]),
    },
    Rule {
        id: "imperial_units",
        severity: Severity::Medium,
        us: true,
        message: "imperial unit: most markets use metric",
        fix: "convert per market and format with new Intl.NumberFormat(locale, { style: \"unit\", unit: \"kilometer\" })",
        hit: |l, _| imperial(l),
    },
    Rule {
        id: "us_phone",
        severity: Severity::Medium,
        us: true,
        message: "US phone format (validation pattern or example): rejects or confuses non-US numbers",
        fix: "accept international numbers: libphonenumber-js isValidPhoneNumber(value, country), store E.164, localized placeholder",
        hit: |l, _| {
            (l.contains(r"\d{3}") && l.contains(r"\d{4}"))
                || (l.contains("[0-9]{3}") && l.contains("[0-9]{4}"))
                || any(l, &["(555)", "555-0", "555-1"])
        },
    },
    Rule {
        id: "us_zip",
        severity: Severity::Medium,
        us: true,
        message: "postal code validated as a US ZIP (5 digits): UK, Canada, Netherlands, Japan… are rejected",
        fix: "validate per country (or accept free text); label it \"postal code\"",
        hit: |l, c| {
            any(
                l,
                &[r"\d{5}(-\d{4})", r"\d{5}(?:-\d{4})", "[0-9]{5}(-[0-9]{4})"],
            ) || (any(l, &[r"\d{5}", "[0-9]{5}"]) && any(c, &["zip", "postal", "postcode"]))
        },
    },
    Rule {
        id: "required_state",
        severity: Severity::Medium,
        us: true,
        message: "required \"state\" field or US state list: most countries have no state in addresses",
        fix: "make region optional and country-dependent (address fields per country, e.g. Google libaddressinput data)",
        hit: |_, c| {
            (any(
                c,
                &[
                    "name=\"state\"",
                    "name='state'",
                    "id=\"state\"",
                    "state:z.",
                    "state:yup.",
                    "\"state\"]",
                    "'state']",
                ],
            ) && any(c, &["required", ".min(1", "nonempty"]))
                || any(c, &["us_states", "usstates", "\"alabama\",\"alaska\""])
        },
    },
    Rule {
        id: "required_name_split",
        severity: Severity::Low,
        us: false,
        message: "first + last name both required: mononyms, family-name-first orders and multi-part names don't fit",
        fix: "one \"full name\" field (plus an optional \"what should we call you\"), or make the family name optional",
        hit: |_, c| {
            any(c, &["firstname", "first_name", "lastname", "last_name"])
                && any(c, &["required", ".min(1", "nonempty"])
        },
    },
    Rule {
        id: "week_start",
        severity: Severity::Medium,
        us: true,
        message: "week hard-coded to start on Sunday: Monday in most of Europe, Saturday in parts of the Middle East",
        fix: "new Intl.Locale(locale).getWeekInfo?.().firstDay (date-fns: the locale's options.weekStartsOn)",
        hit: |_, c| {
            any(
                c,
                &[
                    "weekstartson:0",
                    "weekstartson={0}",
                    "firstdayofweek:0",
                    "firstdayofweek={0}",
                    "firstday:0",
                    "[\"sun\",\"mon\"",
                    "['sun','mon'",
                    "[\"sunday\",\"monday\"",
                    "['sunday','monday'",
                    "[\"s\",\"m\",\"t\"",
                    "['s','m','t'",
                ],
            )
        },
    },
    Rule {
        id: "english_plural",
        severity: Severity::High,
        us: false,
        message: "plural built by appending \"s\": wrong in most languages (Polish has 3 forms, Arabic 6)",
        fix: "ICU plural in the message: {count, plural, one {# day} other {# days}}",
        hit: |_, c| {
            any(
                c,
                &[
                    "?\"\":\"s\"",
                    "?'':'s'",
                    "?\"s\":\"\"",
                    "?'s':''",
                    "?``:`s`",
                    "?`s`:``",
                ],
            )
        },
    },
];

/// Concatenation is checked apart: it needs the project's translation functions.
const CONCAT: (&str, &str, &str) = (
    "concatenation",
    "sentence assembled with `+` or from translated fragments: word order and agreement differ per language",
    "one message with placeholders: t(\"invoice.sent\", { number }) → \"Invoice {number} sent\"",
);

/// Rules over one source file.
pub fn scan_code(path: &str, text: &str, functions: &[&str]) -> Vec<Hit> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || ["//", "/*", "*", "#"].iter().any(|p| t.starts_with(p)) || t.len() > 400
        {
            continue;
        }
        let compact = t.to_lowercase().replace(' ', "");
        let mut push = |rule, severity, us, message, fix| {
            out.push(Hit {
                rule,
                severity,
                location: format!("{path}:{}", n + 1),
                snippet: t.chars().take(120).collect(),
                message,
                fix,
                us_centric: us,
            })
        };
        for r in RULES {
            if (r.hit)(t, &compact) {
                push(r.id, r.severity, r.us, r.message, r.fix);
            }
        }
        if concatenation(t, functions) {
            push(CONCAT.0, Severity::High, false, CONCAT.1, CONCAT.2);
        }
    }
    out
}

/// Rules over the source catalog (text, not code).
pub fn scan_catalog(entries: &std::collections::BTreeMap<String, String>) -> Vec<Hit> {
    let mut out = vec![];
    for (key, v) in entries {
        let compact = v.to_lowercase().replace(' ', "");
        for r in RULES.iter().filter(|r| {
            matches!(
                r.id,
                "currency_symbol" | "imperial_units" | "us_date_format" | "us_phone"
            )
        }) {
            if (r.hit)(v, &compact) {
                out.push(Hit {
                    rule: r.id,
                    severity: r.severity,
                    location: format!("key {key}"),
                    snippet: v.chars().take(120).collect(),
                    message: r.message,
                    fix: r.fix,
                    us_centric: r.us,
                });
            }
        }
        // `{price}` / `{date}` passed as a bare argument: formatted (or not) by the caller, in whatever locale.
        let bare: Vec<&str> = v
            .split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(a, _)| a.trim()))
            .filter(|a| {
                let a = a.to_lowercase();
                // plural/select branches (`{# overdue invoice}`) are not arguments
                a.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && ["price", "amount", "total", "cost", "balance", "date", "due"]
                        .iter()
                        .any(|w| a.contains(w))
            })
            .collect();
        if !bare.is_empty() {
            out.push(Hit {
                rule: "unformatted_argument",
                severity: Severity::Low,
                location: format!("key {key}"),
                snippet: v.chars().take(120).collect(),
                message: "money/date argument without an ICU format: its format depends on the caller's locale",
                fix: "{amount, number, ::currency/EUR} / {date, date, medium}, or format it with the active locale before passing it",
                us_centric: false,
            });
        }
    }
    out
}

const EU_REGIONS: &[&str] = &[
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT", "LV",
    "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE",
];
const EU_LANGS: &[&str] = &[
    "de", "fr", "es", "it", "nl", "pl", "pt", "sv", "da", "fi", "cs", "sk", "sl", "hu", "ro", "bg",
    "hr", "el", "et", "lv", "lt", "ga", "mt",
];

fn split_locale(loc: &str) -> (String, Option<String>) {
    let base = glot_core::variants::split(loc).map_or(loc, |(b, _)| b);
    let mut parts = base.split(['-', '_']);
    let lang = parts.next().unwrap_or_default().to_lowercase();
    let region = parts.find(|p| p.len() == 2).map(str::to_uppercase);
    (lang, region)
}

fn is_us(loc: &str) -> bool {
    split_locale(loc) == ("en".into(), Some("US".into()))
}

/// Whole-word (or CJK substring) presence in a lowercase blob.
fn word(blob: &str, w: &str) -> bool {
    blob.match_indices(w).any(|(i, _)| {
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() || !c.is_ascii());
        edge(blob[..i].chars().last()) && edge(blob[i + w.len()..].chars().next())
    })
}

/// Market-level legal expectations (not legal advice: a hit means "no wording found", not "non-compliant").
pub fn legal(loc: &str, blob: &str, has_prices: bool) -> Vec<Hit> {
    let (lang, region) = split_locale(loc);
    let eu = match &region {
        Some(r) => EU_REGIONS.contains(&r.as_str()),
        None => EU_LANGS.contains(&lang.as_str()),
    };
    let found = |ws: &[&str]| ws.iter().any(|w| word(blob, w));
    let mut out = vec![];
    let mut push = |rule, severity, message, fix| {
        out.push(Hit {
            rule,
            severity,
            location: "market".into(),
            snippet: String::new(),
            message,
            fix,
            us_centric: false,
        })
    };
    if eu
        && !found(&[
            "gdpr",
            "rgpd",
            "dsgvo",
            "datenschutz",
            "privacy",
            "confidentialité",
            "privacidad",
            "riservatezza",
            "privacybeleid",
            "prywatności",
            "cookies",
            "cookie",
        ])
    {
        push(
            "legal_privacy",
            Severity::High,
            "EU market: no privacy policy / GDPR / cookie consent wording found in the catalogs",
            "link a localized privacy policy and cookie consent (GDPR art. 12-13: information in a language the user understands)",
        );
    }
    if eu
        && has_prices
        && !found(&[
            "vat",
            "tva",
            "ttc",
            "ht",
            "hors taxes",
            "mwst",
            "mwst.",
            "ust",
            "umsatzsteuer",
            "iva",
            "btw",
            "moms",
            "alv",
            "dph",
            "áfa",
            "tax",
            "taxes",
            "steuer",
        ])
    {
        push(
            "legal_vat",
            Severity::High,
            "EU market: prices shown without VAT wording (consumers must see the price incl. VAT: Directive 98/6/EC)",
            "show prices incl. VAT and say so: \"TTC\", \"inkl. MwSt.\", \"IVA incluida\" (or \"excl. VAT\" for B2B only)",
        );
    }
    if lang == "de"
        && region.as_deref().is_none_or(|r| r == "DE" || r == "AT")
        && !found(&["impressum"])
    {
        push(
            "legal_impressum",
            Severity::High,
            "German-speaking market: no Impressum found (§ 5 DDG in Germany, § 25 MedienG in Austria)",
            "add an \"Impressum\" page linked from every page (provider name, address, contact, register, VAT ID)",
        );
    }
    if lang == "ja" && has_prices && !found(&["特定商取引法", "特商法", "tokushoho"]) {
        push(
            "legal_tokushoho",
            Severity::High,
            "Japanese market: paid offer without a 特定商取引法に基づく表記 page (Act on Specified Commercial Transactions)",
            "add a 特定商取引法に基づく表記 page: seller, address, phone, prices, payment, delivery, cancellation terms",
        );
    }
    if lang == "fr"
        && region.as_deref().is_none_or(|r| r == "FR")
        && !found(&["mentions légales", "mentions legales", "legal notice"])
    {
        push(
            "legal_mentions",
            Severity::Medium,
            "French market: no \"Mentions légales\" found (LCEN art. 6-III)",
            "add a \"Mentions légales\" page (publisher, host, contact)",
        );
    }
    if loc.starts_with("pt-BR") && !found(&["lgpd"]) {
        push(
            "legal_lgpd",
            Severity::Medium,
            "Brazilian market: no LGPD mention found",
            "mention LGPD in the privacy policy and consent wording",
        );
    }
    out
}

fn weight(s: Severity) -> u32 {
    match s {
        Severity::High => 10,
        Severity::Medium => 4,
        Severity::Low => 1,
    }
}

/// 100 minus the weighted findings; one rule (or LLM category) costs at most 25 so a repeated pattern
/// does not hide everything else.
pub fn score(items: &[(&str, Severity)]) -> u8 {
    let mut per: std::collections::BTreeMap<&str, u32> = Default::default();
    for (rule, s) in items {
        *per.entry(rule).or_default() += weight(*s);
    }
    let lost: u32 = per.values().map(|p| (*p).min(25)).sum();
    100u32.saturating_sub(lost) as u8
}

const CODE_EXT: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "astro", "html",
];
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "dist",
    "build",
    "out",
    "target",
    "coverage",
    "vendor",
    ".glot",
    ".turbo",
    ".vercel",
    "public",
    "__tests__",
];

/// Code files under `root` (relative paths, sorted). Tests, minified and generated files are skipped.
fn code_files(root: &Path) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let (p, name) = (e.path(), e.file_name().to_string_lossy().to_string());
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push(p);
                }
            } else if ft.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| CODE_EXT.contains(&x))
                && !any(&name, &[".test.", ".spec.", ".min.", ".stories.", ".d.ts"])
                && e.metadata().is_ok_and(|m| m.len() < 300_000)
                && let Ok(rel) = p.strip_prefix(root)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

#[derive(Serialize)]
struct Market {
    locale: String,
    score: u8,
    legal: Vec<Hit>,
    findings: Vec<culture::Finding>,
}

#[derive(Serialize)]
struct Report {
    code: Vec<Hit>,
    markets: Vec<Market>,
}

fn sev(s: Severity) -> String {
    match s {
        Severity::High => format!("{RED}HIGH  {RED:#}"),
        Severity::Medium => format!("{YELLOW}MEDIUM{YELLOW:#}"),
        Severity::Low => format!("{DIM}LOW   {DIM:#}"),
    }
}

fn markdown(r: &Report) -> String {
    let mut s = String::from(
        "# Cultural adaptation report (`glot labs culture`)\n\n| Market | Ready | High | Medium | Low |\n|---|---|---|---|---|\n",
    );
    let count = |m: &Market, code: &[Hit], x: Severity| {
        code.iter()
            .filter(|h| !(h.us_centric && is_us(&m.locale)))
            .chain(&m.legal)
            .filter(|h| h.severity == x)
            .count()
            + m.findings.iter().filter(|f| f.raw.severity == x).count()
    };
    for m in &r.markets {
        s += &format!(
            "| {} | {}/100 | {} | {} | {} |\n",
            m.locale,
            m.score,
            count(m, &r.code, Severity::High),
            count(m, &r.code, Severity::Medium),
            count(m, &r.code, Severity::Low)
        );
    }
    s += "\n## Code and catalogs (every market)\n\n";
    for h in &r.code {
        s += &format!(
            "- **{:?}** `{}` {} — {}  \n  `{}`  \n  fix: {}\n",
            h.severity, h.location, h.rule, h.message, h.snippet, h.fix
        );
    }
    for m in &r.markets {
        s += &format!("\n## {} — ready {}/100\n\n", m.locale, m.score);
        for h in &m.legal {
            s += &format!(
                "- **{:?}** {} — {}  \n  fix: {}\n",
                h.severity, h.rule, h.message, h.fix
            );
        }
        for f in &m.findings {
            s += &format!(
                "- **{:?}** {} `{}` — {} ({})  \n  source: {:?}  \n  current: {:?}  \n  proposal: {:?}{}\n",
                f.raw.severity,
                f.raw.category,
                f.key,
                f.raw.problem,
                f.raw.why,
                f.source,
                f.target.as_deref().unwrap_or("—"),
                f.raw.proposal.as_deref().unwrap_or("— (fix in code)"),
                if f.rejected.is_empty() {
                    String::new()
                } else {
                    format!(" — rejected: {}", f.rejected.join("; "))
                }
            );
        }
    }
    s + "\n_Legal checks look for wording only: a clean report means \"no known gap\", not \"compliant\" (not legal advice)._\n"
}

pub async fn run(a: CultureArgs) -> Result<ExitCode> {
    let mut ws = open()?;
    let markets: Vec<String> = ws
        .config
        .target_locales
        .iter()
        .filter(|l| a.locale.is_empty() || a.locale.contains(l))
        .cloned()
        .collect();
    if markets.is_empty() {
        anyhow::bail!("no target locale to audit (check --locale and target_locales)");
    }
    let fns: Vec<&str> = ws
        .config
        .extract
        .functions
        .iter()
        .map(String::as_str)
        .collect();
    let files = code_files(&ws.root);
    let mut code = vec![];
    for f in &files {
        if let Ok(text) = std::fs::read_to_string(ws.root.join(f)) {
            code.extend(scan_code(f, &text, &fns));
        }
    }
    code.extend(scan_catalog(&ws.source().entries));
    code.sort_by(|a, b| (a.severity, &a.location).cmp(&(b.severity, &b.location)));

    let (findings, usage) = if a.no_llm {
        (vec![], Default::default())
    } else {
        let keys = (!a.keys.is_empty()).then_some(a.keys.as_slice());
        culture::audit_workspace(&ws, Some(&markets), keys, a.refresh).await?
    };

    let has_prices = code
        .iter()
        .any(|h| matches!(h.rule, "currency_symbol" | "fixed_currency"))
        || ws.source().entries.iter().any(|(k, v)| {
            any(
                &k.to_lowercase(),
                &["pric", "billing", "checkout", "subscription"],
            ) || any(v, &["$", "€", "£", "¥"])
        });
    let mut report = Report {
        code,
        markets: vec![],
    };
    for loc in &markets {
        let mut blob: Vec<&str> = ws.source().entries.values().map(String::as_str).collect();
        if let Some(c) = ws.catalogs.get(loc) {
            blob.extend(c.entries.values().map(String::as_str));
        }
        blob.extend(files.iter().map(String::as_str));
        let blob = blob.join("\n").to_lowercase();
        let legal = legal(loc, &blob, has_prices);
        let mine: Vec<culture::Finding> = findings
            .iter()
            .filter(|f| &f.locale == loc)
            .cloned()
            .collect();
        let scored: Vec<(&str, Severity)> = report
            .code
            .iter()
            .filter(|h| !(h.us_centric && is_us(loc)))
            .chain(&legal)
            .map(|h| (h.rule, h.severity))
            .chain(
                mine.iter()
                    .map(|f| (f.raw.category.as_str(), f.raw.severity)),
            )
            .collect();
        report.markets.push(Market {
            locale: loc.clone(),
            score: score(&scored),
            legal,
            findings: mine,
        });
    }
    let dir = ws.root.join(".glot/culture");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("report.md"), markdown(&report))?;
    std::fs::write(
        dir.join("report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;

    if a.json {
        print_json(&report)?;
    } else {
        print_text(&report);
        if !a.no_llm {
            anstream::eprintln!(
                "{DIM}LLM cost ${:.4} ({} in / {} out tokens) · report: .glot/culture/report.md{DIM:#}",
                usage.cost_usd,
                usage.input_tokens,
                usage.output_tokens
            );
        }
    }

    if a.apply {
        apply(&mut ws, &findings, a.yes)?;
    }
    let below = a
        .min_score
        .is_some_and(|min| report.markets.iter().any(|m| m.score < min));
    Ok(if below {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn print_text(r: &Report) {
    const SHOW: usize = 40;
    anstream::println!(
        "{BOLD}Code and catalogs{BOLD:#} {DIM}(count for every market; US conventions not for en-US){DIM:#}  {} findings",
        r.code.len()
    );
    for h in r.code.iter().take(SHOW) {
        anstream::println!(
            "  {} {BOLD}{}{BOLD:#}  {}  {DIM}{}{DIM:#}",
            sev(h.severity),
            h.location,
            h.rule,
            h.message
        );
        anstream::println!(
            "         {DIM}{}{DIM:#}\n         {GREEN}fix{GREEN:#} {}",
            h.snippet,
            h.fix
        );
    }
    if r.code.len() > SHOW {
        anstream::println!(
            "  {DIM}… {} more in .glot/culture/report.md{DIM:#}",
            r.code.len() - SHOW
        );
    }
    for m in &r.markets {
        let color = match m.score {
            80.. => GREEN,
            50..80 => YELLOW,
            _ => RED,
        };
        anstream::println!(
            "\n{BOLD}{}{BOLD:#}  ready {color}{}/100{color:#}",
            m.locale,
            m.score
        );
        for h in &m.legal {
            anstream::println!(
                "  {} {}  {}\n         {GREEN}fix{GREEN:#} {}",
                sev(h.severity),
                h.rule,
                h.message,
                h.fix
            );
        }
        for f in m.findings.iter().take(SHOW) {
            anstream::println!(
                "  {} {} {BOLD}{}{BOLD:#}  {}  {DIM}{}{DIM:#}",
                sev(f.raw.severity),
                f.raw.category,
                f.key,
                f.raw.problem,
                f.raw.why
            );
            match (&f.raw.proposal, f.rejected.is_empty()) {
                (Some(p), true) => anstream::println!("         {GREEN}→{GREEN:#} {p:?}"),
                (Some(p), false) => anstream::println!(
                    "         {RED}✗{RED:#} {p:?} {DIM}rejected: {}{DIM:#}",
                    f.rejected.join("; ")
                ),
                (None, _) => anstream::println!("         {DIM}(fix in code){DIM:#}"),
            }
        }
        if m.findings.len() > SHOW {
            anstream::println!(
                "  {DIM}… {} more in .glot/culture/report.md{DIM:#}",
                m.findings.len() - SHOW
            );
        }
    }
}

fn apply(ws: &mut Workspace, findings: &[culture::Finding], yes: bool) -> Result<()> {
    let todo: Vec<&culture::Finding> = findings
        .iter()
        .filter(|f| f.rejected.is_empty() && f.raw.proposal.is_some())
        .collect();
    if todo.is_empty() {
        anstream::println!("{DIM}no valid proposal to apply{DIM:#}");
        return Ok(());
    }
    anstream::println!(
        "\n{BOLD}Proposals to write as suggestions{BOLD:#} (review them with `glot labs queue`):"
    );
    for f in &todo {
        anstream::println!(
            "  [{}] {}  {:?} → {:?}",
            f.locale,
            f.key,
            f.target.as_deref().unwrap_or("—"),
            f.raw.proposal.as_deref().unwrap_or_default()
        );
    }
    if !yes && !confirm("Write these suggestions?")? {
        return Ok(());
    }
    let res = culture::apply(ws, findings)?;
    let n = |x: Applied| res.iter().filter(|r| **r == x).count();
    anstream::println!(
        "{GREEN}✓{GREEN:#} {} suggestions written (origin transcreation, waiting in `glot labs queue`) · {} protected (human/reviewed, use `glot set`)",
        n(Applied::Suggested),
        n(Applied::Protected)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(line: &str) -> Vec<&'static str> {
        scan_code("f.tsx", line, &["t"])
            .into_iter()
            .map(|h| h.rule)
            .collect()
    }

    #[test]
    fn code_rules() {
        let cases: &[(&str, &[&str])] = &[
            (
                r#"return new Date(iso).toLocaleDateString("en-US", { month: "short" });"#,
                &["hardcoded_locale"],
            ),
            (
                r#"new Intl.NumberFormat("en-US", { style: "currency", currency }).format(a)"#,
                &["hardcoded_locale"],
            ),
            (
                r#"export function formatMoney(amount: number, currency = "USD") {"#,
                &["fixed_currency"],
            ),
            (
                r#"<span className="text-4xl font-bold">${price}</span>"#,
                &["currency_symbol"],
            ),
            ("toast(`Total: $${total}`)", &["currency_symbol"]),
            ("toast(`Invoice ${number} marked as paid`)", &[]),
            (
                r#"{ href: "/settings/billing", label: "Billing", icon: "$" },"#,
                &[],
            ),
            (
                r#"return n + " " + word + (n === 1 ? "" : "s");"#,
                &["english_plural"],
            ),
            (r#"const msg = "Hello " + name;"#, &["concatenation"]),
            (r#"const msg = t("hello") + name;"#, &["concatenation"]),
            (r#"<p>{`${t("a")} ${count}`}</p>"#, &["concatenation"]),
            (r#"const x = format(d, "MM/dd/yyyy");"#, &["us_date_format"]),
            (r#"placeholder="10/31/2026""#, &["us_date_format"]),
            (
                r#"<p>Free shipping within 50 miles</p>"#,
                &["imperial_units"],
            ),
            (r#"<p>Takes 5 minutes</p>"#, &[]),
            (
                r#"phone: z.string().regex(/^\(\d{3}\) \d{3}-\d{4}$/),"#,
                &["us_phone"],
            ),
            (r#"zip: z.string().regex(/^\d{5}(-\d{4})?$/),"#, &["us_zip"]),
            (r#"<input name="state" required />"#, &["required_state"]),
            (r#"firstName: z.string().min(1),"#, &["required_name_split"]),
            (r#"<Calendar weekStartsOn={0} />"#, &["week_start"]),
            (r#"<td>{amount.toFixed(2)}</td>"#, &["unformatted_number"]),
            (r#"// toLocaleDateString("en-US")"#, &[]),
            ("✕", &[]),
            ("'Authorization': 'Bearer ' + token,", &[]),
            ("  } ${!isPremium ? 'opacity-50' : ''}`}", &[]),
            (
                "url(https://x.com/p?ixid=M3w3NTY2NzF8MHwxfHNlYXJjaHwxfHxh)",
                &[],
            ),
            (
                r#"<p className="x">€{pack.price}</p>"#,
                &["currency_symbol"],
            ),
        ];
        for (line, want) in cases {
            assert_eq!(rules(line), *want, "{line}");
        }
    }

    #[test]
    fn catalog_rules() {
        let mut e = std::collections::BTreeMap::new();
        e.insert("plan.price".to_string(), "${price}/month".to_string());
        e.insert("due".to_string(), "Due on {date}".to_string());
        e.insert("ok".to_string(), "Due on {date, date, medium}".to_string());
        e.insert(
            "plural".to_string(),
            "{n, plural, one {# overdue item} other {# overdue items}}".to_string(),
        );
        let r: Vec<_> = scan_catalog(&e)
            .into_iter()
            .map(|h| (h.rule, h.location))
            .collect();
        assert_eq!(
            r,
            [
                ("unformatted_argument", "key due".to_string()),
                ("currency_symbol", "key plan.price".to_string()),
                ("unformatted_argument", "key plan.price".to_string()),
            ]
        );
    }

    #[test]
    fn legal_and_score() {
        let rules = |loc, blob: &str, prices| -> Vec<&str> {
            legal(loc, blob, prices)
                .into_iter()
                .map(|h| h.rule)
                .collect()
        };
        assert_eq!(
            rules("de", "preise", true),
            ["legal_privacy", "legal_vat", "legal_impressum"]
        );
        // Switzerland is outside the EU, Impressum is a DE/AT rule.
        assert!(rules("de-CH", "", true).is_empty());
        assert!(rules("de", "impressum · datenschutz · inkl. mwst.", true).is_empty());
        assert_eq!(
            rules(
                "fr-x-informal",
                "cookies · prix ttc · mentions légales",
                true
            ),
            Vec::<&str>::new()
        );
        // "vat" inside "private" is not VAT wording.
        assert_eq!(
            rules("fr", "private · cookies · mentions légales", true),
            ["legal_vat"]
        );
        assert_eq!(rules("ja", "", true), ["legal_tokushoho"]);
        assert!(rules("ja", "特定商取引法に基づく表記", true).is_empty());
        assert!(rules("en-US", "", true).is_empty());
        assert_eq!(score(&[]), 100);
        assert_eq!(score(&[("a", Severity::High), ("b", Severity::Low)]), 89);
        // One rule costs at most 25.
        assert_eq!(score(&[("a", Severity::High); 10]), 75);
    }
}
