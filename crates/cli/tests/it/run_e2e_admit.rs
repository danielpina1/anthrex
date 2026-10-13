//! Milestone 9.14 task M9.14.10: admission for agents' own builds end to end, through a
//! real daemon with `fake-agent` as the workers (decisions 16–23). Each worker's `bash`
//! step runs `./heavy.sh`, a filter prefix of the stored profile and so heavy
//! (decision 18); with `[testing] test_slots = 1` the two workers' runs of it must not
//! overlap. `FAKE_AGENT_TOOL_IDS=1` makes `fake-agent` give each call a `tool_use_id`
//! and a `PostToolUse` (the release, decision 22), and it honours the admission group's
//! `timeout` (`W + ADMIT_HOOK_SLACK_SECS`).

use crate::support;

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use proto::TaskState;
use serde_json::json;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// The run's `admit_wait_secs`, decision 21's W (`stall_after_secs / 2` is larger with
/// the harness's defaults).
const ADMIT_WAIT_SECS: u64 = 60;

/// Appends `start <task> <secs>`, sleeps 2 s, appends `end <task> <secs>` to
/// `$HEAVY_LOG`, outside the worktree (the task is the worktree's name). The order of
/// the lines is what the test reads; macOS `date` has no `%N`, so the stamp is seconds.
const HEAVY_SH: &str = "#!/bin/sh\nt=$(basename \"$PWD\")\n\
echo \"start $t $(date +%s)\" >> \"$HEAVY_LOG\"\nsleep 2\n\
echo \"end $t $(date +%s)\" >> \"$HEAVY_LOG\"\n";

/// Two tasks, each worker running `./heavy.sh` through `fake-agent`'s `bash` step,
/// with one test slot and two writers: the four log lines show the two runs disjoint
/// (an ordering assertion, no wall-clock bound), and both tasks are done.
///
/// Timing: one task path is `RUN_WAIT`, and the two run side by side (`max_writers =
/// 2`), so `2 * RUN_WAIT` covers both even one after the other; on top, each worker may
/// wait a full W in its hook before it runs anyway (decision 21, fail-open), `2 * 60 s`:
/// the scheduler term of `docs/timing-budgets.md` standing rule 5 (`M9.14.10`'s row).
#[test]
fn fake_agent_bash_heavy_commands_do_not_overlap() {
    let testing = format!("[testing]\ntest_slots = 1\nadmit_wait_secs = {ADMIT_WAIT_SECS}\n");
    let h = RunHarness::with_repo_and_config(
        "max_writers = 2\n",
        &[("FAKE_AGENT_TOOL_IDS", "1")],
        &[("heavy.sh", HEAVY_SH)],
        &testing,
    );
    let script = h.repo.join("heavy.sh");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    h.git(&["add", "heavy.sh"]);
    h.git(&["commit", "-q", "-m", "heavy.sh is executable"]);
    let log = h.dir.path().join("heavy.log");
    h.stored_profile(&format!(
        "check = \"true\"\noutput_filter = \"none\"\nfilter_prefixes = [\"./heavy.sh\"]\n\n[env]\nHEAVY_LOG = {:?}\n",
        log.display().to_string()
    ));
    for (id, file) in [("t1", "a.txt"), ("t2", "b.txt")] {
        h.script(
            &format!("worker-{id}-1"),
            &[
                json!({"bash": {"cmd": "./heavy.sh"}}),
                commit(file, "x\n"),
                done(&format!("added {file}")),
            ],
        );
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let plan = format!(
        "goal = \"Admit\"\n{}{}",
        task("t1", &["a.txt"], ""),
        task("t2", &["b.txt"], "")
    );
    let id = h.start(&plan, true);
    let wait = RUN_WAIT * 2 + Duration::from_secs(ADMIT_WAIT_SECS) * 2;
    let run = h.wait_run(&id, complete, wait);
    assert_eq!(t(&run, "t1").state, TaskState::Merged, "{}", report(&run));
    assert_eq!(t(&run, "t2").state, TaskState::Merged, "{}", report(&run));

    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<(&str, &str)> = (text.lines())
        .map(|line| {
            let mut words = line.split(' ');
            (words.next().unwrap(), words.next().unwrap())
        })
        .collect();
    assert_eq!(lines.len(), 4, "{text}");
    let (first, second) = (lines[0].1, lines[2].1);
    assert_ne!(first, second, "{text}");
    assert_eq!(
        lines,
        [
            ("start", first),
            ("end", first),
            ("start", second),
            ("end", second)
        ],
        "the two heavy runs overlapped:\n{text}"
    );
    // Admission ordered them, not chance: each worker's call was granted and released.
    let daemon_log = std::fs::read_to_string(h.data().join("daemon.log")).unwrap();
    for message in ["admit granted", "admit released"] {
        let n = daemon_log.lines().filter(|l| l.contains(message)).count();
        assert_eq!(n, 2, "{message}: {}", h.log_tail());
    }
}
