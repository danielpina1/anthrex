//! Milestone 9.6 task M9.6.7, fix round 1: what the orchestrator and the user are told.
//! A fresh session is told again what its revision is for, in its own words: changes,
//! back, or the engine's read-back (m3), after a lost window or any fresh restart
//! (Claude's or Codex's); a reject before any plan names the phase's document (m6); and
//! BD-2's caps at a full gate (ruling T7-6, m4).

use proto::{DocGateAction, DocGateKind, DocKind, RunState, Runtime};

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::ORCH;
use super::orch_restore::{restart, resume};
use crate::run::engine::{DocChecked, EventKind, OpResult, OrchEvent, notes_seq};

fn changes(note: &str) -> DocGateAction {
    DocGateAction::Changes {
        note: note.into(),
        review: false,
    }
}

/// The orchestrator's session read every note so far.
fn read_notes(fx: &mut Fixture) {
    let seq = notes_seq(fx.run());
    fx.next(EventKind::Orch(OrchEvent::DigestRead {
        run_id: RUN_ID.into(),
        digest_revision: 0,
        notes_seq: seq,
    }));
    assert!(notes(fx).is_empty(), "{:?}", notes(fx));
}

/// Its window was lost: the handoff takes it fresh.
fn adopt_lost(fx: &mut Fixture) {
    fx.next(EventKind::Orch(OrchEvent::AdoptLost {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        first_prompt: "the handoff".into(),
    }));
}

/// Review m3: after a back, the fresh session is told the user went back; after the
/// restore's read-back, it is told what anthrex could not read back, never in the
/// user's name.
#[test]
fn a_fresh_session_is_told_what_its_revision_is_for() {
    let mut fx = at_plan_gate(false);
    let back = DocGateAction::Back {
        note: "R2 is wrong.".into(),
    };
    act(&mut fx, DocGateKind::Plan, back).unwrap();
    read_notes(&mut fx);
    adopt_lost(&mut fx);
    let wake = "the user went back to the spec: R2 is wrong.".to_string();
    assert_eq!(notes(&fx), vec![wake]);

    let mut fx = at_spec_gate(false);
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked: vec![DocChecked {
            kind: DocKind::Spec,
            n: 1,
            read: Err("its file differs from what was stored".into()),
        }],
    });
    let note = "anthrex could not read back the stored spec v1; submit it again".to_string();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, Some(note))));
    read_notes(&mut fx);
    adopt_lost(&mut fx);
    let wake = "the spec v1 could not be read back after a restart; submit it again".to_string();
    assert_eq!(notes(&fx), vec![wake]);
}

/// The review's restart gap: a Codex orchestrator's fresh restart is told its revision
/// again too.
#[test]
fn a_codex_fresh_restart_is_told_its_revision_again() {
    let mut fx = at_spec_gate(false);
    let route = &mut fx.run_mut().orch.orchestrator.as_mut().unwrap().route;
    route.runtime = Runtime::Codex;
    act(&mut fx, DocGateKind::Spec, changes("Name the token store.")).unwrap();
    read_notes(&mut fx);
    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    fx.done(restarts[0].0, OpResult::RestartedFresh);
    let wake = "the user asked for changes to the spec v1: Name the token store.".to_string();
    assert!(notes(&fx).contains(&wake), "{:?}", notes(&fx));
}

/// Review m6: a reject in brainstorming or specifying (or paused there) names that
/// phase's document; no plan exists yet.
#[test]
fn a_reject_before_any_plan_names_the_phase_document() {
    let reject = |fx: &mut Fixture| {
        let reply = fx.reply();
        let effects = fx.next(EventKind::Reject {
            reply,
            run_id: RUN_ID.into(),
        });
        assert_eq!(
            replies(&effects),
            vec![Ok(format!("run {RUN_ID} rejected; discarding it"))]
        );
    };
    let mut fx = design_launched(false);
    reject(&mut fx);
    let wake = "the user rejected the brainstorm; run discarded".to_string();
    assert!(notes(&fx).contains(&wake), "{:?}", notes(&fx));

    let mut fx = at_brainstorm_gate(false);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    reject(&mut fx);
    let wake = "the user rejected the spec; run discarded".to_string();
    assert!(notes(&fx).contains(&wake), "{:?}", notes(&fx));
}

/// Ruling T7-6 at the brainstorm gate's cap (a rethink still passes it: its own cap is
/// the 3 rethinks); review m4: back and reject pass a full gate.
#[test]
fn a_full_gate_still_takes_its_other_actions() {
    let mut fx = at_brainstorm_gate(false);
    for n in 2..=6 {
        act(&mut fx, DocGateKind::Brainstorm, changes("again")).unwrap();
        let text = REPORT.replace("A table.", &format!("A table {n}."));
        assert_eq!(submitted(&mut fx, "brainstorm", &text)["version"], n);
    }
    let full = "the brainstorm has had its 6 versions; approve, rethink or reject".to_string();
    assert_eq!(
        act(&mut fx, DocGateKind::Brainstorm, changes("seventh")),
        Err(full)
    );
    let rethink = DocGateAction::Rethink { note: "r".into() };
    assert!(act(&mut fx, DocGateKind::Brainstorm, rethink).is_ok());

    let full_spec = || {
        let mut fx = at_spec_gate(false);
        for n in 2..=6 {
            act(&mut fx, DocGateKind::Spec, changes("again")).unwrap();
            let text = SPEC.replace("Mail delays.", &format!("Mail delays {n}."));
            assert_eq!(submitted(&mut fx, "spec", &text)["version"], n);
        }
        fx
    };
    let mut fx = full_spec();
    let back = DocGateAction::Back { note: "b".into() };
    let reply = act(&mut fx, DocGateKind::Spec, back);
    assert_eq!(reply, Ok(format!("run {RUN_ID}: back to the brainstorm")));
    let mut fx = full_spec();
    let reply = act(&mut fx, DocGateKind::Spec, DocGateAction::Reject);
    assert_eq!(reply, Ok(format!("run {RUN_ID} rejected; discarding it")));
}

/// Fix round 2: with both caps reached at the brainstorm gate, the refusal offers only
/// what is left, approve or reject, for a rethink and for a change alike.
#[test]
fn both_caps_reached_offer_approve_or_reject() {
    let mut fx = at_brainstorm_gate(false);
    for _ in 0..3 {
        let rethink = DocGateAction::Rethink { note: "r".into() };
        act(&mut fx, DocGateKind::Brainstorm, rethink).unwrap();
        redrafts_in(&mut fx);
        submitted(&mut fx, "brainstorm", REPORT);
    }
    for n in 5..=6 {
        act(&mut fx, DocGateKind::Brainstorm, changes("again")).unwrap();
        let text = REPORT.replace("A table.", &format!("A table {n}."));
        assert_eq!(submitted(&mut fx, "brainstorm", &text)["version"], n);
    }
    let both = Err(
        "the brainstorm has been rethought 3 times and has had its 6 versions; approve or reject"
            .to_string(),
    );
    let rethink = DocGateAction::Rethink { note: "r".into() };
    assert_eq!(act(&mut fx, DocGateKind::Brainstorm, rethink), both);
    assert_eq!(act(&mut fx, DocGateKind::Brainstorm, changes("c")), both);
    assert!(act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).is_ok());
}
