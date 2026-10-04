//! Task M9.5.8, fix round 1: the token refit over recorded usage only (ruling T8-5),
//! "no change" at zero, a change of what is derived, a configured class's refit shown
//! below the 20 % gate, and rung 4's token ceiling (ruling T9-1).

use std::path::Path;

use proto::{Budget, HistoryLine, PathWeights, TuningFile};

use super::super::refit_render::render;
use super::tests::{NOW, budget, record};
use super::{SizeClass, ceiling, moved, refit, report, tuned, tuned_with};

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
    assert!(
        file.budgets.is_empty(),
        "shown, never written (ruling T8-7)"
    );
    let t = tuned_with(&lines, &file, &cfg);
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
        ceiling(SizeClass::M, tok(150, 60, None), tok(150, 60, None), l),
        tok(300, 120, Some(1000))
    );
    assert_eq!(
        ceiling(SizeClass::Hub, tok(150, 60, None), tok(150, 60, None), l),
        tok(300, 120, Some(1000))
    );
    // Both: the larger of L's and twice M's.
    assert_eq!(
        ceiling(
            SizeClass::M,
            tok(150, 60, Some(800)),
            tok(150, 60, Some(800)),
            l
        ),
        tok(300, 120, Some(1600))
    );
    assert_eq!(
        ceiling(
            SizeClass::M,
            tok(150, 60, Some(300)),
            tok(150, 60, Some(300)),
            l
        ),
        tok(300, 120, Some(1000))
    );
    // L has none: no token ceiling.
    assert_eq!(
        ceiling(
            SizeClass::M,
            tok(150, 60, Some(800)),
            tok(150, 60, Some(800)),
            tok(300, 120, None)
        ),
        tok(300, 120, None)
    );
    // S: the effective M budget as it is.
    assert_eq!(
        ceiling(SizeClass::S, tok(40, 15, None), tok(150, 60, None), l),
        tok(150, 60, None)
    );
}

/// Ruling T8-6: rung 4's ceiling is never below the class's own effective budget.
#[test]
fn the_ceiling_never_drops_below_the_classs_own_budget() {
    let tok = |calls, minutes, tokens| Budget {
        tool_calls: calls,
        minutes,
        tokens,
    };
    let (m, l) = (tok(150, 60, None), tok(300, 120, None));
    // An S refit of 175 calls over an effective M of 150.
    assert_eq!(
        ceiling(SizeClass::S, tok(175, 70, None), m, l),
        tok(175, 70, None)
    );
    // Per axis: the calls are S's own, the minutes M's.
    assert_eq!(
        ceiling(SizeClass::S, tok(175, 40, None), m, l),
        tok(175, 60, None)
    );
    // A hub refit above max(L, 2 × M): the hub budget.
    assert_eq!(
        ceiling(SizeClass::Hub, tok(700, 300, None), m, l),
        tok(700, 300, None)
    );
    // The defaults are unchanged.
    let cfg = config::Orchestrator::default();
    let (s, m, l) = (cfg.budget_s, cfg.budget_m, cfg.budget_l);
    assert_eq!(ceiling(SizeClass::S, s, m, l), tok(150, 60, None));
    assert_eq!(ceiling(SizeClass::M, m, m, l), tok(300, 120, None));
    assert_eq!(ceiling(SizeClass::Hub, m, m, l), tok(300, 120, None));
    // Tokens: own above the rule's wins; no rule ceiling stays none.
    let l = tok(300, 120, Some(1000));
    assert_eq!(
        ceiling(
            SizeClass::M,
            tok(150, 60, Some(3000)),
            tok(150, 60, None),
            l
        ),
        tok(300, 120, Some(3000))
    );
    assert_eq!(
        ceiling(SizeClass::S, tok(40, 15, Some(3000)), tok(150, 60, None), l),
        tok(150, 60, None)
    );
}

/// Ruling T8-7: a configured class's refit is shown, never written, and removing the
/// configuration later starts from history under the normal gate.
#[test]
fn a_configured_class_is_never_written_to_the_file() {
    let lines = super::tests::fixture_records("refit");
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.configured.s = true;
    // A refit written before the class was configured stays exactly as it is.
    let mut stale = TuningFile::default();
    stale.budgets.insert(
        "s".into(),
        proto::ClassBudget {
            tool_calls: 60,
            minutes: 20,
            tokens: None,
            samples: 31,
            at: 1,
        },
    );
    let (kept, log) = refit(&lines, &stale, &cfg, NOW);
    assert_eq!(kept.budgets, stale.budgets);
    assert!(log.iter().all(|l| !l.contains("budget S")), "{log:?}");
    let (fresh, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert!(!fresh.budgets.contains_key("s"));
    // Shown from history: report, render and the start line.
    let r = report(&lines, &fresh, &cfg, Path::new("/tmp/t/tuning.toml"));
    assert_eq!(r.classes[0].refit_budget, Some(budget(55, 18)));
    assert!(render(&r).contains("(refit would be 55 calls 18m)"));
    let t = tuned_with(&lines, &kept, &cfg);
    assert_eq!(t.budget_s, None);
    assert_eq!(
        t.log[0],
        "tuning: budget S 40 calls 15m configured (refit would be 55 calls 18m)"
    );
    // With no history to compute from, the start line says only what is used.
    assert_eq!(
        tuned(&kept, &cfg).log[0],
        "tuning: budget S 40 calls 15m configured"
    );
    // The configuration removed: the normal path, from history.
    let plain = config::Orchestrator::default();
    let (after, log) = refit(&lines, &fresh, &plain, NOW);
    assert_eq!(
        (after.budgets["s"].tool_calls, after.budgets["s"].minutes),
        (55, 18)
    );
    assert_eq!(
        log[0],
        "tuning: budget S 40 calls 15m → 55 calls 18m from 34 samples"
    );
    // And under the normal gate: 55/18 is within 20 % of the old 60/20, so it is kept.
    let (after, _) = refit(&lines, &stale, &plain, NOW);
    assert_eq!(after.budgets, stale.budgets);
}

/// Ruling T8-8: with nothing learned from history, the start log keeps decision 12's
/// `none` line, after the `configured` lines.
#[test]
fn nothing_learned_still_says_none_after_the_configured_lines() {
    let too_few = super::tests::fixture_records("too-few");
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.configured.s = true;
    cfg.tuning.configured.m = true;
    let none = "tuning: none (history has fewer than 30 samples per class)";
    let (file, _) = refit(&too_few, &TuningFile::default(), &cfg, NOW);
    for t in [tuned_with(&too_few, &file, &cfg), tuned(&file, &cfg)] {
        assert_eq!(
            t.log,
            [
                "tuning: budget S 40 calls 15m configured",
                "tuning: budget M 150 calls 60m configured",
                none,
            ]
        );
    }
    // Something learned (a refit to show): no `none` line.
    let lines = super::tests::fixture_records("refit");
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let t = tuned_with(&lines, &file, &cfg);
    assert!(!t.log.iter().any(|l| l == none), "{:?}", t.log);
}

/// Whole-branch review B, M4: the `none` line blames too few samples only when that is
/// true. A class that qualifies but whose refit stayed near the default, or a refit
/// that is off, says so instead.
#[test]
fn the_none_line_says_why_nothing_was_learned() {
    let lines = super::tests::fixture_records("refit");
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.table.path_weights = false;
    cfg.tuning.table.min_change_percent = 100;
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert!(file.budgets.is_empty(), "kept: {file:?}");
    assert_eq!(
        tuned_with(&lines, &file, &cfg).log,
        ["tuning: none (history's refit is within 100% of the default budgets)"]
    );
    cfg.tuning.table.refit_budgets = false;
    assert_eq!(
        tuned_with(&lines, &file, &cfg).log,
        ["tuning: none (the budget refit is off)"]
    );
}
