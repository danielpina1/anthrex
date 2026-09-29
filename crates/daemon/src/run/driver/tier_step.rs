//! The tier executor's steps (milestone 9.1 decisions 26, 28, 32 and 33), split out of
//! `tier.rs` for the 600-line rule as task M9.1.9 foresaw: one command of a tier step,
//! a bisect probe or a scratch `setup`, run in its checkout with a fresh private
//! `TMPDIR` of its own and the daemon's coordinates pointed into it, its whole output
//! read line by line for failing-test names ([`run_isolated`], blocking: call only from
//! `spawn_blocking`, AGENTS.md rule 2); a job's shared parts ([`Job`]: the slot wait,
//! the bounded blocking call, M8a's scratch checkout); and a step with decision 33's
//! retry as ruling C-7 amends it ([`run_step`]).
//!
//! Decision 28: the directory is `<base>/s<op>-<step>`, where `<base>` is the checkout
//! repository's own temporary directory (`Repo::tmp()`, M8a's), the confinement's
//! writable `TMPDIR` when confined and the same path, made with M8a's `private_dir`,
//! when not. It is never inside the checkout. It is made 0700 before the command and
//! removed after it. `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` point under it on every
//! command, confined or not: unconfined (Linux with `--unconfined-checks`) the daemon's
//! own `ANTHREX_SOCKET` would otherwise reach a test that starts a daemon.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::OpCtx;
use crate::run::confine::ConfineSpec;
use crate::run::engine::{OpResult, ScratchAt};
use crate::run::exec::{OUTPUT_GRACE, ShellOutcome, run_lines};
use crate::run::git::{self, Git, GitQueue};
use crate::run::model::OpId;
use crate::run::proof::{SETUP_MARKER, proof_command};
use crate::run::slots::{Priority, SlotGrant, SlotRequest, TestScheduler, Want};
use crate::run::tiers::failing::names;
use crate::run::tiers::{RETRY_NAMES_MAX, Step, StepKind, StepOutcome, TierSpec};

/// The most output bytes a step keeps for failing-test names; past it the step has no
/// names (decision 32: a list that may be incomplete is none), so it is retried whole.
pub const OUTPUT_BYTES_MAX: usize = 16 * 1024 * 1024;

/// The three variables decision 28 sets on every command, for the directory `tmp`.
pub fn isolation_env(tmp: &Path) -> Vec<(String, String)> {
    let at = |leaf: &str| tmp.join(leaf).display().to_string();
    vec![
        ("TMPDIR".to_string(), tmp.display().to_string()),
        ("ANTHREX_SOCKET".to_string(), at("d.sock")),
        ("ANTHREX_DATA_DIR".to_string(), at("data")),
    ]
}

/// `<base>` for the checkout `dir` of a run whose data directory is `data_dir`: the
/// checkout repository's temporary directory (`git/tmp.rs`).
pub fn step_base(data_dir: &Path, dir: &Path) -> PathBuf {
    git::task_tmp(&git::checkout_repo_dir(data_dir, dir))
}

/// One command to run in its own directory.
#[derive(Debug, Clone)]
pub struct StepCommand {
    /// The checkout.
    pub dir: PathBuf,
    pub command: String,
    /// The profile's `env`.
    pub env: Vec<(String, String)>,
    /// Decision 26's slot variables; the isolation variables are added after them.
    pub slots: Vec<(String, String)>,
    pub timeout: Duration,
    pub confine: Option<ConfineSpec>,
    /// The repository's git common directory, which `base` must not overlap.
    pub common: PathBuf,
    /// [`step_base`].
    pub base: PathBuf,
    /// The directory's name under `base`: `s<op>-<step>`.
    pub name: String,
}

/// What one command did.
#[derive(Debug, Clone)]
pub struct StepRun {
    pub outcome: ShellOutcome,
    /// Every output line, or `None` when they passed [`OUTPUT_BYTES_MAX`].
    pub lines: Option<Vec<String>>,
}

/// Runs `step` in a fresh `<base>/<name>`, removed afterwards. A directory that cannot
/// be made, or a checkout that cannot be confined, fails the command unrun.
pub fn run_isolated(step: &StepCommand) -> StepRun {
    let refused = |reason: String| StepRun {
        outcome: ShellOutcome::refused(reason),
        lines: Some(Vec::new()),
    };
    let tmp = match fresh_dir(&step.common, &step.base, &step.name) {
        Ok(tmp) => tmp,
        Err(error) => return refused(error),
    };
    let confinement = match step.confine.as_ref().map(|c| c.for_checkout(&step.dir)) {
        Some(Err(error)) => {
            remove_dir(&tmp);
            return refused(error);
        }
        Some(Ok(confinement)) => Some(confinement),
        None => None,
    };
    let mut extra = step.slots.clone();
    extra.extend(isolation_env(&tmp));
    let mut lines = Some(Vec::new());
    let mut bytes = 0usize;
    let mut on_line = |line: &str| {
        bytes = bytes.saturating_add(line.len() + 1);
        if bytes > OUTPUT_BYTES_MAX {
            lines = None;
        } else if let Some(lines) = lines.as_mut() {
            lines.push(line.to_string());
        }
    };
    let outcome = run_lines(
        &step.dir,
        &step.command,
        (&step.env, &extra),
        step.timeout,
        confinement.as_ref(),
        &mut on_line,
    );
    remove_dir(&tmp);
    StepRun { outcome, lines }
}

/// `<base>/<name>`, made now (mode 0700) under the private `base`: whatever an earlier
/// command left there, a link included, is removed first without being followed.
pub fn fresh_dir(common: &Path, base: &Path, name: &str) -> Result<PathBuf, String> {
    let base = git::private_dir(common, base)?;
    let dir = base.join(name);
    remove_dir(&dir);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!("{} is not a plain directory", dir.display()));
    }
    Ok(dir)
}

/// Removes `path` without following a link there; a failure is logged.
pub fn remove_dir(path: &Path) {
    let result = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            if let Err(error) = git::restore_owner_access(path) {
                tracing::debug!(%error, "could not open a step directory up for removal");
            }
            std::fs::remove_dir_all(path)
        }
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        tracing::debug!(%error, path = %path.display(), "could not remove a step directory");
    }
}

/// Added to a blocking call's own bound before the executor stops waiting for it.
pub(super) const SLACK: Duration = Duration::from_secs(30);

/// Why a job stopped before its outcome: the op's result (boxed, as clippy's
/// `result_large_err` asks).
pub(super) type Stop = Box<OpResult>;

pub(super) fn stop(message: impl Into<String>) -> Stop {
    Box::new(failed(message))
}

pub(super) fn failed(message: impl Into<String>) -> OpResult {
    OpResult::Failed {
        message: message.into(),
    }
}

/// `f` on a blocking thread, abandoned (not killed: its own bounds end it) after `bound`.
pub(super) async fn bounded<T: Send + 'static>(
    bound: Duration,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match tokio::time::timeout(bound, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("a blocking step did not finish: {error}")),
        Err(_) => Err(format!(
            "a blocking step did not finish within {} s",
            bound.as_secs()
        )),
    }
}

/// Decision 24: what a step of each class asks for.
fn want(priority: Priority) -> Want {
    match priority {
        Priority::Gate | Priority::Verify => Want::Half,
        Priority::Candidate | Priority::FullStage | Priority::FullIdle => Want::All,
    }
}

/// A setup failure's output, with a note when it timed out (M8a's `SetupFailed`).
fn setup_output(outcome: &ShellOutcome) -> String {
    if outcome.timed_out {
        format!(
            "{}\n[anthrex: timed out after {} s]",
            outcome.tail, outcome.secs
        )
    } else {
        outcome.tail.clone()
    }
}

/// What every command of one job shares.
pub(super) struct Job<'a> {
    pub(super) ctx: &'a OpCtx,
    pub(super) sched: &'a TestScheduler,
    pub(super) queue: &'a GitQueue,
    pub(super) git: OsString,
    pub(super) op: OpId,
    pub(super) root: PathBuf,
    pub(super) dir: PathBuf,
    pub(super) env: Vec<(String, String)>,
    pub(super) timeout: Duration,
    pub(super) priority: Priority,
    pub(super) critical: bool,
    /// The repository's git common directory.
    pub(super) common: PathBuf,
    /// Decision 28's `<base>`.
    pub(super) base: PathBuf,
}

/// A job's fixed parts, before the common directory is read.
pub(super) struct JobSpec<'a> {
    pub(super) root: &'a Path,
    pub(super) dir: &'a Path,
    pub(super) env: &'a [(String, String)],
    pub(super) timeout_secs: u64,
    pub(super) priority: Priority,
    pub(super) critical: bool,
}

impl<'a> Job<'a> {
    pub(super) async fn new(
        ctx: &'a OpCtx,
        (sched, queue, git): (&'a TestScheduler, &'a GitQueue, &OsStr),
        op: OpId,
        spec: JobSpec<'_>,
    ) -> Result<Job<'a>, String> {
        let common = match ctx.confine.as_deref() {
            Some(confine) => confine.common_dir.clone(),
            None => {
                let (g, root, t) = (git.to_os_string(), spec.root.to_path_buf(), ctx.git_timeout);
                bounded(t + SLACK, move || git::common_dir(Git::new(&g, t), &root)).await?
            }
        };
        Ok(Job {
            ctx,
            sched,
            queue,
            git: git.to_os_string(),
            op,
            root: spec.root.to_path_buf(),
            dir: spec.dir.to_path_buf(),
            env: spec.env.to_vec(),
            timeout: Duration::from_secs(spec.timeout_secs),
            priority: spec.priority,
            critical: spec.critical,
            common,
            base: step_base(&ctx.data_dir, spec.dir),
        })
    }

    /// Waits (async, under no lock) for a grant.
    pub(super) async fn acquire(&self, want: Want, exclusive: bool, label: String) -> SlotGrant {
        let req = SlotRequest {
            priority: self.priority,
            critical: self.critical,
            want,
            exclusive,
            label,
        };
        self.sched.acquire(req).await
    }

    /// `command` in a fresh `<base>/s<op>-<step>`, holding `grant`.
    pub(super) async fn run(
        &self,
        step: &str,
        command: &str,
        grant: &SlotGrant,
    ) -> Result<StepRun, String> {
        let command = StepCommand {
            dir: self.dir.clone(),
            command: command.to_string(),
            env: self.env.clone(),
            slots: grant.env(),
            timeout: self.timeout,
            confine: self.ctx.confine.as_deref().cloned(),
            common: self.common.clone(),
            base: self.base.clone(),
            name: format!("s{}-{step}", self.op),
        };
        let bound = self.timeout + OUTPUT_GRACE * 2 + SLACK;
        bounded(bound, move || Ok(run_isolated(&command))).await
    }

    /// A git read on a blocking thread, with the run's git timeout.
    pub(super) async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&OsStr, Duration) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (g, t) = (self.git.clone(), self.ctx.git_timeout);
        bounded(t * 2 + SLACK, move || f(&g, t)).await
    }

    /// A git write through the run's queue.
    pub(super) async fn write<T: Send + 'static>(
        &self,
        f: impl Fn(&OsStr, Duration) -> Result<T, String> + Send + Sync + 'static,
    ) -> Result<T, String> {
        let (g, t) = (self.git.clone(), self.ctx.git_timeout);
        self.queue.write(&self.ctx.project, move || f(&g, t)).await
    }

    /// M8a's scratch contract (`OpKind::Check`, ruling T13-I3): the scratch checkout at
    /// the commit, `setup` once per new checkout (its marker), then `materialize`.
    pub(super) async fn prepare(&self, scratch: &ScratchAt) -> Result<(), Stop> {
        let ScratchAt {
            root,
            commit,
            setup,
        } = scratch.clone();
        let (at, c) = (self.dir.clone(), commit.clone());
        let repo = git::checkout_repo_dir(&self.ctx.data_dir, &self.dir);
        self.write(move |g, t| git::prepare_scratch_in(g, &root, &at, &c, &repo, t))
            .await
            .map_err(stop)?;
        let at = self.dir.clone();
        let marker = self
            .read(move |g, t| Ok(git::absolute_git_dir(g, &at, t)?.join(SETUP_MARKER)))
            .await
            .map_err(stop)?;
        let probe = marker.clone();
        let exists = bounded(SLACK, move || Ok(probe.exists()))
            .await
            .map_err(stop)?;
        if !exists {
            if let Some(setup) = setup {
                let (at, c) = (self.dir.clone(), commit.clone());
                self.write(move |g, t| git::materialize(g, &at, &c, t))
                    .await
                    .map_err(stop)?;
                let grant = self.acquire(Want::One, false, "setup".to_string()).await;
                let ran = self.run("setup", &setup, &grant).await.map_err(stop)?;
                drop(grant);
                if !ran.outcome.ok {
                    return Err(Box::new(OpResult::SetupFailed {
                        output: setup_output(&ran.outcome),
                    }));
                }
            }
            bounded(SLACK, move || {
                std::fs::write(&marker, "")
                    .map_err(|error| format!("could not write {}: {error}", marker.display()))
            })
            .await
            .map_err(stop)?;
        }
        let at = self.dir.clone();
        self.write(move |g, t| git::materialize(g, &at, &commit, t))
            .await
            .map_err(stop)
    }
}

/// The failing names in a run's whole output (none when it overflowed).
fn names_of(run: StepRun) -> (ShellOutcome, Vec<String>) {
    let found = run
        .lines
        .map(|lines| names(lines.into_iter()))
        .unwrap_or_default();
    (run.outcome, found)
}

/// Step `k` of the job, with decision 33's retry (ruling C-7) when `tiered`. Returns
/// the outcome and, when it is red, its tail.
pub(super) async fn run_step(
    job: &Job<'_>,
    spec: &TierSpec,
    tiered: bool,
    k: usize,
    step: &Step,
) -> Result<(StepOutcome, String), Stop> {
    let at = k.to_string();
    let label = || format!("tier {} step {k}", spec.tier);
    let want = want(spec.priority);
    let grant = job.acquire(want, step.exclusive, label()).await;
    let (first, first_names) = names_of(job.run(&at, &step.command, &grant).await.map_err(stop)?);
    let mut ran = StepOutcome {
        kind: step.kind,
        command: step.command.clone(),
        ok: first.ok,
        code: first.code,
        timed_out: first.timed_out,
        secs: first.secs,
        cached: false,
        retried: false,
        failing: Vec::new(),
        flaky: Vec::new(),
        granted: grant.slots(),
    };
    if first.ok {
        return Ok((ran, String::new()));
    }
    if !tiered || step.kind == StepKind::Build || first.timed_out {
        ran.failing = first_names;
        return Ok((ran, first.tail));
    }
    ran.retried = true;
    let single = spec
        .single_test
        .as_deref()
        .filter(|_| !first_names.is_empty() && first_names.len() <= RETRY_NAMES_MAX);
    let whole = match single {
        Some(single) => {
            // By name, holding the step's grant.
            for name in &first_names {
                let one = job
                    .run(&at, &proof_command(single, name), &grant)
                    .await
                    .map_err(stop)?;
                ran.secs += one.outcome.secs;
                if !one.outcome.ok {
                    ran.failing = first_names;
                    return Ok((ran, first.tail));
                }
            }
            // Ruling C-7: the names are a speed-up, never the verdict.
            job.run(&at, &step.command, &grant).await.map_err(stop)?
        }
        None => {
            drop(grant);
            let grant = job.acquire(want, step.exclusive, label()).await;
            job.run(&at, &step.command, &grant).await.map_err(stop)?
        }
    };
    let (whole, whole_names) = names_of(whole);
    ran.secs += whole.secs;
    ran.ok = whole.ok;
    ran.code = whole.code;
    ran.timed_out = whole.timed_out;
    if whole.ok {
        ran.flaky = first_names;
        return Ok((ran, String::new()));
    }
    ran.failing = if whole_names.is_empty() {
        first_names
    } else {
        whole_names
    };
    Ok((ran, whole.tail))
}
