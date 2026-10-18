//! `glot labs panel`: synthetic native users per market. A persona looks at a page (screenshot + text)
//! and answers a 5-second test, what each call to action does, trust, wording that reads translated and
//! culturally off details; a text-only judge then scores the answers against the source-locale intent.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::pricing;
use crate::provider::CompletionRequest;
use crate::translate::{estimate_tokens, parse_json, payload_block};
use crate::visual::image_tokens;

/// A synthetic visitor. `expectations` are the market's conventions (formats, register, legal pages,
/// payment habits), never personality traits attributed to a nationality.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Persona {
    pub name: String,
    pub age: u8,
    pub occupation: String,
    /// low | medium | high
    pub digital_literacy: String,
    pub expectations: String,
}

/// (language, name, age, occupation, literacy, conventions). Reviewed for stereotypes: varied ages and
/// jobs, ordinary names, and only observable localization conventions (formats, register, legal pages,
/// local payment methods), so a finding is always something a translator can act on.
const DEFAULTS: &[(&str, &str, u8, &str, &str, &str)] = &[
    (
        "en",
        "Jordan",
        34,
        "operations manager at a logistics company",
        "high",
        "Amounts like $1,234.56, dates as MM/DD/YYYY (US) or DD/MM/YYYY (UK), plain direct wording, clear pricing and cancellation terms.",
    ),
    (
        "fr",
        "Camille",
        41,
        "comptable dans un cabinet de 12 personnes",
        "medium",
        "Vouvoiement in a business tool, amounts as 1 234,56 € (space before €), dates DD/MM/YYYY, mentions of data hosting and RGPD, French typography (space before : ? !).",
    ),
    (
        "de",
        "Jonas",
        29,
        "Projektleiter in einem Ingenieurbüro",
        "high",
        "Sie in a business tool, amounts as 1.234,56 €, dates DD.MM.YYYY, an Impressum and Datenschutz page, precise feature descriptions and explicit contract terms.",
    ),
    (
        "es",
        "Lucía",
        37,
        "administrativa en una clínica dental",
        "medium",
        "Usted or a neutral register in B2B, amounts as 1.234,56 €, dates DD/MM/YYYY, IVA mentioned on prices, inverted punctuation (¿ ¡).",
    ),
    (
        "it",
        "Marco",
        52,
        "titolare di una piccola impresa edile",
        "medium",
        "Lei or a neutral register, amounts as 1.234,56 €, dates DD/MM/YYYY, IVA and electronic invoicing (fattura elettronica) addressed.",
    ),
    (
        "pt",
        "Ana",
        31,
        "gestora financeira numa startup",
        "high",
        "Brazil: R$ 1.234,56, dates DD/MM/YYYY, Pix or boleto as payment options; Portugal: 1234,56 €, European Portuguese spelling.",
    ),
    (
        "nl",
        "Sanne",
        45,
        "office manager bij een installatiebedrijf",
        "medium",
        "Je/jij is common in software, amounts as € 1.234,56, dates DD-MM-YYYY, iDEAL as a payment option, BTW on prices.",
    ),
    (
        "pl",
        "Katarzyna",
        39,
        "księgowa w biurze rachunkowym",
        "medium",
        "Pan/Pani or a neutral impersonal register, amounts as 1 234,56 zł, dates DD.MM.YYYY, correct plural forms of counted nouns, VAT and NIP fields.",
    ),
    (
        "ja",
        "Haruto",
        46,
        "中小企業の経理課長",
        "medium",
        "Polite desu/masu register, amounts as ¥1,234 (no decimals) incl. tax (税込), dates YYYY/MM/DD or YYYY年M月D日, full-width Japanese punctuation, a company profile and contact page.",
    ),
    (
        "zh",
        "Li Na",
        33,
        "电商公司的运营主管",
        "high",
        "Simplified characters, amounts as ¥1,234.56 (CNY), dates YYYY-MM-DD or YYYY年M月D日, Chinese punctuation, WeChat Pay / Alipay as payment options.",
    ),
    (
        "ko",
        "Min-jun",
        28,
        "스타트업 마케터",
        "high",
        "Polite 존댓말 (-요/-습니다), amounts as ₩1,234 (no decimals), dates YYYY. MM. DD., VAT (부가세) on prices, local payment options.",
    ),
    (
        "ar",
        "Omar",
        44,
        "مدير مالي في شركة تجارية",
        "medium",
        "Right-to-left layout with mirrored navigation, consistent digits (Arabic-Indic or Western), currency of the market (SAR, AED…), Modern Standard Arabic in formal register.",
    ),
    (
        "ru",
        "Elena",
        50,
        "главный бухгалтер производственной компании",
        "medium",
        "Вы in a business tool, amounts as 1 234,56 ₽, dates DD.MM.YYYY, correct plural forms (1 счёт, 2 счёта, 5 счетов).",
    ),
    (
        "tr",
        "Elif",
        35,
        "KOBİ'de satın alma uzmanı",
        "medium",
        "Siz register, amounts as ₺1.234,56, dates DD.MM.YYYY, KDV on prices, dotted/dotless i spelled correctly.",
    ),
    (
        "hi",
        "Priya",
        30,
        "आईटी कंपनी में एचआर एग्ज़ीक्यूटिव",
        "high",
        "Respectful आप register, amounts as ₹1,23,456 (lakh grouping), dates DD/MM/YYYY, GST on prices, UPI as a payment option; English loanwords for tech terms are normal.",
    ),
    (
        "sv",
        "Erik",
        42,
        "ekonomiansvarig på en byrå",
        "high",
        "Du is the norm, amounts as 1 234,56 kr, dates YYYY-MM-DD, moms on prices, Swish or invoice payment.",
    ),
];

fn lang(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or(locale)
}

/// Default persona of a locale; unknown languages get a neutral one built from the locale name.
pub fn default_persona(locale: &str) -> Persona {
    let l = lang(locale).to_ascii_lowercase();
    match DEFAULTS.iter().find(|d| d.0 == l) {
        Some(&(_, name, age, occupation, lit, exp)) => Persona {
            name: name.into(),
            age,
            occupation: occupation.into(),
            digital_literacy: lit.into(),
            expectations: exp.into(),
        },
        None => Persona {
            name: format!("Visitor ({locale})"),
            age: 38,
            occupation: "office worker at a small company".into(),
            digital_literacy: "medium".into(),
            expectations: format!(
                "The usual conventions of `{locale}`: native wording, local number, date and currency formats, the expected politeness register."
            ),
        },
    }
}

/// Personas of a locale: `glot.panel.yaml` `personas.<locale>` (one mapping or a list; missing fields
/// come from the default), else the default.
pub fn personas(locale: &str, config: &Value) -> Vec<Persona> {
    let def = default_persona(locale);
    let node = &config["personas"][locale];
    let list: Vec<Value> = match node {
        Value::Array(a) => a.clone(),
        Value::Object(_) => vec![node.clone()],
        _ => return vec![def],
    };
    list.into_iter()
        .map(|v| {
            let p: Persona = serde_json::from_value(v).unwrap_or_default();
            Persona {
                name: Some(p.name)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(def.name.clone()),
                age: if p.age > 0 { p.age } else { def.age },
                occupation: Some(p.occupation)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(def.occupation.clone()),
                digital_literacy: Some(p.digital_literacy)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(def.digital_literacy.clone()),
                expectations: Some(p.expectations)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(def.expectations.clone()),
            }
        })
        .collect()
}

/// One captured page (`packages/agent-qa` panel capture).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub route: String,
    pub locale: String,
    pub url: String,
    /// Relative to the capture dir.
    pub screenshot: String,
    pub title: String,
    pub headline: String,
    pub text: String,
    pub ctas: Vec<Cta>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cta {
    pub id: usize,
    pub text: String,
    pub href: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FiveSecond {
    /// In the persona's language.
    pub answer: String,
    /// The same answer in the source language (what the judge compares).
    pub answer_source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CtaAnswer {
    pub id: usize,
    pub expects: String,
    pub expects_source: String,
    pub sure: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Trust {
    /// 1-5.
    pub score: u8,
    pub reason: String,
}

/// A wording that reads translated, or a culturally off detail. `quote` is verbatim from the page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Remark {
    pub quote: String,
    /// Why, in the source language.
    pub why: String,
    /// Native rewording in the page language (null when it is not a wording problem).
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Verdict {
    pub five_second: FiveSecond,
    pub ctas: Vec<CtaAnswer>,
    pub trust: Trust,
    pub non_native: Vec<Remark>,
    pub cultural: Vec<Remark>,
}

/// Max output tokens of a persona answer (also the cost estimate).
pub const PERSONA_OUT: u32 = 1500;
/// Judge output: base + per answer.
const JUDGE_OUT_BASE: u32 = 200;
const JUDGE_OUT_PER_ANSWER: u32 = 80;

pub fn persona_request(
    model: &str,
    source_locale: &str,
    persona: &Persona,
    page: &Page,
    screenshot_b64: Option<String>,
) -> CompletionRequest {
    let payload = json!({
        "task": "panel",
        "locale": page.locale,
        "source_locale": source_locale,
        "persona": persona,
        "page": {
            "url": page.url,
            "title": page.title,
            "text": page.text,
            "calls_to_action": page.ctas.iter().map(|c| json!({"id": c.id, "label": c.text})).collect::<Vec<_>>(),
        },
    });
    let system = format!(
        "You take part in a usability study as a real native speaker of `{loc}`: {name}, {age}, {job}, digital literacy {lit}. \
Your market's conventions: {exp}\n\
You are shown a page of a web product (screenshot of what is visible first, then its text). React as this person would on first contact. \
Do not invent problems: an empty list is a valid answer.\n\n\
Answer:\n\
1. five_second: after looking for 5 seconds, what does this product offer? `answer` in `{loc}`, `answer_source` = the same answer translated to `{src}`.\n\
2. ctas: for each call to action (by id), what do you expect to happen if you click it? `expects` in `{loc}`, `expects_source` in `{src}`, `sure` false if the label leaves you guessing.\n\
3. trust: would you enter your company's data here? score 1 (not at all) to 5 (fully), `reason` in `{src}`.\n\
4. non_native: wording that a native speaker would not write (literal translation, wrong register, calque, leftover source language, wrong term). \
`quote` = the exact text as it appears on the page (copy it verbatim, shortest unit: a label or a sentence), `why` in `{src}`, `suggestion` = how a native `{loc}` product would say it (keep any variable such as a name or number as it appears).\n\
5. cultural: details that feel foreign for your market (number/date/currency formats, examples, references, payment or legal expectations). `quote` verbatim, `why` in `{src}`, `suggestion` in `{loc}` or null.\n\n\
Output only one JSON object, no prose: \
{{\"five_second\": {{\"answer\": \"\", \"answer_source\": \"\"}}, \"ctas\": [{{\"id\": 0, \"expects\": \"\", \"expects_source\": \"\", \"sure\": true}}], \
\"trust\": {{\"score\": 4, \"reason\": \"\"}}, \"non_native\": [{{\"quote\": \"\", \"why\": \"\", \"suggestion\": \"\"}}], \"cultural\": [{{\"quote\": \"\", \"why\": \"\", \"suggestion\": null}}]}}",
        loc = page.locale,
        src = source_locale,
        name = persona.name,
        age = persona.age,
        job = persona.occupation,
        lit = persona.digital_literacy,
        exp = persona.expectations,
    );
    CompletionRequest {
        model: model.into(),
        system,
        user: format!(
            "Page {} ({}). The screenshot is attached.\n\n{}",
            page.url,
            page.locale,
            payload_block(&payload)
        ),
        max_tokens: PERSONA_OUT,
        json: true,
        images: screenshot_b64.into_iter().collect(),
    }
}

pub fn parse_verdict(text: &str) -> Option<Verdict> {
    let mut v: Verdict = serde_json::from_value(parse_json(text)?).ok()?;
    v.trust.score = v.trust.score.clamp(1, 5);
    let keep = |r: &Remark| !r.quote.trim().is_empty();
    v.non_native.retain(keep);
    v.cultural.retain(keep);
    Some(v)
}

/// One persona's answers, as the judge sees them (`id` = "locale#persona").
pub struct JudgeItem<'a> {
    pub id: String,
    pub verdict: &'a Verdict,
}

/// Score every answer of a route against the intent (value proposition + real CTA destinations).
pub fn judge_request(
    model: &str,
    source_locale: &str,
    value_proposition: &str,
    source_ctas: &[Cta],
    items: &[JudgeItem],
) -> CompletionRequest {
    let payload = json!({
        "task": "panel_judge",
        "value_proposition": value_proposition,
        "calls_to_action": source_ctas.iter().map(|c| json!({"id": c.id, "label": c.text, "goes_to": c.href})).collect::<Vec<_>>(),
        "answers": items.iter().map(|i| json!({
            "id": i.id,
            "five_second": i.verdict.five_second.answer_source,
            "ctas": i.verdict.ctas.iter().map(|c| json!({"id": c.id, "expects": c.expects_source})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let system = format!(
        "You grade a usability study. Visitors in several languages looked at the same page; their answers were translated to `{source_locale}`.\n\
For each answer: `five_second` = how well their summary matches the intended value proposition, from 0 (unrelated or wrong) to 1 (same idea; wording may differ); \
`ctas` = for each call to action id, true if what they expect matches its label in the source language and where it goes, else false.\n\
Output only one JSON object, no prose: {{\"scores\": {{\"<answer id>\": {{\"five_second\": 0.8, \"ctas\": {{\"<cta id>\": true}}}}}}}}"
    );
    let out = JUDGE_OUT_BASE + JUDGE_OUT_PER_ANSWER * items.len() as u32;
    CompletionRequest {
        model: model.into(),
        system,
        user: format!(
            "Grade these {} answers.\n\n{}",
            items.len(),
            payload_block(&payload)
        ),
        max_tokens: out.max(1024),
        json: true,
        images: vec![],
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// 0-1.
    pub five_second: f64,
    /// Share of the CTAs understood (0-1); None when the page has none.
    pub ctas: Option<f64>,
    /// CTA ids whose outcome the persona got wrong.
    #[serde(default)]
    pub missed: Vec<usize>,
}

impl Score {
    /// Comprehension, 0-100.
    pub fn comprehension(&self) -> f64 {
        100.0
            * match self.ctas {
                Some(c) => (self.five_second + c) / 2.0,
                None => self.five_second,
            }
    }
}

/// `id` -> score; ids the judge skipped are absent.
pub fn parse_judge(text: &str) -> std::collections::BTreeMap<String, Score> {
    let v = parse_json(text).unwrap_or(Value::Null);
    let Some(scores) = v["scores"].as_object() else {
        return Default::default();
    };
    scores
        .iter()
        .map(|(id, s)| {
            let ctas: Vec<(&String, bool)> = s["ctas"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, b)| (k, b.as_bool() == Some(true)))
                        .collect()
                })
                .unwrap_or_default();
            let rate = (!ctas.is_empty())
                .then(|| ctas.iter().filter(|c| c.1).count() as f64 / ctas.len() as f64);
            let five = s["five_second"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
            (
                id.clone(),
                Score {
                    five_second: five,
                    ctas: rate,
                    missed: ctas
                        .iter()
                        .filter(|c| !c.1)
                        .filter_map(|c| c.0.parse().ok())
                        .collect(),
                },
            )
        })
        .collect()
}

/// Upper-bound USD of a request before sending it (vision tokens included, full `max_tokens` out).
pub fn estimate_usd(req: &CompletionRequest) -> f64 {
    let input = estimate_tokens(&req.system)
        + estimate_tokens(&req.user)
        + req.images.iter().map(|i| image_tokens(i)).sum::<u64>();
    pricing::cost_usd(&req.model, input as u32, req.max_tokens)
}

const NON_LATIN: &[&str] = &[
    "ja", "zh", "ko", "ar", "he", "fa", "ru", "uk", "bg", "el", "th", "hi",
];

/// Mock persona/judge (deterministic): mock translations carry a `⟦locale⟧` marker, which the persona
/// flags as non-native wording and which makes the labels it marks unclear; `$` amounts outside
/// English are flagged as a currency mismatch; English left on a non-Latin page reads untranslated.
pub(crate) fn mock(p: &Value) -> Value {
    if p["task"] == "panel_judge" {
        let scores: serde_json::Map<String, Value> = p["answers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                let five = a["five_second"].as_str().unwrap_or_default();
                let ctas: serde_json::Map<String, Value> = a["ctas"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        let ok = !c["expects"]
                            .as_str()
                            .unwrap_or_default()
                            .starts_with("Not sure");
                        (c["id"].to_string(), Value::Bool(ok))
                    })
                    .collect();
                let score = if five.is_empty() {
                    0.0
                } else if five.contains('⟦') {
                    0.5
                } else {
                    1.0
                };
                (
                    a["id"].as_str().unwrap_or_default().to_string(),
                    json!({"five_second": score, "ctas": ctas}),
                )
            })
            .collect();
        return json!({ "scores": scores });
    }
    let locale = p["locale"].as_str().unwrap_or_default();
    let source = p["source_locale"].as_str().unwrap_or("en");
    let page = &p["page"];
    let text = page["text"].as_str().unwrap_or_default();
    let headline = text
        .lines()
        .map(str::trim)
        .find(|l| l.chars().filter(|c| c.is_alphabetic()).count() > 12)
        .unwrap_or(page["title"].as_str().unwrap_or_default());
    let ctas: Vec<Value> = page["calls_to_action"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let label = c["label"].as_str().unwrap_or_default();
            let expects = if label.contains('⟦') {
                "Not sure — the label looks unfinished".to_string()
            } else {
                format!("Opens \"{label}\"")
            };
            json!({"id": c["id"], "expects": expects, "expects_source": expects, "sure": !label.contains('⟦')})
        })
        .collect();
    let non_latin = NON_LATIN.contains(&lang(locale));
    let mut non_native = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if non_native.len() >= 3 {
            break;
        }
        let fixed = line.find('⟦').and_then(|i| {
            let j = i + line[i..].find('⟧')? + '⟧'.len_utf8();
            Some(format!("{}{}", &line[..i], line[j..].trim_start()))
        });
        // A marker alone (an icon-only label) is nothing a reader would quote.
        if let Some(fixed) = fixed.filter(|f| f.chars().any(char::is_alphabetic)) {
            non_native.push(json!({"quote": line, "why": "A stray machine marker in front of the text: it reads like an unedited machine translation.", "suggestion": fixed.trim()}));
        } else if non_latin
            && lang(locale) != lang(source)
            && line.split_whitespace().count() >= 2
            && line.is_ascii()
            && line.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 8
        {
            non_native.push(
                json!({"quote": line, "why": "Left in the source language.", "suggestion": null}),
            );
        }
    }
    let mut cultural = vec![];
    if !lang(locale).eq_ignore_ascii_case("en")
        && let Some(i) = text
            .char_indices()
            .find(|&(i, c)| c == '$' && text[i + 1..].starts_with(|d: char| d.is_ascii_digit()))
            .map(|(i, _)| i)
    {
        let quote: String = text[i..].split_whitespace().next().unwrap_or("$").into();
        cultural.push(json!({"quote": quote, "why": "Amounts are shown in US dollars with US separators.", "suggestion": null}));
    }
    let flagged = !non_native.is_empty();
    json!({
        "five_second": {"answer": headline, "answer_source": headline},
        "ctas": ctas,
        "trust": {
            "score": if flagged { 2 } else { 4 },
            "reason": if flagged { "Several labels look machine-translated." } else { "Looks like a finished product in my language." },
        },
        "non_native": non_native,
        "cultural": cultural,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;

    fn page(locale: &str, text: &str) -> Page {
        Page {
            route: "/".into(),
            locale: locale.into(),
            url: format!("http://x/{locale}"),
            text: text.into(),
            ctas: vec![
                Cta {
                    id: 0,
                    text: text.into(),
                    href: Some("/pricing".into()),
                    key: None,
                },
                Cta {
                    id: 1,
                    text: "Pricing".into(),
                    href: Some("/pricing".into()),
                    key: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn personas_merge_overrides_with_defaults() {
        let cfg = json!({"personas": {"fr": {"name": "Inès", "age": 63}, "de": [{"occupation": "Lehrerin"}, {}]}});
        let fr = personas("fr-CA", &cfg);
        assert_eq!(fr.len(), 1);
        assert_eq!(fr[0], default_persona("fr")); // "fr-CA" has no override, defaults by language
        let fr = personas("fr", &cfg);
        assert_eq!((fr[0].name.as_str(), fr[0].age), ("Inès", 63));
        assert_eq!(fr[0].expectations, default_persona("fr").expectations);
        let de = personas("de", &cfg);
        assert_eq!(de.len(), 2);
        assert_eq!(de[0].occupation, "Lehrerin");
        assert_eq!(de[1], default_persona("de"));
        assert!(default_persona("xx").expectations.contains("`xx`"));
        // Every default names conventions, never a temperament.
        for d in DEFAULTS {
            for w in ["lazy", "rude", "punctual", "emotional", "cold", "loud"] {
                assert!(!d.5.to_lowercase().contains(w), "{} {w}", d.0);
            }
        }
    }

    #[tokio::test]
    async fn mock_round_trip_and_judge() {
        let p = page(
            "fr",
            "⟦fr⟧ Get paid faster\nInvoices for teams\nFrom $29 per month",
        );
        let req = persona_request("mock", "en", &default_persona("fr"), &p, None);
        assert!(req.system.contains("Camille"));
        let c = crate::mock::MockProvider.complete(req).await.unwrap();
        let v = parse_verdict(&c.text).unwrap();
        assert_eq!(v.non_native.len(), 1);
        assert_eq!(
            v.non_native[0].suggestion.as_deref(),
            Some("Get paid faster")
        );
        assert_eq!(v.cultural[0].quote, "$29");
        assert_eq!(v.trust.score, 2);
        assert!(!v.ctas[0].sure && v.ctas[1].sure);

        let en = page("en", "Get paid faster\nInvoices for teams");
        let c = crate::mock::MockProvider
            .complete(persona_request(
                "mock",
                "en",
                &default_persona("en"),
                &en,
                None,
            ))
            .await
            .unwrap();
        let ve = parse_verdict(&c.text).unwrap();
        assert!(ve.non_native.is_empty() && ve.cultural.is_empty());

        let items = [
            JudgeItem {
                id: "en#0".into(),
                verdict: &ve,
            },
            JudgeItem {
                id: "fr#0".into(),
                verdict: &v,
            },
        ];
        let j = judge_request("mock", "en", "Invoicing for teams", &en.ctas, &items);
        let c = crate::mock::MockProvider.complete(j).await.unwrap();
        let s = parse_judge(&c.text);
        assert_eq!(s["en#0"].comprehension(), 100.0);
        assert_eq!(
            s["fr#0"],
            Score {
                five_second: 0.5,
                ctas: Some(0.5),
                missed: vec![0]
            }
        );
        assert_eq!(s["fr#0"].comprehension(), 50.0);
    }

    #[test]
    fn ja_page_flags_leftover_english() {
        let v = mock(
            &json!({"task": "panel", "locale": "ja", "source_locale": "en",
            "page": {"text": "請求書を作成\nCreate an invoice now", "calls_to_action": []}}),
        );
        assert_eq!(v["non_native"][0]["quote"], "Create an invoice now");
    }

    #[test]
    fn estimate_counts_the_image() {
        let mut r = persona_request(
            "claude-sonnet-5-5",
            "en",
            &default_persona("fr"),
            &page("fr", "x"),
            None,
        );
        let text_only = estimate_usd(&r);
        r.images.push(String::new()); // undecodable -> 1600 tokens
        assert!(estimate_usd(&r) > text_only + 0.003);
        assert_eq!(
            estimate_usd(&persona_request(
                "mock",
                "en",
                &default_persona("fr"),
                &page("fr", "x"),
                None
            )),
            0.0
        );
    }
}
