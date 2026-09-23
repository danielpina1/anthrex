//! Decision 18's per-repository write queue. Every engine git **write** runs through
//! [`GitQueue::write`]: it takes the repository's own `tokio::sync::Mutex` (so two writes
//! to one repository never overlap, while writes to two repositories do), then runs the
//! blocking closure on `spawn_blocking` (AGENTS.md rule 2), retrying a failure that
//! reads like another git process holding a `.lock` file after each of
//! [`LOCK_RETRY_DELAYS_MS`]. Reads (`rev-parse`, `status`, `diff`, `merge-tree`) do not
//! come here.
//!
//! This is the only `async` function under `run/git/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Decision 18: a lock-file failure is retried up to five times, after these delays.
pub const LOCK_RETRY_DELAYS_MS: [u64; 5] = [200, 400, 800, 1600, 3200];

/// One write queue per repository, keyed by the `project` path the caller passes.
#[derive(Default)]
pub struct GitQueue {
    repos: std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl GitQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `f` on `spawn_blocking` while holding `repo`'s write lock, for the whole
    /// write including its retries. `f` is `Fn`, not `FnOnce` as the brief's interface
    /// block has it: a write that is retried is called again (Implementation notes,
    /// M8a.8).
    pub async fn write<T, F>(&self, repo: &Path, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Fn() -> Result<T, String> + Send + Sync + 'static,
    {
        let repo_lock = crate::lock(&self.repos)
            .entry(repo.to_path_buf())
            .or_default()
            .clone();
        let _guard = repo_lock.lock().await;
        let f = Arc::new(f);
        let mut retries = LOCK_RETRY_DELAYS_MS.iter();
        loop {
            let attempt = Arc::clone(&f);
            let result = tokio::task::spawn_blocking(move || attempt())
                .await
                .map_err(|err| format!("a git write did not finish: {err}"))?;
            match result {
                Err(message) if is_lock_contention(&message) => match retries.next() {
                    Some(delay) => tokio::time::sleep(Duration::from_millis(*delay)).await,
                    None => return Err(message),
                },
                other => return other,
            }
        }
    }
}

/// Decision 18: stderr naming `.lock': File exists`, or `Unable to create` together
/// with `.lock`.
fn is_lock_contention(message: &str) -> bool {
    message.contains(".lock': File exists")
        || (message.contains("Unable to create") && message.contains(".lock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn writes_to_one_repo_are_serialized() {
        let queue = Arc::new(GitQueue::new());
        let spans: Arc<Mutex<Vec<(Instant, Instant)>>> = Arc::default();
        let repo = Path::new("/tmp/ax-queue-one");
        let mut handles = Vec::new();
        for _ in 0..2 {
            let queue = queue.clone();
            let spans = spans.clone();
            handles.push(tokio::spawn(async move {
                queue
                    .write(repo, move || {
                        let start = Instant::now();
                        std::thread::sleep(Duration::from_millis(200));
                        crate::lock(&spans).push((start, Instant::now()));
                        Ok(())
                    })
                    .await
            }));
        }
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
        let mut spans = crate::lock(&spans).clone();
        spans.sort();
        assert_eq!(spans.len(), 2);
        assert!(
            spans[0].1 <= spans[1].0,
            "the second write started before the first ended: {spans:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn writes_to_two_repos_run_concurrently() {
        let queue = Arc::new(GitQueue::new());
        let (to_b, from_a) = mpsc::channel::<()>();
        let (to_a, from_b) = mpsc::channel::<()>();
        let rendezvous = |tx: mpsc::Sender<()>, rx: mpsc::Receiver<()>| {
            let rx = Mutex::new(rx);
            move || {
                tx.send(()).map_err(|e| e.to_string())?;
                crate::lock(&rx)
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|e| format!("the other write never ran alongside: {e}"))
            }
        };
        let a = {
            let queue = queue.clone();
            let f = rendezvous(to_b, from_b);
            tokio::spawn(async move { queue.write(Path::new("/tmp/ax-queue-a"), f).await })
        };
        let b = {
            let queue = queue.clone();
            let f = rendezvous(to_a, from_a);
            tokio::spawn(async move { queue.write(Path::new("/tmp/ax-queue-b"), f).await })
        };
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lock_errors_are_retried_then_surface() {
        let queue = GitQueue::new();
        let repo = Path::new("/tmp/ax-queue-retry");
        let lock_error = "fatal: Unable to create '/x/.git/index.lock': File exists.";

        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let result = queue
            .write(repo, move || {
                if counter.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(lock_error.to_string())
                } else {
                    Ok(7)
                }
            })
            .await;
        assert_eq!(result, Ok(7));
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let result: Result<(), String> = queue
            .write(repo, move || {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                Err(format!("{lock_error} (attempt {n})"))
            })
            .await;
        assert_eq!(result, Err(format!("{lock_error} (attempt 5)")));
        assert_eq!(calls.load(Ordering::SeqCst), 6);

        // Any other failure is not retried.
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let result: Result<(), String> = queue
            .write(repo, move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Err("fatal: invalid reference: nope".to_string())
            })
            .await;
        assert_eq!(result, Err("fatal: invalid reference: nope".to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
