//! Milestone 9.3 task M9.3.11's shared steps: a goal run's orchestrator script that
//! plans round 1, writes its summary, reads the user's round wake, plans round 2 and
//! writes its summary; the run's green task scripts; `run iterate`; and the waits on
//! what the scripted orchestrator saw. Every agent is `fake-agent` (the run harness pins
//! every `*_BIN`); nothing reaches a real agent or `gh`.

use std::process::Output;
use std::time::Duration;

use proto::{RunInfo, RunState};
use serde_json::{Value, json};

use super::RunningCommand;
use super::orch_script::*;
use super::run_harness::{FINISH_WAIT, GIT_TIMEOUT_SECS, REQUEST_WAIT, RUN_WAIT, RunHarness};
use super::run_orch::ORCH_WAIT;
use super::run_plans::{approve, commit, done};

/// The scripted orchestrator: the first orchestrator session of the project's first
/// run. A chain's later runs keep it (decision 23: the window is adopted).
pub const ORCH: &str = "orchestrator-run-1";

/// Round 1's and round 2's summaries (decision 38).
pub const SUMMARY_1: &str = "a.txt was added.";
pub const SUMMARY_2: &str = "b.txt was added too.";

/// The user's round-2 request.
pub const REQUEST: &str = "also add b";

/// How many git calls a continued start makes at most (`docs/timing-budgets.md`,
/// "Recorded, from M9.3.6b": the goal directory's root, `goal_ready`'s preflight, the
/// build's preflight, protected files, run refs, `.codex` and settings reads, and the
/// handoff's history read).
const CONTINUE_GIT_CALLS: u64 = 24;

/// `run start --goal … --continue <run>`'s reply (`docs/timing-budgets.md`, "Recorded,
/// from M9.3.11", corrected by the final fix wave): every git call of a continued
/// start at the harness's `git_timeout_secs`, then one engine step and the request's
/// round trip (`REQUEST_WAIT`). A local continue runs no host preflight; a `pr` one
/// would add `PREFLIGHT_BOUND` (no test continues in `pr` mode). Both stay under the
/// daemon's deadline on a request's continue, `CONTINUE_START_BOUND`, so the daemon
/// answers with the run.
pub const CONTINUE_WAIT: Duration =
    Duration::from_secs(CONTINUE_GIT_CALLS * GIT_TIMEOUT_SECS).saturating_add(REQUEST_WAIT);

/// The scripted orchestrator's `start_goal` (milestone 9.3's final fix wave, B-I1;
/// `docs/timing-budgets.md`): the daemon answers the tool within its own deadline,
/// `START_GOAL_TOOL_BOUND` (90 s: `anthrex mcp`'s 100 s reply bound less 10 s), with the
/// run or a refusal that says nothing was started, then one engine step and the round
/// trip (`REQUEST_WAIT`).
pub const START_GOAL_WAIT: Duration =
    daemon::run::chain::START_GOAL_TOOL_BOUND.saturating_add(REQUEST_WAIT);

/// A run's completion with its summary, from the user's approval (`docs/timing-budgets.md`,
/// "Recorded, from M9.3.11"): the task path (`RUN_WAIT`), then the scripted
/// orchestrator's poll that sees it and its `summary` edit, one engine step each
/// (`REQUEST_WAIT`).
pub const SUMMARY_WAIT: Duration = RUN_WAIT.saturating_add(REQUEST_WAIT);

/// From a summary to the user's iterate, as the scripted orchestrator polls for it: the
/// test's snapshot that sees the summary and its `run iterate`, one request each.
pub const ITERATE_WAIT: Duration = REQUEST_WAIT.saturating_add(REQUEST_WAIT);

/// The last four characters of a run id: its short id (`Run::short`).
pub fn h4(run: &str) -> &str {
    &run[run.len() - 4..]
}

/// KG §2.4's round wake's first line, exact.
pub fn round_wake_head(run: &str, n: u32, last: u16) -> String {
    format!(
        "the user asks for round {n} of run {}: plan only the new work, in new stages after stage {last}, then submit. Their request:",
        h4(run)
    )
}

/// KG §3.3's next-goal wake's first line, exact, for a run started without `--yes`
/// (task M9.3.7 fix round 1's gate clause).
pub fn next_goal_head(run: &str, prev: &str, outcome: &str) -> String {
    format!(
        "a new goal, run {} (your previous run {} was {outcome}), whose plan stops at the plan gate for the user:",
        h4(run),
        h4(prev)
    )
}

/// `text` fenced as user input (`quote::fence`): no backtick in it, so three, and the
/// closing fence's line end.
pub fn fenced(text: &str) -> String {
    format!("```\n{text}\n```\n")
}

/// The text of each message the scripted orchestrator read, in order.
pub fn texts(h: &RunHarness) -> Vec<String> {
    h.read_messages(ORCH)
        .iter()
        .map(|m| m["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Task `id`'s worker commits `file` and calls `task_done`; its reviewer approves.
pub fn green(h: &RunHarness, id: &str, file: &str) {
    h.script(
        &format!("worker-{id}-1"),
        &[
            commit(file, &format!("{id}\n")),
            done(&format!("added {file}")),
        ],
    );
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// An S `check` task `id` owning `file`, in stage `stage`.
pub fn staged(id: &str, file: &str, stage: u16) -> Value {
    add(plan_task(id, &[file], json!({"stage": stage})))
}

/// The orchestrator's steps for two local rounds: round 1 plans `t1` (`a.txt`), waits
/// for the user's approval and the run's completion, and writes [`SUMMARY_1`]; it polls
/// until the user's iterate made the run round 2 (a read clears the notes, so the next
/// paste is the round wake), reads the round wake, plans `t2` (`b.txt`) in stage 2,
/// waits for completion again, writes [`SUMMARY_2`], and polls until the user accepted
/// the run. Its next step is a `read_message` the caller adds.
pub fn two_rounds() -> Vec<Value> {
    vec![
        prompt(),
        edit_plan(vec![staged("t1", "a.txt", 1)], json!({"submit": true})),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/run/complete", json!(true), RUN_WAIT),
        edit_plan(vec![], json!({"summary": SUMMARY_1})),
        until("/run/round", json!(2), ITERATE_WAIT),
        read(Some("the user asks for round 2 of run ")),
        edit_plan(vec![staged("t2", "b.txt", 2)], json!({"submit": true})),
        // The user's approval (one request), then the task path.
        until("/run/complete", json!(true), SUMMARY_WAIT),
        edit_plan(vec![], json!({"summary": SUMMARY_2})),
        // `run accept` waits on its op (`FINISH_WAIT`).
        until("/run/state", json!("accepted"), FINISH_WAIT),
    ]
}

impl RunHarness {
    /// `anthrex run iterate <h4> <source> --dir <repo>`, with `input` on stdin.
    pub fn iterate(&self, run: &str, source: &str, input: &str) -> Output {
        let repo = self.repo.display().to_string();
        self.anthrex_input(&["run", "iterate", h4(run), source, "--dir", &repo], input)
    }

    /// `anthrex run approve <run>`, once the run waits at its gate in round `n`.
    pub fn approve_round(&self, run: &str, n: u32) {
        let gate = |r: &RunInfo| r.state == RunState::AwaitingApproval && r.round == n;
        self.wait_run(run, gate, ORCH_WAIT);
        ok(&self.anthrex(&["run", "approve", run]));
    }

    /// Waits until the run is complete with the orchestrator's `summary` written.
    pub fn wait_summary(&self, run: &str, summary: &str, wait: Duration) -> RunInfo {
        let written = |r: &RunInfo| {
            r.state == RunState::Complete
                && r.orchestrator
                    .as_ref()
                    .is_some_and(|o| o.summary.as_deref() == Some(summary))
        };
        self.wait_run(run, written, wait)
    }

    /// `anthrex run start --goal <goal> --continue <h4> --dir <repo>`, waiting at most
    /// [`CONTINUE_WAIT`]; the new run's id.
    pub fn continue_goal(&self, goal: &str, run: &str) -> String {
        let repo = self.repo.display().to_string();
        let args = [
            "run",
            "start",
            "--goal",
            goal,
            "--continue",
            h4(run),
            "--dir",
            &repo,
        ];
        let out = RunningCommand::start(&mut self.command(&args)).finish(CONTINUE_WAIT);
        ok(&out);
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(!id.is_empty() && !id.contains('\n'), "{id:?}");
        id
    }

    /// `run accept <run> --yes`.
    pub fn accept(&self, run: &str) {
        let repo = self.repo.display().to_string();
        ok(&self.anthrex_input(&["run", "accept", run, "--yes", "--dir", &repo], ""));
    }

    /// Waits, at most `wait`, until one of `script`'s `run_status` calls answered with
    /// `value` at `pointer`: the script's poll for it has ended, so its next step runs.
    pub fn wait_saw(&self, script: &str, pointer: &str, value: Value, wait: Duration) {
        let saw = |l: &Value| {
            l["script"] == script
                && l["tool"] == "run_status"
                && serde_json::from_str::<Value>(l["result"].as_str().unwrap_or_default())
                    .is_ok_and(|r| r.pointer(pointer) == Some(&value))
        };
        self.wait_log(
            &format!("{script} to see {pointer} = {value}"),
            |log| log.iter().any(saw),
            wait,
        );
    }

    /// The two-round scenario of `run_e2e_rounds.rs` up to the accept: the goal run
    /// (round 1, `t1`), approved and complete; `run iterate` with [`REQUEST`]; round 2
    /// (`t2`) approved and complete; `run accept`. `steps` is [`two_rounds`] and what
    /// follows. The run id and its orchestrator's window.
    pub fn two_rounds_accepted(&self, steps: &[Value]) -> (String, u32) {
        green(self, "t1", "a.txt");
        green(self, "t2", "b.txt");
        self.script(ORCH, steps);
        let run = self.start_goal_id("add a", &[]);
        let window = self.orchestrator_window(&run);
        self.approve_round(&run, 1);
        self.wait_summary(&run, SUMMARY_1, SUMMARY_WAIT);
        let out = self.iterate(&run, REQUEST, "");
        ok(&out);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            format!(
                "run {} round 2 started; its orchestrator plans it",
                h4(&run)
            )
        );
        self.approve_round(&run, 2);
        self.wait_summary(&run, SUMMARY_2, SUMMARY_WAIT);
        self.accept(&run);
        (run, window)
    }
}

/// Asserts `out` succeeded.
pub fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
