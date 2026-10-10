//! Ruling C-27 (I-2): profile verification's commands run as run steps do. Each waits
//! for a grant of the daemon's [`TestScheduler`] at `Priority::Verify` (decision 23:
//! the lowest class, asking for half the slots), holds it for that one command, and
//! runs in a fresh `<base>/<name>` with decision 28's `TMPDIR`, `ANTHREX_SOCKET` and
//! `ANTHREX_DATA_DIR` after decision 26's slot variables (`run_isolated_with`). So a
//! verified command never sees the daemon's own socket or data directory, confined or
//! not.
//!
//! Ruling C-28 (2): the grant is awaited against the job's cancel token, so a rejected
//! verification stops waiting at once (dropping its request gives up its place), and
//! every command after it is refused unrun.
//!
//! Milestone 9.10 decision 10: a counted `Steps` adds one to its [`CheckCounter`]'s
//! `done` and to the service's shared `ticks` after each command it was asked to run,
//! run or refused, so the set-up's `checking commands (2/4)` moves and the snapshot is
//! pushed. Nothing waits on the counter; it is read with `Ordering::Relaxed`.
//!
//! Blocking: called from `verify::run_commands`, which runs on `spawn_blocking`. The
//! grant is awaited with the runtime's `Handle::block_on` on that blocking thread,
//! never on a worker thread, and under no lock (AGENTS.md rule 2).

use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use regex::Regex;
use tokio_util::sync::CancellationToken;

use crate::launch::shell_quote;
use crate::run::confine::ConfineSpec;
use crate::run::driver::tier_step::{StepCommand, run_isolated_with, step_base};
use crate::run::exec::ShellOutcome;
use crate::run::git;
use crate::run::slots::{Priority, SlotRequest, TestScheduler, Want};

/// The most of a graph command's stdout that is read (as `verify_tiers::capture`).
const CAPTURE_BYTES_MAX: u64 = 16 * 1024 * 1024;

/// Decision 10: how far one verification has got. `total` is `verify::planned`'s
/// answer, set before the commands run; `done` counts the commands run so far (read as
/// `done.min(total)`); `ticks` is the service's `progress_ticks`, shared by every
/// counter, so it never goes back when a job ends.
#[derive(Debug, Default)]
pub struct CheckCounter {
    pub done: AtomicU32,
    pub total: AtomicU32,
    pub ticks: Arc<AtomicU64>,
}

impl CheckCounter {
    /// `Some(done/total)` once `total` is set, `done` capped at `total`.
    pub fn progress(&self) -> Option<proto::CheckProgress> {
        let total = self.total.load(Ordering::Relaxed);
        let done = self.done.load(Ordering::Relaxed).min(total);
        (total > 0).then_some(proto::CheckProgress { done, total })
    }
}

/// Where one verification's commands take their slots and directories.
pub struct Steps {
    pub sched: Arc<TestScheduler>,
    /// The runtime the grants are awaited on, from a blocking thread.
    pub handle: tokio::runtime::Handle,
    /// The repository's git common directory, which no step directory may overlap.
    pub common: PathBuf,
    /// The repository's data directory; the checkout's step base is
    /// `step_base(repo_dir, checkout)`, the confinement's own `TMPDIR`.
    pub repo_dir: PathBuf,
    /// The verification's own token: cancelled, no command waits or runs.
    pub cancel: CancellationToken,
    /// Numbers each command's directory, `v<n>`.
    next: AtomicU32,
    /// Decision 10: counts each command, when set ([`Self::counted`]).
    counter: Option<Arc<CheckCounter>>,
}

impl Steps {
    pub fn new(
        sched: Arc<TestScheduler>,
        handle: tokio::runtime::Handle,
        common: PathBuf,
        repo_dir: PathBuf,
        cancel: CancellationToken,
    ) -> Steps {
        Steps {
            sched,
            handle,
            common,
            repo_dir,
            cancel,
            next: AtomicU32::new(0),
            counter: None,
        }
    }

    /// These steps, each command counted on `counter` (decision 10).
    pub fn counted(self, counter: Arc<CheckCounter>) -> Steps {
        Steps {
            counter: Some(counter),
            ..self
        }
    }

    /// One more command done, on the counter and the shared ticks.
    fn count(&self) {
        if let Some(counter) = &self.counter {
            counter.done.fetch_add(1, Ordering::Relaxed);
            // Release: `snapshot_generation`'s Acquire load then sees `done`.
            counter.ticks.fetch_add(1, Ordering::Release);
        }
    }

    /// `command` in `dir` under a `Verify` grant, reporting whether a whole output line
    /// matched `pattern`. A checkout that cannot be confined, or a directory that cannot
    /// be made, fails it unrun.
    pub fn run(
        &self,
        dir: &Path,
        command: &str,
        env: &[(String, String)],
        timeout: Duration,
        pattern: Option<&Regex>,
        confine: Option<&ConfineSpec>,
    ) -> (ShellOutcome, bool) {
        self.run_as(dir, command, (env, timeout), pattern, confine, false)
    }

    /// [`Self::run`] under an exclusive grant when `exclusive` (decision 25: a command
    /// that runs the timing tests with no slot to leave them out, ruling C-28 (3)).
    pub fn run_as(
        &self,
        dir: &Path,
        command: &str,
        (env, timeout): (&[(String, String)], Duration),
        pattern: Option<&Regex>,
        confine: Option<&ConfineSpec>,
        exclusive: bool,
    ) -> (ShellOutcome, bool) {
        let ran = self.run_step(dir, command, (env, timeout), pattern, confine, exclusive);
        self.count();
        ran
    }

    /// [`Self::run_as`]'s command, uncounted.
    fn run_step(
        &self,
        dir: &Path,
        command: &str,
        (env, timeout): (&[(String, String)], Duration),
        pattern: Option<&Regex>,
        confine: Option<&ConfineSpec>,
        exclusive: bool,
    ) -> (ShellOutcome, bool) {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let request = SlotRequest {
            priority: Priority::Verify,
            critical: false,
            want: Want::Half,
            exclusive,
            label: format!("profile verification {n}"),
        };
        let grant = self.handle.block_on(async {
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => None,
                grant = self.sched.acquire(request) => Some(grant),
            }
        });
        let Some(grant) = grant else {
            let reason = "the verification was cancelled".to_string();
            return (ShellOutcome::refused(reason), false);
        };
        let step = StepCommand {
            dir: dir.to_path_buf(),
            command: command.to_string(),
            env: env.to_vec(),
            slots: grant.env(),
            timeout,
            confine: confine.cloned(),
            common: self.common.clone(),
            base: step_base(&self.repo_dir, dir),
            name: format!("v{n}"),
            collect: false,
        };
        let mut matched = false;
        let outcome = run_isolated_with(&step, &mut |line: &str| {
            matched |= pattern.is_some_and(|p| p.is_match(line));
        });
        drop(grant);
        (outcome, matched)
    }

    /// [`Self::run`] with `command`'s stdout sent to a file beside the step directories
    /// (under the private step base, which the confinement may write), read back at
    /// most 16 MiB: a graph's JSON is one line longer than an output line is kept.
    pub fn capture(
        &self,
        dir: &Path,
        command: &str,
        env: &[(String, String)],
        timeout: Duration,
        confine: Option<&ConfineSpec>,
    ) -> (ShellOutcome, Option<String>) {
        let base = match git::private_dir(&self.common, &step_base(&self.repo_dir, dir)) {
            Ok(base) => base,
            Err(error) => {
                self.count();
                return (ShellOutcome::refused(error), None);
            }
        };
        let n = self.next.load(Ordering::Relaxed);
        let file = base.join(format!("anthrex-graph-v{n}.json"));
        let _ = std::fs::remove_file(&file);
        if let Err(error) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
        {
            let reason = format!("cannot create {}: {error}", file.display());
            self.count();
            return (ShellOutcome::refused(reason), None);
        }
        let shell = format!(
            "{{ {command}\n}} > {}",
            shell_quote(&file.to_string_lossy())
        );
        // Counted once, by `run_as`.
        let (outcome, _) = self.run(dir, &shell, env, timeout, None, confine);
        let mut text = String::new();
        let read = std::fs::File::open(&file)
            .and_then(|f| f.take(CAPTURE_BYTES_MAX).read_to_string(&mut text));
        let _ = std::fs::remove_file(&file);
        (outcome, read.ok().map(|_| text))
    }
}
