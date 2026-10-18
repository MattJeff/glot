use super::bidi::{FSI, PDI, RLM};
use super::*;
use std::collections::BTreeMap;

fn checks(
    locale: &str,
    source: Option<&str>,
    text: &str,
    syntax: Syntax,
) -> Vec<(Check, Severity)> {
    check_entry(locale, "k", source, text, syntax)
        .into_iter()
        .map(|f| (f.check, f.severity))
        .collect()
}

/// Fixed text has no fixable finding left.
fn fixed(locale: &str, source: Option<&str>, text: &str, syntax: Syntax) -> String {
    let f = fix_entry(locale, source, text, syntax).expect("a fix");
    let left: Vec<Finding> = check_entry(locale, "k", source, &f, syntax)
        .into_iter()
        .filter(|x| x.fixable)
        .collect();
    assert!(left.is_empty(), "{f:?} still has {left:?}");
    f
}

#[test]
fn segments_printf_and_icu() {
    let t = "%d دولة %1$@ %%";
    let s: Vec<Seg> = segments(t, Syntax::Printf).iter().map(|s| s.seg).collect();
    assert_eq!(s, [Seg::Num, Seg::Text, Seg::Str, Seg::Text]);
    let t = "مرحبا <b>{name}</b>، {n, plural, one {# ملف} other {# ملفات}}";
    let s: Vec<Seg> = segments(t, Syntax::Icu).iter().map(|s| s.seg).collect();
    assert_eq!(
        s,
        [
            Seg::Text,
            Seg::Markup,
            Seg::Str,
            Seg::Markup,
            Seg::Text,
            Seg::Break,
            Seg::Num,
            Seg::Text,
            Seg::Break,
            Seg::Num,
            Seg::Text,
            Seg::Break
        ]
    );
}

#[test]
fn bidi_spillover_is_found_and_isolated() {
    // "France 12" merges into one LTR run: the reader sees the number on the wrong side.
    let t = "%@ %d دولة";
    assert!(checks("ar", None, t, Syntax::Printf).contains(&(Check::Bidi, Severity::Error)));
    let f = fixed("ar", None, t, Syntax::Printf);
    assert_eq!(f, format!("{FSI}%@{PDI} %d دولة"));
    // The validator still accepts it (same placeholders).
    let issues = crate::validate::validate(
        "%@ %d countries",
        &f,
        "ar",
        Syntax::Printf,
        &Default::default(),
    );
    assert!(!crate::validate::has_errors(&issues), "{issues:?}");
}

#[test]
fn bidi_first_strong() {
    // Placeholder first: on iOS/Android the line becomes LTR.
    assert_eq!(
        checks("ar", None, "%@ دولة", Syntax::Printf),
        [(Check::Bidi, Severity::Error)]
    );
    assert_eq!(
        checks("ar", None, "{name} دولة", Syntax::Icu),
        [(Check::Bidi, Severity::Warning)]
    );
    assert_eq!(
        fixed("ar", None, "%@ دولة", Syntax::Printf),
        format!("{FSI}%@{PDI} دولة")
    );
    // Latin word first: RLM.
    assert_eq!(
        fixed("he", None, "Orizn מאפשר", Syntax::Icu),
        format!("{RLM}Orizn מאפשר")
    );
}

#[test]
fn bidi_clean_cases() {
    for t in [
        "مرحبا %@",
        "قبل %d أيام",
        "%d دولة",
        "\u{2068}%@\u{2069} %d دولة",
        "Hello %@ world", // untranslated: `glot check` says so
    ] {
        assert!(checks("ar", None, t, Syntax::Printf).is_empty(), "{t}");
    }
    assert!(
        checks(
            "ar",
            None,
            "<bdi>{name}</bdi> {count, number} دولة",
            Syntax::Icu
        )
        .is_empty()
    );
    // Not an RTL locale: no bidi check.
    assert!(checks("fr", None, "%@ %d pays", Syntax::Printf).is_empty());
}

#[test]
fn reversed_brackets() {
    let t = "التأشيرة )إلكترونية(";
    assert_eq!(
        checks("ar", None, t, Syntax::Icu),
        [(Check::Brackets, Severity::Error)]
    );
    assert_eq!(fixed("ar", None, t, Syntax::Icu), "التأشيرة (إلكترونية)");
    assert_eq!(
        checks("ar", None, "التأشيرة (إلكترونية", Syntax::Icu),
        [(Check::Brackets, Severity::Warning)]
    );
}

#[test]
fn french_spaces() {
    assert_eq!(
        fixed("fr", None, "Attention !", Syntax::Icu),
        "Attention\u{202F}!"
    );
    assert_eq!(
        fixed("fr", None, "Attention!", Syntax::Icu),
        "Attention\u{202F}!"
    );
    assert_eq!(
        fixed("fr", None, "Total: {n} €", Syntax::Icu),
        "Total\u{A0}: {n}\u{202F}€"
    );
    assert_eq!(
        fixed("fr", None, "10 000 visas", Syntax::Icu),
        "10\u{202F}000 visas"
    );
    for ok in [
        "https://glot.dev?x=1",
        "à 10:30",
        "Bonjour\u{202F}!",
        "Qui\u{202F}?!",
        "C'est ça",
    ] {
        assert!(checks("fr", None, ok, Syntax::Icu).is_empty(), "{ok}");
    }
    // fr-CA: no space before ! ? ;
    assert!(checks("fr-CA", None, "Attention!", Syntax::Icu).is_empty());
    // Other locales: only the number + unit rule.
    assert_eq!(
        fixed("de", None, "50 % Rabatt", Syntax::Icu),
        "50\u{A0}% Rabatt"
    );
}

#[test]
fn quotes_and_separators() {
    assert_eq!(
        fixed("fr", None, "Cliquez sur \"Envoyer\"", Syntax::Icu),
        "Cliquez sur «\u{202F}Envoyer\u{202F}»"
    );
    assert_eq!(
        fixed("de", None, "Klicke auf “Senden”", Syntax::Icu),
        "Klicke auf „Senden“"
    );
    assert_eq!(
        fixed("ja", None, "\"送信\"を押す", Syntax::Icu),
        "「送信」を押す"
    );
    let src = Some("Rated 4.5 stars, iOS 17.2.1");
    assert_eq!(
        fixed(
            "de",
            src,
            "Mit 4.5 Sternen bewertet, iOS 17.2.1",
            Syntax::Icu
        ),
        "Mit 4,5 Sternen bewertet, iOS 17.2.1"
    );
    // Not in the source: a version, a list… left alone.
    assert!(checks("de", Some("Rated"), "Version 2.1", Syntax::Icu).is_empty());
    assert!(checks("it", Some("Version 1.0"), "Versione 1.0", Syntax::Icu).is_empty());
}

#[test]
fn tofu_and_digits() {
    assert_eq!(
        checks("ja", None, "送信�", Syntax::Icu),
        [(Check::Tofu, Severity::Error)]
    );
    let e: BTreeMap<String, String> = [
        ("a", "قبل ٣ أيام"),
        ("b", "٢٠٢٦ دولة"),
        ("c", "منذ 5 دقائق"),
        ("d", "شبكة 4G"),
        ("e", "١ من 3"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let f = typo::check_digits("ar", &e, Syntax::Icu);
    let keys: Vec<&str> = f.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["c", "e"], "{f:?}");
}

#[test]
fn glyph_gaps_from_a_real_font() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/Geist-Regular.ttf"
    ))
    .unwrap();
    let faces = fonts::read_faces(&data);
    assert_eq!(faces.len(), 1);
    let (family, cps) = &faces[0];
    assert_eq!(family, "Geist");
    let fam = fonts::Family {
        family: family.clone(),
        files: vec![],
        origin: "test".into(),
        glyphs: cps.len(),
        cps: cps.clone(),
    };
    let cat = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let (en, ja, ar) = (
        cat(&[("a", "Send {name} 🎉")]),
        cat(&[("a", "{name}に送信"), ("b", "OK")]),
        cat(&[("a", "إرسال")]),
    );
    let (l_en, l_ja, l_ar) = ("en".to_string(), "ja".to_string(), "ar".to_string());
    let gaps = fonts::gaps(
        &[fam],
        [(&l_en, &en), (&l_ja, &ja), (&l_ar, &ar)],
        Syntax::Icu,
    );
    assert_eq!(gaps.len(), 2, "{gaps:?}");
    let ja = gaps.iter().find(|g| g.locale == "ja").unwrap();
    assert_eq!((ja.keys, ja.missing.as_str()), (1, "に信送"));
    assert_eq!(ja.unicode_range, "U+3040-30FF, U+4E00-9FFF");
    assert!(ja.fallback.contains("Hiragino"));
    assert!(
        gaps.iter()
            .any(|g| g.locale == "ar" && g.scripts == ["Arabic"])
    );
}

#[test]
fn woff2_cmap() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/render");
    let data = std::fs::read(format!("{dir}/latin-subset.woff2")).unwrap();
    let faces = fonts::read_faces(&data);
    assert_eq!(faces.len(), 1);
    let (family, cps) = &faces[0];
    assert!(!family.is_empty());
    assert!(cps.contains(&('a' as u32)) && cps.contains(&('é' as u32)));
    assert!(!cps.contains(&('あ' as u32)));
}
