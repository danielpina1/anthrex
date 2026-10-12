//! Milestone 9.14 task M9.14.4: the pool book (decisions 1–7, 13 and 14, the pure
//! parts; C9, C10, C20, C25). Paths are plain `PathBuf`s and `now` a `u64`.

use std::path::{Path, PathBuf};

use proto::{ProfileSource, ProfileSpec};

use super::*;
use crate::run::plan::resolve_profile;

fn p(path: &str) -> PathBuf {
    PathBuf::from(path)
}

fn task(k: u16) -> DirId {
    DirId {
        class: PoolClass::Task,
        k,
    }
}

fn engine(k: u16) -> DirId {
    DirId {
        class: PoolClass::Engine,
        k,
    }
}

fn dir(id: DirId, wipe_first: bool) -> Lease {
    Lease::Dir { id, wipe_first }
}

/// Leases `owner` a task directory at `at` and returns which.
fn lease_task(book: &mut PoolBook, owner: &str, at: u64) -> DirId {
    match book.lease(PoolClass::Task, &p(owner), "r1", at) {
        Lease::Dir { id, .. } => id,
        Lease::Full => panic!("{owner}: no free task directory"),
    }
}

fn state(book: &PoolBook, id: DirId) -> DirState {
    book.dir(id).expect("a directory in the book").state
}

#[test]
fn a_lease_returns_the_dir_the_checkout_holds() {
    let mut book = PoolBook::new(3, 2);
    let first = book.lease(PoolClass::Task, &p("/wt/runs/r1/t1"), "r1", 1);
    assert_eq!(first, dir(task(1), false));
    let again = book.lease(PoolClass::Task, &p("/wt/runs/r1/t1"), "r1", 2);
    assert_eq!(again, first);
    let held: Vec<DirId> = [task(1), task(2), task(3)]
        .into_iter()
        .filter(|id| state(&book, *id) == DirState::Leased)
        .collect();
    assert_eq!(held, [task(1)]);
}

#[test]
fn a_lease_returns_the_dir_the_checkout_built_last() {
    let mut book = PoolBook::new(3, 2);
    assert_eq!(lease_task(&mut book, "/a", 1), task(1));
    assert_eq!(book.release(&p("/a"), Released::Cleaned, 2), Some(task(1)));
    // `task-2` and `task-3` were never used, so either beats `task-1` on recency.
    assert_eq!(lease_task(&mut book, "/b", 3), task(2));
    // A comes back to the directory it built last, not to the least recently used
    // (never-used `task-3`).
    assert_eq!(
        book.lease(PoolClass::Task, &p("/a"), "r1", 4),
        dir(task(1), false)
    );
}

#[test]
fn a_free_dir_goes_to_the_least_recently_used() {
    let used_at = |dirs: u16| {
        let mut book = PoolBook::new(dirs, 1);
        for (owner, k) in [("/o1", 1), ("/o2", 2), ("/o3", 3)] {
            assert_eq!(lease_task(&mut book, owner, 0), task(k));
        }
        book.release(&p("/o1"), Released::Cleaned, 10);
        book.release(&p("/o2"), Released::Cleaned, 30);
        book.release(&p("/o3"), Released::Cleaned, 20);
        book
    };
    let mut book = used_at(3);
    assert_eq!(lease_task(&mut book, "/new", 40), task(1));

    // A never-used directory beats all three.
    let mut book = used_at(4);
    assert_eq!(lease_task(&mut book, "/new", 40), task(4));

    // Ties go to the lowest `k`: `task-2` and `task-3` both used at 10.
    let mut book = PoolBook::new(3, 1);
    for owner in ["/o1", "/o2", "/o3"] {
        lease_task(&mut book, owner, 0);
    }
    book.release(&p("/o1"), Released::Cleaned, 30);
    book.release(&p("/o3"), Released::Cleaned, 10);
    book.release(&p("/o2"), Released::Cleaned, 10);
    assert_eq!(lease_task(&mut book, "/new", 40), task(2));

    // And among never-used ones, too.
    let mut book = PoolBook::new(3, 1);
    assert_eq!(lease_task(&mut book, "/new", 40), task(1));
}

#[test]
fn task_and_engine_dirs_never_cross() {
    let mut book = PoolBook::new(1, 2);
    assert_eq!(lease_task(&mut book, "/t1", 1), task(1));
    assert_eq!(book.lease(PoolClass::Task, &p("/t2"), "r1", 2), Lease::Full);
    assert_eq!(state(&book, engine(1)), DirState::Fresh);
    assert_eq!(state(&book, engine(2)), DirState::Fresh);

    let mut book = PoolBook::new(2, 1);
    assert_eq!(
        book.lease(PoolClass::Engine, &p("/integration"), "r1", 1),
        dir(engine(1), false)
    );
    assert_eq!(
        book.lease(PoolClass::Engine, &p("/.full"), "r1", 2),
        Lease::Full
    );
    assert_eq!(state(&book, task(1)), DirState::Fresh);
    assert_eq!(state(&book, task(2)), DirState::Fresh);
}

#[test]
fn no_free_dir_is_full() {
    let task_dirs = 2;
    let mut book = PoolBook::new(task_dirs, 1);
    for n in 1..=task_dirs {
        lease_task(&mut book, &format!("/o{n}"), 1);
    }
    let last = p(&format!("/o{}", task_dirs + 1));
    assert_eq!(book.lease(PoolClass::Task, &last, "r1", 2), Lease::Full);
    assert_eq!(book.release(&p("/o1"), Released::Cleaned, 3), Some(task(1)));
    assert_eq!(
        book.lease(PoolClass::Task, &last, "r1", 4),
        dir(task(1), false)
    );
}

#[test]
fn a_dir_released_without_a_clean_is_wiped_before_its_next_lease() {
    for (how, after, wipe_first) in [
        (Released::NotCleaned, DirState::Dirty, true),
        (Released::Cleaned, DirState::Clean, false),
        (Released::Wiped, DirState::Fresh, false),
    ] {
        let mut book = PoolBook::new(1, 1);
        lease_task(&mut book, "/a", 1);
        assert_eq!(book.release(&p("/a"), how, 2), Some(task(1)));
        assert_eq!(state(&book, task(1)), after, "{how:?}");
        assert_eq!(
            book.lease(PoolClass::Task, &p("/b"), "r1", 3),
            dir(task(1), wipe_first),
            "{how:?}"
        );
    }
    // Releasing an owner that holds nothing changes nothing.
    let mut book = PoolBook::new(1, 1);
    assert_eq!(book.release(&p("/nobody"), Released::Cleaned, 1), None);
    assert_eq!(book, PoolBook::new(1, 1));
}

#[test]
fn reconcile_frees_leases_whose_checkout_is_gone() {
    let mut book = PoolBook::new(3, 1);
    lease_task(&mut book, "/gone", 5);
    lease_task(&mut book, "/here", 5);
    let freed = book.reconcile(&|path: &Path| path == Path::new("/here"), 10);
    assert_eq!(freed, [task(1)]);
    let gone = book.dir(task(1)).unwrap();
    assert_eq!(gone.state, DirState::Dirty);
    assert_eq!(gone.owner, None);
    assert_eq!(gone.run, None);
    assert_eq!(gone.last_owner, Some(p("/gone")));
    let here = book.dir(task(2)).unwrap();
    assert_eq!(here.state, DirState::Leased);
    assert_eq!(here.owner, Some(p("/here")));
    // Affinity survives: the vanished owner, back again, gets its old directory.
    assert_eq!(
        book.lease(PoolClass::Task, &p("/gone"), "r1", 20),
        dir(task(1), true)
    );
}

#[test]
fn reconcile_spares_a_lease_made_since_leased_before() {
    let mut book = PoolBook::new(2, 1);
    lease_task(&mut book, "/missing", 10);
    let absent = |_: &Path| false;
    // Leased at `leased_before` (a lease is made before its checkout exists): spared.
    assert!(book.reconcile(&absent, 10).is_empty());
    assert_eq!(state(&book, task(1)), DirState::Leased);
    assert!(book.reconcile(&absent, 3).is_empty());
    assert_eq!(state(&book, task(1)), DirState::Leased);
    // Leased before it: freed.
    assert_eq!(book.reconcile(&absent, 11), [task(1)]);
    assert_eq!(state(&book, task(1)), DirState::Dirty);
}

#[test]
fn forget_runs_frees_leases_of_unknown_runs() {
    let mut book = PoolBook::new(3, 1);
    book.lease(PoolClass::Task, &p("/wt/runs/r1/t1"), "r1", 1);
    book.lease(PoolClass::Task, &p("/wt/runs/r2/t1"), "r2", 1);
    let freed = book.forget_runs(&|run: &str| run == "r1");
    assert_eq!(freed, [task(2)]);
    assert_eq!(state(&book, task(2)), DirState::Dirty);
    assert_eq!(
        book.dir(task(2)).unwrap().last_owner,
        Some(p("/wt/runs/r2/t1"))
    );
    let kept = book.dir(task(1)).unwrap();
    assert_eq!(kept.state, DirState::Leased);
    assert_eq!(kept.run.as_deref(), Some("r1"));
}

#[test]
fn a_dir_being_wiped_is_not_leased() {
    let idle_secs = 100;
    let mut book = PoolBook::new(3, 1);
    lease_task(&mut book, "/old", 0);
    lease_task(&mut book, "/held", 0);
    lease_task(&mut book, "/recent", 0);
    book.release(&p("/old"), Released::Cleaned, 0);
    book.release(&p("/recent"), Released::Cleaned, 150);
    let now = 200;

    assert!(!book.begin_wipe(task(2), now, idle_secs), "leased");
    assert!(!book.begin_wipe(task(3), now, idle_secs), "recently used");
    assert_eq!(state(&book, task(3)), DirState::Clean);
    assert!(book.begin_wipe(task(1), now, idle_secs));
    assert_eq!(state(&book, task(1)), DirState::Wiping);
    // Marked once: a second sweep cannot take it again.
    assert!(!book.begin_wipe(task(1), now, idle_secs));

    // A directory being wiped is never free, not even to its own last owner.
    let file = book.to_file();
    let saved = file
        .dirs
        .iter()
        .find(|d| d.k == 1 && d.class == PoolClass::Task);
    assert_eq!(saved.unwrap().state, DirState::Dirty);
    assert_eq!(
        book.lease(PoolClass::Task, &p("/old"), "r1", now),
        dir(task(3), false)
    );
    assert_eq!(
        book.lease(PoolClass::Task, &p("/new"), "r1", now),
        Lease::Full
    );

    book.end_wipe(task(1));
    assert_eq!(state(&book, task(1)), DirState::Fresh);
    assert_eq!(
        book.lease(PoolClass::Task, &p("/new"), "r1", now),
        dir(task(1), false)
    );
}

#[test]
fn unlease_rolls_a_lease_back() {
    let mut book = PoolBook::new(1, 1);
    lease_task(&mut book, "/a", 1);
    book.release(&p("/a"), Released::NotCleaned, 5);
    let before = book.clone();
    let before_dir = book.dir(task(1)).cloned();

    assert_eq!(
        book.lease(PoolClass::Task, &p("/b"), "r9", 9),
        dir(task(1), true)
    );
    book.unlease(&p("/b"));
    assert_eq!(book.dir(task(1)).cloned(), before_dir);
    assert_eq!(book, before);
    assert_eq!(book.to_file(), before.to_file());
    // Free again, in the state it had: the next lease still wipes first.
    assert_eq!(
        book.lease(PoolClass::Task, &p("/c"), "r9", 10),
        dir(task(1), true)
    );

    // Unleasing an owner that holds nothing changes nothing.
    let held = book.clone();
    book.unlease(&p("/nobody"));
    assert_eq!(book, held);
}

#[test]
fn idle_lists_free_dirs_past_the_limit_only() {
    let idle_secs = 100;
    let mut book = PoolBook::new(3, 1);
    lease_task(&mut book, "/o1", 0);
    lease_task(&mut book, "/held", 0);
    lease_task(&mut book, "/o3", 0);
    book.release(&p("/o1"), Released::Cleaned, 0);
    book.release(&p("/o3"), Released::NotCleaned, 150);

    // Exactly at the limit is not past it.
    assert!(book.idle(100, idle_secs).is_empty());
    assert_eq!(book.idle(101, idle_secs), [task(1)]);
    assert_eq!(book.idle(200, idle_secs), [task(1)]);
    // A leased directory is never idle, however long ago it was leased; a never-used
    // one (`engine-1`, `Fresh`) has nothing to wipe.
    assert_eq!(book.idle(10_000, idle_secs), [task(1), task(3)]);
}

#[test]
fn applies_needs_a_stored_profile_and_a_worktree_target_dir() {
    let mut profile = resolve_profile(&ProfileSpec::default(), &ProfileSpec::default());
    assert!(!applies(&profile, Some(ProfileSource::Stored)), "absent");
    profile
        .env
        .insert(TARGET_DIR_VAR.to_string(), "{worktree}/target".to_string());
    assert!(applies(&profile, Some(ProfileSource::Stored)));
    assert!(!applies(&profile, Some(ProfileSource::Plan)));
    assert!(!applies(&profile, Some(ProfileSource::None)));
    assert!(!applies(&profile, None));
    profile
        .env
        .insert(TARGET_DIR_VAR.to_string(), "/abs/target".to_string());
    assert!(!applies(&profile, Some(ProfileSource::Stored)));
}

#[test]
fn class_of_each_checkout() {
    let runs_dir = p("/wt/runs/r1");
    let integration = runs_dir.join("integration");
    let full = runs_dir.join(".full");
    let class = |path: &Path| class_of(path, &runs_dir, &integration, &full);
    assert_eq!(class(&integration), Some(PoolClass::Engine));
    assert_eq!(class(&full), Some(PoolClass::Engine));
    assert_eq!(class(&runs_dir.join("t3")), Some(PoolClass::Task));
    assert_eq!(class(&runs_dir.join("t3.proof")), Some(PoolClass::Task));
    assert_eq!(class(&runs_dir.join("t3.a")), Some(PoolClass::Task));
    assert_eq!(class(&runs_dir.join("t3.review")), None);
    assert_eq!(class(&runs_dir.join("t3.a.review")), None);
    assert_eq!(class(&p("/wt/runs/.profile-verify")), None);
    assert_eq!(class(&p("/elsewhere/t3")), None);
    assert_eq!(class(&runs_dir.join("t3").join("sub")), None);
    assert_eq!(class(&runs_dir), None);
}

#[test]
fn a_dir_id_names_its_directory() {
    assert_eq!(task(1).name(), "task-1");
    assert_eq!(engine(12).name(), "engine-12");
}
