//! M8b.14: a fast-path run in the engine (decisions 19, 24 and 25): no plan gate, no size
//! cross-check (triage sized its task), and `run promote` recorded once and nothing else.

use proto::{
    DeciderSource, RunPath, RunState, Scale, Size, SizeCheckInfo, TaskKind, TaskState, TriageInfo,
};

use super::dispatch::replies;
use super::fixture::*;
use crate::run::engine::{Effect, EventKind, OpResult};
use crate::run::model::{Run, SizeCheckState};
use crate::run::snapshot::snapshot;
use crate::run::triage::mark_fast;

fn triage() -> TriageInfo {
    TriageInfo {
        kinds: vec![TaskKind::Code],
        scale: Scale::Single,
        path: RunPath::Fast,
        reason: "one small change".into(),
        source: DeciderSource::Decider,
        fallback_reason: None,
        at: 1_000,
    }
}

/// A one-task run with the deciders on and an onboarding report (so a plan run's task
/// would be cross-checked), started without `--yes` and marked `fast` when `fast`.
fn started(fast: bool) -> Fixture {
    let mut fx = Fixture::deciding(
        &plan_with(PROFILE, &[task("t1", "S", "auth", "")]),
        config::Orchestrator::default(),
    );
    fx.start_with(false, |run: &mut Run| {
        run.onboarding_report = Some("onboarding-7".into());
        if fast {
            mark_fast(run, triage(), None);
        }
    });
    fx
}

fn promote(fx: &mut Fixture, run_id: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Promote {
        reply,
        run_id: run_id.into(),
    })
}

/// Only replies, saves and pushes: no op, window, delivery or report.
fn only_bookkeeping(effects: &[Effect]) {
    for e in effects {
        assert!(
            matches!(
                e,
                Effect::Reply { .. } | Effect::Persist { .. } | Effect::Publish { .. }
            ),
            "unexpected effect {e:?}"
        );
    }
}

#[test]
fn a_fast_path_run_starts_running_without_a_gate() {
    let mut fx = started(true);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.run().approved_by.as_deref(), Some("fast path"));
    assert_eq!(fx.run().path, Some(RunPath::Fast));
    assert!(
        fx.run()
            .log
            .iter()
            .any(|e| e.text == "started on the fast path; no plan gate"),
        "{:?}",
        fx.run().log
    );
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    // Dispatched at once, as a `--yes` run would be.
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    let info = &snapshot(&fx.state, fx.now).runs[0];
    assert_eq!(info.path, Some(RunPath::Fast));
    assert_eq!(info.triage, Some(triage()));
    // The control: the same run from a plan waits at the gate.
    let plan = started(false);
    assert_eq!(plan.run().state, RunState::AwaitingApproval);
    assert_eq!(plan.run().path, None);
}

#[test]
fn a_fast_path_task_skips_the_cross_check() {
    let mut fx = started(true);
    assert!(fx.ops("Decide").is_empty());
    assert!(fx.queued_deciders().is_empty());
    assert_eq!(
        fx.task("t1").size_check,
        Some(SizeCheckState::Done(SizeCheckInfo {
            engine: Size::S,
            decided: None,
            agreed: true,
            reason: "sized by triage".into(),
            source: DeciderSource::Fallback,
        }))
    );
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert!(fx.ops("Decide").is_empty());
    assert_eq!(
        snapshot(&fx.state, fx.now).runs[0].tasks[0]
            .size_check
            .as_ref()
            .map(|s| s.reason.as_str()),
        Some("sized by triage")
    );
    // The control: the same run from a plan is cross-checked.
    let plan = started(false);
    assert!(matches!(
        plan.task("t1").size_check,
        Some(SizeCheckState::Pending { .. })
    ));
}

#[test]
fn promote_records_intent_once() {
    let mut fx = started(true);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (tasks, ops) = (fx.run().tasks.clone(), fx.run().pending_ops.clone());
    fx.now = 3_600 * 13 + 60 * 7 - 1; // the step is one second later: 13:07
    let first = promote(&mut fx, RUN_ID);
    assert_eq!(
        replies(&first),
        vec![Ok(format!(
            "recorded: run {RUN_ID} is marked for promotion to a planned run. Until the orchestrator exists (milestone 9) nothing else changes: the fast-path task continues and the run finishes as a fast-path run."
        ))]
    );
    only_bookkeeping(&first);
    assert!(first.iter().any(|e| matches!(e, Effect::Persist { .. })));
    assert!(first.iter().any(|e| matches!(e, Effect::Publish { .. })));
    let at = 3_600 * 13 + 60 * 7;
    assert_eq!(fx.run().promote_requested_at, Some(at));
    assert_eq!(
        fx.run().log.last().map(|e| (e.at, e.text.as_str())),
        Some((at, "promotion to a planned run requested by the user"))
    );
    let info = &snapshot(&fx.state, fx.now).runs[0];
    assert_eq!(info.promote_requested_at, Some(at));
    assert!(
        info.attention.contains(
            &"promotion requested at 13:07; it takes effect when the orchestrator exists (milestone 9)"
                .to_string()
        ),
        "{:?}",
        info.attention
    );
    assert_eq!(fx.run().tasks, tasks);
    assert_eq!(fx.run().pending_ops, ops);
    assert_eq!(fx.run().path, Some(RunPath::Fast));

    let before = fx.run().clone();
    fx.now += 600;
    let second = promote(&mut fx, RUN_ID);
    assert_eq!(
        replies(&second),
        vec![Ok(format!(
            "run {RUN_ID} was already marked for promotion at 13:07"
        ))]
    );
    only_bookkeeping(&second);
    assert_eq!(*fx.run(), before, "a second promote changes nothing");
    assert!(!second.iter().any(|e| matches!(e, Effect::Persist { .. })));
}

#[test]
fn promote_refuses_plan_runs_and_terminal_runs() {
    let mut plan = started(false);
    let effects = promote(&mut plan, RUN_ID);
    assert_eq!(
        replies(&effects),
        vec![Err(format!("run {RUN_ID} is not a fast-path run"))]
    );
    assert_eq!(plan.run().promote_requested_at, None);
    only_bookkeeping(&effects);

    // Review m3: a `complete` run waits for accept; nothing is left to promote.
    for state in [
        RunState::Complete,
        RunState::Failed,
        RunState::Accepted,
        RunState::Discarded,
    ] {
        let mut fx = started(true);
        fx.run_mut().state = state;
        let effects = promote(&mut fx, RUN_ID);
        assert_eq!(
            replies(&effects),
            vec![Err(format!("run {RUN_ID} is {}", state.label()))]
        );
        assert_eq!(fx.run().promote_requested_at, None);
    }

    let mut fx = started(true);
    assert_eq!(
        replies(&promote(&mut fx, "nope")),
        vec![Err("unknown run nope".to_string())]
    );
}

/// Review I1: the engine's own barrier. A `path == Fast` run with a hub task, an L task
/// or more than one task is refused, and nothing is created: no run, op or window.
#[test]
fn a_fast_path_start_refuses_hub_l_and_many_tasks() {
    let cases: [(Vec<String>, Option<Size>, &str); 3] = [
        (
            vec![task("t1", "S", "proto", "")],
            None,
            "task t1 touches a hub file",
        ),
        (
            vec![task("t1", "S", "auth", "")],
            Some(Size::L),
            "task t1 is L",
        ),
        (
            vec![task("t1", "S", "auth", ""), task("t2", "S", "mail", "")],
            None,
            "a fast-path run has exactly one task, not 2",
        ),
    ];
    for (tasks, size, why) in cases {
        let mut fx =
            Fixture::deciding(&plan_with(PROFILE, &tasks), config::Orchestrator::default());
        let effects = fx.start_with(false, |run: &mut Run| {
            mark_fast(run, triage(), None);
            if let Some(size) = size {
                run.tasks[0].size = size;
            }
        });
        assert_eq!(
            replies(&effects),
            vec![Err(format!("the fast path does not apply: {why}"))]
        );
        only_bookkeeping(&effects);
        assert!(fx.state.runs.is_empty(), "{why}: a run was created");
    }
    // The control: the same one-task run that is neither hub nor L starts.
    let fx = started(true);
    assert_eq!(fx.run().state, RunState::Running);
}
