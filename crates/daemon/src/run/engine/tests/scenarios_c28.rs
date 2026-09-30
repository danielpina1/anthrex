//! Final review of milestone 9.1, ruling C-28 (1): a rebaseline to a commit the engine
//! never recorded on a stage's line sets the stage's `floor`, and a later bisect starts
//! from it instead of the stage's creation point, so a red older than the floor blames
//! nobody.

use proto::RunState;

use super::bisect::{merge_next, probe, probe_result};
use super::control::resume;
use super::control_restore::restart;
use super::fixture::*;
use super::merge::commit;
use super::propagate::land_propagates;
use super::scenarios::{five, halt_and_rebaseline, invariants, red_tier3, roundtrip};
use crate::run::contract::sha7;
use crate::run::model::{FixOf, StageMerge};

/// Every probe answered by `red`, in order; the commits probed.
fn answer_by(fx: &mut Fixture, red: impl Fn(&str) -> bool) -> Vec<String> {
    let mut probed = Vec::new();
    while !super::merge::pending(fx, "TestAt", None).is_empty() {
        let (op, spec) = probe(fx);
        fx.done(op, probe_result(red(&spec.commit), &spec.commit));
        probed.push(spec.commit);
        assert!(probed.len() <= 12, "the bisect does not end: {probed:?}");
    }
    probed
}

/// Stage 1 moved by a rebaseline to `to`, stage 2 kept where it is.
fn rebaseline_stage1(fx: &mut Fixture, to: &str) {
    let s2_head = fx.run().stage_head(2).unwrap().to_string();
    halt_and_rebaseline(fx, vec![(1, to.to_string()), (2, s2_head)]);
    assert_eq!(fx.run().state, RunState::Running);
    fx.tick();
    land_propagates(fx);
}

/// five(): t1 at c1 and t2 at c2, then stage 1 rebaselined to c99, a commit of the
/// user's on top of c2 that the engine never wrote.
fn floored() -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, mut windows) = five();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    land_propagates(&mut fx);
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    land_propagates(&mut fx);
    rebaseline_stage1(&mut fx, &commit(99));
    let s = fx.run().stage(1).unwrap();
    assert_eq!(s.floor, Some(commit(99)));
    assert!(s.merges.is_empty(), "{:#?}", s.merges);
    (fx, windows)
}

/// The reviewer's scenario: the red came in with t1, below the floor c99. The bisect
/// probes the floor first, finds it red, and blames nobody (t3 is innocent).
#[test]
fn a_red_older_than_the_floor_blames_nobody() {
    let (mut fx, mut windows) = floored();
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    land_propagates(&mut fx);
    red_tier3(&mut fx, 1);
    // Red everywhere t1's merge is: c1, c2, c99 and c3.
    let probed = answer_by(&mut fx, |c| c != BASE);
    assert_eq!(probed, [commit(99)]);
    let s = fx.run().stage(1).unwrap();
    assert!(s.bisect.is_none());
    let ended = s.full.ended.last().expect("the ended bisect");
    assert_eq!((&ended.culprit, &ended.fix_task), (&None, &None));
    assert_eq!(
        ended.reason.as_deref(),
        Some(
            format!(
                "red before the rebaselined head {}; not bisected",
                sha7(&commit(99))
            )
            .as_str()
        )
    );
    assert!(fx.run().task("fix1").is_none(), "an innocent task blamed");
    invariants(&fx, "after the bisect");
}

/// A red first introduced by t3 above the floor: probed floor, then c3, and t3 blamed.
#[test]
fn a_red_above_the_floor_is_bisected_from_the_floor() {
    let (mut fx, mut windows) = floored();
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    land_propagates(&mut fx);
    red_tier3(&mut fx, 1);
    let probed = answer_by(&mut fx, |c| c == commit(3));
    assert_eq!(probed, [commit(99), commit(3)]);
    assert!(
        matches!(&fx.task("fix1").fixes, Some(FixOf::Bisect { culprit, .. }) if culprit == "t3")
    );
    invariants(&fx, "after the bisect");
}

/// The floor survives a restart, and the bisect after it still starts there.
#[test]
fn the_floor_survives_a_restart() {
    let (mut fx, mut windows) = floored();
    roundtrip(&mut fx);
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    assert_eq!(fx.run().stage(1).unwrap().floor, Some(commit(99)));
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    land_propagates(&mut fx);
    red_tier3(&mut fx, 1);
    let probed = answer_by(&mut fx, |c| c != BASE);
    assert_eq!(probed, [commit(99)]);
    assert!(fx.run().task("fix1").is_none());
}

/// A run.json written before the floor existed loads with none.
#[test]
fn a_stage_without_a_floor_field_loads_with_none() {
    let (fx, _) = five();
    let mut v = serde_json::to_value(fx.run()).unwrap();
    for s in v["stages"].as_array_mut().unwrap() {
        s.as_object_mut().unwrap().remove("floor");
    }
    let run: crate::run::model::Run = serde_json::from_value(v).unwrap();
    assert!(run.stages.iter().all(|s| s.floor.is_none()));
}

/// A rebaseline mid-line to a recorded merge keeps the floor; one to the stage's
/// creation point clears it.
#[test]
fn a_truncation_keeps_the_floor_and_the_creation_point_clears_it() {
    let (mut fx, mut windows) = five();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    land_propagates(&mut fx);
    rebaseline_stage1(&mut fx, &commit(99));
    assert_eq!(fx.run().stage(1).unwrap().floor, Some(commit(99)));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    land_propagates(&mut fx);
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    land_propagates(&mut fx);

    // A tier 3 green at the floor is on the line still after the truncation.
    let s1 = fx.run_mut().stages.iter_mut().find(|s| s.n == 1).unwrap();
    s1.full.green_at = Some(commit(99));
    rebaseline_stage1(&mut fx, &commit(2));
    let s = fx.run().stage(1).unwrap();
    assert_eq!(s.floor, Some(commit(99)), "a truncation dropped the floor");
    assert_eq!(
        s.full.green_at,
        Some(commit(99)),
        "the floor's green forgotten"
    );
    assert_eq!(
        s.merges,
        [StageMerge::Task {
            id: "t2".into(),
            commit: commit(2)
        }]
    );

    rebaseline_stage1(&mut fx, BASE);
    let s = fx.run().stage(1).unwrap();
    assert_eq!(s.floor, None, "the creation point kept a floor");
    assert!(s.merges.is_empty());
    invariants(&fx, "after the rebaselines");
}
