//! Milestone 9.6 task M9.6.7: a design run's three gates (decisions 5 and 7, DF §2.1):
//! approve moves the run on, changes and back set the orchestrator revising, edit makes
//! the user's version, rethink is the brainstorm's, reject discards; every refusal is
//! exact (Messages), with brief ruling BD-2's caps; and ruling T1-O1's plain approve and
//! reject.

use proto::{ActionKind, DocAuthor, DocGateAction, DocGateKind, DocKind, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::engine::actions::{self, ActionNode};
use crate::run::engine::{EventKind, OpKind};

fn changes(note: &str) -> DocGateAction {
    DocGateAction::Changes {
        note: note.into(),
        review: false,
    }
}

/// Decision 7: approve at the brainstorm gate starts specifying, at the spec gate
/// planning; each wakes the orchestrator with its exact note.
#[test]
fn approve_moves_brainstorm_to_specifying_and_spec_to_planning() {
    let mut fx = at_brainstorm_gate(false);
    assert_eq!(gate(&fx), Some((DocGateKind::Brainstorm, 1, None)));
    let reply = act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve);
    assert_eq!(
        reply,
        Ok(format!(
            "run {RUN_ID}: the brainstorm v1 is approved; specifying"
        ))
    );
    assert_eq!(fx.run().state, RunState::Specifying);
    assert_eq!(gate(&fx), None);
    assert!(notes(&fx).contains(&"the user approved the brainstorm v1".to_string()));

    let reply = submitted(&mut fx, "spec", SPEC);
    assert_eq!(reply["version"], 1);
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, None)));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    assert!(notes(&fx).contains(&"the user approved the spec v1".to_string()));
}

/// Decision 7: changes keeps the gate open with `revising`, wakes the orchestrator with
/// the note, and its next submit opens v2 with a change summary (task 5's carry).
#[test]
fn changes_sets_revising_and_the_next_submit_opens_v2() {
    let mut fx = at_spec_gate(false);
    let reply = act(
        &mut fx,
        DocGateKind::Spec,
        changes("Say how long a link lives."),
    );
    assert_eq!(
        reply,
        Ok(format!(
            "run {RUN_ID}: the orchestrator revises the spec v1"
        ))
    );
    let note = "Say how long a link lives.".to_string();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, Some(note))));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let wake = "the user asked for changes to the spec v1: Say how long a link lives.";
    assert!(notes(&fx).contains(&wake.to_string()), "{:?}", notes(&fx));

    let revised = SPEC.replace("Expired tokens.", "Expired tokens.\nA link lives an hour.");
    let reply = submitted(&mut fx, "spec", &revised);
    assert_eq!(reply["version"], 2);
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 2, None)));
    let design = fx.run().orch.design.as_ref().unwrap();
    let v2 = design.find(DocKind::Spec, Some(2)).unwrap();
    assert_eq!(v2.author, DocAuthor::Orchestrator);
    assert_eq!(v2.reason, "revised: Say how long a link lives.");
    assert_eq!(v2.changes, ["~ Errors and edge cases: 1 line"]);
    // Its snapshot row carries the summary (the gate's data).
    let info = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let doc_gate = info.runs[0].doc_gate.clone().unwrap();
    assert_eq!(
        doc_gate.changes_summary,
        ["~ Errors and edge cases: 1 line"]
    );
}

/// Decision 7: the user's own text is the next version, edited by the user; the gate
/// stays open for an approve.
#[test]
fn edit_makes_a_user_version_that_still_needs_approve() {
    let mut fx = at_spec_gate(false);
    let text = SPEC.replace("Mail delays.", "Mail delays.\nSpam filters.");
    let reply = act(&mut fx, DocGateKind::Spec, DocGateAction::Edit { text });
    assert_eq!(reply, Ok(format!("run {RUN_ID}: the spec is now v2")));
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 2, None)));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let design = fx.run().orch.design.as_ref().unwrap();
    let v2 = design.find(DocKind::Spec, Some(2)).unwrap();
    assert_eq!(
        (&v2.author, v2.reason.as_str()),
        (&DocAuthor::User, "edited by you")
    );
    assert!(notes(&fx).contains(&"the user edited the spec (now v2)".to_string()));
    // The user's text passes the template too.
    let bad = DocGateAction::Edit {
        text: "# T\n".into(),
    };
    let refused = act(&mut fx, DocGateKind::Spec, bad);
    assert_eq!(
        refused,
        Err("the spec is missing the section \"## Goal and success criteria\"".into())
    );
    // At the plan gate the plan is edited task by task.
    let mut fx = at_plan_gate(false);
    let edit = DocGateAction::Edit { text: "x".into() };
    assert_eq!(
        act(&mut fx, DocGateKind::Plan, edit),
        Err("the plan is edited task by task with anthrex run edit".into())
    );
}

/// Decision 7: rethink is the brainstorm gate's only; it returns the run to
/// brainstorming with its note.
#[test]
fn rethink_only_at_brainstorm() {
    let mut fx = at_spec_gate(false);
    let rethink = DocGateAction::Rethink { note: "n".into() };
    assert_eq!(
        act(&mut fx, DocGateKind::Spec, rethink.clone()),
        Err("rethink is only for the brainstorm gate".into())
    );
    assert_eq!(
        act(&mut fx, DocGateKind::Plan, rethink.clone()),
        Err("rethink is only for the brainstorm gate".into())
    );
    let mut fx = at_brainstorm_gate(false);
    let reply = act(&mut fx, DocGateKind::Brainstorm, rethink);
    assert_eq!(
        reply,
        Ok(format!("run {RUN_ID}: the brainstorm is rethought"))
    );
    assert_eq!(fx.run().state, RunState::Brainstorming);
    assert_eq!(gate(&fx), None);
    let wake = "the user asked to rethink the brainstorm: n".to_string();
    assert!(notes(&fx).contains(&wake));
    // The merged report is submitted again in brainstorming: v2.
    assert_eq!(submitted(&mut fx, "brainstorm", REPORT)["version"], 2);
}

/// Decision 7: back reopens the previous gate as a changes request there; the later
/// document stays as a superseded version.
#[test]
fn back_reopens_the_previous_gate_with_the_note_as_changes() {
    let back = |note: &str| DocGateAction::Back { note: note.into() };
    let mut fx = at_brainstorm_gate(false);
    assert_eq!(
        act(&mut fx, DocGateKind::Brainstorm, back("x")),
        Err("back is only for the spec and plan gates".into())
    );
    let mut fx = at_spec_gate(false);
    let reply = act(&mut fx, DocGateKind::Spec, back("Prefer signed tokens."));
    assert_eq!(reply, Ok(format!("run {RUN_ID}: back to the brainstorm")));
    let note = Some("Prefer signed tokens.".to_string());
    assert_eq!(gate(&fx), Some((DocGateKind::Brainstorm, 1, note)));
    let wake = "the user went back to the brainstorm: Prefer signed tokens.".to_string();
    assert!(notes(&fx).contains(&wake));
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(
        design.find(DocKind::Spec, Some(1)).is_some(),
        "kept, superseded"
    );
    // The orchestrator revises the report there: v2 opens the brainstorm gate.
    assert_eq!(submitted(&mut fx, "brainstorm", REPORT)["version"], 2);
    assert_eq!(gate(&fx), Some((DocGateKind::Brainstorm, 2, None)));

    // From the plan gate, back to the spec.
    let mut fx = at_plan_gate(false);
    act(&mut fx, DocGateKind::Plan, back("R2 is wrong.")).unwrap();
    assert_eq!(
        gate(&fx),
        Some((DocGateKind::Spec, 1, Some("R2 is wrong.".into())))
    );
    // While the spec is revised, the plan is not touched (ruling T1-O4).
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert_eq!(
        refused(&effects),
        "edit_plan is for the planning phase; this run is at the spec gate"
    );
}

/// Decision 7: reject at any gate of round 1 discards the run, with its wake note.
#[test]
fn reject_discards_round_one() {
    for (mut fx, kind) in [
        (at_brainstorm_gate(false), DocGateKind::Brainstorm),
        (at_spec_gate(false), DocGateKind::Spec),
        (at_plan_gate(false), DocGateKind::Plan),
    ] {
        let reply = act(&mut fx, kind, DocGateAction::Reject);
        assert_eq!(reply, Ok(format!("run {RUN_ID} rejected; discarding it")));
        let (_, op) = fx.op("Discard");
        assert!(matches!(op, OpKind::Discard { .. }));
        let wake = format!("the user rejected the {}; run discarded", kind.label());
        assert!(notes(&fx).contains(&wake), "{:?}", notes(&fx));
        assert_eq!(gate(&fx), None);
    }
}

/// Decision 7's refusals, exact: an action at a gate the run is not at, and any but
/// reject while the orchestrator revises.
#[test]
fn an_action_at_the_wrong_gate_or_while_revising_is_refused_exactly() {
    let mut fx = design_launched(false);
    assert_eq!(
        act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve),
        Err(format!(
            "run {RUN_ID} is not waiting at the brainstorm gate"
        ))
    );
    let mut fx = at_spec_gate(false);
    assert_eq!(
        act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve),
        Err(format!(
            "run {RUN_ID} is not waiting at the brainstorm gate"
        ))
    );
    assert_eq!(
        act(&mut fx, DocGateKind::Plan, DocGateAction::Approve),
        Err(format!("run {RUN_ID} is not waiting at the plan gate"))
    );
    act(&mut fx, DocGateKind::Spec, changes("more")).unwrap();
    let revising = Err("the orchestrator is revising spec v1; wait for it".to_string());
    for action in [
        DocGateAction::Approve,
        changes("again"),
        DocGateAction::Edit { text: SPEC.into() },
        DocGateAction::Back { note: "b".into() },
    ] {
        assert_eq!(act(&mut fx, DocGateKind::Spec, action), revising);
    }
    assert_eq!(gate(&fx).and_then(|g| g.2), Some("more".into()));
    // A run without the flow has no document gate.
    let mut plain = super::orch::launched(false);
    assert_eq!(
        act(&mut plain, DocGateKind::Plan, DocGateAction::Approve),
        Err(format!("run {RUN_ID} does not use the design flow"))
    );
}

/// Brief ruling BD-2: a gate takes 6 versions and the brainstorm 3 rethinks; past
/// them the action is refused exactly.
#[test]
fn a_sixth_version_and_a_fourth_rethink_are_refused_exactly() {
    let mut fx = at_spec_gate(false);
    for n in 2..=6 {
        act(&mut fx, DocGateKind::Spec, changes("again")).unwrap();
        let text = SPEC.replace("Mail delays.", &format!("Mail delays {n}."));
        assert_eq!(submitted(&mut fx, "spec", &text)["version"], n);
    }
    let full = Err("the spec has had its 6 versions; approve, go back or reject".to_string());
    assert_eq!(act(&mut fx, DocGateKind::Spec, changes("seventh")), full);
    let edit = DocGateAction::Edit { text: SPEC.into() };
    assert_eq!(act(&mut fx, DocGateKind::Spec, edit), full);
    // Approve, back and reject still pass.
    assert!(act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).is_ok());

    let mut fx = at_brainstorm_gate(false);
    for _ in 0..3 {
        let rethink = DocGateAction::Rethink { note: "r".into() };
        act(&mut fx, DocGateKind::Brainstorm, rethink).unwrap();
        submitted(&mut fx, "brainstorm", REPORT);
    }
    let rethink = DocGateAction::Rethink { note: "r".into() };
    assert_eq!(
        act(&mut fx, DocGateKind::Brainstorm, rethink),
        Err("the brainstorm has been rethought 3 times; approve, change or reject".into())
    );
    assert!(act(&mut fx, DocGateKind::Brainstorm, changes("c")).is_ok());
}

/// Ruling T1-O1: a plain approve at the brainstorm or spec gate is refused, exactly,
/// in the request and in the action menu, which offers the document's review instead;
/// a plain reject is the gate's.
#[test]
fn a_plain_approve_at_a_document_gate_is_refused_and_the_menu_offers_review() {
    for (mut fx, kind) in [
        (at_brainstorm_gate(false), "brainstorm"),
        (at_spec_gate(false), "spec"),
    ] {
        let text = format!(
            "run {RUN_ID} is waiting at the {kind} gate; approve it with anthrex run approve {RUN_ID} --gate {kind}"
        );
        assert_eq!(replies(&fx.approve()), vec![Err(text.clone())]);
        assert_eq!(fx.run().state, RunState::AwaitingApproval);
        let check = actions::check(fx.run(), &ActionNode::Run, &ActionKind::Approve);
        assert_eq!(check, Err(text));
        let listed: Vec<ActionKind> = actions::available(fx.run(), &ActionNode::Run)
            .into_iter()
            .map(|a| a.kind)
            .collect();
        assert!(listed.contains(&ActionKind::ReviewDoc), "{listed:?}");
        assert!(listed.contains(&ActionKind::Reject), "{listed:?}");
        assert!(!listed.contains(&ActionKind::Approve), "{listed:?}");
    }
    let fx = at_plan_gate(false);
    let check = actions::check(fx.run(), &ActionNode::Run, &ActionKind::ReviewDoc);
    assert_eq!(
        check,
        Err(format!("run {RUN_ID} has no document waiting for review"))
    );
}

/// Decision 7: `run approve` and `run reject` at a design run's plan gate act as the
/// plan gate's approve and reject.
#[test]
fn approve_and_reject_requests_act_at_a_design_plan_gate() {
    let mut fx = at_plan_gate(false);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    let design = fx.run().orch.design.as_ref().unwrap();
    let plan = design.find(DocKind::Plan, Some(1)).unwrap();
    assert_eq!(plan.reason, "submitted");
    assert_eq!(
        replies(&fx.approve()),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(gate(&fx), None);
    assert!(notes(&fx).contains(&"the user approved the plan v1".to_string()));

    let mut fx = at_plan_gate(false);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID} rejected; discarding it"))]
    );
    let wake = "the user rejected the plan; run discarded".to_string();
    assert!(notes(&fx).contains(&wake));
}

/// Decision 7 at the plan gate: changes there, then the orchestrator's next submit is
/// the plan's v2.
#[test]
fn changes_at_the_plan_gate_make_the_next_submit_v2() {
    let mut fx = at_plan_gate(false);
    act(&mut fx, DocGateKind::Plan, changes("Split t1.")).unwrap();
    let args = json!({"edits": [add_task("t2")], "submit": true});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    let design = fx.run().orch.design.as_ref().unwrap();
    let v2 = design.find(DocKind::Plan, Some(2)).unwrap();
    assert_eq!(v2.reason, "revised: Split t1.");
    assert!(!v2.changes.is_empty(), "{:?}", v2.changes);
    // Without a changes request, a submit at the open gate stores nothing.
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
}
