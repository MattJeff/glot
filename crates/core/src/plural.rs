use icu_locale::{LocaleExpander, TransformResult};
use icu_locale_core::Locale;
use icu_plurals::PluralRules;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl PluralCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }
}

/// CLDR cardinal plural categories required for `locale` (e.g. "ar" => all six, "ja" => [Other]).
/// Accepts tags like "pt-BR" / "zh_Hant" / "fr-x-formal". Rules and data: ICU4X (`icu_plurals`, compiled CLDR).
/// A language CLDR does not know gets `[One, Other]` (the common case), not the root's `[Other]`.
pub fn categories(locale: &str) -> Vec<PluralCategory> {
    let loc = parse(locale);
    let cats = of(&rules(&loc, false));
    if cats == [PluralCategory::Other] && !known(&loc) {
        return vec![PluralCategory::One, PluralCategory::Other];
    }
    cats
}

/// CLDR ordinal categories (`selectordinal`, MF2 `:number select=ordinal`): en => one/two/few/other, fr => one/other.
pub fn ordinal_categories(locale: &str) -> Vec<PluralCategory> {
    of(&rules(&parse(locale), true))
}

/// Category of `number` (a decimal literal: "1", "1.5") for `locale`, cardinal or ordinal.
pub fn category(locale: &str, number: &str, ordinal: bool) -> Option<PluralCategory> {
    let n = fixed_decimal::Decimal::try_from_str(number).ok()?;
    Some(conv(rules(&parse(locale), ordinal).category_for(&n)))
}

fn rules(loc: &Locale, ordinal: bool) -> PluralRules {
    let r = if ordinal {
        PluralRules::try_new_ordinal(loc.into())
    } else {
        PluralRules::try_new_cardinal(loc.into())
    };
    // Compiled data always resolves (root as last resort).
    r.unwrap_or_else(|_| {
        PluralRules::try_new_cardinal((&Locale::UNKNOWN).into()).expect("root plural rules")
    })
}

fn of(r: &PluralRules) -> Vec<PluralCategory> {
    let mut v: Vec<PluralCategory> = r.categories().map(conv).collect();
    v.sort();
    v
}

fn conv(c: icu_plurals::PluralCategory) -> PluralCategory {
    use icu_plurals::PluralCategory as I;
    match c {
        I::Zero => PluralCategory::Zero,
        I::One => PluralCategory::One,
        I::Two => PluralCategory::Two,
        I::Few => PluralCategory::Few,
        I::Many => PluralCategory::Many,
        I::Other => PluralCategory::Other,
    }
}

/// "pt_BR" / "fr-x-formal" / garbage => a parsed locale (language subtag, else root).
fn parse(locale: &str) -> Locale {
    let tag = locale.replace('_', "-");
    tag.parse()
        .or_else(|_| language(&tag).parse())
        .unwrap_or(Locale::UNKNOWN)
}

/// Does CLDR know this language (likely-subtags data)? `xx` / `tlh` => false.
// ponytail: common likely subtags (+20 KB); a few rare "other"-only languages (jbo, osa, kde) read as unknown => [One, Other].
fn known(loc: &Locale) -> bool {
    let mut id = icu_locale_core::LanguageIdentifier::from(loc.id.language);
    LocaleExpander::new_common().maximize(&mut id) == TransformResult::Modified
}

/// Lowercased language subtag: "pt-BR" => "pt", "zh_Hant" => "zh".
pub fn language(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use PluralCategory::*;

    #[test]
    fn table() {
        assert_eq!(categories("ar").len(), 6);
        assert_eq!(categories("ja"), [Other]);
        assert_eq!(categories("zh_Hant"), [Other]);
        assert_eq!(categories("pt-BR"), [One, Many, Other]);
        assert_eq!(categories("pl"), [One, Few, Many, Other]);
        assert_eq!(categories("EN-us"), [One, Other]);
        assert_eq!(categories("xx"), [One, Other]);
        // Beyond the old hand-written table (CLDR via ICU4X).
        assert_eq!(categories("fr-x-formal"), [One, Many, Other]);
        assert_eq!(categories("lt"), [One, Few, Many, Other]);
        assert_eq!(categories("ja"), [Other]);
        assert_eq!(categories("yue"), [Other]);
    }

    #[test]
    fn ordinals() {
        assert_eq!(ordinal_categories("en-GB"), [One, Two, Few, Other]);
        assert_eq!(ordinal_categories("fr"), [One, Other]);
        assert_eq!(ordinal_categories("de"), [Other]);
        assert_eq!(ordinal_categories("cy"), [Zero, One, Two, Few, Many, Other]);
        assert_eq!(ordinal_categories("it"), [Many, Other]);
        assert_eq!(ordinal_categories("sv"), [One, Other]);
        assert_eq!(category("en", "22", true), Some(Two));
        assert_eq!(category("en", "12", true), Some(Other));
        assert_eq!(category("pl", "1.5", false), Some(Other));
        assert_eq!(category("ru", "22", false), Some(Few));
    }
}
