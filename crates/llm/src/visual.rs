//! "The LLM sees the screen": screenshot crops attached to translation calls, and the
//! multimodal "make it fit" request used by `glot fit`.
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

use glot_core::{Key, glossary::Term};

use crate::Result;
use crate::provider::{Completion, CompletionRequest, Provider};
use crate::translate::{extract_payload, parse_json, payload_block};

/// Anthropic accepts more, but past a handful the crops stop helping and cost tokens.
pub const MAX_IMAGES: usize = 8;

pub fn png_b64(path: &Path) -> std::io::Result<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(std::fs::read(path)?))
}

/// Attaches the crop of every key of a batch (base64 PNG, source-locale rendering) to the call.
pub struct VisualProvider {
    inner: Box<dyn Provider>,
    crops: BTreeMap<Key, String>,
}

pub fn wrap(inner: Box<dyn Provider>, crops: &BTreeMap<Key, String>) -> Box<dyn Provider> {
    if crops.is_empty() {
        return inner;
    }
    Box::new(VisualProvider {
        inner,
        crops: crops.clone(),
    })
}

#[async_trait]
impl Provider for VisualProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn complete(&self, mut req: CompletionRequest) -> Result<Completion> {
        attach(&self.crops, &mut req);
        self.inner.complete(req).await
    }
}

/// Attach the crops of the batch's keys (max `MAX_IMAGES`) to a translation request.
pub fn attach(crops: &BTreeMap<Key, String>, req: &mut CompletionRequest) {
    let items = extract_payload(&req.user)
        .and_then(|p| p["items"].as_array().cloned())
        .unwrap_or_default();
    let mut shown = vec![];
    for key in items.iter().filter_map(|i| i["key"].as_str()) {
        if req.images.len() >= MAX_IMAGES {
            break;
        }
        if let Some(img) = crops.get(key) {
            req.images.push(img.clone());
            shown.push(format!("image {} = `{key}`", req.images.len()));
        }
    }
    if !shown.is_empty() {
        req.user += &format!(
            "\n\nScreenshots of the running app (source locale) show where these strings appear: {}. \
Use them to pick the right meaning (verb vs noun, button vs label) and a length that fits the same box.",
            shown.join(", ")
        );
    }
}

/// Vision input tokens of a base64 PNG (Anthropic: w×h/750, ~1600 max after downscaling).
pub fn image_tokens(b64: &str) -> u64 {
    let head = base64::engine::general_purpose::STANDARD
        .decode(b64.get(..32).unwrap_or(b64))
        .unwrap_or_default();
    let dim = |at: usize| {
        head.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
    };
    match (dim(16), dim(20)) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w * h).div_ceil(750).min(1600),
        _ => 1600,
    }
}

/// One overflowing translation to shorten.
#[derive(Debug, Clone)]
pub struct Shorten {
    pub locale: String,
    pub key: Key,
    pub source: String,
    pub current: String,
    pub width_px: u32,
    pub need_width_px: u32,
    pub height_px: u32,
    pub need_height_px: u32,
    pub font: String,
    pub glossary: Vec<Term>,
    /// Earlier proposals that still overflowed or were rejected, with why.
    pub rejected: Vec<(String, String)>,
    /// Base64 PNGs: the overflowing element, then its surrounding component.
    pub images: Vec<String>,
}

impl Shorten {
    /// Visible-character budget, proportional to the space actually available.
    pub fn max_chars(&self) -> usize {
        let n = self.current.chars().count() as f64;
        let ratio = |have: u32, need: u32| {
            if need > have && need > 0 {
                have as f64 / need as f64
            } else {
                1.0
            }
        };
        let r = ratio(self.width_px, self.need_width_px)
            .min(ratio(self.height_px, self.need_height_px));
        // ponytail: chars ∝ px is rough (proportional fonts); the re-render is the real check.
        ((n * r * 0.9).floor() as usize).clamp(1, self.current.chars().count().max(1))
    }

    pub fn request(&self, model: &str) -> CompletionRequest {
        let payload = json!({
            "task": "shorten",
            "target_locale": self.locale,
            "key": self.key,
            "source": self.source,
            "current": self.current,
            "box_px": {"width": self.width_px, "height": self.height_px},
            "text_needs_px": {"width": self.need_width_px, "height": self.need_height_px},
            "font": self.font,
            "max_chars": self.max_chars(),
            "glossary": self.glossary,
            "rejected_attempts": self.rejected.iter().map(|(v, why)| json!({"value": v, "why": why})).collect::<Vec<_>>(),
        });
        let system = format!(
            "You are a senior UI localization specialist for `{loc}`. A translated UI string does not fit its fixed-size box in the running app. \
Write a shorter `{loc}` variant with the same meaning that fits.\n\n\
Images: first the overflowing element as rendered, then its surrounding component.\n\n\
Rules — a variant breaking any is rejected automatically:\n\
1. Keep every placeholder/variable exactly (same names and braces); keep ICU plural/select structure and categories.\n\
2. Keep markup tags with the same names and count.\n\
3. Glossary terms are mandatory; do-not-translate terms stay verbatim.\n\
4. Stay natural and idiomatic: abbreviate, drop filler words, use a shorter synonym or verb form; never truncate mid-word or add an ellipsis.\n\
5. Aim for at most `max_chars` visible characters.\n\n\
Output: only one JSON object, no prose: {{\"value\": \"<shorter string>\"}}",
            loc = self.locale
        );
        CompletionRequest {
            model: model.into(),
            system,
            user: format!(
                "Shorten `{}` so it fits {}px (it needs {}px).\n\n{}",
                self.key,
                self.width_px,
                self.need_width_px,
                payload_block(&payload)
            ),
            max_tokens: 1024,
            json: true,
            images: self.images.clone(),
        }
    }
}

pub fn parse_shortened(text: &str) -> Option<String> {
    parse_json(text)?["value"]
        .as_str()
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty())
}

/// Mock provider answer: drop trailing plain words (outside `{…}`/tags) until `max_chars` fits.
pub(crate) fn mock_shorten(p: &Value) -> Value {
    let current = p["current"].as_str().unwrap_or_default();
    let max = p["max_chars"].as_u64().unwrap_or(u64::MAX) as usize;
    let mut words: Vec<(&str, bool)> = vec![];
    let mut depth = 0i32;
    for w in current.split(' ') {
        let plain = depth == 0 && !w.contains(['{', '}', '<', '>', '#']);
        depth += w.matches('{').count() as i32 - w.matches('}').count() as i32;
        words.push((w, plain));
    }
    let len =
        |ws: &[(&str, bool)]| ws.iter().map(|(w, _)| w.chars().count() + 1).sum::<usize>() - 1;
    while words.len() > 1 && len(&words) > max {
        let Some(i) = words.iter().rposition(|(_, plain)| *plain) else {
            break;
        };
        words.remove(i);
    }
    let mut value: Vec<String> = words.iter().map(|(w, _)| w.to_string()).collect();
    // Still too long: abbreviate the last plain word ("Zahlungserinnerung" -> "Zahlungserin.").
    let over = len(&words).saturating_sub(max);
    if let Some(i) = words
        .iter()
        .rposition(|(_, plain)| *plain)
        .filter(|_| over > 0)
    {
        let w: Vec<char> = value[i].chars().collect();
        let keep = w.len().saturating_sub(over + 1).max(2);
        if keep < w.len() {
            value[i] = w[..keep].iter().collect::<String>() + ".";
        }
    }
    json!({ "value": value.join(" ") })
}

/// Mock retry of a translation rejected as `TooWide`: shorten the rejected attempt to `fit_chars`,
/// and by 15 % more on every further rejection (as a real model tightens its wording).
pub(crate) fn mock_fit(item: &Value) -> Option<String> {
    let rejected = item["rejected_attempt"].as_str()?;
    let too_wide = item["issues_to_fix"]
        .as_array()?
        .iter()
        .any(|m| m.as_str().is_some_and(|m| m.starts_with("Too wide")));
    if !too_wide {
        return None;
    }
    let n = rejected.chars().count();
    let max = (item["fit_chars"].as_u64().unwrap_or(n as u64) as usize).min(n * 85 / 100);
    mock_shorten(&json!({"current": rejected, "max_chars": max}))["value"]
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_shortens_plain_words_only() {
        let v = mock_shorten(
            &json!({"current": "Kostenlose Testversion jetzt starten", "max_chars": 20}),
        );
        assert_eq!(v["value"], "Kostenlose");
        let v = mock_shorten(&json!({"current": "Zahlungserinnerung", "max_chars": 13}));
        assert_eq!(v["value"], "Zahlungserin.");
        let v =
            mock_shorten(&json!({"current": "Hallo {name}, willkommen zurück", "max_chars": 14}));
        assert_eq!(v["value"], "Hallo {name},");
        let v = mock_shorten(
            &json!({"current": "{n, plural, one {# Datei} other {# Dateien}} hochladen", "max_chars": 5}),
        );
        assert_eq!(v["value"], "{n, plural, one {# Datei} other {# Dateien}}");
    }

    struct Echo;
    #[async_trait]
    impl Provider for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        async fn complete(&self, req: CompletionRequest) -> Result<Completion> {
            Ok(Completion {
                text: format!(
                    "{} {}",
                    req.images.join(","),
                    req.user.contains("image 1 = `a`")
                ),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn attaches_crops_of_batch_keys() {
        let crops = BTreeMap::from([
            ("a".to_string(), "PNG_A".to_string()),
            ("z".to_string(), "PNG_Z".to_string()),
        ]);
        let p = wrap(Box::new(Echo), &crops);
        let payload = json!({"items": [{"key": "a"}, {"key": "b"}]});
        let req = CompletionRequest {
            model: "m".into(),
            system: String::new(),
            user: payload_block(&payload),
            max_tokens: 1,
            json: true,
            images: vec![],
        };
        assert_eq!(p.complete(req).await.unwrap().text, "PNG_A true");
    }

    #[test]
    fn budget_follows_px_ratio() {
        let s = Shorten {
            locale: "de".into(),
            key: "cta".into(),
            source: "Start".into(),
            current: "x".repeat(40),
            width_px: 100,
            need_width_px: 200,
            height_px: 20,
            need_height_px: 20,
            font: String::new(),
            glossary: vec![],
            rejected: vec![],
            images: vec![],
        };
        assert_eq!(s.max_chars(), 18);
        assert!(s.request("m").user.contains("\"task\": \"shorten\""));
    }
}
