//! The pure half of the git subsystem's timing: [`Scheduler`], [`Publisher`] and the
//! watcher's path filter (M4.5.5).
//!
//! Design decision 30 binds this suite: the debounce, the 30-second poll and the
//! circuit breaker are driven by an injected clock, never by sleeping. That is
//! possible because none of the three types here reads a clock — every instant is an
//! argument — so a thirty second poll and a sixty second breaker cooldown are
//! arithmetic on one base `Instant` and cost nothing to assert.
//!
//! `crates/daemon/tests/git_registry.rs` is the other half: the tokio task, the real
//! watcher and publication end to end. The two files are split because together they
//! run past this repository's file-length limit, not because they test different
//! things.

use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::git::schedule::{BREAKER_EVENTS, DEBOUNCE, POLL_INTERVAL, Publisher, Scheduler};
use daemon::git::watch::accepts;
use proto::{GitState, Head};
use tokio::time::Instant;

fn state(dirty: u32) -> GitState {
    GitState {
        head: Head::Branch("main".to_string()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------------------------------------------------------------------------
// The pure scheduler
// ---------------------------------------------------------------------------

/// Registration's own probe, consumed so the tests that follow start from a settled
/// scheduler with its poll deadline one [`POLL_INTERVAL`] out.
fn settled(now: Instant) -> Scheduler {
    let mut scheduler = Scheduler::new(now);
    assert!(scheduler.plan(now).probe, "registration must probe at once");
    scheduler.probe_finished();
    scheduler
}

#[test]
fn an_accepted_event_probes_after_the_debounce() {
    let t0 = Instant::now();
    let mut scheduler = settled(t0);
    scheduler.record_event(t0);
    assert!(!scheduler.plan(t0 + millis(299)).probe);
    assert!(scheduler.plan(t0 + millis(300)).probe);
}

#[test]
fn events_inside_the_window_push_the_deadline_out() {
    let t0 = Instant::now();
    let mut scheduler = settled(t0);
    for at in [0, 200, 400] {
        scheduler.record_event(t0 + millis(at));
    }
    assert!(!scheduler.plan(t0 + millis(500)).probe);
    assert!(!scheduler.plan(t0 + millis(699)).probe);
    assert!(scheduler.plan(t0 + millis(700)).probe);
    scheduler.probe_finished();
    // One probe for the three events, not three.
    assert!(!scheduler.plan(t0 + millis(1500)).probe);
}

#[test]
fn the_poll_fires_every_thirty_seconds_without_events() {
    let t0 = Instant::now();
    let mut scheduler = settled(t0);
    assert!(!scheduler.plan(t0 + POLL_INTERVAL - millis(1)).probe);
    assert!(scheduler.plan(t0 + POLL_INTERVAL).probe);
    scheduler.probe_finished();
    assert!(!scheduler.plan(t0 + POLL_INTERVAL + secs(29)).probe);
    assert!(scheduler.plan(t0 + POLL_INTERVAL * 2).probe);
}

#[test]
fn only_one_probe_runs_at_a_time_per_root() {
    let t0 = Instant::now();
    let mut scheduler = Scheduler::new(t0);
    assert!(
        scheduler.plan(t0).probe,
        "registration probe is outstanding"
    );

    // Two refreshes arrive while it runs.
    scheduler.record_event(t0 + millis(10));
    assert!(!scheduler.plan(t0 + millis(400)).probe);
    scheduler.record_event(t0 + millis(500));
    assert!(!scheduler.plan(t0 + millis(900)).probe);

    // Exactly one further probe when the outstanding one returns — never a queue.
    scheduler.probe_finished();
    assert!(scheduler.plan(t0 + millis(950)).probe);
    scheduler.probe_finished();
    assert!(!scheduler.plan(t0 + secs(1)).probe);
}

#[test]
fn the_breaker_drops_a_noisy_root_to_poll_only() {
    let t0 = Instant::now();
    let mut scheduler = settled(t0);

    // 201 accepted events inside the ten second window: one more than the threshold.
    let mut tripped = 0;
    for i in 0..=BREAKER_EVENTS {
        if scheduler.record_event(t0 + millis(i as u64 * 10)) {
            tripped += 1;
        }
    }
    assert_eq!(tripped, 1, "the breaker trips once, and is logged once");
    let trip = t0 + millis(BREAKER_EVENTS as u64 * 10);

    // No debounce probe follows the storm.
    assert!(!scheduler.plan(trip + DEBOUNCE).probe);
    scheduler.record_event(trip + secs(1));
    assert!(!scheduler.plan(trip + secs(1) + DEBOUNCE).probe);

    // The poll still fires.
    assert!(scheduler.plan(t0 + POLL_INTERVAL).probe);
    scheduler.probe_finished();
    assert!(scheduler.plan(t0 + POLL_INTERVAL * 2).probe);
    scheduler.probe_finished();

    // Sixty seconds after the trip, debouncing resumes.
    let after = trip + secs(60);
    assert!(
        !scheduler.plan(after).probe,
        "no probe is due at the reopen"
    );
    scheduler.record_event(after);
    assert!(!scheduler.plan(after + millis(299)).probe);
    assert!(scheduler.plan(after + millis(300)).probe);
}

// ---------------------------------------------------------------------------
// The path filter, and what it does to the scheduler
// ---------------------------------------------------------------------------

/// The worktree root every filter case below is relative to.
const ROOT: &str = "/w";

/// Feeds one watcher path through the filter exactly as the registry's task does, and
/// reports whether a probe became due one debounce later.
fn schedules(path: &str, git_dir: Option<&Path>) -> bool {
    let t0 = Instant::now();
    let mut scheduler = settled(t0);
    if accepts(Path::new(path), Path::new(ROOT), git_dir) {
        scheduler.record_event(t0);
    }
    scheduler.plan(t0 + DEBOUNCE).probe
}

#[test]
fn filtered_paths_do_not_schedule_a_probe() {
    let git_dir = Path::new("/w/.git");
    for path in [
        "/w/target/debug/build/x/output",
        "/w/crates/daemon/node_modules/x/index.js",
        "/w/.venv/lib/python3.13/site-packages/x.py",
        "/w/dist/main.js",
        "/w/build/main.o",
        "/w/.next/server/app.js",
        "/w/.git/objects/ab/cdef",
        "/w/.git/lfs/tmp/x",
        "/w/.git/index.lock",
        "/w/src/main.rs.lock",
    ] {
        assert!(
            !accepts(Path::new(path), Path::new(ROOT), Some(git_dir)),
            "{path} must be rejected"
        );
        assert!(
            !schedules(path, Some(git_dir)),
            "{path} must schedule nothing"
        );
    }
}

#[test]
fn an_event_on_a_real_source_file_schedules_a_probe() {
    let git_dir = Path::new("/w/.git");
    for path in [
        "/w/src/main.rs",
        "/w/crates/daemon/src/git/mod.rs",
        "/w/.git/HEAD",
        "/w/docs/target-audience.md",
    ] {
        assert!(
            accepts(Path::new(path), Path::new(ROOT), Some(git_dir)),
            "{path} must be accepted"
        );
        assert!(
            schedules(path, Some(git_dir)),
            "{path} must schedule a probe"
        );
    }
}

/// The deny list is about the worktree's *own* build directories. A checkout that
/// lives under a directory called `build` or `dist` must not go silently deaf — and it
/// would go silently deaf, because nothing fails: the root just stops seeing events and
/// falls back to the 30-second poll.
#[test]
fn a_root_under_a_denied_directory_name_still_sees_its_own_events() {
    for root in [
        "/home/me/build/repo",
        "/srv/dist/repo",
        "/x/node_modules/repo",
    ] {
        let git_dir = PathBuf::from(root).join(".git");
        let source = PathBuf::from(root).join("src/main.rs");
        assert!(
            accepts(&source, Path::new(root), Some(&git_dir)),
            "{} must be accepted under {root}",
            source.display()
        );
        // The rule still applies below the root.
        let ignored = PathBuf::from(root).join("target/debug/x.o");
        assert!(
            !accepts(&ignored, Path::new(root), Some(&git_dir)),
            "{} must still be rejected",
            ignored.display()
        );
    }
}

// ---------------------------------------------------------------------------
// Publication
// ---------------------------------------------------------------------------

#[test]
fn a_transient_failure_republishes_the_last_state_as_stale() {
    let mut publisher = Publisher::default();
    let good = state(3);
    assert_eq!(
        publisher.decide(Some(good.clone())),
        Some(Some(good.clone()))
    );

    // `None` on the wire means "not a repository". A probe that could not run is not
    // that, so the last known state is re-published as stale instead.
    let stale = GitState {
        stale: true,
        ..good.clone()
    };
    assert_eq!(publisher.decide(None), Some(Some(stale.clone())));
    assert_eq!(publisher.decide(None), None, "and only once");

    // A probe that succeeds again clears it.
    assert_eq!(publisher.decide(Some(good.clone())), Some(Some(good)));
}

#[test]
fn a_root_that_never_succeeded_publishes_none_once() {
    let mut publisher = Publisher::default();
    assert_eq!(publisher.decide(None), Some(None));
    for _ in 0..5 {
        assert_eq!(publisher.decide(None), None);
    }
}
