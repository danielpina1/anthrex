//! The target pool's book (milestone 9.14 decisions 1–7, 13 and 14, the pure parts).
//! Pure (design decision 2): no I/O, no clock. `now` is passed in as unix seconds, and
//! paths are compared as given; the I/O layer (`target_pool/mod.rs`, M9.14.5) resolves
//! them, saves `leases.json`, and does the wipes the book only records.
//!
//! One book per repository. Each directory, `task-<k>` or `engine-<k>`, keeps its
//! state, its owner (the checkout path, decision 4) and run while leased, when it was
//! leased, and its last owner and last use for affinity and recency (decision 5).
//! `book_file.rs` holds its form on disk, `leases.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::run::model::Profile;

#[path = "book_file.rs"]
mod file;
pub use file::{LEASES_VERSION, LeasedDir, LeasesFile};

/// The profile variable the pool substitutes (decision 1).
pub const TARGET_DIR_VAR: &str = "CARGO_TARGET_DIR";

/// Decision 6: the two classes, which never cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolClass {
    Task,
    Engine,
}

/// One pool directory, `<class>-<k>`, `k` from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DirId {
    pub class: PoolClass,
    pub k: u16,
}

impl DirId {
    /// The directory's name in the pool: `task-<k>` or `engine-<k>`.
    pub fn name(&self) -> String {
        let class = match self.class {
            PoolClass::Task => "task",
            PoolClass::Engine => "engine",
        };
        format!("{class}-{}", self.k)
    }
}

/// A directory's state (decisions 8, 13 and 14, C20). `Fresh`: empty (never used, or
/// wiped). `Clean`: its last owner's path packages were cleaned. `Dirty`: released
/// without a clean, or its owner vanished; wiped before its next lease. `Wiping`: the
/// sweep is wiping it; never free, and saved as `dirty`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DirState {
    Fresh,
    Clean,
    Leased,
    Dirty,
    Wiping,
}

impl DirState {
    fn is_free(self) -> bool {
        matches!(self, DirState::Fresh | DirState::Clean | DirState::Dirty)
    }
}

/// What a lease decided. `Full`: no free directory of the class (decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lease {
    /// `wipe_first`: the directory is `Dirty` and must be wiped before it is used.
    Dir {
        id: DirId,
        wipe_first: bool,
    },
    Full,
}

/// How a release left the directory: `Clean`, `Fresh` or `Dirty` (C20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Released {
    Cleaned,
    Wiped,
    NotCleaned,
}

impl Released {
    fn state(self) -> DirState {
        match self {
            Released::Cleaned => DirState::Clean,
            Released::Wiped => DirState::Fresh,
            Released::NotCleaned => DirState::Dirty,
        }
    }
}

/// One directory's record. `owner`, `run` and `leased_at` are set only while `Leased`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolDir {
    pub state: DirState,
    pub owner: Option<PathBuf>,
    pub run: Option<String>,
    pub leased_at: Option<u64>,
    pub last_owner: Option<PathBuf>,
    /// `None`: never used, which counts as the oldest.
    pub last_used: Option<u64>,
}

impl PoolDir {
    fn fresh() -> Self {
        PoolDir {
            state: DirState::Fresh,
            owner: None,
            run: None,
            leased_at: None,
            last_owner: None,
            last_used: None,
        }
    }

    fn held_by(&self, owner: &Path) -> bool {
        self.state == DirState::Leased && self.owner.as_deref() == Some(owner)
    }
}

/// One repository's pool. Directories past the configured size (a shrunk config) stay
/// only while leased; freeing one drops it from the book, and the I/O layer deletes it.
#[derive(Debug, Clone)]
pub struct PoolBook {
    task_dirs: u16,
    engine_dirs: u16,
    dirs: BTreeMap<DirId, PoolDir>,
    /// Each new lease's directory as it was before it, so a lease whose save failed can
    /// be rolled back (C10). Not part of the book's value: never saved, never compared.
    undo: BTreeMap<PathBuf, (DirId, PoolDir)>,
}

impl PartialEq for PoolBook {
    fn eq(&self, other: &Self) -> bool {
        self.task_dirs == other.task_dirs
            && self.engine_dirs == other.engine_dirs
            && self.dirs == other.dirs
    }
}

impl Eq for PoolBook {}

impl PoolBook {
    /// A pool of `task_dirs` task and `engine_dirs` engine directories, all `Fresh`.
    pub fn new(task_dirs: u16, engine_dirs: u16) -> Self {
        let ids = (1..=task_dirs)
            .map(|k| (PoolClass::Task, k))
            .chain((1..=engine_dirs).map(|k| (PoolClass::Engine, k)));
        let dirs = ids
            .map(|(class, k)| (DirId { class, k }, PoolDir::fresh()))
            .collect();
        PoolBook {
            task_dirs,
            engine_dirs,
            dirs,
            undo: BTreeMap::new(),
        }
    }

    /// The record of `id`, when the book has it.
    pub fn dir(&self, id: DirId) -> Option<&PoolDir> {
        self.dirs.get(&id)
    }

    fn in_size(&self, id: DirId) -> bool {
        let size = match id.class {
            PoolClass::Task => self.task_dirs,
            PoolClass::Engine => self.engine_dirs,
        };
        (1..=size).contains(&id.k)
    }

    fn held(&self, owner: &Path) -> Option<DirId> {
        self.dirs
            .iter()
            .find(|(_, d)| d.held_by(owner))
            .map(|(id, _)| *id)
    }

    /// Decision 5: the directory `owner` holds; else a free one of `class` whose last
    /// owner was `owner`; else the free one of `class` used least recently (never used
    /// counts as oldest; ties go to the lowest `k`). `Full` when none is free.
    pub fn lease(&mut self, class: PoolClass, owner: &Path, run: &str, now: u64) -> Lease {
        if let Some(id) = self.held(owner) {
            return Lease::Dir {
                id,
                wipe_first: false,
            };
        }
        let free: Vec<(&DirId, &PoolDir)> = self
            .dirs
            .iter()
            .filter(|(id, d)| id.class == class && self.in_size(**id) && d.state.is_free())
            .collect();
        let affine = free
            .iter()
            .find(|(_, d)| d.last_owner.as_deref() == Some(owner));
        let chosen = affine.or_else(|| free.iter().min_by_key(|(id, d)| (d.last_used, id.k)));
        let Some(id) = chosen.map(|(id, _)| **id) else {
            return Lease::Full;
        };
        let Some(dir) = self.dirs.get_mut(&id) else {
            return Lease::Full;
        };
        self.undo.insert(owner.to_path_buf(), (id, dir.clone()));
        let wipe_first = dir.state == DirState::Dirty;
        *dir = PoolDir {
            state: DirState::Leased,
            owner: Some(owner.to_path_buf()),
            run: Some(run.to_string()),
            leased_at: Some(now),
            last_owner: Some(owner.to_path_buf()),
            last_used: Some(now),
        };
        Lease::Dir { id, wipe_first }
    }

    /// Rolls back `owner`'s last new lease, whose save failed (C10): the directory is
    /// free again in the state it had before. Nothing when `owner` holds nothing.
    pub fn unlease(&mut self, owner: &Path) {
        let Some((id, before)) = self.undo.remove(owner) else {
            return;
        };
        if let Some(dir) = self.dirs.get_mut(&id)
            && dir.held_by(owner)
        {
            *dir = before;
        }
    }

    /// Frees `id` in `state`; `now`, when given, is its last use. A directory past the
    /// configured size leaves the book.
    fn free(&mut self, id: DirId, state: DirState, now: Option<u64>) {
        if !self.in_size(id) {
            self.dirs.remove(&id);
            return;
        }
        if let Some(dir) = self.dirs.get_mut(&id) {
            dir.state = state;
            dir.owner = None;
            dir.run = None;
            dir.leased_at = None;
            if now.is_some() {
                dir.last_used = now;
            }
        }
    }

    /// Ends `owner`'s lease: `Clean`, `Fresh` or `Dirty` as `how` says (C20). The
    /// directory it held, or `None` when it held none.
    pub fn release(&mut self, owner: &Path, how: Released, now: u64) -> Option<DirId> {
        let id = self.held(owner)?;
        self.undo.remove(owner);
        self.free(id, how.state(), Some(now));
        Some(id)
    }

    /// Frees, as `Dirty`, absent owners leased before `leased_before` (C9): a lease is
    /// made before its checkout exists, so a newer one is spared. Each keeps its
    /// `last_owner` for affinity.
    pub fn reconcile(&mut self, exists: &dyn Fn(&Path) -> bool, leased_before: u64) -> Vec<DirId> {
        self.free_where(|d| {
            d.leased_at.unwrap_or(0) < leased_before
                && d.owner.as_deref().is_some_and(|o| !exists(o))
        })
    }

    /// Frees, as `Dirty`, leases whose run `known` does not know (C25).
    pub fn forget_runs(&mut self, known: &dyn Fn(&str) -> bool) -> Vec<DirId> {
        self.free_where(|d| d.run.as_deref().is_none_or(|run| !known(run)))
    }

    fn free_where(&mut self, gone: impl Fn(&PoolDir) -> bool) -> Vec<DirId> {
        let ids: Vec<DirId> = self
            .dirs
            .iter()
            .filter(|(_, d)| d.state == DirState::Leased && gone(d))
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            if let Some(owner) = self.dirs.get(id).and_then(|d| d.owner.clone()) {
                self.undo.remove(&owner);
            }
            self.free(*id, DirState::Dirty, None);
        }
        ids
    }

    fn is_idle(dir: &PoolDir, now: u64, idle_secs: u64) -> bool {
        // A `Fresh` directory is empty already: nothing to wipe.
        matches!(dir.state, DirState::Clean | DirState::Dirty)
            && dir
                .last_used
                .is_none_or(|used| now.saturating_sub(used) > idle_secs)
    }

    /// Free directories with something in them whose last use is more than `idle_secs`
    /// before `now` (decision 13). A leased directory is never idle.
    pub fn idle(&self, now: u64, idle_secs: u64) -> Vec<DirId> {
        self.dirs
            .iter()
            .filter(|(_, d)| Self::is_idle(d, now, idle_secs))
            .map(|(id, _)| *id)
            .collect()
    }

    /// True, and the directory marked `Wiping`, when it is still free and idle (C9).
    pub fn begin_wipe(&mut self, id: DirId, now: u64, idle_secs: u64) -> bool {
        match self.dirs.get_mut(&id) {
            Some(dir) if Self::is_idle(dir, now, idle_secs) => {
                dir.state = DirState::Wiping;
                true
            }
            _ => false,
        }
    }

    /// A sweep's wipe is done: the directory is `Fresh` (C20).
    pub fn end_wipe(&mut self, id: DirId) {
        if let Some(dir) = self.dirs.get_mut(&id)
            && dir.state == DirState::Wiping
        {
            dir.state = DirState::Fresh;
        }
    }
}

/// Decision 1, the profile's half: the run's profile is the stored one and its `env`
/// sets `CARGO_TARGET_DIR` to a value with `{worktree}` in it (one target per checkout).
/// `[target_pool] enabled` and `disabled_repos` are the I/O layer's half.
pub fn applies(profile: &Profile, source: Option<proto::ProfileSource>) -> bool {
    source == Some(proto::ProfileSource::Stored)
        && profile
            .env
            .get(TARGET_DIR_VAR)
            .is_some_and(|value| value.contains("{worktree}"))
}

/// Decision 6: `Engine` for the run's `integration` and `full` checkouts; `Task` for
/// any other directory directly under `runs_dir` (`<wt_dir>/runs/<run id>`) whose name
/// does not end `.review`; `None` (no lease) for anything else.
pub fn class_of(
    checkout: &Path,
    runs_dir: &Path,
    integration: &Path,
    full: &Path,
) -> Option<PoolClass> {
    if checkout == integration || checkout == full {
        return Some(PoolClass::Engine);
    }
    let name = checkout.file_name()?.to_str()?;
    (checkout.parent() == Some(runs_dir) && !name.ends_with(".review")).then_some(PoolClass::Task)
}

#[cfg(test)]
#[path = "book_tests.rs"]
mod tests;
