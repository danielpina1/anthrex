//! Milestone 9.6 task M9.6.13 (decision 32, DF §8.4): each design phase's history
//! record, written when its gate is approved: its clock's time, every design agent's
//! spend and the versions and disputed findings of its gate. Ruling T13-1: an agent's
//! tokens, calls and active time are summed over all its sessions in the phase,
//! relaunches and rethink rounds included, and a rethink's sessions are routed with
//! trigger `rethink`, a T8-7 relaunch's with `relaunch`.

use proto::{AgentRole, DocGateAction, DocGateKind, HistoryLine, PhaseAgent, PhaseRecord};
use proto::{RunState, Runtime, TokenUsage};
use serde_json::{Value, json};

use super::design_agents::*;
use super::design_fixture::*;
use super::design_plan_fixture::{PLAN_REVIEWER, covering, plan_submit, read_back};
use super::design_report::codex_draft;
use super::design_review_fixture::{
    REVIEWER, answers, outcome, reviewed, submit_findings, submit_spec, three_findings,
};
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::engine::{EventKind, OpKind, OrchEvent, ScoutEnd};
use crate::scout::design_spec::BRAINSTORMER_TEXTS;
use crate::scout::machine::unsubmitted;

const REPO: &str = "/tmp/data/repos/x-3f9a";

/// Every phase line appended so far, by record id.
fn phase_lines(fx: &Fixture) -> Vec<(String, PhaseRecord)> {
    (fx.ops("AppendHistory").into_iter())
        .filter_map(|(_, kind)| match kind {
            OpKind::AppendHistory {
                record_id, line, ..
            } => match *line {
                HistoryLine::Phase(record) => Some((record_id, record)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// `role`'s session `session` of `label` ended `secs` after it started, having used
/// `calls` tool calls and `tokens` tokens.
fn end_after(
    fx: &mut Fixture,
    (role, label, session): (AgentRole, &str, u32),
    outcome: ScoutEnd,
    (secs, calls, tokens): (u64, u32, u64),
) {
    let design = fx.run().orch.design.as_ref().unwrap();
    let agent = (design.brainstormers.iter().chain(&design.reviewer))
        .find(|a| a.label == label)
        .unwrap();
    let at = agent.started.unwrap() + secs;
    assert!(at > fx.now, "{at} after {}", fx.now);
    fx.send(
        at,
        EventKind::Orch(OrchEvent::DesignAgentEnded {
            run_id: RUN_ID.into(),
            role,
            label: label.into(),
            session,
            outcome,
            usage: TokenUsage {
                input: tokens - tokens / 4,
                output: tokens / 4,
                ..TokenUsage::default()
            },
            calls,
        }),
    );
}

fn brainstormer(label: &str, session: u32) -> (AgentRole, &str, u32) {
    (AgentRole::Brainstormer, label, session)
}

/// Both brainstormers' drafts in their current sessions, and their sessions ended
/// with the spend given.
fn drafts(fx: &mut Fixture, claude: (u64, u32, u64), codex: (u64, u32, u64)) {
    let current = |fx: &Fixture, k: usize| {
        let a = &agents(fx)[k];
        (a.session, a.window_id.unwrap())
    };
    let ((a, wa), (b, wb)) = (current(fx, 0), current(fx, 1));
    assert!(answered(&submit_draft(fx, wa, DRAFT)).0);
    assert!(answered(&submit_draft(fx, wb, &codex_draft())).0);
    end_after(fx, brainstormer("claude", a), ScoutEnd::Reported, claude);
    end_after(fx, brainstormer("codex", b), ScoutEnd::Reported, codex);
}

fn agent(record: &PhaseRecord, k: usize) -> (AgentRole, u32, u64, u64, &str) {
    let a: &PhaseAgent = &record.agents[k];
    (a.role, a.calls, a.tokens, a.secs, a.outcome.as_str())
}

/// The orchestrator's next call `secs` after the last step.
fn later(fx: &mut Fixture, secs: u64) {
    fx.now += secs - 1;
}

/// Decision 32: one record per phase, written as its gate is approved, never before;
/// its time is the phase clock's (gate waits left out), its agents the phase's design
/// agents, its versions the gate's, its disputed findings the kept ones.
#[test]
fn a_phase_record_is_written_per_phase_with_agents_and_versions() {
    let mut fx = brainstorming();
    fx.run_mut().repo_dir = REPO.into();
    fx.run_mut().roster.push(proto::ModelEntry {
        runtime: Runtime::Codex,
        model: "gpt-6".into(),
        strength: proto::Strength::Frontier,
        note: String::new(),
    });
    drafts(&mut fx, (300, 12, 4_000), (400, 20, 8_000));
    // The clock runs from the drafts-in to the report, then from the user's changes to
    // the revision (50 s); the gate's waits never count.
    let clock = fx
        .run()
        .orch
        .design
        .as_ref()
        .unwrap()
        .phase_started
        .unwrap();
    later(&mut fx, 100);
    submitted(&mut fx, "brainstorm", REPORT);
    let first = fx.now - clock;
    later(&mut fx, 3_000);
    let changes = DocGateAction::Changes {
        note: "Shorter.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Brainstorm, changes).unwrap();
    later(&mut fx, 50);
    submitted(&mut fx, "brainstorm", REPORT);
    assert!(phase_lines(&fx).is_empty(), "none before the approval");
    later(&mut fx, 900);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    let lines = phase_lines(&fx);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let (id, record) = &lines[0];
    assert_eq!(id, &format!("{RUN_ID}/phase/1/brainstorming"));
    assert_eq!(record.record_id, *id);
    assert_eq!((record.run_id.as_str(), record.round), (RUN_ID, 1));
    assert_eq!(
        (record.phase.as_str(), record.secs),
        ("brainstorming", first + 50)
    );
    assert_eq!((record.gate_versions, record.disputed), (2, 0));
    assert_eq!((record.v, record.at), (proto::HISTORY_VERSION, fx.now));
    assert_eq!(
        agent(record, 0),
        (AgentRole::Brainstormer, 12, 4_000, 300, "ok")
    );
    assert_eq!(
        agent(record, 1),
        (AgentRole::Brainstormer, 20, 8_000, 400, "ok")
    );
    assert_eq!(record.agents[1].route.runtime, Runtime::Codex);

    // Specifying: one review (its reviewer's session ended), one finding kept.
    later(&mut fx, 30);
    reviewed(&mut fx, REVIEWER, three_findings());
    let reviewer = (AgentRole::DocReviewer, "spec-r1", 1);
    end_after(&mut fx, reviewer, ScoutEnd::Reported, (60, 5, 1_000));
    let responses = answers(&["F1", "F2", "F3"], &["F2"]);
    outcome(&submit_spec(&mut fx, true, responses)).unwrap();
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    let lines = phase_lines(&fx);
    assert_eq!(lines.len(), 2, "{lines:?}");
    let (id, record) = &lines[1];
    assert_eq!(id, &format!("{RUN_ID}/phase/1/specifying"));
    assert_eq!((record.gate_versions, record.disputed), (1, 1));
    assert_eq!(record.agents.len(), 1);
    assert_eq!(
        agent(record, 0),
        (AgentRole::DocReviewer, 5, 1_000, 60, "ok")
    );

    // Planning: the plan's review, then the gate's one version.
    read_back(&mut fx, 1, SPEC);
    plan_submit(&mut fx, json!([covering("t1", &["R1", "R2"])]), Value::Null).unwrap();
    let label = launches(&fx).last().unwrap().1.kind.label();
    started(&mut fx, &label, PLAN_REVIEWER);
    outcome(&submit_findings(&mut fx, PLAN_REVIEWER, json!([]))).unwrap();
    let plan_reviewer = (AgentRole::DocReviewer, "plan-r1", 1);
    end_after(&mut fx, plan_reviewer, ScoutEnd::Reported, (40, 2, 500));
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    outcome(&effects).unwrap();
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    act(&mut fx, DocGateKind::Plan, DocGateAction::Approve).unwrap();
    let lines = phase_lines(&fx);
    let (id, record) = &lines[2];
    assert_eq!(id, &format!("{RUN_ID}/phase/1/planning"));
    assert_eq!((record.gate_versions, record.disputed), (1, 0));
    assert_eq!(agent(record, 0), (AgentRole::DocReviewer, 2, 500, 40, "ok"));
    assert_eq!(lines.len(), 3);
}

/// A run with history off (no repository data directory) writes no phase line.
#[test]
fn a_run_without_history_writes_no_phase_line() {
    let mut fx = at_brainstorm_gate(false);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    assert!(phase_lines(&fx).is_empty());
}

/// A brainstorm whose Codex brainstormer was relaunched once (ruling T8-7) and then
/// rethought: codex ran sessions 1 and 2 in round 1 and 3 in round 2, claude 1 and 2.
fn relaunched_and_rethought() -> Fixture {
    let mut fx = brainstorming();
    fx.run_mut().repo_dir = REPO.into();
    let reason = unsubmitted(&BRAINSTORMER_TEXTS);
    let quiet = ScoutEnd::Unsubmitted { reason };
    end_after(&mut fx, brainstormer("codex", 1), quiet, (100, 7, 1_000));
    started(&mut fx, "codex", CODEX + 1);
    drafts(&mut fx, (200, 10, 2_000), (150, 8, 3_000));
    submitted(&mut fx, "brainstorm", REPORT);
    let rethink = DocGateAction::Rethink {
        note: "Think about SSO.".into(),
    };
    act(&mut fx, DocGateKind::Brainstorm, rethink).unwrap();
    started(&mut fx, "claude", CLAUDE + 10);
    started(&mut fx, "codex", CODEX + 10);
    drafts(&mut fx, (120, 4, 500), (130, 6, 700));
    submitted(&mut fx, "brainstorm", REPORT);
    fx
}

/// Ruling T13-1: a rethink's sessions are routed `rethink`; a T8-7 relaunch stays
/// `relaunch`; a first session is `start`.
#[test]
fn a_rethink_is_routed_as_rethink_and_a_relaunch_as_relaunch() {
    let fx = relaunched_and_rethought();
    let trigger = |session: &str| {
        let found = (fx.run().role_routing_decisions.iter())
            .find(|d| d.role == AgentRole::Brainstormer && d.session_id == session);
        found
            .unwrap_or_else(|| panic!("no record {session}"))
            .trigger
            .clone()
    };
    assert_eq!(trigger("codex/1"), "start");
    assert_eq!(trigger("codex/2"), "relaunch");
    assert_eq!(trigger("codex/3"), "rethink");
    assert_eq!(trigger("claude/1"), "start");
    assert_eq!(trigger("claude/2"), "rethink");
}

/// Ruling T13-1: the brainstorming record sums each brainstormer's tokens, calls and
/// active time over all its sessions: the relaunch and both rounds.
#[test]
fn a_phase_record_sums_every_session_of_an_agent() {
    let mut fx = relaunched_and_rethought();
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    let lines = phase_lines(&fx);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let record = &lines[0].1;
    // In the order their sessions first ended: codex's relaunch came first.
    let runtimes: Vec<Runtime> = record.agents.iter().map(|a| a.route.runtime).collect();
    assert_eq!(runtimes, [Runtime::Codex, Runtime::Claude]);
    let claude = agent(record, 1);
    assert_eq!(
        claude,
        (AgentRole::Brainstormer, 10 + 4, 2_500, 200 + 120, "ok")
    );
    let codex = agent(record, 0);
    assert_eq!(
        codex,
        (
            AgentRole::Brainstormer,
            7 + 8 + 6,
            4_700,
            100 + 150 + 130,
            "ok"
        )
    );
    assert_eq!(record.gate_versions, 2);
    // The agents' own counts are summed too, not the last session's.
    let a = &agents(&fx)[1];
    assert_eq!((a.calls, a.tokens), (21, 4_700));
}

/// A session that ended without its draft is `failed: <reason>`, or `over budget` when
/// the scout machine's limits stopped it (one and a half times its tool calls, or its
/// minutes); its sums still count.
#[test]
fn a_failed_session_is_recorded_failed_or_over_budget() {
    let mut fx = brainstorming();
    let crashed = ScoutEnd::Failed {
        reason: "it crashed".into(),
    };
    end_after(&mut fx, brainstormer("claude", 1), crashed, (30, 4, 10));
    let stopped = ScoutEnd::Failed {
        reason: "the brainstormer used 60 tool calls without an accepted draft".into(),
    };
    end_after(&mut fx, brainstormer("codex", 1), stopped, (40, 60, 20));
    let design = fx.run().orch.design.as_ref().unwrap();
    let spend = design.phase_spend(1, "brainstorming").unwrap();
    let outcomes: Vec<(&str, u32, &str)> = (spend.agents.iter())
        .map(|a| (a.label.as_str(), a.calls, a.outcome.as_str()))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("claude", 4, "failed: it crashed"),
            ("codex", 60, "over budget")
        ]
    );
}
