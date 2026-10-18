//! MCP tools: back-translation review and glossary mining.
use rmcp::{handler::server::wrapper::Parameters, model::*, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use glot_llm::{mine, review};

use crate::{GlotServer, MAX_ITEMS, check_locale};

#[derive(Deserialize, JsonSchema)]
pub struct ReviewParams {
    /// Target locales (default: all). Prefer one locale at a time.
    pub locales: Option<Vec<String>>,
    /// Only these keys (default: every translated key, cached results are free).
    pub keys: Option<Vec<String>>,
    /// Write the suggestion of every `different`/`nuance` result (validated). Default false.
    pub fix: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct MineParams {
    /// Write the mined terms to glossary.toml and retranslate the inconsistent keys. Default false (report only).
    pub apply: Option<bool>,
}

#[tool_router(router = review_router, vis = "pub(crate)")]
impl GlotServer {
    #[tool(
        description = "Check translations in a language you do not speak: literal back-translation into the source language, a short explanation of the choices (register, glossary terms, plurals, cultural adaptations) and a fidelity verdict (same | nuance | different) with a suggestion. Cached in .glot/review.json (only changed pairs cost tokens). Returns non-`same` results first, at most 100."
    )]
    async fn review_translation(&self, Parameters(p): Parameters<ReviewParams>) -> CallToolResult {
        self.run(move |mut ws| async move {
            for l in p.locales.iter().flatten() {
                check_locale(&ws, l)?;
            }
            let (mut rows, usage) = review::review_workspace(&ws, p.locales.as_deref(), p.keys.as_deref(), false)
                .await
                .map_err(|e| format!("review failed: {e}"))?;
            rows.sort_by_key(|r| r.verdict == review::Fidelity::Same);
            let fixed = if p.fix.unwrap_or(false) {
                let (ok, rejected) = review::apply_suggestions(&mut ws, &rows).map_err(|e| e.to_string())?;
                json!({ "written": ok, "rejected": rejected })
            } else {
                json!(null)
            };
            let total = rows.len();
            let same = rows.iter().filter(|r| r.verdict == review::Fidelity::Same).count();
            rows.truncate(MAX_ITEMS);
            Ok(json!({ "total": total, "same": same, "results": rows, "fixed": fixed, "cost_usd": usage.cost_usd,
                       "next": "for `different` rows, apply the suggestion (fix: true) or set_translation; ask a native speaker when unsure" }))
        })
        .await
    }

    #[tool(
        description = "Mine glossary terms from the source catalog (frequency heuristics + LLM selection), with the canonical translation per locale and the keys whose translation is inconsistent. apply: true writes glossary.toml and retranslates those keys (terminology refactor)."
    )]
    async fn mine_glossary(&self, Parameters(p): Parameters<MineParams>) -> CallToolResult {
        self.run(move |mut ws| async move {
            let (mined, usage) = mine::mine(&ws)
                .await
                .map_err(|e| format!("mining failed: {e}"))?;
            let mut cost = usage.cost_usd;
            if p.apply.unwrap_or(false) && !mined.is_empty() {
                cost += mine::apply(&mut ws, &mined)
                    .await
                    .map_err(|e| format!("apply failed: {e}"))?
                    .cost_usd;
            }
            Ok(json!({ "terms": mined, "applied": p.apply.unwrap_or(false), "cost_usd": cost }))
        })
        .await
    }
}
