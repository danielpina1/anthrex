//! M8a.15: restore after a daemon restart and `run resume` (decisions 28, 44, 45).
//! Restore pauses a running run, ends every session, drops the ops the journal did not
//! replay and replays the rest; resume fires the second-stage deadlines that expired,
//! re-arms the first-stage ones, resumes each live session with its hand-over message
//! and re-issues what the tasks' states need. Every sequence ends with the liveness
//! check.

use proto::{AgentRole, RunState, TaskState};

use super::control::resume;
use super::dispatch::edit;
use super::fixture::*;
use super::gates::{CHECK_MODE, check_result, only_op};
use super::liveness::assert_alive;
use super::merge::{
    candidate, claim, commit, doc_task, head_of, pending, pending_one, start_on, to_queue,
    window_of,
};
use super::turns::working;
use crate::run::contract::{RESUME_REVIEWER, RESUME_WORKER, handover_prompt, stall_nudge};
use crate::run::engine::{AgentSignal, Effect, EngineState, EventKind, OpKind, OpResult};
use crate::run::model::{OpId, StallState};
use crate::run::role_launch::session_uuid;

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";

/// A new daemon: the fixture's run is restored from its `run.json` with `replay` as the
/// journal's results.
pub(super) fn restart(fx: &mut Fixture, replay: Vec<(OpId, OpResult)>) -> Vec<Effect> {
    let run = fx.run().clone();
    fx.state = EngineState::default();
    let replay = replay
        .into_iter()
        .map(|(op, result)| (RUN_ID.to_string(), op, result))
        .collect();
    fx.next(EventKind::Restore {
        held: Vec::new(),
        runs: vec![run],
        replay,
    })
}

/// The `(window, session, message)` of every `ResumeSession` among `effects`.
pub(super) fn resumes(effects: &[Effect]) -> Vec<(u32, String, String)> {
    ops_in(effects, "ResumeSession")
        .into_iter()
        .map(|(_, kind)| match kind {
            OpKind::ResumeSession {
                window_id,
                session_id,
                message,
                ..
            } => (window_id, session_id, message),
            _ => unreachable!(),
        })
        .collect()
}

#[test]
fn restore_pauses_running_runs_only() {
    let (fx, _) = working();
    let states = [
        RunState::Running,
        RunState::Halted,
        RunState::AwaitingApproval,
        RunState::Complete,
        RunState::Accepted,
    ];
    let runs: Vec<_> = states
        .iter()
        .enumerate()
        .map(|(k, &state)| {
            let mut run = fx.run().clone();
            run.id = format!("r{k}-3f9a");
            run.state = state;
            if state == RunState::Halted {
                run.halted_reason = Some("refs/heads/main was deleted".into());
            }
            run
        })
        .collect();
    let (state, _) = crate::run::engine::step(
        EngineState::default(),
        crate::run::engine::Event {
            now: 5_000,
            kind: EventKind::Restore {
                held: Vec::new(),
                runs,
                replay: Vec::new(),
            },
        },
    );
    for (k, &was) in states.iter().enumerate() {
        let run = &state.runs[&format!("r{k}-3f9a")];
        if was == RunState::Running {
            assert_eq!((run.state, run.paused_from), (RunState::Paused, Some(was)));
        } else {
            assert_eq!((run.state, run.paused_from), (was, None), "{was:?}");
        }
    }
}

#[test]
fn restore_replays_journaled_results() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    claim(&mut fx, "t2", window_of(&windows, "t2"), &head_of("t2"));
    let (merge_op, _) = candidate(&fx, "t1");
    let (check_op, _) = pending_one(&fx, "Check", Some("t2"));
    let effects = restart(
        &mut fx,
        vec![(merge_op, OpResult::Merged { commit: commit(1) })],
    );
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!(fx.task("t1").state, TaskState::Merged);
    assert_eq!(fx.run().run_head, commit(1));
    assert_eq!(ops_in(&effects, "RemoveWorktree").len(), 3, "{effects:#?}");
    // The check was not journaled as done: dropped, and a late result of it is too.
    assert!(pending(&fx, "Check", Some("t2")).is_empty());
    fx.done(check_op, check_result(false));
    assert_eq!(fx.task("t2").state, TaskState::Check);
    assert!(fx.task("t2").checks.is_empty(), "op-id correlation");
    assert_alive(&fx);
    let effects = resume(&mut fx);
    let (op, _) = only_op(&effects, "Check");
    assert_ne!(op, check_op, "re-issued under a new id");
    assert_alive(&fx);
}

#[test]
fn restore_marks_every_session_ended() {
    let (mut fx, worker, reviewer) = super::gates_review::reviewed(PROFILE, "");
    fx.signal(worker, AgentSignal::ProcessStarted { pid: 41 });
    fx.signal(reviewer, AgentSignal::ProcessStarted { pid: 42 });
    fx.signal(reviewer, AgentSignal::TurnStarted);
    restart(&mut fx, Vec::new());
    let t1 = fx.task("t1");
    assert_eq!(t1.rounds.len(), 2);
    for round in &t1.rounds {
        assert!(
            round.ended && !round.turn_open && !round.interrupted,
            "{round:#?}"
        );
        assert_eq!((round.pid, round.resume_op), (None, None));
    }
    assert!(fx.run().outbox.iter().all(|m| m.delivered_at.is_none()));
    assert_alive(&fx);
}

/// Decision 45: an interrupted stall whose grace ran out during the downtime is rung 2,
/// its old session not resumed; a watched round is re-armed from `now` and resumed.
#[test]
fn resume_fires_only_second_stage_deadlines() {
    let plan = plan_with(
        &profile_with("max_writers = 2"),
        &[task("t1", "S", "a", ROOMY), task("t2", "S", "b", ROOMY)],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let windows = fx.launch_all();
    let (w1, w2) = (window_of(&windows, "t1"), window_of(&windows, "t2"));
    let quiet = fx.task("t1").rounds[0].last_event + 600;
    fx.send(
        quiet - 1,
        EventKind::Signal {
            window_id: w2,
            signal: AgentSignal::Activity,
        },
    );
    let effects = fx.send(quiet, EventKind::Tick);
    assert!(
        effects.contains(&Effect::Interrupt { window_id: w1 }),
        "{effects:#?}"
    );
    restart(&mut fx, Vec::new());
    assert!(matches!(
        fx.task("t1").rounds[0].stall,
        StallState::Interrupted { .. }
    ));
    let at = quiet + 100;
    fx.now = at - 1;
    let effects = resume(&mut fx);
    let resumed = resumes(&effects);
    let s2 = fx.task("t2").rounds[0].session_id.clone().unwrap();
    assert_eq!(resumed, vec![(w2, s2, RESUME_WORKER.to_string())]);
    let t1 = fx.task("t1");
    assert_eq!((t1.rung, t1.stalls, t1.failures), (2, 1, 1));
    only_op(&effects, "DiffSoFar");
    let round = &fx.task("t2").rounds[0];
    assert_eq!((round.stall, round.last_event), (StallState::Watching, at));
    assert_alive(&fx);
}

/// An interrupt whose grace had not run out: the restart ended the interrupted turn, so
/// its `stall_nudge` goes with the resume, and the next silence is the second stall.
#[test]
fn an_interrupt_the_restart_ended_sends_its_nudge_with_the_resume() {
    let (mut fx, window) = super::turns::working_on(ROOMY);
    let quiet = fx.task("t1").rounds[0].last_event + 600;
    fx.send(quiet, EventKind::Tick);
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let nudge = stall_nudge(10);
    let resumed = resumes(&effects);
    assert_eq!(resumed.len(), 1, "{effects:#?}");
    assert_eq!(resumed[0].0, window);
    assert_eq!(resumed[0].2, format!("{nudge}\n\n{RESUME_WORKER}"));
    assert_eq!(fx.task("t1").rounds[0].stall, StallState::Nudged);
    assert_alive(&fx);
}

#[test]
fn resume_resumes_sessions_and_reissues_gate_ops() {
    let tasks = [
        doc_task("tw", ""),
        doc_task("tc", ""),
        task_toml("tr", "M", "[\"docs/tr/**\"]", CHECK_MODE),
        doc_task("tm", ""),
    ];
    let (mut fx, windows) = start_on(&profile_with("max_writers = 4"), &tasks);
    let w = |id| window_of(&windows, id);
    fx.signal(
        w("tw"),
        AgentSignal::Init {
            session_id: "s-tw".into(),
        },
    );
    claim(&mut fx, "tc", w("tc"), &head_of("tc"));
    to_queue(&mut fx, "tr", w("tr"));
    let (op, _) = pending_one(&fx, "PrepareReview", Some("tr"));
    fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: head_of("tr"),
            patch: "diff".into(),
        },
    );
    let reviewer = fx.complete_windows()[0].1;
    fx.signal(
        reviewer,
        AgentSignal::Init {
            session_id: "s-rw".into(),
        },
    );
    to_queue(&mut fx, "tm", w("tm"));
    candidate(&fx, "tm");
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let mut resumed = resumes(&effects);
    resumed.sort();
    let mut expected = vec![
        (w("tw"), "s-tw".to_string(), RESUME_WORKER.to_string()),
        (reviewer, "s-rw".to_string(), RESUME_REVIEWER.to_string()),
    ];
    expected.sort();
    assert_eq!(resumed, expected, "workers in a gate get their mail later");
    assert!(
        ops_in(&effects, "PrepareReview").is_empty(),
        "one live reviewer: the resumed one\n{effects:#?}"
    );
    let checks = pending(&fx, "Check", Some("tc"));
    assert_eq!(checks.len(), 1);
    assert_eq!(ops_in(&effects, "Check").len(), 1, "{effects:#?}");
    assert_eq!(ops_in(&effects, "MergeCandidate").len(), 1, "{effects:#?}");
    candidate(&fx, "tm");
    assert_eq!(fx.task("tr").state, TaskState::Review);
    assert_alive(&fx);
}

#[test]
fn a_failed_resume_starts_a_fresh_session() {
    let (mut fx, window) = working();
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let (op, _) = only_op(&effects, "ResumeSession");
    let effects = fx.done(
        op,
        OpResult::ResumeFailed {
            error: "no such session".into(),
        },
    );
    let (op, _) = only_op(&effects, "DiffSoFar");
    let (stat, patch) = (" a | 2 +-".to_string(), "+y".to_string());
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: stat.clone(),
            patch: patch.clone(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    let t1 = fx.task("t1");
    let reason = "the session could not be resumed: no such session";
    let expected = handover_prompt(fx.run(), t1, reason, &stat, &patch);
    assert_eq!(first_turn, format!("{expected}\n\n{RESUME_WORKER}"));
    assert_eq!((t1.session, t1.rung, t1.failures), (2, 0, 0));
    assert!(t1.rounds[0].ended && t1.rounds[0].window_id == Some(window));
    fx.complete_windows();
    assert_alive(&fx);
}

/// Decision 45: a halted run's sessions end at the restart too, and `resume
/// --rebaseline` resumes them like any other resume.
#[test]
fn a_restored_halted_run_resumes_its_sessions_on_rebaseline() {
    let (mut fx, window) = working();
    let run = fx.run_mut();
    run.state = RunState::Halted;
    run.halted_reason = Some("refs/heads/main was deleted".into());
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Halted);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.into(), BASE.into())),
    });
    let session = fx.task("t1").rounds[0].session_id.clone().unwrap();
    assert_eq!(
        resumes(&effects),
        vec![(window, session, RESUME_WORKER.to_string())]
    );
    assert_alive(&fx);
}

/// The restart's downtime is no session time: a session resumed hours later is not
/// over its minutes budget (decision 45's "downtime is not a stall", read for decision
/// 40's wall clock).
#[test]
fn a_restored_session_is_not_charged_for_the_downtime() {
    let (mut fx, window) = working();
    restart(&mut fx, Vec::new());
    fx.now += 10 * 3_600;
    let effects = resume(&mut fx);
    assert_eq!(resumes(&effects).len(), 1, "{effects:#?}");
    let effects = fx.tick();
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.budget_exceeded),
        (TaskState::Working, 0, 0)
    );
    assert_alive(&fx);
}

/// Carry M8a.14 (T14-R2): a deferred cancel whose merge did not survive the restart is
/// applied; a merge queue's lost hand-back is sent again once the run runs.
#[test]
fn restore_honours_deferred_cancels_and_lost_hand_backs() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "deps = [\"t1\"]")];
    let (mut fx, windows) = start_on(PROFILE, &tasks);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    edit(
        &mut fx,
        vec![proto::PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(fx.task("t1").cancel_deferred);
    restart(&mut fx, Vec::new());
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert_eq!(fx.task("t2").state, TaskState::Blocked);
    assert_alive(&fx);

    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into()],
        },
    );
    let (lost, _) = pending_one(&fx, "HandBack", Some("t1"));
    let effects = restart(&mut fx, Vec::new());
    assert!(ops_in(&effects, "HandBack").is_empty(), "not while paused");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert_alive(&fx);
    let effects = resume(&mut fx);
    let (op, kind) = only_op(&effects, "HandBack");
    assert_ne!(op, lost);
    let OpKind::HandBack { task_head, .. } = kind else {
        unreachable!()
    };
    assert_eq!(task_head.as_deref(), Some(head_of("t1").as_str()));
    assert_eq!(fx.task("t1").merge_op, Some(op));
    assert_alive(&fx);
}

/// Decision 44: a launch the journal did not see finish is re-issued on resume, under a
/// new op id and so a new session id; a lost integration worktree or abort is re-issued
/// at the restore itself (git only, no session).
#[test]
fn restore_reissues_lost_launches_and_git_ops() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.complete_prepares();
    let (lost, kind) = fx.op("CreateWindow");
    let effects = restart(&mut fx, Vec::new());
    assert!(
        ops_in(&effects, "CreateWindow").is_empty(),
        "no session while paused"
    );
    assert_alive(&fx);
    let effects = resume(&mut fx);
    let (op, relaunched) = only_op(&effects, "CreateWindow");
    assert_ne!(op, lost);
    let (
        OpKind::CreateWindow {
            first_turn: old_turn,
            ..
        },
        OpKind::CreateWindow {
            first_turn,
            session_uuid: uuid,
            ..
        },
    ) = (kind, relaunched)
    else {
        unreachable!()
    };
    assert_eq!(first_turn, old_turn);
    let fresh = session_uuid(RUN_ID, op);
    assert_eq!(uuid.as_deref(), Some(fresh.as_str()));
    let round = &fx.task("t1").rounds[0];
    assert_eq!(
        (round.launch_op, round.session_id.as_deref()),
        (op, Some(fresh.as_str()))
    );
    assert_eq!(fx.task("t1").rounds.len(), 1, "one session");
    fx.complete_windows();
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_alive(&fx);

    // The integration worktree of a run at its plan gate.
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.start(false);
    let (lost, _) = fx.op("CreateRunBranch");
    let effects = restart(&mut fx, Vec::new());
    let (op, _) = only_op(&effects, "CreateRunBranch");
    assert_ne!(op, lost);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
}

/// Carry M8a.11 (T11-RR2): an `AbortMerge` the journal did not see finish is sent again
/// (it does nothing when no merge is in progress), so the next hand-back never finds
/// the worktree mid-merge.
#[test]
fn a_lost_abort_is_sent_again_at_the_restore() {
    let mut fx = super::holds::blocked_t1();
    let first = super::holds::hand_back_in_flight(&mut fx);
    edit(
        &mut fx,
        vec![
            proto::PlanEdit::AddTask {
                task: super::holds::plan_task("t3", "[\"crates/c/**\"]"),
            },
            super::holds::add_dep("t1", "t3"),
        ],
    );
    let effects = fx.done(
        first,
        OpResult::HandedBack {
            files: vec!["crates/a/x.rs".into()],
            head: None,
            onto: None,
        },
    );
    let (lost, _) = only_op(&effects, "AbortMerge");
    let effects = restart(&mut fx, Vec::new());
    let (op, kind) = only_op(&effects, "AbortMerge");
    assert_ne!(op, lost);
    assert_eq!(
        kind,
        OpKind::AbortMerge {
            worktree: fx.task("t1").worktree.clone()
        }
    );
    assert_alive(&fx);
}

/// A reviewer in `review` whose session had no id to resume (a Codex reviewer before
/// its `thread.started`) is replaced by a new round, with no verdict-less round
/// counted.
#[test]
fn a_reviewer_with_no_session_id_is_replaced_on_resume() {
    let (mut fx, _, _) = super::gates_review::reviewed(PROFILE, "");
    let reviewer = fx.task("t1").rounds.last().unwrap();
    assert_eq!(
        (reviewer.role, &reviewer.session_id),
        (AgentRole::Reviewer, &None)
    );
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    assert!(resumes(&effects).is_empty(), "{effects:#?}");
    only_op(&effects, "PrepareReview");
    assert_eq!(fx.task("t1").review_misses, 0);
    assert_alive(&fx);
}

/// M8a.14's R12, reachable through the pause: refs read while the run is paused do not
/// complete it; the resumed run reads them again.
#[test]
fn refs_verified_while_paused_do_not_complete_the_run() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    super::merge::merge(&mut fx, "t1", &commit(1));
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    edit(&mut fx, vec![proto::PlanEdit::Pause]);
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Paused);
    assert_alive(&fx);
    let effects = resume(&mut fx);
    let (again, _) = only_op(&effects, "VerifyRefs");
    assert_ne!(again, op);
    fx.done(again, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
}

/// M8a.14's R14, reachable through the pause: a merge queue's hand-back that comes back
/// after its task was cancelled changes nothing.
#[test]
fn a_hand_back_result_after_a_cancel_is_dropped() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    let files = vec!["docs/t1/a.md".to_string()];
    fx.done(
        op,
        OpResult::Conflict {
            files: files.clone(),
        },
    );
    let (op, _) = pending_one(&fx, "HandBack", Some("t1"));
    edit(&mut fx, vec![proto::PlanEdit::Pause]);
    edit(
        &mut fx,
        vec![proto::PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    fx.done(
        op,
        OpResult::HandedBack {
            files,
            head: Some(HEAD.into()),
            onto: Some(head_of("t1")),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.resolving, t1.handed_back),
        (TaskState::Cancelled, false, false)
    );
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
    assert_alive(&fx);
    resume(&mut fx);
    assert_alive(&fx);
}
