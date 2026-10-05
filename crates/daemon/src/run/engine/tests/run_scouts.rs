//! Milestone 9 task M9.8: run scouts (decision 20), the sub-planners and scouts a
//! daemon restart finds (decisions 20, 32), and `RunInfo.planners` (decision 33).

use proto::{PlannerInfo, PlannerState, Route, RunState, ScoutKind, ScoutReport, TokenUsage};
use serde_json::{Value, json};

use super::fixture::*;
use super::orch::{ORCH, answer, error, launched, orch_tool};
use super::planners::{PLANNER, epic, planner_ended, planning_mail, spawn, submit_epic, task_in};
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::run::orch::contract::scout_first_turn;
use crate::run::orch::{PlannerPhase, RunScoutState};

/// A stored report with `summary` and no files.
pub(super) fn report(id: &str, summary: &str) -> ScoutReport {
    ScoutReport {
        id: id.into(),
        kind: ScoutKind::Area,
        run_id: Some(RUN_ID.into()),
        question: String::new(),
        summary: summary.into(),
        files: Vec::new(),
        modules: Vec::new(),
        interfaces: Vec::new(),
        risks: Vec::new(),
        profile: None,
        route: Route {
            runtime: proto::Runtime::Claude,
            model: String::new(),
            strength: proto::Strength::Fast,
            effort: proto::Effort::Low,
        },
        window_id: None,
        started_at: 0,
        finished_at: 0,
        tool_calls: 0,
        usage: TokenUsage::default(),
    }
}

pub(super) fn scout(fx: &mut Fixture, id: &str) -> Vec<Effect> {
    let args = json!({"id": id, "question": format!("What is {id}?"), "area": ["crates/api/**"]});
    orch_tool(fx, ORCH, "spawn_scout", args)
}

fn ok(effects: &[Effect]) -> Value {
    let (ok, value) = answer(effects);
    assert!(ok, "{value}");
    value
}

pub(super) fn scout_ended(fx: &mut Fixture, id: &str, outcome: ScoutEnd) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::ScoutEnded {
        run_id: RUN_ID.into(),
        scout_id: id.into(),
        outcome,
        usage: TokenUsage {
            input: 7,
            ..Default::default()
        },
    }))
}

/// The fixture's state after a daemon restart: its runs restored, nothing replayed.
fn restart(fx: &mut Fixture) -> Vec<Effect> {
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    })
}

/// Decision 20: a scout queues for a reader slot (behind a live sub-planner here), and
/// the reply is at once.
#[test]
fn spawn_scout_queues_in_a_reader_slot_and_replies_at_once() {
    let mut fx = launched(false);
    fx.run_mut().limits.max_readers = 1;
    spawn(&mut fx, "mail");
    let effects = scout(&mut fx, "api");
    assert_eq!(
        ok(&effects),
        json!({"scout_id": format!("{H4}-api"), "state": "queued"})
    );
    assert!(ops_in(&effects, "StartScout").is_empty(), "{effects:#?}");
    let failed = ScoutEnd::Failed {
        reason: "boom".into(),
    };
    let effects = planner_ended(&mut fx, ("mail", 1), failed);
    let starts = ops_in(&effects, "StartScout");
    assert_eq!(starts.len(), 1, "{effects:#?}");
    let (op, OpKind::StartScout { spec }) = &starts[0] else {
        unreachable!()
    };
    let full = format!("{H4}-api");
    assert_eq!(spec.id, full);
    assert_eq!(spec.kind, ScoutKind::Area);
    assert_eq!(spec.run_id.as_deref(), Some(RUN_ID));
    assert_eq!(spec.cwd, fx.run().root);
    assert_eq!(spec.project, fx.run().project);
    assert_eq!(spec.base_sha, BASE);
    assert!(!spec.web);
    assert_eq!(
        spec.repo_paths,
        vec![fx.run().root.clone(), fx.run().git_common_dir.clone()]
    );
    let area = vec!["crates/api/**".to_string()];
    assert_eq!(
        spec.first_turn,
        scout_first_turn(fx.run(), &full, &area, "What is api?")
    );
    assert_eq!(fx.run().orch.run_scouts[0].state, RunScoutState::Running);
    fx.done(*op, OpResult::ScoutStarted { window_id: 77 });
    assert_eq!(fx.run().orch.run_scouts[0].window_id, Some(77));
    // With a slot free, the next one starts at once.
    fx.run_mut().limits.max_readers = 3;
    assert_eq!(ok(&scout(&mut fx, "db"))["state"], "starting");
}

#[test]
fn scout_id_is_prefixed_and_unique() {
    let mut fx = launched(false);
    assert_eq!(ok(&scout(&mut fx, "api"))["scout_id"], format!("{H4}-api"));
    assert_eq!(
        error(&scout(&mut fx, "api")),
        format!("scout {H4}-api already exists in run {RUN_ID}")
    );
    let text = error(&scout(&mut fx, "API"));
    assert!(text.starts_with("invalid arguments: id: "), "{text}");
    assert_eq!(fx.run().orch.run_scouts.len(), 1);
}

#[test]
fn max_scouts_is_enforced() {
    let mut fx = launched(false);
    fx.run_mut().limits.orch.max_scouts = 2;
    scout(&mut fx, "a");
    scout(&mut fx, "b");
    assert_eq!(
        error(&scout(&mut fx, "c")),
        format!("run {RUN_ID} already has 2 scouts, the most max_scouts allows")
    );
}

#[test]
fn scout_ended_records_the_report_and_usage() {
    let mut fx = launched(false);
    scout(&mut fx, "api");
    scout(&mut fx, "db");
    scout_ended(&mut fx, &format!("{H4}-api"), ScoutEnd::Reported);
    let failed = ScoutEnd::Failed {
        reason: "the scout ran longer than 900 s".into(),
    };
    scout_ended(&mut fx, &format!("{H4}-db"), failed);
    let run = fx.run();
    assert_eq!(run.orch.run_scouts[0].state, RunScoutState::Reported);
    assert_eq!(run.orch.run_scouts[0].ended_at, Some(fx.now - 1));
    assert!(matches!(
        &run.orch.run_scouts[1].state,
        RunScoutState::Failed { reason } if reason == "the scout ran longer than 900 s"
    ));
    assert_eq!(run.scout_reports, vec![format!("{H4}-api")]);
    assert_eq!(run.scout_usage.input, 14);
    let notes = &run.orch.orchestrator.as_ref().unwrap().notes;
    assert_eq!(
        notes,
        &vec![
            format!("scout {H4}-api reported"),
            format!("scout {H4}-db failed: the scout ran longer than 900 s")
        ]
    );
    // A report is now evidence for the plan rules (decision 23.2).
    let mut edit = super::orch::add("t1", "auth");
    edit["task"]["scout_refs"] = json!([format!("{H4}-api")]);
    ok(&super::orch::edit_plan(&mut fx, json!({"edits": [edit]})));
}

#[test]
fn restore_fails_running_scouts_with_the_documented_reason() {
    let mut fx = planning_mail(false);
    scout(&mut fx, "api");
    fx.run_mut().limits.max_readers = 1;
    scout(&mut fx, "db");
    assert_eq!(fx.run().orch.run_scouts[1].state, RunScoutState::Queued);
    restart(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Paused);
    for s in &run.orch.run_scouts {
        assert_eq!(
            s.state,
            RunScoutState::Failed {
                reason: "the daemon restarted during this scout".into()
            },
            "{}",
            s.id
        );
    }
    assert_eq!(
        epic(&fx, "mail").phase,
        PlannerPhase::Failed {
            reason: "the daemon restarted during this sub-planner".into()
        }
    );
    assert!(run.pending_ops.values().all(|p| !matches!(
        p.kind,
        OpKind::StartScout { .. } | OpKind::StartPlanner { .. }
    )));
}

/// The process-kill rule: a restart kills no scout or sub-planner session.
#[test]
fn restore_kills_nothing() {
    let mut fx = planning_mail(false);
    scout(&mut fx, "api");
    fx.done(
        fx.op("StartScout").0,
        OpResult::ScoutStarted { window_id: 77 },
    );
    let effects = restart(&mut fx);
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::KillWindow { .. }
                | Effect::StopPlanner { .. }
                | Effect::RetireWindow { .. }
                | Effect::RemoveWindow { .. }
                | Effect::Interrupt { .. }
        )),
        "{effects:#?}"
    );
}

/// Decision 33: one `PlannerInfo` per epic, a queued planner shown `Planning`.
#[test]
fn planners_snapshot_fields() {
    let mut fx = planning_mail(false);
    let started = fx.now;
    submit_epic(&mut fx, json!([{"op": "cancel_task", "task_id": "t1"}]));
    let effects = submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    assert!(super::planners_holds::reply(&effects).0, "{effects:#?}");
    let accepted_at = fx.now;
    fx.run_mut().limits.max_readers = 0;
    spawn(&mut fx, "web");
    let route = epic(&fx, "mail").route.clone();
    let info = crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0]
        .planners
        .clone();
    assert_eq!(
        info,
        vec![
            PlannerInfo {
                epic: "mail".into(),
                title: "Epic mail".into(),
                area: vec!["crates/mail/**".into()],
                route: route.clone(),
                window_id: Some(PLANNER),
                state: PlannerState::Finished,
                started_at: started - 1,
                ended_at: Some(accepted_at),
                edits_accepted: 1,
                edits_rejected: 1,
                last_rejection: Some(
                    "task t1: a sub-planner changes only its own epic's tasks".into()
                ),
                replans: Vec::new(),
                note: None,
                covers: Vec::new(),
            },
            PlannerInfo {
                epic: "web".into(),
                title: "Epic web".into(),
                area: vec!["crates/web/**".into()],
                route,
                window_id: None,
                state: PlannerState::Planning,
                started_at: fx.now,
                ended_at: None,
                edits_accepted: 0,
                edits_rejected: 0,
                last_rejection: None,
                replans: Vec::new(),
                note: None,
                covers: Vec::new(),
            },
        ]
    );
    assert_eq!(epic(&fx, "web").phase, PlannerPhase::Queued);
}
