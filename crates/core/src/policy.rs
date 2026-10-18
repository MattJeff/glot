//! Copy policy: deterministic checks run on every generated marketing variant, in every locale, before it can be
//! served (copy experiments, segment personalization). Pure string rules, no LLM: the same text always gets the
//! same verdict, and every finding says what to change.
//!
//! Rules (ids usable in `[policy.rules]`):
//! - `fake_urgency` (error): scarcity / deadline wording ("plus que 2 places", "nur noch heute", "last chance")
//!   that neither the official text nor a declared fact backs.
//! - `fake_counter` (error): live-activity counters in static copy ("12 personnes regardent", "just bought").
//! - `unprovable_claim` (error): rank / guarantee claims ("n°1", "#1", "leader", "garanti", "100 %").
//! - `superlative` (warning): unprovable superlatives ("le meilleur", "best", "beste", "unbeatable").
//! - `unsourced_number` (error): a number, percentage, amount or date absent from the official text and the facts.
//! - `price_or_legal` (error): an amount of the official text changed or removed, or its legal mention dropped
//!   ("TTC", "inkl. MwSt.", "*", "sous conditions").
//! - `confirmshaming` (error): a decline option that shames the user ("Non merci, je préfère payer plus cher").
//! - `forbidden_term` (error): project terms banned on a market (`[policy.forbidden]`).
//!
//! Legal note (not legal advice): these rules target practices the EU already sanctions under the Unfair
//! Commercial Practices Directive 2005/29/EC (Annex I: false "limited time" claims, invented stock levels) and
//! the DSA (art. 25, dark patterns on platforms), and that the announced Digital Fairness Act (Commission
//! proposal expected 2026) is set to name explicitly: fake urgency/scarcity, confirmshaming, misleading social
//! proof. Its final wording is not known: a clean report means "no known red flag", never "compliant". Projects
//! can only tighten the defaults per market; relaxing a rule (`off`) is their own legal call.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::glossary::find;
use crate::validate::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Off,
    Warning,
    Error,
}

/// `[policy]` of glot.toml (or the project settings on the server):
/// ```toml
/// [policy.rules]
/// superlative = "error"     # off | warning | error
/// [policy.forbidden]
/// "*" = ["gratuit à vie"]   # every locale
/// de = ["kostenlos"]         # language
/// de-AT = ["Jänner"]         # exact locale
/// [policy.patterns]
/// fake_urgency = ["offre flash"]   # extra wordings; `#` stands for any number
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    pub rules: BTreeMap<String, Level>,
    pub forbidden: BTreeMap<String, Vec<String>>,
    pub patterns: BTreeMap<String, Vec<String>>,
}

impl PolicyConfig {
    /// The `[policy]` table of a glot.toml (absent = defaults). `Config` ignores the table, so it lives here.
    pub fn from_toml(glot_toml: &str) -> crate::Result<Self> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            policy: PolicyConfig,
        }
        toml::from_str::<File>(glot_toml)
            .map(|f| f.policy)
            .map_err(|e| crate::Error::Invalid(format!("[policy]: {e}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    /// The offending wording, as found.
    pub matched: String,
    /// What to change (English, shown as is in the dashboard and the CLI).
    pub message: String,
}

pub fn has_errors(f: &[Finding]) -> bool {
    f.iter().any(|f| f.severity == Severity::Error)
}

// Lowercase, apostrophes and spaces normalized. `#` = any digit run. Every language is checked whatever the
// locale: a German wording in a French text is still fake urgency, and facts may be written in another language.
const URGENCY: &[&str] = &[
    // en
    "only # left",
    "# left in stock",
    "# spots left",
    "# seats left",
    "last chance",
    "hurry",
    "ends tonight",
    "ends today",
    "ends soon",
    "today only",
    "limited time",
    "while stocks last",
    "selling fast",
    "almost gone",
    "almost sold out",
    "before it's too late",
    "don't miss out",
    "act now",
    "expires soon",
    "last spots",
    // fr
    "plus que #",
    "il ne reste que",
    "il reste #",
    "dernière chance",
    "dernières places",
    "dépêchez-vous",
    "dépêche-toi",
    "vite",
    "se termine ce soir",
    "se termine bientôt",
    "aujourd'hui seulement",
    "temps limité",
    "offre limitée",
    "durée limitée",
    "jusqu'à épuisement",
    "presque épuisé",
    "ne ratez pas",
    "avant qu'il ne soit trop tard",
    "places limitées",
    "expire bientôt",
    // de
    "nur noch #",
    "nur noch heute",
    "nur heute",
    "letzte chance",
    "beeilen sie sich",
    "beeil dich",
    "endet heute",
    "endet bald",
    "zeitlich begrenzt",
    "nur für kurze zeit",
    "solange der vorrat reicht",
    "fast ausverkauft",
    "bevor es zu spät ist",
    "nicht verpassen",
    "letzte plätze",
    "begrenzte plätze",
    // es / it (common markets)
    "solo quedan",
    "última oportunidad",
    "date prisa",
    "solo hoy",
    "ultima occasione",
    "solo oggi",
    "affrettati",
];
// What makes a fact (or the official text) a real deadline or stock level: then urgency wording is honest.
const DEADLINE: &[&str] = &[
    "ends",
    "until",
    "valid until",
    "in stock",
    "seats",
    "spots",
    "se termine",
    "jusqu'au",
    "valable",
    "en stock",
    "places",
    "endet",
    "bis zum",
    "gültig bis",
    "auf lager",
    "plätze",
];
const COUNTER: &[&str] = &[
    "people are viewing",
    "people are looking",
    "people viewing",
    "just bought",
    "just signed up",
    "bought in the last",
    "in the last # hours",
    "are looking at this",
    "personnes regardent",
    "personnes consultent",
    "vient d'acheter",
    "viennent de s'inscrire",
    "achetés dans les dernières",
    "dans les dernières # heures",
    "consultent cette offre",
    "personen sehen",
    "personen schauen",
    "gerade gekauft",
    "gerade angemeldet",
    "in den letzten # stunden",
    "personas están viendo",
    "persone stanno guardando",
];
const CLAIM: &[&str] = &[
    "n°1",
    "n° 1",
    "no. 1",
    "no 1",
    "#1",
    "number one",
    "numéro un",
    "numéro 1",
    "nr. 1",
    "nummer 1",
    "nummer eins",
    "leader",
    "leading",
    "market leader",
    "leader du marché",
    "marktführer",
    "führend",
    "guaranteed",
    "garanti",
    "garantie",
    "garantiert",
    "100 %",
    "100%",
    "best-selling",
    "bestseller",
    "most trusted",
    "le plus fiable",
    "risk-free",
    "sans risque",
    "risikofrei",
    "clinically proven",
    "scientifically proven",
    "prouvé",
    "bewiesen",
];
const SUPERLATIVE: &[&str] = &[
    "best",
    "the most",
    "unbeatable",
    "unmatched",
    "world-class",
    "ultimate",
    "meilleur",
    "meilleure",
    "meilleurs",
    "meilleures",
    "le plus",
    "la plus",
    "imbattable",
    "inégalé",
    "sans égal",
    "beste",
    "bester",
    "bestes",
    "besten",
    "unschlagbar",
    "unübertroffen",
    "weltbeste",
    "el mejor",
    "il migliore",
];
const LEGAL: &[&str] = &[
    "*",
    "ttc",
    "ht",
    "hors taxes",
    "hors tva",
    "tva incluse",
    "sous conditions",
    "voir conditions",
    "cgv",
    "incl. vat",
    "excl. vat",
    "plus vat",
    "vat included",
    "terms apply",
    "conditions apply",
    "t&cs apply",
    "inkl. mwst",
    "inkl. mwst.",
    "zzgl. mwst",
    "zzgl. mwst.",
    "agb",
    "es gelten",
    "bedingungen",
];
const CURRENCY: &[char] = &['€', '$', '£', '¥', '₹'];
const CURRENCY_CODES: &[&str] = &["eur", "usd", "gbp", "chf", "cad", "aud", "jpy"];
// Decline + first person: "No thanks, I don't like saving", "Non merci, je préfère…", "Nein danke, ich…".
const DECLINE: &[&str] = &[
    "no thanks",
    "no, thanks",
    "no",
    "non merci",
    "non",
    "nein danke",
    "nein",
    "no gracias",
    "no grazie",
];
const FIRST_PERSON: &[&str] = &[
    "i", "i'm", "i'd", "i'll", "je", "j'aime", "j'ai", "j'adore", "ich", "yo", "io",
];
const DECLINE_ALONE: &[&str] = &[
    "i don't want",
    "i prefer to pay",
    "i'd rather pay",
    "i like paying",
    "i hate saving",
    "je préfère payer",
    "je n'aime pas",
    "je ne veux pas économiser",
    "j'aime payer",
    "ich zahle lieber",
    "ich verzichte gern",
    "ich will nicht sparen",
];

fn norm(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' | '`' | '´' => '\'',
            '\u{a0}' | '\u{202f}' | '\u{2009}' => ' ',
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Digit runs ("2 000", "4,8", "1.299") with their separators removed: `["2000", "48", "1299"]`.
fn numbers(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (vec![], 0);
    while i < c.len() {
        if !c[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut n = String::new();
        while i < c.len()
            && (c[i].is_ascii_digit()
                || (" .,'\u{a0}\u{202f}".contains(c[i])
                    && c.get(i + 1).is_some_and(char::is_ascii_digit)))
        {
            if c[i].is_ascii_digit() {
                n.push(c[i]);
            }
            i += 1;
        }
        out.push(n);
    }
    out
}

/// `s` with each digit run replaced by `#`.
fn collapse(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (String::new(), 0);
    while i < c.len() {
        if c[i].is_ascii_digit() {
            while i < c.len()
                && (c[i].is_ascii_digit()
                    || (" .,'".contains(c[i]) && c.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            out.push('#');
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// First wording of `list` (+ project extras) found in `text` (normalized + collapsed forms).
fn first<'a>(text: &(String, String), list: &[&'a str], extra: &'a [String]) -> Option<&'a str> {
    let hit = |p: &str| {
        let p = norm(p);
        // Symbols-only patterns ("*", "%") have no word boundary.
        if !p.chars().any(char::is_alphanumeric) {
            return text.0.contains(&p);
        }
        find(&text.0, &p, false, false) || (p.contains('#') && find(&text.1, &p, false, false))
    };
    list.iter()
        .copied()
        .chain(extra.iter().map(String::as_str))
        .find(|p| hit(p))
}

fn has_amount(n: &str) -> bool {
    n.chars().any(|c| CURRENCY.contains(&c))
        || CURRENCY_CODES.iter().any(|c| find(n, c, false, false))
}

/// Checks one variant `text` (in `locale`) against the official text of the same key and locale (`control`)
/// and the experiment's declared `facts` ("2 000 équipes", "note 4,8", "offre valable jusqu'au 31/10").
pub fn check(
    text: &str,
    locale: &str,
    control: &str,
    facts: &[String],
    cfg: &PolicyConfig,
) -> Vec<Finding> {
    let forms = |s: &str| {
        let n = norm(s);
        let c = collapse(&n);
        (n, c)
    };
    let (t, c) = (forms(text), forms(control));
    let facts: Vec<(String, String)> = facts.iter().map(|f| forms(f)).collect();
    let none: Vec<String> = vec![];
    let extra = |rule: &str| cfg.patterns.get(rule).unwrap_or(&none);
    // Backed = the official text or a declared fact uses the same kind of wording.
    let backed = |list: &[&str], rule: &str| {
        first(&c, list, extra(rule)).is_some()
            || facts.iter().any(|f| first(f, list, extra(rule)).is_some())
    };
    let mut out = vec![];
    let mut push = |rule: &str, default: Severity, matched: String, message: String| {
        let severity = match cfg.rules.get(rule) {
            Some(Level::Off) => return,
            Some(Level::Warning) => Severity::Warning,
            Some(Level::Error) => Severity::Error,
            None => default,
        };
        out.push(Finding {
            rule: rule.into(),
            severity,
            matched,
            message,
        });
    };

    if let Some(m) = first(&t, URGENCY, extra("fake_urgency"))
        .filter(|_| !backed(URGENCY, "fake_urgency") && !backed(DEADLINE, "-"))
    {
        push(
            "fake_urgency",
            Severity::Error,
            m.into(),
            format!(
                "\"{m}\": urgency or scarcity with no source. Declare the real deadline or stock as a fact \
             (e.g. \"offer ends 31 Oct\") or remove the wording."
            ),
        );
    }
    if let Some(m) =
        first(&t, COUNTER, extra("fake_counter")).filter(|_| first(&c, COUNTER, &[]).is_none())
    {
        push(
            "fake_counter",
            Severity::Error,
            m.into(),
            format!(
                "\"{m}\": live-activity counter in static copy. Remove it; show real data in the product instead."
            ),
        );
    }
    if let Some(m) = first(&t, CLAIM, extra("unprovable_claim")).filter(|m| {
        let one: [&str; 1] = [m];
        !backed(&one, "unprovable_claim")
    }) {
        push(
            "unprovable_claim",
            Severity::Error,
            m.into(),
            format!(
                "\"{m}\": ranking or guarantee claim. Add its proof as a fact (source, date) or remove it."
            ),
        );
    }
    if let Some(m) = first(&t, SUPERLATIVE, extra("superlative")).filter(|m| {
        let one: [&str; 1] = [m];
        !backed(&one, "superlative")
    }) {
        push(
            "superlative",
            Severity::Warning,
            m.into(),
            format!(
                "\"{m}\": superlative that cannot be proven. Prefer a concrete benefit or a declared fact."
            ),
        );
    }

    let known: Vec<String> = numbers(&c.0)
        .into_iter()
        .chain(facts.iter().flat_map(|f| numbers(&f.0)))
        .collect();
    for n in numbers(&t.0) {
        if !known.contains(&n) {
            push(
                "unsourced_number",
                Severity::Error,
                n.clone(),
                format!(
                    "\"{n}\" is in neither the official text nor the facts. Declare it as a fact or remove it."
                ),
            );
        }
    }

    if has_amount(&c.0) {
        let mine = numbers(&t.0);
        if let Some(n) = numbers(&c.0).into_iter().find(|n| !mine.contains(n)) {
            push(
                "price_or_legal",
                Severity::Error,
                n.clone(),
                format!(
                    "the official text states the amount \"{n}\": keep it unchanged (prices are never tested)."
                ),
            );
        }
    }
    if let Some(l) = LEGAL
        .iter()
        .find(|l| first(&c, &[l], &[]).is_some() && first(&t, &[l], &[]).is_none())
    {
        push(
            "price_or_legal",
            Severity::Error,
            l.to_string(),
            format!("the official text carries the legal mention \"{l}\": keep it in the variant."),
        );
    }

    let shames = DECLINE.iter().any(|d| {
        t.0.trim_start().strip_prefix(d).is_some_and(|rest| {
            let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || ",.!—-:".contains(c));
            FIRST_PERSON.iter().any(|p| {
                find(
                    rest.split_whitespace().next().unwrap_or(""),
                    p,
                    false,
                    false,
                ) || rest.starts_with(&format!("{p}'"))
            })
        })
    }) || first(&t, DECLINE_ALONE, extra("confirmshaming")).is_some();
    if shames && first(&c, DECLINE_ALONE, &[]).is_none() {
        push("confirmshaming", Severity::Error, text.trim().into(),
            "a decline option must not shame the user: use a neutral \"No thanks\" / \"Non merci\" / \"Nein danke\"."
                .into());
    }

    let lang = locale.split(['-', '_']).next().unwrap_or(locale);
    let mut banned: Vec<&String> = vec![];
    for scope in ["*", lang, locale] {
        banned.extend(cfg.forbidden.get(scope).into_iter().flatten());
    }
    if let Some(m) = banned
        .into_iter()
        .find(|w| find(&t.0, &norm(w), false, true))
    {
        push(
            "forbidden_term",
            Severity::Error,
            m.clone(),
            format!(
                "\"{m}\" is banned for {locale} by the project policy ([policy.forbidden]). Rephrase without it."
            ),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str, locale: &str, control: &str, facts: &[&str]) -> Vec<String> {
        let facts: Vec<String> = facts.iter().map(|s| s.to_string()).collect();
        check(text, locale, control, &facts, &PolicyConfig::default())
            .into_iter()
            .map(|f| f.rule)
            .collect()
    }

    #[test]
    fn clean_copy_passes_in_three_languages() {
        assert!(rules("Start your free trial", "en", "Get started", &[]).is_empty());
        assert!(
            rules(
                "Rejoignez 2 000 équipes",
                "fr",
                "Commencer",
                &["2,000 teams"]
            )
            .is_empty()
        );
        assert!(
            rules(
                "Für Teams in Deutschland gemacht",
                "de",
                "Jetzt starten",
                &[]
            )
            .is_empty()
        );
        assert!(rules("Non merci", "fr", "Plus tard", &[]).is_empty());
    }

    #[test]
    fn fake_urgency_and_counters() {
        assert_eq!(
            rules("Plus que 2 places !", "fr", "Réservez", &[]),
            ["fake_urgency", "unsourced_number"]
        );
        assert_eq!(
            rules("Only 3 left – hurry", "en", "Book now", &[]),
            ["fake_urgency", "unsourced_number"]
        );
        assert_eq!(
            rules("Nur noch heute: jetzt starten", "de", "Jetzt starten", &[]),
            ["fake_urgency"]
        );
        // Honest urgency: the deadline is a declared fact.
        assert!(
            rules(
                "L'offre se termine bientôt",
                "fr",
                "Profitez de l'offre",
                &["Offre de lancement : se termine le 31/10"]
            )
            .is_empty()
        );
        assert_eq!(
            rules(
                "12 personnes regardent cette offre",
                "fr",
                "Voir l'offre",
                &[]
            ),
            ["fake_counter", "unsourced_number"]
        );
        assert_eq!(
            rules("Gerade gekauft von Anna", "de", "Kaufen", &[]),
            ["fake_counter"]
        );
        // "plus que jamais" is not scarcity.
        assert!(rules("Plus simple que jamais", "fr", "Simple", &[]).is_empty());
    }

    #[test]
    fn claims_superlatives_numbers() {
        assert_eq!(
            rules("L'outil n°1 des équipes", "fr", "L'outil des équipes", &[]),
            ["unprovable_claim", "unsourced_number"]
        );
        assert_eq!(
            rules("Der Marktführer für Übersetzung", "de", "Übersetzung", &[]),
            ["unprovable_claim"]
        );
        let f = check(
            "The best way to ship",
            "en",
            "Ship faster",
            &[],
            &PolicyConfig::default(),
        );
        assert_eq!(
            (f[0].rule.as_str(), f[0].severity),
            ("superlative", Severity::Warning)
        );
        assert_eq!(
            rules("Doublez vos ventes en 30 jours", "fr", "Vendez plus", &[]),
            ["unsourced_number"]
        );
        assert_eq!(
            rules(
                "Bewertet mit 4,8 von 5",
                "de",
                "Beliebt",
                &["rating 4.8 / 5"]
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn price_and_legal_mentions_are_frozen() {
        assert_eq!(
            rules("Seulement 19 € par mois", "fr", "29 € TTC par mois", &[]),
            ["unsourced_number", "price_or_legal", "price_or_legal"]
        );
        assert!(
            rules(
                "29 € TTC / mois, sans engagement",
                "fr",
                "29 € TTC par mois",
                &[]
            )
            .is_empty()
        );
        assert_eq!(
            rules("Ab 29 € im Monat", "de", "29 € pro Monat inkl. MwSt.", &[]),
            ["price_or_legal"]
        );
        assert_eq!(
            rules("Start for $49", "en", "$49/month*", &[]),
            ["price_or_legal"]
        );
    }

    #[test]
    fn confirmshaming() {
        assert_eq!(
            rules(
                "No thanks, I don't like saving money",
                "en",
                "No thanks",
                &[]
            ),
            ["confirmshaming"]
        );
        assert_eq!(
            rules(
                "Non merci, je préfère payer plus cher",
                "fr",
                "Non merci",
                &[]
            ),
            ["confirmshaming"]
        );
        assert_eq!(
            rules("Nein danke, ich zahle lieber mehr", "de", "Nein danke", &[]),
            ["confirmshaming"]
        );
    }

    #[test]
    fn project_overrides() {
        let cfg = PolicyConfig::from_toml(
            r#"
source_locale = "en"
[policy.rules]
superlative = "error"
unsourced_number = "off"
[policy.forbidden]
de = ["kostenlos"]
"*" = ["AI-powered"]
[policy.patterns]
fake_urgency = ["offre flash"]
"#,
        )
        .unwrap();
        let r = |t: &str, l: &str| {
            check(t, l, "x", &[], &cfg)
                .into_iter()
                .map(|f| (f.rule, f.severity))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            r("The best plan", "en"),
            [("superlative".to_string(), Severity::Error)]
        );
        assert!(r("Save 50 %", "en").is_empty());
        assert_eq!(r("Kostenlose Testphase", "de-AT")[0].0, "forbidden_term"); // inflected, language scope
        assert!(r("Kostenlose Testphase", "fr").is_empty());
        assert_eq!(r("Offre flash sur Pro", "fr")[0].0, "fake_urgency");
        assert_eq!(r("AI-powered copy", "fr")[0].0, "forbidden_term");
        assert_eq!(
            PolicyConfig::from_toml("source_locale = \"en\"").unwrap(),
            PolicyConfig::default()
        );
    }
}
