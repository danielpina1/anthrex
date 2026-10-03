//! Milestone 9.5 decision 36, fix round 1: the run-written files' temp leftovers (the
//! tier caches and the tuning file) are swept by the run restore, which the daemon
//! start runs before anything can serve and before any restored op is spawned; the
//! profile restore's later sweep never touches them, so it cannot unlink the temp file
//! of a cache write a restored op has already started.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::RunService;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const PLANTED: [&str; 6] = [
    "test-cache.jsonl.1.0.tmp",
    "module-graph.json.1.0.tmp",
    "tuning.toml.1.0.tmp",
    "tuning.toml",
    "profile.toml.1.0.tmp",
    "other.json.1.0.tmp",
];

/// A repository data directory under `data` holding every [`PLANTED`] file.
fn planted(data: &Path) -> PathBuf {
    let repo = data.join("repos").join("r-00000000");
    std::fs::create_dir_all(&repo).unwrap();
    for name in PLANTED {
        std::fs::write(repo.join(name), "x").unwrap();
    }
    repo
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_removes_the_tier_caches_and_tuning_leftovers() {
    let data = tempfile::tempdir().unwrap();
    let repo = planted(data.path());
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, data.path().to_path_buf(), Arc::new(NoRoots));
    s.restore().await;
    // The profile's own leftover waits for the profile restore's sweep.
    let left = ["other.json.1.0.tmp", "profile.toml.1.0.tmp", "tuning.toml"];
    assert_eq!(names(&repo), left);
}

#[test]
fn the_profile_sweep_leaves_the_run_files_leftovers() {
    let data = tempfile::tempdir().unwrap();
    let repo = planted(data.path());
    crate::profile::store::sweep_leftovers(&repo).unwrap();
    let mut left: Vec<&str> = PLANTED.to_vec();
    left.retain(|n| *n != "profile.toml.1.0.tmp");
    left.sort();
    assert_eq!(names(&repo), left);
}
