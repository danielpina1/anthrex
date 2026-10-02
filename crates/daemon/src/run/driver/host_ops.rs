//! Milestone 9.2 decision 8 (task M9.2.12), I/O: `OpKind::Host` executed. Every
//! [`CodeHost`] call runs on `spawn_blocking` within decision 9's bound, never under the
//! engine lock or the manager lock (AGENTS.md rules 2 and 10): `Push`, `Fetch` (with its
//! adoption's compare-and-swap) and `DeleteBranch` through `GitQueue::write`, keyed by
//! the run's project like every engine git write; every other op on `spawn_blocking`
//! directly. Before a push, fetch or delete, the remote's URLs are checked against the
//! seal preflight recorded (the controller's ruling): a changed remote is refused, never
//! followed. Every answer is an `OpResult::Host`: a timeout is `TimedOut`, retried when
//! next due (decision 9); a panicking host (a `FakeGh` asked to land something) is a
//! bug, answered `Forbidden`, which halts the run and is never retried. A large result
//! stays out of the journal: a CI log is written to `<data_dir>/delivery/ci-<run>.log`
//! (0600) and answered by its path, and a PR body is written to
//! `<data_dir>/delivery/pr-<stage>.md` (0600) before `gh pr create` reads it.

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{OpCtx, RunService};
use crate::host::{
    CodeHost, DeleteBranchReq, FetchReq, HOST_READ_TIMEOUT, HOST_WRITE_TIMEOUT, HostError,
    HostRepo, LOG_TIMEOUT, OpenPrReq, PUSH_TIMEOUT, PushReq, ReplyReq,
};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::OpResult;
use crate::run::engine::delivery::op_name;
use crate::run::git::GitQueue;

/// Room above an op's own commands' bounds, so theirs fire first.
const MARGIN: Duration = Duration::from_secs(5);

/// The directory under a run's data directory that holds its CI logs and PR bodies.
pub(crate) const DELIVERY_DIR: &str = "delivery";

/// What the executor needs beside the op; a test builds one without a service.
pub(crate) struct HostExec {
    pub host: Arc<dyn CodeHost>,
    pub queue: Arc<GitQueue>,
    pub git: OsString,
    /// A test seam: caps every op's bound. `None` in the daemon.
    pub cap: Option<Duration>,
}

/// Where an op's work goes: the run, its project (the queue's key), its data directory,
/// and the remote's seal from preflight.
pub(crate) struct HostAt {
    pub run_id: String,
    pub project: PathBuf,
    pub data_dir: PathBuf,
    pub seal: Option<String>,
}

/// Decision 9: an op's bound, the sum of its commands' own (the seal's two reads for a
/// push, fetch or delete), plus [`MARGIN`].
pub(crate) fn bound(op: &HostOp) -> Duration {
    let (r, w, p, l) = (
        HOST_READ_TIMEOUT,
        HOST_WRITE_TIMEOUT,
        PUSH_TIMEOUT,
        LOG_TIMEOUT,
    );
    MARGIN
        + match op {
            HostOp::Push { .. } | HostOp::DeleteBranch { .. } => p + r * 2,
            // The fetch, its `rev-parse`, `merge-base`, swap and `rev-list`, the seal.
            HostOp::Fetch { .. } => p + r * 6,
            HostOp::OpenPr { .. } => r + w,
            HostOp::ViewPr { .. } => r * 2,
            HostOp::FailedLogs { .. } => l,
            HostOp::RerunFailed { .. } | HostOp::Retarget { .. } => w,
            HostOp::Reply { .. } => r * 2 + w,
            HostOp::Permission { .. } => r,
        }
}

/// A push, fetch or delete writes to the remote and to local refs: through the queue.
fn writes(op: &HostOp) -> bool {
    matches!(
        op,
        HostOp::Push { .. } | HostOp::Fetch { .. } | HostOp::DeleteBranch { .. }
    )
}

impl RunService {
    /// `OpKind::Host { repo, op }` of `ctx`'s run. The seal is read from the engine
    /// state, the lock released at once.
    pub(super) async fn host_op(&self, ctx: &OpCtx, repo: HostRepo, op: HostOp) -> OpResult {
        let seal = crate::lock(&self.state)
            .runs
            .get(&ctx.run_id)
            .and_then(|run| run.delivery.remote_seal.clone());
        let exec = HostExec {
            host: self.ctx.host.clone(),
            queue: self.queue.clone(),
            git: self.ctx.git.clone(),
            cap: self.ctx.host_cap,
        };
        let at = HostAt {
            run_id: ctx.run_id.clone(),
            project: ctx.project.clone(),
            data_dir: ctx.data_dir.clone(),
            seal,
        };
        execute(&exec, &at, repo, op).await
    }
}

/// Decision 8: one host op, answered within its bound.
pub(crate) async fn execute(exec: &HostExec, at: &HostAt, repo: HostRepo, op: HostOp) -> OpResult {
    let limit = exec.cap.map_or(bound(&op), |cap| cap.min(bound(&op)));
    let (name, queued) = (op_name(&op), writes(&op));
    let (host, git) = (exec.host.clone(), exec.git.clone());
    let (run_id, data_dir, seal) = (at.run_id.clone(), at.data_dir.clone(), at.seal.clone());
    let work = move || {
        if queued && let Err(error) = unchanged(&git, &repo, seal.as_deref()) {
            return HostResult::Error(error);
        }
        guarded(|| call(&*host, &run_id, &data_dir, repo.clone(), op.clone()))
    };
    let lost = |why: String| HostResult::Error(HostError::Forbidden(panicked(&why)));
    let answer = if queued {
        let (queue, project) = (exec.queue.clone(), at.project.clone());
        let write = async move { queue.write(&project, move || Ok(work())).await };
        tokio::time::timeout(limit, write)
            .await
            .map(|r| r.unwrap_or_else(lost))
    } else {
        tokio::time::timeout(limit, tokio::task::spawn_blocking(work))
            .await
            .map(|r| r.unwrap_or_else(|e| lost(e.to_string())))
    };
    OpResult::Host(answer.unwrap_or_else(|_| {
        HostResult::Error(HostError::TimedOut(format!(
            "{name} did not answer within {} s",
            limit.as_millis().div_ceil(1000)
        )))
    }))
}

/// The text of a host call that did not return: a bug (the ruling: never retried).
fn panicked(why: &str) -> String {
    tracing::error!(%why, "a host call panicked");
    format!("the host call panicked: {why}")
}

/// `f`, its panic caught and answered `Forbidden` (see the module doc).
fn guarded(f: impl FnOnce() -> HostResult) -> HostResult {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let why = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("a panic");
        HostResult::Error(HostError::Forbidden(panicked(why)))
    })
}

/// The controller's ruling: the remote's URLs are still the ones preflight sealed. A run
/// with no seal (none is recorded before `run start` preflights) is not checked.
fn unchanged(git: &OsString, repo: &HostRepo, seal: Option<&str>) -> Result<(), HostError> {
    let Some(sealed) = seal else {
        return Ok(());
    };
    let now = super::delivery::seal(git, &repo.root, &repo.remote).map_err(|e| {
        HostError::Failed(format!("cannot read remote {}'s URLs: {e}", repo.remote))
    })?;
    if now == sealed {
        return Ok(());
    }
    Err(HostError::Forbidden(format!(
        "remote {}'s URLs changed since the run started; anthrex pushes and fetches only where preflight checked (restore remote.{}.url and pushurl, then anthrex run resume)",
        repo.remote, repo.remote
    )))
}

/// The run's `delivery` directory, 0700.
fn private_dir(dir: &Path) -> Result<(), HostError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| HostError::Failed(format!("cannot create {}: {e}", dir.display())))
}

/// Writes `bytes` to `path`, 0600, in its 0700 directory.
fn private_file(path: &Path, bytes: &[u8]) -> Result<(), HostError> {
    let failed =
        |e: std::io::Error| HostError::Failed(format!("cannot write {}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(failed)?;
    file.write_all(bytes).map_err(failed)?;
    file.sync_all().map_err(failed)
}

/// The one host call of `op` (blocking).
fn call(
    host: &dyn CodeHost,
    run_id: &str,
    data_dir: &Path,
    repo: HostRepo,
    op: HostOp,
) -> HostResult {
    let run_id = run_id.to_string();
    let files = data_dir.join(DELIVERY_DIR);
    let result = match op {
        HostOp::Push { stage, sha } => host
            .push(&PushReq {
                repo,
                run_id,
                stage,
                sha,
            })
            .map(HostResult::Pushed),
        HostOp::Fetch {
            branch,
            into,
            adopt,
            parents_of,
            ..
        } => host
            .fetch(&FetchReq {
                repo,
                run_id,
                branch,
                into,
                adopt,
                parents_of,
            })
            .map(HostResult::Fetched),
        HostOp::OpenPr {
            stage,
            base,
            head,
            title,
            body,
        } => {
            let body_file = files.join(format!("pr-{stage}.md"));
            private_file(&body_file, body.as_bytes()).and_then(|()| {
                host.open_pr(&OpenPrReq {
                    repo,
                    run_id,
                    base,
                    head,
                    title,
                    body_file,
                })
                .map(HostResult::PrOpened)
            })
        }
        HostOp::ViewPr { number, .. } => host
            .view_pr(&repo, number)
            .map(|view| HostResult::PrViewed(Box::new(view))),
        HostOp::FailedLogs {
            ci_run, max_bytes, ..
        } => private_dir(&files).and_then(|()| {
            let out = files.join(format!("ci-{ci_run}.log"));
            host.failed_logs(&repo, ci_run, max_bytes, &out)
                .map(HostResult::Logs)
        }),
        HostOp::RerunFailed { ci_run, .. } => {
            host.rerun_failed(&repo, ci_run).map(|()| HostResult::Rerun)
        }
        HostOp::Reply {
            number,
            target,
            body,
            marker,
            ..
        } => host
            .reply(&ReplyReq {
                repo,
                number,
                target,
                body,
                marker,
            })
            .map(|comment_id| HostResult::Replied { comment_id }),
        HostOp::Retarget { number, base, .. } => host
            .retarget(&repo, number, &base)
            .map(|()| HostResult::Retargeted),
        HostOp::Permission { user } => host
            .permission(&repo, &user)
            .map(|permission| HostResult::Permission { user, permission }),
        HostOp::DeleteBranch { stage } => host
            .delete_branch(&DeleteBranchReq {
                repo,
                run_id,
                stage,
            })
            .map(|()| HostResult::Deleted),
    };
    result.unwrap_or_else(HostResult::Error)
}

#[cfg(test)]
#[path = "host_ops_tests.rs"]
pub(super) mod tests;
