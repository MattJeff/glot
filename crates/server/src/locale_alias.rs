//! Locale code aliases: a site that asks for `zh`, `zh_CN` or `zh-Hans` gets the project's `zh-CN`.
//! [`resolve`] is a BCP 47 lookup against the project's own locales (exact match always wins), used by every
//! public endpoint that takes a locale. [`variants`] lists the spellings a site may accept instead of glot's
//! code (tried by the live check, `crate::live_check`).
// ponytail: likely subtags for the languages people actually ship, not the CLDR table; add rows when one is missed.

/// `(language, script, region)`, canonical case (`zh`, `Hans`, `CN`); empty when absent. None = not a locale code.
fn parse(code: &str) -> Option<(String, String, String)> {
    let mut parts = code.split(['-', '_']);
    let lang = parts.next()?.to_ascii_lowercase();
    if !(2..=3).contains(&lang.len()) || !lang.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    // Deprecated codes browsers and old CMSs still send.
    let lang = match lang.as_str() {
        "iw" => "he".into(),
        "in" => "id".into(),
        "ji" => "yi".into(),
        _ => lang,
    };
    let (mut script, mut region) = (String::new(), String::new());
    for p in parts {
        if !p.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        if p.len() == 4 && script.is_empty() && region.is_empty() {
            script = p[..1].to_ascii_uppercase() + &p[1..].to_ascii_lowercase();
        } else if (p.len() == 2 || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit())))
            && region.is_empty()
        {
            region = p.to_ascii_uppercase();
        } // variants / extensions: ignored
    }
    Some((lang, script, region))
}

/// Fills the script and region a bare code implies (`zh` → Hans/CN, `zh-TW` → Hant, `pt` → BR).
fn maximize((lang, mut script, mut region): (String, String, String)) -> (String, String, String) {
    if script.is_empty() {
        script = match (lang.as_str(), region.as_str()) {
            ("zh", "TW" | "HK" | "MO") => "Hant",
            ("zh", _) => "Hans",
            ("sr", _) => "Cyrl",
            _ => "",
        }
        .into();
    }
    if region.is_empty() {
        region = match (lang.as_str(), script.as_str()) {
            ("zh", "Hant") => "TW",
            ("zh", _) => "CN",
            ("pt", _) => "BR",
            ("en", _) => "US",
            ("es", _) => "ES",
            ("fr", _) => "FR",
            ("de", _) => "DE",
            ("it", _) => "IT",
            ("nl", _) => "NL",
            ("ja", _) => "JP",
            ("ko", _) => "KR",
            ("ru", _) => "RU",
            ("ar", _) => "EG",
            ("sv", _) => "SE",
            ("sr", _) => "RS",
            _ => "",
        }
        .into();
    }
    (lang, script, region)
}

/// The project locale to serve for `requested`, among `available` (source + targets). Order:
/// exact → same code in another spelling (`zh-cn`, `zh_CN`) → same locale once likely subtags are filled
/// (`zh`, `zh-Hans`, `zh-Hans-CN` → `zh-CN`) → for a bare language, the first listed regional variant
/// (`pt` → `pt-PT` when it is the only one) → for a regional code, the bare language (`pt-BR` → `pt`), then a
/// script-only one (`zh-HK` → `zh-Hant`), then a sibling (`en-GB` → `en-US`). Never across scripts
/// (`zh-TW` is not served Simplified Chinese).
pub fn resolve<'a>(requested: &str, available: &'a [String]) -> Option<&'a str> {
    if let Some(a) = available.iter().find(|a| *a == requested) {
        return Some(a);
    }
    let req = parse(requested)?;
    let bare = req.1.is_empty() && req.2.is_empty();
    let same: Vec<(&'a str, (String, String, String))> = available
        .iter()
        .filter_map(|a| Some((a.as_str(), parse(a)?)))
        .filter(|(_, p)| p.0 == req.0)
        .collect();
    if let Some((a, _)) = same.iter().find(|(_, p)| *p == req) {
        return Some(a);
    }
    let max = maximize(req);
    let same: Vec<(&'a str, (String, String, String), (String, String, String))> = same
        .into_iter()
        .map(|(a, p)| (a, p.clone(), maximize(p)))
        .collect();
    if let Some((a, ..)) = same.iter().find(|(_, _, m)| *m == max) {
        return Some(a);
    }
    if bare {
        return same.first().map(|(a, ..)| *a);
    }
    // Scripts only separate the languages `maximize` knows a script for; elsewhere "" matches anything.
    let script_ok =
        |m: &(String, String, String)| m.1 == max.1 || m.1.is_empty() || max.1.is_empty();
    let same: Vec<_> = same.into_iter().filter(|(_, _, m)| script_ok(m)).collect();
    same.iter()
        .find(|(_, p, _)| p.1.is_empty() && p.2.is_empty())
        .or_else(|| same.iter().find(|(_, p, _)| p.2.is_empty()))
        .or(same.first())
        .map(|(a, ..)| *a)
}

/// [`resolve`] over a project's source and target locales.
pub fn resolve_project(requested: &str, source: &str, targets: &[String]) -> Option<String> {
    if requested == source || targets.iter().any(|t| t == requested) {
        return Some(requested.to_string());
    }
    let all: Vec<String> = std::iter::once(source.to_string())
        .chain(targets.iter().cloned())
        .collect();
    resolve(requested, &all).map(str::to_string)
}

/// Other spellings a site may accept for `code`, most likely first: `zh-CN` → `zh`, `zh-cn`, `zh_CN`, `zh-Hans`.
pub fn variants(code: &str) -> Vec<String> {
    let Some(p) = parse(code) else {
        return vec![];
    };
    let (lang, script, region) = maximize(p.clone());
    let mut out = vec![
        lang.clone(),
        code.to_ascii_lowercase(),
        code.replace('-', "_"),
    ];
    if !script.is_empty() {
        out.push(format!("{lang}-{script}"));
    }
    if !region.is_empty() && p.2.is_empty() {
        out.push(format!("{lang}-{region}")); // `pt` → `pt-BR`: sites that only know regional codes
    }
    let mut seen = vec![code.to_string()];
    out.retain(|v| {
        let new = !seen.contains(v);
        seen.push(v.clone());
        new
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_table() {
        // (requested, project locales, expected)
        let cases: &[(&str, &[&str], Option<&str>)] = &[
            // The cliponit case: the site only passes 2-3 letter codes.
            ("zh", &["fr", "en", "zh-CN"], Some("zh-CN")),
            ("zh-cn", &["fr", "zh-CN"], Some("zh-CN")),
            ("zh_CN", &["fr", "zh-CN"], Some("zh-CN")),
            ("ZH-CN", &["fr", "zh-CN"], Some("zh-CN")),
            ("zh-Hans", &["fr", "zh-CN"], Some("zh-CN")),
            ("zh-Hans-CN", &["fr", "zh-CN"], Some("zh-CN")),
            ("zh-hans-cn", &["fr", "zh-CN"], Some("zh-CN")),
            // Several zh-*: likely subtags (zh → Hans/CN), whatever the order.
            ("zh", &["zh-TW", "zh-CN"], Some("zh-CN")),
            ("zh-Hant", &["zh-CN", "zh-TW"], Some("zh-TW")),
            ("zh-HK", &["zh-CN", "zh-TW"], Some("zh-TW")),
            ("zh-HK", &["zh-CN", "zh-Hant"], Some("zh-Hant")),
            ("zh-SG", &["zh-TW", "zh-CN"], Some("zh-CN")),
            // The only zh-* target is served to `zh`, even Traditional.
            ("zh", &["en", "zh-TW"], Some("zh-TW")),
            // Never across scripts.
            ("zh-TW", &["en", "zh-CN"], None),
            ("zh-TW", &["en", "zh"], None),
            ("zh-CN", &["en", "zh"], Some("zh")),
            // Exact match always wins.
            ("zh", &["zh-CN", "zh"], Some("zh")),
            ("pt", &["pt-BR", "pt"], Some("pt")),
            ("pt-BR", &["pt", "pt-BR"], Some("pt-BR")),
            // Parent fallback, and pt → pt-BR only when pt is absent.
            ("pt-BR", &["en", "pt"], Some("pt")),
            ("pt_br", &["en", "pt"], Some("pt")),
            ("pt", &["pt-PT", "pt-BR"], Some("pt-BR")),
            ("pt", &["en", "pt-PT"], Some("pt-PT")),
            ("pt-PT", &["en", "pt-BR"], Some("pt-BR")),
            ("en-GB", &["en-US", "fr"], Some("en-US")),
            ("en", &["en-GB", "en-AU"], Some("en-GB")),
            ("fr-CA", &["fr", "fr-CA"], Some("fr-CA")),
            ("fr-CA", &["en", "fr"], Some("fr")),
            ("es-419", &["en", "es"], Some("es")),
            ("sr-Latn", &["sr"], None),
            ("iw", &["he"], Some("he")),
            ("he", &["iw"], Some("iw")),
            // Not a language of the project, or not a locale at all.
            ("de", &["fr", "zh-CN"], None),
            ("z", &["zh-CN"], None),
            ("zh/../x", &["zh-CN"], None),
            ("", &["zh-CN"], None),
            ("experiments", &["en"], None),
        ];
        for (req, avail, want) in cases {
            let avail: Vec<String> = avail.iter().map(|s| s.to_string()).collect();
            assert_eq!(resolve(req, &avail), *want, "{req} among {avail:?}");
        }
    }

    #[test]
    fn project_and_variants() {
        let targets = vec!["en".to_string(), "zh-CN".to_string()];
        assert_eq!(
            resolve_project("zh", "fr", &targets).as_deref(),
            Some("zh-CN")
        );
        assert_eq!(resolve_project("FR", "fr", &targets).as_deref(), Some("fr"));
        assert_eq!(resolve_project("de", "fr", &targets), None);
        assert_eq!(variants("zh-CN"), ["zh", "zh-cn", "zh_CN", "zh-Hans"]);
        assert_eq!(variants("pt-BR"), ["pt", "pt-br", "pt_BR"]);
        assert_eq!(variants("pt"), ["pt-BR"]);
        assert_eq!(
            variants("zh-Hant-TW"),
            ["zh", "zh-hant-tw", "zh_Hant_TW", "zh-Hant"]
        );
    }
}
