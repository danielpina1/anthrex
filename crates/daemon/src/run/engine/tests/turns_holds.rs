//! M8a.12 and the N5 hold (M8a.6 ruling N5, M8a.11 rulings T11-I1..I3): the paths this
//! task adds — a resume after a process exit, a resume carrying a delivery, and a
//! rung-2 fresh session — never bring a held task back to work before its hand-back.

use proto::{PlanEdit, TaskState};

use super::dispatch::edit;
use super::fixture::*;
use super::holds::{add_dep, answer, assert_held, delivers, hand_back_in_flight, plan_task};
use super::turns::{exited, killed_exit};
use crate::run::contract::{answer_message, is_conflict_message};
use crate::run::engine::{OpKind, OpResult};

/// t1 working with its first turn open, t2 unstarted and overlapping; t1's window.
fn working_t1(extra: &str) -> (Fixture, u32) {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", extra),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    (fx, window)
}

/// The worker asks a question with its turn still open (so its session is live), and
/// the user adds a dependency: t1 is held.
fn held_while_live(fx: &mut Fixture, window: u32) {
    let args = serde_json::json!({"kind": "question", "reason": "which table?"});
    fx.tool(window, "task_blocked", args);
    let effects = edit(fx, vec![add_dep("t1", "t2")]);
    assert!(
        super::dispatch::replies(&effects)[0].is_ok(),
        "{effects:#?}"
    );
    assert_held(fx);
}

#[test]
fn a_held_task_is_not_resumed_after_an_exit_or_by_a_delivery() {
    let (mut fx, window) = working_t1("");
    held_while_live(&mut fx, window);
    assert_held(&fx);
    // Its process dies mid-turn: no resume while held.
    let effects = exited(&mut fx, window);
    assert!(ops_in(&effects, "ResumeSession").is_empty(), "{effects:#?}");
    assert!(fx.task("t1").rounds[0].ended);
    // An answer waits: neither a delivery nor a resume carries it.
    let effects = edit(&mut fx, vec![answer("A")]);
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    assert!(delivers(&effects).is_empty());
    let effects = fx.tick();
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    assert_held(&fx);

    // t2 merges; the hand-back, then the resume carries the answer.
    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    assert!(
        ops_in(&effects, "ResumeSession").is_empty(),
        "hand-back first"
    );
    let (op, _) = ops_in(&effects, "HandBack")[0].clone();
    let effects = fx.done(
        op,
        OpResult::HandedBack {
            files: vec![],
            head: None,
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let resumes = ops_in(&effects, "ResumeSession");
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    let OpKind::ResumeSession { message, .. } = &resumes[0].1 else {
        unreachable!()
    };
    assert_eq!(message, &answer_message("A"));
}

/// A fresh session decided on (rung 2) cannot start while the task is held. Only a
/// working task reaches rung 2 and only a blocked one can gain a dependency, so the
/// pending session is set by hand here: the test pins the guard, not a sequence.
#[test]
fn a_held_task_gets_no_fresh_session_until_its_hand_back() {
    let (mut fx, window) = working_t1("");
    held_while_live(&mut fx, window);
    edit(&mut fx, vec![answer("A")]);
    fx.task_mut("t1").fresh_session = Some(crate::run::model::FreshSession {
        reason: "stalled".into(),
        append: None,
    });
    fx.task_mut("t1").rounds[0].retiring = true;
    let effects = killed_exit(&mut fx, window);
    assert!(ops_in(&effects, "DiffSoFar").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "CreateWindow").is_empty());
    let effects = fx.tick();
    assert!(ops_in(&effects, "DiffSoFar").is_empty());

    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    let (op, _) = ops_in(&effects, "HandBack")[0].clone();
    let effects = fx.done(
        op,
        OpResult::HandedBack {
            files: vec![],
            head: None,
        },
    );
    let diffs = ops_in(&effects, "DiffSoFar");
    assert_eq!(diffs.len(), 1, "{effects:#?}");
    let effects = fx.done(
        diffs[0].0,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1);
    assert_eq!(fx.task("t1").session, 2);
}

/// M8a.11 fix round 3's R5 guard in `abort_untold_conflict`: a conflict message whose
/// delivery is in flight was told, so a task held again is not undone.
#[test]
fn a_conflict_being_delivered_is_not_undone_when_the_task_is_held_again() {
    let mut fx = super::holds::blocked_t1();
    let op = hand_back_in_flight(&mut fx);
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(op, OpResult::HandedBack { files, head: None });
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(delivers(&effects).len(), 1, "answer and conflict in flight");
    // M8a.12 makes this reachable: the worker blocks the task while the delivery is in
    // flight, and a blocked task can gain a dependency.
    let window = fx.task("t1").rounds[0].window_id.unwrap();
    let args = serde_json::json!({"kind": "question", "reason": "which file?"});
    fx.tool(window, "task_blocked", args);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = edit(
        &mut fx,
        vec![
            PlanEdit::AddTask {
                task: plan_task("t3", "[\"crates/c/**\"]"),
            },
            add_dep("t1", "t3"),
        ],
    );
    assert!(fx.task("t1").awaiting_deps, "{effects:#?}");
    assert!(
        ops_in(&effects, "AbortMerge").is_empty(),
        "the worker was told"
    );
    let told = fx
        .run()
        .outbox
        .iter()
        .filter(|m| is_conflict_message(&m.text))
        .count();
    assert_eq!(told, 1, "the message in flight is kept");
}

/// Carry T12-P2: a held Codex task whose process exits mid-turn before it has a session
/// id ends its round with nothing to resume. Once the hold ends, the queued answer must
/// still reach a worker: a fresh session at the same rung and route, with no failure
/// counted, whose hand-over prompt ends with the answer.
#[test]
fn a_held_codex_task_that_lost_its_session_before_an_id_gets_a_fresh_one() {
    let codex = "[task.route]\nruntime = \"codex\"\nmodel = \"\"";
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", codex),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", codex),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").rounds[0].session_id, None);
    let route = fx.task("t1").route.clone();
    held_while_live(&mut fx, window);
    // No `Init` yet: the exit leaves nothing to resume.
    exited(&mut fx, window);
    assert!(fx.task("t1").rounds[0].ended);
    edit(&mut fx, vec![answer("A")]);
    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    let (op, _) = ops_in(&effects, "HandBack")[0].clone();
    let effects = fx.done(
        op,
        OpResult::HandedBack {
            files: vec![],
            head: None,
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let diffs = ops_in(&effects, "DiffSoFar");
    assert_eq!(diffs.len(), 1, "a fresh session: {effects:#?}");
    super::turns_fixes::assert_alive(&fx);
    let effects = fx.done(
        diffs[0].0,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let launches = ops_in(&effects, "CreateWindow");
    assert_eq!(launches.len(), 1, "{effects:#?}");
    let OpKind::CreateWindow { first_turn, .. } = &launches[0].1 else {
        unreachable!()
    };
    assert!(
        first_turn.ends_with(&format!("\n\n{}", answer_message("A"))),
        "{first_turn}"
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.session, t1.failures, t1.rung, t1.stalls), (2, 0, 0, 0));
    assert_eq!(t1.route, route, "no escalation");
    assert!(
        fx.run().outbox.is_empty(),
        "the answer went into the prompt"
    );
}
