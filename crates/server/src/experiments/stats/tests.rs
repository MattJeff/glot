use super::*;
use rand::{SeedableRng, rngs::StdRng};

const DRAWS: usize = 2000;

struct World {
    /// Conversion rate per [cell][arm].
    rates: Vec<Vec<f64>>,
    /// Mean order value per arm (revenue worlds).
    aov: Option<Vec<f64>>,
    /// Visitors per cell per day.
    traffic: usize,
    /// Share of arm 1's exposures lost (SRM injection).
    lose_arm1: f64,
}

struct Run {
    /// First decision per cell (cell winner), and the first "everywhere" / segment recommendations.
    cell_winner: Vec<Option<(usize, f64)>>,
    everywhere: Option<(usize, f64)>,
    segments: Vec<(String, usize)>,
    paused_day: Option<f64>,
    last: Analysis,
}

/// Day-by-day traffic, batch weights recomputed once a day from the last analysis (as `rebalance` does).
fn simulate(
    w: &World,
    allocation: Allocation,
    metric: Metric,
    days: usize,
    min_n: i64,
    rng: &mut StdRng,
) -> Run {
    let (nc, k) = (w.rates.len(), w.rates[0].len());
    let mut cells: Vec<Cell> = (0..nc)
        .map(|c| Cell {
            name: format!("c{c}"),
            arms: vec![Arm::default(); k],
            expected: Some(vec![0.0; k]),
        })
        .collect();
    let mut weights = vec![vec![1.0 / k as f64; k]; nc];
    let (mut look, mut run) = (0, None::<Run>);
    let mut cell_winner = vec![None; nc];
    let (mut everywhere, mut segments, mut paused_day) = (None, Vec::new(), None);
    for day in 1..=days {
        for (c, cell) in cells.iter_mut().enumerate() {
            for _ in 0..w.traffic {
                let u: f64 = rng.random();
                let (mut acc, mut i) = (0.0, k - 1);
                for (j, x) in weights[c].iter().enumerate() {
                    acc += x;
                    if u < acc {
                        i = j;
                        break;
                    }
                }
                let e = cell.expected.as_mut().unwrap();
                e.iter_mut().zip(&weights[c]).for_each(|(e, x)| *e += x);
                if i == 1 && rng.random::<f64>() < w.lose_arm1 {
                    continue; // exposure lost (e.g. cached page, beacon dropped)
                }
                let a = &mut cell.arms[i];
                a.views += 1;
                if rng.random::<f64>() < w.rates[c][i] {
                    a.conversions += 1;
                    if let Some(aov) = &w.aov {
                        // Log-normal order values, σ = 0.6, mean aov[i].
                        let v = aov[i] * (0.6 * normal(rng) - 0.18).exp();
                        a.revenue += v;
                        a.revenue_sq += v * v;
                    }
                }
            }
        }
        let a = analyze(
            &Input {
                metric,
                allocation,
                fixed: None,
                min_n,
                days: day as f64,
                look,
                cells: &cells,
                holdout: None,
                draws: DRAWS,
            },
            rng,
        )
        .unwrap();
        look += a.decision_look as i32;
        weights = a
            .cells
            .iter()
            .map(|c| c.arms.iter().map(|x| x.weight).collect())
            .collect();
        for (c, r) in a.cells.iter().enumerate() {
            if cell_winner[c].is_none() {
                cell_winner[c] = r.winner.map(|x| (x, day as f64));
            }
        }
        if everywhere.is_none() {
            everywhere = a.everywhere.map(|x| (x, day as f64));
        }
        for s in &a.segments {
            if !segments.iter().any(|x: &(String, usize)| x.0 == s.0) {
                segments.push(s.clone());
            }
        }
        if a.paused && paused_day.is_none() {
            paused_day = Some(day as f64);
        }
        let stop = a.paused;
        run = Some(Run {
            cell_winner: cell_winner.clone(),
            everywhere,
            segments: segments.clone(),
            paused_day,
            last: a,
        });
        if stop {
            break; // paused: no more traffic counted
        }
    }
    run.unwrap()
}

fn world(rates: Vec<Vec<f64>>, traffic: usize) -> World {
    World {
        rates,
        aov: None,
        traffic,
        lose_arm1: 0.0,
    }
}

#[test]
fn srm_and_chi2() {
    // χ²(1) = 10.83 ↔ p = 0.001; χ²(2) = 5.991 ↔ p = 0.05.
    assert!((gamma_q(0.5, 10.83 / 2.0) - 0.001).abs() < 2e-5);
    assert!((gamma_q(1.0, 5.991 / 2.0) - 0.05).abs() < 1e-4);
    assert!(srm_p(&[5000, 5000], &[1.0, 1.0]) > 0.99);
    assert!(srm_p(&[5000, 4650], &[1.0, 1.0]) < SRM_P);
    assert_eq!(srm_p(&[10, 1], &[1.0, 0.0]), 0.0);
    assert_eq!(srm_p(&[0, 0], &[1.0, 1.0]), 1.0);
}

#[test]
fn bh_matches_reference() {
    // Classic example: q = p·m/rank, made monotone.
    let q = bh(&[0.01, 0.04, 0.03, 0.005]);
    let want = [0.02, 0.04, 0.04, 0.02];
    for (a, b) in q.iter().zip(want) {
        assert!((a - b).abs() < 1e-12, "{q:?}");
    }
    let mut rng = StdRng::seed_from_u64(3);
    // 20 null slices + 1 real effect: only the real one survives BH.
    let mut slices: Vec<Vec<Arm>> = (0..20)
        .map(|_| {
            (0..2)
                .map(|_| {
                    let c = (0..2000).filter(|_| rng.random::<f64>() < 0.05).count() as i64;
                    Arm {
                        views: 2000,
                        conversions: c,
                        ..Default::default()
                    }
                })
                .collect()
        })
        .collect();
    slices.push(vec![
        Arm {
            views: 4000,
            conversions: 200,
            ..Default::default()
        },
        Arm {
            views: 4000,
            conversions: 320,
            ..Default::default()
        },
    ]);
    let r = exploratory(Metric::Conversion, &slices, 4000, &mut rng);
    assert!(r[20][0].1 < 0.05, "{:?}", r[20]);
    assert!(r[..20].iter().all(|s| s[0].1 > 0.05), "{r:?}");
}

#[test]
fn floors_and_pooling() {
    let w = floor_weights(&[0.97, 0.02, 0.01]);
    assert!(
        (w[1] - 0.1).abs() < 1e-12 && (w[2] - 0.1).abs() < 1e-12 && (w[0] - 0.8).abs() < 1e-12,
        "{w:?}"
    );
    assert_eq!(floor_weights(&[0.5, 0.5]), vec![0.5, 0.5]);
    // Tiny cell (40 visitors, 0 conversions) next to big cells at 5 %: pulled toward 5 %, not stuck near 0.
    let big = Arm {
        views: 10_000,
        conversions: 500,
        ..Default::default()
    };
    let tiny = Arm {
        views: 40,
        conversions: 0,
        ..Default::default()
    };
    let p = posteriors(
        Metric::Conversion,
        &[&[big, big], &[big, big], &[tiny, tiny]],
        true,
    );
    let m = p[2][0].mean();
    assert!(m > 0.03 && m < 0.05, "pooled {m}");
    // Clearly different cells (5 % vs 20 %): the prior is weak, each cell keeps its own rate.
    let hi = Arm {
        views: 10_000,
        conversions: 2000,
        ..Default::default()
    };
    assert!(pool_k(&[big, hi]) < 25.0, "{}", pool_k(&[big, hi]));
    let p = posteriors(Metric::Conversion, &[&[big, big], &[hi, hi]], true);
    assert!((p[1][0].mean() - 0.2).abs() < 0.005);
}

#[test]
fn reindex_by_arm_number() {
    let cells = vec![Cell {
        name: "*".into(),
        arms: vec![
            Arm {
                views: 10_000,
                conversions: 500,
                ..Default::default()
            },
            Arm {
                views: 10_000,
                conversions: 800,
                ..Default::default()
            },
        ],
        expected: None,
    }];
    let input = Input {
        metric: Metric::Conversion,
        allocation: Allocation::Bandit,
        fixed: None,
        min_n: 100,
        days: 10.0,
        look: 0,
        cells: &cells,
        holdout: None,
        draws: 2000,
    };
    let mut a = analyze(&input, &mut StdRng::seed_from_u64(4)).unwrap();
    a.arm_ids = vec![0, 3]; // arms 1-2 unapproved
    let a = a.by_arm_number();
    assert_eq!(a.cells[0].arms.len(), 4);
    assert_eq!((a.everywhere, a.cells[0].winner), (Some(3), Some(3)));
    assert_eq!(a.cells[0].arms[1].weight, 0.0);
    // Top-two with 2 arms = 50/50 by construction (leader + the only challenger).
    assert!((a.cells[0].arms[3].weight - 0.5).abs() < 1e-9);
}

#[test]
fn guardrails() {
    let c = |n| Cell {
        name: "x".into(),
        arms: vec![Arm::default(); n],
        expected: None,
    };
    let mut rng = StdRng::seed_from_u64(1);
    let input = |cells: &'static [Cell]| Input {
        metric: Metric::Conversion,
        allocation: Allocation::Segmented,
        fixed: None,
        min_n: 100,
        days: 30.0,
        look: 0,
        cells,
        holdout: None,
        draws: 100,
    };
    let five: &'static [Cell] =
        Box::leak(vec![c(2), c(2), c(2), c(2), c(2), c(2)].into_boxed_slice());
    assert_eq!(
        analyze(&input(five), &mut rng).unwrap_err(),
        Invalid::TooManyCells
    );
    let mixed: &'static [Cell] = Box::leak(vec![c(2), c(3)].into_boxed_slice());
    assert_eq!(
        analyze(&input(mixed), &mut rng).unwrap_err(),
        Invalid::ArmsMismatch
    );
    // Holdout SRM pauses: 5 % configured, 7 % observed on 20k sessions.
    let one: &'static [Cell] = Box::leak(vec![c(2)].into_boxed_slice());
    let mut i = input(one);
    i.holdout = Some(Holdout {
        held: 1400,
        exposed: 18_600,
        pct: 0.05,
    });
    assert!(analyze(&i, &mut rng).unwrap().paused);
    i.holdout = Some(Holdout {
        held: 1010,
        exposed: 18_990,
        pct: 0.05,
    });
    assert!(!analyze(&i, &mut rng).unwrap().paused);
}

/// Known per-segment rates: c0 favours arm 1, c1 favours arm 2, c2 has no effect.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "heavy simulation: cargo test --release -p glot-server --lib experiments::stats"
)]
fn simulation_right_winner_per_segment() {
    let mut rng = StdRng::seed_from_u64(7);
    let runs = 40;
    let (mut right, mut wrong, mut right_seg, mut early) = (0, 0, 0, 0);
    let mut hits = (0, 0);
    for _ in 0..runs {
        let w = world(
            vec![
                vec![0.04, 0.06, 0.04],
                vec![0.04, 0.04, 0.06],
                vec![0.04, 0.04, 0.04],
            ],
            1500,
        );
        let r = simulate(
            &w,
            Allocation::Segmented,
            Metric::Conversion,
            42,
            500,
            &mut rng,
        );
        for (c, want) in [(0, 1), (1, 2)] {
            match r.cell_winner[c] {
                Some((x, d)) if x == want => {
                    right += 1;
                    early += (d < MIN_DAYS) as usize;
                }
                Some(_) => wrong += 1,
                None => {}
            }
        }
        hits.0 += r.segments.contains(&("c0".into(), 1)) as usize;
        hits.1 += r.segments.contains(&("c1".into(), 2)) as usize;
        right_seg += r
            .segments
            .iter()
            .all(|s| (s.0 == "c0" && s.1 == 1) || (s.0 == "c1" && s.1 == 2))
            as usize;
        wrong += r.cell_winner[2].is_some_and(|x| x.0 != 0) as usize;
    }
    eprintln!(
        "per segment ({runs} runs, 2 cells with an effect): right {right}/{}, wrong {wrong}, segment recos c0 {} c1 {}, \
         runs with only correct segment recos {right_seg}",
        2 * runs,
        hits.0,
        hits.1
    );
    assert_eq!(early, 0, "decision before 7 days");
    assert!(right as f64 >= 0.8 * (2 * runs) as f64, "right {right}");
    assert!(wrong <= runs / 20, "wrong {wrong}");
    assert_eq!(right_seg, runs);
    assert!(hits.0 + hits.1 >= runs, "segment-specific recos {hits:?}");
}

/// A/A: 3 cells × 3 equal arms, daily analyses for 6 weeks. Any winner (cell or global) is a false positive.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "heavy simulation: cargo test --release -p glot-server --lib experiments::stats"
)]
fn simulation_aa_false_positives() {
    let mut rng = StdRng::seed_from_u64(11);
    let runs = 200;
    let (mut fp, mut srm) = (0, 0);
    for _ in 0..runs {
        let w = world(vec![vec![0.05; 3]; 3], 500);
        let r = simulate(
            &w,
            Allocation::Segmented,
            Metric::Conversion,
            42,
            300,
            &mut rng,
        );
        fp += (r.everywhere.is_some()
            || !r.segments.is_empty()
            || r.cell_winner.iter().any(|c| c.is_some())) as usize;
        srm += r.paused_day.is_some() as usize;
    }
    eprintln!("A/A segmented: false positives {fp}/{runs}, false SRM alarms {srm}/{runs}");
    assert!(
        fp as f64 <= 0.05 * runs as f64,
        "false positives {fp}/{runs}"
    );
    assert!(srm <= 3, "false SRM {srm}/{runs}");
    // Global bandit, 2 cells × 2 arms, more traffic.
    let runs = 300;
    let fp = (0..runs)
        .filter(|_| {
            let r = simulate(
                &world(vec![vec![0.05; 2]; 2], 1500),
                Allocation::Bandit,
                Metric::Conversion,
                42,
                500,
                &mut rng,
            );
            r.everywhere.is_some()
                || !r.segments.is_empty()
                || r.cell_winner.iter().any(|c| c.is_some())
        })
        .count();
    eprintln!("A/A bandit 2×2: false positives {fp}/{runs}");
    assert!(
        fp as f64 <= 0.05 * runs as f64,
        "false positives {fp}/{runs}"
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "heavy simulation: cargo test --release -p glot-server --lib experiments::stats"
)]
fn simulation_srm_detected() {
    let mut rng = StdRng::seed_from_u64(5);
    let runs = 20;
    for (loss, days, need) in [(0.10, 14, runs * 9 / 10), (0.05, 28, runs * 3 / 4)] {
        let mut detected = 0;
        for _ in 0..runs {
            let mut w = world(vec![vec![0.05; 2]; 2], 1000);
            w.lose_arm1 = loss; // share of arm 1's exposures that never reach glot
            let r = simulate(
                &w,
                Allocation::Fixed,
                Metric::Conversion,
                days,
                300,
                &mut rng,
            );
            detected += r.paused_day.is_some() as usize;
            // Once paused: no decision look, and the run stops counting.
            assert!(!r.last.paused || !r.last.decision_look);
        }
        eprintln!(
            "SRM ({}% of one arm lost, 1000/cell/day): detected within {days} days {detected}/{runs}",
            loss * 100.0
        );
        assert!(detected >= need);
    }
}

#[test]
fn simulation_low_traffic_no_premature_decision() {
    let mut rng = StdRng::seed_from_u64(9);
    // Huge effect (4 % → 12 %) but 30 visitors/cell/day: arms never reach min_n = 500 within 4 weeks.
    for _ in 0..10 {
        let w = world(vec![vec![0.04, 0.12]; 2], 30);
        let r = simulate(
            &w,
            Allocation::Segmented,
            Metric::Conversion,
            28,
            500,
            &mut rng,
        );
        assert!(r.everywhere.is_none() && r.cell_winner.iter().all(|c| c.is_none()));
        assert!(!r.last.decision_look);
    }
    // Plenty of traffic: decided, but never before day 7.
    for _ in 0..10 {
        let w = world(vec![vec![0.04, 0.12]; 2], 5000);
        let r = simulate(
            &w,
            Allocation::Segmented,
            Metric::Conversion,
            14,
            500,
            &mut rng,
        );
        let (arm, day) = r.everywhere.expect("decided");
        assert_eq!((arm, day), (1, MIN_DAYS));
    }
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "heavy simulation: cargo test --release -p glot-server --lib experiments::stats"
)]
fn simulation_revenue_metric() {
    let mut rng = StdRng::seed_from_u64(13);
    let runs = 20;
    let (mut rev, mut conv) = (0, 0);
    for _ in 0..runs {
        // Same conversion rate, arm 1 sells baskets 30 % larger.
        let mut w = world(vec![vec![0.05, 0.05]; 2], 1500);
        w.aov = Some(vec![40.0, 52.0]);
        rev += (simulate(&w, Allocation::Bandit, Metric::Revenue, 42, 500, &mut rng)
            .everywhere
            .map(|x| x.0)
            == Some(1)) as usize;
        conv += simulate(
            &w,
            Allocation::Bandit,
            Metric::Conversion,
            42,
            500,
            &mut rng,
        )
        .everywhere
        .is_some() as usize;
    }
    eprintln!(
        "revenue (+30 % basket, same rate): revenue metric right {rev}/{runs}, conversion metric any winner {conv}/{runs}"
    );
    assert!(rev >= runs * 3 / 4);
    assert!(conv <= 2);
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "heavy simulation: cargo test --release -p glot-server --lib experiments::stats"
)]
fn simulation_revenue_aa() {
    let mut rng = StdRng::seed_from_u64(17);
    let runs = 300;
    let mut fp = 0;
    for _ in 0..runs {
        let mut w = world(vec![vec![0.05; 2]; 2], 800);
        w.aov = Some(vec![40.0, 40.0]);
        let r = simulate(
            &w,
            Allocation::Segmented,
            Metric::Revenue,
            42,
            300,
            &mut rng,
        );
        fp += (r.everywhere.is_some() || r.cell_winner.iter().any(|c| c.is_some())) as usize;
    }
    eprintln!("A/A revenue: false positives {fp}/{runs}");
    assert!(fp as f64 <= 0.05 * runs as f64);
}
