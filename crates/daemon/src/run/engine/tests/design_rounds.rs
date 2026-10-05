//! Milestone 9.6 task M9.6.15 (decisions 28 and 29, DF §8.1): a design run's later
//! rounds. `run iterate` amends the spec by default (specifying), `full` brainstorms
//! first, `off` is 9.3's round, and a run without the flow takes only `off`. The
//! amendment continues the approved numbering from the round's base, which a back
//! inside the round never clears; an approved amendment's changed requirements carry
//! their marker (ruling T4-4); the round's plan covers only what the amendment added or
//! changed; a reject at a round's gate drops only the round; an unasked later spec
//! cycle is marked not reviewed (task 10's carry); and a halt before the round's plan
//! is approved resumes into its phase (task 7's carry).

use proto::{DocGateAction, DocGateKind, DocKind, RoundDesign, RoundOutcome, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::design_plan_fixture::covering;
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::goal_rounds_start::{complete, reply, started};
use crate::run::design::requirements::Requirement;
use crate::run::engine::{EventKind, design::review::UNREVIEWED};

/// Decision 28's refusal of a design on a run without the flow.
const NO_SPEC: &str = "this run has no spec to amend; iterate with --design off";

/// A task of round 2's first stage covering `covers`, its brief in the design shape.
pub(super) fn round_task(id: &str, covers: &[&str]) -> serde_json::Value {
    let mut edit = covering(id, covers);
    edit["task"]["stage"] = json!(2);
    edit
}

fn requirement(id: &str, text: &str) -> Requirement {
    Requirement {
        id: id.into(),
        text: text.into(),
    }
}

/// Decision 28: a design run's round amends by default: it enters specifying, the
/// round keeps the approved requirements as its base, and an amendment must continue
/// from the base's last number (R3); one that does is the spec gate's next version.
#[test]
fn iterate_amend_enters_specifying_and_the_amendment_continues_numbering() {
    let mut fx = design_complete();
    assert_eq!(iterate_with(&mut fx, None), started(2));
    let run = fx.run();
    assert_eq!(run.state, RunState::Specifying);
    let design = run.orch.design.as_ref().unwrap();
    let round = design.round.as_ref().expect("round 2's design");
    assert_eq!((round.n, round.mode), (2, RoundDesign::Amend));
    let ids: Vec<&str> = round.base.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["R1", "R2"]);
    assert!(design.phase_started.is_some(), "specifying's clock runs");

    let gap = AMENDMENT.replace("R3 The app", "R4 The app");
    let refused = submit_amendment(&mut fx, &gap).unwrap_err();
    assert_eq!(refused, "the amendment must continue from R3");
    let answer = submit_amendment(&mut fx, AMENDMENT).unwrap();
    assert_eq!(answer["version"], 2, "{answer}");
    assert_eq!(gate(&fx).map(|g| (g.0, g.1)), Some((DocGateKind::Spec, 2)));
}

/// Decision 28: `full` brainstorms again: the round enters brainstorming with no answers
/// and no brainstormers, so `start_brainstorm` is taken, and its pack is a new
/// brainstorm round's (ruling T8-6: `pack-r2.md`, never round 1's again).
#[test]
fn iterate_full_enters_brainstorming() {
    let mut fx = design_complete();
    assert_eq!(iterate_with(&mut fx, Some(RoundDesign::Full)), started(2));
    let run = fx.run();
    assert_eq!(run.state, RunState::Brainstorming);
    let design = run.orch.design.as_ref().unwrap();
    assert_eq!(
        design.round.as_ref().map(|r| r.mode),
        Some(RoundDesign::Full)
    );
    assert_eq!(design.answers, None);
    assert!(design.brainstormers.is_empty());
    assert!(!design.drafts_settled);
    start_brainstorm(&mut fx);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.answers.as_deref(), Some("skip"));
    assert_eq!(design.pack.as_ref().map(|p| p.round), Some(2));
}

/// Decision 28: `off` is 9.3's round: planning, a plan with no design checks, 9.3's
/// plan gate (no document gate) and its approve, and no documents commit.
#[test]
fn iterate_off_is_9_3() {
    let mut fx = design_complete();
    assert_eq!(iterate_with(&mut fx, Some(RoundDesign::Off)), started(2));
    assert_eq!(fx.run().state, RunState::Planning);
    let mut t2 = super::orch::add("t2", "mail");
    t2["task"]["stage"] = json!(2);
    let answer = round_submit(&mut fx, json!([t2])).unwrap();
    assert_eq!(answer["awaiting_approval"], true, "{answer}");
    let run = fx.run();
    assert_eq!(run.state, RunState::AwaitingApproval);
    assert!(gate(&fx).is_none(), "no document gate");
    fx.approve();
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    let design = run.orch.design.as_ref().unwrap();
    assert!(!design.commit_due, "an off round commits nothing");
}

/// Decision 28: a run without the flow has no spec to amend; `amend` and `full` are
/// refused with the exact text and change nothing, and its default is `off`.
#[test]
fn design_on_a_non_design_run_is_refused() {
    for design in [RoundDesign::Amend, RoundDesign::Full] {
        let mut fx = complete();
        assert_eq!(iterate_with(&mut fx, Some(design)), Err(NO_SPEC.into()));
        assert_eq!(fx.run().rounds.len(), 1);
        assert_eq!(fx.run().state, RunState::Complete);
    }
    let mut fx = complete();
    assert_eq!(iterate_with(&mut fx, None), started(2));
    assert_eq!(fx.run().state, RunState::Planning);
}

/// Decision 29 and ruling T4-4: the approved amendment is merged into the base (R2's
/// text replaced and marked, R3 added), and the round's plan must cover only R2 and
/// R3: R1, covered by round 1, is not asked for (a refused submit applies nothing).
#[test]
fn the_rounds_plan_covers_only_new_and_changed_requirements() {
    let mut fx = round_planning();
    let design = fx.run().orch.design.as_ref().unwrap();
    let r2 = "Links are single use, in the app too. Check: a reuse test. (changed in round 2)";
    assert_eq!(
        design.requirements,
        vec![
            requirement("R1", "Tokens expire after an hour. Check: a clock test."),
            requirement("R2", r2),
            requirement("R3", "The app opens reset links. Check: a deep-link test."),
        ]
    );
    let refused = round_submit(&mut fx, json!([round_task("t2", &["R2"])])).unwrap_err();
    assert_eq!(refused, "R3 are covered by no task");
    let edits = json!([round_task("t2", &["R2"]), round_task("t3", &["R3"])]);
    let answer = round_submit(&mut fx, edits).unwrap();
    assert_eq!(answer["awaiting_review"], true, "{answer}");
}

/// Ruling T4-4: a changed requirement that already carries its marker keeps one.
#[test]
fn a_marked_change_is_not_marked_twice() {
    let mut fx = design_complete();
    iterate_with(&mut fx, None).unwrap();
    let marked = AMENDMENT.replace("in the app too.", "in the app too (changed in round 2).");
    submit_amendment(&mut fx, &marked).unwrap();
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    super::design_plan_fixture::read_back(&mut fx, 2, &marked);
    let design = fx.run().orch.design.as_ref().unwrap();
    let r2 = &design.requirements[1].text;
    assert_eq!(r2.matches("(changed in round 2)").count(), 1, "{r2}");
}

/// Task 10's carry: round 2 at its plan gate, then a back to the spec (ruling T10-5
/// clears the approval): the revised amendment still continues from the round's base,
/// R3, not from R1.
#[test]
fn a_back_in_a_round_keeps_the_amendments_base() {
    let mut fx = round_plan_gate(json!([round_task("t2", &["R2", "R3"])]));
    let back = DocGateAction::Back {
        note: "R3 needs a fallback.".into(),
    };
    act(&mut fx, DocGateKind::Plan, back).unwrap();
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.requirements.is_empty(), "ruling T10-5");
    let base = &design.round.as_ref().unwrap().base;
    assert_eq!(base.len(), 2, "the base stays");
    let revised = AMENDMENT.replace("deep-link test.", "deep-link test, with a fallback.");
    let answer = submit_amendment(&mut fx, &revised).unwrap();
    assert_eq!(answer["version"], 3, "{answer}");
}

/// Decision 29 (9.3's rule): a reject at round 2's spec gate drops only the round: the
/// run is complete again, round 1's requirements and approved spec stand, no gate is
/// open, and round 1's documents commit is untouched.
#[test]
fn rejecting_a_round_gate_drops_only_the_round() {
    let mut fx = amendment_at_gate();
    let before = fx
        .run()
        .orch
        .design
        .as_ref()
        .unwrap()
        .round
        .clone()
        .unwrap();
    let text = act(&mut fx, DocGateKind::Spec, DocGateAction::Reject).unwrap();
    assert!(text.contains("round 2"), "{text}");
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Rejected));
    let design = run.orch.design.as_ref().unwrap();
    assert_eq!(design.requirements, before.base);
    assert_eq!(design.approved_spec, before.spec_before);
    assert_eq!(design.approved_spec, Some(1));
    assert!(design.gate.is_none());
    assert_eq!(
        design.committed.as_deref(),
        Some(super::design_commit::DOCS)
    );
}

/// Task 10's carry (m6): an unasked later cycle, round 2's amendment submitted ready
/// with no review, is marked not reviewed as round 1's first version would be.
#[test]
fn an_unreviewed_amendment_is_marked_not_reviewed() {
    let fx = amendment_at_gate();
    let design = fx.run().orch.design.as_ref().unwrap();
    let v2 = design.find(DocKind::Spec, Some(2)).unwrap();
    assert_eq!(v2.not_reviewed.as_deref(), Some(UNREVIEWED));
}

/// Task 10's carry (m6): after a back from the spec gate to the brainstorm and its
/// approval again, the spec written unasked is marked not reviewed too.
#[test]
fn a_spec_after_a_brainstorm_back_is_marked_not_reviewed() {
    let mut fx = at_spec_gate(false);
    let back = DocGateAction::Back {
        note: "Compare a third approach.".into(),
    };
    act(&mut fx, DocGateKind::Spec, back).unwrap();
    submitted(&mut fx, "brainstorm", REPORT);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::APPROVE).unwrap();
    assert_eq!(fx.run().state, RunState::Specifying);
    let answer = submitted(&mut fx, "spec", SPEC);
    let n = answer["version"].as_u64().unwrap() as u32;
    let design = fx.run().orch.design.as_ref().unwrap();
    let version = design.find(DocKind::Spec, Some(n)).unwrap();
    assert_eq!(version.not_reviewed.as_deref(), Some(UNREVIEWED));
}

/// Task 7's carry: a halt that reaches round 2 before its plan is approved (a `pr`
/// run's delivery result in flight at the iterate, `delivery/watch.rs::stopped` or
/// `open.rs`'s rewritten branch, goes through `merge::halt`) resumes into the round's
/// phase, retryable or not, never to `running`.
#[test]
fn a_halt_before_the_rounds_plan_is_approved_resumes_into_its_phase() {
    for retryable in [true, false] {
        let mut fx = design_complete();
        iterate_with(&mut fx, None).unwrap();
        let now = fx.now;
        let run = fx.run_mut();
        crate::run::engine::merge::halt(run, "remote stage branch moved".into(), now);
        run.halt_retryable = retryable;
        assert_eq!(fx.run().state, RunState::Halted);
        let reply_id = fx.reply();
        let effects = fx.next(EventKind::Resume {
            reply: reply_id,
            run_id: RUN_ID.into(),
            rebaseline: None,
        });
        assert!(reply(&effects).is_ok(), "{retryable}: {effects:?}");
        assert_eq!(fx.run().state, RunState::Specifying, "{retryable}");
    }
}

/// Task 13's carry: round 2's spec approval writes its phase record for round 2,
/// counting the round's gate versions only (round 1 had spec v1 too).
#[test]
fn a_rounds_phase_record_counts_the_rounds_versions() {
    let mut fx = design_complete();
    let run = fx.run_mut();
    run.history = true;
    run.repo_dir = "/tmp/data/repos/x-3f9a".into();
    let fx = &mut amended(fx);
    act(fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    let records: Vec<proto::PhaseRecord> = (fx.ops("AppendHistory").into_iter())
        .filter_map(|(_, kind)| match kind {
            crate::run::engine::OpKind::AppendHistory { line, .. } => match *line {
                proto::HistoryLine::Phase(record) => Some(record),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let last = records.last().expect("a phase record");
    assert_eq!(last.record_id, format!("{RUN_ID}/phase/2/specifying/v2"));
    assert_eq!((last.round, last.gate_versions), (2, 1));
}
