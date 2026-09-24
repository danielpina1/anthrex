//! M8a final fix batch F1: "nothing reaches the base branch until I accept the run",
//! through a real daemon with `fake-agent` as both runtimes. A worker that moves the
//! base branch onto its own work halts the run (D-2); accept after the user merged the
//! run by hand leaves their merge alone (D-1); a base that advanced and came back is
//! accepted (D-4); a run branch moved after `complete` is not what accept merges (D-6).

mod support;

use proto::{FinishAction, RunReply, RunRequest, RunState, TaskState};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

fn accept(h: &RunHarness, id: &str, confirm: Option<String>) -> RunReply {
    h.request(RunRequest::Finish {
        run_id: id.to_string(),
        action: FinishAction::Accept,
        confirm,
    })
}

#[test]
fn e2e_a_worker_that_moves_the_base_halts_the_run() {
    let h = RunHarness::new("");
    let base = h.git(&["rev-parse", "main"]);
    // What a sandbox could not stop before final fix batch F1: `update-ref` of the base
    // from inside a linked task worktree (fake-agent runs unsandboxed).
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            sh("git update-ref refs/heads/main HEAD"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);

    let run = h.wait_run(&id, |r| r.state == RunState::Halted, RUN_WAIT);
    let reason = run.halted_reason.clone().unwrap_or_default();
    assert_eq!(
        reason, "refs/heads/main contains unaccepted run work (1 commit)",
        "{run:?}"
    );
    assert_ne!(t(&run, "t1").state, TaskState::Merged);
    assert!(run.base_moved.is_none(), "reported as the user's commit");
    assert_ne!(h.git(&["rev-parse", "main"]), base, "the worker moved it");
}

#[test]
fn e2e_a_base_that_advanced_and_came_back_is_accepted() {
    let h = RunHarness::new("");
    let base = h.git(&["rev-parse", "main"]);
    let moved = h.dir.path().join("base-moved");
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            sh(&format!(
                "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
                moved.display()
            )),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    until("t1's worktree", RUN_WAIT, || {
        let run = h.run(&id)?;
        t(&run, "t1").worktree.exists().then_some(())
    });
    // The user commits on main while the run works, then takes it back.
    std::fs::write(h.repo.join("other.txt"), "other\n").unwrap();
    h.git(&["add", "other.txt"]);
    h.git(&["commit", "-qm", "base moves on"]);
    std::fs::write(&moved, "").unwrap();
    let run = h.wait_run(&id, |r| r.base_moved.is_some(), RUN_WAIT);
    assert_ne!(run.state, RunState::Halted);
    h.git(&["reset", "-q", "--hard", &base]);
    h.wait_run(&id, complete, RUN_WAIT);

    // Before final fix batch F1 this was refused with "the base branch moved again" on
    // every attempt.
    match accept(&h, &id, Some(id.clone())) {
        RunReply::Done { .. } => {}
        other => panic!("accept: {other:?}"),
    }
    let run = h.wait_run(&id, |r| r.state == RunState::Accepted, RUN_WAIT);
    assert!(run.base_moved.is_none(), "{run:?}");
    let parents = h.git(&["log", "-1", "--format=%P", "main"]);
    assert_eq!(parents, format!("{base} {}", run.run_head));
}
