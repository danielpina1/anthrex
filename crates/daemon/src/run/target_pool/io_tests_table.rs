//! The pool's I/O without `cargo`: leases, the table on disk, the sweep, `restore`, and
//! the races of C9, C10 and the task-4 review (M1–M5, M7).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::io_tests::{Rig, env, eventually, is_empty, plant, pool, task};
use super::*;

/// Far enough in the future for every directory to be idle.
fn later(pool_now_offset: u64) -> u64 {
    unix_now() + 7 * DAY_SECS + pool_now_offset
}

fn seam(
    pool: &TargetPool,
    stage: &'static str,
) -> (mpsc::UnboundedReceiver<()>, mpsc::UnboundedSender<()>) {
    let (reached_tx, reached_rx) = mpsc::unbounded_channel();
    let (go_tx, go_rx) = mpsc::unbounded_channel();
    pool.set_seam(Seam {
        stage,
        reached: reached_tx,
        go: Arc::new(AsyncMutex::new(go_rx)),
    });
    (reached_rx, go_tx)
}

/// Waits, up to a deadline, for the pool to reach the seam. 10 s bounds a failure only:
/// reaching a seam is one or two `spawn_blocking` file steps, with no constant involved.
async fn wait_reached(reached: &mut mpsc::UnboundedReceiver<()>) {
    tokio::time::timeout(Duration::from_secs(10), reached.recv())
        .await
        .expect("the pool never reached the seam")
        .unwrap();
}

fn set_mode(dir: &Path, mode: u32) {
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// `dir` read-only until dropped, so a failing test still leaves it removable.
struct ReadOnly(std::path::PathBuf);

impl ReadOnly {
    fn new(dir: std::path::PathBuf) -> Self {
        std::fs::create_dir_all(&dir).unwrap();
        set_mode(&dir, 0o500);
        ReadOnly(dir)
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        set_mode(&self.0, 0o700);
    }
}

#[tokio::test]
async fn the_sweep_wipes_idle_dirs_and_frees_vanished_owners() {
    let rig = Rig::new();
    let pool = pool(2);
    let a = rig.checkout("t1");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    plant(&dir);
    assert_eq!(
        pool.release(&rig.ctx, None, &a, false).await,
        ReleaseOutcome::NotCleaned("the run was discarded".to_string())
    );
    let b = rig.checkout("t2");
    assert_eq!(
        pool.lease(&rig.ctx, &b, &env()).await,
        Some(rig.pool_dir("task-2"))
    );
    std::fs::remove_dir_all(&b).unwrap();

    // C9: an absent owner is freed only when it was leased before the previous sweep.
    pool.sweep(later(10)).await;
    assert_eq!(rig.state(&pool, task(2)), Some(DirState::Leased));
    pool.sweep(later(11)).await;

    assert!(is_empty(&dir), "the idle directory is emptied");
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Fresh));
    assert_eq!(rig.state(&pool, task(2)), Some(DirState::Dirty));
    let file = rig.leases();
    let entry = |k| {
        file.dirs
            .iter()
            .find(|d| d.k == k && d.class == PoolClass::Task)
            .unwrap()
    };
    assert_eq!(entry(1).state, DirState::Fresh);
    assert_eq!(entry(2).state, DirState::Dirty);
    assert_eq!(entry(2).owner, None);
    assert_eq!(entry(2).last_owner.as_deref(), Some(b.as_path()));
}

#[tokio::test]
async fn a_gone_checkout_and_a_discarded_one_are_released_dirty_without_cargo() {
    let rig = Rig::new();
    let pool = pool(2);
    let a = rig.checkout("t1");
    pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    std::fs::remove_dir_all(&a).unwrap();
    assert_eq!(
        pool.release(&rig.ctx, None, &a, true).await,
        ReleaseOutcome::NotCleaned("the checkout is gone".to_string())
    );
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Dirty));

    let b = rig.checkout("t2");
    let dir = pool.lease(&rig.ctx, &b, &env()).await.unwrap();
    assert_eq!(dir, rig.pool_dir("task-2"));
    plant(&dir);
    assert_eq!(
        pool.release(&rig.ctx, None, &b, false).await,
        ReleaseOutcome::NotCleaned("the run was discarded".to_string())
    );
    assert!(
        dir.join("debug/app").exists(),
        "its artifacts are still there"
    );
    assert_eq!(rig.state(&pool, task(2)), Some(DirState::Dirty));

    // Both are `Dirty`, so the next lease wipes first (task-5 review M3: `task-1`, the
    // one it picks, holds something too).
    plant(&rig.pool_dir("task-1"));
    let c = rig.checkout("t3");
    let dir = pool.lease(&rig.ctx, &c, &env()).await.unwrap();
    assert!(is_empty(&dir));
}

#[tokio::test]
async fn a_fallen_back_checkout_stays_on_fallback_until_released() {
    let rig = Rig::new();
    let pool = pool(1);
    let (a, b) = (rig.checkout("t1"), rig.checkout("t2"));
    assert!(pool.lease(&rig.ctx, &a, &env()).await.is_some());
    assert_eq!(pool.lease(&rig.ctx, &b, &env()).await, None);
    pool.release(&rig.ctx, None, &a, false).await;
    assert_eq!(pool.lease(&rig.ctx, &b, &env()).await, None);
    assert_eq!(
        pool.release(&rig.ctx, None, &b, true).await,
        ReleaseOutcome::NotLeased
    );
    assert_eq!(
        pool.lease(&rig.ctx, &b, &env()).await,
        Some(rig.pool_dir("task-1"))
    );
}

/// Task-4 review M1 (the ruling replacing `a_lease_waits_for_a_wipe_of_its_dir`): a
/// lease never waits for a wipe; it skips a directory being wiped and, with no other
/// free, falls back until released (C24).
#[tokio::test]
async fn a_lease_skips_a_dir_being_wiped() {
    let rig = Rig::new();
    let pool = Arc::new(pool(1));
    let a = rig.checkout("t1");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    plant(&dir);
    pool.release(&rig.ctx, None, &a, false).await;

    let (mut reached, go) = seam(&pool, "marked");
    let sweeping = tokio::spawn({
        let pool = Arc::clone(&pool);
        async move { pool.sweep(later(10)).await }
    });
    wait_reached(&mut reached).await;
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Wiping));
    assert_eq!(pool.lease(&rig.ctx, &a, &env()).await, None);
    go.send(()).unwrap();
    sweeping.await.unwrap();

    assert!(is_empty(&dir));
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Fresh));
    assert_eq!(pool.lease(&rig.ctx, &a, &env()).await, None, "C24");
    pool.release(&rig.ctx, None, &a, true).await;
    assert_eq!(pool.lease(&rig.ctx, &a, &env()).await, Some(dir));
}

#[tokio::test]
async fn a_lease_whose_save_fails_falls_back() {
    let rig = Rig::new();
    let pool = pool(1);
    let read_only = ReadOnly::new(rig.pool_root());
    let a = rig.checkout("t1");
    let leased = pool.lease(&rig.ctx, &a, &env()).await;
    drop(read_only);
    assert_eq!(leased, None);
    let book = pool.book(&rig.ctx.repo_dir).unwrap();
    assert_eq!(book.dir(task(1)).unwrap().state, DirState::Fresh);
    assert_eq!(book.dir(task(1)).unwrap().owner, None);
}

/// Task-4 review M2: a holder's re-lease waits for its first lease's save, so a failed
/// save cannot free the directory under it.
#[tokio::test]
async fn a_re_lease_waits_for_the_holders_pending_save() {
    let rig = Rig::new();
    let pool = Arc::new(pool(1));
    let read_only = ReadOnly::new(rig.pool_root());
    let a = rig.checkout("t1");
    let (mut reached, go) = seam(&pool, "saving");
    let first = tokio::spawn({
        let (pool, ctx, a) = (Arc::clone(&pool), rig.ctx.clone(), a.clone());
        async move { pool.lease(&ctx, &a, &env()).await }
    });
    wait_reached(&mut reached).await;
    let mut second = tokio::spawn({
        let (pool, ctx, a) = (Arc::clone(&pool), rig.ctx.clone(), a.clone());
        async move { pool.lease(&ctx, &a, &env()).await }
    });
    // Negative check: the re-lease must not finish while the first save is pending. The
    // 300 ms wait cannot fail falsely (a slow machine only lets a broken pool pass).
    if let Ok(early) = tokio::time::timeout(Duration::from_millis(300), &mut second).await {
        panic!(
            "the re-lease returned {:?} before the first save",
            early.unwrap()
        );
    }
    go.send(()).unwrap();
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    drop(read_only);
    assert_eq!((first, second), (None, None));
    let book = pool.book(&rig.ctx.repo_dir).unwrap();
    assert_eq!(book.dir(task(1)).unwrap().owner, None);
}

#[tokio::test]
async fn a_linked_pool_dir_is_replaced_not_followed() {
    let rig = Rig::new();
    let outside = rig.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep"), "mine").unwrap();
    std::fs::create_dir_all(rig.pool_root()).unwrap();
    std::os::unix::fs::symlink(&outside, rig.pool_dir("task-1")).unwrap();

    let pool = pool(1);
    let a = rig.checkout("t1");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    let meta = std::fs::symlink_metadata(&dir).unwrap();
    assert!(meta.is_dir() && !meta.file_type().is_symlink());
    assert!(is_empty(&dir));
    let trash = rig.pool_dir("trash");
    eventually("the trash to be deleted", || {
        !trash.exists() || is_empty(&trash)
    });
    assert_eq!(
        std::fs::read_to_string(outside.join("keep")).unwrap(),
        "mine"
    );
}

/// Task-5 review I1: a holder's re-lease that waits on its directory while its own
/// release cleans it does not get the directory once the release has freed it, so no
/// later owner can share it.
#[tokio::test]
async fn a_re_lease_racing_its_release_falls_back() {
    let rig = Rig::new();
    let pool = Arc::new(pool(1));
    let a = rig.checkout("t1");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    let (mut cleaning, release_go) = seam(&pool, "cleaning");
    let release = tokio::spawn({
        let (pool, ctx, a) = (Arc::clone(&pool), rig.ctx.clone(), a.clone());
        async move { pool.release(&ctx, None, &a, false).await }
    });
    wait_reached(&mut cleaning).await;
    // The re-lease decides (the book still shows A holding `task-1`), then waits on the
    // directory the paused release holds.
    let (mut decided, lease_go) = seam(&pool, "dir lock");
    let re_lease = tokio::spawn({
        let (pool, ctx, a) = (Arc::clone(&pool), rig.ctx.clone(), a.clone());
        async move { pool.lease(&ctx, &a, &env()).await }
    });
    wait_reached(&mut decided).await;
    lease_go.send(()).unwrap();
    release_go.send(()).unwrap();
    release.await.unwrap();
    assert_eq!(
        re_lease.await.unwrap(),
        None,
        "A's re-lease got a freed dir"
    );
    let b = rig.checkout("t2");
    assert_eq!(pool.lease(&rig.ctx, &b, &env()).await, Some(dir));
    let book = pool.book(&rig.ctx.repo_dir).unwrap();
    assert_eq!(
        book.dir(task(1)).unwrap().owner.as_deref(),
        Some(b.as_path())
    );
}

/// Task-5 review M8: one pool has one book, whatever the spelling of its repository
/// directory (a link, or `..`), so two spellings can never lease one directory twice.
#[tokio::test]
async fn one_pool_one_book_whatever_the_spelling() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.data.join("other")).unwrap();
    let link = rig.root.join("data-link");
    std::os::unix::fs::symlink(&rig.data, &link).unwrap();
    let spellings = [
        link.join("repos/repo-1"),
        rig.data.join("other/../repos/repo-1"),
    ];
    for (n, repo_dir) in spellings.into_iter().enumerate() {
        let pool = pool(1);
        let other = PoolCtx {
            repo_dir,
            ..rig.ctx.clone()
        };
        // The other spelling's book is loaded first, with `task-1` free.
        let x = rig.checkout(&format!("x{n}"));
        pool.lease(&other, &x, &env()).await.unwrap();
        pool.release(&other, None, &x, false).await;
        let a = rig.checkout(&format!("a{n}"));
        assert!(
            pool.lease(&rig.ctx, &a, &env()).await.is_some(),
            "spelling {n}"
        );
        let b = rig.checkout(&format!("b{n}"));
        assert_eq!(pool.lease(&other, &b, &env()).await, None, "spelling {n}");
        std::fs::remove_dir_all(rig.pool_root()).unwrap();
    }
    // `restore` through the link reads the same book: a lease is kept.
    let pool = pool(1);
    let a = rig.checkout("t1");
    pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    pool.restore(&link, &["r1".to_string()].into()).await;
    let b = rig.checkout("t2");
    assert_eq!(pool.lease(&rig.ctx, &b, &env()).await, None);
}

/// Task-5 review M1: a `trash` that is a link is never followed: `restore` leaves its
/// target's files alone, and a wipe does not rename into it.
#[tokio::test]
async fn a_linked_trash_is_not_followed() {
    let rig = Rig::new();
    let outside = rig.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep"), "mine").unwrap();
    std::fs::create_dir_all(rig.pool_root()).unwrap();
    std::os::unix::fs::symlink(&outside, rig.pool_dir(POOL_TRASH_DIR)).unwrap();

    let pool = pool(1);
    pool.restore(&rig.data, &BTreeSet::new()).await;
    assert_eq!(
        std::fs::read_to_string(outside.join("keep")).unwrap(),
        "mine"
    );
    let a = rig.checkout("t1");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.unwrap();
    plant(&dir);
    pool.release(&rig.ctx, None, &a, true).await;
    let entries: Vec<_> = std::fs::read_dir(&outside).unwrap().flatten().collect();
    assert_eq!(entries.len(), 1, "a wipe renamed into the link's target");
}
