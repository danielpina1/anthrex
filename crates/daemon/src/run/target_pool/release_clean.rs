//! The release clean's blocking steps (milestone 9.14 decision 8): the size measure,
//! `cargo metadata`, the plan and its `cargo clean` commands, in the outgoing checkout.
//! `TargetPool::release` is async; everything else here is blocking: call it only
//! from `spawn_blocking` (AGENTS.md rule 2).

use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::clean::{CleanPlan, path_packages, plan};
use super::{
    CLEAN_ENV_OVERRIDES, DirId, DirState, POOL_CLEAN_TIMEOUT, POOL_DIR_NAME, POOL_MEASURE_TIMEOUT,
    PoolBook, PoolCtx, ReleaseOutcome, Released, TARGET_DIR_VAR, TargetPool, blocking, disk,
};
use crate::launch::shell_quote;
use crate::lock;
use crate::run::confine::{ConfineSpec, confined};

impl TargetPool {
    /// Ends `checkout`'s lease (decision 8): cleaned when `clean` and the checkout still
    /// exists, else released not-cleaned (C6, C11). The directory's mutex is held
    /// across the clean or wipe (C19).
    pub async fn release(
        &self,
        pool: &PoolCtx,
        confine: Option<&ConfineSpec>,
        checkout: &Path,
        clean: bool,
    ) -> ReleaseOutcome {
        // Task-5 review M8: the one spelling its lease was keyed by.
        let pool = &self.spelled(pool).await;
        let repo = &pool.repo_dir;
        let id = {
            let _table = self.table(repo).await;
            // C24: a checkout's fallback ends with its release.
            lock(&self.fell_back).remove(checkout);
            self.load(repo).await;
            lock(&self.books)
                .get(repo)
                .and_then(|book| held(book, checkout))
        };
        let Some(id) = id else {
            return ReleaseOutcome::NotLeased;
        };
        let root = repo.join(POOL_DIR_NAME);
        let dir_lock = self.dir_lock(&root.join(id.name()));
        let _held = dir_lock.lock().await;
        #[cfg(test)]
        self.pause("cleaning").await;
        let env = lock(&self.envs).remove(checkout).unwrap_or_default();
        let (cap_bytes, checkout_path) = (self.cap_bytes, checkout.to_path_buf());
        let confine = confine.cloned();
        let outcome = blocking(move || {
            Ok(release_dir(
                root,
                id,
                checkout_path,
                env,
                confine,
                cap_bytes,
                clean,
            ))
        })
        .await
        .unwrap_or_else(ReleaseOutcome::NotCleaned);
        let how = match outcome {
            ReleaseOutcome::Cleaned => Released::Cleaned,
            ReleaseOutcome::Wiped(_) => Released::Wiped,
            _ => Released::NotCleaned,
        };
        self.free(repo, checkout, how).await;
        outcome
    }
}

/// The directory `owner` holds in `book`.
fn held(book: &PoolBook, owner: &Path) -> Option<DirId> {
    book.to_file()
        .dirs
        .into_iter()
        .find(|d| d.state == DirState::Leased && d.owner.as_deref() == Some(owner))
        .map(|d| DirId {
            class: d.class,
            k: d.k,
        })
}

/// A release's blocking part: decide, clean or wipe (decision 8).
fn release_dir(
    root: PathBuf,
    id: DirId,
    checkout: PathBuf,
    env: Vec<(String, String)>,
    confine: Option<ConfineSpec>,
    cap_bytes: u64,
    clean: bool,
) -> ReleaseOutcome {
    if std::fs::symlink_metadata(&checkout).is_err() {
        return ReleaseOutcome::NotCleaned("the checkout is gone".to_string());
    }
    if !clean {
        return ReleaseOutcome::NotCleaned("the run was discarded".to_string());
    }
    let name = id.name();
    // Decision 3 (task-5 review M2): the pool directory spelled with its parent
    // resolved, as `confine::plain_dir` spells the grant.
    let dir = match root.canonicalize() {
        Ok(resolved) => resolved.join(&name),
        Err(_) => root.join(&name),
    };
    // C5: the release's own copy of the run's `ConfineSpec` grants the pool directory
    // (and so the metadata directory made inside it); without it `(deny default)`
    // refuses the clean's writes and every confined release would wipe.
    let confine = confine.map(|spec| ConfineSpec {
        target_dir: Some(dir.clone()),
        ..spec
    });
    let job = CleanJob {
        dir,
        checkout,
        env,
        confine,
        cap_bytes,
        timeout: POOL_CLEAN_TIMEOUT,
    };
    let Err(reason) = job.run() else {
        return ReleaseOutcome::Cleaned;
    };
    match disk::wipe(&root, &name) {
        Ok(()) => ReleaseOutcome::Wiped(reason),
        Err(error) => ReleaseOutcome::NotCleaned(format!("{reason}; the wipe failed: {error}")),
    }
}

/// The most of `cargo metadata`'s output the clean reads.
const METADATA_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Distinguishes concurrent metadata directories.
static METADATA_SEQ: AtomicU64 = AtomicU64::new(0);

/// One release's clean.
struct CleanJob {
    /// The pool directory, a plain directory spelled with its parent resolved.
    pub dir: PathBuf,
    pub checkout: PathBuf,
    /// The environment `lease` last recorded for the checkout (empty when unknown: the
    /// daemon's own then applies).
    pub env: Vec<(String, String)>,
    pub confine: Option<ConfineSpec>,
    pub cap_bytes: u64,
    pub timeout: Duration,
}

impl CleanJob {
    /// `Ok` when the directory was cleaned; `Err(reason)` when it must be wiped instead
    /// (decision 8 steps 1–5; the reasons are the Interfaces' exact texts).
    pub(super) fn run(&self) -> Result<(), String> {
        let deadline = Instant::now() + self.timeout;
        match std::fs::symlink_metadata(&self.dir) {
            Ok(meta) if meta.is_dir() => {}
            _ => return Err("not a plain directory".to_string()),
        }
        match measure(&self.dir, self.cap_bytes, POOL_MEASURE_TIMEOUT) {
            None => return Err("timed out".to_string()),
            Some(size) if size > self.cap_bytes => return Err("over the size cap".to_string()),
            Some(_) => {}
        }
        let json = self.metadata(deadline)?;
        let packages =
            path_packages(&json).map_err(|error| format!("cargo metadata failed: {error}"))?;
        let entries = entries(&self.dir).map_err(|_| "not a plain directory".to_string())?;
        let commands = match plan(&packages, &entries) {
            CleanPlan::Wipe(reason) => return Err(reason),
            CleanPlan::Commands(commands) => commands,
        };
        for command in commands {
            self.shell(&command, deadline)
                .map_err(|line| failed("cargo clean", line))?;
        }
        Ok(())
    }

    /// The environment overrides of decision 8 (C12): the rustc wrappers emptied,
    /// `CARGO_TARGET_DIR` the pool directory, and `RUSTUP_TOOLCHAIN` when the recorded
    /// environment does not name one but the daemon's does.
    fn extra(&self) -> Vec<(String, String)> {
        let mut extra: Vec<(String, String)> = CLEAN_ENV_OVERRIDES
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        extra.push((TARGET_DIR_VAR.to_string(), self.dir.display().to_string()));
        let named = self.env.iter().any(|(k, _)| k == "RUSTUP_TOOLCHAIN");
        if !named && let Ok(toolchain) = std::env::var("RUSTUP_TOOLCHAIN") {
            extra.push(("RUSTUP_TOOLCHAIN".to_string(), toolchain));
        }
        extra
    }

    /// Runs `command` in the checkout; `Err` holds the first line of its output, or
    /// "timed out" (which the caller's prefix keeps as the wipe reason's cause).
    fn shell(&self, command: &str, deadline: Instant) -> Result<(), String> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(TIMED_OUT.to_string());
        }
        let extra = self.extra();
        let outcome = confined(
            &self.checkout,
            command,
            &self.env,
            &extra,
            left,
            self.confine.as_ref(),
        );
        if outcome.timed_out {
            return Err(TIMED_OUT.to_string());
        }
        if outcome.ok {
            Ok(())
        } else {
            Err(outcome.tail.lines().next().unwrap_or("").to_string())
        }
    }

    /// `cargo metadata --format-version 1 --offline --locked`, written into a directory
    /// this call creates inside the pool directory (so no link can be planted at the
    /// path), read without following a link, then removed.
    fn metadata(&self, deadline: Instant) -> Result<String, String> {
        let seq = METADATA_SEQ.fetch_add(1, Ordering::Relaxed);
        let holder = self
            .dir
            .join(format!(".anthrex-metadata-{}-{seq}", std::process::id()));
        std::fs::create_dir(&holder).map_err(|error| format!("cargo metadata failed: {error}"))?;
        let out = holder.join("metadata.json");
        let command = format!(
            "cargo metadata --format-version 1 --offline --locked > {}",
            shell_quote(&out.display().to_string())
        );
        let ran = self.shell(&command, deadline);
        let read = read_no_follow(&out);
        let _ = std::fs::remove_dir_all(&holder);
        ran.map_err(|line| failed("cargo metadata", line))?;
        read.map_err(|error| format!("cargo metadata failed: {error}"))
    }
}

/// The wipe reason of a sequence that ran out of `POOL_CLEAN_TIMEOUT`.
const TIMED_OUT: &str = "timed out";

/// `<what> failed: <line>`, or "timed out" for a sequence out of time.
fn failed(what: &str, line: String) -> String {
    if line == TIMED_OUT {
        line
    } else {
        format!("{what} failed: {line}")
    }
}

fn read_no_follow(path: &Path) -> io::Result<String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut text = String::new();
    file.take(METADATA_MAX_BYTES).read_to_string(&mut text)?;
    Ok(text)
}

/// The bytes under `dir` (links not followed), counting stops once past `cap`. `None`
/// when the walk takes longer than `timeout`: the walking thread is then abandoned, not
/// stopped (the `bounded` pattern; C33, recorded under "Risks").
pub(super) fn measure(dir: &Path, cap: u64, timeout: Duration) -> Option<u64> {
    let (tx, rx) = mpsc::channel();
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(walk(&dir, cap));
    });
    rx.recv_timeout(timeout).ok()
}

fn walk(dir: &Path, cap: u64) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
            if total > cap {
                return total;
            }
        }
    }
    total
}

/// The pool directory's subdirectories of depth ≤ 2 (links not followed), as relative
/// paths (`debug`, `debug/.fingerprint`, …), sorted.
pub(super) fn entries(dir: &Path) -> io::Result<Vec<String>> {
    let mut found = Vec::new();
    for top in std::fs::read_dir(dir)? {
        let top = top?;
        let Some(name) = top.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !std::fs::symlink_metadata(top.path())?.is_dir() {
            continue;
        }
        found.push(name.clone());
        let Ok(children) = std::fs::read_dir(top.path()) else {
            continue;
        };
        for child in children.flatten() {
            let is_dir = std::fs::symlink_metadata(child.path()).is_ok_and(|m| m.is_dir());
            if let (true, Some(child)) = (is_dir, child.file_name().to_str()) {
                found.push(format!("{name}/{child}"));
            }
        }
    }
    found.sort();
    Ok(found)
}
