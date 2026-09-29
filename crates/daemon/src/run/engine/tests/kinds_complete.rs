//! Milestone 9 task M9.9: `reported` is a finished state in the engine (M9.2 review
//! ruling 4), completion with an orchestrator (decision 38), decision 25's restart of
//! a rewritten mis-sized task, and the saturating usage sum.

use proto::{
    BlockInfo, BlockReason, HoldKind, HoldState, IntegrationState, PlanEdit, RunState, TaskState,
    TokenUsage,
};
use serde_json::json;

use super::dispatch::edit;
use super::fixture::*;
use super::kinds::running;
use super::kinds_integration::{C1, merge_real};
use super::orch::{add, edit_plan, launched};
use crate::run::engine::{AgentSignal, Effect, TurnOutcome};
use crate::run::model::Outgoing;
use crate::run::orch::{
    EpicRecord, GateHoldRecord, PlannerPhase, PlannerSession, RunScout, RunScoutState,
};

fn verify_refs(effects: &[Effect]) -> usize {
    ops_in(effects, "VerifyRefs").len()
}

#[test]
fn a_run_of_merged_and_reported_tasks_completes() {
    let mut fx = running(
        "",
        &[task("t1", "S", "api", ""), task("t2", "S", "auth", "")],
    );
    // t1 finished as a research or review task does (the kinds' own tests drive it).
    fx.run_mut()
        .pending_ops
        .retain(|_, p| p.task_id.as_deref() != Some("t1"));
    let t1 = fx.task_mut("t1");
    t1.state = TaskState::Reported;
    t1.rounds.clear();
    merge_real(&mut fx, "t2", C1);
    assert_eq!(verify_refs(&fx.log), 1, "{:#?}", fx.log);
}

#[test]
fn the_outbox_skips_a_reported_task() {
    let mut fx = running("", &[task("t1", "S", "auth", "")]);
    let launched = fx.launch_all();
    assert_eq!(launched.len(), 1);
    let window = launched[0].1;
    // A finished session between turns, with a message still queued for it.
    let task = fx.task_mut("t1");
    task.state = TaskState::Reported;
    task.rounds[0].turn_open = false;
    let run = fx.run_mut();
    let id = run.next_message;
    run.next_message += 1;
    run.outbox.push(Outgoing {
        id,
        window_id: window,
        task_id: "t1".into(),
        text: "late news".into(),
        queued_at: 1,
        delivered_at: None,
    });
    let effects = fx.tick();
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::Deliver { .. }
                | Effect::Op {
                    kind: crate::run::engine::OpKind::ResumeSession { .. },
                    ..
                }
        )),
        "{effects:#?}"
    );
}

#[test]
fn a_reported_task_is_recorded_as_finished() {
    let mut fx = running(
        "",
        &[task("t1", "S", "api", ""), task("t2", "S", "auth", "")],
    );
    // History on, into a repository directory of its own.
    fx.run_mut().history = true;
    fx.run_mut().repo_dir = "/tmp/data/repo".into();
    fx.task_mut("t1").state = TaskState::Reported;
    let task = fx.task("t1");
    assert_eq!(
        crate::run::history::outcome(task),
        proto::TaskOutcome::Reported
    );
    // Its record is due at once, as a merged task's is, while the run still runs.
    let due = crate::run::history::due(fx.run());
    assert!(
        due.iter()
            .any(|(i, o)| fx.run().tasks[*i].id() == "t1" && *o == proto::TaskOutcome::Reported),
        "{due:?}"
    );
}

/// A planned `--yes` run with `t1` merged: it completes at that merge unless `blocker`
/// holds it.
fn completes_unless(blocker: impl FnOnce(&mut crate::run::model::Run)) -> usize {
    let mut fx = launched(true);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    assert_eq!(fx.run().state, RunState::Running);
    blocker(fx.run_mut());
    merge_real(&mut fx, "t1", C1);
    fx.tick();
    verify_refs(&fx.log)
}

fn hold(id: &str, kind: HoldKind, state: HoldState) -> GateHoldRecord {
    GateHoldRecord {
        id: id.into(),
        kind,
        state,
        tasks: Vec::new(),
        created_at: 1,
        decided_at: None,
        decided_by: None,
    }
}

fn session() -> PlannerSession {
    PlannerSession {
        session: 1,
        window_id: Some(91),
        op: None,
        started_at: 1,
        ended_at: None,
        usage: TokenUsage::default(),
        rejections: 0,
    }
}

/// One way a run is kept from completing.
type Blocker = Box<dyn FnOnce(&mut crate::run::model::Run)>;

#[test]
fn completion_waits_for_holds_planners_scouts_integration_and_submit() {
    // The control: nothing holds it.
    assert_eq!(completes_unless(|_| {}), 1);
    let cases: Vec<(&str, Blocker)> = vec![
        (
            "a drafting hold",
            Box::new(|run| {
                run.orch.gate_holds.push(hold(
                    "promotion",
                    HoldKind::Promotion,
                    HoldState::Drafting,
                ))
            }),
        ),
        (
            "an awaiting hold",
            Box::new(|run| {
                run.orch.gate_holds.push(hold(
                    "promotion",
                    HoldKind::Promotion,
                    HoldState::Awaiting,
                ))
            }),
        ),
        (
            "a live planner",
            Box::new(|run| {
                let mut e = EpicRecord::new("mail", PlannerPhase::Planning);
                e.sessions.push(session());
                run.orch.epics.push(e);
            }),
        ),
        (
            "a running run scout",
            Box::new(|run| {
                run.orch.run_scouts.push(RunScout {
                    id: format!("{H4}-api"),
                    question: "q".into(),
                    area: vec!["crates/api/**".into()],
                    web: false,
                    state: RunScoutState::Running,
                    queued_at: 1,
                    started_at: Some(1),
                    ended_at: None,
                    window_id: Some(77),
                })
            }),
        ),
        (
            "an epic whose integration asked for changes",
            Box::new(|run| {
                let mut e = EpicRecord::new("mail", PlannerPhase::Finished);
                e.integration_state = IntegrationState::Changes;
                e.integration_rounds = 1;
                run.orch.epics.push(e);
            }),
        ),
        (
            "a plan not submitted",
            Box::new(|run| {
                run.orch.orchestrator.as_mut().unwrap().plan_submitted = false;
            }),
        ),
    ];
    for (name, blocker) in cases {
        assert_eq!(completes_unless(blocker), 0, "{name}");
    }
    // An approved hold, a finished planner, a reported scout and an approved epic hold
    // nothing.
    let released = completes_unless(|run| {
        let mut approved = hold(
            "epic:mail",
            HoldKind::Epic {
                epic: "mail".into(),
            },
            HoldState::Approved,
        );
        approved.decided_at = Some(2);
        run.orch.gate_holds.push(approved);
        let mut e = EpicRecord::new("mail", PlannerPhase::Finished);
        e.integration_state = IntegrationState::Approved;
        run.orch.epics.push(e);
    });
    assert_eq!(released, 1);
}

/// A running run whose `t1` was started, then blocked with `reason`.
pub(super) fn blocked(reason: BlockReason) -> Fixture {
    let mut fx = running("", &[task("t1", "M", "auth", "")]);
    fx.launch_all();
    let task = fx.task_mut("t1");
    task.state = TaskState::Blocked;
    task.block = Some(BlockInfo {
        reason,
        text: "does not fit".into(),
    });
    for round in &mut task.rounds {
        round.ended = true;
        round.turn_open = false;
    }
    fx
}

fn amend_brief(fx: &mut Fixture) -> Vec<Effect> {
    edit(
        fx,
        vec![PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: Some("A smaller first step".into()),
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: None,
            deps: None,
            stage: None,
        }],
    )
}

#[test]
fn rewriting_a_mis_sized_task_restarts_it_at_rung_2() {
    let mut fx = blocked(BlockReason::MisSized);
    let effects = amend_brief(&mut fx);
    let task = fx.task("t1");
    assert_eq!(task.state, TaskState::Working, "{effects:#?}");
    assert_eq!(task.block, None);
    assert_eq!((task.rung, task.failures), (2, 1));
    // The fresh session's hand-over starts from the task's own work.
    assert_eq!(ops_in(&effects, "DiffSoFar").len(), 1, "{effects:#?}");
    // An amend that changes none of brief, acceptance, size or route restarts nothing.
    let mut fx = blocked(BlockReason::MisSized);
    edit(
        &mut fx,
        vec![PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: None,
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: Some(3),
            size: None,
            deps: None,
            stage: None,
        }],
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
}

#[test]
fn editing_a_human_blocked_task_does_not_restart_it() {
    for reason in [
        BlockReason::Human,
        BlockReason::Conflict,
        BlockReason::Environment,
    ] {
        let mut fx = blocked(reason);
        let rung = fx.task("t1").rung;
        let effects = amend_brief(&mut fx);
        let task = fx.task("t1");
        assert_eq!(task.state, TaskState::Blocked, "{reason:?}");
        assert_eq!(task.block.as_ref().map(|b| b.reason), Some(reason));
        assert_eq!(task.rung, rung);
        assert!(ops_in(&effects, "DiffSoFar").is_empty());
    }
}

#[test]
fn usage_sum_saturates() {
    // A reviewer's spend counts toward no budget, so both sums reach its round.
    let (mut fx, _, rwindow) = super::gates_review::reviewed(PROFILE, "");
    let big = TokenUsage {
        input: u64::MAX - 1,
        output: u64::MAX - 1,
        cache_read: u64::MAX - 1,
        cache_write: u64::MAX - 1,
    };
    fx.signal(rwindow, AgentSignal::Spend { usage: big });
    fx.signal(
        rwindow,
        AgentSignal::TurnEnded {
            outcome: TurnOutcome::Interrupted,
            usage: Some(big),
            denials: vec![],
        },
    );
    let round = fx
        .task("t1")
        .rounds
        .iter()
        .rfind(|r| r.role == proto::AgentRole::Reviewer)
        .unwrap();
    let usage = round.usage;
    assert_eq!(
        (
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write
        ),
        (u64::MAX, u64::MAX, u64::MAX, u64::MAX)
    );
}
