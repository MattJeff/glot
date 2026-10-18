//! "Which language next?": ranks candidate locales by observed demand (glot insights), public market
//! data, translation cost and technical effort. Pure: callers pass demand and a cost function
//! (`glot labs markets` from the local catalog, the server from its keys).
use serde::Serialize;

/// One candidate market. Public data, approximate (±20 %), as of 2025-10:
/// - `users_m`: internet users by first language, millions. Internet World Stats "Internet users by
///   language" (2024 edition), cross-checked with ITU Facts and Figures 2024 / DataReportal Digital 2025.
/// - `ppp_k`: GDP per capita at PPP of the main market(s), thousand international $. IMF World Economic
///   Outlook, April 2025 (population-weighted for multi-country languages, rounded).
/// - `ecom_pct`: share of global retail e-commerce sales, %. eMarketer "Worldwide Retail Ecommerce
///   Forecast 2024" and Statista Digital Market Outlook 2025 (main markets summed, rounded).
///
/// ponytail: static table, refresh yearly; one entry per primary language because insights store
/// the primary subtag only (`zh-TW` vs `zh-CN` cannot be told apart from Accept-Language counts).
pub struct Market {
    pub locale: &'static str,
    pub name: &'static str,
    pub users_m: f64,
    pub ppp_k: f64,
    pub ecom_pct: f64,
    pub rtl: bool,
    /// Script that usually needs an extra webfont / fallback stack.
    pub font: Option<&'static str>,
}

const fn m(
    locale: &'static str,
    name: &'static str,
    users_m: f64,
    ppp_k: f64,
    ecom_pct: f64,
    rtl: bool,
    font: Option<&'static str>,
) -> Market {
    Market {
        locale,
        name,
        users_m,
        ppp_k,
        ecom_pct,
        rtl,
        font,
    }
}

/// Date of the public data in `MARKETS`.
pub const DATA_AS_OF: &str = "2025-10";

const CJK: Option<&str> = Some("CJK webfont (large, subset it)");
#[rustfmt::skip]
pub const MARKETS: &[Market] = &[
    m("en", "English", 1190.0, 55.0, 24.0, false, None),
    m("zh-CN", "Chinese (Simplified)", 1100.0, 27.0, 45.0, false, CJK),
    m("es", "Spanish", 390.0, 27.0, 3.0, false, None),
    m("hi", "Hindi", 300.0, 11.0, 1.2, false, Some("Devanagari font")),
    m("ar", "Arabic", 240.0, 22.0, 1.5, true, Some("Arabic font")),
    m("id", "Indonesian", 220.0, 16.0, 1.5, false, None),
    m("pt-BR", "Portuguese (Brazil)", 190.0, 22.0, 1.6, false, None),
    m("fr", "French", 150.0, 52.0, 2.6, false, None),
    m("ru", "Russian", 125.0, 45.0, 1.5, false, None),
    m("ja", "Japanese", 118.0, 53.0, 3.4, false, CJK),
    m("de", "German", 92.0, 70.0, 2.8, false, None),
    m("vi", "Vietnamese", 80.0, 17.0, 0.5, false, None),
    m("tr", "Turkish", 75.0, 45.0, 0.8, false, None),
    m("fa", "Persian", 70.0, 18.0, 0.2, true, Some("Arabic font")),
    m("th", "Thai", 60.0, 25.0, 0.6, false, Some("Thai font")),
    m("it", "Italian", 52.0, 60.0, 1.3, false, None),
    m("ko", "Korean", 50.0, 60.0, 3.0, false, CJK),
    m("pl", "Polish", 33.0, 50.0, 0.8, false, None),
    m("uk", "Ukrainian", 30.0, 18.0, 0.2, false, None),
    m("nl", "Dutch", 24.0, 75.0, 0.9, false, None),
    m("ro", "Romanian", 18.0, 45.0, 0.3, false, None),
    m("cs", "Czech", 10.0, 55.0, 0.3, false, None),
    m("sv", "Swedish", 10.0, 70.0, 0.4, false, None),
    m("he", "Hebrew", 9.0, 55.0, 0.3, true, None),
];

/// Share of the market a foreign web product can realistically sell to, with the reason. glot's judgment,
/// not a public statistic: ICP licence, blocked hosting/payments (China); sanctions and card-payment
/// restrictions since 2022 (Russia, Iran).
fn access(locale: &str) -> (f64, Option<&'static str>) {
    match locale {
        "zh-CN" => (
            0.35,
            Some("restricted market (ICP licence, local payments)"),
        ),
        "ru" => (0.5, Some("restricted market (sanctions, card payments)")),
        "fa" => (0.2, Some("restricted market (sanctions)")),
        _ => (1.0, None),
    }
}

/// Words a reviewer checks per hour (post-editing machine output, common industry range 600-1000).
pub const REVIEW_WORDS_PER_HOUR: f64 = 800.0;

/// Observed demand for one preferred language (insights, last N days).
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
pub struct Demand {
    pub language: String,
    pub visits: i64,
    #[serde(default)]
    pub conversion_rate: Option<f64>,
    /// Extra goal conversions per 30 days if served in their language (insights' observed lift).
    #[serde(default)]
    pub est_extra_conversions: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Opportunity {
    pub locale: String,
    pub name: String,
    /// 0-100, higher is better.
    pub score: u32,
    pub best: bool,
    pub visits: i64,
    /// Share of all visitors whose preferred language is this one.
    pub share: f64,
    pub conversion_rate: Option<f64>,
    pub est_extra_conversions: Option<f64>,
    pub internet_users_m: f64,
    pub gdp_ppp_k: f64,
    pub ecommerce_pct: f64,
    pub cost_usd: f64,
    pub words: usize,
    pub review_hours: f64,
    pub rtl: bool,
    pub font: Option<String>,
    pub plural_forms: usize,
    /// 0 = nothing to do beyond translating; +2 RTL, +1 extra font, +1 few/two/zero plural forms.
    pub effort: u32,
    pub effort_notes: Vec<String>,
    pub summary: String,
}

/// Words in a catalog value, ICU syntax included (close enough for a review estimate).
pub fn words<'a>(values: impl IntoIterator<Item = &'a String>) -> usize {
    values
        .into_iter()
        .map(|v| v.split_whitespace().count())
        .sum()
}

/// Rank every language not already offered. `offered`: locales the project ships (source included).
/// `cost(locale)`: USD to translate the whole catalog into it.
pub fn rank(
    offered: &[String],
    total_visits: i64,
    demand: &[Demand],
    words: usize,
    cost: impl Fn(&str) -> f64,
) -> Vec<Opportunity> {
    let lang = crate::plural::language;
    let served: Vec<String> = offered.iter().map(|l| lang(l)).collect();
    let max_reach = MARKETS
        .iter()
        .map(|m| m.users_m * m.ppp_k)
        .fold(0.0, f64::max);
    let max_ecom = MARKETS.iter().map(|m| m.ecom_pct).fold(0.0, f64::max);
    // How much to trust observed demand over public data: full weight from 500 visits.
    let w = 0.6 * (total_visits as f64 / 500.0).min(1.0);
    // Real demand for a language missing from the table still counts (public data = 0).
    const NONE: Market = m("", "", 0.0, 0.0, 0.0, false, None);
    let known = |l: &str| MARKETS.iter().any(|m| lang(m.locale) == l);
    let all = MARKETS
        .iter()
        .map(|m| (m.locale.to_string(), m.name.to_string(), m))
        .chain(
            demand
                .iter()
                .filter(|d| d.visits > 0 && d.language != "und" && !known(&d.language))
                .map(|d| (d.language.clone(), d.language.clone(), &NONE)),
        );

    let mut out: Vec<Opportunity> = all
        .filter(|(loc, ..)| !served.contains(&lang(loc)))
        .map(|(locale, name, mk)| {
            let d = demand.iter().find(|d| d.language == lang(&locale));
            let visits = d.map_or(0, |d| d.visits);
            let share = visits as f64 / total_visits.max(1) as f64;
            use crate::plural::PluralCategory::{Few, Two, Zero};
            let cats = crate::plural::categories(&locale);
            let plural_forms = cats.len();
            // `many` alone (fr/es/pt: compact millions) is not real complexity; few/two/zero are.
            let hard_plurals = cats.iter().any(|c| matches!(c, Few | Two | Zero));
            let mut notes = vec![];
            if mk.rtl {
                notes.push("right-to-left layout (dir=\"rtl\", mirrored icons)".to_string());
            }
            if let Some(f) = mk.font {
                notes.push(f.to_string());
            }
            if hard_plurals {
                notes.push(format!("{plural_forms} plural forms (CLDR)"));
            }
            let effort = 2 * mk.rtl as u32 + mk.font.is_some() as u32 + hard_plurals as u32;
            let (open, why) = access(&locale);
            notes.extend(why.map(String::from));
            let market = open
                * (0.7 * (mk.users_m * mk.ppp_k / max_reach).sqrt()
                    + 0.3 * (mk.ecom_pct / max_ecom).sqrt());
            let observed = (share / 0.05).min(1.0);
            let base = w * observed + (1.0 - w) * market;
            let cost_usd = cost(&locale);
            Opportunity {
                score: (100.0 * base / (1.0 + 0.15 * effort as f64)).round() as u32,
                best: false,
                visits,
                share,
                conversion_rate: d.and_then(|d| d.conversion_rate),
                est_extra_conversions: d.and_then(|d| d.est_extra_conversions),
                internet_users_m: mk.users_m,
                gdp_ppp_k: mk.ppp_k,
                ecommerce_pct: mk.ecom_pct,
                cost_usd,
                words,
                review_hours: words as f64 / REVIEW_WORDS_PER_HOUR,
                rtl: mk.rtl,
                font: mk.font.map(String::from),
                plural_forms,
                effort,
                effort_notes: notes,
                summary: String::new(),
                locale,
                name,
            }
        })
        .collect();
    out.sort_by(|a, b| b.score.cmp(&a.score).then(b.visits.cmp(&a.visits)));
    if let Some(first) = out.first_mut() {
        first.best = true;
    }
    for o in &mut out {
        o.summary = summary(o, total_visits > 0);
    }
    out
}

fn summary(o: &Opportunity, has_traffic: bool) -> String {
    let why = if o.visits > 0 {
        format!("~{} of your visitors already ask for it", pct(o.share))
    } else if has_traffic {
        format!(
            "none of your visitors ask for it yet, {:.0} M people online",
            o.internet_users_m
        )
    } else {
        format!("{:.0} M people online", o.internet_users_m)
    };
    let effort = if o.effort_notes.is_empty() {
        "no technical effort".to_string()
    } else {
        o.effort_notes.join(", ")
    };
    format!(
        "Add {} ({}): {why}, cost {} (+ ~{} of review), {effort}{}",
        o.name,
        o.locale,
        usd(o.cost_usd),
        duration(o.review_hours),
        if o.best { " → best opportunity" } else { "" }
    )
}

/// `$0.42`, `$0.0005` (tiny catalogs stay visible), `$12.30`.
pub fn usd(x: f64) -> String {
    if x > 0.0 && x < 0.1 {
        format!("${x:.4}")
    } else {
        format!("${x:.2}")
    }
}

/// `1.5 h`, `3 min`.
pub fn duration(hours: f64) -> String {
    if hours < 1.0 {
        format!("{:.0} min", (hours * 60.0).ceil().max(1.0))
    } else {
        format!("{hours:.1} h")
    }
}

fn pct(x: f64) -> String {
    if x < 0.01 {
        format!("{:.1} %", x * 100.0)
    } else {
        format!("{:.0} %", x * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(language: &str, visits: i64) -> Demand {
        Demand {
            language: language.into(),
            visits,
            ..Default::default()
        }
    }

    #[test]
    fn observed_demand_beats_public_data() {
        let offered = ["en".to_string(), "fr".into()];
        // 1000 visits, 6 % ask for Portuguese: it should outrank Chinese despite the bigger market.
        let r = rank(&offered, 1000, &[d("en", 900), d("pt", 60)], 1200, |_| 0.42);
        assert_eq!(r[0].locale, "pt-BR");
        assert!(r[0].best && !r[1].best);
        assert!(r.iter().all(|o| o.locale != "en" && o.locale != "fr"));
        assert_eq!(r[0].effort, 0);
        assert!((r[0].review_hours - 1.5).abs() < 1e-9);
        assert_eq!(
            r[0].summary,
            "Add Portuguese (Brazil) (pt-BR): ~6 % of your visitors already ask for it, cost $0.42 (+ ~1.5 h of review), no technical effort → best opportunity"
        );
    }

    #[test]
    fn effort_and_unknown_languages() {
        let r = rank(&["en".into()], 1000, &[d("sw", 100)], 10, |_| 0.0);
        let ar = r.iter().find(|o| o.locale == "ar").unwrap();
        assert!(ar.rtl && ar.effort == 4, "{ar:?}"); // RTL 2 + font 1 + six plural forms 1
        let ja = r.iter().find(|o| o.locale == "ja").unwrap();
        assert_eq!((ja.effort, ja.plural_forms), (1, 1));
        // Swahili is not in the table but 10 % of visitors want it.
        let sw = r.iter().find(|o| o.locale == "sw").unwrap();
        assert_eq!(sw.visits, 100);
        // No traffic: public data alone, the biggest market (Chinese) wins over small ones.
        let r = rank(&["en".into()], 0, &[], 10, |_| 0.0);
        assert!(r[0].summary.contains("M people online"), "{}", r[0].summary);
        let pos = |l: &str| r.iter().position(|o| o.locale == l).unwrap();
        assert!(pos("zh-CN") < pos("he"));
        // Market access: China's size is discounted (ICP licence), Spanish leads on public data alone.
        assert_eq!(r[0].locale, "es");
        assert!(
            r[pos("zh-CN")]
                .effort_notes
                .iter()
                .any(|n| n.contains("restricted"))
        );
        assert_eq!(duration(0.01), "1 min");
        assert_eq!(usd(0.0005), "$0.0005");
    }
}
