//! Task M9.15.3: decision 3's writes and decision 4's read, on real temporary
//! directories.

use std::sync::Arc;

use proto::{LESSONS_VERSION, LessonLine, RetiredLine, VerdictLine};

use super::super::LESSONS_FILE;
use super::{LessonLocks, append, append_once, read_store};

fn verdict(n: u64) -> LessonLine {
    LessonLine::Verdict(VerdictLine {
        v: LESSONS_VERSION,
        record_id: format!("l-0123456789abcdef/verdict/{n}"),
        at: n,
        id: "l-0123456789abcdef".to_string(),
        approved: n.is_multiple_of(2),
    })
}

fn retired(at: u64) -> LessonLine {
    LessonLine::Retired(RetiredLine {
        v: LESSONS_VERSION,
        record_id: "l-0123456789abcdef/retired".to_string(),
        at,
        id: "l-0123456789abcdef".to_string(),
        reason: "forgotten by the user".to_string(),
    })
}

#[tokio::test]
async fn append_once_is_idempotent_by_record_id() {
    let dir = tempfile::tempdir().unwrap();
    // The repository's data directory need not exist yet.
    let repo = dir.path().join("repos").join("r");
    let locks = LessonLocks::default();
    assert!(append_once(&locks, &repo, retired(1)).await.unwrap());
    assert!(!append_once(&locks, &repo, retired(2)).await.unwrap());
    let (lines, problems) = read_store(&repo.join(LESSONS_FILE));
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(lines, [retired(1)]);
}

#[tokio::test]
async fn a_torn_last_line_gets_a_newline_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    let torn = "{\"type\":\"verdict\",\"v\":1,\"rec";
    std::fs::write(&path, torn).unwrap();
    let locks = LessonLocks::default();
    append(&locks, dir.path(), verdict(7)).await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text,
        format!("{torn}\n{}\n", serde_json::to_string(&verdict(7)).unwrap())
    );
    let (lines, problems) = read_store(&path);
    assert_eq!(lines, [verdict(7)]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("line 1: "), "{problems:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn appends_from_two_tasks_never_interleave() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
    let locks = Arc::new(LessonLocks::default());
    let mut tasks = Vec::new();
    for half in 0..2u64 {
        let (locks, repo) = (locks.clone(), repo.clone());
        tasks.push(tokio::spawn(async move {
            for n in 0..200u64 {
                append(&locks, &repo, verdict(half * 1000 + n))
                    .await
                    .unwrap();
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let text = std::fs::read_to_string(repo.join(LESSONS_FILE)).unwrap();
    assert_eq!(text.lines().count(), 400);
    let (lines, problems) = read_store(&repo.join(LESSONS_FILE));
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(lines.len(), 400);
    // Each task's lines are in its own order.
    for half in 0..2u64 {
        let ats: Vec<u64> = lines
            .iter()
            .map(LessonLine::at)
            .filter(|at| at / 1000 == half)
            .collect();
        assert_eq!(ats, (0..200).map(|n| half * 1000 + n).collect::<Vec<_>>());
    }
}

#[test]
fn read_store_of_a_missing_file_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (lines, problems) = read_store(&dir.path().join(LESSONS_FILE));
    assert!(lines.is_empty());
    assert!(problems.is_empty());
    // An unreadable store (a directory here) is one problem, not a panic.
    let (lines, problems) = read_store(dir.path());
    assert!(lines.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("could not read "), "{problems:?}");
}
