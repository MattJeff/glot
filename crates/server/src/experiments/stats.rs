//! Statistics of segmented copy experiments — pure (no DB, no clock): counters in, `Analysis` out.
//!
//! **Cells.** An experiment has ≤ `MAX_CELLS` targeted segments + `*`, each with the same arms (arm 0 =
//! official copy). Every analysis computes the *global* posterior (all cells summed, flat prior) and one posterior
//! per cell.
//!
//! **Partial pooling (low traffic).** A cell's arm j gets the prior Beta(p·k, (1−p)·k), where p is arm j's rate in
//! the *other* cells (leave-one-out, so the cell's own data is never counted twice) and k (visitors-equivalent) is
//! estimated by empirical Bayes: method of moments on the between-cell spread of arm j's rates,
//! τ² = Σ nᵢ(rᵢ−p̄)²/N − p̄(1−p̄)·m/N (observed spread minus binomial noise), k = p̄(1−p̄)/τ² − 1, clamped to
//! [2, 100]. Cells that look alike share up to 100 visitors of strength; cells that clearly differ get almost none.
//! A cell with 50 visitors is thus pulled toward its arm's rate elsewhere instead of swinging on 2 conversions.
//! Revenue: the order value is shrunk the same way with a fixed k = 10 orders.
//!
//! **Revenue per visitor** (hurdle model, from the stored Σvalue and Σvalue²): conversion ~ Beta posterior, order
//! value ~ Normal(mean, sd²/orders) (CLT on order values, not on the zero-inflated per-visitor values), revenue per
//! visitor = rate × order value, sampled jointly. A log-normal model would need Σlog(value), not stored.
//!
//! **Allocation (batch bandit).** Weights are recomputed per batch, not per hit: top-two Thompson probabilities
//! estimated by Monte-Carlo (P(leader)/2 + P(challenger)/2) — the batch version of `variants::allocate`. `fixed`:
//! the configured weights (uniform by default), no floor; `bandit`: global weights in every cell; `segmented`: per-cell weights once every arm of the cell has
//! `min_n` visitors (global weights before). Every arm keeps ≥ `FLOOR` (10 %) of its cell's traffic.
//!
//! **Stopping rule (alpha spending).** The 5 % error budget is split across arms × tests (cells, + the global test
//! when there are several cells) × looks (½, ¼, … as in `variants::decide`): an arm wins when
//! P(best) > 1 − 5 %/(arms·tests·2^(look+1)). A look is only spent when a decision is possible (≥ `MIN_DAYS` days,
//! traffic ≥ `next_look`, no SRM), each cell additionally needing `min_n` visitors per arm.
//!
//! **Global vs segment winner.** `everywhere`: the global winner (or a cell winner whose lift is not specific to
//! its segment). `segments`: a cell winner w with P(lift_cell(w) > lift_global(w)) > 95 % — the effect really is
//! larger in that segment. Cell and global draws are independent (they share data), which overstates the
//! variance of the difference: conservative.
//!
//! **SRM.** χ² of observed exposures vs expected (Σ visitors × weight in force, so it also holds for bandit
//! weights that change between batches) per cell, and the holdout split; p < 0.001 (Bonferroni across those
//! tests) → `paused`. The χ² is cumulative and re-tested at every analysis: callers test it once a day (pass
//! `expected: None` otherwise) to keep repeated-testing false alarms low (A/A simulation below).
//!
//! **Exploratory slices** (country, device, source…): never decisional; p = 2·min(P, 1−P) of P(beats control),
//! Benjamini-Hochberg adjusted across all slices × arms.
use rand::Rng;
use serde::{Deserialize, Serialize};

pub use super::model::{Allocation, MAX_CELLS, Metric};
use crate::variants::{WIN_PROB, gamma, next_look, normal};

/// Minimum traffic share of every arm in a cell (exploration floor).
pub const FLOOR: f64 = 0.10;
/// No decision before a full week (weekly seasonality).
pub const MIN_DAYS: f64 = 7.0;
/// SRM alarm level: decisions frozen below it.
pub const SRM_P: f64 = 0.001;
/// Strongest partial-pooling prior, in visitors.
pub const POOL_K_MAX: f64 = 100.0;
const AOV_K: f64 = 10.0;

/// Counters of one arm in one cell.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Arm {
    pub views: i64,
    pub conversions: i64,
    /// Σ value and Σ value² of the goal events (revenue metric).
    #[serde(default)]
    pub revenue: f64,
    #[serde(default)]
    pub revenue_sq: f64,
}

impl std::ops::Add for Arm {
    type Output = Arm;
    fn add(self, o: Arm) -> Arm {
        Arm {
            views: self.views + o.views,
            conversions: self.conversions + o.conversions,
            revenue: self.revenue + o.revenue,
            revenue_sq: self.revenue_sq + o.revenue_sq,
        }
    }
}

impl std::ops::Sub for Arm {
    type Output = Arm;
    fn sub(self, o: Arm) -> Arm {
        Arm {
            views: self.views - o.views,
            conversions: self.conversions - o.conversions,
            revenue: self.revenue - o.revenue,
            revenue_sq: self.revenue_sq - o.revenue_sq,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    pub name: String,
    pub arms: Vec<Arm>,
    /// Expected exposures per arm (Σ visitors × weight in force); `None` = no SRM check for this cell.
    #[serde(default)]
    pub expected: Option<Vec<f64>>,
}

/// Holdout split: sessions held out vs sessions in experiments, and the configured holdout share (0..1).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Holdout {
    pub held: i64,
    pub exposed: i64,
    pub pct: f64,
}

#[derive(Clone, Debug)]
pub struct Input<'a> {
    pub metric: Metric,
    pub allocation: Allocation,
    /// `fixed` allocation weights (normalized here); `None` = uniform.
    pub fixed: Option<&'a [f64]>,
    pub min_n: i64,
    /// Days since start.
    pub days: f64,
    /// Decision looks already spent.
    pub look: i32,
    pub cells: &'a [Cell],
    pub holdout: Option<Holdout>,
    /// Monte-Carlo draws (4000 in production).
    pub draws: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ArmResult {
    /// Posterior mean of the metric (rate, or revenue per visitor).
    pub mean: f64,
    pub p_best: f64,
    pub p_beats_control: f64,
    /// Relative lift over arm 0: posterior mean and 95 % credible interval.
    pub lift: f64,
    pub lift_lo: f64,
    pub lift_hi: f64,
    /// Traffic share to serve until the next batch.
    pub weight: f64,
    /// Visitors who actually saw the tested block (`$seen`), and their mean bucketed visible time (filled by
    /// `rebalance`, absent when nothing was seen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CellResult {
    pub name: String,
    pub views: i64,
    pub arms: Vec<ArmResult>,
    /// Arm best above the look's threshold (decision looks only).
    pub winner: Option<usize>,
    /// P(lift_cell(winner) > lift_global(winner)).
    pub p_specific: Option<f64>,
    pub srm_p: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Analysis {
    /// Arm numbers, in the order of every `arms` list below (set by the caller; default 0..k).
    #[serde(default)]
    pub arm_ids: Vec<i32>,
    /// Computed on demand for the results page (no look spent, decisions = the worker's).
    #[serde(default)]
    pub live: bool,
    pub look: i32,
    /// This analysis spent a look: the caller increments its look counter.
    pub decision_look: bool,
    pub threshold: f64,
    pub global: CellResult,
    pub cells: Vec<CellResult>,
    pub holdout_srm_p: Option<f64>,
    /// SRM detected: stop serving variants, alert, no decision.
    pub paused: bool,
    /// Recommendation "promote everywhere" (arm index).
    pub everywhere: Option<usize>,
    /// Recommendation "promote for this segment" (cell name, arm index).
    pub segments: Vec<(String, usize)>,
}

impl Analysis {
    /// Re-indexes every arm list and arm index by arm number (`arm_ids`): gaps (unapproved/deleted arms) get an
    /// empty `ArmResult` (weight 0). This is the stored / REST shape (contract: indices = arm numbers).
    pub fn by_arm_number(mut self) -> Analysis {
        let ids = self.arm_ids.clone();
        let width = ids.iter().map(|i| *i as usize + 1).max().unwrap_or(0);
        let num = |j: usize| ids[j] as usize;
        for c in std::iter::once(&mut self.global).chain(self.cells.iter_mut()) {
            let mut arms = vec![ArmResult::default(); width];
            for (j, r) in std::mem::take(&mut c.arms).into_iter().enumerate() {
                arms[num(j)] = r;
            }
            c.arms = arms;
            c.winner = c.winner.map(num);
        }
        self.everywhere = self.everywhere.map(num);
        self.segments.iter_mut().for_each(|s| s.1 = num(s.1));
        self
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Invalid {
    NoCells,
    TooManyCells,
    ArmsMismatch,
}

// ---------- posteriors ----------

fn beta(a: f64, b: f64, rng: &mut impl Rng) -> f64 {
    if a.min(b) > 50.0 {
        let (n, m) = (a + b, a / (a + b));
        return (m + normal(rng) * (m * (1.0 - m) / (n + 1.0)).sqrt()).clamp(0.0, 1.0);
    }
    let x = gamma(a, rng);
    x / (x + gamma(b, rng))
}

#[derive(Clone, Copy, Debug)]
struct Post {
    a: f64,
    b: f64,
    /// Order value (mean, sd of the mean); `None` for the conversion metric.
    aov: Option<(f64, f64)>,
}

impl Post {
    fn mean(&self) -> f64 {
        self.a / (self.a + self.b) * self.aov.map_or(1.0, |a| a.0.max(0.0))
    }
    fn draw(&self, rng: &mut impl Rng) -> f64 {
        let p = beta(self.a, self.b, rng);
        match self.aov {
            None => p,
            Some((m, s)) => p * (m + s * normal(rng)).max(0.0),
        }
    }
}

/// Empirical-Bayes prior strength for one arm across cells (see module doc).
fn pool_k(cells: &[Arm]) -> f64 {
    let n: f64 = cells.iter().map(|a| a.views.max(0) as f64).sum();
    let c: f64 = cells.iter().map(|a| a.conversions.max(0) as f64).sum();
    let m = cells.iter().filter(|a| a.views > 0).count() as f64;
    if m < 2.0 || n <= 0.0 {
        return POOL_K_MAX;
    }
    let p = c / n;
    let pq = p * (1.0 - p);
    let s2: f64 = cells
        .iter()
        .filter(|a| a.views > 0)
        .map(|a| a.views as f64 * (a.conversions as f64 / a.views as f64 - p).powi(2))
        .sum::<f64>()
        / n;
    let tau2 = s2 - pq * m / n;
    if pq <= 0.0 || tau2 <= 0.0 {
        return POOL_K_MAX;
    }
    (pq / tau2 - 1.0).clamp(2.0, POOL_K_MAX)
}

/// Posteriors per [cell][arm]. `pool`: shrink each cell toward the other cells (needs ≥ 2 cells).
fn posteriors(metric: Metric, cells: &[&[Arm]], pool: bool) -> Vec<Vec<Post>> {
    let arms = cells[0].len();
    let pool = pool && cells.len() > 1;
    let mut out = vec![Vec::with_capacity(arms); cells.len()];
    // Order-value spread pooled over every arm and cell: a handful of skewed orders per arm would understate it.
    let all = cells
        .iter()
        .flat_map(|c| c.iter())
        .fold(Arm::default(), |s, a| s + *a);
    let ac = all.conversions.max(0) as f64;
    let var = if ac >= 2.0 {
        (all.revenue_sq / ac - (all.revenue / ac).powi(2)).max(1e-9)
    } else {
        (all.revenue / ac.max(1.0)).powi(2).max(1.0)
    };
    for j in 0..arms {
        let col: Vec<Arm> = cells.iter().map(|c| c[j]).collect();
        let tot = col.iter().fold(Arm::default(), |s, a| s + *a);
        let k = pool_k(&col);
        let tc = tot.conversions.max(0) as f64;
        for (ci, a) in col.iter().enumerate() {
            let (v, c) = (
                a.views.max(0) as f64,
                a.conversions.clamp(0, a.views.max(0)) as f64,
            );
            let o = tot - *a;
            let (ov, oc) = (o.views.max(0) as f64, o.conversions.max(0) as f64);
            let (pa, pb) = if pool {
                let p = (oc + 1.0) / (ov + 2.0);
                (p * k, (1.0 - p) * k)
            } else {
                (1.0, 1.0)
            };
            let aov = (metric == Metric::Revenue).then(|| {
                if tc == 0.0 {
                    return (1.0, 0.0); // no order anywhere yet: compare rates only
                }
                let (kk, g) = if pool && oc > 0.0 {
                    (AOV_K, o.revenue / oc)
                } else {
                    (0.0, 0.0)
                };
                let n = kk + c;
                if n == 0.0 {
                    (tot.revenue / tc, (var / AOV_K).sqrt())
                } else {
                    ((kk * g + a.revenue) / n, (var / n).sqrt())
                }
            });
            out[ci].push(Post {
                a: pa + c,
                b: pb + v - c,
                aov,
            });
        }
    }
    out
}

struct Mc {
    p_best: Vec<f64>,
    beats: Vec<f64>,
    lift: Vec<Vec<f64>>,
    top_two: Vec<f64>,
}

fn monte_carlo(posts: &[Post], draws: usize, rng: &mut impl Rng) -> Mc {
    let k = posts.len();
    let (mut best, mut beats, mut top_two) = (vec![0.0; k], vec![0.0; k], vec![0.0; k]);
    let mut lift = vec![Vec::with_capacity(draws); k];
    let mut x = vec![0.0; k];
    for _ in 0..draws {
        for (i, p) in posts.iter().enumerate() {
            x[i] = p.draw(rng);
        }
        let leader = argmax(&x, usize::MAX);
        best[leader] += 1.0;
        for i in 0..k {
            beats[i] += (x[i] > x[0]) as u8 as f64;
            lift[i].push(if x[0] > 0.0 { x[i] / x[0] - 1.0 } else { 0.0 });
        }
        // Top-two: half to the leader, half to the best fresh draw among the others.
        for (i, p) in posts.iter().enumerate() {
            x[i] = p.draw(rng);
        }
        top_two[leader] += 0.5;
        top_two[argmax(&x, leader)] += 0.5;
    }
    let n = draws as f64;
    for v in [&mut best, &mut beats, &mut top_two] {
        v.iter_mut().for_each(|x| *x /= n);
    }
    Mc {
        p_best: best,
        beats,
        lift,
        top_two,
    }
}

/// Index of the largest value, skipping `skip`; ties go to the lowest index.
fn argmax(x: &[f64], skip: usize) -> usize {
    (0..x.len())
        .filter(|i| *i != skip)
        .fold(None, |b: Option<usize>, i| match b {
            Some(b) if x[b] >= x[i] => Some(b),
            _ => Some(i),
        })
        .unwrap_or(0)
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// Raises every weight to at least `FLOOR` (or 1/arms if smaller), keeping the others' proportions.
pub fn floor_weights(w: &[f64]) -> Vec<f64> {
    let f = FLOOR.min(1.0 / w.len() as f64);
    let mut fixed = vec![false; w.len()];
    loop {
        let free: f64 = (0..w.len()).filter(|i| !fixed[*i]).map(|i| w[i]).sum();
        let budget = 1.0 - f * fixed.iter().filter(|x| **x).count() as f64;
        if free <= 0.0 {
            return vec![1.0 / w.len() as f64; w.len()];
        }
        let low: Vec<usize> = (0..w.len())
            .filter(|i| !fixed[*i] && w[*i] / free * budget < f)
            .collect();
        if low.is_empty() {
            return (0..w.len())
                .map(|i| if fixed[i] { f } else { w[i] / free * budget })
                .collect();
        }
        low.into_iter().for_each(|i| fixed[i] = true);
    }
}

// ---------- SRM, BH ----------

fn ln_gamma(x: f64) -> f64 {
    // Lanczos (g = 7, n = 9).
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let t = x + 7.5;
    let s = C[1..]
        .iter()
        .enumerate()
        .fold(C[0], |s, (i, c)| s + c / (x + i as f64 + 1.0));
    0.5 * (std::f64::consts::TAU).ln() + (x + 0.5) * t.ln() - t + s.ln()
}

/// Regularized upper incomplete gamma Q(a, x) (series below a+1, Lentz continued fraction above).
fn gamma_q(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    let lg = a * x.ln() - x - ln_gamma(a);
    if x < a + 1.0 {
        let (mut sum, mut del, mut ap) = (1.0 / a, 1.0 / a, a);
        for _ in 0..500 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        return (1.0 - sum * lg.exp()).clamp(0.0, 1.0);
    }
    let tiny = 1e-300;
    let mut b = x + 1.0 - a;
    let (mut c, mut d) = (1.0 / tiny, 1.0 / b);
    let mut h = d;
    for i in 1..500 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        d = if d.abs() < tiny { tiny } else { d };
        c = b + an / c;
        c = if c.abs() < tiny { tiny } else { c };
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    (lg.exp() * h).clamp(0.0, 1.0)
}

/// Sample-ratio-mismatch p-value: χ² goodness of fit of `observed` counts to `expected` (any scale).
pub fn srm_p(observed: &[i64], expected: &[f64]) -> f64 {
    let n: f64 = observed.iter().map(|o| *o as f64).sum();
    let e: f64 = expected.iter().sum();
    if n <= 0.0 || e <= 0.0 || observed.len() != expected.len() {
        return 1.0;
    }
    let mut chi = 0.0;
    let mut df = -1.0;
    for (o, x) in observed.iter().zip(expected) {
        let ei = x / e * n;
        if ei <= 0.0 {
            if *o > 0 {
                return 0.0; // traffic on an arm that should get none
            }
            continue;
        }
        chi += (*o as f64 - ei).powi(2) / ei;
        df += 1.0;
    }
    if df < 1.0 {
        return 1.0;
    }
    gamma_q(df / 2.0, chi / 2.0)
}

/// Benjamini-Hochberg adjusted p-values (q-values), same order as `p`. Significant at FDR q ⇔ q-value ≤ q.
pub fn bh(p: &[f64]) -> Vec<f64> {
    let m = p.len();
    let mut idx: Vec<usize> = (0..m).collect();
    idx.sort_by(|a, b| p[*a].total_cmp(&p[*b]));
    let mut q = vec![1.0; m];
    let mut min: f64 = 1.0;
    for r in (0..m).rev() {
        min = min.min(p[idx[r]] * m as f64 / (r + 1) as f64);
        q[idx[r]] = min.min(1.0);
    }
    q
}

/// Exploratory slices (never decisional): per slice, per arm ≥ 1, (p, BH q-value) of "arm differs from arm 0",
/// p = 2·min(P(beats control), 1 − P(beats control)), flat priors, BH across all slices × arms.
pub fn exploratory(
    metric: Metric,
    slices: &[Vec<Arm>],
    draws: usize,
    rng: &mut impl Rng,
) -> Vec<Vec<(f64, f64)>> {
    let ps: Vec<Vec<f64>> = slices
        .iter()
        .map(|arms| {
            if arms.len() < 2 {
                return vec![];
            }
            let posts = &posteriors(metric, &[arms.as_slice()], false)[0];
            let mc = monte_carlo(posts, draws, rng);
            // Floor at 1/draws: a Monte-Carlo zero is not a true zero.
            (1..arms.len())
                .map(|i| (2.0 * mc.beats[i].min(1.0 - mc.beats[i])).max(1.0 / draws as f64))
                .collect()
        })
        .collect();
    let q = bh(&ps.concat());
    let mut k = 0;
    ps.iter()
        .map(|row| {
            row.iter()
                .map(|p| {
                    k += 1;
                    (*p, q[k - 1])
                })
                .collect()
        })
        .collect()
}

// ---------- analysis ----------

fn result(name: &str, arms: &[Arm], posts: &[Post], mc: Mc, weights: &[f64]) -> CellResult {
    CellResult {
        name: name.to_string(),
        views: arms.iter().map(|a| a.views).sum(),
        arms: (0..arms.len())
            .map(|i| {
                let mut l = mc.lift[i].clone();
                l.sort_by(f64::total_cmp);
                ArmResult {
                    mean: posts[i].mean(),
                    p_best: mc.p_best[i],
                    p_beats_control: mc.beats[i],
                    lift: l.iter().sum::<f64>() / l.len().max(1) as f64,
                    lift_lo: quantile(&l, 0.025),
                    lift_hi: quantile(&l, 0.975),
                    weight: weights[i],
                    ..Default::default()
                }
            })
            .collect(),
        winner: None,
        p_specific: None,
        srm_p: None,
    }
}

/// One analysis point: posteriors, next batch weights, SRM, decisions and recommendations.
pub fn analyze(input: &Input, rng: &mut impl Rng) -> Result<Analysis, Invalid> {
    let cells = input.cells;
    if cells.is_empty() {
        return Err(Invalid::NoCells);
    }
    if cells.len() > MAX_CELLS + 1 {
        return Err(Invalid::TooManyCells);
    }
    let k = cells[0].arms.len();
    if k < 2 || cells.iter().any(|c| c.arms.len() != k) {
        return Err(Invalid::ArmsMismatch);
    }
    let young = |arms: &[Arm]| arms.iter().any(|a| a.views < input.min_n);
    let uniform = vec![1.0 / k as f64; k];
    let fixed = match input.fixed {
        Some(w) if w.len() == k && w.iter().all(|x| *x >= 0.0) && w.iter().sum::<f64>() > 0.0 => {
            let s: f64 = w.iter().sum();
            w.iter().map(|x| x / s).collect()
        }
        Some(_) => return Err(Invalid::ArmsMismatch),
        None => uniform.clone(),
    };

    // Global: all cells summed, flat prior.
    let total: Vec<Arm> = (0..k)
        .map(|j| cells.iter().fold(Arm::default(), |s, c| s + c.arms[j]))
        .collect();
    let gpost = &posteriors(input.metric, &[total.as_slice()], false)[0];
    let gmc = monte_carlo(gpost, input.draws, rng);
    let gweights = match input.allocation {
        Allocation::Fixed => fixed,
        _ if young(&total) => uniform.clone(),
        _ => floor_weights(&gmc.top_two),
    };
    let mut global = result("*", &total, gpost, gmc, &gweights);

    // Cells: partial pooling toward the other cells.
    let refs: Vec<&[Arm]> = cells.iter().map(|c| c.arms.as_slice()).collect();
    let posts = posteriors(input.metric, &refs, true);
    let mut out: Vec<CellResult> = cells
        .iter()
        .zip(&posts)
        .map(|(c, p)| {
            let mc = monte_carlo(p, input.draws, rng);
            let w = match input.allocation {
                Allocation::Segmented if !young(&c.arms) => floor_weights(&mc.top_two),
                _ => gweights.clone(),
            };
            let mut r = result(&c.name, &c.arms, p, mc, &w);
            r.srm_p = c
                .expected
                .as_ref()
                .map(|e| srm_p(&c.arms.iter().map(|a| a.views).collect::<Vec<_>>(), e));
            r
        })
        .collect();
    let holdout_srm_p = input
        .holdout
        .map(|h| srm_p(&[h.held, h.exposed], &[h.pct, 1.0 - h.pct]));
    // Bonferroni across the SRM tests of this analysis (cells + holdout split).
    let srm: Vec<f64> = out
        .iter()
        .filter_map(|c| c.srm_p)
        .chain(holdout_srm_p)
        .collect();
    let paused = srm.iter().any(|p| *p < SRM_P / srm.len() as f64);

    // Stopping rule.
    let tests = cells.len() + (cells.len() > 1) as usize;
    let threshold =
        1.0 - (1.0 - WIN_PROB) / ((k * tests) as f64 * 2f64.powi(input.look.clamp(0, 30) + 1));
    let views: i64 = total.iter().map(|a| a.views).sum();
    let decision_look = !paused
        && input.days >= MIN_DAYS
        && views >= next_look(k * cells.len(), input.min_n, input.look);
    let win = |r: &CellResult, arms: &[Arm]| {
        (decision_look && !young(arms))
            .then(|| r.arms.iter().position(|a| a.p_best > threshold))
            .flatten()
    };
    global.winner = win(&global, &total);
    let mut everywhere = global.winner;
    let mut segments = vec![];
    for (i, c) in cells.iter().enumerate() {
        let Some(w) = win(&out[i], &c.arms) else {
            continue;
        };
        out[i].winner = Some(w);
        if cells.len() == 1 {
            continue; // the cell is the global test
        }
        // ponytail: control winning a cell (w = 0) is never segment-specific (its lift is 0 by definition).
        let n = input.draws;
        let specific = (0..n)
            .filter(|_| {
                let lc = posts[i][w].draw(rng) / posts[i][0].draw(rng).max(1e-12);
                let lg = gpost[w].draw(rng) / gpost[0].draw(rng).max(1e-12);
                lc > lg
            })
            .count() as f64
            / n as f64;
        out[i].p_specific = Some(specific);
        if w != 0 && specific > WIN_PROB {
            segments.push((c.name.clone(), w));
        } else if everywhere.is_none() {
            everywhere = Some(w);
        }
    }
    Ok(Analysis {
        arm_ids: (0..k as i32).collect(),
        live: false,
        look: input.look,
        decision_look,
        threshold,
        global,
        cells: out,
        holdout_srm_p,
        paused,
        everywhere,
        segments,
    })
}

#[cfg(test)]
#[path = "stats/tests.rs"]
mod tests;
