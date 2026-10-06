//! Milestone 9.7's final fix wave, FW-16 (task 10's Minors): the CI bisect's whole-line
//! range (decision 13, DH §3.1) on a stage with a floor, and with a base sync's merge
//! below the last green as the first red merge.

use proto::CiCategory;

use super::bisect::{TEST, answer, probe, with_orchestrator};
use super::delivery_ci::{ci_fixes, logged};
use super::delivery_ci_repro::{red_probe, red_summarised, tiered_watched};
use super::merge::commit;
use crate::run::contract::sha7;
use crate::run::model::StageMerge;

fn commit_of(m: &StageMerge) -> &str {
    match m {
        StageMerge::Task { commit, .. } | StageMerge::Propagate { commit, .. } => commit,
    }
}

/// A stage rebaselined at `commit(2)` (its floor; ruling C-28 (1)): the merges at or
/// below it are forgotten, as `forget_line` does. Tier 3 was green at `commit(3)`.
#[test]
fn a_ci_red_on_a_floored_stage_bisects_from_its_floor() {
    let mut fx = tiered_watched(&["t1", "t2", "t3", "t4"]);
    with_orchestrator(&mut fx);
    let s = &mut fx.run_mut().stages[0];
    s.floor = Some(commit(2));
    s.merges
        .retain(|m| commit_of(m) == commit(3) || commit_of(m) == commit(4));
    s.full.green_at = Some(commit(3));
    red_summarised(&mut fx, &commit(4), &[TEST], CiCategory::Test);
    let (op, spec) = probe(&fx);
    fx.done(op, red_probe(&spec.commands[0]));
    let line = format!(
        "stage 1: CI red at {} reproduces; bisecting 2 merges",
        sha7(&commit(4))
    );
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    let b = fx.run().stage(1).unwrap().bisect.clone().expect("a bisect");
    assert_eq!(b.base, commit(2), "the floor, not the last green");
    let probed = answer(&mut fx, 3);
    assert_eq!(probed[0], commit(2), "the first probe is the floor");
    let fixes = ci_fixes(&fx);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    let brief = &fx.task(&fixes[0]).spec.brief;
    assert!(
        brief.contains("\nBisect found the merge of task t3 ("),
        "{brief}"
    );
}

/// A base sync's merge (`Propagate { from: 0 }`) below tier 3's last green is the first
/// red merge: the whole-line bisect reaches it and ends on decision 36's "no single
/// culprit" path, which gives the CI red its stage fix.
#[test]
fn a_base_syncs_merge_below_the_last_green_ends_the_ci_bisect_without_a_culprit() {
    let mut fx = tiered_watched(&["t1", "t2", "t3"]);
    with_orchestrator(&mut fx);
    let s = &mut fx.run_mut().stages[0];
    let at = s
        .merges
        .iter()
        .position(|m| commit_of(m) == commit(2))
        .unwrap();
    s.merges[at] = StageMerge::Propagate {
        from: 0,
        commit: commit(2),
    };
    s.full.green_at = Some(commit(3));
    red_summarised(&mut fx, &commit(3), &[TEST], CiCategory::Test);
    let (op, spec) = probe(&fx);
    fx.done(op, red_probe(&spec.commands[0]));
    answer(&mut fx, 2);
    let line = "stage 1: bisect ended without a culprit: the first red merge is the merge of the base branch";
    assert!(logged(&fx, line), "{:#?}", fx.run().log);
    assert_eq!(ci_fixes(&fx).len(), 1, "the stage fix");
}
