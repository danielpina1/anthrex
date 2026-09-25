//! M8a final fix batch F1: "nothing reaches the base branch until I accept the run",
//! through a real daemon with `fake-agent` as both runtimes. A worker that moves the
//! base branch onto its own work halts the run (D-2); accept after the user merged the
//! run by hand leaves their merge alone (D-1); a base that advanced and came back is
//! accepted (D-4); a run branch moved after `complete` is not what accept merges (D-6);
//! a failed reattach after the compare-and-swap still merges the task (D-7).

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
    // from inside a linked task worktree. Since F1c the checkout is its own repository,
    // so that reaches only the worker's own refs; a worker that runs unsandboxed
    // (fake-agent does) and writes the user's repository directly is what is left.
    let repo = h.repo.display().to_string();
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            sh(&format!(
                "git -C '{repo}' fetch -q \"$(pwd)\" HEAD && git -C '{repo}' update-ref refs/heads/main FETCH_HEAD"
            )),
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

/// Final fix batch F1c (3a): the task checkout is its own repository, so the same
/// `update-ref` of the base from inside it moves only the worker's own ref; the user's
/// base stays put and the run completes.
#[test]
fn e2e_an_update_ref_of_the_base_in_a_checkout_leaves_the_users_base() {
    let h = RunHarness::new("");
    let base = h.git(&["rev-parse", "main"]);
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
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(run.halted_reason.is_none(), "{run:?}");
    assert_eq!(h.git(&["rev-parse", "main"]), base, "the user's base moved");
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

#[test]
fn e2e_accept_after_the_user_merged_the_run_by_hand_keeps_their_merge() {
    let h = RunHarness::new("");
    std::fs::write(h.repo.join("line.txt"), "base\n").unwrap();
    h.git(&["add", "line.txt"]);
    h.git(&["commit", "-qm", "add line.txt"]);
    h.script(
        "worker-t1-1",
        &[
            commit("line.txt", "from the run\n"),
            done("changed the line"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["line.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    std::fs::write(h.repo.join("line.txt"), "from main\n").unwrap();
    h.git(&["commit", "-qam", "main changes the line"]);
    let head = h.git(&["rev-parse", "main"]);
    let reply = accept(&h, &id, Some(format!("{id}@{head}")));
    let RunReply::Refused { message, .. } = reply else {
        panic!("{reply:?}");
    };
    assert!(message.contains("yourself"), "{message}");

    // Decision 20's advice: the user merges the run branch into main themselves.
    let run_branch = format!("anthrex/{id}/integration");
    // It conflicts, as accept said; the user resolves it.
    let _ = std::process::Command::new("git")
        .args(["merge", "-q", "--no-ff", "--no-edit", &run_branch])
        .current_dir(&h.repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    std::fs::write(h.repo.join("line.txt"), "resolved\n").unwrap();
    h.git(&["add", "line.txt"]);
    h.git(&["commit", "-qm", "my resolution"]);
    let merged = h.git(&["rev-parse", "main"]);
    assert_eq!(
        h.git(&["log", "-1", "--format=%P", "main"]),
        format!("{head} {}", run.run_head)
    );

    let listed = match accept(&h, &id, None) {
        RunReply::ConfirmNeeded {
            base_moved: Some(moved),
            ..
        } => moved,
        other => panic!("{other:?}"),
    };
    assert_eq!(listed.to, merged);
    match accept(&h, &id, Some(format!("{id}@{merged}"))) {
        RunReply::Done { .. } => {}
        other => panic!("accept: {other:?}"),
    }
    h.wait_run(&id, |r| r.state == RunState::Accepted, RUN_WAIT);
    assert_eq!(
        h.git(&["rev-parse", "main"]),
        merged,
        "their merge was rewound"
    );
    assert_eq!(h.git(&["show", "main:line.txt"]), "resolved");
}

#[test]
fn e2e_accept_refuses_a_run_branch_moved_after_complete() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let base = h.git(&["rev-parse", "main"]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    // A quick fix committed in the integration worktree after the run was verified.
    let integration = integration(t(&run, "t1"));
    std::fs::write(integration.join("late.txt"), "late\n").unwrap();
    h.git(&["-C", integration.to_str().unwrap(), "add", "late.txt"]);
    h.git(&[
        "-C",
        integration.to_str().unwrap(),
        "commit",
        "-qm",
        "unverified",
    ]);

    let reply = accept(&h, &id, Some(id.clone()));
    let RunReply::Refused { message, .. } = reply else {
        panic!("{reply:?}");
    };
    assert!(
        message.starts_with(&format!(
            "refs/heads/anthrex/{id}/integration moved from {}",
            &run.run_head[..7]
        )),
        "{message}"
    );
    assert_eq!(h.git(&["rev-parse", "main"]), base, "nothing was merged");
    assert_eq!(h.run(&id).unwrap().state, RunState::Complete);
}

/// Final fix batch F1, finding D-7: the compare-and-swap put the candidate on the run
/// branch, then putting the integration worktree back on its branch failed (here: an
/// `index.lock` the check left behind). The task's work is on the run branch: it is
/// merged, and the run head follows it.
#[test]
fn e2e_a_failed_reattach_after_the_swap_still_merges_the_task() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    // Final fix batch F1c (I2): the check is confined and can no longer write the
    // integration worktree's git dir, so it waits at the candidate while this test
    // leaves the `index.lock` there, from outside.
    let go = h.dir.path().join("go");
    let check = format!(
        "case \"$PWD\" in */integration) for i in $(seq 1 1500); do [ -e '{go}' ] && exit 0; sleep 0.2; done; exit 1;; esac; true",
        go = go.display()
    );
    let toml = plan("", &[task("t1", &["a.txt"], "")])
        .replace("check = \"true\"", &format!("check = {check:?}"));
    let id = h.start(&toml, true);
    let lock = until("the candidate's check", RUN_WAIT, || {
        let run = h.run(&id)?;
        let at = integration(t(&run, "t1"));
        let at_candidate = at.is_dir()
            && git_read(&at, &["symbolic-ref", "-q", "HEAD"]).is_none()
            && git_read(&at, &["rev-parse", "HEAD^2"]).is_some();
        if !at_candidate {
            return None;
        }
        git_read(
            &at,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "index.lock",
            ],
        )
    });
    std::fs::write(&lock, "").unwrap();
    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(
        &id,
        |r| {
            let s = t(r, "t1").state;
            s == TaskState::Merged || s == TaskState::Blocked
        },
        RUN_WAIT,
    );
    assert_eq!(t(&run, "t1").state, TaskState::Merged, "{run:?}");
    let run_ref = format!("refs/heads/anthrex/{id}/integration");
    let head = h.git(&["rev-parse", &run_ref]);
    let run = h.wait_run(&id, |r| r.run_head == head, RUN_WAIT);
    assert_ne!(run.state, RunState::Halted, "{:?}", run.halted_reason);
}
