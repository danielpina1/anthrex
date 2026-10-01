//! Ruling C-27 (I-2, M-6): profile verification's commands take a scheduler grant at
//! `Priority::Verify` and run in a step directory of their own with decision 28's
//! variables, never the daemon's socket or data directory.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use proto::RepoProfile;

use super::tests_tiers::{steps, steps_on};
use super::verify::run_commands;
use crate::run::driver::tier_step::step_base;
use crate::run::slots::{LOAD_VAR, Priority, SLOT_VARS, SlotRequest, TestScheduler, Want};

/// A profile whose only command is `check`.
fn checking(command: &str) -> RepoProfile {
    RepoProfile {
        check: Some(command.to_string()),
        ..RepoProfile::default()
    }
}

/// `KEY=value` lines of `env`'s output.
fn env_of(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn a_verified_command_runs_in_its_own_step_directory() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("env.txt");
    let profile = checking(&format!("env > '{}'", out.display()));
    let v = run_commands(
        dir.path(),
        &profile,
        None,
        Duration::from_secs(30),
        1,
        &steps(dir.path()),
    );
    assert!(v.check.as_ref().is_some_and(|c| c.ok), "{v:?}");
    let env = env_of(&out);
    let base = step_base(&dir.path().join(".anthrex-data"), dir.path());
    let tmp = env.get("TMPDIR").expect("TMPDIR");
    assert!(
        Path::new(tmp).starts_with(&base) && Path::new(tmp) != base,
        "{tmp} is not a step directory under {}",
        base.display()
    );
    assert_eq!(env.get("ANTHREX_SOCKET"), Some(&format!("{tmp}/d.sock")));
    assert_eq!(env.get("ANTHREX_DATA_DIR"), Some(&format!("{tmp}/data")));
    // Decision 26's slot variables: half of the scheduler's 2 slots, and the load.
    for name in SLOT_VARS {
        assert_eq!(env.get(name).map(String::as_str), Some("1"), "{name}");
    }
    assert!(env.contains_key(LOAD_VAR), "{env:?}");
    // M-6: no other `ANTHREX_*` variable reaches it.
    let anthrex: Vec<&String> = env
        .keys()
        .filter(|k| k.starts_with("ANTHREX_"))
        .filter(|k| !["ANTHREX_SOCKET", "ANTHREX_DATA_DIR", LOAD_VAR].contains(&k.as_str()))
        .filter(|k| !SLOT_VARS.contains(&k.as_str()))
        .collect();
    assert!(anthrex.is_empty(), "{anthrex:?}");
    // The step directory is gone once the command has run.
    assert!(!Path::new(tmp).exists(), "{tmp}");
}

#[test]
fn a_verified_command_waits_for_a_scheduler_grant() {
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let sched = TestScheduler::new(1);
    let steps = steps_on(sched.clone(), dir.path());
    // A run's candidate holds the one slot.
    let held = steps.handle.block_on(sched.acquire(SlotRequest {
        priority: Priority::Candidate,
        critical: false,
        want: Want::All,
        exclusive: false,
        label: "candidate".into(),
    }));
    let (path, profile) = (
        dir.path().to_path_buf(),
        checking(&format!("touch '{}'", ran.display())),
    );
    let verifier = std::thread::spawn(move || {
        run_commands(&path, &profile, None, Duration::from_secs(30), 1, &steps)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while sched.waiting() == 0 {
        assert!(
            Instant::now() < deadline,
            "verification never asked for a slot"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!ran.exists(), "the command ran without a grant");
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !verifier.is_finished() {
        assert!(Instant::now() < deadline, "verification did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let v = verifier.join().unwrap();
    assert!(v.check.as_ref().is_some_and(|c| c.ok), "{v:?}");
    assert!(ran.exists());
}

/// Ruling C-28 (2): a cancelled verification waiting for a slot stops waiting, gives
/// up its place, and runs none of its commands.
#[test]
fn a_cancelled_verification_stops_waiting_for_a_slot() {
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let sched = TestScheduler::new(1);
    let steps = steps_on(sched.clone(), dir.path());
    let cancel = steps.cancel.clone();
    let held = steps.handle.block_on(sched.acquire(SlotRequest {
        priority: Priority::Candidate,
        critical: false,
        want: Want::All,
        exclusive: false,
        label: "candidate".into(),
    }));
    let profile = RepoProfile {
        setup: Some(format!("touch '{}'", ran.display())),
        check: Some(format!("touch '{}'", ran.display())),
        ..RepoProfile::default()
    };
    let path = dir.path().to_path_buf();
    let verifier = std::thread::spawn(move || {
        run_commands(&path, &profile, None, Duration::from_secs(30), 1, &steps)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while sched.waiting() == 0 {
        assert!(
            Instant::now() < deadline,
            "verification never asked for a slot"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cancel.cancel();
    // The slot is still held: only the cancel can end the wait.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !verifier.is_finished() {
        assert!(
            Instant::now() < deadline,
            "a cancelled verification still waits for a slot"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let v = verifier.join().unwrap();
    assert_eq!(sched.waiting(), 0, "its request was not withdrawn");
    assert!(!ran.exists(), "a command ran after the cancel");
    assert!(v.setup.as_ref().is_some_and(|c| !c.ok), "{v:?}");
    assert!(v.check.as_ref().is_some_and(|c| !c.ok), "{v:?}");
    drop(held);
}

/// Runs `profile` on a thread while one of the scheduler's two slots is held; whether
/// its `check` ran before the slot was let go.
fn check_ran_beside_a_held_slot(profile: RepoProfile, ran: &Path) -> bool {
    let dir = ran.parent().unwrap().to_path_buf();
    let sched = TestScheduler::new(2);
    let steps = steps_on(sched.clone(), &dir);
    let held = steps.handle.block_on(sched.acquire(SlotRequest {
        priority: Priority::Candidate,
        critical: false,
        want: Want::One,
        exclusive: false,
        label: "candidate".into(),
    }));
    let verifier = std::thread::spawn(move || {
        run_commands(&dir, &profile, None, Duration::from_secs(30), 1, &steps)
    });
    // It either runs (one slot is free for a shared `Half`) or waits for all of them.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ran.exists() && sched.waiting() == 0 {
        assert!(
            Instant::now() < deadline,
            "verification neither ran nor waited"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let ran_beside = ran.exists();
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !verifier.is_finished() {
        assert!(Instant::now() < deadline, "verification did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let v = verifier.join().unwrap();
    assert!(v.check.as_ref().is_some_and(|c| c.ok), "{v:?}");
    ran_beside
}

/// Ruling C-28 (3), decision 25: with `timing_tests` set, verification's whole `check`
/// (which runs them) waits for every slot; without them it shares.
#[test]
fn a_whole_check_with_timing_tests_is_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let check = format!("touch '{}'", ran.display());
    let timed = RepoProfile {
        check: Some(check.clone()),
        timing_tests: Some("timing_".into()),
        ..RepoProfile::default()
    };
    assert!(
        !check_ran_beside_a_held_slot(timed, &ran),
        "the whole check ran beside another test command"
    );
    std::fs::remove_file(&ran).unwrap();
    assert!(check_ran_beside_a_held_slot(checking(&check), &ran));
}
