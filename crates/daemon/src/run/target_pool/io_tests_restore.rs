//! The pool's table at start-up: `restore`, the loader's rules (task-4 review M3, M4),
//! a release past a shrunk size (M5) and the scrub (M7).

use std::collections::BTreeSet;

use super::io_tests::{Rig, env, is_empty, plant, pool, task};
use super::*;

fn known(runs: &[&str]) -> BTreeSet<String> {
    runs.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn restore_reads_the_table_and_deletes_trash() {
    let rig = Rig::new();
    let c1 = rig.checkout("t1");
    let file = LeasesFile {
        version: LEASES_VERSION,
        dirs: vec![
            LeasedDir {
                class: PoolClass::Task,
                k: 1,
                state: DirState::Leased,
                owner: Some(c1.clone()),
                run: Some("r1".to_string()),
                leased_at: Some(1000),
                last_owner: Some(c1.clone()),
                last_used: Some(1000),
            },
            LeasedDir {
                class: PoolClass::Task,
                k: 2,
                state: DirState::Clean,
                owner: None,
                run: None,
                leased_at: None,
                last_owner: Some(rig.ctx.runs_dir.join("t0")),
                last_used: Some(900),
            },
            LeasedDir {
                class: PoolClass::Engine,
                k: 1,
                state: DirState::Fresh,
                owner: None,
                run: None,
                leased_at: None,
                last_owner: None,
                last_used: None,
            },
        ],
    };
    rig.write_leases(&serde_json::to_string(&file).unwrap());
    plant(&rig.pool_dir("task-1"));
    plant(&rig.pool_dir("task-2"));
    std::fs::create_dir_all(rig.pool_dir("trash/x/y")).unwrap();

    let pool1 = pool(2);
    pool1.restore(&rig.data, &known(&["r1"])).await;
    assert_eq!(pool1.book(&rig.ctx.repo_dir).unwrap().to_file(), file);
    assert!(is_empty(&rig.pool_dir("trash")));
    assert!(!is_empty(&rig.pool_dir("task-1")));

    // C25: a lease whose run the daemon does not know is freed `Dirty`.
    let pool2 = pool(2);
    pool2.restore(&rig.data, &known(&[])).await;
    assert_eq!(rig.state(&pool2, task(1)), Some(DirState::Dirty));
    assert_eq!(rig.leases().dirs[0].state, DirState::Dirty);

    // An unreadable table makes every directory `Dirty`.
    rig.write_leases("{ not json");
    let pool3 = pool(2);
    pool3.restore(&rig.data, &known(&["r1"])).await;
    let book = pool3.book(&rig.ctx.repo_dir).unwrap();
    assert!(
        book.to_file()
            .dirs
            .iter()
            .all(|d| d.state == DirState::Dirty)
    );
}

#[tokio::test]
async fn a_table_of_another_version_is_unreadable() {
    let rig = Rig::new();
    let mut file = PoolBook::new(1, 1).to_file();
    file.version = LEASES_VERSION + 1;
    file.dirs[0].state = DirState::Clean;
    rig.write_leases(&serde_json::to_string(&file).unwrap());
    plant(&rig.pool_dir("task-1"));

    let pool = pool(1);
    pool.restore(&rig.data, &known(&[])).await;
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Dirty));
    let a = rig.checkout("t1");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    assert!(
        is_empty(&dir),
        "a dirty directory is wiped before its lease"
    );
}

#[tokio::test]
async fn duplicate_table_entries_are_dropped_keeping_the_first() {
    let rig = Rig::new();
    let c1 = rig.checkout("t1");
    let leased = |k| LeasedDir {
        class: PoolClass::Task,
        k,
        state: DirState::Leased,
        owner: Some(c1.clone()),
        run: Some("r1".to_string()),
        leased_at: Some(1000),
        last_owner: Some(c1.clone()),
        last_used: Some(1000),
    };
    let mut clean_1 = leased(1);
    clean_1.state = DirState::Clean;
    (clean_1.owner, clean_1.run, clean_1.leased_at) = (None, None, None);
    let file = LeasesFile {
        version: LEASES_VERSION,
        // task-1 twice, and c1 on task-1 and task-2.
        dirs: vec![leased(1), clean_1, leased(2)],
    };
    rig.write_leases(&serde_json::to_string(&file).unwrap());
    plant(&rig.pool_dir("task-2"));

    let pool = pool(2);
    pool.restore(&rig.data, &known(&["r1"])).await;
    let book = pool.book(&rig.ctx.repo_dir).unwrap();
    assert_eq!(book.dir(task(1)).unwrap().state, DirState::Leased);
    assert_eq!(
        book.dir(task(1)).unwrap().owner.as_deref(),
        Some(c1.as_path())
    );
    assert_eq!(book.dir(task(2)).unwrap().state, DirState::Fresh);
    assert!(
        is_empty(&rig.pool_dir("task-2")),
        "the dropped entry's directory is wiped"
    );
}

/// Task-4 review M5: a release past a shrunk size deletes the directory.
#[tokio::test]
async fn a_release_past_a_shrunk_size_deletes_the_dir() {
    let rig = Rig::new();
    let c = rig.checkout("t3");
    let mut file = PoolBook::new(3, 1).to_file();
    let third = file.dirs.iter_mut().find(|d| d.k == 3).unwrap();
    third.state = DirState::Leased;
    third.owner = Some(c.clone());
    third.run = Some("r1".to_string());
    third.leased_at = Some(1000);
    rig.write_leases(&serde_json::to_string(&file).unwrap());
    plant(&rig.pool_dir("task-3"));

    let pool = pool(2);
    pool.restore(&rig.data, &known(&["r1"])).await;
    assert_eq!(rig.state(&pool, task(3)), Some(DirState::Leased));
    assert_eq!(
        pool.release(&rig.ctx, None, &c, false).await,
        ReleaseOutcome::NotCleaned("the run was discarded".to_string())
    );
    assert_eq!(rig.state(&pool, task(3)), None);
    assert!(!rig.pool_dir("task-3").exists());
}

/// Task-4 review M7: a pool directory with content that the book shows `Fresh`, or does
/// not know, is wiped or deleted at restore and at the sweep.
#[tokio::test]
async fn a_fresh_or_unknown_dir_with_content_is_scrubbed() {
    let rig = Rig::new();
    rig.write_leases(&serde_json::to_string(&PoolBook::new(2, 1).to_file()).unwrap());
    plant(&rig.pool_dir("task-1"));
    plant(&rig.pool_dir("task-9"));

    let pool = pool(2);
    pool.restore(&rig.data, &known(&[])).await;
    assert!(is_empty(&rig.pool_dir("task-1")));
    assert!(!rig.pool_dir("task-9").exists());

    plant(&rig.pool_dir("task-2"));
    plant(&rig.pool_dir("engine-7"));
    pool.sweep(unix_now()).await;
    assert!(is_empty(&rig.pool_dir("task-2")));
    assert!(!rig.pool_dir("engine-7").exists());
    assert_eq!(rig.state(&pool, task(2)), Some(DirState::Fresh));
}
