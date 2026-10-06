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

/// Milestone 9.5 decision 45 (FU-F21): a stage whose head failed tier 3's executor
/// three times in a row is held (ruling C-18), and its `FullInfo` says so; two failures,
/// or three on an older head, are not a hold.
#[test]
fn a_held_stage_says_so() {
    use crate::run::model::InfraFailures;
    let mut run = run_with(&[task_toml("t1", "S", "[\"crates/a/**\"]", "")]);
    run.state = RunState::Running;
    let head = run.stages[0].head.clone();
    let held_with = |run: &mut crate::run::model::Run, commit: &str, count: u8| {
        run.stages[0].full.infra = Some(InfraFailures {
            commit: commit.to_string(),
            count,
            at: 100,
            line: "executor lost".into(),
        });
        stage_infos(run)[0].full.held
    };
    assert!(!stage_infos(&run)[0].full.held);
    assert!(held_with(&mut run, &head, 3));
    assert!(!held_with(&mut run, &head, 2));
    assert!(!held_with(&mut run, "0123", 3));
}

/// Milestone 9.7 decision 12 (DH §2.3): stage 1 with tier 3 green or red on `H1`, its
/// head since moved on. `pr` names the stage's PR state, when it has one.
fn moved_head(mode: proto::DeliveryMode, pr: Option<proto::PrState>, ok: bool) -> proto::FullState {
    use crate::run::delivery::{PrRecord, StageDelivery};
    use crate::run::model::TierRecord;
    const H1: &str = "1111111111111111111111111111111111111111";
    let mut run = run_with(&[task_toml("t1", "S", "[\"crates/a/**\"]", "")]);
    run.state = RunState::Running;
    assert_ne!(
        stage_infos(&run)[0].head.as_deref(),
        Some(H1),
        "the head moved"
    );
    let full = &mut run.stages[0].full;
    if ok {
        full.green_at = Some(H1.into());
    } else {
        full.red_at = Some(H1.into());
    }
    full.last = Some(TierRecord {
        tier: 3,
        affected: String::new(),
        steps: 1,
        cached: 0,
        ok,
        secs: 30,
        flaky: Vec::new(),
        failing: Vec::new(),
        at: 2_000,
        commit: H1.into(),
    });
    run.delivery.mode = mode;
    run.delivery.stages = vec![StageDelivery {
        pr: pr.map(|state| PrRecord {
            number: 141,
            url: "https://github.com/fake/app/pull/141".into(),
            base: "main".into(),
            opened_at: 3_000,
            pushed_head: H1.into(),
            state,
            merged_at: None,
            merge_commit: None,
            merge_method: None,
            next_poll_at: 0,
            unchanged_views: 0,
            last_view_at: None,
            watermark: Default::default(),
            checks: Vec::new(),
            retargeted_to: None,
            opened_base: None,
            branch_deleted: false,
            confirmed: None,
        }),
        ..StageDelivery::default()
    }];
    stage_infos(&run)[0].full.state
}

/// Milestone 9.7 decision 12 (ruling T9-1): an open PR's stage shows the latest tier-3
/// verdict, from a job on an earlier head than the current one; CI carries later heads.
#[test]
fn an_open_prs_moved_head_shows_the_last_tier_3_verdict() {
    use proto::{DeliveryMode, FullState, PrState};
    let open = Some(PrState::Open);
    assert_eq!(moved_head(DeliveryMode::Pr, open, true), FullState::Green);
    assert_eq!(moved_head(DeliveryMode::Pr, open, false), FullState::Red);
}

#[test]
fn before_its_pr_opens_a_moved_head_reads_none() {
    use proto::{DeliveryMode, FullState, PrState};
    assert_eq!(moved_head(DeliveryMode::Pr, None, true), FullState::None);
    assert_eq!(moved_head(DeliveryMode::Pr, None, false), FullState::None);
    // Only an open PR: a merged one keeps the on-head rule too.
    let merged = Some(PrState::Merged);
    assert_eq!(moved_head(DeliveryMode::Pr, merged, true), FullState::None);
}

#[test]
fn local_mode_keeps_the_on_head_rule() {
    use proto::{DeliveryMode, FullState, PrState};
    let none = FullState::None;
    assert_eq!(moved_head(DeliveryMode::Local, None, true), none);
    assert_eq!(moved_head(DeliveryMode::Local, None, false), none);
    // The mode decides, not a PR record (local mode never opens one).
    let open = Some(PrState::Open);
    assert_eq!(moved_head(DeliveryMode::Local, open, true), none);
}
