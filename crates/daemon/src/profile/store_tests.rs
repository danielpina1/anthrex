//! Milestone 9.5 decision 36 (FU-F15): the daemon start's sweep of a repository's data
//! directory also removes the tier caches' and the tuning file's temp leftovers.

use super::sweep_leftovers;

#[test]
fn the_sweep_removes_the_tier_caches_and_tuning_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let names = |dir: &std::path::Path| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    for name in [
        "test-cache.jsonl.1.0.tmp",
        "module-graph.json.1.0.tmp",
        "tuning.toml.1.0.tmp",
        "tuning.toml",
        "other.json.1.0.tmp",
    ] {
        std::fs::write(dir.path().join(name), "x").unwrap();
    }
    sweep_leftovers(dir.path()).unwrap();
    assert_eq!(names(dir.path()), ["other.json.1.0.tmp", "tuning.toml"]);
}
