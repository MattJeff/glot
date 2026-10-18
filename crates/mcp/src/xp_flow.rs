//! Experiment tools shared by the local MCP (`glot mcp`, REST to the `[server]`) and the remote one
//! (`crates/server/src/mcp_remote.rs`, in-process handlers): `experiment_create`, `experiment_status`,
//! `experiment_promote`. Only the transport differs ([`Api`]); the guardrails are the same:
//! - **traffic**: the segment's traffic is estimated first; too little to conclude within 90 days → refused
//!   unless `allow_low_traffic`;
//! - **budget**: nothing is created and no LLM call is made before the human confirms the plan (variants cost
//!   LLM tokens, a live experiment uses the plan's quota — the server answers 402 past it);
//! - **human confirmation**: every create / start / promote returns `confirmation_required` until the human
//!   said yes (local: MCP elicitation when the client supports it; otherwise `confirm: true` after asking);
//! - **only winners travel**: a promotion needs a declared winner (P(best) over the look threshold).
//!
//! ponytail: this file is compiled into both crates (`#[path]` from the server) so the server does not pull the
//! extractor (oxc) through a glot-mcp dependency; move it to its own crate if a third user appears.
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// `/api/projects/{slug}/xp` + `path`, JSON in and out; errors as one readable line.
pub trait Api {
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send;
}

pub const DEFAULT_INTENTS: [&str; 2] = ["direct_benefit", "social_proof"];

#[derive(Deserialize, JsonSchema, Default, Clone)]
pub struct CreateParams {
    /// Key prefix of the group (`home.hero` = title + subtitle + CTA). Use this or `keys`.
    pub prefix: Option<String>,
    /// Explicit keys (≤ 20).
    pub keys: Option<Vec<String>>,
    /// Who sees it, as attribute → value or values: `{"country": ["DE"], "source": "search"}` for "Google traffic
    /// in Germany". Attributes: country (ISO, upper case), region, locale, device (mobile|tablet|desktop),
    /// source (search|social|email|paid|direct|referral), visitor (new|returning), plan, logged_in, custom.<name>.
    /// Omit for all traffic.
    pub segment: Option<BTreeMap<String, Value>>,
    /// Intents, one variant each: social_proof | direct_benefit | honest_urgency | simplicity | curiosity |
    /// authority | reassurance | segment. Default: direct_benefit, social_proof.
    pub intents: Option<Vec<String>>,
    /// Goal event tracked by the app (`glot.track("signup")`). Default `signup`.
    pub goal: Option<String>,
    /// The only facts a variant may cite ("2,000 teams", "rated 4.8/5").
    pub facts: Option<Vec<String>>,
    /// Locales to test (default: every project locale).
    pub locales: Option<Vec<String>>,
    pub name: Option<String>,
    /// Start right after generation (default true; false = leave a draft to review in the dashboard).
    pub start: Option<bool>,
    /// Proceed although the estimated traffic cannot conclude within 90 days.
    pub allow_low_traffic: Option<bool>,
    /// Only after the human approved the returned plan.
    pub confirm: Option<bool>,
}

#[derive(Deserialize, JsonSchema, Default, Clone)]
pub struct StatusParams {
    /// Experiment id or name (omit to list them all).
    pub experiment: Option<String>,
}

#[derive(Deserialize, JsonSchema, Default, Clone)]
pub struct PromoteParams {
    /// Experiment id or name.
    pub experiment: String,
    /// Winning arm (default: the declared winner).
    pub arm: Option<i32>,
    /// `all`: becomes the official translation everywhere (history; the repo gets it on `glot pull`).
    /// `segment`: served permanently to the segment where it won (edge personalization). Default `all`.
    pub scope: Option<String>,
    /// Segment id or name for `segment` (default: where the arm won).
    pub cell: Option<String>,
    /// Also launch the test of its translation in the locales the experiment did not cover.
    pub propagate: Option<bool>,
    /// Only after the human approved the returned plan.
    pub confirm: Option<bool>,
}

fn rule_of(seg: &BTreeMap<String, Value>) -> Result<Value, String> {
    let all: Vec<Value> = seg
        .iter()
        .map(|(attr, v)| match v {
            Value::Array(a) => Ok(json!({ "attr": attr, "in": a })),
            Value::String(s) => Ok(json!({ "attr": attr, "eq": s })),
            Value::Bool(b) => Ok(json!({ "attr": attr, "eq": b.to_string() })),
            _ => Err(format!("segment `{attr}`: a string or a list of strings")),
        })
        .collect::<Result<_, _>>()?;
    Ok(json!({ "all": all }))
}

fn rule_text(rule: &Value) -> String {
    rule["all"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| match (&p["eq"], &p["in"]) {
            (Value::String(e), _) => format!("{} = {e}", p["attr"].as_str().unwrap_or("")),
            (_, Value::Array(l)) => format!(
                "{} in {}",
                p["attr"].as_str().unwrap_or(""),
                l.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

pub async fn find(api: &impl Api, which: &str) -> Result<Value, String> {
    let all = api.call("GET", "", None).await?;
    all.as_array()
        .and_then(|a| a.iter().find(|x| x["id"] == which || x["name"] == which))
        .cloned()
        .ok_or_else(|| {
            format!(
                "no experiment `{which}`: call experiment_status without arguments to list them"
            )
        })
}

fn summary(x: &Value) -> Value {
    json!({
        "id": x["id"], "name": x["name"], "status": x["status"], "goal": x["goal"],
        "keys": x["keys"].as_array().into_iter().flatten().map(|k| k["key"].clone()).collect::<Vec<_>>(),
        "locales": x["locales"],
        "segments": x["cells"].as_array().into_iter().flatten().map(|c| c["name"].clone()).collect::<Vec<_>>(),
        "arms": x["arms"].as_array().into_iter().flatten().map(|a| json!({ "arm": a["arm"], "intent": a["intent"], "approved": a["approved"] })).collect::<Vec<_>>(),
        "state": x["state"],
    })
}

fn confirmation(question: String, plan: Value) -> Value {
    json!({ "confirmation_required": true, "question": question, "plan": plan,
            "next": "show the plan to the user; only if they agree, call again with the same arguments and confirm: true" })
}

/// Existing segment with the same rule, else an inline one (`{name, rule}`, created with the experiment).
async fn cell_for(api: &impl Api, rule: &Value) -> Result<Value, String> {
    let segs = api.call("GET", "/segments", None).await?;
    let segs = segs.as_array().cloned().unwrap_or_default();
    if let Some(s) = segs.iter().find(|s| s["rule"] == *rule) {
        return Ok(s["id"].clone());
    }
    let base: String = rule_text(rule).chars().take(90).collect();
    let mut name = base.clone();
    for n in 2.. {
        if !segs.iter().any(|s| s["name"] == name.as_str()) {
            break;
        }
        name = format!("{base} ({n})");
    }
    Ok(json!({ "name": name, "rule": rule }))
}

/// `experiment_create`: estimate → (confirm) → draft → AI variants → start.
pub async fn create(api: &impl Api, p: CreateParams, fits: Value) -> Result<Value, String> {
    if p.prefix.is_none() && p.keys.as_ref().is_none_or(Vec::is_empty) {
        return Err("pass `prefix` (e.g. `home.hero`) or `keys`".into());
    }
    let rule = p
        .segment
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(rule_of)
        .transpose()?;
    let intents: Vec<String> = p
        .intents
        .clone()
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| DEFAULT_INTENTS.map(String::from).to_vec());
    let goal = p.goal.clone().unwrap_or_else(|| "signup".into());
    let estimate = api
        .call(
            "POST",
            "/estimate",
            Some(json!({ "rule": rule, "arms": intents.len() + 1, "goal": goal })),
        )
        .await;
    let low = match &estimate {
        Ok(e) => e["too_long"] == true || e["daily_visitors"].as_f64().unwrap_or(0.0) <= 0.0,
        Err(_) => false,
    };
    let estimate = estimate.unwrap_or_else(|e| json!({ "unavailable": e }));
    if low && !p.allow_low_traffic.unwrap_or(false) {
        return Ok(json!({
            "created": false, "reason": "not enough traffic to conclude within 90 days", "estimate": estimate,
            "next": "widen the segment (more countries, drop `source`), test all traffic, or call again with allow_low_traffic: true if the user accepts an inconclusive test",
        }));
    }
    let what = p
        .prefix
        .clone()
        .map(|x| format!("{x}.*"))
        .unwrap_or_else(|| p.keys.clone().unwrap_or_default().join(", "));
    let who = rule
        .as_ref()
        .map(rule_text)
        .unwrap_or_else(|| "all traffic".into());
    let name = p
        .name
        .clone()
        .unwrap_or_else(|| format!("{what} · {who}"))
        .chars()
        .take(100)
        .collect::<String>();
    let start = p.start.unwrap_or(true);
    if !p.confirm.unwrap_or(false) {
        return Ok(confirmation(
            format!(
                "Create experiment \"{name}\": {} AI variant(s) ({}) of {what} for {who}, goal `{goal}`{}? Variants use the project's LLM budget{}.",
                intents.len(),
                intents.join(", "),
                if start {
                    ", started right away"
                } else {
                    ", left as a draft"
                },
                if start {
                    " and the experiment counts against the plan's live-experiment quota"
                } else {
                    ""
                }
            ),
            json!({ "name": name, "keys": p.keys, "prefix": p.prefix, "segment": rule, "intents": intents, "goal": goal,
                    "locales": p.locales, "start": start, "estimate": estimate }),
        ));
    }
    let mut body = json!({ "name": name, "goal": goal, "facts": p.facts.clone().unwrap_or_default(), "fits": fits });
    if let Some(k) = &p.keys {
        body["keys"] = json!(k);
    }
    if let Some(x) = &p.prefix {
        body["prefix"] = json!(x);
    }
    if let Some(l) = &p.locales {
        body["locales"] = json!(l);
    }
    if let Some(r) = &rule {
        body["cells"] = json!([cell_for(api, r).await?]);
    }
    let x = api.call("POST", "", Some(body)).await?;
    let id = x["id"].as_str().unwrap_or_default().to_string();
    let generated = api
        .call(
            "POST",
            &format!("/{id}/generate"),
            Some(json!({ "intents": intents })),
        )
        .await?;
    let mut started = Value::Bool(false);
    if start {
        started = match api
            .call("POST", &format!("/{id}/start"), Some(json!({})))
            .await
        {
            Ok(_) => Value::Bool(true),
            Err(e) => json!({ "error": e }),
        };
    }
    let x = api.call("GET", &format!("/{id}"), None).await?;
    let next = if started == true {
        "live: the edge serves it within a minute; call experiment_status later for results"
    } else {
        "draft: fix what `started.error` / `rejected` says (or edit variants in the dashboard), then start it there"
    };
    Ok(
        json!({ "experiment": summary(&x), "estimate": estimate, "rejected": generated["rejected"],
               "warnings": generated["warnings"], "cost_usd": generated["cost_usd"], "started": started, "next": next }),
    )
}

/// `experiment_status`: list, or one experiment with readiness, latest analysis and cross-language proposals.
pub async fn status(api: &impl Api, p: StatusParams) -> Result<Value, String> {
    let Some(w) = p.experiment else {
        let all = api.call("GET", "", None).await?;
        return Ok(
            json!({ "experiments": all.as_array().into_iter().flatten().map(summary).collect::<Vec<_>>() }),
        );
    };
    let x = find(api, &w).await?;
    let id = x["id"].as_str().unwrap_or_default().to_string();
    let readiness = if x["status"] == "draft" {
        api.call("GET", &format!("/{id}/check"), None).await.ok()
    } else {
        None
    };
    let results = api
        .call("GET", &format!("/{id}/results"), None)
        .await
        .ok()
        .map(|r| r["latest"].clone());
    let proposals = api
        .call("GET", &format!("/{id}/propagate"), None)
        .await
        .ok()
        .map(|r| r["proposals"].clone());
    Ok(
        json!({ "experiment": summary(&x), "readiness": readiness, "results": results, "cross_language": proposals }),
    )
}

/// `experiment_promote`: everywhere or for the winning segment, then optionally the cross-language test.
pub async fn promote(api: &impl Api, p: PromoteParams) -> Result<Value, String> {
    let x = find(api, &p.experiment).await?;
    let id = x["id"].as_str().unwrap_or_default().to_string();
    let won: Vec<&Value> = x["state"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["winner"].as_i64().is_some_and(|w| w > 0))
        .collect();
    let arm = match p.arm {
        Some(a) => a,
        None => won.first().and_then(|c| c["winner"].as_i64()).ok_or("no declared winner yet: keep the experiment running (experiment_status shows progress)")? as i32,
    };
    let cells_won: Vec<String> = won
        .iter()
        .filter(|c| c["winner"] == arm)
        .filter_map(|c| c["cell"].as_str().map(String::from))
        .collect();
    if cells_won.is_empty() {
        return Err(format!(
            "arm {arm} has not won in any segment: promoting it would be a guess (minimum traffic and P(best) not reached)"
        ));
    }
    let scope = p.scope.clone().unwrap_or_else(|| "all".into());
    let mut body = json!({ "scope": scope, "arm": arm });
    let cell_name = |c: &str| {
        x["cells"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["id"] == c || s["name"] == c)
            .and_then(|s| s["name"].as_str())
            .unwrap_or(if c == "*" { "all other traffic" } else { c })
            .to_string()
    };
    let what = match scope.as_str() {
        "all" => format!(
            "arm {arm} becomes the official translation of {} key(s) in every tested locale",
            x["keys"].as_array().map_or(0, Vec::len)
        ),
        "segment" => {
            let cell = match &p.cell {
                Some(c) => x["cells"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["id"] == c.as_str() || s["name"] == c.as_str())
                    .and_then(|s| s["id"].as_str())
                    .unwrap_or(c)
                    .to_string(),
                None => cells_won[0].clone(),
            };
            body["cell"] = json!(cell);
            format!(
                "arm {arm} is served permanently to segment \"{}\"",
                cell_name(&cell)
            )
        }
        s => return Err(format!("scope `{s}`: all | segment")),
    };
    let propagate = p.propagate.unwrap_or(false);
    if !p.confirm.unwrap_or(false) {
        return Ok(confirmation(
            format!(
                "Promote \"{}\": {what}{}?",
                x["name"].as_str().unwrap_or(""),
                if propagate {
                    ", then start a test of its translation in the locales not covered yet (LLM translation)"
                } else {
                    ""
                }
            ),
            json!({ "experiment": summary(&x), "body": body, "propagate": propagate }),
        ));
    }
    let promoted = api
        .call("POST", &format!("/{id}/promote"), Some(body))
        .await?;
    let propagated = if propagate {
        Some(
            api.call(
                "POST",
                &format!("/{id}/propagate"),
                Some(json!({ "arm": arm })),
            )
            .await?,
        )
    } else {
        None
    };
    let proposals = if propagate {
        None
    } else {
        api.call("GET", &format!("/{id}/propagate"), None)
            .await
            .ok()
            .map(|r| r["proposals"].clone())
    };
    Ok(json!({
        "experiment": summary(&promoted),
        "cross_language": propagated.map(|v| json!({ "experiment": summary(&v["experiment"]), "started": v["started"], "start_blocked": v["start_blocked"], "rejected": v["rejected"] })),
        "proposals": proposals,
        "next": if scope == "all" { "the official translations changed on the server: run `glot pull` (catalogs + glot.lock) or translate_repo" } else { "served by the edge to that segment; re-test in ~90 days" },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_shorthand() {
        let seg: BTreeMap<String, Value> =
            serde_json::from_value(json!({"country": ["DE"], "source": "search"})).unwrap();
        let r = rule_of(&seg).unwrap();
        assert_eq!(
            r,
            json!({"all": [{"attr": "country", "in": ["DE"]}, {"attr": "source", "eq": "search"}]})
        );
        assert_eq!(rule_text(&r), "country in DE · source = search");
        assert!(rule_of(&serde_json::from_value(json!({"country": 3})).unwrap()).is_err());
    }

    /// Canned server: records every call; `daily` = estimated visitors/day.
    struct Fake {
        daily: f64,
        winner: Option<i32>,
        log: std::sync::Mutex<Vec<String>>,
    }

    impl Api for Fake {
        fn call(
            &self,
            method: &str,
            path: &str,
            _: Option<Value>,
        ) -> impl std::future::Future<Output = Result<Value, String>> + Send {
            self.log.lock().unwrap().push(format!("{method} {path}"));
            let x = json!({"id": "e1", "name": "hero", "status": "running", "keys": [{"key": "home.hero.title"}], "cells": [],
                           "arms": [{"arm": 0}, {"arm": 1, "intent": "social_proof"}],
                           "state": [{"cell": "*", "winner": self.winner}]});
            let v = match (method, path) {
                ("POST", "/estimate") => {
                    Ok(json!({"daily_visitors": self.daily, "too_long": self.daily < 10.0}))
                }
                ("GET", "") => Ok(json!([x])),
                ("GET", "/segments") => Ok(json!([])),
                ("POST", "") => Ok(json!({"id": "e2"})),
                ("GET", "/e2") | ("POST", "/e1/promote") => Ok(x),
                _ => Ok(json!({})),
            };
            async move { v }
        }
    }

    fn fake(daily: f64, winner: Option<i32>) -> Fake {
        Fake {
            daily,
            winner,
            log: Default::default(),
        }
    }

    #[tokio::test]
    async fn guardrails() {
        let ask = CreateParams {
            prefix: Some("home.hero".into()),
            segment: Some(serde_json::from_value(json!({"country": ["DE"]})).unwrap()),
            ..Default::default()
        };
        // Too little traffic: refused, nothing created.
        let api = fake(1.0, None);
        let v = create(&api, ask.clone(), json!({})).await.unwrap();
        assert_eq!(v["created"], false);
        // Enough traffic: the plan first, still nothing created nor generated.
        let api = fake(5000.0, None);
        let v = create(&api, ask.clone(), json!({})).await.unwrap();
        assert_eq!(v["confirmation_required"], true);
        assert!(
            v["question"].as_str().unwrap().contains("country in DE"),
            "{v}"
        );
        assert_eq!(*api.log.lock().unwrap(), ["POST /estimate"]);
        // Confirmed: draft, generation, start.
        let v = create(
            &api,
            CreateParams {
                confirm: Some(true),
                ..ask
            },
            json!({}),
        )
        .await
        .unwrap();
        assert_eq!(v["started"], true, "{v}");
        let log = api.log.lock().unwrap().join(", ");
        assert!(
            log.contains("POST /e2/generate") && log.contains("POST /e2/start"),
            "{log}"
        );

        // Promotion: no winner → refused; winner → plan, then promoted only once confirmed.
        let p = PromoteParams {
            experiment: "hero".into(),
            ..Default::default()
        };
        assert!(
            promote(&fake(0.0, None), p.clone())
                .await
                .unwrap_err()
                .contains("no declared winner")
        );
        let api = fake(0.0, Some(1));
        assert_eq!(
            promote(&api, p.clone()).await.unwrap()["confirmation_required"],
            true
        );
        assert!(
            !api.log
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.ends_with("/promote"))
        );
        promote(
            &api,
            PromoteParams {
                confirm: Some(true),
                ..p
            },
        )
        .await
        .unwrap();
        assert!(
            api.log
                .lock()
                .unwrap()
                .iter()
                .any(|c| c == "POST /e1/promote")
        );
    }
}
