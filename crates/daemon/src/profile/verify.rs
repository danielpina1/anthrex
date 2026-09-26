//! Decision 9: the engine, not the read-only onboarding scout, runs a proposed profile's
//! `setup`, `check` and `single_test`, each in a **fresh** disposable checkout, under the
//! confinement a run in this repository would get.
//!
//! The checkout is M8a's standalone checkout (final fix batch F1c, 3a):
//! `<wt>/runs/.profile-verify`, detached at `Preflight.base_sha`, whose own repository is
//! `<repo_dir>/tasks/.profile-verify` ([`crate::run::git::checkout_repo_dir`]), borrowing
//! the user's objects as an alternate. No `git worktree add` and no lock, so the user's
//! `.git` gains no `worktrees/` entry. When verification ends the checkout is salvaged if
//! dirty (`refs/anthrex/salvage/onboarding/<unix secs>`, M8a's `salvage`) and removed
//! with its repository (M8a's `remove_checkout`).
//!
//! **Nothing here writes under the repository root.** The commands run with the checkout
//! as their working directory, confined by M8a's `(deny default)` profile to the
//! checkout, its repository's object store, its short `TMPDIR` and the user's own
//! `cache_dirs` for the root; the git steps write only the checkout, its repository in
//! anthrex's data directory, and (a salvage) objects and one ref in the user's `.git`,
//! exactly as M8a's own salvage does.
//!
//! Blocking, except [`verify`], which drives the blocking steps: its git writes go
//! through `GitQueue::write` (which runs them on `spawn_blocking`), and the commands run
//! on `spawn_blocking`. Nothing is done under a lock.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proto::{CommandCheck, ProfileSpec, ProfileVerification, RepoProfile};
use regex::Regex;

use super::VERIFY_CHECKOUT;
use crate::run::confine::{self, ConfineSpec};
use crate::run::env::profile_env;
use crate::run::exec::{ShellOutcome, run_matching};
use crate::run::git::{self, GitQueue, Repo, checkout_repo_dir};
use crate::run::messages::summary;
use crate::run::plan::{Preflight, for_repo, resolve_profile};
use crate::run::proof::{proof_command, proof_pattern};
use crate::worktree::pinned;

/// Where a dirty detection checkout's work is kept (decision 8): `<prefix><unix secs>`.
pub const SALVAGE_PREFIX: &str = "refs/anthrex/salvage/onboarding/";
/// The salvage commit's message.
pub const SALVAGE_MESSAGE: &str = "anthrex salvage onboarding";
/// Further names tried when `<prefix><secs>` already holds other work (two salvages in
/// one second): `<secs>-1` to `<secs>-9`.
const SALVAGE_SUFFIXES: u32 = 9;

/// Decision 9: the confinement verification runs under, `None` when unconfined. Present
/// exactly when the user's workers are sandboxed and this platform can confine. It is
/// built from the user's `[orchestrator.*]` tables for `pre.root`, never from the
/// proposal, with `data_dir = repo_dir`, so
/// [`ConfineSpec::for_checkout`] finds the checkout's repository at
/// `<repo_dir>/tasks/<checkout>` and anthrex's data directory two levels up.
pub fn confine_spec(
    config: &config::Orchestrator,
    repo_dir: &Path,
    pre: &Preflight,
    daemon_socket: &Path,
) -> Option<ConfineSpec> {
    (config.worker_sandbox && confine::available()).then(|| ConfineSpec {
        data_dir: repo_dir.to_path_buf(),
        common_dir: pre.git_common_dir.clone(),
        cache_dirs: for_repo(&config.cache_dirs, &pre.root)
            .cloned()
            .unwrap_or_default(),
        network: for_repo(&config.confined_network, &pre.root)
            .copied()
            .unwrap_or(false),
        unix_sockets: for_repo(&config.confined_unix_sockets, &pre.root)
            .cloned()
            .unwrap_or_default(),
        localhost_ports: for_repo(&config.confined_localhost_ports, &pre.root)
            .cloned()
            .unwrap_or_default(),
        daemon_socket: daemon_socket.to_path_buf(),
    })
}

/// `<wt>/runs/.profile-verify` for the project, `<wt>` its directory under
/// `worktrees_root`.
pub fn checkout_path(worktrees_root: &Path, project: &Path) -> PathBuf {
    crate::worktree::repo_worktrees_dir(worktrees_root, project)
        .join("runs")
        .join(VERIFY_CHECKOUT)
}

/// The standalone checkout at `path`, its repository at `repo`, detached at
/// `pre.base_sha` (M8a's `prepare_scratch_in`). A git write: behind `GitQueue::write`.
pub fn prepare(
    git: &OsStr,
    pre: &Preflight,
    path: &Path,
    repo: &Path,
    timeout: Duration,
) -> Result<(), String> {
    git::prepare_scratch_in(git, &pre.root, path, &pre.base_sha, repo, timeout).map(|_| ())
}

/// One command in `dir`, confined when `confine` is set (a checkout that cannot be
/// confined fails it unrun, as `run::confine::confined` does), also reporting whether a
/// line matched `pattern`.
fn run_one(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
    pattern: Option<&Regex>,
    confine: Option<&ConfineSpec>,
) -> (ShellOutcome, bool) {
    match confine.map(|spec| spec.for_checkout(dir)).transpose() {
        Ok(confinement) => run_matching(dir, command, env, timeout, pattern, confinement.as_ref()),
        Err(error) => (ShellOutcome::refused(error), false),
    }
}

fn record(command: &str, outcome: ShellOutcome, ok: bool) -> CommandCheck {
    CommandCheck {
        command: command.to_string(),
        ok,
        code: outcome.code,
        timed_out: outcome.timed_out,
        secs: outcome.secs,
        tail: summary(&outcome.tail),
    }
}

/// Decision 9's order in the checkout `dir`, each command bounded by `timeout`, with the
/// profile's `env` (`{worktree}` replaced by `dir`) over M8a's engine environment:
/// `setup` (its failure does not stop the rest), `check`, then `single_test` as the
/// proof command for `sample_test`, which passes only when it exits 0 and a line matches
/// `test_passed`. A `single_test` that cannot be verified (no `{test}`, no companions,
/// a pattern that does not compile) is not run; `proposal::apply_verification` says why.
pub fn run_commands(
    dir: &Path,
    profile: &RepoProfile,
    confine: Option<&ConfineSpec>,
    timeout: Duration,
    now: u64,
) -> ProfileVerification {
    let resolved = resolve_profile(
        &ProfileSpec {
            env: Some(profile.env.clone()),
            ..ProfileSpec::default()
        },
        &ProfileSpec::default(),
    );
    let env = profile_env(&resolved, dir);
    let plain = |command: &String| {
        let (outcome, _) = run_one(dir, command, &env, timeout, None, confine);
        let ok = outcome.ok;
        record(command, outcome, ok)
    };
    let setup = profile.setup.as_ref().map(plain);
    let check = profile.check.as_ref().map(plain);
    let single_test = match (
        &profile.single_test,
        &profile.sample_test,
        &profile.test_passed,
    ) {
        (Some(single), Some(sample), Some(passed)) if single.contains("{test}") => {
            Regex::new(&proof_pattern(passed, sample))
                .ok()
                .filter(|_| passed.contains("{test}"))
                .map(|pattern| {
                    let command = proof_command(single, sample);
                    let (outcome, matched) =
                        run_one(dir, &command, &env, timeout, Some(&pattern), confine);
                    let ok = outcome.ok && matched;
                    record(single, outcome, ok)
                })
        }
        _ => None,
    };
    ProfileVerification {
        at: now,
        confined: confine.is_some(),
        setup,
        check,
        single_test,
    }
}

/// Pins a checkout this daemon did not pin (one left by an earlier daemon) as the
/// standalone checkout of `repo`, so the salvage reads and writes its ref in the user's
/// repository, not in the checkout's own git directory.
fn pin_leftover(git: &OsStr, root: &Path, path: &Path, repo: &Path, timeout: Duration) {
    let repo = Repo::at(repo);
    if pinned::pinned(path).is_some() || !repo.git_dir().join("HEAD").is_file() {
        return;
    }
    if let Ok(common) = git::common_dir(git::Git::new(git, timeout), root) {
        pinned::pin(
            &common,
            path,
            pinned::PinAs {
                repo: Some(repo.git_dir()),
                ..Default::default()
            },
        );
    }
}

/// The checkout at `path` salvaged to `salvage_ref` if dirty (M8a's `salvage`), then
/// removed with its repository `repo` (M8a's `remove_checkout`); `Some(ref)` when it
/// was dirty. A salvage that fails keeps the checkout: nothing is deleted dirty. Git
/// writes: behind `GitQueue::write`.
pub fn discard(
    git: &OsStr,
    root: &Path,
    path: &Path,
    repo: &Path,
    salvage_ref: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let salvaged = if path.exists() {
        pin_leftover(git, root, path, repo, timeout);
        git::salvage(git, path, salvage_ref, SALVAGE_MESSAGE, timeout)?
    } else {
        None
    };
    git::remove_checkout(git, root, path, Some(repo), timeout)?;
    Ok(salvaged)
}

/// What [`verify`] needs, owned, so its steps can move to other threads.
#[derive(Debug, Clone)]
pub struct VerifyJob {
    pub git: OsString,
    pub pre: Preflight,
    /// The repository's data directory, `profile::repo_dir(data_dir, pre.project)`.
    pub repo_dir: PathBuf,
    /// The daemon's worktrees root; the checkout is under its `<wt>/runs/`.
    pub worktrees_root: PathBuf,
    pub profile: RepoProfile,
    /// [`confine_spec`]'s answer.
    pub confine: Option<ConfineSpec>,
    /// Each command's bound, `[orchestrator.onboarding] verify_timeout_secs`.
    pub timeout: Duration,
    /// Each git step's bound.
    pub git_timeout: Duration,
}

/// What a verification ran, and every salvage ref it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub verification: ProfileVerification,
    pub salvaged: Vec<String>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// [`discard`] through the project's write queue, under the first salvage name not
/// already holding other work.
async fn discard_queued(
    queue: &GitQueue,
    job: &VerifyJob,
    path: &Path,
    repo: &Path,
) -> Result<Option<String>, String> {
    let (git, root, path, repo, timeout) = (
        job.git.clone(),
        job.pre.root.clone(),
        path.to_path_buf(),
        repo.to_path_buf(),
        job.git_timeout,
    );
    let secs = unix_now();
    queue
        .write(&job.pre.project, move || {
            let mut result = Err(String::new());
            for n in 0..=SALVAGE_SUFFIXES {
                let name = match n {
                    0 => format!("{SALVAGE_PREFIX}{secs}"),
                    n => format!("{SALVAGE_PREFIX}{secs}-{n}"),
                };
                result = discard(&git, &root, &path, &repo, &name, timeout);
                match &result {
                    Err(error) if error.ends_with("already holds other work") => continue,
                    _ => break,
                }
            }
            result
        })
        .await
}

/// Decision 9 end to end: a leftover verification checkout is discarded (salvaged)
/// first, a fresh one is made at `pre.base_sha`, the commands run in it
/// ([`run_commands`]), and it is discarded again whatever happened. An `Err` is a git
/// step that failed; the commands' own failures are in the verification.
pub async fn verify(queue: &GitQueue, job: VerifyJob) -> Result<Verified, String> {
    let path = checkout_path(&job.worktrees_root, &job.pre.project);
    let repo = checkout_repo_dir(&job.repo_dir, &path);
    let mut salvaged = Vec::new();
    if path.exists() || repo.exists() {
        salvaged.extend(discard_queued(queue, &job, &path, &repo).await?);
    }
    let prepared = {
        let (git, pre, p, r, t) = (
            job.git.clone(),
            job.pre.clone(),
            path.clone(),
            repo.clone(),
            job.git_timeout,
        );
        queue
            .write(&job.pre.project, move || prepare(&git, &pre, &p, &r, t))
            .await
    };
    let verification = match prepared {
        Ok(()) => {
            let (dir, profile, confine, timeout) = (
                path.clone(),
                job.profile.clone(),
                job.confine.clone(),
                job.timeout,
            );
            tokio::task::spawn_blocking(move || {
                run_commands(&dir, &profile, confine.as_ref(), timeout, unix_now())
            })
            .await
            .map_err(|error| format!("the verification did not finish: {error}"))
        }
        Err(error) => Err(format!(
            "could not prepare the verification checkout: {error}"
        )),
    };
    let discarded = discard_queued(queue, &job, &path, &repo).await;
    let verification = verification?;
    salvaged.extend(
        discarded
            .map_err(|error| format!("could not remove the verification checkout: {error}"))?,
    );
    Ok(Verified {
        verification,
        salvaged,
    })
}
