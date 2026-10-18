//! Remote MCP tools over the live review (same names and behaviour as the local `glot mcp` ones in
//! `glot_mcp::annotations`): they call the `review_live` HTTP handlers as the token's user (a project member).
use axum::{body::to_bytes, extract::State, http::request::Parts};
use rmcp::{
    handler::server::{tool::Extension, wrapper::Parameters},
    model::*,
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{Remote, done, e, from};
use crate::{
    AppJson, AppPath, AppQuery,
    projects::{self, Role},
    review_live::{
        api::{self, Ids},
        model::{Access, Actor, Ctx},
        rewrite,
    },
};

#[derive(Deserialize, JsonSchema)]
pub struct ListParams {
    /// Comma-separated statuses: open, in_progress, resolved, wontfix. Default `open,in_progress`.
    pub status: Option<String>,
    /// Page route without locale prefix, e.g. `/pricing`.
    pub route: Option<String>,
    pub locale: Option<String>,
    pub key: Option<String>,
    /// Max annotations (default 50, max 200).
    pub limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReplyParams {
    /// Annotation id (UUID).
    pub id: String,
    /// Reply posted in the thread (simple markdown).
    pub message: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ResolveParams {
    pub id: String,
    /// What you changed (posted in the thread before resolving).
    pub message: Option<String>,
    /// Close as `wontfix` instead of `resolved`. Default false.
    pub wontfix: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ApplyParams {
    pub id: String,
    /// Text to write instead of the annotation's suggestion.
    pub value: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RewriteParams {
    pub id: String,
    /// How to rephrase. Default: the reviewer's comment.
    pub instruction: Option<String>,
    /// shorter | clearer | formal | informal | warmer | direct | no_jargon | brand_tone
    pub preset: Option<String>,
    /// Proposal id to write (second call, same instruction/preset: served from the cache).
    pub pick: Option<usize>,
}

impl Remote {
    /// The token's user as a review member (the handlers check roles and ownership themselves).
    async fn review_ctx(&self, parts: &Parts) -> Result<Ctx, String> {
        let a = super::auth_of(parts)?;
        let project = projects::load(&self.state, a.user.id, &a.slug, Role::Translator)
            .await
            .map_err(e)?;
        Ok(Ctx {
            project,
            actor: Actor {
                user_id: Some(a.user.id),
                link_id: None,
                name: a.user.name.clone(),
            },
            access: Access::Member,
            expires_at: None,
            widget: false,
        })
    }

    fn st(&self) -> State<crate::AppState> {
        State(self.state.clone())
    }
}

/// `apply` answers a raw response (422 carries per-locale details).
async fn body_of(r: axum::response::Response) -> Result<Value, String> {
    let ok = r.status().is_success();
    let b = to_bytes(r.into_body(), 1 << 20)
        .await
        .map_err(|x| x.to_string())?;
    let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    if ok {
        Ok(v)
    } else {
        Err(format!("{}", v["error"]))
    }
}

#[tool_router(router = annotations_router, vis = "pub(crate)")]
impl Remote {
    #[tool(
        description = "What humans annotated on the running site (live review: reviewers pin comments on texts). Each annotation: id, status, page route/url, locale, the translation key, the reviewer's comment and suggested replacement, the rendered text and CSS selector, the current translation, where the key is used in the code (file:line, from `glot push`) and screenshot URLs. Default: open + in_progress. Workflow: fix each open annotation (annotation_apply_suggestion, set_translation, or edit the code for hardcoded text), then annotation_resolve with a message saying what you changed."
    )]
    async fn annotations_list(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ListParams>,
    ) -> CallToolResult {
        done(
            async {
                let ctx = self.review_ctx(&parts).await?;
                let pid = ctx.project.id;
                let slug = ctx.project.slug.clone();
                let f = from(json!({
                    "status": p.status.unwrap_or_else(|| "open,in_progress".into()),
                    "route": p.route, "locale": p.locale, "key": p.key,
                    "per_page": p.limit.unwrap_or(50).min(200),
                }))?;
                let page = api::list(self.st(), ctx, AppQuery(f)).await.map_err(e)?.0;
                let page = serde_json::to_value(page).map_err(|x| x.to_string())?;
                let items = page["items"].as_array().cloned().unwrap_or_default();
                let keys: Vec<String> = items
                    .iter()
                    .filter_map(|a| a["key"].as_str().map(String::from))
                    .collect();
                let refs: BTreeMap<String, Value> = sqlx::query_as::<_, (String, sqlx::types::Json<Value>)>(
                    "SELECT key, refs FROM keys WHERE project_id = $1 AND key = ANY($2)",
                )
                .bind(pid)
                .bind(&keys)
                .fetch_all(&self.state.db)
                .await
                .map_err(|x| e(x.into()))?
                .into_iter()
                .map(|(k, r)| {
                    let used: Vec<String> = r.0.as_array().into_iter().flatten()
                        .map(|x| format!("{}:{}", x["path"].as_str().unwrap_or_default(), x["line"]))
                        .collect();
                    (k, json!(used))
                })
                .collect();
                let base = format!("{}/api/projects/{slug}/review", crate::github_app::web_origin());
                let view: Vec<Value> = items.iter().map(|a| {
                    let id = a["id"].as_str().unwrap_or_default();
                    json!({
                        "id": id, "status": a["status"], "route": a["route"], "url": a["url"], "locale": a["locale"],
                        "key": a["key"], "comment": a["body"], "suggestion": a["suggestion"], "author": a["author"]["name"],
                        "rendered_text": a["anchor"]["text"], "selector": a["anchor"]["selector"],
                        "source": a["source"], "current": a["translation"]["value"],
                        "comments": a["comment_count"], "approvals": a["approvals"], "updated_at": a["updated_at"],
                        "used_in": a["key"].as_str().and_then(|k| refs.get(k)).cloned().unwrap_or(json!([])),
                        "screenshots": a["attachments"].as_array().into_iter().flatten()
                            .filter_map(|x| x["id"].as_str())
                            .map(|x| format!("{base}/annotations/{id}/attachments/{x}"))
                            .collect::<Vec<_>>(),
                    })
                }).collect();
                Ok(json!({ "total": page["total"], "annotations": view,
                    "next": if view.is_empty() { "nothing to fix" } else { "fix each annotation, then annotation_resolve {id, message}" } }))
            }
            .await,
        )
    }

    #[tool(
        description = "Reply in an annotation's thread (ask the reviewer a question, explain a choice). Does not change its status: use annotation_resolve to close it."
    )]
    async fn annotation_reply(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ReplyParams>,
    ) -> CallToolResult {
        done(
            async {
                let ctx = self.review_ctx(&parts).await?;
                let b = from(json!({ "body": p.message }))?;
                let ids = Ids::of([("id", p.id.to_string())]);
                Ok(api::comment(self.st(), ctx, ids, AppJson(b))
                    .await
                    .map_err(e)?
                    .1
                    .0)
            }
            .await,
        )
    }

    #[tool(
        description = "Close an annotation once fixed: posts `message` (say what you changed) in the thread, then sets the status to resolved (or wontfix with wontfix: true, explaining why)."
    )]
    async fn annotation_resolve(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ResolveParams>,
    ) -> CallToolResult {
        done(
            async {
                if let Some(m) = p.message.as_deref().filter(|m| !m.trim().is_empty()) {
                    let ctx = self.review_ctx(&parts).await?;
                    let b = from(json!({ "body": m }))?;
                    let _ = api::comment(
                        self.st(),
                        ctx,
                        Ids::of([("id", p.id.to_string())]),
                        AppJson(b),
                    )
                    .await
                    .map_err(e)?;
                }
                let status = if p.wontfix.unwrap_or(false) {
                    "wontfix"
                } else {
                    "resolved"
                };
                let ctx = self.review_ctx(&parts).await?;
                let b = from(json!({ "status": status }))?;
                let a = api::update(
                    self.st(),
                    ctx,
                    Ids::of([("id", p.id.to_string())]),
                    AppJson(b),
                )
                .await
                .map_err(e)?
                .0;
                Ok(json!({ "id": a.id, "status": a.status }))
            }
            .await,
        )
    }

    #[tool(
        description = "Apply an annotation's suggested text (or `value`) as the reviewed translation of its key and locale (validated: placeholders, plurals, glossary; nothing written on errors), and resolve the annotation."
    )]
    async fn annotation_apply_suggestion(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ApplyParams>,
    ) -> CallToolResult {
        done(
            async {
                let ctx = self.review_ctx(&parts).await?;
                let body = serde_json::to_vec(&json!({ "value": p.value })).map_err(|x| x.to_string())?;
                let r = api::apply(self.st(), ctx, Ids::of([("id", p.id.to_string())]), body.into())
                    .await
                    .map_err(e)?;
                let a = body_of(r).await?;
                Ok(json!({ "id": a["id"], "status": a["status"], "key": a["key"], "locale": a["locale"],
                           "value": a["translation"]["value"] }))
            }
            .await,
        )
    }

    #[tool(
        description = "Ask the project LLM for 2-3 rewordings of the annotated text following the reviewer's comment (or `instruction` / `preset`), each declined and validated in every project locale. First call: read the proposals (id, text, explanation, valid). Second call with the same arguments plus `pick: <id>`: writes that proposal in every locale as reviewed translations, then annotation_resolve."
    )]
    async fn annotation_rewrite(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RewriteParams>,
    ) -> CallToolResult {
        done(
            async {
                let ids = || AppPath(std::collections::HashMap::from([("id".to_string(), p.id.to_string())]));
                let b = from(json!({ "instruction": p.instruction, "preset": p.preset }))?;
                let ctx = self.review_ctx(&parts).await?;
                let r = rewrite::rephrase_handler(self.st(), ctx, ids(), AppJson(b)).await.map_err(e)?.0;
                let Some(n) = p.pick else {
                    let proposals: Vec<Value> = r.rewrite.proposals.iter()
                        .map(|x| json!({ "id": x.id, "text": x.text, "explanation": x.explanation, "valid": x.valid,
                                         "locales": x.variants.keys().collect::<Vec<_>>() }))
                        .collect();
                    return Ok(json!({ "id": p.id, "rewrite_id": r.rewrite_id, "proposals": proposals,
                                      "rejected": r.rewrite.rejected }));
                };
                let ctx = self.review_ctx(&parts).await?;
                let b = from(json!({ "rewrite_id": r.rewrite_id, "proposal_id": n }))?;
                let out = body_of(rewrite::apply_handler(self.st(), ctx, ids(), AppJson(b)).await.map_err(e)?).await?;
                Ok(json!({ "id": p.id, "status": out["annotation"]["status"], "results": out["results"],
                           "source_text": out["source_text"] }))
            }
            .await,
        )
    }
}
