//! How long until a result: visitors per arm to detect a relative lift (α = 0.05 two-sided, power 0.8, the same
//! normal approximation as `insights::required_per_arm`), and the smallest lift a traffic budget can detect.
use super::stats::MIN_DAYS;

/// Relative lifts shown to the user (+10 %, +20 %, +50 %).
pub const LIFTS: [f64; 3] = [0.10, 0.20, 0.50];
/// Past this, the dashboard warns and proposes alternatives.
pub const WARN_DAYS: f64 = 56.0;
const Z: f64 = 1.96 + 0.8416;

/// Visitors per arm to detect `baseline × (1 + lift)`; None when the rate is 0 or the target ≥ 100 %.
pub fn per_arm(baseline: f64, lift: f64) -> Option<i64> {
    let q = baseline * (1.0 + lift);
    if baseline <= 0.0 || lift <= 0.0 || q >= 1.0 {
        return None;
    }
    Some(
        (Z * Z * (baseline * (1.0 - baseline) + q * (1.0 - q)) / (q - baseline).powi(2)).ceil()
            as i64,
    )
}

/// Days for `arms` arms (official included) of `n` visitors each, on `daily` visitors in the test.
pub fn days(n: i64, arms: usize, daily: f64) -> Option<f64> {
    (daily > 0.0).then(|| (n as f64 * arms as f64 / daily).max(MIN_DAYS))
}

/// Smallest relative lift detectable within `budget` days (bisection: n decreases with the lift).
/// None when even a ×10 rate (or the 100 % ceiling) is out of reach.
pub fn detectable_lift(
    baseline: f64,
    arms: usize,
    daily: f64,
    min_n: i64,
    budget: f64,
) -> Option<f64> {
    let fits = |lift: f64| {
        per_arm(baseline, lift)
            .and_then(|n| days(n.max(min_n), arms, daily))
            .is_some_and(|d| d <= budget)
    };
    let mut hi = (1.0 / baseline.max(1e-9) - 1.0).min(9.0) * 0.999;
    if !fits(hi) {
        return None;
    }
    let mut lo = 0.0;
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) { hi = mid } else { lo = mid }
    }
    Some(hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_insights_formula() {
        for b in [0.002, 0.03, 0.2] {
            assert_eq!(per_arm(b, 0.2), crate::insights::required_per_arm(b));
        }
        assert_eq!(per_arm(0.0, 0.2), None);
        assert_eq!(per_arm(0.8, 0.5), None);
    }

    /// cliponit, for real: 26 185 visits / 30 days, 0.2 % signup, official + 1 variant.
    #[test]
    fn low_traffic_low_rate() {
        let (daily, b) = (26_185.0 / 30.0, 0.002);
        let d = |lift| days(per_arm(b, lift).unwrap(), 2, daily).unwrap();
        assert!(d(0.10) > 1_500.0, "{}", d(0.10)); // ~5 years: hopeless
        assert!((400.0..600.0).contains(&d(0.20)), "{}", d(0.20));
        assert!((80.0..100.0).contains(&d(0.50)), "{}", d(0.50));
        let l = detectable_lift(b, 2, daily, 500, WARN_DAYS).unwrap();
        assert!((0.55..0.75).contains(&l), "{l}");
        // Fewer arms or a more frequent goal: smaller detectable lift.
        assert!(detectable_lift(b, 3, daily, 500, WARN_DAYS).unwrap() > l);
        assert!(detectable_lift(0.05, 2, daily, 500, WARN_DAYS).unwrap() < 0.15);
        // The lift found is just enough.
        assert!(days(per_arm(b, l).unwrap(), 2, daily).unwrap() <= WARN_DAYS);
        assert!(days(per_arm(b, l * 0.95).unwrap(), 2, daily).unwrap() > WARN_DAYS);
    }

    #[test]
    fn floors_and_no_traffic() {
        assert_eq!(days(10, 2, 1_000.0), Some(MIN_DAYS));
        assert_eq!(days(10, 2, 0.0), None);
        assert_eq!(detectable_lift(0.001, 2, 1.0, 500, WARN_DAYS), None);
    }
}
