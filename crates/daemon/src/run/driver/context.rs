//! What the driver needs from the daemon ([`RunContext`]) and what each op carries
//! ([`OpCtx`]), moved out of `driver.rs` (task M9.13's preparatory move; no behaviour
//! change).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::DONE_CHECK_GIT_TIMEOUT;
use crate::headless::argv::CliCaps;
use crate::host::CodeHost;
use crate::live_config::{LiveSettings, SettingsIo};
use crate::manager::{GitRoots, ManagerConfig};

/// What the service needs from the daemon.
pub struct RunContext {
    pub data_dir: PathBuf,
    pub worktrees_root: PathBuf,
    /// Milestone 9.0.6 decision 29: the live `[orchestrator]` settings. Every read takes
    /// `settings.current()` once per request; a Settings save swaps the owned keys.
    pub settings: Arc<LiveSettings>,
    /// Decision 31: where a Settings save writes. `RunContext::new` puts it inside
    /// `data_dir` (preflight F20), so only `with_settings` can name the user's file.
    pub config_path: PathBuf,
    /// How a Settings save writes and how long it may take (a test seam).
    pub settings_io: SettingsIo,
    pub git_roots: Arc<dyn GitRoots>,
    pub git: OsString,
    /// The manager's `cli_caps` (decision 53's project-settings check reads the same
    /// caps the sessions are launched with, test overrides included).
    pub cli_caps: CliCaps,
    /// Milestone 9.1 decision 3: the daemon's `[testing]` table, read once at start;
    /// each run freezes its run rules from it (`RunLimits.testing`).
    pub testing: config::Testing,
    /// Milestone 9.2 decision 16: the daemon's `[delivery]` table, read once at start;
    /// each run freezes it (`RunDelivery.limits`).
    pub delivery: config::Delivery,
    /// The git budget of `task_result`'s reads and of `ResolveTarget`: always
    /// [`GitBudget::DONE_CHECK`] in the daemon. A test seam only (M9.1 flake fix).
    pub read_git: GitBudget,
    /// Milestone 9.2 decision 15: the daemon's one code host, built once from
    /// `ManagerConfig.code_host` (`RunContext::new`); the profile service shares it.
    pub host: Arc<dyn CodeHost>,
    /// A test seam: caps every host op's bound (decision 9). `None` in the daemon.
    pub host_cap: Option<Duration>,
}

/// One git read's budget: a deadline for all its calls, and a cap on each call's own
/// bound (the run's `git_timeout_secs`, capped by `each_cap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitBudget {
    pub deadline: Duration,
    pub each_cap: Duration,
}

impl GitBudget {
    /// The daemon's budget: one [`DONE_CHECK_GIT_TIMEOUT`] deadline, each call capped by
    /// the same value.
    pub const DONE_CHECK: GitBudget = GitBudget {
        deadline: DONE_CHECK_GIT_TIMEOUT,
        each_cap: DONE_CHECK_GIT_TIMEOUT,
    };

    /// The bound of each call of a run whose `git_timeout_secs` is `git_timeout`.
    pub fn each(&self, git_timeout: Duration) -> Duration {
        git_timeout.min(self.each_cap)
    }
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
            config_path: data_dir.join("config.toml"),
            data_dir,
            worktrees_root: manager.worktrees_root.clone(),
            settings: LiveSettings::defaults_of(orchestrator),
            settings_io: SettingsIo::default(),
            git_roots,
            git: OsString::from("git"),
            cli_caps: manager.cli_caps,
            testing: config::Testing::default(),
            delivery: config::Delivery::default(),
            read_git: GitBudget::DONE_CHECK,
            host: crate::host::select::build(&manager.code_host),
            host_cap: None,
        }
    }

    /// This context with `host` in place of the one `ManagerConfig` chose (tests).
    pub fn with_host(mut self, host: Arc<dyn CodeHost>) -> Self {
        self.host = host;
        self
    }

    /// The daemon's context: `manager`'s, with the `[orchestrator]`, `[testing]` and
    /// `[delivery]` tables of the loaded `config.toml`. `lifecycle` builds the daemon's
    /// context with it, so a test covers the same wiring (M9.2.3's review fix 1).
    pub fn from_config(
        data_dir: PathBuf,
        manager: &ManagerConfig,
        config: &config::Config,
        git_roots: Arc<dyn GitRoots>,
    ) -> Self {
        RunContext::new(data_dir, manager, config.orchestrator.clone(), git_roots)
            .with_testing(config.testing.clone())
            .with_delivery(config.delivery.clone())
    }

    /// This context with the daemon's `[delivery]` table (the defaults otherwise).
    pub fn with_delivery(mut self, delivery: config::Delivery) -> Self {
        self.delivery = delivery;
        self
    }

    /// This context with the daemon's `[testing]` table (the defaults otherwise).
    pub fn with_testing(mut self, testing: config::Testing) -> Self {
        self.testing = testing;
        self
    }

    /// This context with the daemon's live settings and its `config.toml` (decision 31).
    pub fn with_settings(mut self, settings: Arc<LiveSettings>, config_path: PathBuf) -> Self {
        self.settings = settings;
        self.config_path = config_path;
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
