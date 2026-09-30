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
