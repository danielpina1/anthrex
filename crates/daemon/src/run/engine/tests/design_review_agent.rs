//! Milestone 9.6 task M9.6.10: the document reviewer's session, as a brainstormer's
//! (rulings T8-4 and T8-7): never resumed, so an end without its findings is relaunched
//! fresh once and a second one fails it, opening the gate not reviewed; a restart
//! relaunches it; a rejected run stops it; its routing record; the orchestrator's digest
//! shows its findings; and the approved spec's read-back, whose failure reopens the
//! spec gate. The helpers are `design_review.rs`'s.

use std::path::Path;

use proto::{AgentRole, DocGateAction, DocGateKind, DocKind, RoleOutcome, RunState};
use serde_json::{Value, json};

use super::design_agents::{launches, started};
use super::design_fixture::*;
use super::design_review_fixture::*;
use super::fixture::*;
use super::orch_restore::{restart, resume};
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, codex_args};
use crate::run::design::pack::previous_spec;
use crate::run::design::state::{DesignAgentState, Revision};
use crate::run::engine::{DocChecked, Effect, EventKind, ScoutEnd};
use crate::scout::design_spec::DOC_REVIEWER_TEXTS;
use crate::scout::machine::unsubmitted;

const LINE: &str =
    "Your previous attempt ended without submitting; submit it now with submit_findings.";

/// The reviewer's turn ended without its findings, unnudged (the driver's typed cause).
fn unsubmitted_end(fx: &mut Fixture, session: u32) -> Vec<Effect> {
    let reason = unsubmitted(&DOC_REVIEWER_TEXTS);
    reviewer_ended(fx, "spec-r1", session, ScoutEnd::Unsubmitted { reason })
}

/// Ruling T8-7 for the reviewer: its relaunch is a fresh Codex session (`--ephemeral`,
/// never `resume`) on the same draft with the resubmit line; a second end fails it,
/// `ended twice without submitting`, and the spec's gate then opens not reviewed.
#[test]
fn a_codex_reviewer_that_ends_without_submitting_is_relaunched_fresh_once() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let first = launches(&fx).last().unwrap().1.clone();
    assert_eq!(first.route.runtime, proto::Runtime::Codex);
    started(&mut fx, "spec-r1", REVIEWER);
    unsubmitted_end(&mut fx, 1);
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!(
        (spec.kind.label(), spec.session),
        ("spec-r1".to_string(), 2)
    );
    assert_eq!(spec.first_turn, format!("{}\n{LINE}", first.first_turn));
    let new = SessionArg::New { uuid: None };
    let (exe, sock) = (Path::new("/opt/anthrex"), Path::new("/tmp/sock"));
    let args = codex_args(
        &spec.headless,
        &new,
        &spec.first_turn,
        exe,
        7,
        sock,
        &CLI_CAPS,
    );
    assert!(args.contains(&"--ephemeral".to_string()), "{args:?}");
    assert!(!args.contains(&"resume".to_string()), "{args:?}");
    assert!(
        notes(&fx).iter().all(|n| !n.contains("spec review")),
        "no wake yet"
    );
    started(&mut fx, "spec-r1", REVIEWER + 1);
    let before = launches(&fx).len();
    unsubmitted_end(&mut fx, 2);
    assert_eq!(launches(&fx).len(), before, "no third launch");
    let reason = "ended twice without submitting";
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Failed(reason.into()));
    outcome(&submit_spec(&mut fx, true, Value::Null)).unwrap();
    assert_eq!(spec_v(&fx, 1).not_reviewed.as_deref(), Some(reason));
}

/// Task 8's re-review: the relaunch follows the end's type. A failure whose text reads
/// like an unsubmitted end is a failure, not relaunched.
#[test]
fn a_failure_that_reads_like_an_unsubmitted_end_is_not_relaunched() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    started(&mut fx, "spec-r1", REVIEWER);
    let before = launches(&fx).len();
    let reason = unsubmitted(&DOC_REVIEWER_TEXTS);
    let failed = ScoutEnd::Failed {
        reason: reason.clone(),
    };
    reviewer_ended(&mut fx, "spec-r1", 1, failed);
    assert_eq!(launches(&fx).len(), before);
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Failed(reason));
}

/// DF §8.4: a reviewer running at a daemon restart is relaunched fresh, a new session
/// on the same draft, once the run resumes.
#[test]
fn a_restart_relaunches_a_running_reviewer_fresh() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    started(&mut fx, "spec-r1", REVIEWER);
    let first = launches(&fx).last().unwrap().1.first_turn.clone();
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Queued);
    resume(&mut fx);
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!(
        (spec.kind.label(), spec.session),
        ("spec-r1".to_string(), 2)
    );
    assert_eq!(spec.first_turn, first);
    let said = log_lines(&fx)
        .iter()
        .any(|l| l == "document reviewer spec-r1 relaunches after a daemon restart");
    assert!(said, "{:?}", log_lines(&fx));
}

/// A run rejected while its spec is reviewed stops the reviewer.
#[test]
fn a_rejected_run_stops_its_reviewer() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    started(&mut fx, "spec-r1", REVIEWER);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    let stopped = (effects.iter())
        .any(|e| matches!(e, Effect::StopPlanner { window_id, .. } if *window_id == REVIEWER));
    assert!(stopped, "{effects:?}");
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(
        reviewer.state,
        DesignAgentState::Failed("the run was rejected".into())
    );
    // Fix round 1 (m7): the review records it.
    let failed = design(&fx).reviews.last().unwrap().failed.clone();
    assert_eq!(failed.as_deref(), Some("the run was rejected"));
}

/// Fix round 1 (m7): a reviewer still queued for a reader slot when the run is
/// rejected fails with its review; there is no window to stop.
#[test]
fn a_rejected_run_fails_its_queued_reviewer() {
    let mut fx = specifying();
    fx.run_mut().limits.max_readers = 0;
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Queued);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    let stopped = (effects.iter()).any(|e| matches!(e, Effect::StopPlanner { .. }));
    assert!(!stopped, "{effects:?}");
    let reason = "the run was rejected";
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Failed(reason.into()));
    let failed = design(&fx).reviews.last().unwrap().failed.clone();
    assert_eq!(failed.as_deref(), Some(reason));
}

/// Fix round 1 (m7): the next review's reviewer is a fresh agent, its relaunch unused
/// although the previous review's reviewer used its own.
#[test]
fn a_new_reviews_reviewer_starts_with_its_relaunch_unused() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    started(&mut fx, "spec-r1", REVIEWER);
    unsubmitted_end(&mut fx, 1);
    assert!(design(&fx).reviewer.as_ref().unwrap().unsubmitted);
    started(&mut fx, "spec-r1", REVIEWER + 1);
    outcome(&submit_findings(&mut fx, REVIEWER + 1, three_findings())).unwrap();
    reviewer_ended(&mut fx, "spec-r1", 2, ScoutEnd::Reported);
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.label, "spec-r2");
    assert!(!reviewer.unsubmitted);
}

/// Ruling T10-5: a Back from the plan reopens the spec, so the earlier approval is
/// gone: a run paused there gives its continuation no approved spec.
#[test]
fn a_back_to_the_spec_clears_its_approval() {
    let mut fx = at_plan_gate(false);
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked: vec![DocChecked {
            kind: DocKind::Spec,
            n: 1,
            read: Ok(Some(SPEC.to_string())),
        }],
    });
    assert!(!design(&fx).requirements.is_empty());
    fx.run_mut().continued_by = Some("next-run".into());
    let earlier = |fx: &Fixture| previous_spec(fx.state.runs.values(), "next-run").is_some();
    assert!(earlier(&fx), "approved");
    let back = DocGateAction::Back {
        note: "Rethink R2.".into(),
    };
    act(&mut fx, DocGateKind::Plan, back).unwrap();
    assert_eq!(design(&fx).approved_spec, None);
    assert!(design(&fx).requirements.is_empty());
    assert!(design(&fx).goal_section.is_empty());
    // Paused there (a restore leaves a gate waiting, so the state is set directly).
    let run = fx.run_mut();
    (run.paused_from, run.state) = (Some(run.state), RunState::Paused);
    assert!(!earlier(&fx), "paused after the Back");
}

/// Decision 10's record of the reviewer's session: `spec-r1/1`, the doc reviewer's
/// policy, its source the peer route, completed once its findings are in.
#[test]
fn the_reviewers_session_has_its_routing_record() {
    let mut fx = specifying();
    reviewed(&mut fx, REVIEWER, three_findings());
    reviewer_ended(&mut fx, "spec-r1", 1, ScoutEnd::Reported);
    let record = (fx.run().role_routing_decisions.iter())
        .find(|d| d.role == AgentRole::DocReviewer)
        .cloned()
        .unwrap();
    assert_eq!(record.session_id, "spec-r1/1");
    assert_eq!(
        record.policy_version,
        crate::run::orch::roles::DOC_REVIEWER_POLICY
    );
    assert_eq!(record.source, "peer_route");
    assert_eq!(record.outcome, Some(RoleOutcome::Completed));
}

/// The orchestrator reads the findings to answer in `run_status`: the digest's
/// `spec_review`, until the gate's version answers them.
#[test]
fn the_digest_shows_the_review_to_answer() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    assert_eq!(digest["gate"]["spec_review"]["running"], json!(true));
    started(&mut fx, "spec-r1", REVIEWER);
    outcome(&submit_findings(&mut fx, REVIEWER, three_findings())).unwrap();
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    let review = &digest["gate"]["spec_review"];
    assert_eq!(review["review"], json!(1));
    assert_eq!(review["running"], json!(false));
    assert_eq!(review["findings"][0]["id"], json!("F1"));
    assert_eq!(review["findings"][0]["severity"], json!("blocking"));
    outcome(&submit_spec(
        &mut fx,
        true,
        answers(&["F1", "F2", "F3"], &[]),
    ))
    .unwrap();
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    assert!(digest["gate"].get("spec_review").is_none(), "{digest}");
}

/// The approval asks the driver for the approved version's text (`Effect::ReadBack`);
/// a read that failed reopens the spec gate for the orchestrator to submit it again.
#[test]
fn the_approved_spec_is_read_back_and_a_failed_read_reopens_its_gate() {
    let mut fx = at_spec_gate(false);
    let reply = fx.reply();
    let effects = fx.next(EventKind::DocGate {
        reply,
        run_id: RUN_ID.into(),
        kind: DocGateKind::Spec,
        action: DocGateAction::APPROVE,
    });
    let asked: Vec<(DocKind, u32, String)> = (effects.iter())
        .filter_map(|e| match e {
            Effect::ReadBack { docs, .. } => Some(docs.clone()),
            _ => None,
        })
        .flatten()
        .map(|(v, path)| (v.kind, v.n, path.file_name().unwrap().display().to_string()))
        .collect();
    assert_eq!(asked, [(DocKind::Spec, 1, "spec-v1.md".to_string())]);
    assert_eq!(design(&fx).approved_spec, Some(1));
    let checked = vec![DocChecked {
        kind: DocKind::Spec,
        n: 1,
        read: Err("its file differs from what was stored".into()),
    }];
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked,
    });
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let gate = design(&fx).gate.clone().unwrap();
    assert_eq!(
        (gate.kind, gate.cause),
        (DocGateKind::Spec, Revision::ReadBack)
    );
    assert!(design(&fx).requirements.is_empty());
    let note = "the approved spec v1 could not be read back; submit it again";
    assert!(notes(&fx).contains(&note.to_string()), "{:?}", notes(&fx));
    // Task 10's review (m9): the line is ruling T10-3's halt text (task M9.6.11).
    let logged = "design flow: the approved spec could not be read back: its file differs from what was stored";
    // Fix round 1 (m3): logged once.
    let failed: Vec<String> = (log_lines(&fx).into_iter())
        .filter(|l| l.contains("could not be read back"))
        .collect();
    assert_eq!(failed, [logged]);
}
