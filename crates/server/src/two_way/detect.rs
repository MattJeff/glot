//! Language detection, local (whatlang trigram + script model: no network, no cost, nothing leaves the server).
//! Short Latin-script texts are the hard case ("ok merci"): the caller gets a confidence and decides
//! (thread language, then the integrator's hint) instead of trusting a guess.

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Detection {
    /// Best guess (BCP 47 language subtag), `None` when the text has no letters.
    pub locale: Option<String>,
    /// 0-1.
    pub confidence: f32,
    /// More than one language carries a real share of the text (the first is `locale`).
    pub mixed: Vec<String>,
}

/// A second language needs this share of the text to count as "mixed".
const MIXED_SHARE: f32 = 0.2;

/// ISO 639-3 (whatlang) → the 639-1 code the rest of glot uses; unknown ones keep three letters.
fn code(l: whatlang::Lang) -> String {
    const MAP: &[(&str, &str)] = &[
        ("eng", "en"),
        ("fra", "fr"),
        ("spa", "es"),
        ("deu", "de"),
        ("ita", "it"),
        ("por", "pt"),
        ("nld", "nl"),
        ("swe", "sv"),
        ("dan", "da"),
        ("nob", "nb"),
        ("fin", "fi"),
        ("pol", "pl"),
        ("ces", "cs"),
        ("slk", "sk"),
        ("hun", "hu"),
        ("ron", "ro"),
        ("bul", "bg"),
        ("hrv", "hr"),
        ("srp", "sr"),
        ("slv", "sl"),
        ("lit", "lt"),
        ("lav", "lv"),
        ("est", "et"),
        ("ell", "el"),
        ("tur", "tr"),
        ("rus", "ru"),
        ("ukr", "uk"),
        ("bel", "be"),
        ("heb", "he"),
        ("ara", "ar"),
        ("pes", "fa"),
        ("urd", "ur"),
        ("hin", "hi"),
        ("ben", "bn"),
        ("tam", "ta"),
        ("tel", "te"),
        ("tha", "th"),
        ("vie", "vi"),
        ("ind", "id"),
        ("jpn", "ja"),
        ("kor", "ko"),
        ("cmn", "zh"),
        ("cat", "ca"),
        ("afr", "af"),
        ("tgl", "tl"),
        ("kat", "ka"),
        ("hye", "hy"),
        ("aze", "az"),
        ("mkd", "mk"),
        ("mar", "mr"),
        ("guj", "gu"),
        ("pan", "pa"),
        ("kan", "kn"),
        ("mal", "ml"),
        ("nep", "ne"),
        ("sin", "si"),
        ("khm", "km"),
        ("mya", "my"),
        ("amh", "am"),
        ("epo", "eo"),
        ("lat", "la"),
        ("uzb", "uz"),
        ("zul", "zu"),
        ("sna", "sn"),
        ("jav", "jv"),
        ("ori", "or"),
        ("yid", "yi"),
        ("tuk", "tk"),
        ("aka", "ak"),
    ];
    let c = l.code();
    MAP.iter()
        .find(|(a, _)| *a == c)
        .map_or(c, |(_, b)| b)
        .to_string()
}

/// Measured on the real ClipOnIt/Orizn catalogs: whatlang's most frequent confident mistakes are Latin and
/// Esperanto (nobody writes to support in them: never answered) and Afrikaans for Dutch (17 of 70 long Dutch
/// strings). ponytail: Afrikaans is only answered when Dutch is not the runner-up; a real Afrikaans customer
/// who writes like Dutch is read as Dutch, which a Dutch-speaking team understands anyway.
static DETECTOR: std::sync::LazyLock<whatlang::Detector> = std::sync::LazyLock::new(|| {
    whatlang::Detector::with_denylist(vec![whatlang::Lang::Lat, whatlang::Lang::Epo])
});
static NOT_AFRIKAANS: std::sync::LazyLock<whatlang::Detector> = std::sync::LazyLock::new(|| {
    whatlang::Detector::with_denylist(vec![
        whatlang::Lang::Lat,
        whatlang::Lang::Epo,
        whatlang::Lang::Afr,
    ])
});

fn one(text: &str) -> Option<(String, f32)> {
    let mut i = DETECTOR.detect(text)?;
    if i.lang() == whatlang::Lang::Afr
        && let Some(nl) = NOT_AFRIKAANS
            .detect(text)
            .filter(|n| n.lang() == whatlang::Lang::Nld)
    {
        i = nl;
    }
    let mut conf = i.confidence() as f32;
    // A script that belongs to one language (kana, hangul, Thai…) is certain whatever the length; a few
    // Latin or Cyrillic words are not, whatever the model says.
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let shared_script = matches!(
        i.script(),
        whatlang::Script::Latin | whatlang::Script::Cyrillic | whatlang::Script::Arabic
    );
    // whatlang's confidence is the gap to the runner-up, low between close languages (fr/ca/it) even on a
    // clear sentence; past 20 letters, length is what makes the guess reliable. ponytail: heuristic, measured
    // on the real ClipOnIt/Orizn catalogs (tests/two_way.rs `real_data`); retune there.
    let own_script = matches!(
        i.script(),
        whatlang::Script::Hiragana
            | whatlang::Script::Katakana
            | whatlang::Script::Hangul
            | whatlang::Script::Thai
            | whatlang::Script::Greek
            | whatlang::Script::Georgian
            | whatlang::Script::Armenian
            | whatlang::Script::Khmer
            | whatlang::Script::Sinhala
    );
    if own_script {
        conf = 1.0;
    } else if shared_script {
        if letters < 20 {
            conf *= letters as f32 / 20.0;
        } else {
            conf += (1.0 - conf) * ((letters - 20) as f32 / 80.0).min(1.0);
        }
    }
    Some((code(i.lang()), conf.clamp(0.0, 1.0)))
}

pub fn detect(text: &str) -> Detection {
    let Some((locale, confidence)) = one(text) else {
        return Detection::default();
    };
    // Mixed languages: detect sentence by sentence, weigh by letters. Only sentences long enough to be
    // trusted vote, so "ok thanks" inside a Japanese message does not make it "mixed".
    let mut votes: Vec<(String, usize)> = vec![];
    for part in text.split(['\n', '.', '!', '?', '。', '！', '？', '؟']) {
        let letters = part.chars().filter(|c| c.is_alphabetic()).count();
        if let Some((l, c)) = one(part)
            && c >= 0.5
        {
            match votes.iter_mut().find(|(v, _)| *v == l) {
                Some(v) => v.1 += letters,
                None => votes.push((l, letters)),
            }
        }
    }
    votes.sort_by_key(|v| std::cmp::Reverse(v.1));
    let total: usize = votes.iter().map(|v| v.1).sum();
    let mixed: Vec<String> = votes
        .iter()
        .filter(|v| v.1 as f32 >= total as f32 * MIXED_SHARE)
        .map(|v| v.0.clone())
        .collect();
    if mixed.len() > 1 {
        // The whole-text guess is unreliable on a mixed text: the biggest voter wins, with its share.
        return Detection {
            locale: Some(mixed[0].clone()),
            confidence: votes[0].1 as f32 / total as f32,
            mixed,
        };
    }
    Detection {
        locale: Some(locale),
        confidence,
        mixed: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confident_on_real_messages() {
        for (text, want) in [
            (
                "注文した商品がまだ届いていません。至急ご確認ください。",
                "ja",
            ),
            (
                "Bonjour, je n'ai toujours pas reçu ma commande passée la semaine dernière.",
                "fr",
            ),
            (
                "Hallo, ich habe meine Bestellung noch nicht erhalten und möchte mein Geld zurück.",
                "de",
            ),
            (
                "Hola, todavía no he recibido mi pedido y necesito una respuesta urgente.",
                "es",
            ),
            ("안녕하세요, 주문한 상품이 아직 도착하지 않았습니다.", "ko"),
            ("ありがとう", "ja"),
            (
                "Zonder sleutel krijg je een gemeten demo-tegoed per IP, genoeg om rustig te verkennen.",
                "nl",
            ),
        ] {
            let d = detect(text);
            assert_eq!(d.locale.as_deref(), Some(want), "{text}");
            assert!(d.confidence >= 0.5, "{text}: {}", d.confidence);
            assert!(d.mixed.is_empty(), "{text}: {:?}", d.mixed);
        }
        // Arabic script is shared (ar, fa, ur): a one-line message is guessed right but not trusted on its
        // own, so the thread's language or the caller's hint decides (see `inbound`).
        let d = detect("مرحبا، لم أستلم طلبي حتى الآن وأريد استرداد أموالي");
        assert_eq!(d.locale.as_deref(), Some("ar"));
        assert!(d.confidence < 0.5, "{}", d.confidence);
    }

    #[test]
    fn short_latin_text_is_below_the_threshold() {
        for text in ["ok", "merci", "ok thanks", "hola!"] {
            let d = detect(text);
            assert!(d.confidence < 0.5, "{text}: {:?}", d);
        }
        assert_eq!(detect("👍 123"), Detection::default());
    }

    #[test]
    fn mixed_languages_are_reported() {
        let d = detect(
            "Bonjour, je n'ai toujours pas reçu ma commande passée la semaine dernière.\n\
             注文した商品がまだ届いていません。至急ご確認ください、よろしくお願いします。",
        );
        assert_eq!(d.mixed.len(), 2, "{d:?}");
        assert!(d.mixed.contains(&"fr".to_string()) && d.mixed.contains(&"ja".to_string()));
    }
}
