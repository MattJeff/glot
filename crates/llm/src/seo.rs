//! Search-localized metadata (`glot seo keywords`): for page titles and descriptions, the LLM
//! finds the query natives actually type ("logiciel de facturation", not "app de factures") and
//! writes 2-3 variants around it within the SERP length budget, instead of translating literally.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use glot_core::{Key, Locale};

use crate::{
    Result,
    engine::Workspace,
    provider::{self, CompletionRequest, Provider},
    translate::{Usage, parse_json, payload_block},
};

/// Google truncates titles around 600px (~60 Latin chars) and descriptions around 160 chars.
pub const TITLE_MAX: usize = 60;
pub const DESCRIPTION_MAX: usize = 160;

/// SERP width in "Latin chars": CJK / Hangul / fullwidth glyphs count double.
pub fn serp_width(s: &str) -> usize {
    s.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6 => 2,
            _ => 1,
        })
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Title,
    Description,
}

impl Kind {
    pub fn max(self) -> usize {
        match self {
            Kind::Title => TITLE_MAX,
            Kind::Description => DESCRIPTION_MAX,
        }
    }
}

/// Keys rendered in `<title>` / `<meta name="description">`: i18nize notes them
/// "heading in generateMetadata" (title) / "text in generateMetadata" (description).
pub fn metadata_keys(ws: &Workspace) -> Vec<(Key, Kind)> {
    ws.lock
        .notes
        .iter()
        .filter(|(k, n)| n.contains("generateMetadata") && ws.source().entries.contains_key(*k))
        .map(|(k, n)| {
            let title = n.starts_with("heading") || k.ends_with("title");
            (
                k.clone(),
                if title {
                    Kind::Title
                } else {
                    Kind::Description
                },
            )
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub key: Key,
    pub locale: Locale,
    pub kind: Kind,
    pub source: String,
    pub current: Option<String>,
    /// What natives type in a search engine for this page.
    pub search_terms: Vec<String>,
    /// Best first; every variant fits `kind.max()` (others are dropped).
    pub variants: Vec<String>,
}

const SYSTEM: &str = "You are a native SEO copywriter. Page titles and meta descriptions must be LOCALIZED FOR SEARCH, not translated literally: \
a literal translation of a keyword is often not what natives type (e.g. French users search \"logiciel de facturation\", not \"factures app\"). \
For each item: (1) give 1-3 `search_terms`: the queries natives of the target locale really type for this page; \
(2) write 2-3 `variants` in the target locale, best first, that read naturally and contain the main search term, \
keep the source's intent, brand names and placeholders/ICU syntax exactly, use the glossary translations and never translate do_not_translate terms. \
Each variant must fit `max_width` (CJK characters count as 2).\n\
Output only one JSON object: {\"items\": {\"<key>\": {\"search_terms\": [\"...\"], \"variants\": [\"...\"]}}}";

#[derive(Debug, Clone)]
pub struct Item {
    pub key: Key,
    pub kind: Kind,
    pub source: String,
    pub current: Option<String>,
    pub note: Option<String>,
}

pub async fn localize(
    provider: &dyn Provider,
    model: &str,
    ws: &Workspace,
    locale: &str,
    items: &[Item],
) -> Result<(Vec<Suggestion>, Usage)> {
    let mut terms = vec![];
    for i in items {
        for t in ws.glossary.matches(&i.source) {
            terms.push(
                json!({"term": t.term, "do_not_translate": t.do_not_translate,
                "translation": t.translations.get(locale)}),
            );
        }
    }
    let payload = json!({
        "task": "seo_keywords",
        "source_locale": ws.config.source_locale,
        "target_locale": locale,
        "style": ws.config.llm.style,
        "glossary": terms,
        "items": items.iter().map(|i| json!({
            "key": i.key, "kind": i.kind, "source": i.source, "current_translation": i.current,
            "context": i.note, "max_width": i.kind.max(),
        })).collect::<Vec<_>>(),
    });
    let c = provider
        .complete(CompletionRequest {
            model: model.into(),
            system: SYSTEM.into(),
            user: format!(
                "Localize these {} page metadata strings for search in `{locale}`.\n\n{}",
                items.len(),
                payload_block(&payload)
            ),
            max_tokens: 8_000,
            json: true,
            images: vec![],
        })
        .await?;
    let parsed = parse_json(&c.text).unwrap_or(Value::Null);
    let strs = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| Some(s.as_str()?.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect()
    };
    let out = items
        .iter()
        .map(|i| {
            let v = &parsed["items"][&i.key];
            Suggestion {
                key: i.key.clone(),
                locale: locale.into(),
                kind: i.kind,
                source: i.source.clone(),
                current: i.current.clone(),
                search_terms: strs(&v["search_terms"]),
                variants: strs(&v["variants"]).into_iter().fold(vec![], |mut acc, s| {
                    if serp_width(&s) <= i.kind.max() && !acc.contains(&s) {
                        acc.push(s);
                    }
                    acc
                }),
            }
        })
        .collect();
    Ok((out, Usage::of_completion(&c)))
}

/// Metadata keys of `locales` (default: all targets), one LLM call per locale.
pub async fn localize_workspace(
    ws: &Workspace,
    locales: Option<&[Locale]>,
    keys: Option<&[Key]>,
) -> Result<(Vec<Suggestion>, Usage)> {
    let meta: Vec<(Key, Kind)> = metadata_keys(ws)
        .into_iter()
        .filter(|(k, _)| keys.is_none_or(|w| w.contains(k)))
        .collect();
    let mut out = vec![];
    let mut usage = Usage::default();
    if meta.is_empty() {
        return Ok((out, usage));
    }
    let provider = provider::from_config(&ws.config.llm, None)?;
    for loc in &ws.config.target_locales {
        if locales.is_some_and(|w| !w.contains(loc)) {
            continue;
        }
        let items: Vec<Item> = meta
            .iter()
            .map(|(k, kind)| Item {
                key: k.clone(),
                kind: *kind,
                source: ws.source().entries[k].clone(),
                current: ws.catalogs.get(loc).and_then(|c| c.entries.get(k).cloned()),
                note: ws.lock.notes.get(k).cloned(),
            })
            .collect();
        let (s, u) = localize(provider.as_ref(), &ws.config.llm.model, ws, loc, &items).await?;
        out.extend(s);
        usage.add(&u);
    }
    Ok((out, usage))
}

/// `SEO fr: a, b` lines of a key's note, replaced per locale (idempotent).
pub fn note_with_terms(note: Option<&str>, locale: &str, terms: &[String]) -> String {
    let tag = format!("SEO {locale}:");
    let mut lines: Vec<String> = note
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with(&tag))
        .map(String::from)
        .collect();
    lines.push(format!("{tag} {}", terms.join(", ")));
    lines.join("\n").trim().to_string()
}

/// Mock answer: the search term is the source's first two words; variants are the mock
/// translation and a shorter one, so tests see the length filter at work.
pub(crate) fn mock(p: &Value) -> Value {
    let locale = p["target_locale"].as_str().unwrap_or("xx");
    let items: serde_json::Map<String, Value> = p["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let src = i["source"].as_str()?;
            let words: Vec<&str> = src.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).collect();
            let term = words.iter().take(2).copied().collect::<Vec<_>>().join(" ").to_lowercase();
            let short = words.iter().take(4).copied().collect::<Vec<_>>().join(" ");
            Some((i["key"].as_str()?.to_string(), json!({
                "search_terms": [format!("⟦{locale}⟧ {term}")],
                "variants": [crate::mock::fake_translate(src, locale, &[]), crate::mock::fake_translate(&short, locale, &[])],
            })))
        })
        .collect();
    json!({ "items": items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_and_notes() {
        assert_eq!(serp_width("Invoices"), 8);
        assert_eq!(serp_width("請求書ソフト"), 12);
        assert_eq!(
            note_with_terms(
                Some("heading in generateMetadata\nSEO fr: old"),
                "fr",
                &["a".into(), "b".into()]
            ),
            "heading in generateMetadata\nSEO fr: a, b"
        );
    }

    #[tokio::test]
    async fn mock_localizes_metadata_keys() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(r.join("glot.toml"), "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n").unwrap();
        let long = "Create, send and track invoices with your whole team. ".repeat(3);
        std::fs::write(r.join("en.json"), serde_json::to_string(&json!({"meta": {"title": "Acme Invoices — Get paid faster", "desc": long}, "x": "y"})).unwrap()).unwrap();
        std::fs::write(r.join("glot.lock"), r#"{"version":1,"notes":{"meta.title":"heading in generateMetadata","meta.desc":"text in generateMetadata","x":"text in Page"},"entries":{}}"#).unwrap();
        let ws = Workspace::open(r).unwrap();
        let (s, _) = localize_workspace(&ws, None, None).await.unwrap();
        assert_eq!(s.len(), 2);
        let title = s.iter().find(|s| s.key == "meta.title").unwrap();
        assert_eq!(title.kind, Kind::Title);
        assert_eq!(title.search_terms, ["⟦fr⟧ acme invoices"]);
        assert_eq!(title.variants[0], "⟦fr⟧ Acme Invoices — Get paid faster");
        // The full 165-char description is over budget: only the short variant survives.
        let desc = s.iter().find(|s| s.key == "meta.desc").unwrap();
        assert_eq!(desc.variants, ["⟦fr⟧ Create, send and track"]);
    }
}
