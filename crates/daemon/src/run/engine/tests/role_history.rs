//! Milestone 9 task M9.13b (decision 43): the role-routing records of the orchestrator,
//! sub-planners, run scouts and run-bound deciders. Each is kept in the run before its
//! session starts, finished once with the session's factual outcome, appended once, and
//! finished `interrupted` by a restore.

use std::path::PathBuf;

use proto::{
    AgentRole, DeciderSource, HistoryLine, RoleOutcome, RoleRoutingDecision, RunState, Strength,
};
use serde_json::json;

use super::fixture::*;
use super::orch::{ORCH, add, edit_plan, launched, orch_tool};
use super::planners::{PLANNER, planner_ended, planner_started, planning_mail, spawn, task_in};
use crate::decider::{DeciderAnswer, DeciderKind, Decision};
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::run::orch::roles;
use crate::scout::spec::ScoutContext;

const REPO: &str = "/tmp/data/repos/x-3f9a";

/// The `role_route` lines among `effects`' `AppendHistory` ops: (op, line).
pub(super) fn role_lines(effects: &[Effect]) -> Vec<(u64, RoleRoutingDecision)> {
    ops_in(effects, "AppendHistory")
        .into_iter()
        .filter_map(|(op, kind)| match kind {
            OpKind::AppendHistory {
                path,
                record_id,
                line,
            } => {
                assert_eq!(path, PathBuf::from(REPO).join("history.jsonl"));
                match *line {
                    HistoryLine::RoleRoute(d) => {
                        assert_eq!(d.record_id, record_id);
                        Some((op, d))
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// `fx`'s run writes history to [`REPO`].
pub(super) fn with_history(fx: &mut Fixture) {
    fx.run_mut().repo_dir = REPO.into();
    fx.run_mut().history = true;
}

pub(super) fn of_role(fx: &Fixture, role: AgentRole) -> Vec<RoleRoutingDecision> {
    fx.run()
        .role_routing_decisions
        .iter()
        .filter(|d| d.role == role)
        .cloned()
        .collect()
}

/// The scout service's context of these tests: the default configuration.
fn scout_ctx() -> ScoutContext {
    let orchestrator = config::Orchestrator::default();
    ScoutContext {
        roster: orchestrator.models.clone(),
        default_runtime: proto::Runtime::Claude,
        scouts: orchestrator.scouts.clone(),
        claude: orchestrator.claude.clone(),
        caps: crate::headless::argv::CLI_CAPS,
        data_dir: PathBuf::from("/nonexistent/anthrex-test/data"),
    }
}

/// The orchestrator spawns scout `id`; its `StartScout` is issued and the driver sends
/// its record (as `start_scout` does) before the session starts. Returns the full id.
fn scout_dispatched(fx: &mut Fixture, id: &str, ctx: &ScoutContext) -> String {
    let args = json!({"id": id, "question": "How is auth wired?", "area": ["crates/auth/**"]});
    orch_tool(fx, ORCH, "spawn_scout", args);
    let (_, kind) = fx.op("StartScout");
    let OpKind::StartScout { spec } = kind else {
        unreachable!()
    };
    let decision = roles::scout_record(fx.run(), &spec.id, ctx, 1_500);
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::RoleRoute {
        reply,
        run_id: RUN_ID.into(),
        decision: Box::new(decision),
    }));
    spec.id.clone()
}

fn scout_ended(fx: &mut Fixture, id: &str, outcome: ScoutEnd) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::ScoutEnded {
        run_id: RUN_ID.into(),
        scout_id: id.into(),
        outcome,
        usage: Default::default(),
    }))
}

/// A run-bound decider's record, as the driver's `decide_as` sends it.
fn decider_dispatched(fx: &mut Fixture, op: u64) -> String {
    let run = fx.run();
    let route = proto::Route {
        runtime: proto::Runtime::Claude,
        model: "claude-haiku-4-5".into(),
        strength: Strength::Fast,
        effort: proto::Effort::Low,
    };
    let input = roles::input_of(run);
    let d = roles::decider_record(
        Some(run),
        (&op.to_string(), "check_summary"),
        &["t1".to_string()],
        (&route, Vec::new()),
        input,
        1_600,
    );
    let id = d.record_id.clone();
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::RoleRoute {
        reply,
        run_id: RUN_ID.into(),
        decision: Box::new(d),
    }));
    id
}

fn fallback(reason: &str) -> Decision {
    Decision {
        kind: DeciderKind::CheckSummary,
        answer: DeciderAnswer::CheckSummary { lines: Vec::new() },
        source: DeciderSource::Fallback,
        fallback_reason: Some(reason.into()),
        usage: None,
        secs: 60,
    }
}

fn decider_ended(fx: &mut Fixture, record_id: &str, decision: &Decision) -> Vec<Effect> {
    let (outcome, result) = roles::decider_outcome(decision);
    fx.next(EventKind::Orch(OrchEvent::RoleRouteEnded {
        run_id: RUN_ID.into(),
        record_id: record_id.into(),
        outcome,
        result,
    }))
}

/// A daemon restart: the runs restored, nothing replayed.
fn restart(fx: &mut Fixture) -> Vec<Effect> {
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    })
}

fn resume(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    })
}

/// A running run (`t1` approved) with its orchestrator in window [`ORCH`], its
/// sub-planner of epic `mail` live in window [`PLANNER`], and history on.
fn running() -> Fixture {
    let mut fx = planning_mail(false);
    with_history(&mut fx);
    fx
}

#[test]
fn each_role_keeps_its_dispatch_snapshot_after_a_config_change() {
    let mut fx = running();
    let ctx = scout_ctx();
    let scout = scout_dispatched(&mut fx, "api", &ctx);
    let decider = decider_dispatched(&mut fx, 77);
    let dispatched = fx.run().role_routing_decisions.clone();
    let roles_seen: Vec<AgentRole> = dispatched.iter().map(|d| d.role).collect();
    assert_eq!(
        roles_seen,
        vec![
            AgentRole::Orchestrator,
            AgentRole::Planner,
            AgentRole::Scout,
            AgentRole::Decider
        ]
    );
    for d in &dispatched {
        assert_eq!(
            d.candidates[d.selected_index as usize].route, d.chosen,
            "{d:#?}"
        );
        assert_eq!(d.outcome, None, "open while its session runs");
        assert!(d.input.goal.is_some() && d.run_id.as_deref() == Some(RUN_ID));
    }
    let orchestrator = &dispatched[0];
    assert_eq!(orchestrator.source, "roster_default");
    assert!(orchestrator.candidates.len() > 1, "{orchestrator:#?}");
    assert_eq!(dispatched[1].input.epic.as_deref(), Some("mail"));
    assert_eq!(dispatched[1].input.area, vec!["crates/mail/**".to_string()]);
    assert_eq!(dispatched[2].input.area, vec!["crates/auth/**".to_string()]);
    assert_eq!(
        dispatched[3].input.question_kind.as_deref(),
        Some("check_summary")
    );
    // The configuration changes: the roster and every role's settings.
    let run = fx.run_mut();
    run.roster.retain(|e| e.strength == Strength::Frontier);
    run.limits.orch.planners.strength = Strength::Fast;
    run.limits.orch.agent.effort = proto::Effort::Low;
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.routing.candidates.clear();
    }
    // Every session ends; each record keeps what it was dispatched with.
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        live: false,
        launch: 1,
    }));
    planner_ended(
        &mut fx,
        ("mail", 1),
        ScoutEnd::Failed {
            reason: "timed out".into(),
        },
    );
    scout_ended(&mut fx, &scout, ScoutEnd::Reported);
    decider_ended(
        &mut fx,
        &decider,
        &fallback("the decider timed out after 60 s"),
    );
    let ended = fx.run().role_routing_decisions.clone();
    assert_eq!(ended.len(), dispatched.len());
    for (before, after) in dispatched.iter().zip(&ended) {
        assert!(after.outcome.is_some(), "{after:#?}");
        let mut unchanged = after.clone();
        unchanged.outcome = None;
        unchanged.result = None;
        assert_eq!(&unchanged, before, "the dispatch snapshot is kept");
    }
}

#[test]
fn orchestrator_restart_gets_a_new_session_id() {
    let mut fx = launched(false);
    with_history(&mut fx);
    restart(&mut fx);
    let effects = resume(&mut fx);
    assert_eq!(
        ops_in(&effects, "RestartOrchestrator").len(),
        1,
        "{effects:#?}"
    );
    let records = of_role(&fx, AgentRole::Orchestrator);
    assert_eq!(records.len(), 2, "{records:#?}");
    assert_eq!(records[0].session_id, "1");
    assert_eq!(records[0].outcome, Some(RoleOutcome::Interrupted));
    assert_eq!(
        (records[1].session_id.as_str(), records[1].trigger.as_str()),
        ("2", "restart")
    );
    assert_ne!(records[0].record_id, records[1].record_id);
    assert_eq!(records[1].outcome, None);
    // The restart's record keeps the launch's snapshot.
    assert_eq!(records[1].candidates, records[0].candidates);
    assert_eq!(records[1].source, records[0].source);
    // The user's own `anthrex restart` after an exit is a session of its own too.
    let (op, _) = fx.op("RestartOrchestrator");
    fx.done(op, OpResult::Restarted);
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    for live in [false, true] {
        fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
            run_id: RUN_ID.into(),
            window_id: ORCH,
            live,
            launch,
        }));
    }
    let records = of_role(&fx, AgentRole::Orchestrator);
    assert_eq!(records.len(), 3);
    // Its window exited before it submitted a plan (review M-1).
    assert_eq!(records[1].outcome, Some(RoleOutcome::Failed));
    assert_eq!(
        (records[2].session_id.as_str(), records[2].outcome),
        ("3", None)
    );
}

#[test]
fn planner_retry_gets_a_new_session_id() {
    let mut fx = running();
    planner_ended(
        &mut fx,
        ("mail", 1),
        ScoutEnd::Failed {
            reason: "timed out".into(),
        },
    );
    // The orchestrator re-plans the failed epic: a fresh session.
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER + 1);
    let records = of_role(&fx, AgentRole::Planner);
    assert_eq!(records.len(), 2, "{records:#?}");
    assert_eq!(records[0].session_id, "mail/1");
    assert_eq!(records[0].outcome, Some(RoleOutcome::Failed));
    assert_eq!(records[0].result.as_deref(), Some("timed out"));
    assert_eq!(
        (records[1].session_id.as_str(), records[1].trigger.as_str()),
        ("mail/2", "replan")
    );
    assert_eq!(records[1].outcome, None);
    assert_ne!(records[0].record_id, records[1].record_id);
}

#[test]
fn scout_retry_gets_a_new_session_id() {
    let mut fx = running();
    let ctx = scout_ctx();
    let first = scout_dispatched(&mut fx, "api", &ctx);
    let (op, _) = fx.op("StartScout");
    let effects = fx.done(
        op,
        OpResult::Failed {
            message: "no window".into(),
        },
    );
    let lines = role_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    assert_eq!(lines[0].1.outcome, Some(RoleOutcome::Failed));
    // A scout id is used once per run (decision 20): the retry is a new scout.
    let second = scout_dispatched(&mut fx, "api2", &ctx);
    let records = of_role(&fx, AgentRole::Scout);
    assert_eq!(records.len(), 2, "{records:#?}");
    assert_eq!(records[0].session_id, first);
    assert_eq!(records[1].session_id, second);
    assert_ne!(records[0].record_id, records[1].record_id);
    assert_eq!(
        records[0].candidates, records[1].candidates,
        "the same configuration"
    );
    assert_eq!(records[1].outcome, None);
}

#[test]
fn decider_fallback_is_recorded_as_fallback() {
    let mut fx = running();
    let id = decider_dispatched(&mut fx, 77);
    assert_eq!(id, format!("{RUN_ID}/decider/77"));
    let effects = decider_ended(&mut fx, &id, &fallback("the decider timed out after 60 s"));
    let lines = role_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    let d = &lines[0].1;
    assert_eq!(
        (d.role, d.outcome),
        (AgentRole::Decider, Some(RoleOutcome::Fallback))
    );
    assert_eq!(
        d.result.as_deref(),
        Some("the decider timed out after 60 s")
    );
    assert_eq!(d.task_id.as_deref(), Some("t1"));
    // An answer is `completed`.
    let id = decider_dispatched(&mut fx, 78);
    let mut answered = fallback("");
    answered.source = DeciderSource::Decider;
    answered.fallback_reason = None;
    let d = role_lines(&decider_ended(&mut fx, &id, &answered))
        .remove(0)
        .1;
    assert_eq!(d.outcome, Some(RoleOutcome::Completed));
    assert_eq!(d.result.as_deref(), Some("answered"));
}

#[test]
fn session_end_appends_one_line_with_the_outcome() {
    let mut fx = running();
    let args = json!({"edits": [task_in("t2", "mail")]});
    let submitted = super::planners::planner_tool(&mut fx, (PLANNER, "mail"), "submit_epic", args);
    assert!(
        role_lines(&submitted).is_empty(),
        "the session has not ended"
    );
    let effects = planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    let lines = role_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    let d = &lines[0].1;
    assert_eq!(d.record_id, format!("{RUN_ID}/planner/mail/1"));
    assert_eq!(d.outcome, Some(RoleOutcome::Completed));
    assert_eq!(d.result.as_deref(), Some("epic accepted"));
    // The same end reported again appends nothing.
    let again = planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    assert!(role_lines(&again).is_empty(), "{again:#?}");
    // A run whose history is off keeps its records and appends none.
    fx.run_mut().history = false;
    let ctx = scout_ctx();
    let scout = scout_dispatched(&mut fx, "api", &ctx);
    let effects = scout_ended(&mut fx, &scout, ScoutEnd::Reported);
    assert!(role_lines(&effects).is_empty());
    let kept = of_role(&fx, AgentRole::Scout);
    assert_eq!(kept[0].outcome, Some(RoleOutcome::Completed));
}

#[test]
fn scout_failed_on_restore_is_recorded_interrupted() {
    let mut fx = running();
    let ctx = scout_ctx();
    let scout = scout_dispatched(&mut fx, "api", &ctx);
    let effects = restart(&mut fx);
    let state = &fx.run().orch.run_scouts[0].state;
    assert!(
        matches!(state, crate::run::orch::RunScoutState::Failed { .. }),
        "decision 20: {state:?}"
    );
    let lines: Vec<RoleRoutingDecision> =
        role_lines(&effects).into_iter().map(|(_, d)| d).collect();
    let d = lines
        .iter()
        .find(|d| d.session_id == scout)
        .unwrap_or_else(|| panic!("{lines:#?}"));
    assert_eq!(d.outcome, Some(RoleOutcome::Interrupted));
    assert_eq!(
        d.result.as_deref(),
        Some("the daemon restarted during this session")
    );
}

#[test]
fn restore_marks_every_open_record_interrupted_once() {
    let mut fx = running();
    let ctx = scout_ctx();
    scout_dispatched(&mut fx, "api", &ctx);
    let decider = decider_dispatched(&mut fx, 77);
    let ended = decider_ended(&mut fx, &decider, &fallback("no reader slot"));
    // Its line was written before the restart.
    let (op, _) = role_lines(&ended).remove(0);
    fx.done(op, OpResult::HistoryAppended);
    let effects = restart(&mut fx);
    let lines = role_lines(&effects);
    let mut interrupted: Vec<AgentRole> = lines.iter().map(|(_, d)| d.role).collect();
    interrupted.sort_by_key(|r| roles::role_name(*r));
    assert_eq!(
        interrupted,
        vec![
            AgentRole::Orchestrator,
            AgentRole::Planner,
            AgentRole::Scout
        ],
        "the decider had ended: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .all(|(_, d)| d.outcome == Some(RoleOutcome::Interrupted))
    );
    for (op, _) in &lines {
        fx.done(*op, OpResult::HistoryAppended);
    }
    // A second restart finds nothing open.
    let again = restart(&mut fx);
    assert!(role_lines(&again).is_empty(), "{again:#?}");
    let open = fx
        .run()
        .role_routing_decisions
        .iter()
        .filter(|d| d.outcome.is_none())
        .count();
    assert_eq!(open, 0);
}

#[test]
fn a_run_never_attributes_its_outcome_to_one_role() {
    let mut fx = launched(false);
    with_history(&mut fx);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    // The user rejects the plan: the run is discarded.
    let reply = fx.reply();
    fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    let (op, _) = fx.op("Discard");
    let effects = fx.done(
        op,
        OpResult::Finished {
            outcome: "discarded".into(),
            kept_branches: Vec::new(),
        },
    );
    let state = fx.run().state;
    assert!(state.is_terminal(), "{state:?}");
    let lines = role_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    let d = &lines[0].1;
    // The orchestrator's session ran until the run ended: its own outcome, never the
    // run's.
    assert_eq!(d.role, AgentRole::Orchestrator);
    assert_eq!(d.outcome, Some(RoleOutcome::Completed));
    assert_eq!(
        d.result.as_deref(),
        Some("live until the run ended; plan submitted")
    );
    let run_words = ["cancel", "fail", "discard", "accept", state.label()];
    for d in &fx.run().role_routing_decisions {
        let text = d.result.clone().unwrap_or_default();
        assert!(
            !run_words.iter().any(|w| text.contains(w)),
            "{text} names the run's outcome"
        );
    }
    assert_ne!(state, RunState::Planning);

    // Review I-1: `run cancel` stops a live sub-planner and a running scout. Their
    // sessions were stopped by anthrex, not failed: `interrupted`, with a reason that
    // names neither the run's outcome nor the user's action.
    let mut fx = running();
    let scout = scout_dispatched(&mut fx, "api", &scout_ctx());
    let reply = fx.reply();
    let mut effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    // The driver's halted sessions end with the halt's reason.
    let halted = || ScoutEnd::Failed {
        reason: crate::run::engine::planners::RUN_CANCELLED.into(),
    };
    effects.extend(planner_ended(&mut fx, ("mail", 1), halted()));
    effects.extend(scout_ended(&mut fx, &scout, halted()));
    let lines = role_lines(&effects);
    let stopped: Vec<(AgentRole, Option<RoleOutcome>, Option<String>)> = lines
        .iter()
        .map(|(_, d)| (d.role, d.outcome, d.result.clone()))
        .collect();
    let why = Some("stopped when the run ended".to_string());
    assert_eq!(
        stopped,
        vec![
            (
                AgentRole::Planner,
                Some(RoleOutcome::Interrupted),
                why.clone()
            ),
            (AgentRole::Scout, Some(RoleOutcome::Interrupted), why),
        ]
    );
    for d in &fx.run().role_routing_decisions {
        let text = d.result.clone().unwrap_or_default();
        assert!(!run_words[..4].iter().any(|w| text.contains(w)), "{text}");
    }
}
