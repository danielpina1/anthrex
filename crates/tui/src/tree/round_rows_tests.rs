//! Milestone 9.3 task 10b: a run of several rounds hangs each round's stages under its
//! separator, and only an earlier round's rows are muted (decision 32).

use super::{earlier_round, muted_row};
use crate::tree::stage_fixtures::{two_round_fixture, two_stage_fixture};
use crate::tree::{NodeKey, RunFilter, TreeState, run_rows};

const MUL: &str = "add-mul-0723";

fn shape(snap: &proto::RunsSnapshot, state: &TreeState) -> Vec<(u16, NodeKey)> {
    run_rows(&snap.runs[0], &[], state, RunFilter::All)
        .into_iter()
        .map(|row| (row.depth, row.key))
        .collect()
}

fn round(n: u32) -> NodeKey {
    NodeKey::Round { run: MUL.into(), n }
}

fn stage(n: u16) -> NodeKey {
    NodeKey::Stage { run: MUL.into(), n }
}

fn task(id: &str) -> NodeKey {
    NodeKey::Task {
        run: MUL.into(),
        id: id.into(),
    }
}

#[test]
fn each_rounds_stages_hang_under_its_separator() {
    let (snap, _) = two_round_fixture();
    assert_eq!(
        shape(&snap, &TreeState::default()),
        vec![
            (0, NodeKey::Run(MUL.into())),
            (1, round(1)),
            (2, stage(1)),
            (3, task("t1")),
            (1, round(2)),
            (2, stage(2)),
            (3, task("t2")),
            (3, task("t3")),
        ]
    );
}

/// Pinning: a run of one round keeps 9.1's shape, no separator.
#[test]
fn a_one_round_run_has_no_separator() {
    let (snap, _) = two_stage_fixture();
    assert_eq!(
        shape(&snap, &TreeState::default()),
        vec![
            (0, NodeKey::Run(MUL.into())),
            (1, stage(1)),
            (2, task("t1")),
            (1, stage(2)),
            (2, task("t2")),
            (2, task("t3")),
        ]
    );
}

/// A round with no stage listed yet (its plan at the gate) takes its tasks itself; with
/// one stage listed there is no stage node (9.1), so round 1 takes `t1` too.
#[test]
fn a_rounds_tasks_with_no_stage_listed_sit_under_its_separator() {
    let (mut snap, _) = two_round_fixture();
    snap.runs[0].stages.truncate(1);
    assert_eq!(
        shape(&snap, &TreeState::default()),
        vec![
            (0, NodeKey::Run(MUL.into())),
            (1, round(1)),
            (2, task("t1")),
            (1, round(2)),
            (2, task("t2")),
            (2, task("t3")),
        ]
    );
}

#[test]
fn a_folded_round_hides_its_stages() {
    let (snap, _) = two_round_fixture();
    let mut state = TreeState::default();
    assert!(state.toggle(&round(1)), "a round folds like a stage");
    let keys: Vec<NodeKey> = shape(&snap, &state).into_iter().map(|(_, k)| k).collect();
    assert_eq!(keys[1], round(1));
    assert_eq!(
        keys[2],
        round(2),
        "round 1's stage and task are folded away"
    );
}

#[test]
fn only_an_earlier_rounds_rows_are_muted() {
    let (snap, _) = two_round_fixture();
    let rows = run_rows(&snap.runs[0], &[], &TreeState::default(), RunFilter::All);
    let muted: Vec<NodeKey> = (rows.iter())
        .filter(|row| muted_row(&row.kind))
        .map(|row| row.key.clone())
        .collect();
    assert_eq!(muted, vec![round(1), stage(1), task("t1")]);
    let (one, _) = two_stage_fixture();
    let rows = run_rows(&one.runs[0], &[], &TreeState::default(), RunFilter::All);
    assert!(
        rows.iter().all(|row| !earlier_round(&row.kind)),
        "one round"
    );
}
