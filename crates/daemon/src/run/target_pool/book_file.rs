//! `leases.json`, the book's form on disk (milestone 9.14 decision 14). Pure, as
//! `book.rs` is: the I/O layer reads and writes the file atomically (M9.14.5).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{DirId, DirState, PoolBook, PoolClass, PoolDir};

/// The `version` of `leases.json` this book writes and reads (decision 14).
pub const LEASES_VERSION: u32 = 1;

/// `leases.json` (decision 14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasesFile {
    pub version: u32,
    pub dirs: Vec<LeasedDir>,
}

/// One directory in `leases.json`. `owner`, `run` and `leased_at` are written only for
/// a leased directory; `state` is never `wiping`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedDir {
    pub class: PoolClass,
    pub k: u16,
    pub state: DirState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leased_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_owner: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
}

impl PoolBook {
    /// The book as `leases.json` holds it; a directory being wiped is saved `dirty`.
    pub fn to_file(&self) -> LeasesFile {
        let dirs = self
            .dirs
            .iter()
            .map(|(id, d)| {
                let leased = d.state == DirState::Leased;
                LeasedDir {
                    class: id.class,
                    k: id.k,
                    state: match d.state {
                        DirState::Wiping => DirState::Dirty,
                        state => state,
                    },
                    owner: d.owner.clone().filter(|_| leased),
                    run: d.run.clone().filter(|_| leased),
                    leased_at: d.leased_at.filter(|_| leased),
                    last_owner: d.last_owner.clone(),
                    last_used: d.last_used,
                }
            })
            .collect();
        LeasesFile {
            version: LEASES_VERSION,
            dirs,
        }
    }

    /// The book `file` records, sized `task_dirs` and `engine_dirs`. A free directory
    /// past the new size is dropped (the I/O layer deletes it); a leased one stays until
    /// released (decision 14). A `wiping` entry, or a leased one without an owner, is
    /// `Dirty`. The caller checks `version`.
    pub fn from_file(file: &LeasesFile, task_dirs: u16, engine_dirs: u16) -> Self {
        let mut book = PoolBook::new(task_dirs, engine_dirs);
        for entry in &file.dirs {
            if entry.k == 0 {
                continue;
            }
            let id = DirId {
                class: entry.class,
                k: entry.k,
            };
            let leased = entry.state == DirState::Leased && entry.owner.is_some();
            let state = match entry.state {
                DirState::Wiping => DirState::Dirty,
                DirState::Leased if !leased => DirState::Dirty,
                state => state,
            };
            if !leased && !book.in_size(id) {
                continue;
            }
            let dir = PoolDir {
                state,
                owner: entry.owner.clone().filter(|_| leased),
                run: entry.run.clone().filter(|_| leased),
                leased_at: entry.leased_at.filter(|_| leased),
                last_owner: entry.last_owner.clone(),
                last_used: entry.last_used,
            };
            book.dirs.insert(id, dir);
        }
        book
    }
}

#[cfg(test)]
#[path = "book_file_tests.rs"]
mod tests;
