//! The pending results' file on its own: its backoff, and a file that does not parse.

use std::time::Duration;

use super::{PENDING_FILE, delay, load_blocking};

#[test]
fn the_backoff_doubles_per_failed_try() {
    let base = Duration::from_secs(30);
    let waits: Vec<u64> = (1..=5).map(|n| delay(base, n).as_secs()).collect();
    assert_eq!(waits, [30, 60, 120, 240, 480]);
    assert_eq!(delay(base, 0), base);
    assert_eq!(delay(Duration::MAX, 9), Duration::MAX);
}

#[test]
fn a_file_that_does_not_parse_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(PENDING_FILE);
    std::fs::write(&path, b"{\"v\":1,\"records\":[").unwrap();
    assert_eq!(load_blocking(&path).unwrap(), Vec::new());
    assert!(!path.exists());
    let aside: Vec<_> = (std::fs::read_dir(dir.path()).unwrap().flatten())
        .map(|e| e.path())
        .filter(|p| {
            p.to_string_lossy()
                .contains("lessons.pending.json.corrupt.")
        })
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(std::fs::read(&aside[0]).unwrap(), b"{\"v\":1,\"records\":[");
    // A second corrupt file does not overwrite the first.
    std::fs::write(&path, b"no").unwrap();
    assert_eq!(load_blocking(&path).unwrap(), Vec::new());
    let count = (std::fs::read_dir(dir.path()).unwrap().flatten())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("lessons.pending.json.corrupt.")
        })
        .count();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn a_save_never_makes_a_removed_repository_directory_again() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("gone");
    let record = super::PendingRecord {
        queued: "r/t1/distill".into(),
        lines: vec![],
        tries: 0,
        last_error: None,
        gave_up: false,
    };
    super::save(&repo, vec![record]).await;
    assert!(!repo.exists());
}

#[test]
fn no_file_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        load_blocking(&dir.path().join(PENDING_FILE)).unwrap(),
        Vec::new()
    );
}
