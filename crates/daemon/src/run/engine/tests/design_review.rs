//! Milestone 9.6 task M9.6.10: the spec and its peer review (DF §4, decisions 14 to 16,
//! ruling T5-1). A `ready: false` spec is a review draft that dispatches the document
//! reviewer on the orchestrator's peer runtime; its findings wake the orchestrator; a
//! `ready: true` spec answers every finding of the latest review and opens the gate,
//! with the findings kept as disputed; at most two reviews; a reviewer's failure opens
//! the gate not reviewed; the spec's approval stores its requirements from the approved
//! text the driver reads back. The reviewer's session is stubbed by its op's result and
//! its ended event; the helpers are `design_review_fixture.rs`'s.

use proto::{AgentRole, DocGateAction, DocGateKind, DocKind, DocSeverity, RunState, Runtime};
use serde_json::{Value, json};

use super::design_agents::{launches, started};
use super::design_fixture::*;
use super::design_review_fixture::*;
use super::fixture::*;
use crate::run::design::state::{DesignAgentState, Revision};
use crate::run::engine::{DocChecked, Effect, EventKind, ScoutEnd};
use crate::run::snapshot::snapshot;

/// DF §4.2: a `ready: false` spec is stored as review draft 1 (ruling T5-1: not a gate
/// version) and dispatches the document reviewer (milestone 9.8: the reviewer row's pick
/// against the orchestrator), pointed at its draft; the run stays in specifying.
#[test]
fn ready_false_dispatches_the_peer_reviewer() {
    let mut fx = specifying();
    let orchestrator = fx.run().orch.orchestrator.as_ref().unwrap().route.clone();
    assert_eq!(orchestrator.runtime, Runtime::Claude);
    let effects = submit_spec(&mut fx, false, Value::Null);
    let reply = outcome(&effects).unwrap();
    assert_eq!(reply["accepted"], json!(true));
    assert_eq!(reply["review"], json!(1));
    assert_eq!(reply["awaiting_review"], json!(true));
    assert_eq!(written(&effects), ["spec-draft-r1.md"]);
    assert_eq!(design(&fx).gate_versions(DocKind::Spec), 0);
    assert_eq!(design(&fx).find(DocKind::Spec, None).unwrap().n, 0);
    assert_eq!(
        (fx.run().state, design(&fx).gate.is_none()),
        (RunState::Specifying, true)
    );
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!(spec.kind.label(), "spec-r1");
    assert_eq!(spec.kind.role(), AgentRole::DocReviewer);
    assert_eq!(spec.session, 1);
    // Milestone 9.8 (decision 27): the reviewer row, the built-in Codex default at high.
    assert_eq!(spec.route.runtime, Runtime::Codex);
    assert_eq!(spec.route.model, "");
    assert_eq!(spec.route.effort, proto::Effort::HIGH);
    assert!(
        spec.first_turn.contains("get_doc") && spec.first_turn.contains("draft 1"),
        "{}",
        spec.first_turn
    );
    let mcp = spec.headless.mcp.as_ref().unwrap();
    assert_eq!(mcp.agent_label.as_deref(), Some("spec-r1"));
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Running);
    assert_eq!(crate::run::engine::schedule::readers_busy(fx.run()), 1);
    // Ruling T5-1: the run's listed documents are its gate versions, not the draft.
    let snap = snapshot(&fx.state, fx.now);
    let docs: Vec<DocKind> = snap.runs[0].docs.iter().map(|d| d.kind).collect();
    // (The brainstorm's two drafts, which the fixture stores since ruling T9-1a.)
    assert_eq!(
        docs,
        [
            DocKind::BrainstormDraft,
            DocKind::BrainstormDraft,
            DocKind::Brainstorm
        ]
    );
}

/// DF §4.2: with no peer installed, the orchestrator's own runtime reviews, and the run
/// says so: its log, and the gate's version (`reviewed by the same runtime`).
#[test]
fn no_peer_reviews_on_the_same_runtime_and_says_so() {
    let mut fx = specifying();
    fx.run_mut().orch.installed = [("codex".to_string(), false)].into();
    let orchestrator = fx.run().orch.orchestrator.as_ref().unwrap().route.clone();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!(spec.route, orchestrator);
    // Fix round 1 (m1): the true reason.
    let line = "the document reviewer spec-r1: no peer reviewer: the codex runtime is not installed; reviewing on the same runtime, claude";
    assert!(
        log_lines(&fx).contains(&line.to_string()),
        "{:?}",
        log_lines(&fx)
    );
    started(&mut fx, "spec-r1", REVIEWER);
    outcome(&submit_findings(&mut fx, REVIEWER, json!([]))).unwrap();
    outcome(&submit_spec(&mut fx, true, json!([]))).unwrap();
    assert!(spec_v(&fx, 1).same_runtime);
    let snap = snapshot(&fx.state, fx.now);
    assert!(snap.runs[0].doc_gate.as_ref().unwrap().same_runtime);
}

/// Fix round 1 (m1): the reviewer falls back to the orchestrator's own runtime with its
/// reason: the reviewer row's pick is not installed, or its CLI cannot run a session
/// unsaved (ruling T8-4).
#[test]
fn the_same_runtime_fallback_names_its_reason() {
    use crate::decider::{DECIDER_CAPS, DeciderCaps};
    use crate::run::orch::roles::lists::review_pick;
    let mut fx = specifying();
    let peer = review_pick(fx.run(), &DECIDER_CAPS).unwrap();
    assert_eq!((peer.route.runtime, peer.why), (Runtime::Codex, None));
    let unsaved = DeciderCaps {
        codex_ephemeral: false,
        ..DECIDER_CAPS
    };
    let own = review_pick(fx.run(), &unsaved).unwrap();
    assert_eq!(own.route.runtime, Runtime::Claude);
    let line = "the codex CLI cannot run a session without saving it";
    assert_eq!(own.why.as_deref(), Some(line));
    fx.run_mut().orch.installed = [("codex".to_string(), false)].into();
    let own = review_pick(fx.run(), &unsaved).unwrap();
    assert_eq!(
        own.why.as_deref(),
        Some("the codex runtime is not installed")
    );
}

/// MR D3, decision 27 (fix round 1, I1): a document reviewer on the orchestrator's own
/// model is warned in the run log once, when its review is queued.
#[test]
fn a_document_reviewer_on_the_authors_model_is_logged() {
    let mut fx = specifying();
    let own = fx.run().orch.orchestrator.as_ref().unwrap().route.clone();
    let model = format!("claude:{}", own.model);
    let reviewer = proto::models::Role::Reviewer;
    crate::run::test_support::set_row(fx.run_mut(), reviewer, &model, None, None);
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let line = format!(
        "reviewer: {model} reviews work by the same model; set an \"if it struggles\" model for the reviewer in C-b S"
    );
    let lines = log_lines(&fx);
    assert_eq!(lines.iter().filter(|l| **l == line).count(), 1, "{lines:?}");
}

/// Rulings T10-4 and T10-6: a spec gate revising after a Back or a failed read-back is
/// not sent for review, and a review draft is refused with the cause-neutral text.
#[test]
fn a_back_or_read_back_revision_is_not_sent_for_review() {
    // Ruling T10-6 (amends T10-4): a refusal that says what to do.
    let not_sent = "this revision is not sent for review; submit it with ready = true";
    let mut fx = at_plan_gate(false);
    let back = DocGateAction::Back {
        note: "Rethink R2.".into(),
    };
    act(&mut fx, DocGateKind::Plan, back).unwrap();
    let gate = design(&fx).gate.clone().unwrap();
    assert_eq!((gate.kind, gate.cause), (DocGateKind::Spec, Revision::Back));
    let draft = outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap_err();
    assert_eq!(draft, not_sent);
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    let checked = vec![DocChecked {
        kind: DocKind::Spec,
        n: 1,
        read: Err("its file is missing".into()),
    }];
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked,
    });
    let gate = design(&fx).gate.clone().unwrap();
    assert_eq!(gate.cause, Revision::ReadBack);
    let draft = outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap_err();
    assert_eq!(draft, not_sent);
}

/// Decision 15: the findings are stored and wake the orchestrator with the exact note;
/// the reviewer's session is retired, and its window takes no second submission.
#[test]
fn findings_wake_the_orchestrator_exactly() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    started(&mut fx, "spec-r1", REVIEWER);
    // Nothing is submitted while the review runs.
    for ready in [false, true] {
        let early = outcome(&submit_spec(&mut fx, ready, Value::Null)).unwrap_err();
        assert_eq!(
            early,
            "the spec review 1 is still running; wait for its findings"
        );
    }
    let effects = submit_findings(&mut fx, REVIEWER, three_findings());
    assert_eq!(outcome(&effects).unwrap()["accepted"], json!(true));
    let retired = (effects.iter())
        .any(|e| matches!(e, Effect::PlannerAccepted { window_id } if *window_id == REVIEWER));
    assert!(retired, "{effects:?}");
    let note = "spec review 1 is in: 3 findings (1 blocking); answer each in your next submit_doc";
    assert!(notes(&fx).contains(&note.to_string()), "{:?}", notes(&fx));
    let review = design(&fx).reviews.last().cloned().unwrap();
    assert_eq!(
        (review.doc, review.n, review.failed),
        (DocKind::Spec, 1, None)
    );
    let ids: Vec<&str> = review.findings.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["F1", "F2", "F3"]);
    assert_eq!(review.findings[0].severity, DocSeverity::Blocking);
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Done);
    let again = outcome(&submit_findings(&mut fx, REVIEWER, json!([])));
    assert_eq!(
        again.unwrap_err(),
        format!("this window is not the document reviewer of run {RUN_ID}")
    );
}

/// Decision 15: `ready: true` answers every finding of the latest review, or is refused
/// naming the missing ids, and nothing changes.
#[test]
fn ready_true_needs_every_finding_answered() {
    let mut fx = specifying();
    reviewed(&mut fx, REVIEWER, three_findings());
    let partial = json!([{"id": "F1", "answer": "fixed"}]);
    let refused = outcome(&submit_spec(&mut fx, true, partial)).unwrap_err();
    assert_eq!(refused, "answer every finding of review 1; missing: F2, F3");
    assert_eq!(
        (fx.run().state, design(&fx).gate.is_none()),
        (RunState::Specifying, true)
    );
    let all = answers(&["F1", "F2", "F3"], &[]);
    let reply = outcome(&submit_spec(&mut fx, true, all)).unwrap();
    assert_eq!(reply["version"], json!(1));
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, None)));
    assert_eq!(spec_v(&fx, 1).not_reviewed, None);
}

/// Decision 15: two reviews at most; a third draft is refused exactly, and `ready: true`
/// answers the latest review, the second.
#[test]
fn at_most_two_reviews() {
    let mut fx = specifying();
    reviewed(&mut fx, REVIEWER, three_findings());
    let effects = submit_spec(&mut fx, false, Value::Null);
    assert_eq!(outcome(&effects).unwrap()["review"], json!(2));
    assert_eq!(written(&effects), ["spec-draft-r2.md"]);
    started(&mut fx, "spec-r2", REVIEWER + 1);
    let second =
        json!([{"id": "G1", "severity": "minor", "place": "## Design", "text": "Say where."}]);
    outcome(&submit_findings(&mut fx, REVIEWER + 1, second)).unwrap();
    let third = outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap_err();
    assert_eq!(
        third,
        "the spec has had its two reviews; submit with ready = true"
    );
    let missing = outcome(&submit_spec(
        &mut fx,
        true,
        answers(&["F1", "F2", "F3"], &[]),
    ));
    assert_eq!(
        missing.unwrap_err(),
        "answer every finding of review 2; missing: G1"
    );
    outcome(&submit_spec(&mut fx, true, answers(&["G1"], &[]))).unwrap();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, None)));
}

/// DF §4.3: a reviewer that fails records `not reviewed: <reason>`; the orchestrator is
/// woken, and `ready: true` opens the gate without responses, the reason beside it.
#[test]
fn a_reviewer_failure_opens_the_gate_not_reviewed() {
    let mut fx = specifying();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    started(&mut fx, "spec-r1", REVIEWER);
    let reason = "the document reviewer ran longer than 600 s";
    let failed = ScoutEnd::Failed {
        reason: reason.into(),
    };
    reviewer_ended(&mut fx, "spec-r1", 1, failed);
    let review = design(&fx).reviews.last().cloned().unwrap();
    assert_eq!(review.failed.as_deref(), Some(reason));
    let reviewer = design(&fx).reviewer.clone().unwrap();
    assert_eq!(reviewer.state, DesignAgentState::Failed(reason.into()));
    let woken = notes(&fx)
        .iter()
        .any(|n| n.contains("spec review 1 failed") && n.contains(reason));
    assert!(woken, "{:?}", notes(&fx));
    outcome(&submit_spec(&mut fx, true, Value::Null)).unwrap();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 1, None)));
    assert_eq!(spec_v(&fx, 1).not_reviewed.as_deref(), Some(reason));
    let snap = snapshot(&fx.state, fx.now);
    let info = snap.runs[0].doc_gate.clone().unwrap();
    assert_eq!(info.not_reviewed.as_deref(), Some(reason));
}

/// Decision 16: findings answered `kept: <reason>` are disputed, stored with the version
/// and shown at the gate; the version's findings file keeps every answer.
#[test]
fn kept_answers_are_disputed_at_the_gate() {
    let mut fx = specifying();
    reviewed(&mut fx, REVIEWER, three_findings());
    let effects = submit_spec(&mut fx, true, answers(&["F1", "F2", "F3"], &["F2"]));
    outcome(&effects).unwrap();
    let disputed: Vec<String> = (spec_v(&fx, 1).disputed.iter())
        .map(|f| f.id.clone())
        .collect();
    assert_eq!(disputed, ["F2"]);
    let snap = snapshot(&fx.state, fx.now);
    let info = snap.runs[0].doc_gate.clone().unwrap();
    assert_eq!(info.disputed.len(), 1);
    assert_eq!(info.disputed[0].text, "Name a second risk.");
    let file = (effects.iter()).find_map(|e| match e {
        Effect::WriteDoc { path, text, .. } if path.ends_with("findings-spec-v1.json") => {
            Some(text.clone())
        }
        _ => None,
    });
    let file = file.expect("the findings file of v1");
    assert!(
        file.contains("kept: out of scope") && file.contains("F3"),
        "{file}"
    );
}

/// DF §4.2: a revision after the user's changes is submitted `ready: true` directly and
/// not reviewed; with `Changes { review: true }` it must go to one fresh review first.
#[test]
fn a_revision_after_changes_is_reviewed_only_when_asked() {
    let mut fx = at_spec_gate(false);
    let changes = |review| DocGateAction::Changes {
        note: "Tighten R2.".into(),
        review,
    };
    act(&mut fx, DocGateKind::Spec, changes(false)).unwrap();
    let draft = outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap_err();
    assert_eq!(
        draft,
        "the user asked for no review of this revision; submit it with ready = true"
    );
    let before = launches(&fx).len();
    outcome(&submit_spec(&mut fx, true, Value::Null)).unwrap();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 2, None)));
    assert_eq!(launches(&fx).len(), before, "no review");
    act(&mut fx, DocGateKind::Spec, changes(true)).unwrap();
    let direct = outcome(&submit_spec(&mut fx, true, Value::Null)).unwrap_err();
    assert_eq!(
        direct,
        "the user asked for a review of this revision; submit it with ready = false first"
    );
    reviewed(&mut fx, REVIEWER, three_findings());
    assert_eq!(launches(&fx).len(), before + 1);
    let again = outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap_err();
    assert_eq!(
        again,
        "the spec has had the review the user asked for; submit with ready = true"
    );
    outcome(&submit_spec(
        &mut fx,
        true,
        answers(&["F1", "F2", "F3"], &[]),
    ))
    .unwrap();
    assert_eq!(gate(&fx), Some((DocGateKind::Spec, 3, None)));
}

/// Decision 12 and the carry from task 7's review: the approved spec's requirements and
/// Goal section are stored from the approved version's text as the driver read it back
/// (a checked read), never from the engine's cache of texts.
#[test]
fn approving_the_spec_stores_its_requirements_and_goal_section() {
    let mut fx = at_spec_gate(false);
    let stale = SPEC.replace("R2 Links are single use. Check: a reuse test.\n", "");
    let cached = (design(&fx).texts.iter()).position(|(k, _, _)| *k == DocKind::Spec);
    fx.run_mut().orch.design.as_mut().unwrap().texts[cached.unwrap()].2 = stale;
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    assert!(design(&fx).requirements.is_empty(), "not from the cache");
    let checked = vec![DocChecked {
        kind: DocKind::Spec,
        n: 1,
        read: Ok(Some(SPEC.to_string())),
    }];
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked,
    });
    let ids: Vec<&str> = (design(&fx).requirements.iter())
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(ids, ["R1", "R2"]);
    assert_eq!(
        design(&fx).requirements[1].text,
        "Links are single use. Check: a reuse test."
    );
    assert_eq!(design(&fx).goal_section, "Users reset their password.");
    assert_eq!(fx.run().state, RunState::Planning);
}
