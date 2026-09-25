//! Milestone 8a, task 25: what keeps a run safe (salvage of a cancelled dirty task, the
//! run ref guard, a base that advances or is rewritten during the run), through a real
//! daemon with `fake-agent` as both runtimes.

mod support;

use std::path::Path;

use proto::{PlanEdit, RunReply, RunRequest, RunState, TaskState};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

fn hang() -> Value {
    json!({"hang": {}})
}

/// Waits with `sh` until `path` exists: a deadline loop of at most one `RUN_WAIT`
/// (1500 × 0.2 s = 300 s).
fn wait_for_file(path: &Path) -> Value {
    sh(&format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        path.display()
    ))
}

fn edit(h: &RunHarness, id: &str, edit: PlanEdit) {
    match h.request(RunRequest::Edit {
        run_id: id.to_string(),
        edits: vec![edit],
    }) {
        RunReply::Done { .. } => {}
        other => panic!("the edit: {other:?}"),
    }
}

fn ok(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "status {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn e2e_cancel_of_a_dirty_running_task_is_salvaged() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[sh("mkdir -p a && printf 'wip\\n' > a/wip.txt"), hang()],
    );
    let id = h.start(&plan("", &[task("t1", &["a/**"], "")]), true);
    let worktree = t(&h.run(&id).unwrap(), "t1").worktree.clone();
    until("a/wip.txt in the task worktree", RUN_WAIT, || {
        worktree.join("a/wip.txt").exists().then_some(())
    });

    edit(
        &h,
        &id,
        PlanEdit::CancelTask {
            task_id: "t1".into(),
        },
    );
    let salvage = format!("refs/anthrex/salvage/{id}/t1/1");
    let run = h.wait_run(
        &id,
        |r| t(r, "t1").state == TaskState::Cancelled && !worktree.exists(),
        RUN_WAIT,
    );
    until("the salvage ref", RUN_WAIT, || {
        (!h.git(&["for-each-ref", &salvage]).is_empty()).then_some(())
    });
    assert_eq!(t(&run, "t1").state, TaskState::Cancelled);
    assert!(!worktree.exists(), "the worktree is removed");
    let files = h.git(&["ls-tree", "-r", "--name-only", &salvage]);
    assert!(files.lines().any(|f| f == "a/wip.txt"), "{files}");
}

#[test]
fn e2e_moved_run_ref_halts_the_run() {
    let h = RunHarness::new("");
    let release = h.dir.path().join("release-review");
    h.script("worker-t1-1", &[commit("a.txt", "a\n"), done("added a")]);
    h.script("reviewer-t1-1", &[approve()]);
    h.script("worker-t2-1", &[commit("b.txt", "b\n"), done("added b")]);
    h.script("reviewer-t2-1", &[wait_for_file(&release), approve()]);
    let base = h.git(&["rev-parse", "HEAD"]);
    let id = h.start(
        &plan(
            "",
            &[task("t1", &["a.txt"], ""), task("t2", &["b.txt"], "")],
        ),
        true,
    );
    let run = h.wait_run(
        &id,
        |r| t(r, "t1").state == TaskState::Merged && t(r, "t2").state == TaskState::Review,
        RUN_WAIT,
    );
    let run_ref = format!("refs/heads/anthrex/{id}/integration");
    assert_ne!(run.run_head, base);
    h.git(&["update-ref", &run_ref, &base]);
    std::fs::write(&release, "").unwrap();

    let run = h.wait_run(&id, |r| r.state == RunState::Halted, RUN_WAIT);
    let reason = run.halted_reason.clone().unwrap_or_default();
    assert!(
        reason.contains(&format!("{run_ref} moved from")),
        "{reason}"
    );
    assert_eq!(t(&run, "t2").merge_commit, None);
    assert_ne!(t(&run, "t2").state, TaskState::Merged);
    assert_eq!(h.git(&["rev-parse", &run_ref]), base, "no merge happened");

    ok(&h.anthrex(&["run", "resume", &id, "--rebaseline"]));
    // The part after the halt is a path of its own (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    assert_eq!(t(&run, "t2").state, TaskState::Merged);
}

/// A harness whose `t1` worker, after its first commit, waits for `<tmp>/base-moved`.
fn held_t1(h: &RunHarness) -> std::path::PathBuf {
    let moved = h.dir.path().join("base-moved");
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            wait_for_file(&moved),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    moved
}

/// Waits until `t1` has its first commit (it is then held on its release file).
/// Final fix batch F1b: the worker commits on a detached `HEAD` (its objects in its
/// private directory until the engine imports them), so its commit shows as the
/// worktree's `HEAD` moving off the run's base, not on the task's branch.
fn t1_committed(h: &RunHarness, id: &str) {
    let run = h.run(id).unwrap();
    let worktree = t(&run, "t1").worktree.clone();
    until("t1's first commit", RUN_WAIT, || {
        let head = git_read(&worktree, &["rev-parse", "HEAD"])?;
        (head.trim() != run.base_sha).then_some(())
    });
}

fn commit_on_main(h: &RunHarness, file: &str, content: &str, message: &str) -> String {
    std::fs::write(h.repo.join(file), content).unwrap();
    h.git(&["add", file]);
    h.git(&["commit", "-qm", message]);
    h.git(&["rev-parse", "HEAD"])
}

#[test]
fn e2e_base_advanced_during_run_continues_and_accept_lists_it() {
    let h = RunHarness::new("");
    let moved = held_t1(&h);
    h.script("worker-t2-1", &[commit("b.txt", "b\n"), done("added b")]);
    h.script("reviewer-t2-1", &[approve()]);
    let watcher = h.subscribe();
    let id = h.start(
        &plan(
            "",
            &[task("t1", &["a.txt"], ""), task("t2", &["b.txt"], "")],
        ),
        true,
    );
    t1_committed(&h, &id);
    let to = commit_on_main(&h, "other.txt", "other\n", "base moves on");
    std::fs::write(&moved, "").unwrap();

    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    assert!(
        watcher.snapshots().iter().all(|s| s
            .runs
            .iter()
            .all(|r| r.run_id != id || r.state != RunState::Halted)),
        "the run halted"
    );
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert_eq!(t(&run, "t2").state, TaskState::Merged);
    assert_eq!(
        run.base_moved.as_ref().map(|m| m.to.clone()),
        Some(to.clone())
    );
    assert!(
        run.attention
            .iter()
            .any(|l| l.starts_with("base main moved")),
        "{:?}",
        run.attention
    );

    let out = h.anthrex_input(&["run", "accept", &id, "--yes"], "");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("{} Test User: base moves on", &to[..7])),
        "{stderr}"
    );
    assert!(
        stderr.contains("including these commits? [y/N]"),
        "{stderr}"
    );
    assert_eq!(h.git(&["rev-parse", "main"]), to, "not merged");

    ok(&h.anthrex_input(&["run", "accept", &id, "--yes", "--base", &to], ""));
    let parents = h.git(&["log", "-1", "--format=%P", "main"]);
    assert_eq!(parents, format!("{to} {}", run.run_head));
    assert!(h.repo.join("other.txt").exists());
    assert_eq!(h.git(&["show", "main:other.txt"]), "other");

    // A second run whose task and a commit on main change the same line.
    commit_on_main(&h, "line.txt", "base\n", "add line.txt");
    h.script(
        "worker-t3-1",
        &[
            commit("line.txt", "from the run\n"),
            done("changed the line"),
        ],
    );
    h.script("reviewer-t3-1", &[approve()]);
    let second = h.start(&plan("", &[task("t3", &["line.txt"], "")]), true);
    h.wait_run(&second, complete, RUN_WAIT);
    let head = commit_on_main(&h, "line.txt", "from main\n", "main changes the line");
    let out = h.anthrex_input(&["run", "accept", &second, "--yes", "--base", &head], "");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("accept conflicts with"), "{stderr}");
    assert_eq!(h.git(&["rev-parse", "main"]), head, "main is unchanged");
    let merge_head = h.git(&["rev-parse", "--git-path", "MERGE_HEAD"]);
    assert!(
        !h.repo.join(&merge_head).exists(),
        "the root has a MERGE_HEAD"
    );
    assert_eq!(h.run(&second).unwrap().state, RunState::Complete);
}

#[test]
fn e2e_base_rewritten_halts_the_run() {
    let h = RunHarness::new("");
    let moved = held_t1(&h);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    t1_committed(&h, &id);
    // A root commit: not a descendant of the run's base.
    let tree = h.git(&["rev-parse", "HEAD^{tree}"]);
    let rewritten = h.git(&["commit-tree", &tree, "-m", "rewritten history"]);
    h.git(&["update-ref", "refs/heads/main", &rewritten]);
    std::fs::write(&moved, "").unwrap();

    let run = h.wait_run(&id, |r| r.state == RunState::Halted, RUN_WAIT);
    let reason = run.halted_reason.clone().unwrap_or_default();
    assert!(reason.contains("refs/heads/main was rewritten"), "{reason}");
    assert_ne!(t(&run, "t1").state, TaskState::Merged);

    ok(&h.anthrex(&["run", "resume", &id, "--rebaseline"]));
    // The part after the halt is a path of its own (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
}
