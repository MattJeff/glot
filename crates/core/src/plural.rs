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
/// Accepts tags like "pt-BR" / "zh_Hant" (falls back to the language subtag).
pub fn categories(locale: &str) -> Vec<PluralCategory> {
    use PluralCategory::*;
    let lang = language(locale);
    // CLDR 44 cardinal rules, grouped by category set.
    match lang.as_str() {
        "ja" | "zh" | "yue" | "ko" | "th" | "vi" | "id" | "ms" | "lo" | "my" | "km" | "jv"
        | "su" | "bo" | "dz" | "ig" | "ii" | "kea" | "ses" | "sg" | "to" | "wo" | "yo" | "sah"
        | "jbo" | "kde" | "nqo" | "osa" => vec![Other],
        "ar" | "ars" | "cy" | "kw" => vec![Zero, One, Two, Few, Many, Other],
        "ga" | "br" | "mt" => vec![One, Two, Few, Many, Other],
        "ru" | "uk" | "be" | "pl" | "lt" | "cs" | "sk" => vec![One, Few, Many, Other],
        "hr" | "sr" | "bs" | "sh" | "ro" | "mo" => vec![One, Few, Other],
        "sl" | "dsb" | "hsb" | "gd" => vec![One, Two, Few, Other],
        "he" | "iw" | "iu" | "naq" | "se" | "sma" | "smi" | "smj" | "smn" | "sms" => {
            vec![One, Two, Other]
        }
        "lv" | "prg" | "ksh" | "lag" => vec![Zero, One, Other],
        "fr" | "es" | "it" | "pt" | "ca" | "vec" => vec![One, Many, Other],
        _ => vec![One, Other],
    }
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
    }
}
