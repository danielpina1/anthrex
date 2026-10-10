//! M9.11.34 (C1, E1): the module-graph capture file never collides.

use super::graph_file;

/// The injected collision: two captures in one process at the same instant (the same
/// pid, the same `nanos`). Before the fix the second failed `File exists (os error 17)`,
/// CI 37632597289's message.
#[test]
fn the_same_instant_gets_two_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (first, _a) = graph_file(tmp.path(), 7).expect("the first capture file");
    let (second, _b) = graph_file(tmp.path(), 7).expect("the second capture file");
    assert_ne!(first, second);
    assert!(first.exists() && second.exists());
    for file in [&first, &second] {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let prefix = format!("anthrex-graph-{}-7", std::process::id());
        assert!(name.starts_with(&prefix), "{name}");
        assert!(name.ends_with(".json"), "{name}");
    }
}

/// A leftover that already has the next names (a recycled pid's, say) is stepped over:
/// `AlreadyExists` takes the next sequence number, within `GRAPH_FILE_TRIES`.
/// Not a fails-before test of C1: the old names had no sequence, so these planted
/// names could not match and it passed on the old code too. It pins the retry
/// relative to the sequence scheme: a sequence without the retry fails it.
#[test]
fn a_leftover_with_the_next_name_is_stepped_over() {
    let tmp = tempfile::tempdir().unwrap();
    let next = super::GRAPH_SEQ.load(std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let planted: Vec<_> = (next..next + 3)
        .map(|seq| tmp.path().join(format!("anthrex-graph-{pid}-9-{seq}.json")))
        .collect();
    for file in &planted {
        std::fs::write(file, "leftover").unwrap();
    }
    let (file, _handle) = graph_file(tmp.path(), 9).expect("a free name past the leftovers");
    assert!(!planted.contains(&file), "{}", file.display());
    for file in &planted {
        assert_eq!(std::fs::read_to_string(file).unwrap(), "leftover");
    }
}
