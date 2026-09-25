//! Milestone 8a, task 25: decision 48's crash injection after each logged intent kind,
//! then decision 44's restore and reconciliation, end to end: whichever intent the
//! daemon died after, the green one-task run ends as it does without a crash.
//!
//! The crash-time checks carry decision 43's ordering (M8a.22's carry): `run.json` is
//! written before an op's intent line, and an op's `done` line before its result is
//! acted on. So at the moment of the crash, every op in the journal is either still
//! pending in `run.json` or has its `done` line.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use proto::{RunRequest, RunState, TaskState};
use serde_json::Value;
use support::run_harness::{RUN_WAIT, RunHarness, git_in};
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

fn wait_dead(h: &RunHarness) {
    let deadline = Instant::now() + RUN_WAIT;
    while std::os::unix::net::UnixStream::connect(h.socket()).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the daemon never reached its crash:\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(50));
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

fn crash_after(kind: &str) {
    let mut h = RunHarness::with_env(
        "",
        &[
            ("ANTHREX_TEST_ABORT_AFTER_INTENT", kind),
            ("ANTHREX_TEST_DELAY_DONE_MS", DELAY_DONE_MS),
        ],
        true,
    );
    green_scripts(&h.repo);
    let base = h.git(&["rev-parse", "HEAD"]);
    let _ = h.request_unanswered(RunRequest::Start {
        plan_toml: plan("", &[task("t1", &["a.txt"], "")]),
        dir: h.repo.clone(),
        yes: true,
        trust_project: false,
        unconfined_checks: false,
    });
    wait_dead(&h);
    h.forget_dead_daemon();
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
    h.restart_daemon(&[]);
    let id = h
        .snapshot()
        .runs
        .first()
        .unwrap_or_else(|| panic!("{kind}: the run was not restored\n{}", h.log_tail()))
        .run_id
        .clone();
    if h.run(&id).unwrap().state == RunState::Paused {
        let output = h.anthrex(&["run", "resume", &id]);
        assert!(
            output.status.success(),
            "{kind}: run resume: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // The part before the crash and the part after it are a path each (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
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

    until("the task worktrees to go", RUN_WAIT, || {
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
    until("the final run.json", Duration::from_secs(30), || {
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
    // `AX_CRASH_KIND=<kind>` runs one kind alone (for a mutant check by hand).
    let only = std::env::var("AX_CRASH_KIND").ok();
    for kind in KINDS {
        if only.as_deref().is_some_and(|o| o != kind) {
            continue;
        }
        eprintln!("crash after {kind}");
        crash_after(kind);
    }
}
