//! Milestone 9.1 decision 28 (task M9.1.9, `step_environment_puts_the_socket_and_data_dir_under_its_tmpdir`'s
//! half that needs the daemon's own coordinates in this process's environment): a tier
//! step run unconfined, as every step is on Linux with `--unconfined-checks`, sees
//! `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under its own `TMPDIR`, never the daemon's.
//! Alone in its own test binary because it sets variables in this process's
//! environment, which in edition 2024 races any process spawn on another libtest
//! thread (`run_exec_env.rs` is the precedent). Keep this file to this one test.

use std::time::Duration;

use daemon::run::driver::tier_step::{StepCommand, run_isolated, step_base};

#[test]
fn an_unconfined_step_never_sees_the_daemons_socket_or_data_dir() {
    // SAFETY: this binary has exactly one test, so no other thread reads or spawns while
    // the environment changes.
    unsafe {
        std::env::set_var("ANTHREX_SOCKET", "/nonexistent/daemon.sock");
        std::env::set_var("ANTHREX_DATA_DIR", "/nonexistent/daemon-data");
    }
    let dir = tempfile::tempdir().unwrap();
    let top = dir.path().canonicalize().unwrap();
    let checkout = top.join("wt/runs/r1/t1.proof");
    let common = top.join("repo/.git");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(&common).unwrap();
    let data = top.join("data/runs/r1");
    let base = step_base(&data, &checkout);
    let step = StepCommand {
        dir: checkout.clone(),
        command: "echo \"tmp=$TMPDIR\"; echo \"sock=$ANTHREX_SOCKET\"; \
                  echo \"data=$ANTHREX_DATA_DIR\""
            .to_string(),
        env: Vec::new(),
        slots: Vec::new(),
        timeout: Duration::from_secs(60),
        confine: None,
        common,
        base: base.clone(),
        name: "s9-1".to_string(),
        collect: true,
    };
    let ran = run_isolated(&step);
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran.outcome.ok, "{:?}", ran.outcome);
    let tmp = base.join("s9-1");
    let lines: Vec<&str> = ran.outcome.tail.lines().collect();
    assert_eq!(
        lines,
        [
            format!("tmp={}", tmp.display()),
            format!("sock={}", tmp.join("d.sock").display()),
            format!("data={}", tmp.join("data").display()),
        ]
    );
    assert_eq!(ran.names, Some(Vec::new()), "read, and no test failed");
}
