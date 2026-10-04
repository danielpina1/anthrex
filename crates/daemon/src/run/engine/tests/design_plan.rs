//! Milestone 9.6 task M9.6.11 (DF §5.1, decisions 17 to 22): a design run's plan. Its
//! submit is checked for coverage of the approved spec's requirements, dangling
//! `covers` and the briefs' five headings, in that order; the first passing submit goes
//! to the plan review instead of the gate, and the next answers its findings and opens
//! the gate; a sub-planner gets its epic's requirements; a run without the flow
//! submits as in 9.5.

use proto::{DocKind, RunState};
use serde_json::{Value, json};

use super::design_agents::launches;
use super::design_fixture::*;
use super::design_plan_fixture::*;
use super::design_review_fixture::{answers, outcome, three_findings, written};
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, add, launched, orch_tool};
use super::planners::{PLANNER, planner_started, spawn, submit_epic};
use crate::run::engine::OpKind;

/// The number of plan reviews launched so far.
fn plan_reviews(fx: &Fixture) -> usize {
    (launches(fx).iter())
        .filter(|(_, s)| s.kind.label().starts_with("plan-r"))
        .count()
}

/// Decision 18's first check: every requirement of the approved spec is covered. The
/// call is all or nothing, so its edits are not applied either.
#[test]
fn an_uncovered_requirement_refuses_submit_exactly() {
    let mut fx = planning();
    let before = fx.run().tasks.clone();
    let refused = plan_submit(&mut fx, json!([covering("t1", &["R1"])]), Value::Null);
    assert_eq!(refused.unwrap_err(), "R2 are covered by no task");
    assert_eq!(fx.run().tasks, before);
    assert_eq!(fx.run().state, RunState::Planning);
    let refused = plan_submit(&mut fx, json!([covering("t1", &[])]), Value::Null);
    assert_eq!(refused.unwrap_err(), "R1, R2 are covered by no task");
    // Coverage comes before the other two checks.
    let mut both = covering("t1", &["R1", "R9"]);
    both["task"]["brief"] = json!("no headings");
    let refused = plan_submit(&mut fx, json!([both]), Value::Null);
    assert_eq!(refused.unwrap_err(), "R2 are covered by no task");
    assert_eq!(plan_reviews(&fx), 0);
}

/// Decision 18's second check: every `covers` entry names a requirement of the
/// approved spec.
#[test]
fn a_dangling_cover_refuses_submit() {
    let mut fx = planning();
    let edits = json!([covering("t1", &["R1", "R2", "R3"])]);
    let refused = plan_submit(&mut fx, edits, Value::Null);
    assert_eq!(
        refused.unwrap_err(),
        "task t1 covers R3, which the spec does not have"
    );
    assert!(fx.run().task("t1").is_none());
}

/// Decisions 18 and 19: the third check, each brief's five headings.
#[test]
fn a_brief_without_a_heading_refuses_submit() {
    let mut fx = planning();
    let mut t1 = covering("t1", &["R1", "R2"]);
    t1["task"]["brief"] = json!("Files:\na\nTests first:\nt\nSteps:\ns\nAcceptance:\na");
    let refused = plan_submit(&mut fx, json!([t1]), Value::Null);
    assert_eq!(
        refused.unwrap_err(),
        "task t1's brief is missing the heading \"Verify:\""
    );
    assert_eq!(plan_reviews(&fx), 0);
}

/// Decision 20 and task 6's carry (e): the first passing submit does not open the gate.
/// The rendered `plan.md` is stored as the review's draft (so the reviewer's `get_doc`
/// reads it) and the document reviewer is launched on it; the reply says so.
#[test]
fn the_first_passing_submit_starts_the_plan_review() {
    let mut fx = planning();
    let args = json!({"edits": [covering("t1", &["R1", "R2"])], "submit": true});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    let reply = outcome(&effects).unwrap();
    assert_eq!(reply["accepted"], true);
    assert_eq!(reply["awaiting_review"], true);
    assert_eq!(reply["awaiting_approval"], false);
    assert_eq!(fx.run().state, RunState::Planning);
    assert!(gate(&fx).is_none());
    assert!(!fx.run().orch.orchestrator.as_ref().unwrap().plan_submitted);
    assert_eq!(written(&effects), ["plan-draft-r1.md"]);
    let design = fx.run().orch.design.as_ref().unwrap();
    let draft = design.draft(DocKind::Plan, 1).expect("the review's draft");
    assert_eq!((draft.n, draft.draft_review), (0, Some(1)));
    assert_eq!(design.gate_versions(DocKind::Plan), 0);
    assert_eq!(design.find(DocKind::Plan, None), Some(draft));
    let text = (effects.iter()).find_map(|e| match e {
        crate::run::engine::Effect::WriteDoc { text, .. } => Some(text.clone()),
        _ => None,
    });
    let text = text.expect("the draft is written");
    assert!(text.contains("### t1 Title t1\nCovers: R1, R2\n"), "{text}");
    assert!(text.contains("| R1 | t1 |\n| R2 | t1 |\n"), "{text}");
    assert_eq!(plan_reviews(&fx), 1);
    let (_, spec) = launches(&fx).last().unwrap().clone();
    assert!(
        spec.first_turn.contains("get_doc, kind \"plan\""),
        "{}",
        spec.first_turn
    );
    assert!(design.plan_review_done);
}

const IN: &str = "plan review is in: 3 findings (1 blocking); answer each in edit_plan's responses when you submit";

/// Decision 20: the findings wake the orchestrator exactly; a submit waits for them,
/// then must answer every one; edits between the two submits are checked again; the
/// answering submit opens the gate with the kept findings disputed and the findings
/// file; the plan gets one review.
#[test]
fn the_second_submit_must_answer_the_findings_and_opens_the_gate() {
    let mut fx = planning();
    let first = plan_submit(&mut fx, json!([covering("t1", &["R1", "R2"])]), Value::Null);
    assert_eq!(first.unwrap()["awaiting_review"], true);
    let waiting = plan_submit(&mut fx, json!([]), Value::Null).unwrap_err();
    assert_eq!(
        waiting,
        "the plan review 1 is still running; wait for its findings"
    );
    plan_reviewed(&mut fx, three_findings());
    assert!(notes(&fx).contains(&IN.to_string()), "{:?}", notes(&fx));
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    let review = &digest["gate"]["plan_review"];
    assert_eq!(review["review"], 1, "{digest}");
    assert_eq!(review["findings"].as_array().unwrap().len(), 3);
    let missing = plan_submit(&mut fx, json!([]), Value::Null).unwrap_err();
    assert_eq!(
        missing,
        "answer every finding of review 1; missing: F1, F2, F3"
    );
    let some = plan_submit(&mut fx, json!([]), answers(&["F1"], &[])).unwrap_err();
    assert_eq!(some, "answer every finding of review 1; missing: F2, F3");
    // An edit between the two submits is checked again.
    let all = answers(&["F1", "F2", "F3"], &["F2"]);
    let cancel = json!([{"op": "cancel_task", "task_id": "t1"}, add_task("t2")]);
    let refused = plan_submit(&mut fx, cancel, all.clone()).unwrap_err();
    assert_eq!(refused, "R1, R2 are covered by no task");
    let args = json!({"edits": [], "submit": true, "responses": all});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    let reply = outcome(&effects).unwrap();
    assert_eq!(reply["awaiting_approval"], true);
    assert!(reply.get("awaiting_review").is_none(), "{reply}");
    assert_eq!(gate(&fx), Some((proto::DocGateKind::Plan, 1, None)));
    assert_eq!(written(&effects), ["plan-v1.md", "findings-plan-v1.json"]);
    let design = fx.run().orch.design.as_ref().unwrap();
    let v1 = design.find(DocKind::Plan, Some(1)).unwrap();
    let disputed: Vec<&str> = v1.disputed.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(disputed, ["F2"]);
    assert_eq!(v1.not_reviewed, None);
    assert_eq!(plan_reviews(&fx), 1);
}

/// DF §4.3 for the plan: a failed plan review wakes the orchestrator, and its next
/// submit opens the gate not reviewed, with no responses asked.
#[test]
fn a_failed_plan_review_opens_the_gate_not_reviewed() {
    let mut fx = planning();
    plan_submit(&mut fx, json!([covering("t1", &["R1", "R2"])]), Value::Null).unwrap();
    super::design_agents::started(&mut fx, "plan-r1", PLAN_REVIEWER);
    let failed = crate::run::engine::ScoutEnd::Failed {
        reason: "it crashed".into(),
    };
    super::design_review_fixture::reviewer_ended(&mut fx, "plan-r1", 1, failed);
    let note = "plan review 1 failed: it crashed; submit the plan again with edit_plan, and its gate opens not reviewed";
    assert!(notes(&fx).contains(&note.to_string()), "{:?}", notes(&fx));
    plan_submit(&mut fx, json!([]), Value::Null).unwrap();
    let design = fx.run().orch.design.as_ref().unwrap();
    let v1 = design.find(DocKind::Plan, Some(1)).unwrap();
    assert_eq!(v1.not_reviewed.as_deref(), Some("it crashed"));
}

/// Review focus 2: coverage is checked against the approved spec's requirements only.
/// A plan citing a requirement found only in an earlier, unapproved version is refused.
#[test]
fn a_plan_citing_a_requirement_only_in_an_unapproved_spec_version_is_refused() {
    let three = SPEC.replace(
        "R2 Links are single use. Check: a reuse test.\n",
        "R2 Links are single use. Check: a reuse test.\nR3 Old links expire. Check: a test.\n",
    );
    let mut fx = at_brainstorm_gate(false);
    let (spec, approve) = (proto::DocGateKind::Spec, proto::DocGateAction::Approve);
    act(&mut fx, proto::DocGateKind::Brainstorm, approve.clone()).unwrap();
    submitted(&mut fx, "spec", &three);
    let changes = proto::DocGateAction::Changes {
        note: "Drop R3.".into(),
        review: false,
    };
    act(&mut fx, spec, changes).unwrap();
    submitted(&mut fx, "spec", SPEC);
    act(&mut fx, spec, approve).unwrap();
    read_back(&mut fx, 2, SPEC);
    let edits = json!([covering("t1", &["R1", "R2"]), covering("t2", &["R3"])]);
    let refused = plan_submit(&mut fx, edits, Value::Null);
    assert_eq!(
        refused.unwrap_err(),
        "task t2 covers R3, which the spec does not have"
    );
}

/// Decision 22: a sub-planner's first turn carries the approved spec's requirements
/// its epic covers, verbatim, then its Goal and Interfaces sections. A new epic, whose
/// tasks cover nothing yet, gets every requirement; its re-plan gets only those its
/// tasks cover.
#[test]
fn a_sub_planner_gets_its_epics_requirements() {
    let mut fx = planning();
    spawn(&mut fx, "mail");
    let turn = planner_turn(&fx, "mail");
    let every = "\n\nSpec requirements this epic delivers:\n\
                 R1  Tokens expire after an hour. Check: a clock test.\n\
                 R2  Links are single use. Check: a reuse test.\n\
                 Goal (from the spec): Users reset their password.\n\
                 Interfaces (from the spec):\n`reset(token)`.";
    assert!(turn.ends_with(every), "{turn}");
    planner_started(&mut fx, PLANNER);
    let effects = submit_epic(&mut fx, json!([covering("mail", &["R2"])]));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    spawn(&mut fx, "mail");
    let turn = planner_turn(&fx, "mail");
    let mine = "\n\nSpec requirements this epic delivers:\n\
                R2  Links are single use. Check: a reuse test.\n\
                Goal (from the spec): Users reset their password.\n\
                Interfaces (from the spec):\n`reset(token)`.";
    assert!(turn.ends_with(mine), "{turn}");
}

/// The first turn of `epic`'s latest sub-planner launch.
fn planner_turn(fx: &Fixture, epic: &str) -> String {
    let found = (fx.ops("StartPlanner").into_iter().rev()).find_map(|(_, kind)| match kind {
        OpKind::StartPlanner { spec } if spec.epic == epic => Some(spec.first_turn),
        _ => None,
    });
    found.unwrap_or_else(|| panic!("no sub-planner of {epic} launched"))
}

/// Decision 18's guard: a run without the design flow submits as in 9.5, with the same
/// reply byte for byte, and no plan review.
#[test]
fn a_non_design_run_submits_as_in_9_5() {
    let mut fx = launched(false);
    let args = json!({"edits": [add("t1", "auth")], "submit": true});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    assert_eq!(
        replies(&effects),
        vec![Ok(concat!(
            r#"{"accepted":true,"awaiting_approval":true,"held":null,"notes":["#,
            r#""t1: the profile has no test_passed: the test proof will require the test's "#,
            r#"name in the single-test command's output (rule 8.1)","#,
            r#""t1: size not backed by a scout report"],"revision":2}"#
        )
        .to_string())]
    );
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert!(fx.run().orch.design.is_none());
}

/// Task 6's review (m5): `edit_plan`'s responses belong to a design run's plan review.
#[test]
fn responses_in_a_run_without_the_design_flow_are_refused() {
    let mut fx = launched(false);
    let args =
        json!({"edits": [add("t1", "auth")], "responses": [{"id": "F1", "answer": "fixed"}]});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    assert_eq!(
        outcome(&effects).unwrap_err(),
        "responses are only for a design run's plan review"
    );
    assert!(fx.run().task("t1").is_none());
}
