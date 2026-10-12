//! Milestone 9.13 task M9.13.8: decisions 19 and 20 through a real daemon socket
//! (`orch_read_rig.rs`). The orchestrator's `run_status` that quotes a comment, and its
//! `task_result` of a review fix task, send `OrchEvent::UntrustedRead` before they
//! answer, on the channel a later call takes; a sub-planner's read never taints.
//!
//! `Driver::send` does not wait for the engine (pre-flight ruling C9), so these tests
//! assert decision 20's order: a later call that the engine answers (an `edit_plan`, or
//! the user's `run edit`) is applied after the read's event, so once its reply is in,
//! the record is set, or, for a read that is not a source, still `None`.

use proto::{AgentRole, TaskOrigin};
use serde_json::json;

use super::read_rig::{PLANNER, Rig};
use crate::run::delivery::ThreadState;
use crate::run::model::Run;
use crate::run::orch::test_support::task_mut;
use crate::run::orch::untrusted::UntrustedRecord;
use crate::run::orch::untrusted::tests::{COMMENT, quote_comment};
use crate::run::orch::{EpicRecord, PlannerPhase, PlannerSession};

const QUOTED: &str = "review comments on stage 1's pull request";

fn untrusted(rig: &Rig) -> Option<UntrustedRecord> {
    rig.run(|run| run.orch.untrusted.clone())
}

fn read_lines(rig: &Rig) -> Vec<String> {
    rig.run(|run| {
        (run.log.iter())
            .filter(|e| e.text.starts_with("the orchestrator read "))
            .map(|e| e.text.clone())
            .collect()
    })
}

/// A live sub-planner of epic `mail` in window [`PLANNER`].
fn with_planner(run: &mut Run) {
    let mut epic = EpicRecord::new("mail", PlannerPhase::Planning);
    epic.area = vec!["crates/m0/**".into()];
    epic.sessions.push(PlannerSession {
        session: 1,
        window_id: Some(PLANNER),
        op: Some(3),
        started_at: 1_000,
        ended_at: None,
        usage: Default::default(),
        rejections: 0,
    });
    run.orch.epics.push(epic);
}

/// Decision 20 (C9): a writer's current thread on stage 1 and a blocked `t1`. The
/// orchestrator reads the digest, then at once asks to override `t1`; that call is
/// decided after the read's event, so the record is set by the time it is answered.
#[tokio::test(flavor = "multi_thread")]
async fn run_status_with_a_quoted_comment_taints_before_a_later_edit() {
    let rig = Rig::new(|run, _| quote_comment(run, 1, COMMENT)).await;
    let (ok, digest) = rig.orch("run_status", json!({})).await;
    assert!(ok, "{digest}");
    let comment = digest["delivery"]["stages"][0]["threads"][0]["comment"].as_str();
    assert!(comment.unwrap().contains(COMMENT), "{digest}");
    let edit = json!({"op": "override", "task_id": "t1", "reason": "the fix is in"});
    let (_, answer) = rig.orch("edit_plan", json!({"edits": [edit]})).await;
    let record = untrusted(&rig).unwrap_or_else(|| panic!("no record when {answer} came"));
    assert_eq!(record.source, QUOTED);
    assert_eq!(
        read_lines(&rig),
        [format!(
            "the orchestrator read {QUOTED}; override, accept_red and approve_hold now go to the user"
        )]
    );
    // A second quoting read keeps the first record and logs nothing more.
    let (ok, _) = rig.orch("run_status", json!({})).await;
    assert!(ok);
    rig.cancel_t1().await;
    assert_eq!(untrusted(&rig), Some(record));
    assert_eq!(read_lines(&rig).len(), 1);
    rig.stop().await;
}

/// Decision 20 with decision 21 (task 8's C9 follow-up): `t1` has a start commit, so
/// the user's override rule lets it through; the override that follows a quoting
/// `run_status` is decided after the read's event, so it is asked of the user, never
/// run or refused, and the ask carries it as its proposal.
#[tokio::test(flavor = "multi_thread")]
async fn after_a_tainting_read_an_override_is_asked_of_the_user() {
    let rig = Rig::new(|run, _| {
        quote_comment(run, 1, COMMENT);
        task_mut(run, "t1").start_commit = Some("a".repeat(40));
    })
    .await;
    let (ok, digest) = rig.orch("run_status", json!({})).await;
    assert!(ok, "{digest}");
    let edit = json!({"op": "override", "task_id": "t1", "reason": "the fix is in"});
    let (ok, answer) = rig.orch("edit_plan", json!({"edits": [edit]})).await;
    assert!(ok, "{answer}");
    assert_eq!(
        answer,
        json!(
            "asked the user to decide: this session read text from outside the run, so override and accept_red are theirs now; their answer arrives as a message"
        )
    );
    let (ask, state) = rig.run(|run| (run.orch.ask.clone(), run.task("t1").unwrap().state));
    let ask = ask.expect("the user's question");
    assert_eq!(ask.question, "override t1? the fix is in");
    assert!(
        matches!(&ask.proposal, Some(proto::PlanEdit::Override { task_id, .. }) if task_id == "t1"),
        "{ask:?}"
    );
    assert_eq!(state, proto::TaskState::Blocked);
    assert!(rig.run(|run| run.orch.handled.is_empty()));
    rig.stop().await;
}

/// A delivery block whose thread is listed without text (already tasked) quotes
/// nothing: the read is not a source.
#[tokio::test(flavor = "multi_thread")]
async fn a_counts_only_delivery_block_does_not_taint() {
    let rig = Rig::new(|run, _| {
        quote_comment(run, 1, COMMENT);
        run.delivery.stages[0].threads[0].state = ThreadState::Tasked { task: "t0".into() };
    })
    .await;
    let (ok, digest) = rig.orch("run_status", json!({})).await;
    assert!(ok, "{digest}");
    let thread = &digest["delivery"]["stages"][0]["threads"][0];
    assert_eq!(thread["state"], "tasked", "{digest}");
    assert_eq!(thread["comment"], json!(null));
    assert!(!digest.to_string().contains(COMMENT));
    rig.cancel_t1().await;
    assert_eq!(untrusted(&rig), None);
    assert!(read_lines(&rig).is_empty());
    rig.stop().await;
}

/// Decision 19's second rule: `task_result` of a review fix task, while a plan task's
/// result is the run's own text.
#[tokio::test(flavor = "multi_thread")]
async fn task_result_of_a_review_fix_task_taints() {
    let rig = Rig::new(|run, _| {
        let mut t5 = run.task("t1").unwrap().clone();
        t5.spec.id = "t5".into();
        t5.origin = TaskOrigin::Review;
        run.tasks.push(t5);
    })
    .await;
    let (ok, result) = rig.orch("task_result", json!({"task_id": "t1"})).await;
    assert!(ok, "{result}");
    rig.cancel_t1().await;
    assert_eq!(untrusted(&rig), None, "a plan task");
    let (ok, result) = rig.orch("task_result", json!({"task_id": "t5"})).await;
    assert!(ok, "{result}");
    assert_eq!(result["task"]["id"], "t5");
    let edit = json!({"op": "cancel_task", "task_id": "t5"});
    let (ok, answer) = rig.orch("edit_plan", json!({"edits": [edit]})).await;
    assert!(ok, "{answer}");
    let record = untrusted(&rig).expect("set before the later call was answered");
    assert_eq!(record.source, "task t5's result (review fix)");
    rig.stop().await;
}

/// Decision 19: a sub-planner's reads never taint. `parse_call` gives a planner only
/// `get_context` and `submit_epic`, so its `run_status` and `task_result` are refused
/// before any digest or result is built; the driver's role check stands behind that.
#[tokio::test(flavor = "multi_thread")]
async fn a_subplanners_read_does_not_taint() {
    let rig = Rig::new(|run, _| {
        quote_comment(run, 1, COMMENT);
        with_planner(run);
        let mut t5 = run.task("t1").unwrap().clone();
        t5.spec.id = "t5".into();
        t5.origin = TaskOrigin::Review;
        run.tasks.push(t5);
        task_mut(run, "t5").spec.epic = Some("mail".into());
    })
    .await;
    let planner = rig.opts(AgentRole::Planner, PLANNER, Some("mail"));
    for (tool, args) in [
        ("run_status", json!({})),
        ("task_result", json!({"task_id": "t5"})),
    ] {
        let (ok, answer) = rig.call(planner.clone(), tool, args).await;
        assert!(!ok, "{answer}");
        let refusal = format!("tool {tool} is not available to the planner role");
        assert_eq!(answer, json!({"error": refusal}));
    }
    let (ok, context) = rig.call(planner, "get_context", json!({})).await;
    assert!(ok, "{context}");
    rig.cancel_t1().await;
    assert_eq!(untrusted(&rig), None);
    assert!(read_lines(&rig).is_empty());
    rig.stop().await;
}
