//! What the driver needs from the daemon ([`RunContext`]) and what each op carries
//! ([`OpCtx`]), moved out of `driver.rs` (task M9.13's preparatory move; no behaviour
//! change).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::headless::argv::CliCaps;
use crate::manager::{GitRoots, ManagerConfig};

/// What the service needs from the daemon.
pub struct RunContext {
    pub data_dir: PathBuf,
    pub worktrees_root: PathBuf,
    pub orchestrator: config::Orchestrator,
    pub git_roots: Arc<dyn GitRoots>,
    pub git: OsString,
    /// The manager's `cli_caps` (decision 53's project-settings check reads the same
    /// caps the sessions are launched with, test overrides included).
    pub cli_caps: CliCaps,
    /// Milestone 9.1 decision 3: the daemon's `[testing]` table, read once at start;
    /// each run freezes its run rules from it (`RunLimits.testing`).
    pub testing: config::Testing,
}

impl RunContext {
    /// The context of a daemon whose manager was built from `manager`.
    pub fn new(
        data_dir: PathBuf,
        manager: &ManagerConfig,
        orchestrator: config::Orchestrator,
        git_roots: Arc<dyn GitRoots>,
    ) -> Self {
        RunContext {
            data_dir,
            worktrees_root: manager.worktrees_root.clone(),
            orchestrator,
            git_roots,
            git: OsString::from("git"),
            cli_caps: manager.cli_caps,
            testing: config::Testing::default(),
        }
    }

    /// This context with the daemon's `[testing]` table (the defaults otherwise).
    pub fn with_testing(mut self, testing: config::Testing) -> Self {
        self.testing = testing;
        self
    }
}

/// Where an op's work goes: captured under the engine lock at the step that emitted it.
#[derive(Clone)]
pub(crate) struct OpCtx {
    pub run_id: String,
    pub project: PathBuf,
    pub data_dir: PathBuf,
    pub git_timeout: Duration,
    pub check_timeout: Duration,
    /// Final fix batch F1c (I2): how checks and proofs are confined, when the run's
    /// workers are sandboxed and this platform can confine them. Boxed: it is large,
    /// and every queued op carries a context.
    pub confine: Option<Box<crate::run::confine::ConfineSpec>>,
}

impl OpCtx {
    /// `run`'s context for an op.
    pub(crate) fn of(run: &crate::run::model::Run) -> Self {
        OpCtx {
            run_id: run.id.clone(),
            project: run.project.clone(),
            data_dir: run.data_dir.clone(),
            git_timeout: Duration::from_secs(run.limits.git_timeout_secs),
            check_timeout: Duration::from_secs(run.profile.check_timeout_secs),
            confine: crate::run::confine::ConfineSpec::for_run(run).map(Box::new),
        }
    }
}
