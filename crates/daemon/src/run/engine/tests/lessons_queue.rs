//! Task M9.15.9 (decisions 9 and 10): a qualifying task or run record queues one
//! distill entry, as a journaled `QueueDistill` op whose line carries the redacted
//! dossier; the op is replayed or re-issued after a restart, and a failed write is
//! logged and lost. Fake secrets are built at run time (the shared test rules).

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use proto::{
    BlockInfo, BlockReason, DistillCause, Finding, LESSONS_VERSION, LessonLine, RunState, Severity,
    TaskState, Verdict,
};

use super::control_restore::restart;
use super::fixture::*;
use crate::lessons::LESSONS_FILE;
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::journal::JournalLine;
use crate::run::model::{
    CheckRecord, LessonLimits, OpId, PendingOp, ProofRecord, ReviewRecord, Run, RunLessons, Task,
};
use crate::run::reconcile::{Reconciled, reconcile};

const REPO: &str = "/tmp/data/repos/x-3f9a";
const NOW: u64 = 1_800_000_000;

fn ghp() -> String {
    format!("ghp_{}", "FAKE".repeat(9))
}

fn lessons_path() -> PathBuf {
    PathBuf::from(REPO).join(LESSONS_FILE)
}

/// Lessons on, distillation on, no risk note.
fn on() -> LessonLimits {
    LessonLimits {
        risk_note: false,
        ..LessonLimits::default()
    }
}

/// A running run of `ids` with history on and lessons frozen with `limits` (`None`: a
/// run from before lessons), its repository data directory `repo`.
fn started(ids: &[&str], limits: Option<LessonLimits>, repo: &str) -> Fixture {
    let tasks: Vec<String> = ids.iter().map(|id| task(id, "S", id, "")).collect();
    let mut fx = Fixture::new(&plan_with(PROFILE, &tasks));
    fx.start_with(true, |run| {
        run.repo_dir = repo.into();
        run.lessons = limits.map(|limits| RunLessons {
            limits,
            items: Vec::new(),
            frozen_at: NOW,
        });
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx
}

/// Every `QueueDistill` among `effects`: (op, path, record id, line).
fn queued(effects: &[Effect]) -> Vec<(OpId, PathBuf, String, LessonLine)> {
    ops_in(effects, "QueueDistill")
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::QueueDistill {
                path,
                record_id,
                line,
            } => (op, path, record_id, *line),
            other => panic!("not a QueueDistill: {other:?}"),
        })
        .collect()
}

/// Every history line still in flight answered `HistoryAppended`, and every distill
/// entry `DistillQueued`, until none is left.
fn settle(fx: &mut Fixture) {
    loop {
        let open: Vec<(OpId, OpResult)> = (fx.run().pending_ops.values())
            .filter_map(|p| match p.kind {
                OpKind::AppendHistory { .. } => Some((p.op, OpResult::HistoryAppended)),
                OpKind::QueueDistill { .. } => Some((p.op, OpResult::DistillQueued)),
                _ => None,
            })
            .collect();
        if open.is_empty() {
            return;
        }
        for (op, result) in open {
            fx.done(op, result);
        }
    }
}

/// The run ends as `state` after `edit`, and every record comes back: the `(record id,
/// cause)` of each distill entry queued.
fn end(
    fx: &mut Fixture,
    state: RunState,
    edit: impl FnOnce(&mut Run),
) -> Vec<(String, DistillCause)> {
    edit(fx.run_mut());
    fx.run_mut().state = state;
    fx.tick();
    settle(fx);
    queued(&fx.log)
        .into_iter()
        .map(|(_, path, record_id, line)| {
            assert_eq!(path, lessons_path());
            let LessonLine::Queued(q) = line else {
                panic!("a queued line, not {line:?}");
            };
            assert_eq!(q.record_id, record_id);
            (record_id, q.cause)
        })
        .collect()
}

fn failed_check() -> CheckRecord {
    CheckRecord {
        at: 1_200,
        ok: false,
        code: Some(1),
        timed_out: false,
        tail: "error[E0308]: mismatched types\n".into(),
        secs: 4,
        on_candidate: false,
        summary: None,
        summary_source: None,
        tier: None,
        lane: None,
    }
}

fn failed_proof() -> ProofRecord {
    ProofRecord {
        at: 1_100,
        test: "t_add".into(),
        red: "r".into(),
        head: "h".into(),
        red_failed: true,
        head_passed: false,
        matched: true,
        red_tail: String::new(),
        head_tail: "test t_add ... FAILED\n".into(),
        lane: None,
        red_only: false,
    }
}

fn review(task: &Task, verdict: Verdict, findings: &[Severity]) -> ReviewRecord {
    ReviewRecord {
        round: 1,
        route: task.route.clone(),
        base: "b".into(),
        head: "h".into(),
        verdict: Some(verdict),
        summary: "looked".into(),
        findings: (findings.iter())
            .map(|s| Finding {
                severity: *s,
                file: None,
                line: None,
                input: None,
                text: "a finding".into(),
            })
            .collect(),
        lane: None,
    }
}

fn merged(task: &mut Task) {
    task.state = TaskState::Merged;
    task.merge_commit = Some(HEAD.into());
}

type Edit = fn(&mut Task);

/// One task's edit and the cause its record queues, if any, in a run accepted after.
fn task_cases() -> Vec<(&'static str, Edit, Option<DistillCause>)> {
    use DistillCause::*;
    vec![
        (
            "blocked",
            |t| {
                t.state = TaskState::Blocked;
                t.block = Some(BlockInfo::new(BlockReason::Human, "which port?"));
            },
            Some(TaskBlocked),
        ),
        (
            "blocked mis-sized",
            |t| {
                t.state = TaskState::Blocked;
                t.block = Some(BlockInfo::new(BlockReason::MisSized, "too big"));
            },
            Some(TaskBlocked),
        ),
        (
            "merged after a failed check",
            |t| {
                merged(t);
                t.checks.push(failed_check());
            },
            Some(TaskRecovered),
        ),
        (
            "merged after a failed proof",
            |t| {
                merged(t);
                t.proofs.push(failed_proof());
            },
            Some(TaskRecovered),
        ),
        (
            "merged after a rejected review",
            |t| {
                merged(t);
                t.reviews.push(review(t, Verdict::Changes, &[]));
            },
            Some(TaskRecovered),
        ),
        (
            "merged without approval after a failed check",
            |t| {
                merged(t);
                t.merged_without_approval = Some("the user merged it".into());
                t.checks.push(failed_check());
            },
            Some(TaskRecovered),
        ),
        (
            "merged with an important finding",
            |t| {
                merged(t);
                t.reviews
                    .push(review(t, Verdict::Approve, &[Severity::Important]));
            },
            Some(ReviewBlocking),
        ),
        (
            "merged with a critical finding",
            |t| {
                merged(t);
                t.reviews
                    .push(review(t, Verdict::Approve, &[Severity::Critical]));
            },
            Some(ReviewBlocking),
        ),
        (
            "recovered with an important finding",
            |t| {
                merged(t);
                t.checks.push(failed_check());
                t.reviews
                    .push(review(t, Verdict::Approve, &[Severity::Important]));
            },
            Some(TaskRecovered),
        ),
        ("merged clean", merged, None),
        (
            "merged with a minor finding",
            |t| {
                merged(t);
                t.reviews
                    .push(review(t, Verdict::Approve, &[Severity::Minor]));
            },
            None,
        ),
        (
            "cancelled after a failed check",
            |t| {
                t.state = TaskState::Cancelled;
                t.checks.push(failed_check());
            },
            None,
        ),
        (
            "unfinished after a failed check",
            |t| {
                t.state = TaskState::Working;
                t.checks.push(failed_check());
            },
            None,
        ),
        ("reported", |t| t.state = TaskState::Reported, None),
    ]
}

#[test]
fn queue_distill_emitted_only_for_qualifying_causes() {
    let task_id = format!("{RUN_ID}/t1/distill");
    for (name, edit, cause) in task_cases() {
        let mut fx = started(&["t1"], Some(on()), REPO);
        let got = end(&mut fx, RunState::Accepted, |run| edit(&mut run.tasks[0]));
        let want: Vec<(String, DistillCause)> =
            cause.map(|c| (task_id.clone(), c)).into_iter().collect();
        assert_eq!(got, want, "{name}");
    }
    // The run's own record: a discarded or failed run queues one entry; an accepted
    // run, and a complete one, queue none.
    let run_id = format!("{RUN_ID}/run/distill");
    for (state, cause) in [
        (RunState::Discarded, Some(DistillCause::RunDiscarded)),
        (RunState::Failed, Some(DistillCause::RunFailed)),
        (RunState::Accepted, None),
        (RunState::Complete, None),
    ] {
        let mut fx = started(&["t1", "t2"], Some(on()), REPO);
        // t1 merged clean, t2 unfinished: neither queues, and a complete run's record
        // is due (one task is not finished).
        let got = end(&mut fx, state, |run| merged(&mut run.tasks[0]));
        let want: Vec<(String, DistillCause)> =
            cause.map(|c| (run_id.clone(), c)).into_iter().collect();
        assert_eq!(got, want, "{state:?}");
        assert!(
            fx.run().run_record_written,
            "{state:?}: the run record went out"
        );
    }
}

#[test]
fn nothing_queued_when_disabled_or_without_repo_dir() {
    let blocked = |run: &mut Run| {
        run.tasks[0].state = TaskState::Blocked;
        run.tasks[0].block = Some(BlockInfo::new(BlockReason::Human, "stuck"));
    };
    let cases = [
        (
            "lessons off",
            Some(LessonLimits {
                enabled: false,
                ..on()
            }),
            REPO,
        ),
        (
            "distill off",
            Some(LessonLimits {
                distill: false,
                ..on()
            }),
            REPO,
        ),
        ("a run from before lessons", None, REPO),
        ("no repository data directory", Some(on()), ""),
    ];
    for (name, limits, repo) in cases {
        let mut fx = started(&["t1"], limits, repo);
        let got = end(&mut fx, RunState::Failed, blocked);
        assert!(got.is_empty(), "{name}: {got:?}");
    }
    // The same run with both on queues the task and the run.
    let mut fx = started(&["t1"], Some(on()), REPO);
    assert_eq!(end(&mut fx, RunState::Failed, blocked).len(), 2);
}

#[test]
fn queue_line_carries_the_redacted_dossier_and_fps() {
    let mut fx = started(&["t1"], Some(on()), REPO);
    let secret = ghp();
    let edit = |run: &mut Run| {
        let task = &mut run.tasks[0];
        merged(task);
        let mut check = failed_check();
        check.tail = format!("error: auth failed for token {secret}\n");
        task.checks.push(check);
    };
    end(&mut fx, RunState::Accepted, edit);
    let entries = queued(&fx.log);
    assert_eq!(entries.len(), 1, "{entries:#?}");
    let LessonLine::Queued(line) = &entries[0].3 else {
        panic!("a queued line");
    };
    assert_eq!(line.v, LESSONS_VERSION);
    assert_eq!(line.run_id, RUN_ID);
    assert_eq!(line.task_id.as_deref(), Some("t1"));
    assert_eq!(line.cause, DistillCause::TaskRecovered);
    assert!(line.at > 0);
    // The dossier is decision 11's, built from the run, and holds no part of the token.
    let (dossier, fps) = crate::lessons::dossier::build(
        fx.run(),
        Some(&fx.run().tasks[0]),
        DistillCause::TaskRecovered,
        &on(),
    );
    assert_eq!(line.dossier, dossier);
    assert!(!line.dossier.contains("FAKEFAKE"), "{}", line.dossier);
    assert!(!line.dossier.contains("ghp_"), "{}", line.dossier);
    assert!(line.dossier.contains("## signatures"), "{}", line.dossier);
    // Its fingerprints are the task record's signatures'.
    let sigs = crate::lessons::signature::task_signatures(&fx.run().tasks[0]);
    assert!(!sigs.is_empty());
    let want: Vec<String> = sigs.into_iter().map(|s| s.fp).collect();
    assert_eq!(line.fps, want);
    assert_eq!(fps, want);
}

/// A running run whose t1 merged after a failed check while t2 still works: its
/// distill entry is in flight. Returns the op and its kind.
fn in_flight() -> (Fixture, OpId, OpKind) {
    let mut fx = started(&["t1", "t2"], Some(on()), REPO);
    let task = fx.task_mut("t1");
    merged(task);
    task.checks.push(failed_check());
    let effects = fx.tick();
    let entries = ops_in(&effects, "QueueDistill");
    assert_eq!(entries.len(), 1, "{effects:#?}");
    let (op, kind) = entries[0].clone();
    (fx, op, kind)
}

#[test]
fn queue_op_is_journaled_and_replayed() {
    let (fx, op, kind) = in_flight();
    // Journaled like every op: its intent line round-trips.
    let intent = JournalLine::Intent {
        op,
        kind: kind.clone(),
    };
    let text = serde_json::to_string(&intent).unwrap();
    let back: JournalLine = serde_json::from_str(&text).unwrap();
    assert_eq!(back, intent);
    let OpKind::QueueDistill { line, .. } = &kind else {
        unreachable!()
    };

    // Reconcile: the store holding the record (as a line that parses) replays it; a
    // store without it, or with only a torn line of it, re-issues it.
    let tmp = tempfile::tempdir().unwrap();
    let mut run = fx.run().clone();
    let path = tmp.path().join(LESSONS_FILE);
    let pending = PendingOp {
        op,
        task_id: None,
        kind: OpKind::QueueDistill {
            path: path.clone(),
            record_id: line.record_id().to_string(),
            line: line.clone(),
        },
        lane: None,
    };
    run.pending_ops.clear();
    run.pending_ops.insert(op, pending.clone());
    run.tasks.iter_mut().for_each(|t| t.rounds.clear());
    let journal = vec![JournalLine::Intent {
        op,
        kind: pending.kind.clone(),
    }];
    let git = OsStr::new("/nonexistent/anthrex-test/git");
    let check = |run: &Run| reconcile(git, run, &journal, &[], Duration::from_secs(1)).ops;
    assert_eq!(check(&run), vec![(op, Reconciled::NotStarted)], "no file");
    let whole = serde_json::to_string(line.as_ref()).unwrap();
    std::fs::write(&path, &whole[..whole.len() / 2]).unwrap();
    assert_eq!(
        check(&run),
        vec![(op, Reconciled::NotStarted)],
        "a torn line"
    );
    std::fs::write(&path, format!("{whole}\n")).unwrap();
    assert_eq!(
        check(&run),
        vec![(op, Reconciled::Replay(OpResult::DistillQueued))],
        "written"
    );

    // Restore, replayed: nothing is queued again.
    let (mut fx, op, _) = in_flight();
    let effects = restart(&mut fx, vec![(op, OpResult::DistillQueued)]);
    assert!(queued(&effects).is_empty(), "{effects:#?}");
    assert!(!fx.run().pending_ops.contains_key(&op));

    // Restore, not started: re-issued once, under a new id, with the same line.
    let (mut fx, op, kind) = in_flight();
    let effects = restart(&mut fx, Vec::new());
    let again = queued(&effects);
    assert_eq!(again.len(), 1, "{effects:#?}");
    assert_ne!(again[0].0, op);
    let OpKind::QueueDistill { line, .. } = kind else {
        unreachable!()
    };
    assert_eq!(again[0].3, *line);
    fx.done(again[0].0, OpResult::DistillQueued);
    let effects = fx.tick();
    assert!(queued(&effects).is_empty(), "{effects:#?}");

    // An ended run still owes it too.
    let mut fx = started(&["t1"], Some(on()), REPO);
    fx.task_mut("t1").state = TaskState::Blocked;
    fx.run_mut().state = RunState::Failed;
    let effects = fx.tick();
    let first = queued(&effects);
    assert_eq!(first.len(), 1, "{effects:#?}");
    let effects = restart(&mut fx, Vec::new());
    let again = queued(&effects);
    assert_eq!(again.len(), 1, "{effects:#?}");
    assert_eq!(again[0].3, first[0].3);
}

#[test]
fn a_failed_write_is_logged_and_lost() {
    let (mut fx, op, _) = in_flight();
    let id = format!("{RUN_ID}/t1/distill");
    let effects = fx.done(
        op,
        OpResult::Failed {
            message: "disk full".into(),
        },
    );
    assert!(queued(&effects).is_empty(), "lost, not sent again");
    assert!(!fx.run().pending_ops.contains_key(&op));
    let want = format!("lessons: the distill entry {id} was not written: disk full");
    assert!(
        fx.run().log.iter().any(|e| e.text == want),
        "{:#?}",
        fx.run().log
    );
    // A later step does not queue it again.
    let effects = fx.tick();
    assert!(queued(&effects).is_empty());
}
