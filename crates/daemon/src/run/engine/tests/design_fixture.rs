//! Milestone 9.6 task M9.6.7's fixtures: a planned run with the design flow on, its
//! orchestrator launched, and documents that pass their templates. The brainstormers
//! (task M9.6.8) are stubbed by their records and `design::drafts_in`.

use std::collections::BTreeMap;

use proto::{
    AgentRole, DesignMode, DocAuthor, DocGateAction, DocGateKind, DocKind, Effort, Route, RunPath,
    RunState, Runtime, Strength,
};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, error, first_turn_woken, mcp_ready, orch_tool, triage};
use crate::run::design::state::{DesignAgent, DesignAgentState, DesignState, NewDoc};
use crate::run::engine::{Effect, EventKind, OpResult, design};
use crate::run::orch::launch::resolve_orchestrator;
use crate::run::orch::make_planned;

pub(super) const REPORT: &str = "\
## Where they agree
Both want tokens.

## Where they disagree
claude: stateless. codex: stored. Judgment: stored.

## Approaches
### 1. Signed tokens [claude]
Stateless.
### 2. Stored tokens [both]
A table.

## Recommendation
Go with Stored tokens, because links must be revocable.

## Questions for you
- How long should a link live?
";

pub(super) const SPEC: &str = "\
# Password reset

## Goal and success criteria
Users reset their password.

## Non-goals
SSO.

## Approach
Stored tokens.

## Design
A table of tokens.

## Requirements
R1 Tokens expire after an hour. Check: a clock test.
R2 Links are single use. Check: a reuse test.

## Interfaces
`reset(token)`.

## Errors and edge cases
Expired tokens.

## Testing
A clock test and a reuse test.

## Risks
Mail delays.

## Open questions
";

/// A design run, started (decision 4: in brainstorming); its run branch and its
/// orchestrator not yet made.
pub(super) fn design_planned(yes: bool) -> Fixture {
    let text = plan_with(PROFILE, &[task("t0", "S", "auth", "")]);
    let mut fx = Fixture::new(&text);
    let mut plan = crate::run::plan::parse_plan(&text).unwrap();
    plan.tasks.clear();
    let mut run = crate::run::plan::build_run(
        plan,
        preflight(),
        crate::run::plan::BuildContext {
            tuning: Default::default(),
            id: RUN_ID.to_string(),
            wt_dir: WT.into(),
            data_dir: format!("/tmp/data/runs/{RUN_ID}").into(),
            config: &fx.config,
            testing: &config::Testing::default(),
            now: 1_000,
            yes: false,
            delivery: &config::Delivery::default(),
        },
    )
    .unwrap_or_else(|e| panic!("an empty plan builds: {e:?}"));
    // The driver froze the mode before (`driver/build.rs::make_planned`).
    run.design_mode = DesignMode::Full;
    let agent = config::AgentConfig::default();
    let default = run.limits.default_runtime;
    let resolved = resolve_orchestrator(None, &agent, default, &run.roster).unwrap();
    let triage = Some(triage(RunPath::Plan));
    make_planned(&mut run, triage, resolved, yes, BTreeMap::new());
    let reply = fx.reply();
    fx.next(EventKind::Start {
        reply,
        run: Box::new(run),
    });
    fx
}

/// [`design_planned`], its run branch made and its orchestrator in window [`ORCH`],
/// first turn delivered.
pub(super) fn design_launched(yes: bool) -> Fixture {
    let mut fx = design_planned(yes);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (op, _) = fx.op("CreateOrchestrator");
    let window = OpResult::Window {
        window_id: ORCH,
        pid: None,
    };
    fx.done(op, window);
    mcp_ready(&mut fx, ORCH);
    first_turn_woken(&mut fx);
    fx
}

fn brainstormer(label: &str, runtime: Runtime) -> DesignAgent {
    DesignAgent {
        label: label.into(),
        role: AgentRole::Brainstormer,
        route: Route {
            runtime,
            model: "m".into(),
            strength: Strength::Frontier,
            effort: Effort::High,
        },
        session: 1,
        window_id: None,
        state: DesignAgentState::Done,
        calls: 0,
        tokens: 0,
        started: None,
        listed: false,
        unsubmitted: false,
    }
}

/// The brainstorm started, and both drafts in (stubbing task M9.6.8): the two
/// brainstormers' records and their stored drafts, then `design::drafts_in`, at the
/// fixture's time.
pub(super) fn drafts_in(fx: &mut Fixture) {
    start_brainstorm(fx);
    let design = fx.run_mut().orch.design.as_mut().expect("a design run");
    design.brainstormers = vec![
        brainstormer("claude", Runtime::Claude),
        brainstormer("codex", Runtime::Codex),
    ];
    redrafts_in(fx);
}

/// Each brainstormer's draft stored and held in memory, as `design_drafts::flush`
/// does (ruling T9-1a: the merged report needs each draft's text), then
/// `design::drafts_in`. After a rethink (task M9.6.9 relaunches both brainstormers), the
/// next round's drafts.
pub(super) fn redrafts_in(fx: &mut Fixture) {
    let now = fx.now;
    let run = fx.run_mut();
    let labels: Vec<String> = (run.orch.design.as_ref().unwrap().brainstormers.iter())
        .map(|a| a.label.clone())
        .collect();
    for label in labels {
        let author = DocAuthor::Brainstormer {
            label: label.clone(),
        };
        let text = format!("## Understanding\n{label}'s draft\n");
        let doc = NewDoc::new(DocKind::BrainstormDraft, author, "submitted", &text);
        let (version, _) = crate::run::design::state::store(run, doc, now).unwrap();
        let design = run.orch.design.as_mut().unwrap();
        design.keep_text(version.kind, version.n, text);
    }
    let design = run.orch.design.as_mut().expect("a design run");
    for agent in design.brainstormers.iter_mut() {
        agent.state = DesignAgentState::Done;
    }
    design::drafts_in(run, None, now);
}

pub(super) fn start_brainstorm(fx: &mut Fixture) -> Vec<Effect> {
    let effects = orch_tool(fx, ORCH, "start_brainstorm", json!({"answers": "skip"}));
    assert_eq!(replies(&effects).len(), 1);
    effects
}

/// The orchestrator's `submit_doc`.
pub(super) fn submit(fx: &mut Fixture, kind: &str, text: &str) -> Vec<Effect> {
    let args = json!({"kind": kind, "text": text, "ready": true});
    orch_tool(fx, ORCH, "submit_doc", args)
}

/// [`submit`], accepted: its reply.
pub(super) fn submitted(fx: &mut Fixture, kind: &str, text: &str) -> Value {
    let effects = submit(fx, kind, text);
    match &replies(&effects)[..] {
        [Ok(text)] => serde_json::from_str(text).unwrap(),
        other => panic!("{kind} refused: {other:?}"),
    }
}

/// A user's action at a gate: its one reply.
pub(super) fn act(
    fx: &mut Fixture,
    kind: DocGateKind,
    action: DocGateAction,
) -> Result<String, String> {
    let reply = fx.reply();
    let effects = fx.next(EventKind::DocGate {
        reply,
        run_id: RUN_ID.into(),
        kind,
        action,
    });
    let mut all = replies(&effects);
    assert_eq!(all.len(), 1, "{effects:?}");
    all.remove(0)
}

/// A design run at the brainstorm gate, v1.
pub(super) fn at_brainstorm_gate(yes: bool) -> Fixture {
    let mut fx = design_launched(yes);
    drafts_in(&mut fx);
    submitted(&mut fx, "brainstorm", REPORT);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx
}

/// A design run at the spec gate, v1.
pub(super) fn at_spec_gate(yes: bool) -> Fixture {
    let mut fx = at_brainstorm_gate(yes);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    submitted(&mut fx, "spec", SPEC);
    fx
}

/// An `add_task` edit whose brief has the design flow's five headings.
pub(super) fn add_task(id: &str) -> Value {
    let brief = "Files: a\nTests first: t\nSteps: s\nAcceptance: a\nVerify: v";
    json!({"op": "add_task", "task": {
        "id": id, "title": format!("Title {id}"), "size": "S",
        "owns": [format!("crates/{id}/**")],
        "brief": brief, "acceptance": [format!("Accept {id}")]
    }})
}

/// A design run at the plan gate, v1: its plan submitted by the orchestrator.
pub(super) fn at_plan_gate(yes: bool) -> Fixture {
    let mut fx = at_spec_gate(yes);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    let args = json!({"edits": [add_task("t1")], "submit": true});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    fx
}

/// [`at_plan_gate`], the user's changes asked: the orchestrator revises the plan.
pub(super) fn plan_revising() -> Fixture {
    let mut fx = at_plan_gate(false);
    let changes = DocGateAction::Changes {
        note: "Split t1.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Plan, changes).unwrap();
    fx
}

/// [`at_spec_gate`], the spec approved, then halted by the planning phase's budget.
pub(super) fn budget_halted() -> Fixture {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    let late = fx.now + u64::from(fx.run().limits.orch.design.phase_minutes) * 60 + 1;
    fx.send(late, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
    fx
}

/// The open gate: kind, version, revising.
pub(super) fn gate(fx: &Fixture) -> Option<(DocGateKind, u32, Option<String>)> {
    let design: &DesignState = fx.run().orch.design.as_ref()?;
    let g = design.gate.as_ref()?;
    Some((g.kind, g.version, g.revising.clone()))
}

/// The orchestrator's pending wake notes.
pub(super) fn notes(fx: &Fixture) -> Vec<String> {
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    o.notes.clone()
}

/// Task M9.6.9: the merged report's template, the second note of the drafts-in wake,
/// with the failed brainstormer when one failed.
pub(super) fn template(fx: &Fixture, failed: Option<(&str, &str)>) -> String {
    design::report::template_note(fx.run(), failed)
}

/// The run's log lines.
pub(super) fn log_lines(fx: &Fixture) -> Vec<String> {
    fx.run().log.iter().map(|e| e.text.clone()).collect()
}

/// The refusal text of an orchestrator tool call.
pub(super) fn refused(effects: &[Effect]) -> String {
    error(effects)
}
