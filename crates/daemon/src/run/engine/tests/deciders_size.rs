//! M8b.13: the size cross-check (M8b decision 19, spec §7.2 rule 5). `Start`, and every
//! accepted edit for the tasks it added or amended, asks one size-check decider about
//! the tasks with scout evidence; a task waiting for it is not runnable. The answer can
//! only raise: S → M re-derives what depends on the size, L blocks as mis-sized.

use proto::{
    BlockReason, DeciderSource, Effort, PlanEdit, RunState, Size, SizeCheckInfo, TaskState,
};

use super::deciders::usage;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::liveness::assert_alive;
use crate::decider::fallback::fallback_decision;
use crate::decider::{DeciderAnswer, DeciderKind, DeciderRequest, Decision, SizeVerdict};
use crate::run::engine::{Effect, OpKind};
use crate::run::model::{OpId, SizeCheckState};

/// The stored profile's onboarding report every evidenced test names.
pub(super) const ONBOARDING: &str = "onboarding-7";

pub(super) fn plan(tasks: &[String]) -> String {
    plan_with(PROFILE, tasks)
}

/// A run of `tasks` whose repository has an onboarding report, started (`yes`: at
/// once, else at the plan gate) with its integration worktree made; the deciders on.
pub(super) fn evidenced(tasks: &[String], yes: bool, config: config::Orchestrator) -> Fixture {
    let mut fx = Fixture::deciding(&plan(tasks), config);
    fx.start_with(yes, |run| run.onboarding_report = Some(ONBOARDING.into()));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(
        op,
        crate::run::engine::OpResult::Worktree { head: BASE.into() },
    );
    fx
}

/// The latest `Decide` op, which must be a size check: its id, task ids and input.
pub(super) fn size_check_op(fx: &Fixture) -> (OpId, Vec<String>, crate::decider::SizeCheckInput) {
    match fx.op("Decide") {
        (
            op,
            OpKind::Decide {
                task_ids,
                request: DeciderRequest::SizeCheck(input),
                ..
            },
        ) => (op, task_ids, input),
        other => panic!("not a size check: {other:?}"),
    }
}

/// The decider's answer: one verdict per `(id, size, reason)`.
pub(super) fn verdicts(v: &[(&str, Size, &str)]) -> Decision {
    Decision {
        kind: DeciderKind::SizeCheck,
        answer: DeciderAnswer::SizeCheck(
            v.iter()
                .map(|(id, size, reason)| SizeVerdict {
                    id: id.to_string(),
                    size: *size,
                    reason: reason.to_string(),
                })
                .collect(),
        ),
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: Some(usage(1)),
        secs: 2,
    }
}

pub(super) fn done_info(fx: &Fixture, id: &str) -> SizeCheckInfo {
    match &fx.task(id).size_check {
        Some(SizeCheckState::Done(info)) => info.clone(),
        other => panic!("{id}'s size check is {other:?}"),
    }
}

pub(super) fn prepared(effects: &[Effect]) -> Vec<String> {
    tasks_of(effects, "PrepareWorktree")
}

#[test]
fn size_check_raises_s_to_m_and_rederives_route_budget_and_review() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], true, Default::default());
    let t1 = fx.task("t1");
    assert_eq!((t1.size, t1.route.effort), (Size::S, Effort::Low));
    let (op, task_ids, input) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t1".to_string()]);
    assert_eq!(input.evidence_refs, vec!["onboarding".to_string()]);
    assert!(input.evidence.is_empty(), "the driver fills the evidence");
    assert_eq!(input.tasks[0].size, Size::S);
    assert_eq!(input.modules, vec!["crates/*".to_string()]);

    let effects = fx.decided(op, verdicts(&[("t1", Size::M, "it touches three files")]));
    let t1 = fx.task("t1");
    assert_eq!(t1.size, Size::M);
    assert_eq!(t1.raised_size, Some(Size::M));
    assert_eq!(t1.route.effort, Effort::Medium, "the plan set no effort");
    assert_eq!(t1.budget, fx.run().limits.budget_m);
    assert_eq!(
        t1.review_level,
        Some(crate::run::model::ReviewLevel::Medium)
    );
    assert!(t1.review_route.is_some());
    assert!(
        t1.notes.contains(
            &"size raised from S to M: decider cross-check (rule 7.2.5): it touches three files"
                .to_string()
        ),
        "{:?}",
        t1.notes
    );
    assert_eq!(
        done_info(&fx, "t1"),
        SizeCheckInfo {
            engine: Size::S,
            decided: Some(Size::M),
            agreed: false,
            reason: "it touches three files".into(),
            source: DeciderSource::Decider,
        }
    );
    assert_eq!(prepared(&effects), vec!["t1"], "dispatched once answered");
    assert_eq!(fx.run().decider_calls, 1);
    assert_eq!(fx.run().decider_usage, usage(1));
    assert_alive(&fx);
}

#[test]
fn a_planner_set_effort_survives_a_raise() {
    let route = "[task.route]\neffort = \"high\"";
    let mut fx = evidenced(&[task("t1", "S", "a", route)], true, Default::default());
    assert_eq!(fx.task("t1").route.effort, Effort::High);
    let (op, ..) = size_check_op(&fx);
    fx.decided(op, verdicts(&[("t1", Size::M, "bigger")]));
    let t1 = fx.task("t1");
    assert_eq!(t1.size, Size::M);
    assert_eq!(t1.route.effort, Effort::High);
}

#[test]
fn an_amend_never_lowers_a_cross_check_raise() {
    let tasks = [
        task("t1", "S", "a", ""),
        task("t2", "S", "b", "deps = [\"t1\"]"),
    ];
    let mut fx = evidenced(&tasks, true, Default::default());
    let (op, task_ids, _) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t1".to_string(), "t2".to_string()]);
    fx.decided(
        op,
        verdicts(&[("t1", Size::S, "one file"), ("t2", Size::M, "two files")]),
    );
    assert_eq!(fx.task("t2").size, Size::M);
    assert_eq!(fx.task("t2").state, TaskState::Pending);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::AmendTask {
            task_id: "t2".into(),
            brief: None,
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: Some(Size::S),
            deps: None,
            stage: None,
            race: None,
            pair: None,
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let t2 = fx.task("t2");
    assert_eq!((t2.size, t2.raised_size), (Size::M, Some(Size::M)));
    // The amended task is checked again, stated at its kept size.
    let (_, task_ids, input) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t2".to_string()]);
    assert_eq!(input.tasks[0].size, Size::M);
}

#[test]
fn size_check_l_blocks_as_mis_sized_without_a_rung() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], true, Default::default());
    let (op, ..) = size_check_op(&fx);
    let effects = fx.decided(op, verdicts(&[("t1", Size::L, "it spans three modules")]));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().expect("blocked");
    assert_eq!(block.reason, BlockReason::MisSized);
    assert_eq!(
        block.text,
        "the size cross-check judged this task L: it spans three modules; split it (rule 7.2.5)"
    );
    assert_eq!((t1.rung, t1.failures), (0, 0));
    assert_eq!(t1.size, Size::S);
    assert_eq!(done_info(&fx, "t1").decided, Some(Size::L));
    assert!(prepared(&effects).is_empty());
    assert_alive(&fx);
}

#[test]
fn size_check_never_lowers_and_records_agreement() {
    let tasks = [task("t1", "M", "a", ""), task("t2", "S", "b", "")];
    let mut fx = evidenced(&tasks, true, Default::default());
    let (op, ..) = size_check_op(&fx);
    fx.decided(
        op,
        verdicts(&[
            ("t1", Size::S, "a one-line change"),
            ("t2", Size::S, "one file"),
        ]),
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.size, t1.raised_size), (Size::M, None));
    assert_eq!(
        done_info(&fx, "t1"),
        SizeCheckInfo {
            engine: Size::M,
            decided: Some(Size::S),
            agreed: false,
            reason: "a one-line change".into(),
            source: DeciderSource::Decider,
        }
    );
    let t2 = done_info(&fx, "t2");
    assert!(t2.agreed);
    assert_eq!(t2.decided, Some(Size::S));
    // The disagreement is in the report and the snapshot; the agreement is not reported.
    let report = crate::run::report::render(fx.run(), fx.now);
    assert!(
        report.contains("Size cross-check: M by the engine, S by the decider: a one-line change"),
        "{report}"
    );
    assert!(!report.contains("one file"), "{report}");
    let snap = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let info = snap.runs[0].tasks[0].size_check.clone();
    assert_eq!(info.map(|i| i.decided), Some(Some(Size::S)));
}

#[test]
fn missing_ids_keep_their_size_as_fallback() {
    let tasks = [task("t1", "S", "a", ""), task("t2", "S", "b", "")];
    let mut fx = evidenced(&tasks, true, Default::default());
    let (op, ..) = size_check_op(&fx);
    fx.decided(op, verdicts(&[("t1", Size::S, "one file")]));
    assert_eq!(
        done_info(&fx, "t2"),
        SizeCheckInfo {
            engine: Size::S,
            decided: None,
            agreed: true,
            reason: "the engine's size is kept".into(),
            source: DeciderSource::Fallback,
        }
    );
    assert_eq!(fx.task("t2").size, Size::S);
    assert_eq!(done_info(&fx, "t1").source, DeciderSource::Decider);
    // A whole fallback records its reason on every task.
    let mut fx = evidenced(&tasks, true, Default::default());
    let (op, _, input) = size_check_op(&fx);
    let reason = "the decider timed out after 90 s".to_string();
    fx.decided(
        op,
        fallback_decision(&DeciderRequest::SizeCheck(input), reason.clone()),
    );
    for id in ["t1", "t2"] {
        let info = done_info(&fx, id);
        assert_eq!((info.decided, info.source), (None, DeciderSource::Fallback));
        assert_eq!(info.reason, reason);
    }
    assert_eq!(fx.run().decider_fallbacks, 1);
}

#[test]
fn a_pending_size_check_blocks_dispatch() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], true, Default::default());
    assert_eq!(fx.run().state, RunState::Running);
    let (op, ..) = size_check_op(&fx);
    assert!(matches!(
        fx.task("t1").size_check,
        Some(SizeCheckState::Pending { .. })
    ));
    assert!(prepared(&fx.log).is_empty(), "{:#?}", fx.log);
    assert_alive(&fx);
    fx.tick();
    assert!(prepared(&fx.log).is_empty());
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    let effects = fx.decided(op, verdicts(&[("t1", Size::S, "one file")]));
    assert_eq!(prepared(&effects), vec!["t1"]);
}

#[test]
fn tasks_without_evidence_skip_the_cross_check() {
    let skipped = "size cross-check skipped: no scout evidence".to_string();
    // No onboarding report: no call at all, and every task is noted.
    let mut fx = Fixture::deciding(&plan(&[task("t1", "S", "a", "")]), Default::default());
    let effects = fx.ready(true);
    assert!(fx.ops("Decide").is_empty());
    assert!(fx.task("t1").notes.contains(&skipped));
    assert_eq!(fx.task("t1").size_check, None);
    assert_eq!(prepared(&effects), vec!["t1"]);
    // A task whose `scout_refs` name no report has no evidence; one without any uses
    // the onboarding report.
    let tasks = [
        task("t1", "S", "a", "scout_refs = [\"api-1\"]"),
        task("t2", "S", "b", ""),
    ];
    let fx = evidenced(&tasks, true, Default::default());
    let (_, task_ids, input) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t2".to_string()]);
    assert_eq!(input.evidence_refs, vec!["onboarding".to_string()]);
    assert!(fx.task("t1").notes.contains(&skipped));
    assert!(!fx.task("t2").notes.contains(&skipped));
    assert_eq!(fx.ops("Decide").len(), 1);
}

#[test]
fn a_run_scout_ref_comes_before_the_onboarding_alias() {
    let tasks = [
        task("t1", "S", "a", "scout_refs = [\"onboarding\", \"api-1\"]"),
        task("t2", "S", "b", "scout_refs = [\"../../etc\"]"),
    ];
    let mut fx = Fixture::deciding(&plan(&tasks), Default::default());
    fx.start_with(true, |run| {
        run.onboarding_report = Some(ONBOARDING.into());
        run.scout_reports = vec!["api-1".into()];
    });
    let (_, task_ids, input) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t1".to_string()]);
    assert_eq!(
        input.evidence_refs,
        vec!["api-1".to_string(), "onboarding".to_string()]
    );
}

#[test]
fn deciders_off_keep_every_size_with_no_op_and_no_note() {
    let mut config = config::Orchestrator::default();
    config.deciders.mode = proto::DeciderMode::Off;
    let fx = evidenced(&[task("t1", "S", "a", "")], true, config);
    assert!(fx.ops("Decide").is_empty());
    let info = done_info(&fx, "t1");
    assert_eq!((info.decided, info.source), (None, DeciderSource::Fallback));
    assert_eq!(info.reason, "deciders are off");
    assert!(
        fx.task("t1")
            .notes
            .iter()
            .all(|n| !n.contains("cross-check"))
    );
    assert_eq!(fx.run().decider_calls, 0);
    assert_eq!(prepared(&fx.log), vec!["t1"]);
}

#[test]
fn an_edit_cross_checks_only_the_touched_tasks() {
    let tasks = [
        task("t1", "S", "a", ""),
        task("t2", "S", "b", "deps = [\"t1\"]"),
    ];
    let mut fx = evidenced(&tasks, true, Default::default());
    let (op, ..) = size_check_op(&fx);
    fx.decided(
        op,
        verdicts(&[("t1", Size::S, "one"), ("t2", Size::S, "one")]),
    );
    let text = plan(&[task("t3", "S", "c", "")]);
    let t3 = crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0);
    let effects = edit(
        &mut fx,
        vec![
            PlanEdit::AddTask { task: t3 },
            PlanEdit::AmendTask {
                task_id: "t2".into(),
                brief: Some("A new brief".into()),
                acceptance: None,
                route: None,
                test_mode: None,
                test_mode_reason: None,
                priority: None,
                size: None,
                deps: None,
                stage: None,
                race: None,
                pair: None,
            },
        ],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let (op, task_ids, input) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t2".to_string(), "t3".to_string()]);
    assert_eq!(input.tasks[0].brief, "A new brief");
    assert_eq!(fx.ops("Decide").len(), 2);
    // t1 is working and keeps its answer; t3 waits for the new one.
    assert!(matches!(
        fx.task("t1").size_check,
        Some(SizeCheckState::Done(_))
    ));
    assert!(prepared(&effects).is_empty());
    let effects = fx.decided(
        op,
        verdicts(&[("t2", Size::S, "one"), ("t3", Size::S, "one")]),
    );
    assert_eq!(prepared(&effects), vec!["t3"]);
}

/// Task 12 review m7: a size check can be queued at the plan gate, but never starts
/// before the run runs. A rejected run drops it, so no `Decide` ever runs beside the
/// `Discard`; an approved run starts it then, its slot wait counted from the approval.
#[test]
fn a_size_check_at_the_gate_starts_on_approval_and_goes_with_a_reject() {
    let tasks = [task("t1", "S", "a", "")];
    let mut fx = evidenced(&tasks, false, Default::default());
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert!(fx.ops("Decide").is_empty());
    assert_eq!(fx.queued_deciders().len(), 1);
    let reply = fx.reply();
    fx.next(crate::run::engine::EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(fx.queued_deciders().is_empty());
    assert!(fx.ops("Decide").is_empty());
    fx.op("Discard");

    let config = config::Orchestrator {
        max_readers: 1,
        ..Default::default()
    };
    let mut fx = evidenced(&tasks, false, config);
    // A second size check at the gate, from an added task.
    let text = plan(&[task("t2", "S", "b", "")]);
    let t2 = crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0);
    edit(&mut fx, vec![PlanEdit::AddTask { task: t2 }]);
    assert_eq!(fx.queued_deciders().len(), 2);
    // Approved long after both were queued: one starts, the other still waits.
    let late = fx.now + 1_000;
    let reply = fx.reply();
    fx.send(
        late,
        crate::run::engine::EventKind::Approve {
            reply,
            run_id: RUN_ID.into(),
        },
    );
    assert_eq!(fx.ops("Decide").len(), 1);
    assert_eq!(fx.queued_deciders().len(), 1);
    assert!(matches!(
        fx.task("t2").size_check,
        Some(SizeCheckState::Pending { .. })
    ));
    assert_alive(&fx);
}

/// Cancelling one task of a batch leaves the others' check queued.
#[test]
fn a_cancelled_task_leaves_the_batch_and_the_rest_is_still_checked() {
    let tasks = [task("t1", "S", "a", ""), task("t2", "S", "b", "")];
    let mut fx = evidenced(&tasks, false, Default::default());
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    let queued = fx.queued_deciders();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].task_ids, vec!["t2".to_string()]);
    match &queued[0].request {
        DeciderRequest::SizeCheck(input) => {
            let ids: Vec<&str> = input.tasks.iter().map(|t| t.id.as_str()).collect();
            assert_eq!(ids, vec!["t2"]);
        }
        other => panic!("{other:?}"),
    }
    fx.approve();
    let (_, task_ids, _) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t2".to_string()]);
    assert_alive(&fx);
}
