//! Milestone 8a, task 22 fix round 1: decision 20's accept onto a moved base, through
//! raw socket requests. Only `confirm == "<run id>@<to>"` merges; any other confirmation
//! is answered with `ConfirmNeeded` listing the new commits, and the base is untouched.

mod support;

use proto::{FinishAction, RunReply, RunRequest, RunState};
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
fn e2e_accept_onto_a_moved_base_needs_the_listed_head() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(&id, complete, RUN_WAIT);

    std::fs::write(h.repo.join("other.txt"), "other\n").unwrap();
    h.git(&["add", "other.txt"]);
    h.git(&["commit", "-qm", "base moves on"]);
    let to = h.git(&["rev-parse", "HEAD"]);

    let wrong = [
        None,
        Some(id.clone()),
        Some(format!("{id}@0000000000000000000000000000000000000000")),
        Some(format!("{id}@{}", &to[..7])),
        Some("anything".to_string()),
    ];
    for confirm in wrong {
        match accept(&h, &id, confirm.clone()) {
            RunReply::ConfirmNeeded {
                run_id, base_moved, ..
            } => {
                assert_eq!(run_id, id);
                let moved = base_moved.expect("the moved base is listed");
                assert_eq!((moved.to.as_str(), moved.total), (to.as_str(), 1));
                assert_eq!(moved.commits.len(), 1, "{:?}", moved.commits);
                assert!(moved.commits[0].ends_with("base moves on"), "{moved:?}");
            }
            other => panic!("{confirm:?}: expected ConfirmNeeded, got {other:?}"),
        }
        assert_eq!(
            h.git(&["rev-parse", "HEAD"]),
            to,
            "{confirm:?} moved the base"
        );
        assert_eq!(h.run(&id).unwrap().state, RunState::Complete);
    }

    match accept(&h, &id, Some(format!("{id}@{to}"))) {
        RunReply::Done { .. } => {}
        other => panic!("expected the accept, got {other:?}"),
    }
    let parents = h.git(&["log", "-1", "--format=%P"]);
    assert!(parents.starts_with(&to), "{parents}");
    assert_eq!(h.run(&id).unwrap().state, RunState::Accepted);
}

/// Ruling T22-minors, m5: an accept whose merge landed before the daemon died is
/// finished after the restart, and what its re-run clean-up did reaches the run's log
/// and report, not only the daemon's own log.
#[test]
fn e2e_a_replayed_accept_reports_its_clean_up() {
    let mut h = RunHarness::new("");
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(&id, complete, RUN_WAIT);

    h.restart_daemon(&[("ANTHREX_TEST_ABORT_AFTER_INTENT", "Accept")]);
    let reply = h.request_unanswered(RunRequest::Finish {
        run_id: id.clone(),
        action: FinishAction::Accept,
        confirm: Some(id.clone()),
    });
    assert!(reply.is_none(), "the daemon answered: {reply:?}");
    h.forget_dead_daemon();
    // The merge the dead daemon's accept would have made.
    let message = format!("anthrex: accept run {id}: Add a");
    h.git(&[
        "merge",
        "--no-ff",
        "--no-edit",
        "-m",
        &message,
        &format!("anthrex/{id}/integration"),
    ]);
    h.restart_daemon(&[("ANTHREX_TEST_ABORT_AFTER_INTENT", "")]);

    let run = h.wait_run(&id, |r| r.state == RunState::Accepted, RUN_WAIT);
    until("the clean-up to reach the report", RUN_WAIT, || {
        report(&run)
            .contains("clean-up after the restart: merged into the base branch")
            .then_some(())
    });
    let text = report(&h.run(&id).unwrap());
    assert!(!text.contains("branches kept because"), "{text}");
    assert!(no_run_branches(&h.repo));
}
