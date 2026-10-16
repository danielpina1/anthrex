//! Milestone 9.16 decision 9: locating a Codex thread's rollout, and reading a file's tail.

use std::path::{Path, PathBuf};

use super::{TRANSCRIPT_TAIL_BYTES, find_rollout, read_tail};

const THREAD: &str = "019a0000-0000-7000-8000-00000000abcd";
/// 2026-10-15 12:00:00 UTC.
const START: u64 = 1_792_065_600;

/// `<sessions>/<YYYY>/<MM>/<DD>/<name>`, written.
fn put(sessions: &Path, date: &str, name: &str) -> PathBuf {
    let dir = date
        .split('-')
        .fold(sessions.to_path_buf(), |d, p| d.join(p));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, "{}\n").unwrap();
    path
}

fn rollout(time: &str, thread: &str) -> String {
    format!("rollout-{time}-{thread}.jsonl")
}

#[test]
fn find_rollout_lists_three_days_only() {
    assert_eq!(common::text::format_utc(START), "2026-10-15 12:00:00Z");
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    // A decoy with another thread's id, and the same name one level too deep.
    put(
        &sessions,
        "2026-10-15",
        &rollout("2026-10-15T09-00-00", "other-thread"),
    );
    let deep = sessions.join("2026/10/15/nested");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join(rollout("2026-10-15T09-00-00", THREAD)), "{}\n").unwrap();
    assert_eq!(
        find_rollout(&sessions, THREAD, START),
        None,
        "never below DD/"
    );

    // The thread's file under the day after (a local date ahead of UTC).
    let next = put(
        &sessions,
        "2026-10-16",
        &rollout("2026-10-16T01-30-00", THREAD),
    );
    assert_eq!(find_rollout(&sessions, THREAD, START), Some(next));
    // Under the start's own day, it is the one found first.
    let own = put(
        &sessions,
        "2026-10-15",
        &rollout("2026-10-15T12-00-01", THREAD),
    );
    assert_eq!(find_rollout(&sessions, THREAD, START), Some(own));

    // Only two days away: not found.
    let far = dir.path().join("far");
    put(&far, "2026-10-17", &rollout("2026-10-17T12-00-00", THREAD));
    put(&far, "2026-10-13", &rollout("2026-10-13T12-00-00", THREAD));
    assert_eq!(find_rollout(&far, THREAD, START), None);
    // The day before is searched too.
    let before = put(&far, "2026-10-14", &rollout("2026-10-14T23-59-59", THREAD));
    assert_eq!(find_rollout(&far, THREAD, START), Some(before));

    // No sessions folder, an empty or a path-like thread id: nothing.
    assert_eq!(find_rollout(&dir.path().join("none"), THREAD, START), None);
    assert_eq!(find_rollout(&sessions, "", START), None);
    assert_eq!(find_rollout(&sessions, "../x", START), None);
}

#[test]
fn read_tail_returns_at_most_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl");
    let bytes: Vec<u8> = (0..600 * 1024u32).map(|n| b'a' + (n % 26) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    let tail = read_tail(&path, TRANSCRIPT_TAIL_BYTES).unwrap();
    assert_eq!(TRANSCRIPT_TAIL_BYTES, 262_144);
    assert_eq!(tail.len(), 262_144);
    assert_eq!(tail.as_bytes(), &bytes[bytes.len() - 262_144..]);
    // A shorter file comes back whole.
    std::fs::write(&path, "short\n").unwrap();
    assert_eq!(read_tail(&path, TRANSCRIPT_TAIL_BYTES).unwrap(), "short\n");
    // A missing file is an error, not an empty tail.
    assert!(read_tail(&dir.path().join("missing"), 10).is_err());
}

/// Task 7's review M1: a FIFO named like the thread's rollout is not a rollout. The
/// listing skips it, and a cached path that became one is refused without opening it
/// (an `open` of a FIFO nobody writes would block for good). The read runs on a thread
/// of its own with a guard; past the guard the FIFO is opened `O_RDWR` (which never
/// blocks) so the stuck read is released before the test fails.
#[test]
fn a_fifo_named_like_a_rollout_is_skipped() {
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    let day = sessions.join("2026").join("10").join("15");
    std::fs::create_dir_all(&day).unwrap();
    let fifo = day.join(rollout("2026-10-15T12-00-01", THREAD));
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert_eq!(find_rollout(&sessions, THREAD, START), None);

    let (tx, rx) = std::sync::mpsc::channel();
    let read = fifo.clone();
    let reader = std::thread::spawn(move || {
        let _ = tx.send(read_tail(&read, TRANSCRIPT_TAIL_BYTES).is_err());
    });
    let refused = match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(refused) => refused,
        Err(_) => {
            let mut rdwr = std::fs::OpenOptions::new();
            rdwr.read(true).write(true);
            let _held = rdwr.open(&fifo).expect("a FIFO opens O_RDWR");
            reader.join().unwrap();
            panic!("read_tail opened a FIFO");
        }
    };
    reader.join().unwrap();
    assert!(refused, "a FIFO's tail was read");
}
