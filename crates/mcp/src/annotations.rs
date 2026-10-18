//! Live-review annotations (docs/design/live-review-contract.md) through the configured glot server (`[server]` +
//! API token): what humans pinned on the running site, for the coding agent (MCP tools below) and the terminal
//! (`glot annotations`, which reuses [`ReviewApi`] and [`agent_view`]).
use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine;
use rmcp::{handler::server::wrapper::Parameters, model::*, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use glot_llm::engine::Workspace;

use crate::{GlotServer, pretty};

pub const ACTIVE: &str = "open,in_progress";
const ALL: &str = "open,in_progress,resolved,wontfix";
/// Screenshots inlined in `annotations_list` (first capture of the first N annotations).
const MAX_IMAGES: usize = 3;

/// `/api/projects/{slug}/review` with a bearer API token.
pub struct ReviewApi {
    pub base: String,
    token: String,
    http: reqwest::Client,
}

impl ReviewApi {
    pub fn from_workspace(ws: &Workspace) -> Result<Self, String> {
        let srv = ws.config.server.as_ref().ok_or(
            "no [server] section in glot.toml: annotations live on the glot server (add `[server] url = \"…\" project = \"<slug>\"` and an API token in GLOT_TOKEN)",
        )?;
        let token = std::env::var(&srv.token_env).map_err(|_| {
            format!(
                "set {} to an API token (dashboard → API tokens)",
                srv.token_env
            )
        })?;
        Ok(Self {
            base: format!(
                "{}/api/projects/{}/review",
                srv.url.trim_end_matches('/'),
                srv.project
            ),
            token,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .map_err(|e| e.to_string())?,
        })
    }

    pub async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .query(query);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req
            .send()
            .await
            .map_err(|e| format!("glot server unreachable: {e}"))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or(text);
            return Err(format!("server returned {status}: {msg}"));
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| format!("invalid server response: {e}"))
    }

    pub async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        self.call(reqwest::Method::GET, path, query, None).await
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.call(reqwest::Method::POST, path, &[], Some(body))
            .await
    }

    pub async fn bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        Ok(res.bytes().await.map_err(|e| e.to_string())?.to_vec())
    }

    /// Full UUID from a unique prefix (the CLI shows 8 characters).
    pub async fn resolve_id(&self, id: &str) -> Result<String, String> {
        if id.len() == 36 {
            return Ok(id.to_string());
        }
        let page = self
            .get(
                "/annotations",
                &[("status", ALL.into()), ("per_page", "200".into())],
            )
            .await?;
        let hits: Vec<&str> = page["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["id"].as_str())
            .filter(|i| i.starts_with(id))
            .collect();
        match hits.as_slice() {
            [one] => Ok(one.to_string()),
            [] => Err(format!("no annotation id starts with `{id}`")),
            _ => Err(format!("`{id}` is ambiguous ({} annotations)", hits.len())),
        }
    }

    /// `{items, total}`, newest activity first.
    pub async fn list(&self, f: &Filter) -> Result<Value, String> {
        let mut q = vec![
            ("status", f.status.clone().unwrap_or_else(|| ACTIVE.into())),
            ("per_page", f.limit.unwrap_or(50).to_string()),
        ];
        for (k, v) in [("route", &f.route), ("locale", &f.locale), ("key", &f.key)] {
            if let Some(v) = v {
                q.push((k, v.clone()));
            }
        }
        self.get("/annotations", &q).await
    }

    /// Optional reply, then the status change (the reply lands in the thread before the status event).
    pub async fn set_status(
        &self,
        id: &str,
        status: &str,
        message: Option<&str>,
    ) -> Result<Value, String> {
        if let Some(m) = message.filter(|m| !m.trim().is_empty()) {
            self.post(&format!("/annotations/{id}/comments"), json!({ "body": m }))
                .await?;
        }
        self.call(
            reqwest::Method::PATCH,
            &format!("/annotations/{id}"),
            &[],
            Some(json!({ "status": status })),
        )
        .await
    }

    /// Write the suggestion (or `value`) locally first when the repo has the key (validated, nothing written on
    /// errors), then on the server, which records it as reviewed and resolves the annotation.
    pub async fn apply(
        &self,
        ws: Option<&mut Workspace>,
        id: &str,
        value: Option<String>,
    ) -> Result<Value, String> {
        let a = self.get(&format!("/annotations/{id}"), &[]).await?;
        let value = value
            .or_else(|| a["suggestion"].as_str().map(String::from))
            .ok_or("this annotation has no suggestion: pass a value")?;
        let (Some(key), Some(locale)) = (a["key"].as_str(), a["locale"].as_str()) else {
            return Err(
                "this annotation is not attached to a key: fix the text in code, then resolve it"
                    .into(),
            );
        };
        let mut local = Value::Null;
        if let Some(ws) =
            ws.filter(|w| w.source().entries.contains_key(key) && locale != w.config.source_locale)
        {
            let issues = ws
                .set_translation(locale, key, &value)
                .map_err(|e| format!("could not write the local catalog: {e}"))?;
            if glot_core::validate::has_errors(&issues) {
                return Err(format!(
                    "not applied, validation errors: {}",
                    pretty(&json!(issues))
                ));
            }
            local = json!({ "written": true, "issues": issues });
        }
        let mut out = self
            .post(
                &format!("/annotations/{id}/apply"),
                json!({ "value": value }),
            )
            .await?;
        out["local"] = local;
        Ok(out)
    }
}

#[derive(Default, Deserialize, JsonSchema)]
pub struct Filter {
    /// Comma-separated statuses: open, in_progress, resolved, wontfix. Default `open,in_progress`.
    pub status: Option<String>,
    /// Page route without locale prefix, e.g. `/pricing`.
    pub route: Option<String>,
    /// Displayed locale, e.g. `de`.
    pub locale: Option<String>,
    /// Only annotations on this translation key.
    pub key: Option<String>,
    /// Max annotations (default 50, server max 200).
    pub limit: Option<usize>,
}

/// One annotation as the coding agent needs it: what/where on the site, the key, where it lives in the repo
/// (code usages file:line, catalog file, local value), the thread and the screenshots.
pub fn agent_view(
    api: &ReviewApi,
    a: &Value,
    ws: Option<&Workspace>,
    refs: &BTreeMap<String, Vec<String>>,
) -> Value {
    let id = a["id"].as_str().unwrap_or_default();
    let key = a["key"].as_str();
    let locale = a["locale"].as_str().unwrap_or_default();
    let mut v = json!({
        "id": id, "status": a["status"], "route": a["route"], "url": a["url"], "locale": locale,
        "key": key, "comment": a["body"], "suggestion": a["suggestion"], "author": a["author"]["name"],
        "rendered_text": a["anchor"]["text"], "selector": a["anchor"]["selector"],
        "source": a["source"], "server_value": a["translation"]["value"],
        "comments": a["comment_count"], "approvals": a["approvals"], "updated_at": a["updated_at"],
        "screenshots": a["attachments"].as_array().into_iter().flatten()
            .filter_map(|x| x["id"].as_str())
            .map(|x| format!("{}/annotations/{id}/attachments/{x}", api.base))
            .collect::<Vec<_>>(),
    });
    if let Some(thread) = a.get("comments").and_then(Value::as_array) {
        v["thread"] = thread
            .iter()
            .map(|c| json!({ "author": c["author"]["name"], "body": c["body"], "at": c["created_at"] }))
            .collect();
    }
    if let (Some(ws), Some(k)) = (ws, key) {
        v["used_in"] = json!(refs.get(k).cloned().unwrap_or_default());
        v["local_value"] = json!(ws.catalogs.get(locale).and_then(|c| c.entries.get(k)));
        v["catalog_file"] = json!(ws.config.catalog_paths(&ws.root, locale).first().map(|p| {
            p.strip_prefix(&ws.root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        }));
    }
    v
}

/// key -> ["file:line"] from the extractor (empty when extraction fails: best effort).
pub fn code_refs(ws: &Workspace) -> BTreeMap<String, Vec<String>> {
    let Ok(report) = ws.extract() else {
        return BTreeMap::new();
    };
    ws.contexts(&report)
        .into_iter()
        .map(|(k, c)| {
            let r = c
                .refs
                .iter()
                .map(|r| format!("{}:{}", r.path, r.line))
                .collect();
            (k.to_string(), r)
        })
        .collect()
}

// ---------- MCP tools ----------

#[derive(Deserialize, JsonSchema)]
pub struct ReplyParams {
    /// Annotation id (full UUID or the unique prefix shown by `glot annotations list`).
    pub id: String,
    /// Reply posted in the annotation thread (simple markdown).
    pub message: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ResolveParams {
    pub id: String,
    /// What you changed (posted in the thread before resolving, so the reviewer sees why it is closed).
    pub message: Option<String>,
    /// Close as `wontfix` instead of `resolved` (explain why in `message`). Default false.
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
    /// How to rephrase (e.g. "shorter, keep the verb first"). Default: the reviewer's comment.
    pub instruction: Option<String>,
    /// shorter | clearer | formal | informal | warmer | direct | no_jargon | brand_tone
    pub preset: Option<String>,
    /// Proposal id to write (second call, same instruction/preset: the proposals come from the cache).
    pub pick: Option<usize>,
}

impl ReviewApi {
    /// Proposals for an annotation (`rephrase`, cached server-side), then optionally write proposal `pick`
    /// in every locale (`rephrase/apply`).
    pub async fn rewrite(
        &self,
        id: &str,
        instruction: Option<String>,
        preset: Option<String>,
        pick: Option<usize>,
    ) -> Result<Value, String> {
        let r = self
            .post(
                &format!("/annotations/{id}/rephrase"),
                json!({ "instruction": instruction, "preset": preset }),
            )
            .await?;
        let Some(n) = pick else {
            let proposals: Vec<Value> = r["proposals"].as_array().into_iter().flatten()
                .map(|p| json!({ "id": p["id"], "text": p["text"], "explanation": p["explanation"], "valid": p["valid"],
                                 "locales": p["variants"].as_object().map(|v| v.keys().cloned().collect::<Vec<_>>()) }))
                .collect();
            return Ok(
                json!({ "rewrite_id": r["rewrite_id"], "proposals": proposals, "rejected": r["rejected"] }),
            );
        };
        self.post(
            &format!("/annotations/{id}/rephrase/apply"),
            json!({ "rewrite_id": r["rewrite_id"], "proposal_id": n }),
        )
        .await
    }
}

#[tool_router(router = annotations_router, vis = "pub(crate)")]
impl GlotServer {
    #[tool(
        description = "What humans annotated on the running site (live review: reviewers pin comments on texts; needs `[server]` in glot.toml and GLOT_TOKEN). Each annotation: id, status, page route/url, locale, the translation key, the reviewer's comment and suggested replacement, the rendered text and CSS selector, where the key is used in the code (file:line), its catalog file and local value, and screenshot URLs (the first screenshots are attached as images). Default: open + in_progress. Workflow: fix each open annotation (annotation_apply_suggestion, set_translation, or edit the code for untranslated/hardcoded text), then annotation_resolve with a message saying what you changed."
    )]
    async fn annotations_list(&self, Parameters(f): Parameters<Filter>) -> CallToolResult {
        let r = self
            .with_ws(move |ws| async move {
                let api = ReviewApi::from_workspace(&ws)?;
                let page = api.list(&f).await?;
                let items = page["items"].as_array().cloned().unwrap_or_default();
                let refs = if items.iter().any(|a| a["key"].is_string()) {
                    code_refs(&ws)
                } else {
                    BTreeMap::new()
                };
                let view: Vec<Value> = items
                    .iter()
                    .map(|a| agent_view(&api, a, Some(&ws), &refs))
                    .collect();
                let mut images = vec![];
                for a in &items {
                    if images.len() == MAX_IMAGES {
                        break;
                    }
                    let Some(att) = a["attachments"].get(0) else { continue };
                    let path = format!(
                        "/annotations/{}/attachments/{}",
                        a["id"].as_str().unwrap_or_default(),
                        att["id"].as_str().unwrap_or_default()
                    );
                    if let Ok(b) = api.bytes(&path).await {
                        images.push(json!([
                            base64::engine::general_purpose::STANDARD.encode(b),
                            att["content_type"].as_str().unwrap_or("image/png"),
                            a["id"]
                        ]));
                    }
                }
                Ok(json!({ "total": page["total"], "annotations": view, "_images": images,
                    "next": if view.is_empty() { "nothing to fix" } else { "fix each annotation, then annotation_resolve {id, message}" } }))
            })
            .await;
        match r {
            Ok(mut v) => {
                let images = v
                    .as_object_mut()
                    .and_then(|o| o.remove("_images"))
                    .and_then(|i| i.as_array().cloned())
                    .unwrap_or_default();
                let mut blocks = vec![ContentBlock::text(pretty(&v))];
                for i in images {
                    blocks.push(ContentBlock::text(format!(
                        "screenshot of annotation {}",
                        i[2].as_str().unwrap_or_default()
                    )));
                    blocks.push(ContentBlock::image(
                        i[0].as_str().unwrap_or_default(),
                        i[1].as_str().unwrap_or_default(),
                    ));
                }
                CallToolResult::success(blocks)
            }
            Err(m) => CallToolResult::error(vec![ContentBlock::text(m)]),
        }
    }

    #[tool(
        description = "Reply in an annotation's thread (e.g. ask the reviewer a question, or explain a choice). Does not change its status: use annotation_resolve to close it."
    )]
    async fn annotation_reply(&self, Parameters(p): Parameters<ReplyParams>) -> CallToolResult {
        self.run(move |ws| async move {
            let api = ReviewApi::from_workspace(&ws)?;
            let id = api.resolve_id(&p.id).await?;
            api.post(
                &format!("/annotations/{id}/comments"),
                json!({ "body": p.message }),
            )
            .await
        })
        .await
    }

    #[tool(
        description = "Close an annotation once fixed: posts `message` (say what you changed, e.g. \"Shortened the CTA in de.json\") in the thread, then sets the status to resolved (or wontfix with wontfix: true, explaining why)."
    )]
    async fn annotation_resolve(&self, Parameters(p): Parameters<ResolveParams>) -> CallToolResult {
        self.run(move |ws| async move {
            let api = ReviewApi::from_workspace(&ws)?;
            let id = api.resolve_id(&p.id).await?;
            let status = if p.wontfix.unwrap_or(false) {
                "wontfix"
            } else {
                "resolved"
            };
            let a = api.set_status(&id, status, p.message.as_deref()).await?;
            Ok(json!({ "id": id, "status": a["status"] }))
        })
        .await
    }

    #[tool(
        description = "Apply an annotation's suggested text (or `value`) as the translation of its key and locale: validated (placeholders, plurals, glossary), written to the local catalog when the repo has the key, recorded on the server as human-reviewed, and the annotation is resolved. On validation errors nothing is written."
    )]
    async fn annotation_apply_suggestion(
        &self,
        Parameters(p): Parameters<ApplyParams>,
    ) -> CallToolResult {
        self.run(move |mut ws| async move {
            let api = ReviewApi::from_workspace(&ws)?;
            let id = api.resolve_id(&p.id).await?;
            let a = api.apply(Some(&mut ws), &id, p.value).await?;
            Ok(
                json!({ "id": id, "status": a["status"], "key": a["key"], "locale": a["locale"],
                       "value": a["translation"]["value"], "local": a["local"] }),
            )
        })
        .await
    }

    #[tool(
        description = "Ask the project LLM for 2-3 rewordings of the annotated text following the reviewer's comment (or `instruction` / `preset`), each declined and validated in every project locale. First call: read the proposals (id, text, explanation, valid). Second call with the same arguments plus `pick: <id>`: writes that proposal in every locale as reviewed translations (then `glot pull` to get them locally), then annotation_resolve."
    )]
    async fn annotation_rewrite(&self, Parameters(p): Parameters<RewriteParams>) -> CallToolResult {
        self.run(move |ws| async move {
            let api = ReviewApi::from_workspace(&ws)?;
            let id = api.resolve_id(&p.id).await?;
            let mut r = api.rewrite(&id, p.instruction, p.preset, p.pick).await?;
            r["id"] = json!(id);
            Ok(r)
        })
        .await
    }
}
