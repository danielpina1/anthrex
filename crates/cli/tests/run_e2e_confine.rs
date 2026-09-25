//! M8a final fix batch F1c (re-review 4, I2), through a real daemon with `fake-agent`
//! as both runtimes: the plan's check runs code a worker wrote, at the task's done
//! check and at the integration candidate. With workers sandboxed (the default) it runs
//! confined, so its writes to the user's object store, the user's index and `$HOME` are
//! denied, and the run goes on as if they had never been tried.
//!
//! The payloads are harmless and aimed inside this test's own temporary directories.

#![cfg(target_os = "macos")]

mod support;

use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

#[test]
fn e2e_a_check_cannot_write_the_users_git_or_home() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let common = h.repo.join(".git").canonicalize().unwrap();
    let home = h.dir.path().canonicalize().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let index_before = std::fs::read(common.join("index")).unwrap();
    let check = format!(
        "printf x > '{c}/objects/pwned'; printf x >> '{c}/index'; printf x > \"$TEST_HOME/pwned\"; true",
        c = common.display()
    );
    // Final fix batch F2 (C-I4): a profile may not set `HOME`, so the test's stand-in
    // for it has its own name; the confinement denies the write either way.
    let profile = format!(
        "\n[profile.env]\nTEST_HOME = {:?}\n",
        home.display().to_string()
    );
    let plan = plan(&profile, &[task("t1", &["a.txt"], "")])
        .replace("check = \"true\"", &format!("check = {check:?}"));
    let id = h.start(&plan, true);

    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert!(run.halted_reason.is_none(), "{run:?}");
    assert!(
        !common.join("objects/pwned").exists(),
        "a check wrote the user's object store"
    );
    assert_eq!(
        std::fs::read(common.join("index")).unwrap(),
        index_before,
        "a check wrote the user's index"
    );
    assert!(!home.join("pwned").exists(), "a check wrote $HOME");
}

/// Round 2: the profile's `setup` runs worker-reachable code (the repository's own
/// script, at the integration checkout and each task checkout), so it runs confined
/// too: its writes to the user's `.git` and `$HOME` are denied and the run completes.
#[test]
fn e2e_setup_cannot_write_the_users_git_or_home() {
    let dir = support::tempdir();
    let home = dir.path().canonicalize().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let script = "printf x > \"$TEST_COMMON/objects/pwned\"; \
         printf x >> \"$TEST_COMMON/index\"; printf x > \"$TEST_HOME/pwned\"; true\n";
    let h = RunHarness::with_repo("", &[], true, &[("setup.sh", script)]);
    green_scripts(&h.repo);
    let common = h.repo.join(".git").canonicalize().unwrap();
    let index_before = std::fs::read(common.join("index")).unwrap();
    let profile = format!(
        "setup = \"sh setup.sh\"\n\n[profile.env]\nTEST_HOME = {:?}\nTEST_COMMON = {:?}\n",
        home.display().to_string(),
        common.display().to_string()
    );
    let id = h.start(&plan(&profile, &[task("t1", &["a.txt"], "")]), true);

    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert!(run.halted_reason.is_none(), "{run:?}");
    assert!(
        !common.join("objects/pwned").exists(),
        "setup wrote the user's object store"
    );
    assert_eq!(
        std::fs::read(common.join("index")).unwrap(),
        index_before,
        "setup wrote the user's index"
    );
    assert!(!home.join("pwned").exists(), "setup wrote $HOME");
}
