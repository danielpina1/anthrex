//! Milestone 9.5 task M9.5.17b: a racing task's requests (ruling RR-4, decision 23).
//! A message, a refresh and an amended brief reach every live lane; `stop_and_wait`,
//! `run retry` and `run override` are refused while it races; a lane's note keeps its
//! lane; a restart resumes each live lane. Then task 17a's re-review items (a)–(c):
//! one reader slot per reviewer after the crown, a retried race that ended without a
//! winner runs single on one slot, and a dispatched race has started.

use proto::{AgentRole, LaneState, MessageKind, PlanEdit, RaceLane, TaskState};
use serde_json::json;

use super::control::{override_task, resume, retry};
use super::control_restore::{restart, resumes};
use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::kinds::approve;
use super::race::{lane, lane_path, racing, tdd};
use super::race_crown::{crowned, handed_back};
use super::race_end::{adopted_on_a_question, b_out, never_prepared};
use super::race_lanes::{HEAD_B, proof, submit, to_review};
use super::turns::{exited, killed_exit};
use super::worker_messages::{from_user, msg};
use crate::run::contract::RESUME_WORKER;
use crate::run::edits_orch::{Takes, takes};
use crate::run::engine::schedule::{readers_busy, writer_slots};
use crate::run::engine::{Effect, OpKind, OpResult};

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;

/// The windows that got a `Deliver` containing `text` since log entry `from`, sorted.
fn delivered_to(fx: &Fixture, from: usize, text: &str) -> Vec<u32> {
    let mut windows: Vec<u32> = (fx.log[from..].iter())
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text: t, ..
            } if t.contains(text) => Some(*window_id),
            _ => None,
        })
        .collect();
    windows.sort();
    windows
}

/// Each racer's turn ends, and any commit count the turn end started is answered.
fn turns_end(fx: &mut Fixture, windows: &[u32]) {
    for window in windows {
        let effects = fx.turn_completed(*window);
        for (op, _) in ops_in(&effects, "CountCommits") {
            let commits = OpResult::Commits {
                count: 0,
                head: BASE.into(),
            };
            fx.done(op, commits);
        }
    }
}

fn amend_brief(brief: &str) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: Some(brief.into()),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: None,
        pair: None,
    }
}

/// Ruling RR-4: a message and an amended brief reach both live lanes; with lane b out,
/// lane a alone.
#[test]
fn messages_reach_every_live_lane() {
    let (mut fx, a, b) = racing();
    let from = fx.log.len();
    let effects = edit(
        &mut fx,
        vec![msg(&["t1"], MessageKind::Info, "use the v2 API")],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    turns_end(&mut fx, &[a, b]);
    let text = from_user(MessageKind::Info, "use the v2 API");
    assert_eq!(delivered_to(&fx, from, &text), [a, b]);
    let from = fx.log.len();
    let effects = edit(&mut fx, vec![amend_brief("do it in v2")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    turns_end(&mut fx, &[a, b]);
    assert_eq!(delivered_to(&fx, from, "Brief: do it in v2"), [a, b]);
    // Lane b leaves the race: the next message is lane a's alone.
    let args = json!({"kind": "question", "reason": "which API?"});
    fx.tool_as(AgentRole::Racer, b, "t1", "task_blocked", args);
    assert_eq!(lane(&fx, B).state, LaneState::Out);
    let from = fx.log.len();
    edit(&mut fx, vec![msg(&["t1"], MessageKind::Change, "now v3")]);
    turns_end(&mut fx, &[a]);
    assert_eq!(delivered_to(&fx, from, "now v3"), [a]);
    assert!(fx.run().outbox.iter().all(|m| m.task_id != "t1.b"));
}

/// Ruling RR-4: a refresh merges the run head into each live lane's checkout, each at
/// its racer's turn boundary.
#[test]
fn a_refresh_reaches_every_live_lane() {
    let (mut fx, a, b) = racing();
    let refresh = PlanEdit::Refresh {
        task_id: "t1".into(),
    };
    let effects = edit(&mut fx, vec![refresh]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    turns_end(&mut fx, &[a, b]);
    let mut worktrees: Vec<String> = (fx.ops("HandBack").into_iter())
        .map(|(_, kind)| match kind {
            OpKind::HandBack { worktree, .. } => worktree.display().to_string(),
            other => panic!("{other:?}"),
        })
        .collect();
    worktrees.sort();
    let lanes = [lane_path(A), lane_path(B)].map(|p| p.display().to_string());
    assert_eq!(worktrees, lanes);
}

/// Ruling RR-4 (review ruling I8): `stop_and_wait` to a racing task is refused on the
/// request path, as the MCP `message` tool and `run message --kind stop_and_wait` send
/// it; `info` and `change` are taken.
#[test]
fn stop_and_wait_is_refused_while_racing() {
    let (mut fx, _, _) = racing();
    let text = "task t1 is racing: it cannot stop and wait; message it or cancel it";
    let effects = edit(
        &mut fx,
        vec![msg(&["t1"], MessageKind::StopAndWait, "hold on")],
    );
    let reply = replies(&effects).pop().expect("a reply");
    assert!(reply.as_ref().is_err_and(|e| e.contains(text)), "{reply:?}");
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let t1 = fx.task("t1");
    assert_eq!(
        takes(t1, MessageKind::StopAndWait, 3).err().as_deref(),
        Some(text)
    );
    assert!(matches!(takes(t1, MessageKind::Info, 3), Ok(Takes::Queue)));
    assert!(matches!(
        takes(t1, MessageKind::Change, 3),
        Ok(Takes::Queue)
    ));
}

/// Decision 23: `run retry` and `run override` wait for adoption, with their texts;
/// once lane a is adopted and blocked, both apply.
#[test]
fn retry_and_override_wait_for_adoption() {
    let (mut fx, _, _) = racing();
    let effects = retry(&mut fx, "t1");
    let text = "task t1 is racing: it has nothing to retry until one racer is left";
    assert_eq!(replies(&effects), vec![Err(text.to_string())]);
    let effects = override_task(&mut fx, "t1", "ship it");
    let text = "task t1 is racing: there is no failed gate to override";
    assert_eq!(replies(&effects), vec![Err(text.to_string())]);
    let (mut fx, _) = adopted_on_a_question();
    let effects = retry(&mut fx, "t1");
    let reply = replies(&effects).pop().expect("a reply");
    assert!(
        reply
            .as_ref()
            .is_ok_and(|r| r.contains("retried at rung 2")),
        "{reply:?}"
    );
    let (mut fx, _) = adopted_on_a_question();
    let effects = override_task(&mut fx, "t1", "ship it");
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert_eq!(
        ops_in(&effects, "CountCommits").len(),
        1,
        "the override counts"
    );
}

/// Ruling RR-4: a racer's `task_note` is recorded with its lane, and the digest's note
/// entry says so.
#[test]
fn a_lanes_note_is_recorded_with_its_lane() {
    let (mut fx, _, b) = racing();
    let args = json!({"kind": "discovery", "text": "the API is v2"});
    let effects = fx.tool_as(AgentRole::Racer, b, "t1", "task_note", args);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let notes = &fx.task("t1").orch.worker_notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].lane, Some(B));
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    assert_eq!(digest["task_notes"][0]["lane"], "lane b", "{digest:#}");
}

/// Lane b's Codex racer has its session id (Codex reports it with its first turn), so
/// it can be resumed.
pub(super) fn with_codex_session(fx: &mut Fixture) {
    let t1 = fx.task_mut("t1");
    let racer = (t1.rounds.iter_mut()).find(|r| r.lane == Some(B) && r.role == AgentRole::Racer);
    racer.expect("lane b's racer").session_id = Some("thread-b".into());
}

/// Decision 23: a restart resumes each live lane's racer, and not a lane that is out.
#[test]
fn resume_after_restart_resumes_each_live_lane() {
    let (mut fx, a, b) = racing();
    with_codex_session(&mut fx);
    restart(&mut fx, Vec::new());
    let mut effects = resume(&mut fx);
    effects.extend(fx.tick());
    let mut windows: Vec<u32> = (resumes(&effects).into_iter())
        .filter(|(_, _, message)| message.contains(RESUME_WORKER))
        .map(|(window, _, _)| window)
        .collect();
    windows.sort();
    assert_eq!(windows, [a, b], "{effects:#?}");
    // With lane b out, lane a alone.
    let (mut fx, a, b) = b_out();
    with_codex_session(&mut fx);
    killed_exit(&mut fx, b);
    restart(&mut fx, Vec::new());
    let mut effects = resume(&mut fx);
    effects.extend(fx.tick());
    let windows: Vec<u32> = (resumes(&effects).into_iter())
        .filter(|(_, _, message)| message.contains(RESUME_WORKER))
        .map(|(window, _, _)| window)
        .collect();
    assert_eq!(windows, [a], "{effects:#?}");
}

/// Task 17a's re-review (a): after the crown, the loser's reviewer (given up, not yet
/// exited) holds its own reader slot only, so it neither stands in for the crowned
/// task's review nor counts twice.
#[test]
fn the_losers_reviewer_holds_one_reader_slot_after_the_crown() {
    let (mut fx, _, b) = racing();
    let reviewer_a = to_review(&mut fx, A, HEAD);
    let reviewer_b = to_review(&mut fx, B, HEAD_B);
    let effects = submit(&mut fx, reviewer_b, approve());
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    // Lane b's reviewer, done, exits; lane a's, given up, has not.
    exited(&mut fx, reviewer_b);
    assert_eq!(
        readers_busy(fx.run()),
        1,
        "lane a's reviewer until it exits"
    );
    handed_back(&mut fx);
    let effects = fx.tool_as(AgentRole::Racer, b, "t1", "task_done", tdd());
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut check = fx.clean_check("t1");
    if let OpResult::DoneChecked { head, .. } = &mut check {
        *head = HEAD_B.to_string();
    }
    let effects = fx.done(op, check);
    let (op, _) = only_op(&effects, "Proof");
    fx.turn_completed(b);
    let effects = fx.done(op, proof(true));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    match only_op(&effects, "PrepareReview").1 {
        OpKind::PrepareReview { path, .. } => assert_eq!(path, task_path("t1.b.review")),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        readers_busy(fx.run()),
        2,
        "the task's review and lane a's reviewer"
    );
    killed_exit(&mut fx, reviewer_a);
    assert_eq!(readers_busy(fx.run()), 1, "the task's review");
}

/// Task 17a's re-review (b): a race whose two lanes are both out is ended by `run
/// retry`; the task runs single in its own checkout and holds one writer slot.
#[test]
fn a_retried_race_with_both_lanes_out_runs_single_on_one_slot() {
    let mut fx = never_prepared();
    let effects = retry(&mut fx, "t1");
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert!(t1.race.as_ref().is_some_and(|r| r.ended), "{:?}", t1.race);
    let (_, kind) = fx.op("PrepareWorktree");
    assert!(matches!(kind, OpKind::PrepareWorktree { path, .. } if path == task_path("t1")));
    assert_eq!(writer_slots(t1), vec![t1.route.runtime]);
    let windows = fx.launch_all();
    assert_eq!(windows.len(), 1);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!(writer_slots(t1), vec![t1.route.runtime]);
    let worker = t1
        .rounds
        .iter()
        .rfind(|r| r.window_id == Some(windows[0].1));
    assert_eq!(
        worker.map(|r| (r.role, r.lane)),
        Some((AgentRole::Worker, None))
    );
}

/// Task 17a's second re-review (c): a race that was dispatched has started, so a
/// blocked one (its lanes out, no start commit of its own) refuses a race or pair
/// amend with the started-task text.
#[test]
fn a_dispatched_race_has_started() {
    let mut fx = never_prepared();
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.start_commit.as_deref()),
        (TaskState::Blocked, None)
    );
    assert!(crate::run::edits_state::has_started(t1));
    let amend = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: Some(false),
        pair: None,
    };
    let effects = edit(&mut fx, vec![amend]);
    let reply = replies(&effects).pop().expect("a reply");
    let text = "task t1 has started: its race cannot change";
    assert!(reply.as_ref().is_err_and(|e| e.contains(text)), "{reply:?}");
    assert!(fx.task("t1").spec.race);
}

/// An amend of `t1` setting only `size` or only `deps` (the started-task rule's fields).
fn amend(size: Option<proto::Size>, deps: Option<Vec<String>>, route: bool) -> PlanEdit {
    let route = route.then_some(proto::RouteSpec {
        runtime: None,
        model: None,
        strength: None,
        effort: None,
    });
    PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size,
        deps,
        stage: None,
        race: None,
        pair: None,
    }
}

/// Ruling T17b-1: a race that was dispatched has started whatever its state, so a
/// blocked one refuses an added dependency, a split, and a route, size or deps amend
/// with the started-task text, and nothing changes.
#[test]
fn a_blocked_race_refuses_deps_splits_and_amends_as_started() {
    let cases = [
        (
            PlanEdit::AddDep {
                task_id: "t1".into(),
                dep: "t0".into(),
            },
            "task t1 has started: its deps cannot change",
        ),
        (
            PlanEdit::SplitTask {
                task_id: "t1".into(),
                into: vec![super::holds::plan_task("t1x", "[\"crates/a/**\"]")],
            },
            "task t1 has started: it cannot be split",
        ),
        (
            amend(Some(proto::Size::S), None, false),
            "task t1 has started: its size cannot change",
        ),
        (
            amend(None, None, true),
            "task t1 has started: its route cannot change",
        ),
        (
            amend(None, Some(Vec::new()), false),
            "task t1 has started: its deps cannot change",
        ),
    ];
    for (change, text) in cases {
        let mut fx = never_prepared();
        assert_eq!(fx.task("t1").state, TaskState::Blocked);
        assert!(!crate::run::edits_state::not_started(fx.task("t1")));
        let before = fx.task("t1").spec.clone();
        let effects = edit(&mut fx, vec![change]);
        let reply = replies(&effects).pop().expect("a reply");
        assert!(
            reply.as_ref().is_err_and(|e| e.contains(text)),
            "{text}: {reply:?}"
        );
        assert_eq!(fx.task("t1").spec, before);
        assert_eq!(fx.run().tasks.len(), 1, "no split");
    }
}

/// Task 17b's review, m2: `run retry` refuses a racing task with a live lane whatever
/// its state (a blocked one here, a constructed state no path reaches today), and
/// never re-dispatches it single while its lanes run.
#[test]
fn retry_refuses_a_blocked_race_with_a_live_lane() {
    let (mut fx, _, _) = racing();
    fx.task_mut("t1").state = TaskState::Blocked;
    let effects = retry(&mut fx, "t1");
    let text = "task t1 is racing: it has nothing to retry until one racer is left";
    assert_eq!(replies(&effects), vec![Err(text.to_string())]);
    let race = fx.task("t1").race.clone().expect("a race");
    assert!(!race.ended);
    assert_eq!(
        (lane(&fx, A).state, lane(&fx, B).state),
        (LaneState::Working, LaneState::Working)
    );
}
