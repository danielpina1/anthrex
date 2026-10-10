//! Milestone 9.0.6 §4.5: the Profile screen's requests end to end. Each request is the
//! one the screen sends (`app/profile_pages.rs`): an editor's `Edit` carries
//! `profile_view::value_literal` of what was typed, `yes` from the screen's `store once
//! verification passes` toggle (off), and the confirm page's `Confirm` carries the exact
//! TOML of the `Shown` proposal. Both runtime commands are the test's `fake-agent`.

mod support;

use std::time::Duration;

use proto::{ProfileReply, ProfileRequest, ProfileStatus, ProposalState, RunReply, RunRequest};
use tui::profile_view::value_literal;

use support::orch_script::wait_file;
use support::run_adapt::{ADAPT_FILES, PROFILE_LINES, PROFILE_WAIT, STORED_PROFILE};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::{approve, commit, done, plan, task};

/// The new check, typed in the editor. Sent bare, `true` would read back as a TOML
/// boolean and be refused for `check`, a string: only `value_literal`'s `"true"` stores
/// the command.
const NEW_CHECK: &str = "true";

fn harness() -> RunHarness {
    let h = RunHarness::with_repo(PROFILE_LINES, &[], true, ADAPT_FILES);
    h.stored_profile(STORED_PROFILE);
    h
}

/// One profile request sent tagged with `id`, as the screen sends it; its reply.
fn screen(h: &RunHarness, id: u64, request: ProfileRequest, wait: Duration) -> ProfileReply {
    match h.tagged(id, RunRequest::Profile(request), wait) {
        RunReply::Profile {
            reply,
            request_id: Some(got),
        } if got == id => *reply,
        other => panic!("a profile request answered {other:?}"),
    }
}

/// The editor's request for `key` set to `typed`, with the screen's toggle off.
fn edit(h: &RunHarness, key: &str, typed: &str) -> ProfileRequest {
    ProfileRequest::Edit {
        dir: h.repo.clone(),
        key: key.to_string(),
        value: Some(value_literal(key, typed).expect("a valid value")),
        yes: false,
        unconfined_checks: false,
        anyway: false,
        on_proposal: false,
    }
}

fn ready(status: &ProfileStatus) -> bool {
    status
        .proposal
        .as_ref()
        .is_some_and(|r| matches!(r.state, ProposalState::Ready | ProposalState::Failed { .. }))
}

fn shown_text(h: &RunHarness) -> String {
    let out = h.profile(&["show"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn e2e_tui_profile_edit_then_confirm() {
    let h = harness();
    let reply = screen(&h, 1, edit(&h, "check", NEW_CHECK), REQUEST_WAIT);
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    let status = h.wait_profile("the edit's verification", ready, PROFILE_WAIT);
    let record = status.proposal.expect("the edit's proposal");
    assert_eq!(record.state, ProposalState::Ready, "{record:?}");

    // The confirm page shows the proposal's text and confirms exactly it.
    let toml = match screen(
        &h,
        2,
        ProfileRequest::Show {
            dir: h.repo.clone(),
            proposed: true,
        },
        REQUEST_WAIT,
    ) {
        ProfileReply::Shown { toml, .. } => toml,
        other => panic!("show answered {other:?}"),
    };
    let line = format!("check = \"{NEW_CHECK}\"\n");
    assert!(toml.contains(&line), "{toml}");
    let reply = screen(
        &h,
        3,
        ProfileRequest::Confirm {
            dir: h.repo.clone(),
            shown: Some(toml),
        },
        REQUEST_WAIT,
    );
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert_eq!(h.profile_status().proposal, None);
    assert!(shown_text(&h).contains(&line), "{}", shown_text(&h));
}

#[test]
fn e2e_tui_profile_edit_waits_for_a_live_run() {
    let h = harness();
    let gate = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[wait_file(&gate), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let run = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(
        &run,
        |r| r.tasks.iter().any(|t| t.state == proto::TaskState::Working),
        RUN_WAIT,
    );

    // Decision 37: refused with the daemon's own text, which the screen's error row
    // shows; nothing is proposed.
    let reply = screen(&h, 1, edit(&h, "check", NEW_CHECK), REQUEST_WAIT);
    // Milestone 9.10 decision 20's text.
    assert_eq!(
        reply,
        ProfileReply::Refused {
            message: "finish or cancel the run in this repo to change its profile".into()
        }
    );
    assert_eq!(h.profile_status().proposal, None);

    std::fs::write(&gate, "").unwrap();
    h.wait_run(&run, |r| r.state == proto::RunState::Complete, RUN_WAIT);
    // A complete run is no longer live: the same request is taken.
    let reply = screen(&h, 2, edit(&h, "check", NEW_CHECK), REQUEST_WAIT);
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    let status = h.wait_profile("the edit's verification", ready, PROFILE_WAIT);
    assert_eq!(status.proposal.map(|r| r.state), Some(ProposalState::Ready));
}
