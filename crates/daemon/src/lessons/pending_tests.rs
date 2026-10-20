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
    let aside = dir.path().join("lessons.pending.json.corrupt");
    assert_eq!(std::fs::read(aside).unwrap(), b"{\"v\":1,\"records\":[");
}

#[test]
fn no_file_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        load_blocking(&dir.path().join(PENDING_FILE)).unwrap(),
        Vec::new()
    );
}
