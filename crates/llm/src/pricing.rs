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
    cost_cached_usd(model, input_tokens, 0, 0, output_tokens)
}

/// Message Batches API: every token (cache reads/writes included) at 50 %.
pub const BATCH_DISCOUNT: f64 = 0.5;

/// (cache write, cache read) multipliers of the input price. Anthropic: 5-minute writes 1.25×, reads
/// 0.1× (Opus 5.5 0.05×, Fable 5.1 0.025×). OpenAI: automatic caching, no write premium, reads
/// 0.1× (gpt-5) / 0.25× (gpt-4.1, o-series) / 0.5× (gpt-4o). Others: no known discount.
pub fn cache_factors(model: &str) -> (f64, f64) {
    let m = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let p = |x: &str| m.starts_with(x);
    if p("claude-fable-5-1") || p("claude-mythos-5-1") {
        (1.25, 0.025)
    } else if p("claude-opus-5-5") {
        (1.25, 0.05)
    } else if p("claude-") {
        (1.25, 0.1)
    } else if p("gpt-5") {
        (1.0, 0.1)
    } else if p("gpt-4.1") || p("o3") || p("o4") {
        (1.0, 0.25)
    } else if p("gpt-4o") {
        (1.0, 0.5)
    } else {
        (1.0, 1.0)
    }
}

/// Cost with prompt caching: `input_tokens` is the uncached part only.
pub fn cost_cached_usd(
    model: &str,
    input_tokens: u32,
    cache_write: u32,
    cache_read: u32,
    output_tokens: u32,
) -> f64 {
    match price_per_mtok(model) {
        Some((i, o)) => {
            let (w, r) = cache_factors(model);
            let input = input_tokens as f64 + cache_write as f64 * w + cache_read as f64 * r;
            (input * i + output_tokens as f64 * o) / 1_000_000.0
        }
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
        // Sonnet 5.5: write $2.50, read $0.20 per MTok; Opus 5.5 reads at $0.20.
        let c = cost_cached_usd("claude-sonnet-5-5", 0, 1_000_000, 1_000_000, 0);
        assert!((c - 2.7).abs() < 1e-9, "{c}");
        assert!((cost_cached_usd("claude-opus-5-5", 0, 0, 1_000_000, 0) - 0.2).abs() < 1e-9);
        assert!((cost_cached_usd("gpt-4o", 0, 0, 1_000_000, 0) - 1.25).abs() < 1e-9);
    }
}
