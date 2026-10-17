//! Milestone 8a, task 25: decision 48's crash injection after each logged intent kind,
//! then decision 44's restore and reconciliation, end to end: whichever intent the
//! daemon died after, the green one-task run ends as it does without a crash.
//!
//! The crash-time checks carry decision 43's ordering (M8a.22's carry): `run.json` is
//! written before an op's intent line, and an op's `done` line before its result is
//! acted on. So at the moment of the crash, every op in the journal is either still
//! pending in `run.json` or has its `done` line.

use crate::support;

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use proto::{RunReply, RunRequest, RunState, TaskState};
use serde_json::Value;
use support::run_daemon::{DAEMON_EXIT_WAIT, DAEMON_START_WAIT};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness, git_in};
use support::run_plans::*;

const KINDS: [&str; 8] = [
    "CreateRunBranch",
    "PrepareWorktree",
    "CreateWindow",
    "VerifyDone",
    "Check",
    "PrepareReview",
    "MergeCandidate",
    "RemoveWorktree",
];

/// Every op's `done` line is held this long before it is written (the debug build's
/// `ANTHREX_TEST_DELAY_DONE_MS`), far over a `run.json` save and an intent append
/// (a few fsyncs, milliseconds): an engine that acts on an op's result before its
/// line is written (decision 43's order broken) then crashes at the next intent with
/// the line missing. It only slows a correct daemon.
const DELAY_DONE_MS: &str = "300";

/// The journal's lines: op id → (intent kind, whether a `done` line follows).
fn journal(dir: &Path) -> BTreeMap<u64, (String, bool)> {
    let text = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap_or_default();
    let mut ops: BTreeMap<u64, (String, bool)> = BTreeMap::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let op = value["op"].as_u64().expect("an op id");
        if let Some(intent) = value.get("intent") {
            let kind = match intent {
                Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
                Value::String(s) => s.clone(),
                _ => String::new(),
            };
            ops.entry(op).or_insert((kind, false));
        } else if value.get("done").is_some() {
            ops.entry(op).or_insert((String::new(), false)).1 = true;
        }
    }
    ops
}

/// The op ids `run.json` holds pending.
fn pending(dir: &Path) -> BTreeSet<u64> {
    let text = std::fs::read_to_string(dir.join("run.json")).unwrap_or_default();
    let Ok(run) = serde_json::from_str::<Value>(&text) else {
        return BTreeSet::new();
    };
    run["pending_ops"]
        .as_object()
        .map(|m| m.keys().filter_map(|k| k.parse().ok()).collect())
        .unwrap_or_default()
}

/// Decision 43 at the moment of the crash: every intent in the journal is pending in
/// `run.json` (written before the intent) or has its `done` line (written before the
/// result is acted on, and so before any `run.json` that retires it).
fn assert_ordered(kind: &str, dir: &Path) {
    let ops = journal(dir);
    let pending = pending(dir);
    let last = ops
        .iter()
        .rev()
        .find(|(_, (k, _))| k == kind)
        .map(|(op, _)| *op);
    assert!(
        last.is_some(),
        "{kind}: the journal has no {kind} intent: {ops:?}"
    );
    assert!(
        pending.contains(&last.unwrap()),
        "{kind}: the crashed op is not pending in run.json (a Persist was deferred): {pending:?}"
    );
    for (op, (k, done)) in &ops {
        assert!(
            *done || pending.contains(op),
            "{kind}: op {op} ({k}) has no done line, yet run.json no longer holds it pending"
        );
    }
}

/// The longest the whole test may wait, counted from its start. nextest ends a test at
/// 600 s (`.config/nextest.toml`) with a signal, so nothing unwinds: no `Drop` runs, and
/// the harness's daemon (in its own process group) and its sessions outlive the test,
/// as on Ubuntu CI, where the restored run of the `VerifyDone` kind never completed and
/// the test died at 600 s with nothing on stderr past "crash after VerifyDone". Every
/// wait here ends by this one, as in `run_e2e_tuning.rs`: 600 s, less the teardown's
/// worst case (`daemon stop` within `REQUEST_WAIT`, 75 s, then `DAEMON_EXIT_WAIT`, 30
/// s), less one poll's request begun just before a deadline (`REQUEST_WAIT`), less 10
/// s: 410 s, so 400 s (`docs/timing-budgets.md`).
const SCENARIO_WAIT: Duration = Duration::from_secs(400);

/// The forced failure's hold on every op's `done` line in the restarted daemon (an
/// hour, past any wait here), and how long after the restart its scenario may wait.
const HELD_MS: &str = "3600000";
const HELD_WAIT: Duration = Duration::from_secs(10);

/// How long the restarted daemon holds a `CreateWindow`'s `done` line (the debug builds'
/// `ANTHREX_TEST_DELAY_WINDOW_MS`): far over `fake-agent`'s start and scripted turn
/// (tens of milliseconds), so a fresh session's events reach the engine before its
/// round has the window, as `run_e2e_sessions.rs`'s `WINDOW_HOLD_MS`.
const LATE_WINDOW_MS: &str = "2000";

/// How the restarted daemon runs.
#[derive(Clone, Copy)]
struct Restart {
    /// The daemon's extra environment from the restart on.
    env: &'static [(&'static str, &'static str)],
    /// The scenario ends this long after the restart, if that is sooner.
    cut: Option<Duration>,
}

/// As the crashed daemon ran, without its abort.
const AS_BEFORE: Restart = Restart {
    env: &[],
    cut: None,
};

/// Every op's `done` line held an hour: the restored run cannot finish.
const HELD: Restart = Restart {
    env: &[("ANTHREX_TEST_DELAY_DONE_MS", HELD_MS)],
    cut: Some(HELD_WAIT),
};

/// Every `CreateWindow`'s `done` line held `LATE_WINDOW_MS`.
const LATE_WINDOW: Restart = Restart {
    env: &[("ANTHREX_TEST_DELAY_WINDOW_MS", LATE_WINDOW_MS)],
    cut: None,
};

thread_local! {
    /// The restarted daemon's pid, for the forced failure's leak check.
    static RESTARTED: Cell<Option<u32>> = const { Cell::new(None) };
}

/// One kind's run through the crash: what is left of the test's deadline, and what a
/// step that fails prints (the step, the run's state on disk, the daemon's log).
struct Scenario {
    kind: &'static str,
    deadline: Cell<Instant>,
    dir: PathBuf,
}

impl Scenario {
    /// What is left of `wait` before the deadline.
    fn within(&self, wait: Duration) -> Duration {
        wait.min(
            self.deadline
                .get()
                .saturating_duration_since(Instant::now()),
        )
    }

    /// Runs `step`; a panic in it fails the scenario naming `what`, with the run's state.
    fn step<T>(&self, what: &str, step: impl FnOnce() -> T) -> T {
        match catch_unwind(AssertUnwindSafe(step)) {
            Ok(value) => value,
            Err(payload) => {
                let message = (payload.downcast_ref::<String>().cloned())
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                let message: String = message.chars().take(4000).collect();
                panic!(
                    "{}: step \"{what}\" failed ({:?} left of the scenario): {message}\n{}",
                    self.kind,
                    self.within(SCENARIO_WAIT),
                    self.state()
                )
            }
        }
    }

    /// Polls `f` for at most `wait` (cut to the deadline); the step `what`.
    fn until<T>(&self, what: &str, wait: Duration, f: impl FnMut() -> Option<T>) -> T {
        let wait = self.within(wait);
        self.step(what, || until(what, wait, f))
    }

    /// The run's state on disk (no request: the daemon may be the stuck part) and the
    /// tails of the daemon's log and output.
    fn state(&self) -> String {
        let data = self.dir.join("data");
        let mut out = String::new();
        let runs = std::fs::read_dir(data.join("runs")).into_iter().flatten();
        for run in runs.filter_map(Result::ok).map(|e| e.path()) {
            out += &format!("--- {}:\n{}", run.display(), run_state(&run));
        }
        out += &format!("--- daemon.log:\n{}\n", tail(&data.join("daemon.log"), 80));
        out += &format!(
            "--- daemon.out:\n{}",
            tail(&self.dir.join("daemon.out"), 20)
        );
        out
    }
}

/// `run.json`'s state, tasks (their claims and rounds) and pending ops, and the
/// journal's last lines.
fn run_state(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("run.json")).unwrap_or_default();
    let run: Value = serde_json::from_str(&text).unwrap_or_default();
    let mut out = format!(
        "state {} (paused from {}), revision {}\n",
        run["state"], run["paused_from"], run["revision"]
    );
    for task in run["tasks"].as_array().into_iter().flatten() {
        out += &format!(
            "task {} {}: claim {}\n",
            task["spec"]["id"], task["state"], task["claim"]
        );
        for round in task["rounds"].as_array().into_iter().flatten() {
            let keys = [
                "role",
                "session",
                "launch_op",
                "window_id",
                "pid",
                "ended",
                "turn_open",
                "turns",
                "last_event",
                "stall",
                "fallback",
                "fallback_waiting",
                "count_op",
                "resume_op",
                "relaunch",
                "retiring",
                "delivery_failures",
            ];
            let fields: Vec<String> = (keys.iter())
                .filter_map(|k| round.get(*k).map(|v| format!("{k}={v}")))
                .collect();
            let fields: String = fields.join(" ").chars().take(600).collect();
            out += &format!("  round {fields}\n");
        }
    }
    for message in run["outbox"].as_array().into_iter().flatten() {
        let message: String = message.to_string().chars().take(300).collect();
        out += &format!("outbox {message}\n");
    }
    for (op, pending) in run["pending_ops"].as_object().into_iter().flatten() {
        let kind = match &pending["kind"] {
            Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
            other => other.to_string(),
        };
        out += &format!("pending op {op} {kind} (task {})\n", pending["task_id"]);
    }
    let journal = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap_or_default();
    let lines: Vec<&str> = journal.lines().collect();
    for line in &lines[lines.len().saturating_sub(16)..] {
        let line: String = line.chars().take(200).collect();
        out += &format!("journal {line}\n");
    }
    out
}

/// The last `n` lines of the file at `path`; empty when it cannot be read.
fn tail(path: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Waits at most `wait` for the daemon's socket to refuse connections (its abort).
fn wait_dead(h: &RunHarness, wait: Duration) {
    let deadline = Instant::now() + wait;
    while std::os::unix::net::UnixStream::connect(h.socket()).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the daemon never reached its crash within {wait:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `List`'s runs within `wait`.
fn listed(h: &RunHarness, wait: Duration) -> Vec<proto::RunInfo> {
    match h.request_within(RunRequest::List, wait) {
        RunReply::Snapshot(snapshot) => snapshot.runs,
        other => panic!("List answered {other:?}"),
    }
}

/// The tree the green run merges: the base's `README` and `a.txt` reading `a`.
fn green_tree(h: &RunHarness, base: &str) -> BTreeSet<String> {
    let mut files: BTreeSet<String> = h
        .git(&["ls-tree", "-r", base])
        .lines()
        .map(str::to_string)
        .collect();
    let a = hash_blob(h, "a\n");
    files.insert(format!("100644 blob {a}\ta.txt"));
    files
}

/// The blob id of `content`, written to a scratch file in the harness's temp dir.
fn hash_blob(h: &RunHarness, content: &str) -> String {
    let file = h.dir.path().join("blob");
    std::fs::write(&file, content).unwrap();
    git_read(&h.repo, &["hash-object", file.to_str().unwrap()]).expect("git hash-object")
}

/// Kind `kind`'s crash and its restore, every wait ending by `deadline`; the restarted
/// daemon runs as `restart` says.
fn crash_after(kind: &'static str, deadline: Instant, restart: Restart) {
    let mut h = RunHarness::with_env(
        "",
        &[
            ("ANTHREX_TEST_ABORT_AFTER_INTENT", kind),
            ("ANTHREX_TEST_DELAY_DONE_MS", DELAY_DONE_MS),
        ],
        true,
    );
    let s = Scenario {
        kind,
        deadline: Cell::new(deadline),
        dir: h.dir.path().to_path_buf(),
    };
    green_scripts(&h.repo);
    let base = h.git(&["rev-parse", "HEAD"]);
    s.step("the run's start, whose daemon may die first", || {
        h.request_unanswered_within(
            RunRequest::Start {
                plan_toml: plan("", &[task("t1", &["a.txt"], "")]),
                dir: h.repo.clone(),
                yes: true,
                trust_project: false,
                unconfined_checks: false,
                delivery: None,
                design: None,
            },
            s.within(REQUEST_WAIT),
        )
    });
    s.step(&format!("the daemon's crash after {kind}"), || {
        wait_dead(&h, s.within(RUN_WAIT))
    });
    s.step("the dead daemon forgotten", || {
        h.forget_dead_daemon_within(s.within(REQUEST_WAIT + DAEMON_EXIT_WAIT))
    });
    let runs: Vec<_> = std::fs::read_dir(h.data().join("runs"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    assert_eq!(runs.len(), 1, "{kind}: {runs:?}");
    assert_ordered(kind, &runs[0]);

    // The ordering checks above mean something only with the hold in force.
    let log = std::fs::read_to_string(h.data().join("daemon.log")).unwrap_or_default();
    assert!(
        log.contains("ANTHREX_TEST_DELAY_DONE_MS: holding op done lines"),
        "{kind}: the done-line hold was not armed:\n{log}"
    );

    h.unset_env("ANTHREX_TEST_ABORT_AFTER_INTENT");
    s.step("the daemon's restart", || {
        h.restart_daemon_within(restart.env, s.within(DAEMON_START_WAIT))
    });
    RESTARTED.set(h.daemon_pid());
    for (key, ms) in restart.env {
        let line = format!("{key}: holding op done lines ms={ms}");
        s.step("the restart's hold armed", || {
            h.wait_daemon_log(kind, &line)
        });
    }
    if let Some(cut) = restart.cut {
        s.deadline.set(s.deadline.get().min(Instant::now() + cut));
    }
    let id = s.step("the restored run listed", || {
        let runs = listed(&h, s.within(REQUEST_WAIT));
        runs.first()
            .expect("the run was not restored")
            .run_id
            .clone()
    });
    let paused = s.step("the restored run's state", || {
        let runs = listed(&h, s.within(REQUEST_WAIT));
        let run = runs
            .iter()
            .find(|r| r.run_id == id)
            .expect("the run listed");
        run.state == RunState::Paused
    });
    if paused {
        s.step("the restored run resumed", || {
            let output = h.anthrex_within(&["run", "resume", &id], s.within(REQUEST_WAIT));
            assert!(
                output.status.success(),
                "run resume: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        });
    }
    // The part before the crash and the part after it are a path each (k = 2).
    let run = s.step("the restored run complete", || {
        let wait = s.within(2 * RUN_WAIT);
        h.wait_run_for("the restored run complete", &id, complete, wait)
    });
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged, "{kind}");

    let branch = format!("anthrex/{id}/integration");
    let merges = h.git(&["log", "--merges", "--format=%H", &branch]);
    let merges: Vec<&str> = merges.lines().collect();
    assert_eq!(merges.len(), 1, "{kind}: {merges:?}");
    let parents = h.git(&["log", "-1", "--format=%P", merges[0]]);
    assert!(parents.starts_with(&base), "{kind}: {parents}");
    let tree: BTreeSet<String> = h
        .git(&["ls-tree", "-r", merges[0]])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(tree, green_tree(&h, &base), "{kind}");

    s.until("the task worktrees to go", RUN_WAIT, || {
        (!t1.worktree.exists()
            && !t1.worktree.with_file_name("t1.review").exists()
            && !t1.worktree.with_file_name("t1.proof").exists())
        .then_some(())
    });
    let integration = integration(t1);
    assert!(integration.exists(), "{kind}");
    assert_eq!(
        git_in(&integration, &["symbolic-ref", "-q", "HEAD"]),
        format!("refs/heads/{branch}"),
        "{kind}"
    );
    let branches = h.git(&[
        "for-each-ref",
        "--format=%(refname)",
        &format!("refs/heads/anthrex/{id}/"),
    ]);
    for name in branches.lines() {
        assert!(
            name == format!("refs/heads/{branch}") || name == format!("refs/heads/anthrex/{id}/t1"),
            "{kind}: an extra branch {name}: {branches}"
        );
    }

    let dir = run.report_path.parent().unwrap().to_path_buf();
    s.until("the final run.json", Duration::from_secs(30), || {
        let pending = pending(&dir);
        (pending.is_empty()).then_some(())
    });
    let pending = pending(&dir);
    for (op, (k, done)) in journal(&dir) {
        assert!(
            done || pending.contains(&op),
            "{kind}: op {op} ({k}) completed without a done line"
        );
    }
}

#[test]
fn e2e_crash_after_each_intent_kind_reconciles() {
    let deadline = Instant::now() + SCENARIO_WAIT;
    // `AX_CRASH_KIND=<kind>` runs one kind alone (for a mutant check by hand).
    let only = std::env::var("AX_CRASH_KIND").ok();
    for kind in KINDS {
        if only.as_deref().is_some_and(|o| o != kind) {
            continue;
        }
        eprintln!("crash after {kind}");
        crash_after(kind, deadline, AS_BEFORE);
    }
}

/// The scenario's own failure, forced: a restored run that cannot finish (the restarted
/// daemon holds every op's `done` line) fails a step after the restart by the
/// scenario's deadline, naming the step, with the run's state and the restarted
/// daemon's log; and the unwinding stops that daemon. Before the deadline, the test
/// waited `2 * RUN_WAIT` there, past nextest's 600 s, which kills it silently and
/// leaks the daemon (`docs/timing-budgets.md`).
#[test]
fn e2e_a_restored_run_that_cannot_finish_fails_by_the_deadline_with_its_state() {
    let failed = catch_unwind(|| crash_after("VerifyDone", Instant::now() + SCENARIO_WAIT, HELD));
    let payload = failed.expect_err("a run whose done lines are all held completed");
    let message = (payload.downcast_ref::<String>().cloned()).unwrap_or_default();
    assert!(
        message.starts_with("VerifyDone: step \"the restored run"),
        "{message}"
    );
    // The restarted daemon's log, with the hold that keeps the run from finishing.
    assert!(message.contains("--- daemon.log:"), "{message}");
    assert!(
        message.contains(&format!("holding op done lines ms={HELD_MS}")),
        "{message}"
    );
    // `Drop` stopped and reaped it: its pid names no process.
    let pid = RESTARTED.get().expect("the daemon restarted") as libc::pid_t;
    // SAFETY: signal 0 only checks that the pid exists; nothing is sent.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!alive, "the restarted daemon {pid} outlived the test");
}

/// The Ubuntu CI hang (PR #75), forced: after a crash after `VerifyDone`, the restored
/// worker's resume fails (its window died with the old daemon) and a fresh session
/// starts, whose first turn ends before its `CreateWindow` result is stepped. The new
/// daemon handed that session the window id the restored worker round still named, so
/// its turn's end went to that ended round and was dropped instead of waiting for its
/// own: the fresh round's turn stayed open, and nothing counted its commits before the
/// stall clock (600 s by default), past nextest's limit.
#[test]
fn e2e_a_fresh_session_after_a_crash_is_heard_before_its_window_result() {
    crash_after("VerifyDone", Instant::now() + SCENARIO_WAIT, LATE_WINDOW);
}
