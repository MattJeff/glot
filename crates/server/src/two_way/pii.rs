//! PII masking before any text leaves for the LLM: emails, URLs, IBANs, card-like numbers, phone numbers and
//! order/reference numbers become `{email1}`-style placeholders, restored after translation. The placeholders
//! are ICU arguments on purpose: the translation pipeline already refuses an answer that loses or invents one.
//!
//! ponytail: pattern-based. Names and postal addresses written in free text are NOT detected (no NER);
//! add a local NER pass if a customer needs it. Over-masking (a date read as a phone number) is the safe side:
//! the original is put back verbatim.
use regex::Regex;
use std::sync::LazyLock;

/// Order matters: the most specific pattern first, each one sees the placeholders of the previous ones.
static PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    let re = |s: &str| Regex::new(s).expect("pii regex");
    vec![
        // Braces and angle brackets are syntax for the pipeline (ICU arguments, tags): hide the customer's own.
        ("sym", re(r"[{}<>]+")),
        // ASCII classes and no `\b`: Japanese or Chinese text has no spaces around an address or a number.
        ("url", re(r"(?i)(?:https?://|www\.)[!-~]+")),
        (
            "email",
            re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+"),
        ),
        (
            "iban",
            re(r"[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,4})?"),
        ),
        ("card", re(r"\d(?:[ -]?\d){12,18}")),
        ("phone", re(r"\+?\(?\d[\d ().-]{6,}\d")),
        // `#482`, `ORD-2024-0091`, `1Z999AA10123456784`: any token with a `#` or at least 5 digits.
        ("ref", re(r"#\d+|[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*")),
    ]
});

#[derive(Debug, Default, Clone)]
pub struct Masker {
    /// (placeholder without braces, original)
    map: Vec<(String, String)>,
}

fn digits(s: &str) -> usize {
    s.chars().filter(char::is_ascii_digit).count()
}

fn luhn(s: &str) -> bool {
    let mut sum = 0;
    for (i, d) in s.chars().filter_map(|c| c.to_digit(10)).rev().enumerate() {
        let d = if i % 2 == 1 { d * 2 } else { d };
        sum += if d > 9 { d - 9 } else { d };
    }
    sum % 10 == 0
}

fn keep(kind: &str, m: &str) -> bool {
    match kind {
        "card" => luhn(m),
        // 8 digits or more: a phone number, or any long number we would rather not send.
        "phone" => digits(m) >= 8,
        "ref" => m.starts_with('#') || digits(m) >= 5,
        _ => true,
    }
}

impl Masker {
    /// `text` with every detected item replaced by its placeholder. The same original always gets the same
    /// placeholder, so one masker used for a whole thread keeps `{email1}` meaning the same address.
    pub fn mask(&mut self, text: &str) -> String {
        let mut out = text.to_string();
        for (kind, re) in PATTERNS.iter() {
            let mut next = String::with_capacity(out.len());
            let mut last = 0;
            for m in re.find_iter(&out) {
                // Never re-mask one of our own placeholders (`{ref1}` is `{` + token + `}`).
                let inside = out[..m.start()].ends_with('{') && out[m.end()..].starts_with('}');
                if inside || !keep(kind, m.as_str()) {
                    continue;
                }
                next.push_str(&out[last..m.start()]);
                next.push('{');
                next.push_str(&self.placeholder(kind, m.as_str()));
                next.push('}');
                last = m.end();
            }
            next.push_str(&out[last..]);
            out = next;
        }
        out
    }

    fn placeholder(&mut self, kind: &str, original: &str) -> String {
        if let Some((p, _)) = self
            .map
            .iter()
            .find(|(p, o)| o == original && p.starts_with(kind))
        {
            return p.clone();
        }
        let n = self.map.iter().filter(|(p, _)| p.starts_with(kind)).count() + 1;
        let p = format!("{kind}{n}");
        self.map.push((p.clone(), original.to_string()));
        p
    }

    /// Put the originals back. `Err` lists the placeholders of `masked_source` the translation lost.
    pub fn restore(&self, masked_source: &str, translated: &str) -> Result<String, Vec<String>> {
        let mut out = translated.to_string();
        let mut lost = vec![];
        for (p, original) in &self.map {
            let ph = format!("{{{p}}}");
            if masked_source.contains(&ph) && !out.contains(&ph) {
                lost.push(p.clone());
            }
            out = out.replace(&ph, original);
        }
        if lost.is_empty() { Ok(out) } else { Err(lost) }
    }

    /// Restore, never failing (context shown to a person, back-translations).
    pub fn restore_lossy(&self, text: &str) -> String {
        self.restore("", text).unwrap_or_default()
    }

    /// Items masked that are personal data (the `sym` placeholders are syntax, not data).
    pub fn count(&self) -> usize {
        self.map
            .iter()
            .filter(|(p, _)| !p.starts_with("sym"))
            .count()
    }

    pub fn originals(&self) -> impl Iterator<Item = &str> {
        self.map
            .iter()
            .filter(|(p, _)| !p.starts_with("sym"))
            .map(|(_, o)| o.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_and_restores() {
        let text = "Hi, I'm at jane.doe@example.co.uk / +33 6 12 34 56 78. Order #48213 and ORD-2024-00917, \
                    card 4242 4242 4242 4242, IBAN FR76 3000 6000 0112 3456 7890 189, see https://shop.example/o/9?t=abc {x}";
        let mut m = Masker::default();
        let masked = m.mask(text);
        for secret in [
            "jane.doe",
            "example.co.uk",
            "12 34 56 78",
            "48213",
            "ORD-2024-00917",
            "4242",
            "FR76",
            "shop.example",
        ] {
            assert!(!masked.contains(secret), "{secret} leaked in: {masked}");
        }
        assert!(
            masked.contains("{email1}") && masked.contains("{card1}"),
            "{masked}"
        );
        assert!(
            masked.contains("{iban1}") && masked.contains("{url1}"),
            "{masked}"
        );
        assert!(
            masked.contains("{phone1}") && masked.contains("{ref1}"),
            "{masked}"
        );
        assert_eq!(m.count(), 7, "{masked}");
        assert_eq!(m.restore(&masked, &masked).unwrap(), text);
        // A translation that reorders the placeholders still restores; one that drops one is reported.
        assert!(m.restore(&masked, "{email1}").is_err());
    }

    #[test]
    fn same_value_same_placeholder_and_plain_text_untouched() {
        let mut m = Masker::default();
        assert_eq!(
            m.mask("Thanks a lot, see you in 2 days!"),
            "Thanks a lot, see you in 2 days!"
        );
        let a = m.mask("write to a@b.io");
        let b = m.mask("again: a@b.io and c@d.io");
        assert_eq!(a, "write to {email1}");
        assert_eq!(b, "again: {email1} and {email2}");
        // A 16-digit number failing Luhn is not a card, but is still hidden.
        assert_eq!(m.mask("1234 5678 9012 3456"), "{phone1}");
        // No spaces around the data in Japanese: still found, and only the data is hidden.
        assert_eq!(
            m.mask("注文番号は48213です。メールはjane@x.ioです"),
            "注文番号は{ref1}です。メールは{email3}です"
        );
    }
}
