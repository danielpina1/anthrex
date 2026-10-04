//! Milestone 9.2 task M9.2.7: a stage's pull request opens only when the stage is ready
//! (decision 19), in order, after tier 3 is green on its head (9.1's `full::request`,
//! or an untiered profile's `check`); its head is pushed fast-forward, then the PR is
//! created against the base branch or the nearest lower open stage (decision 20). An
//! empty stage is skipped; a PR found already open after a restart is adopted; a host
//! op lost in a restart is emitted again (decision 10). Local mode emits no host op.
//! Host answers are scripted `OpResult::Host` values: no host runs here.

use proto::{DeliveryMode, PlanEdit, PrState, RunState, TaskState};

use super::control::resume;
use super::control_restore::restart;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{attention, full_job, full_jobs, merge_tiered, outcome, profile, tier};
use super::merge::{commit, config, doc_task, merge, pending, start_on, to_queue, window_of};
use super::propagate::{land_propagates, stages_on};
use crate::host::{HostRepo, PrRef, PushOutcome};
use crate::run::delivery::body::{pr_body, pr_title};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::delivery::DeliveryRequest;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::{OpId, Run};
use crate::run::slots::Priority;

/// The repository preflight would have frozen into the run (decision 17).
pub(super) fn repo() -> HostRepo {
    HostRepo {
        host: "github.com".into(),
        owner: "fake".into(),
        name: "app".into(),
        remote: "origin".into(),
        root: "/tmp/x".into(),
    }
}

/// What a `pr`-mode start freezes into the run (decisions 3, 17 and 25).
pub(super) fn pr_mode(run: &mut Run) {
    run.delivery.mode = DeliveryMode::Pr;
    run.delivery.repo = Some(repo());
    run.delivery.watching = true;
    run.delivery.poll_base_secs = run.delivery.limits.poll_secs;
}

/// A running `pr`-mode run of `tasks` on `profile`, every stage created that can be
/// and every dispatched worker launched.
pub(super) fn pr_on(profile: &str, tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    let mut fx = Fixture::with_config(&plan_with(profile, tasks), config());
    fx.start_with(true, pr_mode);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    while let Some((op, _)) = pending(&fx, "CreateStageBranch", None).first().cloned() {
        fx.done(op, OpResult::StageCreated);
    }
    let windows = fx.launch_all();
    (fx, windows)
}

/// `anthrex/<run>/stage-<n>`, the remote branch of stage `n` in every layout.
pub(super) fn remote_branch(n: u16) -> String {
    format!("anthrex/{RUN_ID}/stage-{n}")
}

pub(super) fn url(number: u64) -> String {
    format!("https://github.com/fake/app/pull/{number}")
}

/// The pending host ops, oldest first.
pub(super) fn host_ops(fx: &Fixture) -> Vec<(OpId, HostOp)> {
    pending(fx, "Host", None)
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::Host { op: host, repo: r } => {
                let frozen = fx.run().delivery.repo.as_ref();
                assert_eq!(Some(&r), frozen, "the run's frozen repository");
                (op, host)
            }
            _ => unreachable!(),
        })
        .collect()
}

/// The one pending host op.
pub(super) fn host_op(fx: &Fixture) -> (OpId, HostOp) {
    let ops = host_ops(fx);
    assert_eq!(ops.len(), 1, "one pending host op: {ops:#?}");
    ops[0].clone()
}

/// Every host op among `effects`.
pub(super) fn host_ops_in(effects: &[Effect]) -> Vec<HostOp> {
    ops_in(effects, "Host")
        .into_iter()
        .map(|(_, kind)| match kind {
            OpKind::Host { op, .. } => op,
            _ => unreachable!(),
        })
        .collect()
}

pub(super) fn answer(fx: &mut Fixture, op: OpId, result: HostResult) -> Vec<Effect> {
    fx.done(op, OpResult::Host(result))
}

pub(super) fn opened(number: u64, existed: bool) -> HostResult {
    HostResult::PrOpened(PrRef {
        number,
        url: url(number),
        state: PrState::Open,
        existed,
        base: None,
    })
}

/// Answers the pending tier-3 job of stage `n` green.
pub(super) fn green(fx: &mut Fixture, n: u16) -> Vec<Effect> {
    let (op, spec) = full_job(fx);
    assert_eq!(spec.stage, n, "tier 3 runs on stage {n}");
    fx.done(op, tier(outcome(3, &[])))
}

/// Answers the pending push of stage `n` (asserting it pushes its head) and the PR it
/// opens with `number`; returns the `OpenPr`.
pub(super) fn open_stage(fx: &mut Fixture, n: u16, number: u64) -> HostOp {
    let (op, push) = host_op(fx);
    let head = fx.run().stage_head(n).unwrap().to_string();
    assert_eq!(
        push,
        HostOp::Push {
            stage: n,
            sha: head
        }
    );
    answer(fx, op, HostResult::Pushed(PushOutcome::Pushed));
    let (op, open) = host_op(fx);
    assert!(
        matches!(&open, HostOp::OpenPr { stage, .. } if *stage == n),
        "{open:?}"
    );
    answer(fx, op, opened(number, false));
    open
}

pub(super) fn deliver(fx: &mut Fixture, stage: u16) -> Result<String, String> {
    let reply = fx.reply();
    let request = DeliveryRequest::Deliver {
        reply,
        run_id: RUN_ID.into(),
        stage,
    };
    let effects = fx.next(EventKind::Delivery(request));
    let mut out = replies(&effects);
    assert_eq!(out.len(), 1, "{effects:#?}");
    out.remove(0)
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

/// `t1` merges into a running `Single` run of `profile` at `commit(1)`.
fn single(profile: &str) -> Fixture {
    let (mut fx, windows) = pr_on(profile, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    fx
}

#[test]
fn stage_opens_only_when_ready_and_in_order() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", ""),
    ];
    let (mut fx, windows) = pr_on(&profile_with("max_writers = 3"), &tasks);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    // t3 is still working: stage 1 is not ready, so no tier 3 and nothing is pushed.
    assert!(full_jobs(&fx.log).is_empty(), "{:#?}", fx.log);
    assert!(host_ops(&fx).is_empty());
    let unfinished = "stage 1 is not ready: 1 of its tasks are not finished";
    assert_eq!(deliver(&mut fx, 1), Err(unfinished.into()));
    land_propagates(&mut fx);

    // Stage 2's tasks are merged, but stage 1 has no PR yet.
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    let lower = "stage 2 is not ready: stage 1's PR is not open";
    assert_eq!(deliver(&mut fx, 2), Err(lower.into()));
    // A due base sync holds a stage's queue (decision 33's item).
    fx.run_mut().delivery.base_sync_due.insert(2, commit(8));
    let busy = "stage 2 is not ready: its merge queue is busy";
    assert_eq!(deliver(&mut fx, 2), Err(busy.into()));
    fx.run_mut().delivery.base_sync_due.clear();

    // t3 merges: stage 1 is ready, and its head flows into stage 2 first.
    to_queue(&mut fx, "t3", window_of(&windows, "t3"));
    merge(&mut fx, "t3", &commit(3));
    assert_eq!(fx.run().stage_head(1), Some(commit(3).as_str()));
    assert_eq!(full_job(&fx).1.head, commit(3), "tier 3 on stage 1's head");
    assert!(
        !pending(&fx, "Propagate", None).is_empty(),
        "stage 2 is due a propagate"
    );
    assert_eq!(
        deliver(&mut fx, 2),
        Err(busy.into()),
        "a propagate in flight"
    );
    assert!(host_ops(&fx).is_empty(), "nothing is pushed before tier 3");
    green(&mut fx, 1);
    let open = open_stage(&mut fx, 1, 11);
    assert!(matches!(open, HostOp::OpenPr { ref base, .. } if base == "main"));
    assert!(host_ops(&fx).is_empty(), "stage 2 waits for its propagate");

    land_propagates(&mut fx);
    let head2 = fx.run().stage_head(2).unwrap().to_string();
    assert_eq!(full_job(&fx).1.head, head2, "tier 3 on stage 2's new head");
    green(&mut fx, 2);
    let open = open_stage(&mut fx, 2, 12);
    assert!(matches!(open, HostOp::OpenPr { ref base, .. } if *base == remote_branch(1)));
    let numbers: Vec<u64> = (1..=2)
        .map(|n| fx.run().delivery.pr(n).unwrap().number)
        .collect();
    assert_eq!(numbers, vec![11, 12]);
}

#[test]
fn ready_calls_full_request_before_opening() {
    let (mut fx, windows) = pr_on(&profile(), &[doc_task("t1", "")]);
    let effects = merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    // The same step that merged t1 asks 9.1 for tier 3 on the head, ahead of the idle
    // trigger, at "before a PR opens" priority; nothing is pushed.
    let jobs = full_jobs(&effects);
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    let (_, spec) = &jobs[0];
    assert_eq!((spec.stage, spec.head.as_str()), (1, commit(1).as_str()));
    assert_eq!(spec.priority, Priority::FullStage);
    let line = format!("stage 1: running tier 3 at {} (delivery)", &commit(1)[..7]);
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    assert!(host_ops_in(&effects).is_empty());
    assert!(host_ops(&fx).is_empty());

    let effects = green(&mut fx, 1);
    assert_eq!(
        host_ops_in(&effects),
        vec![HostOp::Push {
            stage: 1,
            sha: commit(1)
        }]
    );
    let (op, _) = host_op(&fx);
    let effects = answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    let ops = host_ops_in(&effects);
    assert!(
        matches!(ops.as_slice(), [HostOp::OpenPr { stage: 1, .. }]),
        "{ops:#?}"
    );
}

#[test]
fn untiered_profile_runs_check_before_opening() {
    let mut fx = single(PROFILE);
    // Decision 19: the profile's `check` is the one step, as a tier-3 job.
    let (op, spec) = full_job(&fx);
    assert_eq!((spec.tier, spec.stage), (3, 1));
    assert_eq!(spec.head, commit(1));
    assert_eq!(spec.check.as_deref(), Some("cargo test"));
    assert!(!spec.profile.is_tiered());
    assert_eq!(spec.cache, None, "no cache");
    assert_eq!(spec.priority, Priority::FullStage);
    assert_eq!(spec.dir, fx.run().full_path());
    assert!(host_ops(&fx).is_empty());

    // Red: no bisect for an untiered profile, and the PR waits for green.
    let effects = fx.done(op, tier(outcome(3, &["it_breaks"])));
    assert!(ops_in(&effects, "TestAt").is_empty(), "never bisected");
    assert!(host_ops(&fx).is_empty(), "no PR on a red head");
    let red = "tier 3 red, no single culprit: it_breaks (stage 1: the profile is untiered, so nothing is bisected)";
    assert!(
        attention(&fx).contains(&red.to_string()),
        "{:?}",
        attention(&fx)
    );
    assert!(
        full_jobs(&fx.tick()).is_empty(),
        "a red head is not run again"
    );

    // A fix moves the head: tier 3 runs on it, and green opens the PR.
    set_stage_head(fx.run_mut(), 1, &commit(9));
    fx.tick();
    assert_eq!(full_job(&fx).1.head, commit(9));
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 3);
    assert_eq!(fx.run().delivery.pr(1).unwrap().pushed_head, commit(9));
}

#[test]
fn a_profile_without_check_opens_unverified() {
    let profile = PROFILE.replace("check = \"cargo test\"\n", "");
    let task = task_toml(
        "t1",
        "S",
        "[\"docs/t1/**\"]",
        "test_mode = \"none\"\ntest_mode_reason = \"docs only\"",
    );
    let (mut fx, windows) = pr_on(&profile, &[task]);
    merged_unchecked(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    assert!(full_jobs(&fx.log).is_empty(), "nothing to run");
    let open = open_stage(&mut fx, 1, 4);
    let HostOp::OpenPr { body, .. } = open else {
        unreachable!()
    };
    assert!(
        body.contains("not run (the profile has no check, so the run is unverified)"),
        "{body}"
    );
}

/// A task with no check claims done; it then stands merged at `at` (the fixture's
/// stand-in for its review and the merge queue).
fn merged_unchecked(fx: &mut Fixture, id: &str, window: u32, at: &str) {
    super::merge::claim(fx, id, window, &super::merge::head_of(id));
    assert_eq!(fx.task(id).state, TaskState::Review);
    fx.merge(id, at);
}

#[test]
fn empty_stage_is_skipped_and_the_next_is_based_lower() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", "stage = 3"),
    ];
    let (mut fx, windows) = stages_on_pr(&tasks);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t2".into(),
        }],
    );
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    to_queue(&mut fx, "t3", window_of(&windows, "t3"));
    land_propagates(&mut fx);
    land_propagates(&mut fx);
    merge(&mut fx, "t3", &commit(3));
    land_propagates(&mut fx);
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 21);

    // Stage 2 merged nothing: it opens no PR (GitHub refuses one without commits).
    assert!(fx.run().delivery.stage(2).unwrap().skipped);
    assert_eq!(fx.run().delivery.pr(2), None);
    assert!(logged(&fx, "stage 2: skipped (no changes)"));
    let info = crate::run::delivery::snapshot::delivery_info(fx.run()).unwrap();
    assert_eq!(info.skipped_stages, vec![2]);
    let skipped = "stage 2 has no changes, so it opens no pull request";
    assert_eq!(deliver(&mut fx, 2), Err(skipped.into()));

    // Stage 3 is based on stage 1, the nearest lower stage with a PR.
    green(&mut fx, 3);
    let open = open_stage(&mut fx, 3, 22);
    let HostOp::OpenPr { base, body, .. } = open else {
        unreachable!()
    };
    assert_eq!(base, remote_branch(1));
    assert!(body.contains("**Stack:** based on stage 1, #21"), "{body}");
    assert_eq!(fx.run().delivery.pr(3).unwrap().base, remote_branch(1));
}

/// Three stages in `pr` mode, every stage created and every worker launched.
fn stages_on_pr(tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, windows) = stages_on(PROFILE, tasks);
    pr_mode(fx.run_mut());
    fx.tick();
    (fx, windows)
}

#[test]
fn open_pushes_then_creates_with_the_right_base_title_and_body() {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    green(&mut fx, 1);
    let (op, push) = host_op(&fx);
    assert_eq!(
        push,
        HostOp::Push {
            stage: 1,
            sha: commit(1)
        }
    );
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::UpToDate));
    let (op, open) = host_op(&fx);
    let run = fx.run();
    let expected = HostOp::OpenPr {
        stage: 1,
        base: "main".into(),
        head: remote_branch(1),
        title: pr_title(run, 1),
        body: pr_body(run, 1),
    };
    assert_eq!(open, expected);
    let HostOp::OpenPr { title, body, .. } = open else {
        unreachable!()
    };
    assert!(
        title.starts_with(&format!("[anthrex r{H4} 1/2] ")),
        "{title}"
    );
    let marker = format!("<!-- anthrex:pr {RUN_ID} stage 1 -->\n");
    assert!(body.starts_with(&marker), "{body}");
    assert!(body.contains("**Stack:** based on the base branch main"));
    let at = fx.now + 1;
    answer(&mut fx, op, opened(7, false));
    let pr = fx.run().delivery.pr(1).unwrap().clone();
    assert_eq!((pr.number, pr.url.as_str()), (7, url(7).as_str()));
    assert_eq!((pr.base.as_str(), pr.state), ("main", PrState::Open));
    assert_eq!((pr.pushed_head, pr.opened_at), (commit(1), at));
    assert!(logged(&fx, &format!("stage 1: PR #7 opened: {}", url(7))));

    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    green(&mut fx, 2);
    let open = open_stage(&mut fx, 2, 8);
    let HostOp::OpenPr {
        base, head, body, ..
    } = open
    else {
        unreachable!()
    };
    assert_eq!((base, head), (remote_branch(1), remote_branch(2)));
    assert!(body.contains("**Stack:** based on stage 1, #7"), "{body}");
}

#[test]
fn a_single_run_pushes_its_integration_head_to_stage_1() {
    let mut fx = single(PROFILE);
    assert_eq!(
        fx.run().stage_branch(1),
        format!("anthrex/{RUN_ID}/integration")
    );
    green(&mut fx, 1);
    let open = open_stage(&mut fx, 1, 5);
    let HostOp::OpenPr { head, title, .. } = open else {
        unreachable!()
    };
    assert_eq!(
        head,
        remote_branch(1),
        "the remote branch is always stage-1"
    );
    assert!(
        title.starts_with(&format!("[anthrex r{H4} 1/1] ")),
        "{title}"
    );
    assert_eq!(
        fx.run().delivery.pr(1).unwrap().pushed_head,
        fx.run().run_head
    );
}

#[test]
fn the_pr_records_the_head_its_push_sent() {
    let mut fx = single(PROFILE);
    green(&mut fx, 1);
    let (op, _) = host_op(&fx);
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    let (op, _) = host_op(&fx);
    // A fix lands while the PR is being created: the PR still holds the pushed head,
    // so the newer one is pushed as an update later (decision 28).
    set_stage_head(fx.run_mut(), 1, &commit(9));
    answer(&mut fx, op, opened(5, false));
    let run = fx.run();
    assert_eq!(run.delivery.pr(1).unwrap().pushed_head, commit(1));
    assert_eq!(run.delivery.stage(1).unwrap().pushed, None, "consumed");
}

#[test]
fn existing_pr_after_a_restart_is_adopted() {
    let mut fx = single(PROFILE);
    green(&mut fx, 1);
    // A restart between the push and its answer: the push is issued again.
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    assert!(host_ops(&fx).is_empty(), "the lost push is dropped");
    resume(&mut fx);
    let (op, push) = host_op(&fx);
    assert!(matches!(push, HostOp::Push { stage: 1, .. }));
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    // A restart between the PR's creation and its record: the push is idempotent
    // (`UpToDate`), and `open_pr` finds the PR it made (`PrRef.existed`).
    assert!(matches!(host_op(&fx).1, HostOp::OpenPr { .. }));
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    let (op, push) = host_op(&fx);
    assert!(matches!(push, HostOp::Push { stage: 1, .. }), "{push:?}");
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::UpToDate));
    let (op, open) = host_op(&fx);
    assert!(matches!(open, HostOp::OpenPr { stage: 1, .. }));
    answer(&mut fx, op, opened(6, true));
    let pr = fx.run().delivery.pr(1).unwrap();
    assert_eq!((pr.number, pr.state), (6, PrState::Open));
    let line = format!("stage 1: PR #6 already existed; adopted: {}", url(6));
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    // Nothing more is opened.
    assert!(host_ops_in(&fx.tick()).is_empty());
    assert!(host_ops(&fx).is_empty());
}

#[test]
fn host_ops_reconcile_as_not_started_and_are_emitted_again_after_restore() {
    // Decision 10's row: a host op with an intent and no result was not started.
    use crate::run::journal::JournalLine;
    use crate::run::model::PendingOp;
    use crate::run::reconcile::{Reconciled, reconcile};
    let mut run = crate::run::test_support::run_ok(crate::run::test_support::EXAMPLE_PLAN);
    let kind = OpKind::Host {
        repo: repo(),
        op: HostOp::Push {
            stage: 1,
            sha: commit(1),
        },
    };
    run.pending_ops.insert(
        4,
        PendingOp {
            op: 4,
            task_id: None,
            kind: kind.clone(),
            lane: None,
        },
    );
    let journal = vec![JournalLine::Intent { op: 4, kind }];
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let timeout = std::time::Duration::from_secs(1);
    let out = reconcile(git, &run, &journal, &[], timeout);
    assert_eq!(out.ops, vec![(4, Reconciled::NotStarted)]);

    // The engine drops it at restore and the delivery pass emits it again once the
    // run runs: one op, never two.
    let mut fx = single(PROFILE);
    green(&mut fx, 1);
    let (lost, push) = host_op(&fx);
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    let (again, push_again) = host_op(&fx);
    assert_eq!(push_again, push);
    assert_ne!(again, lost, "a new op id");
    fx.tick();
    assert_eq!(host_ops(&fx).len(), 1, "never two in flight for a stage");
    // The lost op's late answer is stale and ignored.
    answer(&mut fx, lost, HostResult::Pushed(PushOutcome::Pushed));
    assert_eq!(host_op(&fx).0, again);
}

#[test]
fn local_mode_emits_no_host_op() {
    // Pinning over 9.1's local scenarios: three stages with propagates, and an
    // untiered single-stage run, to completion.
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", "stage = 3"),
    ];
    let (mut fx, windows) = stages_on(PROFILE, &tasks);
    for (n, id) in ["t1", "t2", "t3"].iter().enumerate() {
        to_queue(&mut fx, id, window_of(&windows, id));
        merge(&mut fx, id, &commit(n as u32 + 1));
        land_propagates(&mut fx);
        land_propagates(&mut fx);
    }
    assert_eq!(fx.run().delivery.mode, DeliveryMode::Local);
    assert!(ops_in(&fx.log, "Host").is_empty());
    assert!(
        full_jobs(&fx.log).is_empty(),
        "an untiered local run runs no tier 3"
    );

    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    let (op, _) = pending(&fx, "VerifyRefs", None)[0].clone();
    fx.done(op, OpResult::RefsOk);
    assert!(
        full_jobs(&fx.log).is_empty(),
        "M8a's final check, not tier 3"
    );
    assert!(ops_in(&fx.log, "Host").is_empty());
    assert_eq!(fx.run().delivery, Default::default());
}
