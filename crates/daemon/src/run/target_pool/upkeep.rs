//! The pool's upkeep (milestone 9.14 decision 13, C9, C10, C25, task-4 review M7): the
//! periodic `sweep` and the start-up `restore`, under the locking rules of `mod.rs`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{POOL_DIR_NAME, TargetPool, blocking, disk};
use crate::lock;

impl TargetPool {
    /// Decision 13 for every loaded repository: idle directories are wiped (C9: marked
    /// under the table lock with the directory's mutex held), then absent owners leased
    /// before the previous sweep are freed `Dirty`, then directories on disk that the
    /// book does not know or shows `Fresh` with content are deleted or wiped (review M7),
    /// and each table is saved.
    pub async fn sweep(&self, now: u64) {
        let repos: Vec<PathBuf> = lock(&self.books).keys().cloned().collect();
        for repo in repos {
            self.wipe_idle(&repo, now).await;
            let before = lock(&self.last_sweep).get(&repo).copied();
            self.reconcile(&repo, before.unwrap_or(self.started)).await;
            lock(&self.last_sweep).insert(repo.clone(), now);
            self.scrub(&repo).await;
            let _table = self.table(&repo).await;
            self.save_logged(&repo).await;
        }
    }

    /// Trash cleanup and `reconcile` (leased before this daemon started) for each table
    /// on disk, then `forget_runs` for runs not in `known` (C10, C25), and the scrub of
    /// review M7.
    pub async fn restore(&self, data_dir: &Path, known: &BTreeSet<String>) {
        let data_dir = data_dir.to_path_buf();
        let repos = blocking(move || Ok(disk::pools_on_disk(&data_dir)))
            .await
            .unwrap_or_default();
        for repo in repos {
            let root = repo.join(POOL_DIR_NAME);
            let _ = blocking(move || {
                disk::empty_trash(&root);
                Ok(())
            })
            .await;
            {
                let _table = self.table(&repo).await;
                self.load(&repo).await;
            }
            self.reconcile(&repo, self.started).await;
            {
                let _table = self.table(&repo).await;
                let freed = match lock(&self.books).get_mut(&repo) {
                    Some(book) => book.forget_runs(&|run| known.contains(run)),
                    None => Vec::new(),
                };
                self.delete_dropped(&repo, freed).await;
            }
            self.scrub(&repo).await;
            let _table = self.table(&repo).await;
            self.save_logged(&repo).await;
        }
    }

    /// The sweep's idle wipes (decision 13, C9).
    async fn wipe_idle(&self, repo: &Path, now: u64) {
        let idle = match lock(&self.books).get(repo) {
            Some(book) => book.idle(now, self.idle_secs),
            None => return,
        };
        let root = repo.join(POOL_DIR_NAME);
        for id in idle {
            let dir_lock = self.dir_lock(&root.join(id.name()));
            let _held = dir_lock.lock().await;
            {
                let _table = self.table(repo).await;
                let marked = lock(&self.books)
                    .get_mut(repo)
                    .is_some_and(|book| book.begin_wipe(id, now, self.idle_secs));
                if !marked {
                    continue;
                }
                self.save_logged(repo).await;
            }
            #[cfg(test)]
            self.pause("marked").await;
            let (root, name) = (root.clone(), id.name());
            let wiped = blocking(move || disk::wipe(&root, &name).map_err(|e| e.to_string())).await;
            let _table = self.table(repo).await;
            match wiped {
                // A failed wipe stays `Wiping` (saved `dirty`) until the next start.
                Err(error) => tracing::warn!(%error, "target pool: an idle wipe failed"),
                Ok(()) => {
                    if let Some(book) = lock(&self.books).get_mut(repo) {
                        book.end_wipe(id);
                    }
                }
            }
        }
    }

    /// Frees, as `Dirty`, absent owners leased before `leased_before` (C9).
    async fn reconcile(&self, repo: &Path, leased_before: u64) {
        let owners: Vec<PathBuf> = match lock(&self.books).get(repo) {
            Some(book) => book
                .to_file()
                .dirs
                .into_iter()
                .filter_map(|d| d.owner)
                .collect(),
            None => return,
        };
        let absent = blocking(move || Ok(disk::absent(owners)))
            .await
            .unwrap_or_default();
        let _table = self.table(repo).await;
        let freed = match lock(&self.books).get_mut(repo) {
            Some(book) => book.reconcile(&|owner| !absent.contains(owner), leased_before),
            None => Vec::new(),
        };
        for owner in &absent {
            lock(&self.envs).remove(owner);
        }
        self.delete_dropped(repo, freed).await;
    }

    /// Review M7 under the directory's mutex and a re-check of the book: a directory
    /// the book does not know is deleted, a `Fresh` one with content wiped. A lease that
    /// picks a `Fresh` directory meanwhile waits for its mutex, so it gets it empty.
    async fn scrub(&self, repo: &Path) {
        let root = repo.join(POOL_DIR_NAME);
        let Some(book) = self.book(repo) else {
            return;
        };
        let scan_root = root.clone();
        let names = blocking(move || Ok(disk::scrub_candidates(&scan_root, &book)))
            .await
            .unwrap_or_default();
        for name in names {
            let dir_lock = self.dir_lock(&root.join(&name));
            let _held = dir_lock.lock().await;
            let Some(book) = self.book(repo) else {
                return;
            };
            let root = root.clone();
            let scrubbed =
                blocking(move || disk::scrub(&root, &name, &book).map_err(|e| e.to_string()));
            if let Err(error) = scrubbed.await {
                tracing::warn!(%error, "target pool: could not scrub a directory");
            }
        }
    }
}
