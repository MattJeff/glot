//! Local MCP experiment tools: [`crate::xp_flow`] over REST to the `[server]` of glot.toml (API token in
//! GLOT_TOKEN), with the human confirmation asked through MCP elicitation when the client supports it.
use std::sync::Arc;

use rmcp::{
    Peer, RoleServer,
    handler::server::wrapper::Parameters,
    model::*,
    service::{ElicitationError, RequestContext},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::annotations::ReviewApi;
use crate::xp_flow::{self, Api, CreateParams, PromoteParams, StatusParams};
use crate::{GlotServer, pretty};

/// `/api/projects/{slug}/xp` with the workspace's server settings and token.
struct Http(ReviewApi);

impl Http {
    fn of(ws: &glot_llm::engine::Workspace) -> Result<Self, String> {
        let mut api = ReviewApi::from_workspace(ws)?;
        // ponytail: same client and token as the annotations API, other resource.
        api.base = format!("{}/xp", api.base.trim_end_matches("/review"));
        Ok(Self(api))
    }
}

impl Api for Http {
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
        async move { self.0.call(m, path, &[], body).await }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct Approval {
    /// true = go ahead with the plan above.
    pub approve: bool,
}
rmcp::elicit_safe!(Approval);

impl GlotServer {
    /// Runs `f(api, fits, confirm = false)`; when the flow asks for confirmation and the client supports MCP
    /// elicitation, asks the human and runs it again confirmed. Otherwise the plan goes back to the agent, which
    /// must ask and call again with `confirm: true`.
    async fn xp<F, Fut>(&self, peer: Option<Peer<RoleServer>>, f: F) -> CallToolResult
    where
        F: Fn(Arc<Http>, Value, bool) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Value, String>> + Send,
    {
        let r = self
            .with_ws(move |ws| async move {
                let api = Arc::new(Http::of(&ws)?);
                let fits = json!(glot_core::measure::load_layout(&ws.root).unwrap_or_default());
                let first = f(api.clone(), fits.clone(), false).await?;
                let Some(peer) = peer.filter(|_| first["confirmation_required"] == true) else {
                    return Ok(first);
                };
                let question = first["question"].as_str().unwrap_or("Proceed?").to_string();
                match peer.elicit::<Approval>(question).await {
                    Ok(Some(Approval { approve: true })) => f(api, fits, true).await,
                    Ok(_) | Err(ElicitationError::UserDeclined) | Err(ElicitationError::UserCancelled) => {
                        Ok(json!({ "done": false, "reason": "the user did not approve", "plan": first["plan"] }))
                    }
                    Err(_) => Ok(first),
                }
            })
            .await;
        match r {
            Ok(v) => CallToolResult::success(vec![ContentBlock::text(pretty(&v))]),
            Err(m) => CallToolResult::error(vec![ContentBlock::text(m)]),
        }
    }
}

#[tool_router(router = experiments_router, vis = "pub(crate)")]
impl GlotServer {
    #[tool(
        description = "Create a segmented copy experiment on the glot server (needs [server] + GLOT_TOKEN): AI variants by intent of a key group (title + subtitle + CTA), validated in every locale (placeholders, glossary, pixel budget from .glot/layout.json, copy policy), served at the edge without flicker. Example: \"experiment on the hero for Google traffic in Germany\" → prefix: \"home.hero\", segment: {\"country\": [\"DE\"], \"source\": \"search\"}. Guardrails: traffic is estimated first (refused if it cannot conclude in 90 days unless allow_low_traffic), and nothing is created or paid before the human approves the plan."
    )]
    async fn experiment_create(
        &self,
        Parameters(p): Parameters<CreateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.xp(Some(ctx.peer), move |api, fits, confirm| {
            let mut p = p.clone();
            p.confirm = Some(confirm || p.confirm == Some(true));
            async move { xp_flow::create(&*api, p, fits).await }
        })
        .await
    }

    #[tool(
        description = "Segmented copy experiments: without `experiment`, the list (status, keys, segments, winners); with an id or name, its state per segment, readiness to start (drafts), the latest analysis (P(best), lift and interval per segment, SRM) and the cross-language proposals (a winner to test in the locales not covered)."
    )]
    async fn experiment_status(&self, Parameters(p): Parameters<StatusParams>) -> CallToolResult {
        self.xp(None, move |api, _, _| {
            let p = p.clone();
            async move { xp_flow::status(&*api, p).await }
        })
        .await
    }

    #[tool(
        description = "Promote an experiment's declared winner: scope `all` writes it as the official translation (validated, history; `glot pull` brings it to the repo and glot.lock) or `segment` serves it permanently to the segment where it won (edge personalization). propagate: true also launches the test of its translation in the locales the experiment did not cover, against their official translation. Only declared winners can be promoted; the human approves first."
    )]
    async fn experiment_promote(
        &self,
        Parameters(p): Parameters<PromoteParams>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.xp(Some(ctx.peer), move |api, _, confirm| {
            let mut p = p.clone();
            p.confirm = Some(confirm || p.confirm == Some(true));
            async move { xp_flow::promote(&*api, p).await }
        })
        .await
    }
}
