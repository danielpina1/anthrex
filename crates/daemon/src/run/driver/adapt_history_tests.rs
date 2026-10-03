//! Milestone 9.5 decision 35 (FU-F17): `run stats` reads the repository's stored
//! profile, and a test its `slow_tests` names as a whole token is not proposed again.

use std::time::Duration;

use proto::{FlakyRecord, HISTORY_VERSION, HistoryLine, RepoProfile};

use super::{checkout_of, stats_of};
use crate::profile::store::{PROFILE_FILE, write_atomic};
use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::append_line;

const NOW: u64 = 1_800_000_000;
const T: Duration = Duration::from_secs(10);

fn flaky(run: &str, test: &str) -> HistoryLine {
    HistoryLine::Flaky(FlakyRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/flaky/1/{test}"),
        at: NOW - 60,
        run_id: run.into(),
        task_id: Some("t1".into()),
        tier: 1,
        test: test.into(),
    })
}

#[test]
fn a_quarantined_test_is_not_proposed() {
    let tmp = tempfile::Builder::new()
        .prefix("ax-quarantine")
        .tempdir_in("/tmp")
        .unwrap();
    let (repo, data) = (tmp.path().join("repo"), tmp.path().join("data"));
    std::fs::create_dir_all(&repo).unwrap();
    let git = std::ffi::OsStr::new("git");
    // The scrubbed environment of the other driver tests' `git` (`adapt_goal_tests.rs`).
    let init = std::process::Command::new(git)
        .args(["--no-optional-locks", "init", "-q"])
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .status()
        .unwrap();
    assert!(init.success());
    let (_, project) = checkout_of(git, &repo, T).unwrap();
    let repo_dir = crate::profile::repo_dir(&data, &project);
    for run in ["r1", "r2", "r3"] {
        for test in ["t_a", "t_b"] {
            append_line(&repo_dir.join(HISTORY_FILE), &flaky(run, test)).unwrap();
        }
    }
    let testing = config::Testing::default();
    let proposed = |slow_tests: Option<&str>| {
        let profile = RepoProfile {
            slow_tests: slow_tests.map(str::to_string),
            ..RepoProfile::default()
        };
        let text = toml::to_string(&profile).unwrap();
        write_atomic(&repo_dir.join(PROFILE_FILE), text.as_bytes()).unwrap();
        let stats = stats_of(git, &repo, &data, &testing, NOW, T).unwrap();
        let names = stats.flaky_proposals.iter().map(|p| p.test.clone());
        names.collect::<Vec<_>>()
    };
    assert_eq!(proposed(Some("t_a | other")), ["t_b"]);
    assert_eq!(proposed(None), ["t_a", "t_b"]);
    // A whole token only: a longer name that merely contains it is still proposed.
    assert_eq!(proposed(Some("t_ab,(t_b)")), ["t_a"]);
    assert_eq!(proposed(Some("'t_a' \"t_b\"")), Vec::<String>::new());
}
