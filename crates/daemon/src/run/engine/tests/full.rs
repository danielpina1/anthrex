//! Milestone 9.1 task M9.1.14: tier 3 when the merge queue is idle (decision 17(b)),
//! per stage at completion (decision 19), 9.2's entry `full::request` (decision 17(a)),
//! a lost tier-3 job (decision 29), and M8a's final check kept for an untiered profile
//! (decision 6).

use proto::{PlanEdit, RunState, TaskState};
use serde_json::json;

use super::control::resume;
use super::control_restore::restart;
use super::dispatch::{edit, task_path};
use super::fixture::*;
use super::gates::check_result;
use super::merge::{
    claim, commit, doc_task, head_of, pending, pending_one, start, start_on, to_queue, window_of,
};
use super::tiers::tiered;
use super::turns_fixes::assert_alive;
use crate::run::engine::full::{FullWhy, request};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, ScratchAt};
use crate::run::model::OpId;
use crate::run::slots::Priority;
use crate::run::snapshot::snapshot;
use crate::run::tiers::{Affected, Scope, StepKind, StepOutcome, TierOutcome, TierSpec};

/// The tiered profile with room for three writers.
pub(super) fn profile() -> String {
    tiered().replacen("\n[profile]", "\nmax_writers = 3\n[profile]", 1)
}

/// A tier outcome: tier 3's one `check` step (scope `full`), or a gate job's.
pub(super) fn outcome(tier: u8, failing: &[&str]) -> TierOutcome {
    let ok = failing.is_empty();
    let (scope, affected) = if tier == 3 {
        (Scope::Full, Affected::Full("full suite".into()))
    } else {
        (Scope::Gate, Affected::Modules(Default::default()))
    };
    TierOutcome {
        tier,
        scope,
        affected,
        tree: "7".repeat(40),
        steps: vec![StepOutcome {
            kind: StepKind::Tests,
            command: "cargo test".into(),
            ok,
            code: Some(if ok { 0 } else { 101 }),
            timed_out: false,
            secs: 30,
            cached: false,
            retried: !ok,
            failing: failing.iter().map(|s| s.to_string()).collect(),
            flaky: vec![],
            granted: 4,
        }],
        ok,
        secs: 30,
        tail: if ok { String::new() } else { "FAILED".into() },
        toolchain: None,
        graph_note: None,
    }
}

pub(super) fn tier(result: TierOutcome) -> OpResult {
    OpResult::Tier(Box::new(result))
}

/// `id` claims done, passes tier 1, and its candidate merges at `at` with a green tier
/// 2; its worktree removals complete.
pub(super) fn merge_tiered(fx: &mut Fixture, id: &str, window: u32, at: &str) -> Vec<Effect> {
    claim(fx, id, window, &head_of(id));
    let (op, _) = pending_one(fx, "Tier", Some(id));
    fx.done(op, tier(outcome(1, &[])));
    let (op, _) = pending_one(fx, "MergeCandidate", Some(id));
    let mut effects = fx.done(
        op,
        OpResult::Merged {
            commit: at.into(),
            tier: Some(Box::new(outcome(2, &[]))),
        },
    );
    for (op, _) in pending(fx, "RemoveWorktree", Some(id)) {
        effects.extend(fx.done(op, OpResult::Removed { salvage_ref: None }));
    }
    effects
}

/// The run-level tier jobs among `effects`.
pub(super) fn full_jobs(effects: &[Effect]) -> Vec<(OpId, TierSpec)> {
    ops_in(effects, "Tier")
        .into_iter()
        .filter_map(|(op, kind)| match kind {
            OpKind::Tier(spec) if spec.tier == 3 => Some((op, *spec)),
            _ => None,
        })
        .collect()
}

/// The one pending tier-3 job.
pub(super) fn full_job(fx: &Fixture) -> (OpId, TierSpec) {
    match pending_one(fx, "Tier", None) {
        (op, OpKind::Tier(spec)) => (op, *spec),
        _ => unreachable!(),
    }
}

/// The step at `now + secs`.
pub(super) fn later(fx: &mut Fixture, secs: u64) -> Vec<Effect> {
    fx.send(fx.now + secs, EventKind::Tick)
}

pub(super) fn block(fx: &mut Fixture, id: &str, window: u32) {
    let args = json!({"kind": "question", "reason": "which table?"});
    fx.tool_as(proto::AgentRole::Worker, window, id, "task_blocked", args);
    assert_eq!(fx.task(id).state, TaskState::Blocked);
}

pub(super) fn attention(fx: &Fixture) -> Vec<String> {
    snapshot(&fx.state, fx.now).runs[0].attention.clone()
}

pub(super) fn verify_ok(fx: &mut Fixture) -> Vec<Effect> {
    let (op, _) = pending_one(fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk)
}

/// A running `Multi` run of `tasks` whose stages are created, every worker launched.
fn multi(tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, mut windows) = start_on(&profile(), tasks);
    while let Some((op, _)) = pending(&fx, "CreateStageBranch", None).first().cloned() {
        fx.done(op, OpResult::StageCreated);
    }
    windows.extend(fx.launch_all());
    (fx, windows)
}

#[test]
fn idle_queue_starts_tier3_on_the_lowest_stage_without_green() {
    let tasks = [doc_task("t1", ""), doc_task("t2", ""), doc_task("t3", "")];
    let (mut fx, windows) = start_on(&profile(), &tasks);
    block(&mut fx, "t3", window_of(&windows, "t3"));
    // Idle, but nothing merged yet: the base needs no tier 3 (as M8a's final check).
    assert!(fx.run().queue_idle_since.is_some());
    assert!(full_jobs(&later(&mut fx, 130)).is_empty());
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let since = fx.run().queue_idle_since.expect("the queue is idle");
    let idle = fx.run().limits.testing.full_idle_secs;
    assert_eq!(idle, 120);
    fx.send(since + idle - 1, EventKind::Tick);
    assert!(
        fx.ops("Tier")
            .iter()
            .all(|(_, k)| matches!(k, OpKind::Tier(s) if s.tier == 1))
    );

    let effects = fx.send(since + idle, EventKind::Tick);
    let jobs = full_jobs(&effects);
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    let (op, spec) = jobs[0].clone();
    let full = task_path(".full");
    assert_eq!(full, fx.run().full_path());
    assert_eq!((spec.tier, spec.stage), (3, 1));
    assert_eq!(spec.dir, full);
    assert_eq!(
        spec.scratch,
        Some(ScratchAt {
            root: "/tmp/x".into(),
            commit: commit(1),
            setup: Some("make deps".into()),
        })
    );
    assert_eq!(spec.head, commit(1));
    assert_eq!(spec.priority, Priority::FullIdle);
    assert_eq!(
        spec.env,
        vec![("TARGET".into(), format!("{}/target", full.display()))]
    );
    assert_eq!(fx.run().full_op, Some(op));
    assert_eq!(pending_one(&fx, "Tier", None).0, op);
    // None while one runs.
    assert!(full_jobs(&later(&mut fx, idle * 2)).is_empty());
    assert_alive(&fx);

    // Green: the stage's head has its tier 3, so no second one.
    fx.done(op, tier(outcome(3, &[])));
    let stage = fx.run().stage(1).unwrap().clone();
    assert_eq!(stage.full.green_at, Some(commit(1)));
    assert_eq!(stage.full.last.map(|t| (t.tier, t.ok)), Some((3, true)));
    assert_eq!(fx.run().full_op, None);
    assert!(full_jobs(&later(&mut fx, idle * 2)).is_empty());

    // A merge resets the clock.
    merge_tiered(&mut fx, "t2", window_of(&windows, "t2"), &commit(2));
    let reset = fx.run().queue_idle_since.expect("idle again");
    assert!(reset > since + idle * 4, "{reset} after {since}");
    assert!(full_jobs(&fx.send(reset + idle - 1, EventKind::Tick)).is_empty());
    let jobs = full_jobs(&fx.send(reset + idle, EventKind::Tick));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].1.head, commit(2));
    assert_alive(&fx);

    // Two stages: the lowest without a green tier 3 goes first, then the next.
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", "stage = 2"),
    ];
    let (mut fx, windows) = multi(&tasks);
    block(&mut fx, "t3", window_of(&windows, "t3"));
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    // Task M9.1.17: stage 1's head propagates into stage 2 first.
    super::propagate::land_propagates(&mut fx);
    merge_tiered(&mut fx, "t2", window_of(&windows, "t2"), &commit(2));
    assert_eq!(fx.run().stage_head(1), Some(commit(1).as_str()));
    assert_eq!(fx.run().stage_head(2), Some(commit(2).as_str()));
    let since = fx.run().queue_idle_since.unwrap();
    let jobs = full_jobs(&fx.send(since + idle, EventKind::Tick));
    assert_eq!(
        jobs.iter()
            .map(|(_, s)| (s.stage, s.head.clone()))
            .collect::<Vec<_>>(),
        vec![(1, commit(1))]
    );
    // Still idle: stage 2's starts in the pass after stage 1's green.
    let jobs = full_jobs(&fx.done(jobs[0].0, tier(outcome(3, &[]))));
    assert_eq!(
        jobs.iter()
            .map(|(_, s)| (s.stage, s.head.clone(), s.priority))
            .collect::<Vec<_>>(),
        vec![(2, commit(2), Priority::FullIdle)]
    );
    assert_alive(&fx);
}

#[test]
fn completion_runs_tier3_per_stage_bottom_up() {
    let (mut fx, windows) = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    // Task M9.1.17: stage 1's head propagates into stage 2 first.
    super::propagate::land_propagates(&mut fx);
    merge_tiered(&mut fx, "t2", window_of(&windows, "t2"), &commit(2));
    assert_eq!(fx.run().run_head, commit(2));
    let effects = verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Running);
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");
    let jobs = full_jobs(&effects);
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    let (op, spec) = jobs[0].clone();
    assert_eq!(
        (spec.stage, spec.head.clone(), spec.priority),
        (1, commit(1), Priority::FullStage)
    );
    assert_alive(&fx);

    // After stage 1's green, the next pass verifies again and runs stage 2's.
    fx.done(op, tier(outcome(3, &[])));
    let effects = verify_ok(&mut fx);
    let jobs = full_jobs(&effects);
    assert_eq!(
        jobs.iter()
            .map(|(_, s)| (s.stage, s.head.clone()))
            .collect::<Vec<_>>(),
        vec![(2, commit(2))]
    );
    assert_eq!(fx.run().state, RunState::Running);
    fx.done(jobs[0].0, tier(outcome(3, &[])));
    let effects = verify_ok(&mut fx);
    assert!(full_jobs(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(!fx.run().final_check_failed);
    assert!(effects.contains(&Effect::WriteReport {
        run_id: RUN_ID.into()
    }));

    // A green tier 3 for the current head is reused, not run again.
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", "")]);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    fx.run_mut().stages[0].full.green_at = Some(commit(1));
    let effects = verify_ok(&mut fx);
    assert!(full_jobs(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Complete);
}

/// A tiered run without `single_test` whose one task merged at `commit(1)`, and whose
/// completion's tier 3 came back red on `a::works`.
pub(super) fn red_at_completion() -> Fixture {
    let profile = profile().replace("single_test = \"cargo test -- --exact {test}\"\n", "");
    let (mut fx, windows) = start_on(&profile, &[doc_task("t1", "")]);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    verify_ok(&mut fx);
    let (op, spec) = full_job(&fx);
    assert_eq!(spec.head, commit(1));
    fx.done(op, tier(outcome(3, &["a::works"])));
    fx
}

#[test]
fn red_tier3_at_completion_waits_and_raises_attention() {
    let mut fx = red_at_completion();
    let full = fx.run().stage(1).unwrap().full.clone();
    assert_eq!(full.red_at, Some(commit(1)));
    assert_eq!(full.last.map(|t| t.failing), Some(vec!["a::works".into()]));
    assert_eq!(fx.run().full_op, None);
    // No second tier 3 on that head, no guard, and the run waits.
    let effects = later(&mut fx, 1_000);
    assert!(full_jobs(&effects).is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    assert!(fx.run().pending_ops.is_empty());
    assert_eq!(fx.run().state, RunState::Running);
    let line = "tier 3 red, no single culprit: a::works (stage 1: the profile has no single_test to bisect with)";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:?}",
        attention(&fx)
    );
    assert_alive(&fx);

    // The head moves (a fix merged): tier 3 runs on the new head (here the idle pass,
    // whose clock ran out while the run waited, starts it first).
    set_stage_head(fx.run_mut(), 1, &commit(2));
    fx.tick();
    let (_, spec) = full_job(&fx);
    assert_eq!(spec.head, commit(2));

    // The `finish` edit: the red stage no longer holds completion.
    let mut fx = red_at_completion();
    edit(&mut fx, vec![PlanEdit::Finish]);
    let effects = verify_ok(&mut fx);
    assert!(full_jobs(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(fx.run().final_check_failed);
    let lines = attention(&fx);
    assert!(
        lines.contains(&"tier 3 red on stage 1: a::works".to_string()),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("final check failed")),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("no single culprit")),
        "{lines:?}"
    );
}

/// Pinning (decision 6): an untiered profile's completion is M8a's final check, and it
/// never starts a tier-3 job, however long its queue is idle.
#[test]
fn untiered_completion_is_m8a_final_check() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    block(&mut fx, "t2", window_of(&windows, "t2"));
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    super::merge::merge(&mut fx, "t1", &commit(1));
    let effects = later(&mut fx, 10_000);
    assert!(ops_in(&effects, "Tier").is_empty());
    assert_eq!(fx.run().queue_idle_since, None);
    assert_eq!(fx.run().full_op, None);
    assert!(fx.ops("Tier").is_empty(), "{:#?}", fx.log);

    // Completion with the last candidate green: no final check.
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    super::merge::merge(&mut fx, "t1", &commit(1));
    let effects = verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(ops_in(&effects, "Check").is_empty() && ops_in(&effects, "Tier").is_empty());

    // A head no candidate covered: M8a's final check, red, and complete anyway.
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    super::merge::merge(&mut fx, "t1", &commit(1));
    fx.run_mut().last_green_candidate = None;
    let effects = verify_ok(&mut fx);
    assert!(ops_in(&effects, "Tier").is_empty());
    let (op, kind) = pending_one(&fx, "Check", None);
    assert!(matches!(kind, OpKind::Check { scratch: None, .. }));
    fx.done(op, check_result(false));
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(fx.run().final_check_failed);
    assert!(attention(&fx).contains(&"final check failed on the run head".to_string()));
}

#[test]
fn full_request_is_refused_while_one_runs_and_queued_for_deliver() {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", ""), doc_task("t2", "")]);
    block(&mut fx, "t2", window_of(&windows, "t2"));
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let since = fx.run().queue_idle_since.unwrap();
    let jobs = full_jobs(&fx.send(since + 120, EventKind::Tick));
    let (idle_op, _) = jobs[0].clone();
    let now = fx.now;
    let mut effects = Vec::new();
    // An idle job is in flight.
    assert!(!request(
        fx.run_mut(),
        1,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    assert!(effects.is_empty());
    fx.done(idle_op, tier(outcome(3, &[])));
    // The head is green already.
    assert!(!request(
        fx.run_mut(),
        1,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    assert!(effects.is_empty());
    // A stage that is not created.
    assert!(!request(
        fx.run_mut(),
        2,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    assert!(effects.is_empty());
    // The head moved: the job starts at `FullStage`, the queue class of delivery.
    set_stage_head(fx.run_mut(), 1, &commit(2));
    assert!(request(
        fx.run_mut(),
        1,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    let jobs = full_jobs(&effects);
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    let (op, spec) = jobs[0].clone();
    assert_eq!(
        (spec.stage, spec.head.clone(), spec.priority),
        (1, commit(2), Priority::FullStage)
    );
    assert_eq!(fx.run().full_op, Some(op));
    assert!(fx.run().pending_ops.contains_key(&op));
    let mut again = Vec::new();
    assert!(!request(fx.run_mut(), 1, FullWhy::Deliver, now, &mut again));
    assert!(again.is_empty());
}

/// Milestone 9.2 decision 19 (task M9.2.7) changed this 9.1 pin for `Deliver` only: an
/// untiered profile's tier 3 before a PR opens is its `check`, one step. Idle and
/// completion still do nothing for it (9.1 decision 6).
#[test]
fn full_request_runs_only_check_for_an_untiered_profile_before_delivery() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    block(&mut fx, "t2", window_of(&windows, "t2"));
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    super::merge::merge(&mut fx, "t1", &commit(1));
    let before = fx.run().clone();
    let mut effects = Vec::new();
    let now = fx.now;
    for why in [FullWhy::Idle, FullWhy::Completion] {
        assert!(!request(fx.run_mut(), 1, why, now, &mut effects));
        assert!(effects.is_empty());
        assert_eq!(fx.run(), &before);
    }
    assert!(request(
        fx.run_mut(),
        1,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    let jobs = full_jobs(&effects);
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    let spec = &jobs[0].1;
    assert_eq!(
        (spec.head.as_str(), spec.check.as_deref()),
        (commit(1).as_str(), Some("cargo test"))
    );
    assert!(!spec.profile.is_tiered() && spec.cache.is_none());
}

#[test]
fn a_lost_tier3_is_issued_again_after_the_restart() {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", ""), doc_task("t2", "")]);
    block(&mut fx, "t2", window_of(&windows, "t2"));
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let since = fx.run().queue_idle_since.unwrap();
    let (lost, _) = full_jobs(&fx.send(since + 120, EventKind::Tick))[0].clone();
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().full_op, None);
    let effects = resume(&mut fx);
    let jobs = full_jobs(&effects);
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    assert_ne!(jobs[0].0, lost);
    assert_eq!(fx.run().full_op, Some(jobs[0].0));
}
