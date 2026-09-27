//! M8b.16: routing decisions recorded before a session starts (decision 33a), the diff
//! of a merged or cancelled task (decision 32), and `history.jsonl`'s task and run
//! records as journaled ops (decision 33).

use proto::{
    AgentRole, DiffStats, FinishAction, HistoryLine, PlanEdit, RunState, TaskOutcome, TaskState,
};

use super::control::resume;
use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::gates::{CHECK_MODE, check_result, only_op, working_on};
use super::gates_review::in_review;
use super::merge::{
    candidate, claim, commit, config, doc_task, head_of, merge, pending, pending_one, to_queue,
    window_of,
};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};

const REPO: &str = "/tmp/data/repos/x-3f9a";

fn history_path() -> std::path::PathBuf {
    std::path::PathBuf::from(REPO).join("history.jsonl")
}

/// A running run of `tasks` whose history is on, every dispatched worker launched.
fn start_history(profile: &str, tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    let plan = plan_with(profile, tasks);
    let mut fx = Fixture::with_config(&plan, config());
    fx.start_with(true, |run| run.repo_dir = REPO.into());
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let windows = fx.launch_all();
    (fx, windows)
}

/// The position of the first effect matching `f`.
fn at(effects: &[Effect], f: impl Fn(&Effect) -> bool) -> usize {
    effects
        .iter()
        .position(f)
        .unwrap_or_else(|| panic!("no such effect in {effects:#?}"))
}

fn is_launch(e: &Effect) -> bool {
    matches!(
        e,
        Effect::Op {
            kind: OpKind::CreateWindow { .. },
            ..
        }
    )
}

fn is_persist(e: &Effect) -> bool {
    matches!(e, Effect::Persist { urgent: true, .. })
}

/// The `AppendHistory` ops among `effects`: (op, record id, line).
fn appends(effects: &[Effect]) -> Vec<(u64, String, HistoryLine)> {
    ops_in(effects, "AppendHistory")
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::AppendHistory {
                path,
                record_id,
                line,
            } => {
                assert_eq!(path, history_path());
                (op, record_id, *line)
            }
            _ => unreachable!(),
        })
        .collect()
}

fn task_line(line: &HistoryLine) -> &proto::TaskRecord {
    match line {
        HistoryLine::Task(record) => record,
        other => panic!("a task line, not {other:?}"),
    }
}

#[test]
fn routing_decision_precedes_launch_and_survives_reconcile() {
    let plan = plan_with(PROFILE, &[doc_task("t1", "")]);
    let mut fx = Fixture::with_config(&plan, config());
    fx.ready(true);
    assert!(
        fx.task("t1").routing_decisions.is_empty(),
        "no decision before a launch"
    );
    let effects = fx.complete_prepares();
    // The decision is in the run the step persists, and that persist comes first.
    assert!(
        at(&effects, is_persist) < at(&effects, is_launch),
        "{effects:#?}"
    );
    let decisions = fx.task("t1").routing_decisions.clone();
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!((d.role, d.session, d.round), (AgentRole::Worker, 1, None));
    assert_eq!((d.trigger.as_str(), d.seq), ("initial", 1));
    assert_eq!(
        d.candidates[d.selected_index as usize].route,
        fx.task("t1").route
    );

    // The daemon restarts before the window came back: the launch is re-issued on the
    // first running pass, and no second decision is added.
    let (lost, _) = pending_one(&fx, "CreateWindow", Some("t1"));
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let (again, _) = only_op(&effects, "CreateWindow");
    assert_ne!(again, lost);
    assert_eq!(fx.task("t1").routing_decisions, decisions);
    // A replayed launch adds none either.
    restart(
        &mut fx,
        vec![(
            again,
            OpResult::Window {
                window_id: 9,
                pid: None,
            },
        )],
    );
    resume(&mut fx);
    assert_eq!(fx.task("t1").routing_decisions, decisions);
}

#[test]
fn a_reviewer_round_is_decided_before_its_launch() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (op, _) = in_review(&mut fx, window);
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: "diff".into(),
        },
    );
    assert!(
        at(&effects, is_persist) < at(&effects, is_launch),
        "{effects:#?}"
    );
    let decisions = &fx.task("t1").routing_decisions;
    assert_eq!(decisions.len(), 2, "{decisions:#?}");
    let review = &decisions[1];
    assert_eq!(
        (
            review.role,
            review.session,
            review.round,
            review.trigger.as_str()
        ),
        (AgentRole::Reviewer, 1, Some(1), "review")
    );
    assert_eq!(Some(&review.chosen), fx.task("t1").review_route.as_ref());
}

#[test]
fn a_rung_two_worker_records_its_escalation() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let before = fx.task("t1").route.clone();
    // Two failed checks: rung 2, a fresh session on the escalated route.
    for _ in 0..2 {
        let effects = super::gates::accepted(&mut fx, window, serde_json::json!({"summary": "s"}));
        let (op, _) = only_op(&effects, "Check");
        fx.done(op, check_result(false));
    }
    assert_eq!(fx.task("t1").rung, 2);
    assert_eq!(fx.task("t1").escalated_from.as_ref(), Some(&before));
    let effects = super::turns::killed_exit(&mut fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let decisions = &fx.task("t1").routing_decisions;
    assert_eq!(decisions.len(), 2, "{decisions:#?}");
    let d = &decisions[1];
    assert_eq!((d.session, d.trigger.as_str()), (2, "escalation"));
    assert_eq!(d.chosen, fx.task("t1").route);
    assert_ne!(d.chosen, before);
    assert_eq!(fx.task("t1").escalated_from, None);
}

#[test]
fn merged_task_measures_then_appends_once() {
    let (mut fx, windows) = start_history(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    let effects = fx.done(op, OpResult::Merged { commit: commit(1) });
    let (measure, kind) = only_op(&effects, "MeasureDiff");
    assert_eq!(
        kind,
        OpKind::MeasureDiff {
            root: "/tmp/x".into(),
            from: BASE.into(),
            to: commit(1),
            three_dot: false,
        }
    );
    assert!(
        appends(&effects).is_empty(),
        "the record waits for the diff"
    );
    assert_eq!(fx.task("t1").state, TaskState::Merged);
    for (op, _) in pending(&fx, "RemoveWorktree", Some("t1")) {
        let effects = fx.done(op, OpResult::Removed { salvage_ref: None });
        assert!(appends(&effects).is_empty());
    }
    let stats = DiffStats {
        files: 2,
        hunks: 3,
        added: 10,
        removed: 1,
    };
    let effects = fx.done(measure, OpResult::DiffMeasured(stats));
    let added = appends(&effects);
    assert_eq!(added.len(), 1, "{effects:#?}");
    let (append, id, line) = &added[0];
    assert_eq!(id, &format!("{RUN_ID}/t1"));
    let record = task_line(line);
    assert_eq!(record.outcome, TaskOutcome::Merged);
    assert_eq!(record.diff, Some(stats));
    assert_eq!(record.merge_commit, Some(commit(1)));
    assert!(record.phases.check + record.phases.working + record.phases.queued > 0);
    assert_eq!(fx.task("t1").diff, Some(stats));
    assert!(fx.task("t1").history_written);
    fx.tick();
    fx.done(*append, OpResult::HistoryAppended);
    fx.tick();
    assert_eq!(appends(&fx.log).len(), 1, "one record for t1");
    assert_eq!(fx.ops("MeasureDiff").len(), 1);
}

#[test]
fn a_failed_measure_appends_without_a_diff_and_history_off_emits_nothing() {
    let (mut fx, windows) = start_history(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    let (measure, _) = fx.op("MeasureDiff");
    let effects = fx.done(
        measure,
        OpResult::Failed {
            message: "git timed out".into(),
        },
    );
    let added = appends(&effects);
    assert_eq!(added.len(), 1);
    assert_eq!(task_line(&added[0].2).diff, None);
    assert_eq!(fx.ops("MeasureDiff").len(), 1, "never measured again");

    // A run restored from milestone 8a (no repository data directory) writes none.
    let (mut fx, windows) = super::merge::start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    fx.tick();
    assert!(fx.ops("MeasureDiff").is_empty() && fx.ops("AppendHistory").is_empty());
}

#[test]
fn cancel_without_a_recorded_head_appends_without_a_diff() {
    let profile = profile_with("max_writers = 1");
    let tasks = [doc_task("t1", ""), doc_task("t2", "")];
    let (mut fx, windows) = start_history(&profile, &tasks);
    // t2 never started: its cancel appends its record at once, with no diff.
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t2".into(),
        }],
    );
    assert!(ops_in(&effects, "MeasureDiff").is_empty());
    let added = appends(&effects);
    assert_eq!(added.len(), 1, "{effects:#?}");
    let record = task_line(&added[0].2);
    assert_eq!(
        (record.task_id.as_str(), record.outcome),
        ("t2", TaskOutcome::Cancelled)
    );
    assert_eq!(record.diff, None);

    // t1 claimed a head: `run cancel` measures what it did against the run head.
    claim(&mut fx, "t1", window_of(&windows, "t1"), &head_of("t1"));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    let (measure, kind) = only_op(&effects, "MeasureDiff");
    assert_eq!(
        kind,
        OpKind::MeasureDiff {
            root: "/tmp/x".into(),
            from: BASE.into(),
            to: head_of("t1"),
            three_dot: true,
        }
    );
    assert!(appends(&effects).is_empty());
    let effects = fx.done(measure, OpResult::DiffMeasured(DiffStats::default()));
    let added = appends(&effects);
    assert_eq!(added.len(), 1);
    assert_eq!(task_line(&added[0].2).outcome, TaskOutcome::Cancelled);
    assert_eq!(appends(&fx.log).len(), 2);
}

#[test]
fn accept_appends_the_run_record_after_the_tasks() {
    let (mut fx, windows) = start_history(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    let (measure, _) = fx.op("MeasureDiff");
    let effects = fx.done(measure, OpResult::DiffMeasured(DiffStats::default()));
    let (task_append, _, _) = appends(&effects)[0].clone();
    // The run completes only once its ops, the history's included, came back.
    assert!(pending(&fx, "VerifyRefs", None).is_empty());
    fx.done(task_append, OpResult::HistoryAppended);
    let (refs, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(refs, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(
        appends(&fx.log).len() == 1,
        "no run record before the accept"
    );
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action: FinishAction::Accept,
    });
    let (accept, _) = fx.op("Accept");
    let effects = fx.done(
        accept,
        OpResult::Finished {
            outcome: "merged".into(),
            kept_branches: Vec::new(),
        },
    );
    assert_eq!(fx.run().state, RunState::Accepted);
    let added = appends(&effects);
    assert_eq!(added.len(), 1, "{effects:#?}");
    let (run_append, id, line) = added[0].clone();
    assert_eq!(id, RUN_ID);
    let HistoryLine::Run(record) = &line else {
        panic!("a run line");
    };
    assert_eq!((record.outcome.as_str(), record.tasks), ("accepted", 1));
    assert_eq!(record.accepted_commit, None, "the driver fills it");
    assert!(fx.run().run_record_written);
    // Lost in a restart, it is appended again; a restored accepted run is not paused.
    let effects = restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Accepted);
    let again = appends(&effects);
    assert_eq!(again.len(), 1, "sent again");
    assert_ne!(again[0].0, run_append);
    assert_eq!(again[0].2, line);
    fx.done(again[0].0, OpResult::HistoryAppended);
    fx.tick();
    assert!(fx.run().pending_ops.is_empty());
    let all: Vec<String> = appends(&fx.log).into_iter().map(|(_, id, _)| id).collect();
    assert_eq!(
        all,
        vec![format!("{RUN_ID}/t1"), RUN_ID.into(), RUN_ID.into()]
    );
}

#[test]
fn a_failed_run_records_its_unfinished_tasks_then_itself() {
    let plan = plan_with(PROFILE, &[doc_task("t1", ""), doc_task("t2", "")]);
    let mut fx = Fixture::with_config(&plan, config());
    fx.start_with(true, |run| run.repo_dir = REPO.into());
    let (op, _) = fx.op("CreateRunBranch");
    let effects = fx.done(
        op,
        OpResult::Failed {
            message: "disk full".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Failed);
    let added = appends(&effects);
    let ids: Vec<String> = added.iter().map(|(_, id, _)| id.clone()).collect();
    assert_eq!(ids, vec![format!("{RUN_ID}/t1"), format!("{RUN_ID}/t2")]);
    for (_, _, line) in &added {
        assert_eq!(task_line(line).outcome, TaskOutcome::Unfinished);
    }
    // The run record waits for both task lines.
    let effects = fx.done(added[0].0, OpResult::HistoryAppended);
    assert!(appends(&effects).is_empty());
    let effects = fx.done(added[1].0, OpResult::HistoryAppended);
    let run = appends(&effects);
    assert_eq!(run.len(), 1);
    let HistoryLine::Run(record) = &run[0].2 else {
        panic!("a run line");
    };
    assert_eq!(record.outcome, "failed");
}
