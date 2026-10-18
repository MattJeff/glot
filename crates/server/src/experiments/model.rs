//! Public types of segmented experiments (JSON = docs/design/segmented-copy-experiments-contract.md §2-§6).
use chrono::{DateTime, Utc};
use glot_core::validate::Issue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const MAX_CELLS: usize = 4;
pub const MAX_ARMS: i32 = 8;
pub const MAX_KEYS: usize = 20;
pub const ANY: &str = "*";

macro_rules! str_enum {
    ($name:ident { $($v:ident = $s:literal),+ }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($v),+ }
        impl $name {
            pub fn as_str(self) -> &'static str { match self { $(Self::$v => $s),+ } }
            pub fn parse(s: &str) -> Option<Self> { match s { $($s => Some(Self::$v),)+ _ => None } }
        }
    };
}
str_enum!(Status { Draft = "draft", Running = "running", Paused = "paused", Stopped = "stopped",
    Promoted = "promoted", Personalized = "personalized" });
str_enum!(Allocation { Fixed = "fixed", Bandit = "bandit", Segmented = "segmented" });
str_enum!(Metric { Conversion = "conversion", Revenue = "revenue" });

impl Status {
    /// Statuses that own their keys (`xp_keys.live`).
    pub fn live(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Personalized)
    }
}

// ---------- segments ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Predicate {
    pub attr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<String>,
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub r#in: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_in: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SegmentRule {
    pub all: Vec<Predicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Segment {
    pub id: Uuid,
    pub name: String,
    pub rule: SegmentRule,
}

fn valid_value(attr: &str, v: &str) -> bool {
    let enum_of = |set: &[&str]| set.contains(&v);
    match attr {
        "country" => v.len() == 2 && v.bytes().all(|b| b.is_ascii_uppercase()),
        "region" => {
            (1..=8).contains(&v.len())
                && v.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        }
        "locale" => crate::validate_locale(v).is_ok(),
        "device" => enum_of(&["mobile", "tablet", "desktop"]),
        "source" => enum_of(&["search", "social", "email", "paid", "direct", "referral"]),
        "visitor" => enum_of(&["new", "returning", "unknown"]),
        "logged_in" => enum_of(&["true", "false"]),
        _ => (1..=64).contains(&v.len()) && !v.chars().any(char::is_control),
    }
}

fn valid_attr(a: &str) -> bool {
    const FIXED: [&str; 8] = [
        "country",
        "region",
        "locale",
        "device",
        "source",
        "visitor",
        "plan",
        "logged_in",
    ];
    FIXED.contains(&a)
        || a.strip_prefix("custom.").is_some_and(|n| {
            (1..=32).contains(&n.len())
                && n.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

impl SegmentRule {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=8).contains(&self.all.len()) {
            return Err("a segment needs 1-8 predicates".into());
        }
        for p in &self.all {
            if !valid_attr(&p.attr) {
                return Err(format!("unknown segment attribute '{}'", p.attr));
            }
            let values: Vec<&String> = match (&p.eq, &p.r#in, &p.not_in) {
                (Some(v), None, None) => vec![v],
                (None, Some(l), None) | (None, None, Some(l)) if (1..=20).contains(&l.len()) => {
                    l.iter().collect()
                }
                _ => {
                    return Err(format!(
                        "'{}': exactly one of eq, in, not_in (lists of 1-20 values)",
                        p.attr
                    ));
                }
            };
            if let Some(v) = values.iter().find(|v| !valid_value(&p.attr, v)) {
                return Err(format!("invalid value '{v}' for '{}'", p.attr));
            }
        }
        Ok(())
    }

    /// Reference semantics for the edge's `decide()`: missing attribute ⇒ eq/in false, not_in true.
    pub fn matches(&self, ctx: &BTreeMap<String, String>) -> bool {
        self.all.iter().all(|p| {
            let v = ctx.get(&p.attr);
            match (&p.eq, &p.r#in, &p.not_in) {
                (Some(e), ..) => v == Some(e),
                (_, Some(l), _) => v.is_some_and(|v| l.contains(v)),
                (.., Some(l)) => v.is_none_or(|v| !l.contains(v)),
                _ => false,
            }
        })
    }
}

// ---------- experiments ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KeyGroup {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub prefix: Option<String>,
}

/// One arm = one intention, with a text per (locale, key). Arm 0 = official catalog (no values).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Variant {
    pub arm: i32,
    pub intent: String,
    pub approved: bool,
    pub values: BTreeMap<String, BTreeMap<String, String>>,
}
pub type Intention = Variant;

/// Per-cell state (`xp_cells`).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Allocations {
    pub cell: String,
    pub weights: Vec<f64>,
    pub looks: i32,
    pub winner: Option<i32>,
    pub personalized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExperimentKey {
    pub key: String,
    pub source: String,
    pub source_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Experiment {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub status: Status,
    pub keys: Vec<ExperimentKey>,
    pub locales: Vec<String>,
    pub cells: Vec<Segment>,
    pub allocation: Allocation,
    pub weights: Option<Vec<f64>>,
    pub traffic_pct: i32,
    pub min_n: i32,
    pub goal: String,
    pub metric: Metric,
    /// Stats denominator: `render` (documents served) or `seen` (block actually seen).
    pub exposure: String,
    pub currency: Option<String>,
    pub guardrails: Vec<String>,
    pub facts: Vec<String>,
    #[schema(value_type = Object)]
    pub fits: serde_json::Value,
    pub review: bool,
    pub arms: Vec<Variant>,
    pub state: Vec<Allocations>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// One element of `Hit.exposures`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Exposure {
    pub id: Uuid,
    pub cell: String,
    pub arm: i32,
    /// Client "seen" beacon (≥ 50 % visible for ≥ 1 s): counted as `$seen`, not `$exposure`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub seen: bool,
    /// Cumulative visible time of the tested block, bucketed by the client (≤ 60 000 ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_ms: Option<f64>,
}

/// Logical view of a goal hit (`Hit` with `event`).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Conversion {
    pub event: String,
    pub value: Option<f64>,
    pub currency: Option<String>,
    pub exposures: Vec<Exposure>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CellCheck {
    pub arm: i32,
    pub key: String,
    pub locale: String,
    pub value: Option<String>,
    pub ok: bool,
    #[schema(value_type = Vec<Object>)]
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Readiness {
    pub ready: bool,
    pub problems: Vec<String>,
    pub cells: Vec<CellCheck>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(j: serde_json::Value) -> SegmentRule {
        serde_json::from_value(j).unwrap()
    }

    #[test]
    fn segment_rules() {
        let r = rule(serde_json::json!({"all": [
            {"attr": "country", "in": ["DE", "AT"]}, {"attr": "source", "eq": "search"},
            {"attr": "custom.tier", "not_in": ["gold"]}]}));
        r.validate().unwrap();
        let ctx = |c: &[(&str, &str)]| {
            c.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert!(r.matches(&ctx(&[("country", "DE"), ("source", "search")])));
        assert!(!r.matches(&ctx(&[
            ("country", "DE"),
            ("source", "search"),
            ("custom.tier", "gold")
        ])));
        assert!(!r.matches(&ctx(&[("country", "FR"), ("source", "search")])));
        assert!(!r.matches(&ctx(&[("source", "search")])));
        for bad in [
            serde_json::json!({"all": []}),
            serde_json::json!({"all": [{"attr": "ip", "eq": "1.2.3.4"}]}),
            serde_json::json!({"all": [{"attr": "country", "eq": "de"}]}),
            serde_json::json!({"all": [{"attr": "device", "eq": "tv"}]}),
            serde_json::json!({"all": [{"attr": "country", "eq": "DE", "in": ["AT"]}]}),
            serde_json::json!({"all": [{"attr": "custom.Bad", "eq": "x"}]}),
        ] {
            assert!(rule(bad.clone()).validate().is_err(), "{bad}");
        }
    }
}
