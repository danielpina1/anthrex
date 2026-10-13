//! Milestone 9.14 task M9.14.4: `leases.json` (decision 14; C9, C25).

use std::path::PathBuf;

use super::super::*;

fn p(path: &str) -> PathBuf {
    PathBuf::from(path)
}

fn task(k: u16) -> DirId {
    DirId {
        class: PoolClass::Task,
        k,
    }
}

fn dir(id: DirId, wipe_first: bool) -> Lease {
    Lease::Dir { id, wipe_first }
}

fn state(book: &PoolBook, id: DirId) -> DirState {
    book.dir(id).expect("a directory in the book").state
}

/// Leases `owner` a task directory at `at` and returns which.
fn lease_task(book: &mut PoolBook, owner: &str, at: u64) -> DirId {
    match book.lease(PoolClass::Task, &p(owner), "r1", at) {
        Lease::Dir { id, .. } => id,
        Lease::Full => panic!("{owner}: no free task directory"),
    }
}

#[test]
fn the_leases_file_round_trips_and_a_shrunk_pool_drops_free_dirs_past_the_new_size() {
    let mut book = PoolBook::new(3, 2);
    book.lease(PoolClass::Task, &p("/wt/runs/r1/t1"), "r1", 1_760_000_000);
    lease_task(&mut book, "/wt/runs/r1/t2", 5);
    book.release(&p("/wt/runs/r1/t2"), Released::Cleaned, 6);
    book.lease(PoolClass::Task, &p("/wt/runs/r2/t3"), "r2", 7);
    book.lease(PoolClass::Engine, &p("/wt/runs/r1/integration"), "r1", 8);
    book.release(&p("/wt/runs/r1/integration"), Released::NotCleaned, 9);

    let json = serde_json::to_string(&book.to_file()).unwrap();
    let back: LeasesFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back, book.to_file());
    assert_eq!(PoolBook::from_file(&back, 3, 2), book);

    // Decision 14's shape: `run` and `leased_at` only on a leased directory.
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["version"], 1);
    let leased = &value["dirs"][0];
    assert_eq!(leased["class"], "task");
    assert_eq!(leased["k"], 1);
    assert_eq!(leased["state"], "leased");
    assert_eq!(leased["owner"], "/wt/runs/r1/t1");
    assert_eq!(leased["run"], "r1");
    assert_eq!(leased["leased_at"], 1_760_000_000u64);
    assert_eq!(leased["last_owner"], "/wt/runs/r1/t1");
    let clean = &value["dirs"][1];
    assert_eq!(clean["state"], "clean");
    assert!(clean.get("owner").is_none() && clean.get("run").is_none());
    assert!(clean.get("leased_at").is_none());
    assert_eq!(clean["last_owner"], "/wt/runs/r1/t2");
    assert_eq!(clean["last_used"], 6);
    let dirty = value["dirs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["class"] == "engine" && d["k"] == 1)
        .unwrap();
    assert_eq!(dirty["state"], "dirty");

    // One task directory now: `task-2` (free) is dropped, `task-3` (leased) is kept
    // until its release, and never handed to anyone else.
    let mut shrunk = PoolBook::from_file(&back, 1, 2);
    assert!(shrunk.dir(task(2)).is_none());
    assert_eq!(state(&shrunk, task(3)), DirState::Leased);
    assert_eq!(
        shrunk.lease(PoolClass::Task, &p("/wt/runs/r2/t3"), "r2", 10),
        dir(task(3), false)
    );
    assert_eq!(
        shrunk.lease(PoolClass::Task, &p("/new"), "r2", 10),
        Lease::Full
    );
    assert_eq!(
        shrunk.release(&p("/wt/runs/r2/t3"), Released::Cleaned, 11),
        Some(task(3))
    );
    assert!(shrunk.dir(task(3)).is_none());
    assert_eq!(
        shrunk.lease(PoolClass::Task, &p("/new"), "r2", 12),
        Lease::Full
    );
}
