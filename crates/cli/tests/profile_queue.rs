//! Milestone 9.10, task 5 (decisions 6, 9, 10 and 12): a goal in a repository with no
//! stored profile waits in its queue while anthrex sets the repository up, and starts
//! when the user picks Use this (or, with `--yes`, as soon as every command checks
//! out). Through a real daemon with `fake-agent` as both runtimes, the onboarding
//! scout and the decider (`RunHarness::adapt`); no profile is stored beforehand.

mod support;

use std::process::Output;
use std::time::{Duration, Instant};

use daemon::profile::service::RESTART_REASON;
use proto::{ProfileSource, ProposalState, QueuedGoalInfo, SetupState, TaskState};
use serde_json::json;
use support::run_adapt::{ADAPT_FILES, PROFILE_LINES, PROFILE_WAIT, hang, triage_single};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness, RunWatcher};
use support::run_plans::*;

/// Decision 39's stderr line for the goal `add a`.
const QUEUED_LINE: &str =
    "queued: add a waits for the repository profile (anthrex profile); no run started yet";

const HOOKED_SETTINGS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#;

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn triage_calls(h: &RunHarness) -> usize {
    h.decider_calls()
        .iter()
        .filter(|c| c["kind"] == "triage")
        .count()
}

fn harness() -> RunHarness {
    RunHarness::adapt("claude", PROFILE_LINES, &[], ADAPT_FILES)
}

/// The goal `add a` came back queued: exit 3, nothing on stdout, decision 39's line.
fn queued(out: &Output) {
    assert_eq!(out.status.code(), Some(3), "{}", stderr(out)); // EXIT_QUEUED, decision 39
    assert_eq!(stdout(out), "");
    assert_eq!(stderr(out).trim_end(), QUEUED_LINE);
}

fn ok(out: Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        stdout(&out),
        stderr(&out)
    );
    stdout(&out)
}

fn ready(status: &proto::ProfileStatus) -> bool {
    matches!(
        status.proposal.as_ref().map(|p| &p.state),
        Some(ProposalState::Ready)
    )
}

/// The snapshot's only run, the queued goal's (`add a`), once it is listed; a deadline
/// loop over `List`.
fn wait_one_run(h: &RunHarness, wait: Duration) -> String {
    let deadline = Instant::now() + wait;
    loop {
        let snapshot = h.snapshot();
        if let Some(run) = snapshot.runs.iter().find(|r| r.goal == "add a") {
            assert_eq!(snapshot.runs.len(), 1, "{:?}", snapshot.runs);
            return run.run_id.clone();
        }
        never_started(h);
        assert!(
            Instant::now() < deadline,
            "no run for the queued goal within {wait:?}\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Fails at once, from the repository's state, when the goal can no longer start (fix
/// round 1, m4): it was dropped, or its set-up failed while it waits.
fn never_started(h: &RunHarness) {
    let status = h.profile_status();
    assert!(
        status.dropped_goals.is_empty(),
        "the goal was dropped: {:?}\n{}",
        status.dropped_goals,
        h.log_tail()
    );
    let failed = status
        .proposal
        .as_ref()
        .is_some_and(|p| matches!(p.state, ProposalState::Failed { .. }));
    assert!(
        !failed || status.queued.is_empty(),
        "the set-up failed: {:?}\n{}",
        status.proposal.map(|p| p.state),
        h.log_tail()
    );
}

/// The run for `add a` merged its task.
fn completes(h: &RunHarness, wait: Duration) {
    let run = wait_one_run(h, wait);
    let run = h.wait_run(&run, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(h.snapshot().queued_goals.is_empty());
}

#[test]
fn e2e_a_goal_without_a_profile_is_queued_then_starts_on_use_this() {
    let h = RunHarness::adapt("claude", PROFILE_LINES, &[], ADAPT_FILES);
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo); // run_plans.rs:63: worker-t1-1 commits a.txt, reviewer-t1-1 approves
    let out = h.start_goal("add a", &[]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out)); // EXIT_QUEUED, decision 39
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out).trim_end(),
        "queued: add a waits for the repository profile (anthrex profile); no run started yet"
    );
    assert_eq!(
        triage_calls(&h),
        0,
        "no triage before the profile is stored"
    );
    let status = h.wait_profile(
        "the proposal",
        |s| {
            matches!(
                s.proposal.as_ref().map(|p| &p.state),
                Some(ProposalState::Ready)
            )
        },
        PROFILE_WAIT,
    );
    assert_eq!(status.queued.len(), 1);
    assert_eq!(status.queued[0].setup, SetupState::NeedsReview);
    assert_eq!(h.snapshot().queued_goals.len(), 1);
    let used = h.profile_input(&["use", "--yes"], "");
    assert!(String::from_utf8_lossy(&used.stdout).contains("; starting 1 queued goal"));
    let run = wait_one_run(&h, RUN_WAIT); // the snapshot's only run, by its goal "add a"
    let run = h.wait_run(&run, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(h.snapshot().queued_goals.is_empty());
}

/// Decision 9: queued with `--yes`, the goal stores the proposal itself once every
/// command passed, and starts with no `use`.
#[test]
fn e2e_a_goal_with_yes_stores_when_every_command_passes() {
    let h = harness();
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    queued(&h.start_goal("add a", &["--yes"]));
    completes(&h, PROFILE_WAIT + RUN_WAIT);
    let status = h.profile_status();
    assert_eq!(status.source, ProfileSource::Stored, "{status:?}");
    assert!(status.confirmed_at.is_some());
    assert!(status.queued.is_empty(), "{:?}", status.queued);
}

/// Decision 9: a proposal whose verification dropped a command is never stored
/// without the user's review, whatever the goal's `--yes` says.
#[test]
fn e2e_a_goal_with_yes_waits_for_review_when_a_command_was_dropped() {
    let h = harness();
    h.onboarding_report(1, json!({"check": "sh check.sh", "setup": "false"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    queued(&h.start_goal("add a", &["--yes"]));
    let status = h.wait_profile("the proposal", ready, PROFILE_WAIT);
    let record = status.proposal.expect("a proposal");
    let dropped: Vec<&str> = record.dropped.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(dropped, ["setup"], "{:?}", record.dropped);
    let proposals = h.snapshot().proposals;
    assert_eq!(
        proposals.len(),
        1,
        "ready_proposals lists it: {proposals:?}"
    );
    // A negative check (`docs/timing-budgets.md`, M9.10.5): `after_ready` decides in the
    // same task that wrote `Ready`, and a store is a few small writes, so one poll
    // (200 ms) after `Ready` would show it.
    std::thread::sleep(Duration::from_millis(200));
    let snapshot = h.snapshot();
    assert!(snapshot.runs.is_empty(), "{:?}", snapshot.runs);
    assert_eq!(snapshot.queued_goals.len(), 1);
    assert_eq!(snapshot.queued_goals[0].setup, SetupState::NeedsReview);
    assert_eq!(h.profile_status().source, ProfileSource::None);
    let used = ok(h.profile_input(&["use", "--yes"], ""));
    assert!(used.contains("; starting 1 queued goal"), "{used}");
    completes(&h, RUN_WAIT);
}

/// Decision 9: with a ready proposal, `run start --goal --yes` stores it at once and
/// starts as any goal does.
#[test]
fn e2e_a_ready_proposal_and_yes_store_at_once() {
    let h = harness();
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    ok(h.profile(&["detect"]));
    h.wait_profile("the proposal", ready, PROFILE_WAIT);
    let out = h.start_goal("add a", &["--yes"]);
    let id = ok(out.clone()).trim().to_string();
    assert!(!id.is_empty() && !id.contains('\n'), "{id:?}");
    assert!(
        stderr(&out).starts_with("stored the proposed profile for "),
        "{}",
        stderr(&out)
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert_eq!(h.profile_status().source, ProfileSource::Stored);
}

/// Waits at most `wait` for a snapshot pushed to `watcher` whose first queued goal
/// satisfies `pred` (a deadline loop over what it received; its first snapshot, sent
/// before the goal was queued, lists none).
fn wait_pushed(
    h: &RunHarness,
    watcher: &RunWatcher,
    what: &str,
    pred: impl Fn(&QueuedGoalInfo) -> bool,
) {
    let deadline = Instant::now() + REQUEST_WAIT;
    loop {
        let snapshots = watcher.snapshots();
        if snapshots
            .iter()
            .any(|s| s.queued_goals.first().is_some_and(&pred))
        {
            return;
        }
        if Instant::now() >= deadline {
            let seen: Vec<_> = snapshots.iter().map(|s| &s.queued_goals).collect();
            panic!(
                "{what}: no pushed snapshot within {REQUEST_WAIT:?}; queued goals pushed: {seen:?}\n{}",
                h.log_tail()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Decisions 7 and 10: a set-up that fails keeps the queue, and the failure is pushed
/// to every client; detecting again and Use this start the goal.
#[test]
fn e2e_a_failed_set_up_keeps_the_queue_and_retry_detects_again() {
    let h = harness();
    h.onboarding_script(1, &[json!({"startup_fail": {"stderr": ["no"], "code": 1}})]);
    h.onboarding_report(2, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    let watcher = h.subscribe();
    queued(&h.start_goal("add a", &[]));
    wait_pushed(&h, &watcher, "the failed set-up", |g| {
        matches!(g.setup, SetupState::Failed { .. })
    });
    let status = h.profile_status();
    assert_eq!(status.queued.len(), 1);
    assert!(
        matches!(status.queued[0].setup, SetupState::Failed { .. }),
        "{:?}",
        status.queued[0].setup
    );
    ok(h.profile(&["detect"]));
    h.wait_profile("the second proposal", ready, PROFILE_WAIT);
    let used = ok(h.profile_input(&["use", "--yes"], ""));
    assert!(used.contains("; starting 1 queued goal"), "{used}");
    completes(&h, RUN_WAIT);
}

/// Decisions 7 and 11: a restart fails the set-up it interrupted and keeps the queue.
#[test]
fn e2e_the_queue_survives_a_daemon_restart() {
    let mut h = harness();
    h.onboarding_script(1, &[hang()]);
    h.onboarding_report(2, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    queued(&h.start_goal("add a", &[]));
    h.wait_profile(
        "the scout to run",
        |s| s.proposal.as_ref().is_some_and(|r| r.window_id.is_some()),
        PROFILE_WAIT,
    );
    wait_claimed(&h, "scout-onboarding-1", CLAIM_WAIT);
    h.restart_daemon(&[]);
    let status = h.profile_status();
    assert_eq!(
        status.proposal.as_ref().map(|p| &p.state),
        Some(&ProposalState::Failed {
            reason: RESTART_REASON.into()
        })
    );
    assert_eq!(status.queued.len(), 1, "{:?}", status.queued);
    assert_eq!(status.queued[0].goal, "add a");
    ok(h.profile(&["detect"]));
    h.wait_profile("the new proposal", ready, PROFILE_WAIT);
    let used = ok(h.profile_input(&["use", "--yes"], ""));
    assert!(used.contains("; starting 1 queued goal"), "{used}");
    completes(&h, RUN_WAIT);
}

/// M9.11.38a (C14): the restart above must wait until the first scout has claimed its
/// script, not only until its window exists. `fake-agent` takes the role's script with
/// the smallest unclaimed `n` (`fake-agent/src/roles.rs`), after its own exec and a
/// `git rev-parse`; a scout stopped before it claims leaves `scout-onboarding-1` (the
/// hang) to the scout `detect` starts after the restart, which then fails "the scout
/// ran longer than 60 s". Here the first scout's `claude` stalls 5 s before it execs
/// `fake-agent`, so it is still unclaimed whenever the window is first seen.
#[test]
fn the_restart_waits_for_the_first_scout_to_claim_its_script() {
    let stall = tempfile::tempdir().unwrap();
    let wrapper = stall.path().join("claude");
    testexec::write_executable(
        &wrapper,
        format!(
            "#!/bin/sh\ncase \"$*\" in *scout*) mkdir '{first}' 2>/dev/null && sleep {secs} ;; esac\nexec '{fake}' \"$@\"\n",
            first = stall.path().join("first").display(),
            secs = SCOUT_STALL.as_secs(),
            fake = support::fake_agent_bin().display(),
        ),
    );
    // The warm-up (`docs/timing-budgets.md`, the codex-probe ordering row): the
    // wrapper's first exec is paid here, before any daemon budget can time it.
    let warm = std::process::Command::new(&wrapper)
        .arg("--version")
        .output()
        .unwrap();
    assert!(warm.status.success(), "{warm:?}");
    let wrapper = wrapper.display().to_string();
    let mut h = RunHarness::adapt(
        "claude",
        PROFILE_LINES,
        &[("ANTHREX_CLAUDE_BIN", wrapper.as_str())],
        ADAPT_FILES,
    );
    h.onboarding_script(1, &[hang()]);
    h.onboarding_report(2, json!({"check": "sh check.sh"}));
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    queued(&h.start_goal("add a", &[]));
    h.wait_profile(
        "the scout to run",
        |s| s.proposal.as_ref().is_some_and(|r| r.window_id.is_some()),
        PROFILE_WAIT,
    );
    // The restart must come after the first scout's claim (it panics otherwise).
    wait_claimed(&h, "scout-onboarding-1", CLAIM_WAIT + SCOUT_STALL);
    h.restart_daemon(&[]);
    ok(h.profile(&["detect"]));
    wait_claimed(&h, "scout-onboarding-2", CLAIM_WAIT);
    h.wait_profile("the new proposal", ready, PROFILE_WAIT);
}

/// The injected stall of the first scout's start in the test above.
const SCOUT_STALL: Duration = Duration::from_secs(5);

/// Concurrent first execs of `fake-agent` in this file's parallel tests: ~0.44 s each,
/// serialised, eight at once finishing by 3.5 s (`docs/timing-budgets.md`, "Measured
/// primitive costs"), rounded up.
const FIRST_EXECS: Duration = Duration::from_secs(4);
/// `fake-agent`'s `GIT_TIMEOUT` for the `git rev-parse` before it claims
/// (`fake-agent/src/roles.rs`; a binary crate, so coupled by this comment).
const FAKE_AGENT_GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// Wall-clock slack, as the harness's other derived bounds use.
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(10);
/// From a scout's window being up to its script claimed: its exec, its `git
/// rev-parse`, then one `create_new`.
const CLAIM_WAIT: Duration = FIRST_EXECS
    .saturating_add(FAKE_AGENT_GIT_TIMEOUT)
    .saturating_add(WALL_CLOCK_SLACK);

/// Whether `fake-agent` has claimed the script `<name>.jsonl`.
fn claimed(h: &RunHarness, name: &str) -> bool {
    h.repo
        .join(".git/fake-agent")
        .join(format!("{name}.jsonl.claimed"))
        .exists()
}

/// Waits until `fake-agent` has claimed `<name>.jsonl` (its `.claimed` file, next to
/// the scripts `script_in` writes), for at most `within`.
fn wait_claimed(h: &RunHarness, name: &str, within: Duration) {
    let deadline = Instant::now() + within;
    while !claimed(h, name) {
        assert!(
            Instant::now() < deadline,
            "{name} was not claimed within {within:?} (a restart must follow the claim)\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Decision 12: a set-up that cannot start refuses the goal with its reason and queues
/// nothing.
#[test]
fn e2e_settings_a_scout_would_run_refuse_the_goal() {
    let mut files: Vec<(&str, &str)> = ADAPT_FILES.to_vec();
    files.push((".claude/settings.json", HOOKED_SETTINGS));
    let h = RunHarness::adapt(
        "claude",
        PROFILE_LINES,
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        &files,
    );
    let out = h.start_goal("add a", &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out).trim_end(),
        "this repository has project settings that headless sessions would run without asking: .claude/settings.json; review them, then detect again with --trust-project"
    );
    let status = h.profile_status();
    assert!(status.queued.is_empty(), "{:?}", status.queued);
    assert!(status.proposal.is_none(), "{:?}", status.proposal);
    assert!(h.snapshot().queued_goals.is_empty());
}

/// Decision 10: the queued goal and its set-up's state are pushed while it sets up.
#[test]
fn e2e_a_queued_goal_reaches_the_snapshot_while_setting_up() {
    let h = harness();
    h.onboarding_script(1, &[hang()]);
    let watcher = h.subscribe();
    queued(&h.start_goal("add a", &[]));
    wait_pushed(&h, &watcher, "the set-up reading the repo", |g| {
        g.goal == "add a" && g.setup == SetupState::Reading
    });
    // The hanging scout is stopped, and its goal dropped (decision 8).
    let rejected = ok(h.profile(&["reject"]));
    assert!(rejected.contains("; dropped 1 queued goal"), "{rejected}");
}

/// Fix round 1 (the review's unverified item): a hostile goal's queued line on stderr
/// goes through `status::printable`; no control character reaches the terminal.
#[test]
fn e2e_a_queued_goals_hostile_text_is_printable() {
    let h = harness();
    h.onboarding_script(1, &[hang()]);
    let goal = "add a\u{1b}]0;owned\u{7}\u{1b}[2J and\rmore";
    let out = h.start_goal(goal, &[]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    let line = stderr(&out);
    assert!(
        !line
            .chars()
            .any(|c| c == '\u{1b}' || c == '\u{7}' || c == '\r'),
        "{line:?}"
    );
    assert_eq!(
        line.trim_end(),
        format!(
            "queued: {} waits for the repository profile (anthrex profile); no run started yet",
            proto::safe_text::multi_line(goal)
        )
    );
    let rejected = ok(h.profile(&["reject"]));
    assert!(rejected.contains("; dropped 1 queued goal"), "{rejected}");
}
