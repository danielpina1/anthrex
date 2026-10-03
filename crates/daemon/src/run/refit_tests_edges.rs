//! Task M9.5.8, fix round 1: the token refit over recorded usage only (ruling T8-5),
//! "no change" at zero, a change of what is derived, a configured class's refit shown
//! below the 20 % gate, and rung 4's token ceiling (ruling T9-1).

use std::path::Path;

use proto::{Budget, HistoryLine, PathWeights, TuningFile};

use super::super::refit_render::render;
use super::tests::{NOW, budget, record};
use super::{SizeClass, ceiling, moved, refit, report, tuned};

/// 30 S records of three runs: 20 tool calls, 600 working seconds; the first
/// `with_usage` record 1000 + i worker tokens, the rest none recorded.
fn usage_lines(with_usage: u32) -> Vec<HistoryLine> {
    (0..30u32)
        .map(|i| {
            let mut r = record(&format!("k{}", i / 10), i, 100 + u64::from(i), 20, 600, 5);
            if i < with_usage {
                r.worker_usage.input = 1000 + u64::from(i);
            }
            HistoryLine::Task(r)
        })
        .collect()
}

fn tokens_on() -> config::Orchestrator {
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.table.refit_tokens = true;
    cfg
}

#[test]
fn the_token_refit_reads_only_samples_with_recorded_usage() {
    let cfg = tokens_on();
    // Every sample recorded: lower median 1014, × 2.5 = 2535.
    let (file, _) = refit(&usage_lines(30), &TuningFile::default(), &cfg, NOW);
    assert_eq!(file.budgets["s"].tokens, Some(2535));
    // Half recorded (15 of 30, under min_samples): no token refit, though the median
    // over all 30 would be 0 and the calls and minutes still refit.
    let (file, log) = refit(&usage_lines(15), &TuningFile::default(), &cfg, NOW);
    let s = file.budgets["s"];
    assert_eq!((s.tool_calls, s.minutes, s.tokens), (50, 25, None));
    assert_eq!(tuned(&file, &cfg).budget_s.and_then(|b| b.tokens), None);
    assert_eq!(
        log[0],
        "tuning: budget S 40 calls 15m → 50 calls 25m from 30 samples"
    );
    // No rewrite loop: refitting again changes nothing.
    let (again, log) = refit(&usage_lines(15), &file, &cfg, NOW + 60);
    assert_eq!(again, file);
    assert!(log.is_empty(), "{log:?}");
    // A configured token budget is kept.
    let mut with_tokens = tokens_on();
    with_tokens.budget_s.tokens = Some(9000);
    let t = tuned(&file, &with_tokens);
    assert_eq!(t.budget_s.and_then(|b| b.tokens), Some(9000));
}

#[test]
fn a_zero_median_of_recorded_usage_refits_no_tokens() {
    // 30 records whose worker usage is only cache reads: recorded, billable 0.
    let lines: Vec<HistoryLine> = (0..30u32)
        .map(|i| {
            let mut r = record(&format!("k{}", i / 10), i, 100 + u64::from(i), 20, 600, 5);
            r.worker_usage.cache_read = 5000;
            HistoryLine::Task(r)
        })
        .collect();
    let cfg = tokens_on();
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert_eq!(file.budgets["s"].tokens, None);
    let (again, log) = refit(&lines, &file, &cfg, NOW + 60);
    assert_eq!(again, file);
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn zero_to_zero_is_no_change() {
    assert!(!moved(0, 0, 20));
    assert!(!moved(40, 40, 1));
    assert!(moved(1, 0, 20));
    assert!(moved(48, 40, 20));
    assert!(!moved(47, 40, 20));
}

#[test]
fn a_change_of_what_is_derived_rewrites_the_weights() {
    // S measured at 550 s; M derived at 1650 s in the file.
    let lines = super::tests::fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    // Same numbers, but the file says S was derived: rewritten.
    let mut stale = file.clone();
    if let Some(w) = stale.weights.as_mut() {
        w.derived = vec!["S".into(), "M".into(), "hub".into()];
    }
    let (fresh, log) = refit(&lines, &stale, &cfg, NOW + 60);
    assert_eq!(
        fresh.weights,
        Some(PathWeights {
            at: NOW + 60,
            ..file.weights.clone().unwrap()
        })
    );
    assert_eq!(log.len(), 1, "{log:?}");
    // The same set: kept.
    let (same, log) = refit(&lines, &file, &cfg, NOW + 60);
    assert_eq!(same, file);
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn a_configured_class_shows_a_refit_below_the_change_gate() {
    // Median 22 calls and 480 working seconds at a factor of 200: 44 calls 16m, within
    // 20 % of the configured 40/15.
    let lines: Vec<HistoryLine> = (0..30u32)
        .map(|i| {
            let r = record(&format!("k{}", i / 10), i, 100 + u64::from(i), 22, 480, 5);
            HistoryLine::Task(r)
        })
        .collect();
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.table.budget_factor_percent = 200;
    cfg.tuning.table.path_weights = false;
    cfg.tuning.configured.s = true;
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let r = report(&lines, &file, &cfg, Path::new("/tmp/t/tuning.toml"));
    assert_eq!(r.classes[0].budget, budget(40, 15));
    assert_eq!(r.classes[0].refit_budget, Some(budget(44, 16)));
    assert!(
        render(&r)
            .contains("\n  budget S: configured 40 calls 15m (refit would be 44 calls 16m)\n"),
        "{}",
        render(&r)
    );
    let t = tuned(&file, &cfg);
    assert_eq!(t.budget_s, None);
    assert_eq!(
        t.log[0],
        "tuning: budget S 40 calls 15m configured (refit would be 44 calls 16m)"
    );
    // Settled: no rewrite.
    let (again, log) = refit(&lines, &file, &cfg, NOW + 60);
    assert_eq!(again, file);
    assert!(log.is_empty(), "{log:?}");
    // Not configured, the same refit is kept, not written.
    cfg.tuning.configured.s = false;
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert!(file.budgets.is_empty());
}

#[test]
fn rung_4_token_ceiling_falls_back_per_axis() {
    let tok = |calls, minutes, tokens| Budget {
        tool_calls: calls,
        minutes,
        tokens,
    };
    let l = tok(300, 120, Some(1000));
    // M has no token budget: L's stands.
    assert_eq!(
        ceiling(SizeClass::M, tok(150, 60, None), l),
        tok(300, 120, Some(1000))
    );
    assert_eq!(
        ceiling(SizeClass::Hub, tok(150, 60, None), l),
        tok(300, 120, Some(1000))
    );
    // Both: the larger of L's and twice M's.
    assert_eq!(
        ceiling(SizeClass::M, tok(150, 60, Some(800)), l),
        tok(300, 120, Some(1600))
    );
    assert_eq!(
        ceiling(SizeClass::M, tok(150, 60, Some(300)), l),
        tok(300, 120, Some(1000))
    );
    // L has none: no token ceiling.
    assert_eq!(
        ceiling(SizeClass::M, tok(150, 60, Some(800)), tok(300, 120, None)),
        tok(300, 120, None)
    );
    // S: the effective M budget as it is.
    assert_eq!(
        ceiling(SizeClass::S, tok(150, 60, None), l),
        tok(150, 60, None)
    );
}
