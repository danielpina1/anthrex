//! The target pool (milestone 9.14 decisions 1–15). `book.rs` is the pure book,
//! `clean.rs` the pure release plan; this file is the I/O layer over them (M9.14.5):
//! the per-checkout `lease`, `release` with its clean or wipe, the idle `sweep` and
//! `restore`. `apply.rs` holds `apply`, the op-shaped wrapper over `lease_why` that the
//! driver calls before every op (M9.14.6).
//!
//! Locks (AGENTS.md rules 2 and 3, C9, C19). The books sit under one `std::sync::Mutex`
//! taken with `daemon::lock`, held only to read or update a book, never across an
//! `.await`. Each repository has a `tokio::sync::Mutex`, its *table* lock, which
//! serialises loading its book, deciding a lease together with the save that makes it
//! durable (so a holder's re-lease waits for its first lease's save, task-4 review M2),
//! and every later update and save. Each pool directory has its own `tokio::sync::Mutex`,
//! held across its clean or wipe; a lease takes it after the table lock is released and
//! drops it before returning. Order: a directory's lock may be held while the table
//! lock is taken, never the reverse. Every file-system step runs on `spawn_blocking`.

mod apply;
mod book;
pub mod clean;
mod disk;
mod release_clean;
#[cfg(test)]
mod seam;
#[cfg(test)]
pub(crate) use seam::Seam;
mod spelling;
mod upkeep;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub use book::{
    DirId, DirState, LEASES_VERSION, Lease, LeasedDir, LeasesFile, PoolBook, PoolClass, PoolDir,
    Released, TARGET_DIR_VAR, applies, class_of,
};

use crate::lock;
use crate::run::driver::unix_now;

/// The pool's directory under a repository's data directory (decision 3).
pub const POOL_DIR_NAME: &str = "target-pool";
/// The lease table beside the pool directories (decision 14).
pub const POOL_LEASES_FILE: &str = "leases.json";
/// Where wipes rename directories before deleting them (decision 12).
pub const POOL_TRASH_DIR: &str = "trash";
/// One bound for a release's whole `cargo metadata` + `cargo clean` sequence.
pub const POOL_CLEAN_TIMEOUT: Duration = Duration::from_secs(120);
/// The size measure's bound; its walking thread is abandoned past it (C33).
pub const POOL_MEASURE_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the sweep runs (`RunService::spawn` starts its task).
pub const POOL_SWEEP_EVERY: Duration = Duration::from_secs(3600);
/// The clean's environment overrides (decision 8, C12); `RUSTUP_TOOLCHAIN` is added when
/// known.
pub const CLEAN_ENV_OVERRIDES: [(&str, &str); 3] = [
    ("RUSTC_WRAPPER", ""),
    ("RUSTC_WORKSPACE_WRAPPER", ""),
    ("CARGO_BUILD_RUSTC_WRAPPER", ""),
];

/// Bytes in one `max_gb` unit.
const GB: u64 = 1_000_000_000;
const DAY_SECS: u64 = 86_400;

/// What the pool needs to know about a run's checkouts (C4). `OpCtx.pool` holds one
/// from M9.14.6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolCtx {
    pub run_id: String,
    pub repo_root: PathBuf,
    /// `profile::repo_dir(data, project)`: the pool lives in `<repo_dir>/target-pool/`.
    pub repo_dir: PathBuf,
    /// `<wt_dir>/runs/<run id>`.
    pub runs_dir: PathBuf,
    pub integration: PathBuf,
    pub full: PathBuf,
}

/// What a release did (decision 8). The reasons are exact: wipes `"over the size cap"`,
/// `"cargo metadata failed: <first line>"`, `"cargo clean failed: <first line>"`,
/// `"timed out"`, `"a target triple directory"`, `"an ambiguous package spec"`, `"not a
/// plain directory"`; not cleaned `"the checkout is gone"`, `"the run was discarded"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    Cleaned,
    Wiped(String),
    NotCleaned(String),
    NotLeased,
}

/// What [`TargetPool::lease_why`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Leased {
    Dir(PathBuf),
    /// No free directory of this class: the checkout falls back (decision 7).
    Full(PoolClass),
    /// No lease for any other reason (no class, the pool off, already fallen back, a
    /// failed save or preparation).
    Skip,
}

/// The daemon-wide pool: one book per repository, loaded lazily (C10).
pub struct TargetPool {
    enabled: bool,
    /// `disabled_repos` as configured, and resolved on a blocking thread at the first
    /// lease or release (C27; task-5 review M6: never on the constructing thread).
    disabled_raw: Vec<PathBuf>,
    disabled_repos: std::sync::OnceLock<Vec<PathBuf>>,
    /// Each repository directory and root as given, resolved once (task-5 review M8):
    /// the books, tables and directory locks are keyed by one spelling.
    spellings: Mutex<BTreeMap<PathBuf, PathBuf>>,
    task_dirs: u16,
    engine_dirs: u16,
    cap_bytes: u64,
    idle_secs: u64,
    /// This pool's construction (the daemon's start): `restore` frees only absent owners
    /// leased before it (C9).
    started: u64,
    books: Mutex<BTreeMap<PathBuf, PoolBook>>,
    tables: Mutex<BTreeMap<PathBuf, Arc<AsyncMutex<()>>>>,
    dirs: Mutex<BTreeMap<PathBuf, Arc<AsyncMutex<()>>>>,
    /// The environment `lease` last recorded per leased checkout (decision 8).
    envs: Mutex<BTreeMap<PathBuf, Vec<(String, String)>>>,
    /// The checkouts that fell back, until released (decision 7, C24).
    fell_back: Mutex<BTreeSet<PathBuf>>,
    /// Each repository's previous sweep (C9).
    last_sweep: Mutex<BTreeMap<PathBuf, u64>>,
    /// The checkouts whose fallback the run log was told of: once per daemon lifetime
    /// (decision 7).
    noted: Mutex<BTreeSet<PathBuf>>,
    #[cfg(test)]
    seam: Mutex<Option<Seam>>,
}

impl TargetPool {
    /// The pool `config` describes; `max_writers` sizes an unset `task_dirs` (C26).
    pub fn new(config: &config::TargetPool, max_writers: u8) -> Self {
        TargetPool {
            enabled: config.enabled,
            disabled_raw: config.disabled_repos.iter().map(PathBuf::from).collect(),
            disabled_repos: std::sync::OnceLock::new(),
            spellings: Mutex::new(BTreeMap::new()),
            task_dirs: config.task_dirs_for(max_writers),
            engine_dirs: config.engine_dirs,
            cap_bytes: u64::from(config.max_gb).saturating_mul(GB),
            idle_secs: u64::from(config.idle_days).saturating_mul(DAY_SECS),
            started: unix_now(),
            books: Mutex::new(BTreeMap::new()),
            tables: Mutex::new(BTreeMap::new()),
            dirs: Mutex::new(BTreeMap::new()),
            envs: Mutex::new(BTreeMap::new()),
            fell_back: Mutex::new(BTreeSet::new()),
            last_sweep: Mutex::new(BTreeMap::new()),
            noted: Mutex::new(BTreeSet::new()),
            #[cfg(test)]
            seam: Mutex::new(None),
        }
    }

    /// The size cap in bytes (the byte form of `max_gb`), for tests.
    #[cfg(test)]
    pub(crate) fn with_cap_bytes(mut self, cap_bytes: u64) -> Self {
        self.cap_bytes = cap_bytes;
        self
    }

    /// A copy of `repo_dir`'s book, when it is loaded.
    pub(crate) fn book(&self, repo_dir: &Path) -> Option<PoolBook> {
        lock(&self.books).get(repo_dir).cloned()
    }

    /// The per-checkout primitive (C4): class, fallback memory, lease, durable save,
    /// the directory's mutex, `wipe_first`; records `env`. `None`: keep the profile's
    /// value. The tests' form of [`TargetPool::lease_why`], which `apply` calls.
    #[cfg(test)]
    pub(crate) async fn lease(
        &self,
        pool: &PoolCtx,
        checkout: &Path,
        env: &[(String, String)],
    ) -> Option<PathBuf> {
        match self.lease_why(pool, checkout, env).await {
            Leased::Dir(dir) => Some(dir),
            Leased::Full(_) | Leased::Skip => None,
        }
    }

    /// [`TargetPool::lease`], saying why there is no directory: `Full` when this call
    /// found no free directory of the checkout's class (the checkout now falls back,
    /// decision 7), `Skip` for every other reason.
    pub(crate) async fn lease_why(
        &self,
        pool: &PoolCtx,
        checkout: &Path,
        env: &[(String, String)],
    ) -> Leased {
        let Some(class) = class_of(checkout, &pool.runs_dir, &pool.integration, &pool.full) else {
            return Leased::Skip;
        };
        if !self.enabled {
            return Leased::Skip;
        }
        let pool = &self.spelled(pool).await;
        let disabled = self.disabled_repos.get().map(Vec::as_slice).unwrap_or(&[]);
        if disabled.contains(&pool.repo_root) {
            return Leased::Skip;
        }
        let repo = &pool.repo_dir;
        let table = self.table(repo).await;
        // Under the table lock, so a holder's re-lease sees a failed first save (M2).
        if lock(&self.fell_back).contains(checkout) {
            return Leased::Skip;
        }
        self.load(repo).await;
        let decided = {
            let mut books = lock(&self.books);
            books.get_mut(repo).map(|book| {
                let before = book.clone();
                let lease = book.lease(class, checkout, &pool.run_id, unix_now());
                (lease, *book != before)
            })
        };
        let Some((lease, new)) = decided else {
            return Leased::Skip;
        };
        let Lease::Dir { id, wipe_first } = lease else {
            lock(&self.fell_back).insert(checkout.to_path_buf());
            return Leased::Full(class);
        };
        if new && let Err(error) = self.save(repo).await {
            tracing::warn!(
                ?error,
                "target pool: a lease could not be saved; falling back"
            );
            if let Some(book) = lock(&self.books).get_mut(repo) {
                book.unlease(checkout);
            }
            lock(&self.fell_back).insert(checkout.to_path_buf());
            return Leased::Skip;
        }
        drop(table);

        let root = repo.join(POOL_DIR_NAME);
        let dir_lock = self.dir_lock(&root.join(id.name()));
        #[cfg(test)]
        self.pause("dir lock").await;
        let held = dir_lock.lock().await;
        // Task-5 review I1: a release of this checkout that held the directory may have
        // freed it meanwhile; the lease stands only while the book still says so.
        let still = lock(&self.books).get(repo).is_some_and(|book| {
            book.dir(id).is_some_and(|d| {
                d.state == DirState::Leased && d.owner.as_deref() == Some(checkout)
            })
        });
        if !still {
            drop(held);
            lock(&self.fell_back).insert(checkout.to_path_buf());
            return Leased::Skip;
        }
        let name = id.name();
        let prepared =
            blocking(move || disk::prepare(&root, &name, wipe_first).map_err(|e| e.to_string()))
                .await;
        drop(held);
        match prepared {
            Ok(dir) => {
                lock(&self.envs).insert(checkout.to_path_buf(), env.to_vec());
                Leased::Dir(dir)
            }
            Err(error) => {
                tracing::warn!(%error, "target pool: could not prepare a directory");
                self.free(repo, checkout, Released::NotCleaned).await;
                lock(&self.fell_back).insert(checkout.to_path_buf());
                Leased::Skip
            }
        }
    }

    /// The table lock of `repo` (see the module's "Locks").
    async fn table(&self, repo: &Path) -> OwnedMutexGuard<()> {
        let table = Arc::clone(lock(&self.tables).entry(repo.to_path_buf()).or_default());
        table.lock_owned().await
    }

    fn dir_lock(&self, dir: &Path) -> Arc<AsyncMutex<()>> {
        Arc::clone(lock(&self.dirs).entry(dir.to_path_buf()).or_default())
    }

    /// Loads `repo`'s book on its first touch (C10). The caller holds its table lock.
    async fn load(&self, repo: &Path) {
        if lock(&self.books).contains_key(repo) {
            return;
        }
        let root = repo.join(POOL_DIR_NAME);
        let (task_dirs, engine_dirs) = (self.task_dirs, self.engine_dirs);
        let book = match blocking(move || Ok(disk::load(&root, task_dirs, engine_dirs))).await {
            Ok(book) => book,
            Err(error) => {
                tracing::warn!(%error, "target pool: could not load a table");
                return;
            }
        };
        lock(&self.books).entry(repo.to_path_buf()).or_insert(book);
    }

    /// Saves `repo`'s book (decision 14). The caller holds its table lock, so saves are
    /// ordered and the last one holds the latest book.
    async fn save(&self, repo: &Path) -> Result<(), String> {
        let Some(file) = lock(&self.books).get(repo).map(PoolBook::to_file) else {
            return Ok(());
        };
        #[cfg(test)]
        self.pause("saving").await;
        let root = repo.join(POOL_DIR_NAME);
        blocking(move || disk::save(&root, &file).map_err(|e| e.to_string())).await
    }

    async fn save_logged(&self, repo: &Path) {
        if let Err(error) = self.save(repo).await {
            tracing::warn!(%error, repo = %repo.display(), "target pool: could not save the table");
        }
    }

    /// Frees `checkout`'s directory as `how` says and saves; a directory past a shrunk
    /// size leaves the book and is deleted on disk (review M5).
    async fn free(&self, repo: &Path, checkout: &Path, how: Released) {
        let _table = self.table(repo).await;
        let freed = lock(&self.books)
            .get_mut(repo)
            .and_then(|book| book.release(checkout, how, unix_now()));
        self.delete_dropped(repo, freed.into_iter().collect()).await;
        self.save_logged(repo).await;
    }

    /// Deletes on disk each of `freed` the book no longer has (review M5).
    async fn delete_dropped(&self, repo: &Path, freed: Vec<DirId>) {
        let gone: Vec<String> = {
            let books = lock(&self.books);
            let Some(book) = books.get(repo) else {
                return;
            };
            freed
                .into_iter()
                .filter(|id| book.dir(*id).is_none())
                .map(|id| id.name())
                .collect()
        };
        if gone.is_empty() {
            return;
        }
        let root = repo.join(POOL_DIR_NAME);
        let removed = blocking(move || {
            gone.iter()
                .try_for_each(|name| disk::remove(&root, name))
                .map_err(|e| e.to_string())
        })
        .await;
        if let Err(error) = removed {
            tracing::warn!(%error, "target pool: could not delete a directory past the size");
        }
    }
}

/// `f` on a blocking thread (AGENTS.md rule 2).
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(error) => Err(format!("a blocking step did not finish: {error}")),
    }
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod io_tests;
#[cfg(test)]
#[path = "io_tests_restore.rs"]
mod io_tests_restore;
#[cfg(test)]
#[path = "io_tests_table.rs"]
mod io_tests_table;
