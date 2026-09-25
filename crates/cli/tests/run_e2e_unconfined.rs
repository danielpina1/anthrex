//! M8a final fix batch F1c, round 2: anthrex never runs worker-written code (a check, a
//! proof, `setup`) unconfined without being told to. Where the platform cannot confine
//! those commands, `run start` refuses unless the user passes `--unconfined-checks` or
//! sets `[orchestrator] unconfined_checks = true` in their own config; the run records
//! it, and `run status` and the report show it.
//!
//! The daemon's `ANTHREX_CHECK_CONFINEMENT=unavailable` stands in for such a platform, so
//! these run on macOS too.

mod support;

use proto::RunReply;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

const UNAVAILABLE: (&str, &str) = ("ANTHREX_CHECK_CONFINEMENT", "unavailable");

/// `anthrex run <args> --dir <repo>`: its exit code, stdout and stderr.
fn run_cli(h: &RunHarness, args: &[&str]) -> (i32, String, String) {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    let out = h.anthrex(&all);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn e2e_start_refuses_unconfinable_checks_without_the_flag() {
    // The harness allows unconfined checks on a platform without confinement (Linux
    // CI); this test says no explicitly.
    let h = RunHarness::with_env("unconfined_checks = false", &[UNAVAILABLE], true);
    green_scripts(&h.repo);
    let toml = plan("", &[task("t1", &["a.txt"], "")]);

    let reply = h.start_reply(&h.repo, &toml, true, false);
    let RunReply::Refused { message, .. } = reply else {
        panic!("started without --unconfined-checks: {reply:?}");
    };
    assert!(message.contains("--unconfined-checks"), "{message}");
    assert!(message.contains("unconfined_checks = true"), "{message}");
    assert!(h.snapshot().runs.is_empty(), "a refused start left a run");

    // The CLI refuses the same way.
    let plan_path = h.plan(&toml).display().to_string();
    let (code, _, stderr) = run_cli(&h, &["start", "--plan", &plan_path, "--yes"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("--unconfined-checks"), "{stderr}");

    // With the flag it starts, records it, and says so in status and the report.
    let (code, stdout, stderr) = run_cli(
        &h,
        &[
            "start",
            "--plan",
            &plan_path,
            "--yes",
            "--unconfined-checks",
        ],
    );
    assert_eq!(code, 0, "{stderr}{}", h.log_tail());
    let id = stdout.trim().to_string();
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert!(run.unconfined_checks, "{run:?}");
    let (code, status, _) = run_cli(&h, &["status", &id]);
    assert_eq!(code, 0);
    assert!(status.contains("checks: unconfined"), "{status}");
    report_with(&run, "checks, proofs and setup: unconfined");
}

#[test]
fn e2e_the_users_config_allows_unconfined_checks() {
    let h = RunHarness::with_env("unconfined_checks = true", &[UNAVAILABLE], true);
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert!(run.unconfined_checks, "{run:?}");
}

/// Where confinement is available, the flag never turns it off.
#[cfg(target_os = "macos")]
#[test]
fn e2e_the_flag_does_not_unconfine_where_confinement_is_available() {
    let h = RunHarness::new("unconfined_checks = true");
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert!(!run.unconfined_checks, "{run:?}");
}
