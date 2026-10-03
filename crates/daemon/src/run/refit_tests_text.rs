//! Task M9.5.8: the report and its text (decision 11, Interfaces "CLI"): too few
//! samples, a configured budget (ruling RH-5), the refit off, and the exact block for
//! `refit.jsonl`.

use std::path::Path;

use proto::{ClassBudget, RefitState, TuningFile};

use super::super::refit_render::render;
use super::tests::{NOW, budget, fixture_records};
use super::{SizeClass, refit, report, tuned};

#[test]
fn too_few_samples_refit_nothing_and_say_so() {
    let lines = fixture_records("too-few");
    let cfg = config::Orchestrator::default();
    let (file, log) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert_eq!(file, TuningFile::default());
    assert!(log.is_empty(), "{log:?}");
    assert!(super::proposals(&lines, &file, &cfg).is_empty());
    let r = report(&lines, &file, &cfg, Path::new("/tmp/ax-repo/tuning.toml"));
    let s = &r.classes[0];
    assert_eq!((s.class.as_str(), s.samples), ("S", 29));
    assert_eq!(s.refit, RefitState::NotYet);
    assert_eq!(s.refit_budget, None);
    let text = render(&r);
    assert!(
        text.contains("\n  S      29/30    40 calls 15m    -       not yet\n"),
        "{text}"
    );
    assert!(text.contains("\ntuning proposals: none\n"), "{text}");
    assert!(!text.contains("apply with"), "{text}");
    assert_eq!(tuned(&file, &cfg).budget_s, None);
}

#[test]
fn a_configured_budget_beats_the_refit() {
    let lines = fixture_records("refit");
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.configured.s = true;
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let t = tuned(&file, &cfg);
    assert_eq!(t.budget_s, None, "the configured budget is used");
    assert_eq!(t.effective(&cfg, SizeClass::S), budget(40, 15));
    assert_eq!(
        t.log[0],
        "tuning: budget S 40 calls 15m configured (refit would be 55 calls 18m)"
    );
    let r = report(&lines, &file, &cfg, Path::new("/tmp/ax-repo/tuning.toml"));
    let s = &r.classes[0];
    assert!(s.configured);
    assert_eq!(s.refit, RefitState::Configured);
    assert_eq!(s.budget, budget(40, 15));
    assert_eq!(s.refit_budget, Some(budget(55, 18)));
    let text = render(&r);
    assert!(
        text.contains("\n  S      34       40 calls 15m    9m      configured\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "  * derived from another class's median\n  budget S: configured 40 calls 15m (refit would be 55 calls 18m)\n"
        ),
        "{text}"
    );
    // Not configured, the refit is used.
    let t = tuned(&file, &config::Orchestrator::default());
    assert_eq!(t.budget_s, Some(budget(55, 18)));
    assert_eq!(t.log[0], "tuning: budget S 55 calls 18m from 34 samples");
}

#[test]
fn render_matches_the_cli_text() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let r = report(&lines, &file, &cfg, Path::new("/tmp/ax-repo/tuning.toml"));
    let expected = "\
tuning: /tmp/ax-repo/tuning.toml  (refit after 30 samples per class)
  CLASS  SAMPLES  BUDGET          WEIGHT  REFIT
  S      34       55 calls 18m    9m      2026-09-27 09:06
  M      12/30    150 calls 60m   27m*    not yet
  hub    3/30     150 calls 60m   27m*    not yet
  * derived from another class's median
tuning proposals:
  thresholds.s  S line threshold 20 → 35 (p90 of 34 merged S tasks)
  route.s       S route standard/low → standard/medium (14 of 34 S tasks, 41%, reached rung 2 or higher)
apply with anthrex run stats --apply <id>; dismiss with anthrex run stats --dismiss <id>
";
    assert_eq!(render(&r), expected);
    assert_eq!(r.min_samples, 30);
    assert!(r.refit_budgets);
    assert_eq!(r.orchestrator_list, None);
}

#[test]
fn render_says_when_the_budget_refit_is_off() {
    let lines = fixture_records("refit");
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.table.refit_budgets = false;
    let (file, log) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert!(file.budgets.is_empty());
    assert_eq!(log.len(), 1, "weights only: {log:?}");
    let r = report(&lines, &file, &cfg, Path::new("/tmp/t/tuning.toml"));
    let text = render(&r);
    assert!(text.starts_with(
        "tuning: /tmp/t/tuning.toml  (budget refit off: [orchestrator.tuning] refit_budgets = false)\n"
    ));
    assert!(
        text.contains("\n  S      34       40 calls 15m    9m      off\n"),
        "{text}"
    );
    // A refit already in the file is ignored with the refit off.
    let mut written = TuningFile::default();
    written.budgets.insert(
        "s".into(),
        ClassBudget {
            tool_calls: 55,
            minutes: 18,
            tokens: None,
            samples: 34,
            at: NOW,
        },
    );
    assert_eq!(tuned(&written, &cfg).budget_s, None);
}
