//! Remote MCP experiment tools: the same flows as the local `glot mcp` (`crates/mcp/src/xp_flow.rs`), over the
//! in-process `/xp` handlers with the caller's identity (roles, quotas and validation identical to the REST API).
//! Remote MCP is stateless (no elicitation): create / promote return the plan with `confirmation_required` until
//! the agent calls again with `confirm: true` after asking the human.
use axum::{
    extract::State,
    http::request::Parts,
    response::{IntoResponse, Response},
};
use rmcp::{
    handler::server::{tool::Extension, wrapper::Parameters},
    model::*,
    tool, tool_router,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Remote, done};
use crate::{
    AppJson, AppPath, AppState,
    auth::{AuthUser, User},
    experiments::{api, generate, promote, rebalance},
};

#[path = "../../../mcp/src/xp_flow.rs"]
mod xp_flow;
use xp_flow::{Api, CreateParams, PromoteParams, StatusParams};

struct InProcess {
    state: AppState,
    user: User,
    slug: String,
}

/// Any handler result as JSON, errors as `code (status): message`.
async fn val(r: Response) -> Result<Value, String> {
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 8 << 20)
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() {
        Ok(v)
    } else {
        Err(format!(
            "{} ({}): {}",
            v["error"]["code"].as_str().unwrap_or("error"),
            status.as_u16(),
            v["error"]["message"].as_str().unwrap_or("")
        ))
    }
}

fn json_body<T: serde::de::DeserializeOwned>(b: &Value) -> Result<AppJson<T>, String> {
    serde_json::from_value(b.clone())
        .map(AppJson)
        .map_err(|e| format!("invalid arguments: {e}"))
}

impl Api for InProcess {
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send {
        let (method, path) = (method.to_string(), path.to_string());
        async move {
            let s = || State(self.state.clone());
            let u = || AuthUser(self.user.clone());
            let slug = || AppPath(self.slug.clone());
            let b = body.unwrap_or_else(|| json!({}));
            let segs: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
            let id = |x: &str| {
                x.parse::<Uuid>()
                    .map_err(|_| format!("not an experiment id: {x}"))
            };
            let at = |x: &str| id(x).map(|i| AppPath((self.slug.clone(), i)));
            let r = match (method.as_str(), segs.as_slice()) {
                ("GET", []) => api::list(s(), u(), slug()).await.into_response(),
                ("POST", []) => api::create(s(), u(), slug(), json_body(&b)?)
                    .await
                    .into_response(),
                ("GET", ["segments"]) => api::list_segments(s(), u(), slug()).await.into_response(),
                ("POST", ["estimate"]) => rebalance::estimate(s(), u(), slug(), json_body(&b)?)
                    .await
                    .into_response(),
                ("GET", [x]) => api::get_one(s(), u(), at(x)?).await.into_response(),
                ("GET", [x, "check"]) => api::check(s(), u(), at(x)?).await.into_response(),
                ("GET", [x, "results"]) => {
                    rebalance::results(s(), u(), at(x)?).await.into_response()
                }
                ("GET", [x, "propagate"]) => {
                    promote::proposals(s(), u(), at(x)?).await.into_response()
                }
                ("POST", [x, "propagate"]) => promote::propagate(s(), u(), at(x)?, json_body(&b)?)
                    .await
                    .into_response(),
                ("POST", [x, "generate"]) => generate::generate(s(), u(), at(x)?, json_body(&b)?)
                    .await
                    .into_response(),
                ("POST", [x, "start"]) => api::start(s(), u(), at(x)?).await.into_response(),
                ("POST", [x, "promote"]) => api::promote(s(), u(), at(x)?, json_body(&b)?)
                    .await
                    .into_response(),
                (m, _) => return Err(format!("unsupported {m} /xp{path}")),
            };
            val(r).await
        }
    }
}

impl Remote {
    fn xp(&self, parts: &Parts) -> Result<InProcess, String> {
        let (_, AuthUser(user), AppPath(slug), _) = self.ctx(parts)?;
        Ok(InProcess {
            state: self.state.clone(),
            user,
            slug,
        })
    }
}

#[tool_router(router = experiments_router, vis = "pub(super)")]
impl Remote {
    #[tool(
        description = "Create a segmented copy experiment: AI variants by intent of a key group (title + subtitle + CTA), validated in every locale (placeholders, glossary, pixel budget, copy policy), served at the edge without flicker. Example: \"experiment on the hero for Google traffic in Germany\" → prefix: \"home.hero\", segment: {\"country\": [\"DE\"], \"source\": \"search\"}. Guardrails: traffic is estimated first (refused if it cannot conclude in 90 days unless allow_low_traffic); the first call only returns the plan (confirmation_required) — show it to the user and call again with confirm: true only if they agree (variants cost LLM budget, a live experiment uses the plan quota)."
    )]
    async fn experiment_create(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CreateParams>,
    ) -> CallToolResult {
        done(async { xp_flow::create(&self.xp(&parts)?, p, json!({})).await }.await)
    }

    #[tool(
        description = "Segmented copy experiments: without `experiment`, the list (status, keys, segments, winners); with an id or name, its state per segment, readiness to start (drafts), the latest analysis (P(best), lift and interval per segment, SRM) and the cross-language proposals (a winner to test in the locales not covered)."
    )]
    async fn experiment_status(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<StatusParams>,
    ) -> CallToolResult {
        done(async { xp_flow::status(&self.xp(&parts)?, p).await }.await)
    }

    #[tool(
        description = "Promote an experiment's declared winner: scope `all` writes it as the official translation (validated, history; the repo gets it with `glot pull` or translate_repo) or `segment` serves it permanently to the segment where it won. propagate: true also launches the test of its translation in the locales not covered, against their official translation. Only declared winners; the first call returns the plan (confirmation_required): ask the user, then call again with confirm: true."
    )]
    async fn experiment_promote(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PromoteParams>,
    ) -> CallToolResult {
        done(async { xp_flow::promote(&self.xp(&parts)?, p).await }.await)
    }
}
