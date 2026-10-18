/// (model prefix, input $/MTok, output $/MTok). Longest matching prefix wins.
const PRICES: &[(&str, f64, f64)] = &[
    ("mock", 0.0, 0.0),
    ("claude-fable-5-1", 10.0, 50.0),
    ("claude-fable-5", 10.0, 50.0),
    ("claude-opus-5-5", 4.0, 20.0),
    ("claude-opus-5", 5.0, 25.0),
    ("claude-opus-4", 5.0, 25.0),
    ("claude-sonnet-5-5", 2.0, 10.0),
    ("claude-sonnet-5", 2.0, 10.0),
    ("claude-sonnet-4", 3.0, 15.0),
    ("claude-haiku-5-5", 0.10, 0.50),
    ("claude-haiku-4-5", 1.0, 5.0),
    ("gpt-5-nano", 0.05, 0.40),
    ("gpt-5-mini", 0.25, 2.0),
    ("gpt-5", 1.25, 10.0),
    ("gpt-4.1-nano", 0.10, 0.40),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gpt-4.1", 2.0, 8.0),
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.50, 10.0),
    ("o4-mini", 1.10, 4.40),
    ("o3", 2.0, 8.0),
    ("mistral-large", 2.0, 6.0),
    ("mistral-small", 0.10, 0.30),
];

/// USD per million tokens (input, output). Unknown models => None (cost reported as 0 with a warning).
pub fn price_per_mtok(model: &str) -> Option<(f64, f64)> {
    // OpenRouter-style ids: "anthropic/claude-sonnet-5-5".
    let m = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    PRICES
        .iter()
        .filter(|(p, ..)| m.starts_with(p))
        .max_by_key(|(p, ..)| p.len())
        .map(|&(_, i, o)| (i, o))
}

pub fn cost_usd(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    match price_per_mtok(model) {
        Some((i, o)) => (input_tokens as f64 * i + output_tokens as f64 * o) / 1_000_000.0,
        None => {
            tracing::warn!(model, "unknown model price, cost counted as $0");
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices() {
        assert_eq!(price_per_mtok("claude-haiku-5-5"), Some((0.10, 0.50)));
        assert_eq!(price_per_mtok("gpt-4o-mini-2024-07-18"), Some((0.15, 0.60)));
        assert_eq!(
            price_per_mtok("anthropic/claude-sonnet-5-5"),
            Some((2.0, 10.0))
        );
        assert_eq!(price_per_mtok("llama3.2"), None);
        assert!((cost_usd("claude-sonnet-5-5", 1_000_000, 100_000) - 3.0).abs() < 1e-9);
    }
}
