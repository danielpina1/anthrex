//! Milestone 9.6 task M9.6.7: a design run's phases (decisions 4, 6 and 8, DF §2): it
//! enters brainstorming with its scouts, `--yes` skips none of its gates, its tools are
//! admitted by phase (ruling T1-O4), each phase's clock and budget, the gate across a
//! restart (task 5's carry: what cannot be read back is never shown), and a lost window
//! while the orchestrator revises (review focus 1).

use proto::{DocGateAction, DocGateKind, DocKind, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, launched, orch_tool};
use super::orch_restore::resume;
use crate::run::design::state::DesignState;
use crate::run::engine::design::YES_LINE;
use crate::run::engine::{DocChecked, EngineState, EventKind, OpKind, OrchEvent};

/// Decision 4: a design goal starts in brainstorming, its orchestrator launched, and a
/// scout it asks for runs there as in planning.
#[test]
fn a_design_goal_enters_brainstorming_and_scouts_run() {
    let mut fx = design_launched(false);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    assert_eq!(fx.run().orch.design, Some(DesignState::default()));
    assert!(log_lines(&fx).contains(&"started; brainstorming with its orchestrator".into()));
    let args = json!({"id": "s1", "question": "Where is auth?", "area": ["crates/auth/**"]});
    let effects = orch_tool(&mut fx, ORCH, "spawn_scout", args);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(
        fx.ops("StartScout").len(),
        1,
        "the scout starts in brainstorming"
    );
    // The digest names the phase (M-2).
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    assert_eq!(digest["gate"]["state"], "brainstorming");
    // `start_brainstorm` is brainstorming's, once.
    start_brainstorm(&mut fx);
    let again = orch_tool(&mut fx, ORCH, "start_brainstorm", json!({"answers": ""}));
    assert_eq!(refused(&again), "the brainstormers are already running");
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    let late = orch_tool(&mut fx, ORCH, "start_brainstorm", json!({"answers": ""}));
    assert_eq!(
        refused(&late),
        "start_brainstorm is only for the brainstorming phase"
    );
}

/// Decision 6: `--yes` skips none of the three gates, says so once, and the first
/// prompt does not say the gate is off.
#[test]
fn yes_never_skips_a_design_gate_and_logs_once() {
    let fx = at_plan_gate(true);
    assert_eq!(
        fx.run().state,
        RunState::AwaitingApproval,
        "the plan gate waits"
    );
    assert_eq!(gate(&fx).map(|g| g.0), Some(DocGateKind::Plan));
    let yes: Vec<_> = log_lines(&fx)
        .into_iter()
        .filter(|l| l == YES_LINE)
        .collect();
    assert_eq!(yes.len(), 1);
    let prompt = &fx.run().orch.orchestrator.as_ref().unwrap().first_prompt;
    assert!(!prompt.contains("Plan gate: off"), "{prompt}");
    // The brainstorm and spec gates waited too (`at_plan_gate` approved each).
    let opened: Vec<_> = (log_lines(&fx).into_iter())
        .filter(|l| l.ends_with("awaits the user"))
        .collect();
    assert_eq!(
        opened,
        [
            "the brainstorm v1 awaits the user",
            "the spec v1 awaits the user",
            "the plan v1 awaits the user"
        ]
    );
    // A run without the flow still skips its gate with --yes.
    let mut plain = launched(true);
    let args = json!({"edits": [super::orch::add("t1", "auth")], "submit": true});
    orch_tool(&mut plain, ORCH, "edit_plan", args);
    assert_eq!(plain.run().state, RunState::Running);
}

/// Ruling T1-O4: plan tools before planning, exactly refused, the run unchanged; and a
/// document submitted outside its phase.
#[test]
fn plan_tools_before_planning_are_refused_exactly_and_the_run_stays() {
    let subplanner = json!({"epic": "e1", "title": "E", "area": ["crates/e/**"], "brief": "b"});
    let mut fx = at_brainstorm_gate(false);
    let before = fx.run().clone();
    let effects = orch_tool(&mut fx, ORCH, "spawn_subplanner", subplanner.clone());
    assert_eq!(
        refused(&effects),
        "spawn_subplanner is for the planning phase; this run is at the brainstorm gate"
    );
    let effects = orch_tool(
        &mut fx,
        ORCH,
        "edit_plan",
        json!({"edits": [add_task("t1")]}),
    );
    assert_eq!(
        refused(&effects),
        "edit_plan is for the planning phase; this run is at the brainstorm gate"
    );
    assert_eq!(
        (fx.run().state, &fx.run().tasks),
        (before.state, &before.tasks)
    );
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    let effects = orch_tool(&mut fx, ORCH, "spawn_subplanner", subplanner);
    assert_eq!(
        refused(&effects),
        "spawn_subplanner is for the planning phase; this run is specifying"
    );
    assert_eq!(fx.run().state, RunState::Specifying);
    assert!(fx.run().orch.epics.is_empty());
    // A brainstorm report while specifying is refused; so is the spec at its gate
    // when nobody asked for a revision.
    let effects = submit(&mut fx, "brainstorm", REPORT);
    assert_eq!(
        refused(&effects),
        "submit_doc kind \"brainstorm\" is for the brainstorming phase; this run is specifying"
    );
    submitted(&mut fx, "spec", SPEC);
    let effects = submit(&mut fx, "spec", SPEC);
    assert_eq!(
        refused(&effects),
        "submit_doc kind \"spec\" is for the specifying phase; this run is at the spec gate"
    );
    let mut fx = design_launched(false);
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert_eq!(
        refused(&effects),
        "edit_plan is for the planning phase; this run is brainstorming"
    );
}

/// Task 6's carry (m4): a design tool in a run without the flow.
#[test]
fn design_tools_in_a_run_without_the_flow_are_refused() {
    let mut fx = launched(false);
    for (tool, args) in [
        ("start_brainstorm", json!({"answers": ""})),
        (
            "submit_doc",
            json!({"kind": "spec", "text": SPEC, "ready": true}),
        ),
    ] {
        let effects = orch_tool(&mut fx, ORCH, tool, args);
        assert_eq!(
            refused(&effects),
            format!("run {RUN_ID} does not use the design flow")
        );
    }
}

/// Decision 8: brainstorming's clock starts when both drafts are in, not when the
/// phase starts.
#[test]
fn the_brainstorming_clock_starts_when_the_drafts_are_in() {
    let mut fx = design_launched(false);
    start_brainstorm(&mut fx);
    let late = fx.now + 3 * 3600;
    fx.send(late, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Brainstorming, "no clock yet");
    assert_eq!(fx.run().orch.design.as_ref().unwrap().phase_started, None);
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.answers = None;
    drafts_in(&mut fx);
    let started = fx.run().orch.design.as_ref().unwrap().phase_started;
    assert_eq!(started, Some(fx.now));
    let wake = "both brainstorm drafts are in; read them with get_doc and submit the merged report";
    assert!(notes(&fx).contains(&wake.to_string()));
    // One failure wakes it with the other note.
    let mut fx = design_launched(false);
    let now = fx.now;
    crate::run::engine::design::drafts_in(fx.run_mut(), Some(("codex", "over budget")), now);
    let wake = "one brainstormer failed (codex: over budget); read the other draft with get_doc and submit the merged report";
    assert!(notes(&fx).contains(&wake.to_string()));
}

/// Decision 8: past `phase_minutes` the run halts with the exact text; a plain `run
/// resume` returns it to its phase and restarts the clock (F-3).
#[test]
fn a_phase_over_budget_halts_and_resume_restarts_its_clock() {
    let mut fx = at_brainstorm_gate(false);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    let started = fx.now;
    fx.send(started + 60 * 60, EventKind::Tick);
    assert_eq!(
        fx.run().state,
        RunState::Specifying,
        "60 min is within budget"
    );
    fx.send(started + 60 * 60 + 1, EventKind::Tick);
    let text = "design flow: the specifying phase passed its 60 min budget";
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text));
    assert!(log_lines(&fx).contains(&text.to_string()));
    // Its tools wait for the user.
    let effects = submit(&mut fx, "spec", SPEC);
    assert_eq!(refused(&effects), format!("run {RUN_ID} is halted"));
    let effects = resume(&mut fx);
    assert_eq!(replies(&effects), vec![Ok(format!("run {RUN_ID} resumed"))]);
    assert_eq!(fx.run().state, RunState::Specifying);
    assert_eq!(fx.run().halted_reason, None);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.phase_started, Some(fx.now), "the clock restarts");
    // The halted time is paused time: a full budget again from the resume.
    let resumed = fx.now;
    fx.send(resumed + 60 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Specifying);
    // A revision at a gate has its phase's clock too.
    let mut fx = at_spec_gate(false);
    let changes = DocGateAction::Changes {
        note: "n".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Spec, changes).unwrap();
    let asked = fx.now;
    fx.send(asked + 60 * 60 + 1, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(gate(&fx).and_then(|g| g.2), Some("n".into()));
}

/// DF §2.2: a gate's wait never counts, nor does paused time (a daemon restart's
/// downtime).
#[test]
fn gate_wait_and_pause_do_not_count_toward_the_phase_budget() {
    let mut fx = at_spec_gate(false);
    // A day at the gate.
    fx.send(fx.now + 24 * 3600, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    let started = fx.now;
    // A restart at once: the run is paused, and its downtime of three hours is paused
    // time (decision 15's bookkeeping).
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    let restore = EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    };
    fx.send(started + 10, restore);
    assert_eq!(fx.run().state, RunState::Paused);
    fx.send(started + 2 * 3600, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Paused);
    let reply = fx.reply();
    let back = started + 3 * 3600;
    let resume = EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    };
    fx.send(back, resume);
    assert_eq!(fx.run().state, RunState::Planning);
    // 59 unpaused minutes after the resume: within budget, though four hours passed.
    fx.send(back + 59 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Planning);
    fx.send(back + 61 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
    let text = "design flow: the planning phase passed its 60 min budget";
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text));
}

/// DF §2.1: an open gate and every version survive a daemon restart (run.json's round
/// trip); the restored orchestrator is relaunched by `run resume`, and the gate's
/// actions still work.
#[test]
fn the_gate_survives_a_restart() {
    let mut fx = at_spec_gate(false);
    let stored = serde_json::to_string(fx.run()).unwrap();
    let run: crate::run::model::Run = serde_json::from_str(&stored).unwrap();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs: vec![run],
        replay: Vec::new(),
        held: Vec::new(),
    });
    assert_eq!(
        fx.run().state,
        RunState::AwaitingApproval,
        "a gate is not paused"
    );
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, None)));
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.versions.len(), 2);
    let effects = resume(&mut fx);
    assert_eq!(ops_in(&effects, "RestartOrchestrator").len(), 1);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
}

/// Task 5's carry: what the restore could not read back is logged once; the open
/// gate's version among it reopens the gate as revising, and the orchestrator is told
/// to submit it again. The latest texts refill the change summaries.
#[test]
fn a_gate_whose_version_cannot_be_read_back_reopens_revising() {
    let mut fx = at_spec_gate(false);
    fx.run_mut().orch.design.as_mut().unwrap().texts.clear();
    let checked = vec![
        DocChecked {
            kind: DocKind::Brainstorm,
            n: 1,
            read: Ok(Some(REPORT.into())),
        },
        DocChecked {
            kind: DocKind::Spec,
            n: 1,
            read: Err("its file differs from what was stored".into()),
        },
    ];
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked,
    });
    let line =
        "design flow: the spec v1 could not be read back: its file differs from what was stored";
    let lines: Vec<_> = log_lines(&fx).into_iter().filter(|l| l == line).collect();
    assert_eq!(lines.len(), 1);
    let note = "the stored spec v1 could not be read back; submit it again".to_string();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, Some(note))));
    let wake = "the spec v1 could not be read back after a restart; submit it again";
    assert!(notes(&fx).contains(&wake.to_string()));
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.text_of(DocKind::Brainstorm), Some((1, REPORT)));
    // The orchestrator submits it again: v2.
    assert_eq!(submitted(&mut fx, "spec", SPEC)["version"], 2);
}

/// Review focus 1: the orchestrator's window is lost while it revises. The run's
/// handoff takes it fresh, the fresh session gets the note again, and reject (`x`)
/// still works.
#[test]
fn a_lost_window_while_revising_hands_off_with_the_note_and_x_still_works() {
    let mut fx = at_spec_gate(false);
    let changes = DocGateAction::Changes {
        note: "Name the token store.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Spec, changes).unwrap();
    let wake = "the user asked for changes to the spec v1: Name the token store.".to_string();
    // The old session read its notes, then its window was lost.
    let seq = crate::run::engine::notes_seq(fx.run());
    fx.next(EventKind::Orch(OrchEvent::DigestRead {
        run_id: RUN_ID.into(),
        digest_revision: 0,
        notes_seq: seq,
    }));
    assert!(!notes(&fx).contains(&wake));
    fx.next(EventKind::Orch(OrchEvent::AdoptLost {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        first_prompt: "the handoff".into(),
    }));
    assert!(notes(&fx).contains(&wake), "{:?}", notes(&fx));
    let effects = resume(&mut fx);
    let launches = ops_in(&effects, "CreateOrchestrator");
    assert_eq!(launches.len(), 1, "a fresh session: {effects:?}");
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.first_prompt, "the handoff");
    assert!(notes(&fx).contains(&wake));
    // Every other action waits for the revision; reject does not.
    assert!(act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).is_err());
    let reply = act(&mut fx, DocGateKind::Spec, DocGateAction::Reject);
    assert_eq!(reply, Ok(format!("run {RUN_ID} rejected; discarding it")));
    assert!(matches!(fx.op("Discard").1, OpKind::Discard { .. }));
}
