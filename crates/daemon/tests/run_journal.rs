//! M8a.21: the run file and the intent journal (decision 43), and reconcile on start
//! (decision 44) against real temporary repositories, restored-window lists and real
//! leftover processes.
//!
//! Split by seam (AGENTS.md rule 8): this file holds the journal's own tests,
//! `run_journal/fixture.rs` the runs and records they share, `run_journal/git.rs` the
//! per-kind reconcile rows that read git, `run_journal/git_guards.rs` fix round 1's
//! guards on them, and `run_journal/sessions.rs` the windows and
//! session processes.

mod support;

#[path = "run_journal/fixture.rs"]
mod fixture;
#[path = "run_journal/git.rs"]
mod git;
#[path = "run_journal/git_guards.rs"]
mod git_guards;
#[path = "run_journal/sessions.rs"]
mod sessions;

use daemon::run::engine::OpKind;
use daemon::run::journal::{
    JOURNAL_FILE, JournalLine, RUN_FILE, RUN_TMP, append, compact, load_all, save_run,
};
use fixture::{full_run, pend, plain_run, some_results};
use std::io::Write;
use std::path::PathBuf;

fn intent(op: u64) -> JournalLine {
    JournalLine::Intent {
        op,
        kind: OpKind::AbortMerge {
            worktree: PathBuf::from(format!("/tmp/wt-{op}")),
        },
    }
}

#[test]
fn save_run_is_atomic_and_round_trips() {
    let data = tempfile::tempdir().unwrap();
    let run = full_run(data.path());
    save_run(&run).unwrap();
    let dir = run.data_dir.clone();
    assert!(dir.join(RUN_FILE).is_file());
    assert!(!dir.join(RUN_TMP).exists(), "the temp file is renamed away");

    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].0, run,
        "every nested record survives the round trip"
    );
    assert!(runs[0].1.is_empty(), "no journal yet");

    // A crash between writing the temp file and the rename: the temp file holds a newer
    // run, possibly torn. It is ignored, removed, and the last complete save loads.
    let mut newer = run.clone();
    newer.goal = "a newer goal".into();
    let bytes = serde_json::to_vec(&newer).unwrap();
    std::fs::write(dir.join(RUN_TMP), &bytes[..bytes.len() / 2]).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, run);
    assert!(
        !dir.join(RUN_TMP).exists(),
        "the leftover temp file is removed"
    );

    // A later save replaces the file whole.
    save_run(&newer).unwrap();
    let (runs, _) = load_all(data.path());
    assert_eq!(runs[0].0.goal, "a newer goal");
}

#[test]
fn journal_lines_round_trip_and_a_torn_last_line_is_dropped() {
    let data = tempfile::tempdir().unwrap();
    let mut run = full_run(data.path());
    run.pending_ops.clear();
    save_run(&run).unwrap();
    let dir = run.data_dir.clone();

    let mut lines: Vec<JournalLine> = Vec::new();
    for (i, result) in some_results().into_iter().enumerate() {
        let op = i as u64 + 1;
        lines.push(intent(op));
        lines.push(JournalLine::Done { op, result });
    }
    let window = fixture::create_window(
        std::path::Path::new("/tmp/t1"),
        fixture::run_ref("t1", proto::AgentRole::Worker, 1),
        Some("00000000-0000-4000-8000-0000000000aa"),
    );
    lines.push(JournalLine::Intent {
        op: 9,
        kind: window,
    });
    for line in &lines {
        append(&dir, line).unwrap();
    }

    // Decision 43's shape: `{"op":<id>,"intent":{…}}` and `{"op":<id>,"done":{…}}`.
    let text = std::fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
    let first: Vec<&str> = text.lines().take(2).collect();
    assert!(
        first[0].starts_with(r#"{"op":1,"intent":{"AbortMerge":"#),
        "{}",
        first[0]
    );
    assert!(
        first[1].starts_with(r#"{"op":1,"done":{"Worktree":"#),
        "{}",
        first[1]
    );
    assert!(text.ends_with('\n'));

    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs[0].1, lines);

    // A crash mid-append: the last line has no newline.
    let torn = serde_json::to_string(&intent(10)).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(JOURNAL_FILE))
        .unwrap();
    file.write_all(&torn.as_bytes()[..torn.len() - 3]).unwrap();
    drop(file);
    let (runs, problems) = load_all(data.path());
    assert_eq!(runs[0].1, lines, "the torn line is dropped, the rest kept");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("torn"), "{problems:?}");
    assert!(problems[0].contains(JOURNAL_FILE), "{problems:?}");
}

#[test]
fn load_all_skips_a_bad_run_with_a_problem() {
    let data = tempfile::tempdir().unwrap();
    let run = plain_run(data.path());
    save_run(&run).unwrap();
    let bad = data.path().join("runs").join("a-bad-run-0000");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join(RUN_FILE), b"{ not a run").unwrap();
    let missing = data.path().join("runs").join("b-no-run-file-0000");
    std::fs::create_dir_all(&missing).unwrap();
    // A stray file beside the run directories is not a run.
    std::fs::write(data.path().join("runs").join("stray.txt"), b"x").unwrap();

    let (runs, problems) = load_all(data.path());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, run);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("a-bad-run-0000"), "{problems:?}");
    assert!(problems[0].contains("does not parse"), "{problems:?}");
    assert!(problems[1].contains("b-no-run-file-0000"), "{problems:?}");

    // No `runs/` at all is no runs and no problem.
    let empty = tempfile::tempdir().unwrap();
    let (runs, problems) = load_all(empty.path());
    assert!(runs.is_empty() && problems.is_empty());
}

#[test]
fn compact_keeps_only_pending_intents() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let dir = run.data_dir.clone();
    // Op 1 finished and was retired; op 2 is still running; op 3 returned but the
    // `Persist` that retires it has not happened yet.
    for line in [
        intent(1),
        JournalLine::Done {
            op: 1,
            result: daemon::run::engine::OpResult::MergeAborted,
        },
        intent(2),
        intent(3),
        JournalLine::Done {
            op: 3,
            result: daemon::run::engine::OpResult::RefsOk,
        },
    ] {
        append(&dir, &line).unwrap();
    }
    let JournalLine::Intent { kind: two, .. } = intent(2) else {
        unreachable!()
    };
    let JournalLine::Intent { kind: three, .. } = intent(3) else {
        unreachable!()
    };
    pend(&mut run, 2, Some("t1"), two);
    pend(&mut run, 3, Some("t2"), three);
    save_run(&run).unwrap();
    let before = std::fs::metadata(dir.join(JOURNAL_FILE)).unwrap().len();

    compact(&dir, &run.pending_ops).unwrap();

    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        runs[0].1,
        vec![
            intent(2),
            intent(3),
            JournalLine::Done {
                op: 3,
                result: daemon::run::engine::OpResult::RefsOk,
            },
        ],
        "op 1 is gone; the pending ops keep their intents (and op 3 its result)"
    );
    assert!(std::fs::metadata(dir.join(JOURNAL_FILE)).unwrap().len() < before);
    assert!(!dir.join("journal.jsonl.tmp").exists());

    // Nothing pending: an empty journal.
    compact(&dir, &Default::default()).unwrap();
    assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), b"");
}

/// Fix round 1, m3: a line compaction cannot read is reported, not silently dropped.
#[test]
fn compact_reports_the_lines_it_cannot_read() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let dir = run.data_dir.clone();
    append(&dir, &intent(2)).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(JOURNAL_FILE))
        .unwrap();
    file.write_all(b"{ not a journal line\n").unwrap();
    drop(file);
    let JournalLine::Intent { kind, .. } = intent(2) else {
        unreachable!()
    };
    pend(&mut run, 2, Some("t1"), kind);

    let problems = compact(&dir, &run.pending_ops).unwrap();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("line 2"), "{problems:?}");
    assert!(problems[0].contains("does not parse"), "{problems:?}");
}

/// Fix round 1, I2 (the review's probe P1): the first append after a crash that tore
/// the last line must not be glued onto the fragment.
#[test]
fn an_append_after_a_torn_line_survives_the_next_load() {
    let data = tempfile::tempdir().unwrap();
    let run = plain_run(data.path());
    save_run(&run).unwrap();
    let dir = run.data_dir.clone();
    append(&dir, &intent(1)).unwrap();
    let torn = serde_json::to_string(&intent(2)).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(JOURNAL_FILE))
        .unwrap();
    file.write_all(&torn.as_bytes()[..torn.len() / 2]).unwrap();
    drop(file);
    let (_, problems) = load_all(data.path());
    assert_eq!(problems.len(), 1, "{problems:?}");

    // The restarted daemon journals op 3.
    let done = JournalLine::Done {
        op: 3,
        result: daemon::run::engine::OpResult::RefsOk,
    };
    append(&dir, &intent(3)).unwrap();
    append(&dir, &done).unwrap();

    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "the torn tail is gone: {problems:?}");
    assert_eq!(runs[0].1, vec![intent(1), intent(3), done]);
    let text = std::fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
    assert_eq!(text.lines().count(), 3, "{text}");
}

/// Fix round 1, m4 (mutant M1): a compaction that died before its rename leaves
/// `journal.jsonl.tmp`; loading removes it and keeps the journal.
#[test]
fn load_all_removes_a_leftover_journal_temp_file() {
    let data = tempfile::tempdir().unwrap();
    let run = plain_run(data.path());
    save_run(&run).unwrap();
    let dir = run.data_dir.clone();
    append(&dir, &intent(1)).unwrap();
    std::fs::write(dir.join("journal.jsonl.tmp"), b"{\"op\":9,").unwrap();

    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs[0].1, vec![intent(1)]);
    assert!(!dir.join("journal.jsonl.tmp").exists());
}
