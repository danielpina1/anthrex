//! Ruling C-24: a stage that is not created yet has no head, and shows the branch it
//! will be created on.

use proto::RunState;

use super::stage_infos;
use crate::run::model::task_branch;
use crate::run::orch::test_support::{run_with, task_toml};

#[test]
fn stages_not_created_before_approval_have_no_head_and_their_own_branch() {
    let mut run = run_with(&[
        task_toml("t1", "S", "[\"crates/a/**\"]", ""),
        task_toml("t2", "S", "[\"crates/b/**\"]", "stage = 2"),
        task_toml("t3", "S", "[\"crates/c/**\"]", "stage = 3"),
    ]);
    run.state = RunState::AwaitingApproval;
    let got: Vec<(u16, String, Option<String>)> = stage_infos(&run)
        .into_iter()
        .map(|s| (s.n, s.branch, s.head))
        .collect();
    let branch = |n: u16| task_branch(&run.id, &format!("stage-{n}"));
    assert_eq!(
        got,
        [
            (1, branch(1), None),
            (2, branch(2), None),
            (3, branch(3), None)
        ]
    );
}

#[test]
fn a_single_stage_run_keeps_its_one_stage_on_integration() {
    let mut run = run_with(&[task_toml("t1", "S", "[\"crates/a/**\"]", "")]);
    run.state = RunState::Running;
    let got = stage_infos(&run);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].branch, run.run_branch());
    assert_eq!(got[0].head.as_deref(), Some(run.run_head.as_str()));
}
