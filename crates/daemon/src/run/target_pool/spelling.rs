//! One spelling per pool (task-5 review M6, M8; C27): the repository directory and root
//! an op names, resolved once on a blocking thread, so the books, tables and directory
//! locks are keyed by one path and `disabled_repos` is compared on resolved paths.

use std::path::{Path, PathBuf};

use super::{PoolCtx, TargetPool, blocking, disk};
use crate::lock;

impl TargetPool {
    /// `pool` with its repository directory and root resolved ([`disk::resolve`], on a
    /// blocking thread, once per spelling), and `disabled_repos` resolved on first use
    /// (task-5 review M6, M8; C27).
    pub(super) async fn spelled(&self, pool: &PoolCtx) -> PoolCtx {
        let mut pool = pool.clone();
        let known = |path: &Path| lock(&self.spellings).get(path).cloned();
        if let (Some(dir), Some(root)) = (known(&pool.repo_dir), known(&pool.repo_root))
            && self.disabled_repos.get().is_some()
        {
            (pool.repo_dir, pool.repo_root) = (dir, root);
            return pool;
        }
        let (dir, root, raw) = (
            pool.repo_dir.clone(),
            pool.repo_root.clone(),
            self.disabled_raw.clone(),
        );
        let resolved = blocking(move || {
            let disabled: Vec<PathBuf> = raw.iter().map(|p| disk::resolve(p)).collect();
            Ok((disk::resolve(&dir), disk::resolve(&root), disabled))
        })
        .await;
        let Ok((dir, root, disabled)) = resolved else {
            return pool;
        };
        let _ = self.disabled_repos.set(disabled);
        {
            let mut spellings = lock(&self.spellings);
            spellings.insert(pool.repo_dir.clone(), dir.clone());
            spellings.insert(pool.repo_root.clone(), root.clone());
        }
        (pool.repo_dir, pool.repo_root) = (dir, root);
        pool
    }
}
