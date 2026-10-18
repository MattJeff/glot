//! MCP tool `render_check`: `glot labs render` for the agent (read-only; fixes come back as
//! ready-to-apply values for `set_translation`).
use rmcp::{handler::server::wrapper::Parameters, model::*, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use glot_core::render;

use crate::{GlotServer, MAX_ITEMS, check_locale};

#[derive(Deserialize, JsonSchema)]
pub struct RenderParams {
    /// Locales to check (default: all, source included).
    pub locales: Option<Vec<String>>,
}

#[tool_router(router = render_router, vis = "pub(crate)")]
impl GlotServer {
    #[tool(
        description = "Render safety per locale without a browser: characters the webfonts have no glyph for (falls back to a system font; with the unicode-range subset to add), RTL bidi bugs (placeholders/Latin words that reorder the line, reversed brackets), French no-break spaces, locale quotes, decimal separators, mixed digit systems. Errors first, at most 100 findings; each fixable one carries `fixed`, the corrected value to pass to set_translation (it is validated)."
    )]
    async fn render_check(&self, Parameters(p): Parameters<RenderParams>) -> CallToolResult {
        self.run(move |ws| async move {
            let locales = p.locales.unwrap_or_default();
            for l in &locales {
                check_locale(&ws, l)?;
            }
            let r = render::report(&ws.root, &ws.config, &ws.catalogs, &locales, &[]);
            let source = ws.source();
            let findings: Vec<_> = r
                .findings
                .iter()
                .take(MAX_ITEMS)
                .map(|f| {
                    let mut v = json!(f);
                    let text = ws.catalogs.get(&f.locale).and_then(|c| c.entries.get(&f.key));
                    if f.fixable
                        && f.locale != ws.config.source_locale
                        && let Some(t) = text
                        && let Some(fixed) = render::fix_entry(
                            &f.locale,
                            source.entries.get(&f.key).map(String::as_str),
                            t,
                            ws.config.syntax,
                        )
                    {
                        v["fixed"] = json!(fixed);
                    }
                    v
                })
                .collect();
            Ok(json!({
                "errors": r.errors, "warnings": r.warnings, "fixable": r.fixable,
                "fonts": r.fonts, "notes": r.notes, "glyphs": r.glyphs,
                "findings": findings, "findings_total": r.findings.len(),
                "next": "apply each `fixed` with set_translation; for glyph gaps add the unicode-range subset (or a fallback webfont) to the CSS",
            }))
        })
        .await
    }
}
