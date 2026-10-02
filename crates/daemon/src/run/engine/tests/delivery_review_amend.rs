//! The final fix wave's I-5 (review B): an amend of a review fix the orchestrator added
//! with `addresses` is not refused for the threads it already tasked. An amend re-checks
//! `addresses` only when it changes them, and then only for the threads this task has
//! not tasked; it is refused only when the task's stage has landed (its PR merged). A
//! closed stage is not landed and stays amendable.

use proto::PrState;
use serde_json::{Value, json};

use super::delivery_review::state;
use super::delivery_review_reply::{add, add_plan, planned};
use super::delivery_watch::PR;
use super::fixture::*;
use super::orch::{answer as orch_answer, edit_plan};
use crate::run::delivery::ThreadState;
use crate::run::delivery::validate;
use crate::run::model::FixOf;
use crate::run::orch::EditSource;

fn amend(fx: &mut Fixture, id: &str, brief: &str) -> (bool, Value) {
    let call = json!({"edits": [{"op": "amend_task", "task_id": id, "brief": brief}]});
    orch_answer(&edit_plan(fx, call))
}

/// `planned()`'s batch with `rev1` added for `7:c5`.
fn rev1() -> Fixture {
    let mut fx = planned();
    let (ok, value) = add_plan(&mut fx, add("rev1", &["docs/t1/**"], &["7:c5"]));
    assert!(ok, "{value}");
    fx
}

#[test]
fn an_amend_of_a_review_fixs_brief_succeeds() {
    let mut fx = rev1();
    let (ok, value) = amend(&mut fx, "rev1", "A better brief.");
    assert!(ok, "{value}");
    let t = fx.run().task("rev1").unwrap();
    assert_eq!(t.spec.brief, "A better brief.");
    let threads = vec!["7:c5".to_string()];
    assert_eq!(
        t.fixes,
        Some(FixOf::Review {
            stage: 1,
            pr: PR,
            threads
        })
    );
    let tasked = ThreadState::Tasked {
        task: "rev1".into(),
    };
    assert_eq!(state(&fx, "c5"), tasked);
    // A closed stage has not landed: its tasks stay amendable.
    fx.run_mut().delivery.stages[0].pr.as_mut().unwrap().state = PrState::Closed;
    let (ok, value) = amend(&mut fx, "rev1", "Once more.");
    assert!(ok, "{value}");
}

#[test]
fn an_amend_on_a_landed_stage_is_refused() {
    let mut fx = rev1();
    fx.run_mut().delivery.stages[0].pr.as_mut().unwrap().state = PrState::Merged;
    let (ok, value) = amend(&mut fx, "rev1", "Too late.");
    assert!(!ok);
    assert_eq!(
        value["errors"][0]["message"],
        "task rev1 is in stage 1, whose PR is merged; a landed stage's tasks cannot be amended"
    );
    assert_eq!(fx.run().task("rev1").unwrap().spec.brief, "Brief rev1");
}

/// `amend_task` has no `addresses` field, so the rule for an amend that changes them is
/// pinned on `validate::apply` itself: a fresh `new` thread joins, the task's own
/// tasked thread passes, and a thread another task holds is refused.
#[test]
fn an_amend_that_adds_a_fresh_new_thread_succeeds() {
    let mut fx = rev1();
    let (ok, value) = add_plan(&mut fx, add("rev2", &["docs/t1/**"], &["7:c7"]));
    assert!(ok, "{value}");
    let mut run = fx.run().clone();
    let source = EditSource::Orchestrator;
    let mut task = run.task("rev1").unwrap().clone();
    let before = task.spec.addresses.clone();
    task.spec.addresses = vec!["7:c5".into(), "7:c6".into()];
    let errors = validate::apply(&mut run, &mut task, &source, Some(&before));
    assert!(errors.is_empty(), "{errors:?}");
    let threads = vec!["7:c5".to_string(), "7:c6".to_string()];
    assert_eq!(
        task.fixes,
        Some(FixOf::Review {
            stage: 1,
            pr: PR,
            threads
        })
    );
    let stage = run.delivery.stage(1).unwrap();
    let of = |key: &str| {
        stage
            .threads
            .iter()
            .find(|t| t.key == key)
            .unwrap()
            .state
            .clone()
    };
    let tasked = ThreadState::Tasked {
        task: "rev1".into(),
    };
    assert_eq!((of("c5"), of("c6")), (tasked.clone(), tasked));
    // Another task's thread is not this amend's to take.
    task.spec.addresses = vec!["7:c5".into(), "7:c7".into()];
    let errors = validate::apply(&mut run, &mut task, &source, Some(&before));
    let text = "task rev1: addresses 7:c7, which is not a new thread of stage 1's PR";
    assert_eq!(
        errors
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>(),
        [text]
    );
}

/// Cleanup after the merge (c2): an amend that drops a thread from a review fix's
/// `addresses` returns that thread to `new` by the path a cancel takes (`untask`), so it
/// is handed out again; the threads the task still addresses stay its own. Dropping the
/// last one returns it too.
#[test]
fn an_amend_that_drops_a_thread_returns_it_to_new() {
    let mut fx = planned();
    let (ok, value) = add_plan(&mut fx, add("rev1", &["docs/t1/**"], &["7:c5", "7:c6"]));
    assert!(ok, "{value}");
    let tasked = ThreadState::Tasked {
        task: "rev1".into(),
    };
    assert_eq!(
        (state(&fx, "c5"), state(&fx, "c6")),
        (tasked.clone(), tasked.clone())
    );
    let amend_to = |fx: &mut Fixture, addresses: &[&str]| {
        let run = fx.run_mut();
        let i = run.tasks.iter().position(|t| t.id() == "rev1").unwrap();
        let mut task = run.tasks[i].clone();
        let before = task.spec.addresses.clone();
        task.spec.addresses = addresses.iter().map(|a| a.to_string()).collect();
        let errors = validate::apply(run, &mut task, &EditSource::Orchestrator, Some(&before));
        assert!(errors.is_empty(), "{errors:?}");
        run.tasks[i] = task;
        fx.tick();
    };
    amend_to(&mut fx, &["7:c5"]);
    assert_eq!(
        (state(&fx, "c5"), state(&fx, "c6")),
        (tasked, ThreadState::New)
    );
    let line = "stage 1 (PR #7): thread c6 is new again: its fix task rev1 no longer addresses it";
    assert!(
        fx.run().log.iter().any(|l| l.text == line),
        "{:#?}",
        fx.run().log
    );
    amend_to(&mut fx, &[]);
    assert_eq!(state(&fx, "c5"), ThreadState::New);
}
