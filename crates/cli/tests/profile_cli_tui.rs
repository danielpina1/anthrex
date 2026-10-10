//! Milestone 9.0.6 §4.5 and milestone 9.10: the Profile screen's requests end to end.
//! Each request is the one the screen sends (`app/profile_pages.rs`,
//! `app/profile_keys.rs`): an editor's `Edit` carries `profile_view::value_literal` of
//! what was typed and always `yes: true` (milestone 9.10 decision 16), and the card's
//! **Use this** sends `Confirm` with the exact TOML of the proposal's `Shown`
//! (decision 29). A goal is the goal form's own request (`GoalForm::request`). Both
//! runtime commands, the onboarding scout and the decider are the test's `fake-agent`.

mod support;

use std::time::{Duration, Instant};

use proto::{
    ProfileReply, ProfileRequest, ProfileSource, ProfileStatus, ProposalState, RunReply,
    RunRequest, RunsSnapshot, SetupState, TaskState,
};
use serde_json::json;
use tui::profile_view::value_literal;

use support::orch_script::wait_file;
use support::run_adapt::{
    ADAPT_FILES, GOAL_WAIT, PROFILE_LINES, PROFILE_WAIT, STORED_PROFILE, triage_single,
};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::{approve, commit, complete, done, green_scripts, plan, t, task};

/// The new check, typed in the editor. Sent bare, `true` would read back as a TOML
/// boolean and be refused for `check`, a string: only `value_literal`'s `"true"` stores
/// the command.
const NEW_CHECK: &str = "true";

/// The queued goal, typed in the goal form.
const GOAL: &str = "add a";

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

/// The editor's request for `key` set to `typed` on the stored profile: `yes: true`,
/// as the screen always sends (milestone 9.10 decision 16).
fn edit(h: &RunHarness, key: &str, typed: &str) -> ProfileRequest {
    edit_on(h, key, typed, false)
}

/// The editor's request for `key` set to `typed`: on the review proposal while the card
/// shows (`on_proposal: true`, decision 27), else on the stored profile.
fn edit_on(h: &RunHarness, key: &str, typed: &str, on_proposal: bool) -> ProfileRequest {
    ProfileRequest::Edit {
        dir: h.repo.clone(),
        key: key.to_string(),
        value: Some(value_literal(key, typed).expect("a valid value")),
        yes: true,
        unconfined_checks: false,
        anyway: false,
        on_proposal,
    }
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

/// The `check = "true"` line of a profile's TOML.
fn new_check_line() -> String {
    format!("check = \"{NEW_CHECK}\"\n")
}

/// Decision 16: with `yes: true` a passing check is stored once verified, with nothing
/// left to confirm. A deadline loop (`PROFILE_WAIT`) until the stored profile shows the
/// new check and no proposal remains.
fn wait_stored_new_check(h: &RunHarness) {
    let line = new_check_line();
    let deadline = Instant::now() + PROFILE_WAIT;
    loop {
        let status = h.profile_status();
        if status.proposal.is_none()
            && status.source == ProfileSource::Stored
            && shown_text(h).contains(&line)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the edit was not stored within {PROFILE_WAIT:?}: {:?}\n{}",
            status.proposal,
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn e2e_tui_profile_edit_is_stored_on_a_pass() {
    let h = harness();
    let reply = screen(&h, 1, edit(&h, "check", NEW_CHECK), REQUEST_WAIT);
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    wait_stored_new_check(&h);
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
    // TODO(M9.10.6): decision 20 replaces this text with exactly `finish or cancel the
    // run in this repo to change its profile`; M9.10.6's merge updates this expectation.
    let project = h.profile_status().project;
    let reply = screen(&h, 1, edit(&h, "check", NEW_CHECK), REQUEST_WAIT);
    assert_eq!(
        reply,
        ProfileReply::Refused {
            message: format!(
                "run {run} is live in {}; edit the profile once it finishes (runs keep the profile they started with)",
                project.display()
            )
        }
    );
    assert_eq!(h.profile_status().proposal, None);

    std::fs::write(&gate, "").unwrap();
    h.wait_run(&run, |r| r.state == proto::RunState::Complete, RUN_WAIT);
    // A complete run is no longer live: the same request is taken.
    let reply = screen(&h, 2, edit(&h, "check", NEW_CHECK), REQUEST_WAIT);
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    wait_stored_new_check(&h);
}

// Milestone 9.10, task 12: a first goal sets the repository up, one review, Use this.

/// A repository with no stored profile; the scout proposes `check = "sh check.sh"`, the
/// decider triages `add a` as one task, and its worker and reviewer are green.
fn unset_harness() -> RunHarness {
    let h = RunHarness::adapt("claude", PROFILE_LINES, &[], ADAPT_FILES);
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo); // run_plans.rs:63: worker-t1-1 commits a.txt, reviewer-t1-1 approves
    h
}

/// The goal form's own request for `GOAL`, sent tagged as the form sends it; where checks
/// cannot be confined (Linux) the user turns `unconfined checks` on. Decision 35: the
/// reply is `Queued`, its message the toast's.
fn queue_from_the_form(h: &RunHarness) {
    let mut form = tui::run_goal::GoalForm::new(h.repo.clone());
    form.goal = tui::text_area::TextArea::from_text(GOAL);
    form.unconfined_checks = !cfg!(target_os = "macos");
    let request = form.request().expect("the form's request");
    match h.tagged(1, request, GOAL_WAIT) {
        RunReply::Queued {
            message,
            request_id: Some(1),
            ..
        } => assert!(
            message.starts_with("queued: setting up anthrex for "),
            "{message}"
        ),
        other => panic!(
            "the goal form's request answered {other:?}\n{}",
            h.log_tail()
        ),
    }
    assert!(h.snapshot().runs.is_empty(), "no run before Use this");
}

/// A deadline loop over `List` until the snapshot lists the review proposal (what raises
/// the review alert, decision 33), at most `PROFILE_WAIT`.
fn wait_review(h: &RunHarness) -> RunsSnapshot {
    let deadline = Instant::now() + PROFILE_WAIT;
    loop {
        let snapshot = h.snapshot();
        if !snapshot.proposals.is_empty() {
            return snapshot;
        }
        let failed = h
            .profile_status()
            .proposal
            .is_some_and(|p| matches!(p.state, ProposalState::Failed { .. }));
        assert!(!failed, "the set-up failed\n{}", h.log_tail());
        assert!(
            Instant::now() < deadline,
            "no review proposal within {PROFILE_WAIT:?}\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The card's **Use this** (decision 29): `Show { proposed: true }`'s TOML, sent back in
/// `Confirm { shown }`; its reply, which must start the one queued goal (decision 6).
fn use_this(h: &RunHarness, first_id: u64) -> String {
    let toml = match screen(
        h,
        first_id,
        ProfileRequest::Show {
            dir: h.repo.clone(),
            proposed: true,
        },
        REQUEST_WAIT,
    ) {
        ProfileReply::Shown { toml, .. } => toml,
        other => panic!("show answered {other:?}"),
    };
    let reply = screen(
        h,
        first_id + 1,
        ProfileRequest::Confirm {
            dir: h.repo.clone(),
            shown: Some(toml.clone()),
        },
        REQUEST_WAIT,
    );
    match reply {
        ProfileReply::Done { message } => {
            assert!(message.ends_with("; starting 1 queued goal"), "{message}")
        }
        other => panic!("Use this answered {other:?}\n{}", h.log_tail()),
    }
    toml
}

/// The drained goal's run, the snapshot's only one, starts and completes (`RUN_WAIT` for
/// each: the drain's start is a triage and a plan, the run one green task).
fn the_goal_completes(h: &RunHarness) {
    let deadline = Instant::now() + RUN_WAIT;
    let id = loop {
        let snapshot = h.snapshot();
        if let Some(run) = snapshot.runs.iter().find(|r| r.goal == GOAL) {
            assert_eq!(snapshot.runs.len(), 1, "{:?}", snapshot.runs);
            break run.run_id.clone();
        }
        let dropped = h.profile_status().dropped_goals;
        assert!(dropped.is_empty(), "the goal was dropped: {dropped:?}");
        assert!(
            Instant::now() < deadline,
            "no run for the queued goal within {RUN_WAIT:?}\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(h.snapshot().queued_goals.is_empty());
    assert_eq!(h.profile_status().source, ProfileSource::Stored);
}

#[test]
fn e2e_tui_queued_goal_review_and_use_this() {
    let h = unset_harness();
    queue_from_the_form(&h);
    let snapshot = wait_review(&h);
    assert_eq!(
        snapshot.queued_goals.len(),
        1,
        "{:?}",
        snapshot.queued_goals
    );
    assert_eq!(snapshot.queued_goals[0].goal, GOAL);
    assert_eq!(snapshot.queued_goals[0].setup, SetupState::NeedsReview);
    let toml = use_this(&h, 2);
    assert!(toml.contains("check = \"sh check.sh\"\n"), "{toml}");
    the_goal_completes(&h);
}

#[test]
#[ignore = "needs M9.10.6"]
fn e2e_tui_row_edit_on_a_proposal_then_use_this() {
    let h = unset_harness();
    queue_from_the_form(&h);
    wait_review(&h);
    // Decision 15: the card's `e` edits the proposal; its check verifies once and, on
    // ✓, is written into the proposal, which stays `Ready` and unstored.
    let reply = screen(&h, 2, edit_on(&h, "check", NEW_CHECK, true), REQUEST_WAIT);
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    let status = h.wait_profile(
        "the proposal's row edit checked",
        |s: &ProfileStatus| {
            s.proposal.as_ref().is_some_and(|p| {
                p.edit.is_none()
                    && p.profile.as_ref().and_then(|r| r.check.as_deref()) == Some(NEW_CHECK)
            })
        },
        PROFILE_WAIT,
    );
    assert_eq!(status.source, ProfileSource::None, "nothing stored yet");
    assert_eq!(status.proposal.map(|p| p.state), Some(ProposalState::Ready));
    let toml = use_this(&h, 3);
    assert!(toml.contains(&new_check_line()), "{toml}");
    let shown = shown_text(&h);
    assert!(shown.contains(&new_check_line()), "{shown}");
    the_goal_completes(&h);
}
