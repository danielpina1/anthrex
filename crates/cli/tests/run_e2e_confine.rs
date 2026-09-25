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
        "printf x > '{c}/objects/pwned'; printf x >> '{c}/index'; printf x > \"$HOME/pwned\"; true",
        c = common.display()
    );
    let profile = format!("\n[profile.env]\nHOME = {:?}\n", home.display().to_string());
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
