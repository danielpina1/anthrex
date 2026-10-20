//! PR #80's review, item 1: a crash mid-compaction leaves the rewrite's temporary file
//! (`lessons.jsonl.<pid>.<n>.tmp`, over 1 MiB), and nothing removed it. The sweep runs
//! under the repository's [`LessonLocks`] entry before each compaction, so a worker
//! start sweeps before compaction can run again; under that lock no rewrite is in
//! flight, so a temporary file of that exact shape is a crashed one's. The rewrite
//! also keeps the store's own mode, not `write_atomic`'s `0600`.

use std::os::unix::fs::PermissionsExt;

use super::super::LESSONS_FILE;
use super::tests_compact::{NOW, padding, write_lines};
use super::{Compacted, LessonLocks, compact, compact_locked};

/// Names beside the store that are not a crashed rewrite's temporary file.
const KEPT: [&str; 9] = [
    "lessons.jsonl.tmp",
    "lessons.pending.json.tmp",
    "lessons.pending.json.corrupt",
    "lessons.jsonl.12.tmp",
    "lessons.jsonl.a.0.tmp",
    "lessons.jsonl.1.2.tmp.old",
    "lessons.jsonl.1..tmp",
    "xlessons.jsonl.1.2.tmp",
    "profile.toml.1.2.tmp",
];

#[tokio::test]
async fn a_crashed_compactions_temp_file_is_swept_under_the_lock() {
    for small in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LESSONS_FILE);
        if small {
            write_lines(&path, &["{}".to_string()]);
        } else {
            write_lines(&path, &padding());
        }
        // The pending results' file (`super::super::pending`) is written by the same
        // worker, between its passes, so its crashed writes go too.
        let stale = [
            "lessons.jsonl.4242.0.tmp",
            "lessons.jsonl.7.63.tmp",
            "lessons.pending.json.5.1.tmp",
        ];
        for name in stale {
            std::fs::write(dir.path().join(name), vec![b'x'; 1024 * 1024]).unwrap();
        }
        for name in KEPT {
            std::fs::write(dir.path().join(name), b"kept").unwrap();
        }
        // A directory of the shape is no file of the rewrite's.
        std::fs::create_dir(dir.path().join("lessons.jsonl.9.9.tmp")).unwrap();

        let locks = LessonLocks::default();
        let done = compact_locked(&locks, dir.path(), NOW).await;
        let want = if small {
            Compacted::Skipped
        } else {
            Compacted::Rewrote {
                before: 5,
                after: 2,
            }
        };
        assert_eq!(done, Some(want), "small {small}");
        for name in stale {
            assert!(
                !dir.path().join(name).exists(),
                "{name} swept (small {small})"
            );
        }
        for name in KEPT {
            assert!(
                dir.path().join(name).is_file(),
                "{name} kept (small {small})"
            );
        }
        assert!(dir.path().join("lessons.jsonl.9.9.tmp").is_dir());
    }
}

#[test]
fn compaction_keeps_the_stores_mode() {
    for mode in [0o644, 0o600, 0o640] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LESSONS_FILE);
        write_lines(&path, &padding());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(matches!(
            compact(&path, NOW).unwrap(),
            Compacted::Rewrote { .. }
        ));
        let got = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(got, mode, "{mode:o}");
    }
}
