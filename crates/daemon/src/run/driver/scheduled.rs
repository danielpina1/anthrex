//! Controller ruling 1 and decision 23 for M8a's own commands (task M9.1.13, ruling
//! C-12b): `setup`, the test proof's runs, the check, the final check and the candidate
//! check each wait for their slots in the daemon's test scheduler, then run isolated
//! (decision 28: a fresh private `TMPDIR` with `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR`
//! under it) with the slot variables (decision 26), confined exactly as before. The
//! wait is async and under no lock; the command runs on a bounded blocking thread; the
//! grant is held for that one command and given back before anything else waits.
//! The command, its directory, its environment, its timeout and its confinement are
//! M8a's, so its result is too (decision 6).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::super::tier_step::{
    SLACK, StepCommand, bounded, fresh_dir, isolation_env, remove_dir, run_isolated, step_base,
};
use super::super::{OpCtx, RunService};
use crate::run::exec::{OUTPUT_GRACE, ShellOutcome};
use crate::run::git::{self, Git};
use crate::run::model::OpId;
use crate::run::slots::{Priority, SlotGrant, SlotRequest, TestScheduler, Want};

/// One M8a command's place in the scheduler: its class, its size and its label.
pub(crate) type Class = (Priority, Want, String);

/// An M8a command of op `op`, to run through the scheduler.
pub(crate) struct Scheduled<'a> {
    service: &'a Arc<RunService>,
    ctx: &'a OpCtx,
    op: OpId,
    class: Class,
}

pub(crate) fn scheduled<'a>(
    service: &'a Arc<RunService>,
    ctx: &'a OpCtx,
    op: OpId,
    class: Class,
) -> Scheduled<'a> {
    Scheduled {
        service,
        ctx,
        op,
        class,
    }
}

/// The request for `class`.
pub(crate) fn request((priority, want, label): &Class) -> SlotRequest {
    SlotRequest {
        priority: *priority,
        critical: false,
        want: *want,
        exclusive: false,
        label: label.clone(),
    }
}

/// The run's git common directory, which a step's directory must not overlap: the
/// confinement's, else read once in the run's project (a bounded blocking git read).
pub(crate) async fn common_dir(ctx: &OpCtx, git: std::ffi::OsString) -> Result<PathBuf, String> {
    match ctx.confine.as_deref() {
        Some(confine) => Ok(confine.common_dir.clone()),
        None => {
            let (root, t) = (ctx.project.clone(), ctx.git_timeout);
            bounded(t + SLACK, move || git::common_dir(Git::new(&git, t), &root)).await
        }
    }
}

impl Scheduled<'_> {
    /// `command` in `dir` (a checkout of the run's repository), once its slots are
    /// granted, in `<base>/s<op>-<step>` where `<step>` is this command's label's first
    /// word.
    pub(crate) async fn run(
        &self,
        dir: &Path,
        command: &str,
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<ShellOutcome, String> {
        let common = common_dir(self.ctx, self.service.git()).await?;
        let grant = self.service.scheduler().acquire(request(&self.class)).await;
        let step = StepCommand {
            dir: dir.to_path_buf(),
            command: command.to_string(),
            env: env.to_vec(),
            slots: grant.env(),
            timeout,
            confine: self.ctx.confine.as_deref().cloned(),
            common,
            base: step_base(&self.ctx.data_dir, dir),
            name: format!("s{}-{}", self.op, step_name(&self.class.2)),
            collect: false,
        };
        let ran = bounded(timeout + OUTPUT_GRACE * 2 + SLACK, move || {
            Ok(run_isolated(&step).outcome)
        })
        .await;
        drop(grant);
        ran
    }
}

/// A label's first word, as a step directory's name.
fn step_name(label: &str) -> &str {
    label.split_whitespace().next().unwrap_or("cmd")
}

/// What a blocking caller (the test proof) holds while one command runs: the grant and
/// the isolation directory, given back and removed when dropped.
pub(crate) struct Held {
    _grant: SlotGrant,
    tmp: PathBuf,
}

impl Drop for Held {
    fn drop(&mut self) {
        remove_dir(&self.tmp);
    }
}

/// For a caller on a blocking thread: waits for `class`'s grant on the daemon's
/// runtime (`handle`, the multi-thread one: blocking here never stalls a tokio
/// worker), makes `<base>/<name>`, and returns the variables to add and what to hold.
pub(crate) fn hold_blocking(
    handle: &tokio::runtime::Handle,
    sched: &Arc<TestScheduler>,
    class: &Class,
    (common, base, name): (&Path, &Path, &str),
) -> Result<(Vec<(String, String)>, Held), String> {
    let grant = handle.block_on(sched.acquire(request(class)));
    let tmp = fresh_dir(common, base, name)?;
    let mut extra = grant.env();
    extra.extend(isolation_env(&tmp));
    Ok((extra, Held { _grant: grant, tmp }))
}

#[cfg(test)]
#[path = "scheduled_tests.rs"]
mod tests;
