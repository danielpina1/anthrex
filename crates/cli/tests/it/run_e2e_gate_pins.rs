//! Milestone 9.13 task M9.13.11, Goal bullet 3 (decisions 3, 5, 10-13; ruling C25): a
//! run whose task rewrites the gate's own script (`check = "sh scripts/check.sh"`)
//! halts before the rewritten script ever runs, and only the user resumes it; a task
//! whose `owns` names the script, approved with the plan, may change it. Through a real
//! daemon with `fake-agent` as both runtimes.

use crate::support;

use std::path::Path;

use proto::{RunState, TaskState};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// The base commit's gate script.
const BASE_CHECK: &str = "exit 0\n";

/// The rewritten gate script: it leaves `$MARKER` behind when it runs.
const REWRITTEN: &str = "touch \"$MARKER\"\n";

/// Decision 13's text for `scripts/check.sh`.
const GATE_CHANGED: &str = "the gate's script scripts/check.sh differs from the base commit's; the run changed what measures it";

/// A harness whose base commit has `scripts/check.sh` (`exit 0`) and `src/lib.rs`.
fn harness() -> RunHarness {
    RunHarness::with_repo(
        "",
        &[],
        true,
        &[
            ("scripts/check.sh", BASE_CHECK),
            ("src/lib.rs", "fn lib() {}\n"),
        ],
    )
}

/// A plan whose `check` is the gate script, `MARKER` (under the harness's one writable
/// cache directory, where a confined check may write) in the profile's `env`.
fn gate_plan(marker: &Path, tasks: &[String]) -> String {
    plan(
        &format!(
            "\n[profile.env]\nMARKER = {:?}\n",
            marker.display().to_string()
        ),
        tasks,
    )
    .replace("check = \"true\"", "check = \"sh scripts/check.sh\"")
}

/// A worker step that commits [`REWRITTEN`] as `scripts/check.sh`.
fn rewrite_check() -> Value {
    commit("scripts/check.sh", REWRITTEN)
}

fn blocked(reason: &str) -> Value {
    json!({"mcp_call": {"tool": "task_blocked", "args": {"kind": "question", "reason": reason}}})
}

fn text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn ok(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "status {:?}\n{}",
        output.status,
        text(output)
    );
    text(output)
}

#[test]
fn e2e_a_rewritten_gate_script_halts_user_only() {
    let h = harness();
    let marker = h.cache_dir().join("marker");
    let bounce = h.cache_dir().join("bounce.txt");
    h.script(
        "worker-t1-1",
        &[
            commit("src/a.rs", "fn a() {}\n"),
            rewrite_check(),
            done_expecting_error(),
            sh(&format!(
                "printf '%s' \"$FAKE_AGENT_RESULT\" > '{}'",
                bounce.display()
            )),
            blocked("the check script must change; the user decides"),
        ],
    );
    let id = h.start(&gate_plan(&marker, &[task("t1", &["src/**"], "")]), true);
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Blocked, RUN_WAIT);
    assert_eq!(t(&run, "t1").bounces.done, 1);
    // Decision 3's gate line.
    let bounced = std::fs::read_to_string(&bounce).unwrap_or_default();
    assert!(
        bounced.contains(
            "scripts/check.sh is run by this run's own gate (check, setup or a tier command); this task may change it only if its owns names it exactly"
        ),
        "{bounced}"
    );
    assert!(!marker.exists(), "the rewritten script ran at done");

    // The user's override lists the file and says it stays pinned (decision 5, C25).
    let reply = ok(&h.anthrex(&["run", "override", &id, "t1", "--reason", "trusted"]));
    assert!(
        reply.contains("; it changes protected scripts/check.sh"),
        "{reply}"
    );
    assert!(
        reply.contains("; scripts/check.sh stays pinned until a task's owns names it (run edit)"),
        "{reply}"
    );

    // The merge candidate's check is refused: the run halts for the user.
    let run = h.wait_run(&id, |r| r.state == RunState::Halted, RUN_WAIT);
    let reason = run.halted_reason.clone().unwrap_or_default();
    assert!(reason.contains(GATE_CHANGED), "{reason}");
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().unwrap();
    assert!(block.text.contains(GATE_CHANGED), "{block:?}");
    assert!(block.user_only, "{block:?}");
    assert_eq!(t1.merge_commit, None);
    assert!(!marker.exists(), "the rewritten script ran");

    // The way out (C25): a task whose owns names the script, added by the user's
    // `run edit`, unpins it. `t2` runs after `t1` and keeps the script.
    let edits = h.dir.path().join("grant.toml");
    std::fs::write(
        &edits,
        "[[edit]]\nop = \"add_task\"\n[edit.task]\nid = \"t2\"\ntitle = \"Keep the check script\"\nbrief = \"Keep scripts/check.sh.\"\nacceptance = [\"kept\"]\nowns = [\"scripts/check.sh\"]\ndeps = [\"t1\"]\nsize = \"S\"\ntest_mode = \"check\"\ntest_mode_reason = \"a script\"\n",
    )
    .unwrap();
    h.script(
        "worker-t2-1",
        &[
            commit("scripts/check.sh", "touch \"$MARKER\"\n# kept\n"),
            done("kept the check script"),
        ],
    );
    h.script("reviewer-t2-1", &[approve()]);
    ok(&h.anthrex(&["run", "edit", &id, "--file", &edits.display().to_string()]));
    assert!(!marker.exists(), "the rewritten script ran at the edit");
    ok(&h.anthrex(&["run", "resume", &id]));
    // The refused candidate left `t1` blocked for the user with nothing pending. A
    // `run retry` would send it back to a worker whose `task_done` the done gate still
    // bounces (its owns does not name the script); the user overrides it again, and
    // the script is no longer pinned.
    let run = h.wait_run(&id, |r| r.state == RunState::Running, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Blocked);
    let reply = ok(&h.anthrex(&["run", "override", &id, "t1", "--reason", "granted"]));
    assert!(
        reply.contains("; it changes protected scripts/check.sh"),
        "{reply}"
    );
    assert!(!reply.contains("stays pinned"), "{reply}");

    // `t1`'s path after the resume and `t2`'s (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert_eq!(t(&run, "t2").state, TaskState::Merged);
    assert!(marker.exists(), "the new script ran once granted");
    let script = h.git(&[
        "show",
        &format!("anthrex/{id}/integration:scripts/check.sh"),
    ]);
    assert_eq!(script, "touch \"$MARKER\"\n# kept");
}

#[test]
fn e2e_a_granted_gate_script_runs() {
    let h = harness();
    let marker = h.cache_dir().join("marker");
    h.script(
        "worker-t1-1",
        &[
            commit("src/a.rs", "fn a() {}\n"),
            rewrite_check(),
            done("added a and the new check"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let plan = gate_plan(&marker, &[task("t1", &["src/**", "scripts/check.sh"], "")]);
    let id = h.start(&plan, true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!((t1.state, t1.bounces.done), (TaskState::Merged, 0));
    assert!(marker.exists(), "the granted script ran");
    let script = h.git(&[
        "show",
        &format!("anthrex/{id}/integration:scripts/check.sh"),
    ]);
    assert_eq!(script, REWRITTEN.trim_end());
}
