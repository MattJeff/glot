//! `GET /api/projects/{slug}/insights/recommendations`: which language to add next and what it should bring.
//! Reads only the existing `insight_daily` aggregates (preferred language × served locale × event per day),
//! the catalog coverage and the dry-run cost estimate. Nothing new is collected.
//!
//! Method (docs: web/content/docs/insights.md, "Language recommendations"):
//! - demand = visits whose preferred language is L, scaled to 30 days; trend = second half vs first half.
//! - fallback rate = goal conversions / visits of those visitors today (served another locale, or a
//!   partially translated L); native rate = same ratio for visitors served their own language
//!   (other languages than the source when there are enough, else every matched visit; L excluded).
//! - both rates get a Beta(1 + conversions, 1 + visits − conversions) posterior (uniform prior);
//!   gain/month = monthly demand × (native − fallback), 90 % interval from the normal approximation
//!   of the difference of the two posteriors.
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    ApiResult, AppPath, AppQuery, AppState,
    auth::AuthUser,
    insights::{estimate_cost, lang},
    projects::{self, ProjectRow, Role},
};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/projects/{slug}/insights/recommendations",
        get(handler),
    )
}

/// Below this many visits from a language, its conversion rate says nothing.
pub const MIN_VISITS: i64 = 30;
/// Below this many native visits, the "served in their language" baseline says nothing.
pub const MIN_NATIVE: i64 = 100;
/// "solid": both groups at least this many visits and `SOLID_CONVERSIONS` conversions each.
pub const SOLID_VISITS: i64 = 300;
pub const SOLID_CONVERSIONS: i64 = 10;
/// An offered locale under this translated share is a candidate too (missing strings fall back).
pub const UNDER_TRANSLATED: f64 = 0.9;
const Z90: f64 = 1.645;

/// One aggregate row of `insight_daily` over the window (`late` = second half of it).
#[derive(Debug, Clone)]
pub struct Row {
    pub preferred: String,
    pub served: String,
    /// false = a visit, true = a goal conversion.
    pub conversion: bool,
    pub late: bool,
    pub n: i64,
}

#[derive(Serialize, Debug, Clone, Copy, Default, PartialEq)]
pub struct Rate {
    pub visits: i64,
    pub conversions: i64,
    /// Posterior mean (uniform prior), not the raw ratio: never 0 or 1 on thin data.
    pub rate: f64,
    pub ci_low: f64,
    pub ci_high: f64,
}

impl Rate {
    pub fn new(visits: i64, conversions: i64) -> Self {
        let (a, b) = beta(visits, conversions);
        let (mean, sd) = (a / (a + b), var(a, b).sqrt());
        Self {
            visits,
            conversions: conversions.min(visits),
            rate: mean,
            ci_low: (mean - Z90 * sd).max(0.0),
            ci_high: (mean + Z90 * sd).min(1.0),
        }
    }
}

fn beta(visits: i64, conversions: i64) -> (f64, f64) {
    let c = conversions.clamp(0, visits.max(0));
    (1.0 + c as f64, 1.0 + (visits.max(0) - c) as f64)
}

fn var(a: f64, b: f64) -> f64 {
    a * b / ((a + b).powi(2) * (a + b + 1.0))
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub mid: f64,
    pub low: f64,
    pub high: f64,
}

/// Extra conversions per month if `fallback` visitors converted like `native` ones (90 % interval).
pub fn gain(monthly_visits: f64, native: Rate, fallback: Rate) -> Interval {
    let (an, bn) = beta(native.visits, native.conversions);
    let (af, bf) = beta(fallback.visits, fallback.conversions);
    let diff = an / (an + bn) - af / (af + bf);
    let sd = (var(an, bn) + var(af, bf)).sqrt();
    Interval {
        mid: monthly_visits * diff,
        low: monthly_visits * (diff - Z90 * sd),
        high: monthly_visits * (diff + Z90 * sd),
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct Recommendation {
    /// Locale to add (or to finish translating).
    pub locale: String,
    /// Preferred language (primary subtag) the demand is counted on.
    pub language: String,
    /// missing | under_translated
    pub kind: &'static str,
    pub visits: i64,
    /// `visits` scaled to 30 days.
    pub monthly_visits: f64,
    /// Share of all visits in the window.
    pub share: f64,
    /// Second half of the window vs first half (+0.2 = +20 %); `None` without visits in the first half.
    pub trend: Option<f64>,
    /// Locale these visitors are served most today, and the share of their visits it covers.
    pub served_now: Option<String>,
    pub served_now_share: f64,
    /// Translated share of the catalog in `locale` (0 when missing).
    pub coverage: f64,
    pub fallback: Rate,
    pub native: Rate,
    /// `None` when the goal is not tracked: the ranking then uses demand and cost only.
    pub gain_monthly: Option<Interval>,
    pub cost_usd: f64,
    pub keys_to_translate: i64,
    /// cost / expected monthly gain (first month payback), when the gain is positive.
    pub cost_per_conversion: Option<f64>,
    /// Billing on and the owner below Growth: `gain_monthly` and `cost_per_conversion` withheld (null).
    pub gain_locked: bool,
    /// insufficient | indicative | solid
    pub confidence: &'static str,
    /// Machine codes, translated by the dashboard: no_conversions, thin_demand, thin_baseline,
    /// baseline_includes_source, gain_uncertain, no_gain, rising, falling, under_translated.
    pub reasons: Vec<&'static str>,
    /// Plain English sentence (API, CLI, agents).
    pub summary: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct Recommendations {
    pub days: i64,
    pub goal: String,
    pub model: String,
    /// Insights public key (empty state: the one tag to install).
    pub public_key: String,
    pub total_visits: i64,
    /// At least one `goal` event in the window.
    pub conversions_tracked: bool,
    pub keys: i64,
    pub recommendations: Vec<Recommendation>,
}

/// Offered locale with its translated key count.
pub struct Offered {
    pub locale: String,
    pub translated: i64,
}

/// Pure ranking over aggregates. `cost(locale, kind)` = (USD, keys to translate).
pub fn rank(
    days: i64,
    source_locale: &str,
    offered: &[Offered],
    keys: i64,
    rows: &[Row],
    cost: impl Fn(&str, &'static str) -> (f64, i64),
) -> (i64, bool, Vec<Recommendation>) {
    let source = lang(source_locale);
    let per_month = 30.0 / days.max(1) as f64;
    #[derive(Default)]
    struct Lang {
        visits: i64,
        conv: i64,
        early: i64,
        late: i64,
        served: BTreeMap<String, i64>,
        // matched = served their own language
        m_visits: i64,
        m_conv: i64,
        // not matched = what a missing language's visitors get today
        u_visits: i64,
        u_conv: i64,
    }
    let mut by: BTreeMap<String, Lang> = BTreeMap::new();
    // Share is over every visit, unknown language included.
    let total: i64 = rows.iter().filter(|r| !r.conversion).map(|r| r.n).sum();
    let mut tracked = false;
    for r in rows.iter().filter(|r| r.preferred != "und") {
        let l = by.entry(r.preferred.clone()).or_default();
        let matched = lang(&r.served) == r.preferred;
        if r.conversion {
            tracked |= r.n > 0;
            l.conv += r.n;
            if matched {
                l.m_conv += r.n
            } else {
                l.u_conv += r.n
            }
        } else {
            l.visits += r.n;
            *l.served.entry(r.served.clone()).or_default() += r.n;
            if r.late {
                l.late += r.n
            } else {
                l.early += r.n
            }
            if matched {
                l.m_visits += r.n
            } else {
                l.u_visits += r.n
            }
        }
    }
    let pool = |skip: &str, with_source: bool| {
        by.iter()
            .filter(|(k, _)| *k != skip && (with_source || **k != source))
            .fold((0, 0), |(v, c), (_, l)| (v + l.m_visits, c + l.m_conv))
    };

    let mut out = vec![];
    for (language, l) in &by {
        if *language == source {
            continue;
        }
        let off = offered.iter().find(|o| lang(&o.locale) == *language);
        let coverage = off.map_or(0.0, |o| o.translated as f64 / keys.max(1) as f64);
        let (kind, locale, fb_visits, fb_conv) = match off {
            None => ("missing", default_locale(language), l.u_visits, l.u_conv),
            Some(o) if keys > 0 && coverage < UNDER_TRANSLATED => {
                ("under_translated", o.locale.clone(), l.visits, l.conv)
            }
            Some(_) => continue,
        };
        if fb_visits == 0 {
            continue;
        }
        let mut reasons = vec![];
        let (mut nv, mut nc) = pool(language, false);
        if nv < MIN_NATIVE {
            (nv, nc) = pool(language, true);
            reasons.push("baseline_includes_source");
        }
        let (native, fallback) = (Rate::new(nv, nc), Rate::new(fb_visits, fb_conv));
        let monthly = fb_visits as f64 * per_month;
        let g = tracked.then(|| gain(monthly, native, fallback));
        let served_now = l.served.iter().max_by_key(|(_, n)| **n);
        let trend = (l.early > 0).then(|| l.late as f64 / l.early as f64 - 1.0);
        if !tracked {
            reasons.push("no_conversions");
        }
        if fb_visits < MIN_VISITS {
            reasons.push("thin_demand");
        }
        if tracked && nv < MIN_NATIVE {
            reasons.push("thin_baseline");
        }
        match g {
            Some(g) if g.high <= 0.0 => reasons.push("no_gain"),
            Some(g) if g.low <= 0.0 => reasons.push("gain_uncertain"),
            _ => {}
        }
        if l.early.min(l.late) >= 10 {
            match trend {
                Some(t) if t >= 0.2 => reasons.push("rising"),
                Some(t) if t <= -0.2 => reasons.push("falling"),
                _ => {}
            }
        }
        if kind == "under_translated" {
            reasons.push("under_translated");
        }
        let confidence = if fb_visits < MIN_VISITS || (tracked && nv < MIN_NATIVE) {
            "insufficient"
        } else if fb_visits >= SOLID_VISITS
            && (!tracked
                || (nv >= SOLID_VISITS && nc >= SOLID_CONVERSIONS && fb_conv >= SOLID_CONVERSIONS))
        {
            "solid"
        } else {
            "indicative"
        };
        out.push(Recommendation {
            locale,
            language: language.clone(),
            kind,
            visits: fb_visits,
            monthly_visits: monthly,
            share: fb_visits as f64 / total.max(1) as f64,
            trend,
            served_now_share: served_now.map_or(0.0, |s| *s.1 as f64 / l.visits.max(1) as f64),
            served_now: served_now.map(|s| s.0.clone()),
            coverage,
            fallback,
            native,
            gain_monthly: g,
            cost_usd: 0.0,
            keys_to_translate: 0,
            cost_per_conversion: None,
            gain_locked: false,
            confidence,
            reasons,
            summary: String::new(),
        });
    }
    // ponytail: dry-run cost (pure CPU, scales with the catalog) for the 10 biggest demands only.
    out.sort_by_key(|r| std::cmp::Reverse(r.visits));
    out.truncate(10);
    for r in &mut out {
        (r.cost_usd, r.keys_to_translate) = cost(&r.locale, r.kind);
        r.cost_per_conversion = r
            .gain_monthly
            .filter(|g| g.mid > 0.0)
            .map(|g| r.cost_usd / g.mid);
        r.summary = summary(r);
    }
    let score = |r: &Recommendation| r.gain_monthly.map_or(0.0, |g| g.mid.max(0.0));
    out.sort_by(|a, b| {
        score(b)
            .total_cmp(&score(a))
            .then(b.visits.cmp(&a.visits))
            .then(a.cost_usd.total_cmp(&b.cost_usd))
    });
    (total, tracked, out)
}

/// `pt` → `pt-BR`, `zh` → `zh-CN` (first entry of the public market table), else the language itself.
fn default_locale(language: &str) -> String {
    glot_core::markets::MARKETS
        .iter()
        .find(|m| lang(m.locale) == language)
        .map_or_else(|| language.to_string(), |m| m.locale.to_string())
}

fn summary(r: &Recommendation) -> String {
    let what = if r.kind == "missing" {
        format!("Add {}", r.locale)
    } else {
        format!(
            "Finish {} ({:.0} % translated)",
            r.locale,
            r.coverage * 100.0
        )
    };
    let gain = match r.gain_monthly {
        None if r.gain_locked => "expected gain included in Growth".to_string(),
        None => "conversions not tracked, ranked on demand and cost".to_string(),
        Some(g) => format!(
            "{:+.1} conversions/month expected (90 % interval {:.1} to {:.1})",
            g.mid, g.low, g.high
        ),
    };
    format!(
        "{what}: {:.0} % of your visitors ask for it, ~{} to translate, {gain} — {}",
        r.share * 100.0,
        glot_core::markets::usd(r.cost_usd),
        r.confidence
    )
}

/// Everything the endpoint returns, for reuse (e.g. launch measurement): same auth as the caller's.
pub async fn for_project(
    state: &AppState,
    p: &ProjectRow,
    days: i64,
    goal: &str,
) -> ApiResult<Recommendations> {
    let days = days.clamp(1, 365);
    let half = (days / 2) as i32;
    let raw: Vec<(String, String, bool, bool, i64)> = sqlx::query_as(
        "SELECT preferred, served, event <> '$view', day > (now() AT TIME ZONE 'utc')::date - $3::int,
                sum(n)::bigint
         FROM insight_daily
         WHERE project_id = $1 AND day > (now() AT TIME ZONE 'utc')::date - $2::int
           AND (event = '$view' OR event = $4)
         GROUP BY 1, 2, 3, 4",
    )
    .bind(p.id)
    .bind(days as i32)
    .bind(half)
    .bind(goal)
    .fetch_all(&state.db)
    .await?;
    let rows: Vec<Row> = raw
        .into_iter()
        .map(|(preferred, served, conversion, late, n)| Row {
            preferred,
            served,
            conversion,
            late,
            n,
        })
        .collect();
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT key, source FROM keys WHERE project_id = $1 ORDER BY key")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let translated: Vec<(String, i64)> = sqlx::query_as(
        "SELECT t.locale, count(*) FROM translations t JOIN keys k ON k.id = t.key_id
         WHERE k.project_id = $1 AND t.status <> 'stale' GROUP BY 1",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let offered: Vec<Offered> = p
        .target_locales
        .iter()
        .map(|l| Offered {
            locale: l.clone(),
            translated: translated.iter().find(|t| t.0 == *l).map_or(0, |t| t.1),
        })
        .collect();
    // Keys still to translate per under-translated locale (missing locales need the whole catalog).
    let mut todo: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for o in offered
        .iter()
        .filter(|o| (o.translated as f64) < UNDER_TRANSLATED * keys.len() as f64)
    {
        let k: Vec<(String, String)> = sqlx::query_as(
            "SELECT k.key, k.source FROM keys k WHERE k.project_id = $1 AND NOT EXISTS (
               SELECT 1 FROM translations t WHERE t.key_id = k.id AND t.locale = $2 AND t.status <> 'stale')
             ORDER BY k.key",
        )
        .bind(p.id)
        .bind(&o.locale)
        .fetch_all(&state.db)
        .await?;
        todo.insert(o.locale.clone(), k);
    }
    let (total_visits, conversions_tracked, recommendations) = rank(
        days,
        &p.source_locale,
        &offered,
        keys.len() as i64,
        &rows,
        |locale, kind| {
            let k = match kind {
                "under_translated" => todo.get(locale).map_or(&keys[..0], |v| &v[..]),
                _ => &keys[..],
            };
            (estimate_cost(p, k, locale), k.len() as i64)
        },
    );
    Ok(Recommendations {
        days,
        goal: goal.to_string(),
        model: p.llm.0.model.clone(),
        public_key: crate::insights::public_key(state, p.id).await?,
        total_visits,
        conversions_tracked,
        keys: keys.len() as i64,
        recommendations,
    })
}

/// Below Growth: demand, current experience, cost and confidence stay; the expected gain (and the reasons
/// that would reveal it) goes.
pub fn lock_gain(r: &mut Recommendation) {
    r.gain_monthly = None;
    r.cost_per_conversion = None;
    r.gain_locked = true;
    r.reasons
        .retain(|x| !matches!(*x, "no_gain" | "gain_uncertain"));
    r.summary = summary(r);
}

#[derive(Deserialize)]
pub struct Query {
    days: Option<i64>,
    goal: Option<String>,
}

async fn handler(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    AppPath(slug): AppPath<String>,
    AppQuery(q): AppQuery<Query>,
) -> ApiResult<Json<Recommendations>> {
    let p = projects::load(&state, user.id, &slug, Role::Translator).await?;
    let goal = match q.goal {
        Some(g) => {
            crate::insights::check_event(&g)?;
            g
        }
        None => "signup".into(),
    };
    let mut r = for_project(&state, &p, q.days.unwrap_or(30), &goal).await?;
    if !crate::billing::has_growth(&state, p.id).await? {
        r.recommendations.iter_mut().for_each(lock_gain);
    }
    Ok(Json(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pref: &str, served: &str, conversion: bool, late: bool, n: i64) -> Row {
        Row {
            preferred: pref.into(),
            served: served.into(),
            conversion,
            late,
            n,
        }
    }

    #[test]
    fn posterior_and_gain() {
        // Uniform prior: 0/0 → 0.5 with a wide interval; 50/1000 → ~5 %.
        let r = Rate::new(0, 0);
        assert!((r.rate - 0.5).abs() < 1e-9 && r.ci_low < 0.1 && r.ci_high > 0.9);
        let r = Rate::new(1000, 50);
        assert!((r.rate - 51.0 / 1002.0).abs() < 1e-9);
        assert!(r.ci_low > 0.03 && r.ci_high < 0.07);
        // 1,000 visits/month at 2 % vs 5 % native: ~+30/month, interval excludes 0.
        let g = gain(1000.0, Rate::new(2000, 100), Rate::new(1000, 20));
        assert!((g.mid - 29.0).abs() < 2.0, "{g:?}");
        assert!(g.low > 0.0 && g.high > g.mid);
        // Thin data: the interval spans 0.
        let g = gain(30.0, Rate::new(40, 2), Rate::new(10, 0));
        assert!(g.low < 0.0 && g.high > 0.0, "{g:?}");
    }

    #[test]
    fn ranks_by_expected_gain_and_labels_confidence() {
        let offered = [Offered {
            locale: "de".into(),
            translated: 10,
        }];
        let rows = vec![
            row("en", "en", false, false, 1000),
            row("en", "en", false, true, 1000),
            row("en", "en", true, true, 100),
            row("de", "de", false, true, 400),
            row("de", "de", true, true, 24),
            // es: big, converts badly on the English fallback
            row("es", "en", false, false, 200),
            row("es", "en", false, true, 400),
            row("es", "en", true, true, 12),
            // it: tiny
            row("it", "en", false, true, 5),
            row("und", "en", false, true, 50),
        ];
        let (total, tracked, recs) = rank(30, "en", &offered, 10, &rows, |_, _| (2.0, 10));
        assert_eq!(total, 3055);
        assert!(tracked);
        assert_eq!(recs.len(), 2, "en is the source, de fully translated");
        let es = &recs[0];
        assert_eq!((es.locale.as_str(), es.kind), ("es", "missing"));
        assert_eq!(es.served_now.as_deref(), Some("en"));
        assert_eq!(es.visits, 600);
        // native pool excludes the source when it has enough (de: 400 visits, 24 conv = 6 %), es at 2 %.
        assert_eq!(es.native.visits, 400);
        let g = es.gain_monthly.unwrap();
        assert!(g.mid > 15.0 && g.low > 0.0, "{g:?}");
        assert_eq!(es.confidence, "solid");
        assert!(es.reasons.contains(&"rising"), "{:?}", es.reasons);
        assert!(es.cost_per_conversion.unwrap() < 1.0);
        assert!(
            es.summary.starts_with("Add es: 20 % of your visitors"),
            "{}",
            es.summary
        );
        let it = &recs[1];
        assert_eq!(it.confidence, "insufficient");
        assert!(it.reasons.contains(&"thin_demand"));
    }

    #[test]
    fn no_conversions_ranks_on_demand_and_flags_under_translated() {
        let offered = [Offered {
            locale: "fr".into(),
            translated: 3,
        }];
        let rows = vec![
            row("en", "en", false, true, 500),
            row("fr", "fr", false, true, 100),
            row("ja", "en", false, true, 300),
        ];
        let (_, tracked, recs) = rank(30, "en", &offered, 10, &rows, |l, k| {
            (if k == "missing" { 3.0 } else { 0.5 }, l.len() as i64)
        });
        assert!(!tracked);
        assert_eq!(recs[0].locale, "ja");
        assert!(recs[0].gain_monthly.is_none() && recs[0].reasons.contains(&"no_conversions"));
        assert_eq!(recs[0].confidence, "solid", "300 visits of demand");
        assert_eq!(
            (recs[1].locale.as_str(), recs[1].kind),
            ("fr", "under_translated")
        );
        assert!((recs[1].coverage - 0.3).abs() < 1e-9);
        assert!(recs[1].summary.contains("conversions not tracked"));
    }

    #[test]
    fn locked_gain_keeps_demand_cost_and_confidence() {
        let rows = vec![
            row("en", "en", false, true, 1000),
            row("en", "en", true, true, 50),
            row("de", "de", false, true, 400),
            row("de", "de", true, true, 4),
            row("es", "en", false, true, 30),
            row("es", "en", true, true, 1),
        ];
        let (_, _, mut recs) = rank(30, "en", &[], 10, &rows, |_, _| (2.0, 10));
        let mut r = recs.remove(0);
        let (visits, cost, confidence) = (r.visits, r.cost_usd, r.confidence);
        lock_gain(&mut r);
        assert!(r.gain_locked && r.gain_monthly.is_none() && r.cost_per_conversion.is_none());
        assert_eq!(
            (r.visits, r.cost_usd, r.confidence),
            (visits, cost, confidence)
        );
        assert!(
            !r.reasons
                .iter()
                .any(|x| *x == "no_gain" || *x == "gain_uncertain")
        );
        assert!(r.summary.contains("included in Growth"), "{}", r.summary);
    }
}
