//! Milestone 9.16: the brakes end to end, through a real daemon (its socket and data
//! directory under the harness's `/tmp` directory) and `fake-agent` as both runtimes.
//! M9.16.8: the hold persists across a restart, and a held run finishes what it
//! started. M9.16.10: the restart test drives `anthrex hold on|off|status` and reads
//! `anthrex run status`; the other sends a raw `RunRequest::Hold`.

use crate::support;

use proto::{AgentRole, RunReply, RunRequest, RunState, TaskState};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

const HOLD_TEXT: &str = "dispatch is held daemon-wide · anthrex hold off";

fn hold(h: &RunHarness, on: bool) -> Option<proto::DispatchHold> {
    match h.request(RunRequest::Hold { on }) {
        RunReply::Held { hold, .. } => hold,
        other => panic!("not held: {other:?}\n{}", h.log_tail()),
    }
}

/// `anthrex <args>`'s stdout; it must succeed.
fn cli(h: &RunHarness, args: &[&str]) -> String {
    let out = h.anthrex(args);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "anthrex {args:?}: {stdout}{}\n{}",
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    stdout
}

/// Decisions 16 and 34: `anthrex hold on`, a restart, the hold is still on with its
/// first `since` (`hold status` and `run status` say so); `hold off`, a restart, it is
/// off and `run status` prints no brake line.
#[test]
fn hold_persists_across_daemon_restart() {
    let mut h = RunHarness::new("");
    let on = cli(&h, &["hold", "on"]);
    assert_eq!(
        on.trim_end(),
        "dispatch held daemon-wide; running sessions continue · anthrex hold off releases it"
    );
    let first = h.snapshot().brakes.hold.expect("held");
    assert_eq!(first.by, "user");
    h.restart_daemon(&[]);
    assert_eq!(h.snapshot().brakes.hold, Some(first.clone()));
    let status = cli(&h, &["hold", "status"]);
    let since = format!(
        "hold: on since {} UTC (user)",
        common::text::hh_mm(first.since)
    );
    assert_eq!(status.lines().next(), Some(since.as_str()), "{status}");
    let runs = cli(&h, &["run", "status"]);
    assert!(runs.starts_with(&format!("{since}\n")), "{runs}");

    assert_eq!(cli(&h, &["hold", "off"]).trim_end(), "dispatch released");
    assert_eq!(cli(&h, &["hold", "off"]).trim_end(), "dispatch is not held");
    h.restart_daemon(&[]);
    assert_eq!(h.snapshot().brakes.hold, None);
    let status = cli(&h, &["hold", "status"]);
    assert_eq!(status.lines().next(), Some("hold: off"), "{status}");
    assert!(!cli(&h, &["run", "status"]).contains("hold:"));
}

/// U12 and decision 18: the hold stops new sessions only. A task whose worker turn is
/// open when the hold comes on finishes that turn, is reviewed and merged, and the run
/// completes; it never leaves `Running` for a paused state on the way.
#[test]
fn hold_keeps_run_running_and_open_turns_alive() {
    let h = RunHarness::new("");
    let go = h.dir.path().join("go");
    let wait = format!("while [ ! -f '{}' ]; do sleep 0.05; done", go.display());
    h.script(
        "worker-t1-1",
        &[sh(&wait), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(
        &id,
        |r| window_of(r, "t1", AgentRole::Worker).is_some(),
        RUN_WAIT,
    );

    assert!(hold(&h, true).is_some());
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::Running);
    assert!(
        run.attention.iter().any(|l| l == HOLD_TEXT),
        "{:?}",
        run.attention
    );

    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(h.snapshot().brakes.hold.is_some(), "the hold is still on");
}
