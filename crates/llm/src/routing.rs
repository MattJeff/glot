//! Quality score of a written translation + routing to human review (`glot queue`).
use glot_core::{
    config::{LlmConfig, QaConfig},
    validate::{Issue, IssueCode, Severity},
};

use crate::{Result, provider::Provider};

/// 0-100: the judge score when a judge ran (else 100), minus deterministic penalties for what the
/// validator still reports on the written value (length, glossary, whitespace… warnings; errors are
/// never written). The judge's own LowScore warning is already in its score.
pub fn score(judge: Option<u8>, issues: &[Issue]) -> u8 {
    let penalty: i32 = issues
        .iter()
        .filter(|i| i.code != IssueCode::LowScore)
        .map(|i| match i.severity {
            Severity::Error => 50,
            _ => 15,
        })
        .sum();
    (judge.unwrap_or(100) as i32 - penalty).clamp(0, 100) as u8
}

/// Why a written translation must wait for a human, if it must: a `risk: <tag>` line in the key's
/// note, a key matching a `[qa.risk]` pattern, or a score under `review_threshold`.
pub fn route(qa: &QaConfig, key: &str, note: Option<&str>, score: u8) -> Option<String> {
    if let Some(tag) = note.and_then(|n| {
        n.lines()
            .find_map(|l| l.trim().strip_prefix("risk:"))
            .map(|t| t.trim().to_string())
    }) {
        return Some(format!("risk: {tag}"));
    }
    for (tag, patterns) in &qa.risk {
        if patterns
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .any(|p| p.matches(key))
        {
            return Some(format!("risk: {tag}"));
        }
    }
    qa.review_threshold
        .filter(|t| score < *t)
        .map(|t| format!("score {score} < {t}"))
}

/// Provider of `fallback_model` when `fallback_provider` names another one (cross-provider escalation,
/// e.g. a local first pass escalated to Anthropic). None => the main provider serves the fallback model.
pub fn fallback_provider(cfg: &LlmConfig) -> Result<Option<Box<dyn Provider>>> {
    let Some(p) = cfg
        .fallback_provider
        .as_ref()
        .filter(|p| **p != cfg.provider)
    else {
        return Ok(None);
    };
    let fb = LlmConfig {
        provider: p.clone(),
        base_url: cfg.fallback_base_url.clone(),
        api_key_env: cfg.fallback_api_key_env.clone(),
        ..cfg.clone()
    };
    crate::provider::from_config(&fb, None).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_and_route() {
        let warn = Issue {
            code: IssueCode::TooLong,
            severity: Severity::Warning,
            message: String::new(),
        };
        assert_eq!(score(None, &[]), 100);
        assert_eq!(score(Some(90), std::slice::from_ref(&warn)), 75);
        let mut qa = QaConfig {
            review_threshold: Some(80),
            ..Default::default()
        };
        qa.risk.insert("legal".into(), vec!["legal.*".into()]);
        assert_eq!(route(&qa, "home.title", None, 95), None);
        assert_eq!(
            route(&qa, "home.title", None, 75).as_deref(),
            Some("score 75 < 80")
        );
        assert_eq!(
            route(&qa, "legal.terms", None, 100).as_deref(),
            Some("risk: legal")
        );
        assert_eq!(
            route(&qa, "checkout.pay", Some("Pay button\nrisk: payment"), 100).as_deref(),
            Some("risk: payment")
        );
    }
}
