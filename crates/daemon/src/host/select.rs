//! Decision 15: which [`CodeHost`] the daemon uses, read once at daemon start beside
//! `ANTHREX_CLAUDE_BIN` (`ManagerConfig::from_vars`) and built once into
//! `RunContext.host`, which the profile service shares for detection. `ANTHREX_CODE_HOST`
//! unset or `gh` is the user's own `gh` (`ANTHREX_GH_BIN`, default `gh`); `fake` is
//! [`FakeHost`] on `ANTHREX_FAKE_HOST_DIR`, which never reaches the network; any other
//! value is a warning and `gh`. I/O only in what it builds (design decision 1).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::fake::FakeHost;
use super::{
    CodeHost, DeleteBranchReq, FetchOutcome, FetchReq, GhHost, HostError, HostRepo, LogFile,
    OpenPrReq, PrRef, PrView, PreflightReq, PushOutcome, PushReq, ReplyReq, RepoPermission,
    SystemRunner,
};

/// Every call of a `fake` host with no directory fails with this (decision 15).
pub const FAKE_NEEDS_DIR: &str = "ANTHREX_CODE_HOST=fake needs ANTHREX_FAKE_HOST_DIR";

/// Decision 15's choice (Interfaces "select.rs").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeHostChoice {
    Gh { bin: PathBuf },
    Fake { dir: Option<PathBuf> },
}

impl Default for CodeHostChoice {
    fn default() -> Self {
        CodeHostChoice::Gh { bin: "gh".into() }
    }
}

impl CodeHostChoice {
    /// The choice `var` (the daemon's environment) makes, and the warning an unknown
    /// `ANTHREX_CODE_HOST` gets. An empty variable counts as unset.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> (CodeHostChoice, Option<String>) {
        let set = |key: &str| var(key).filter(|value| !value.is_empty());
        let gh = || CodeHostChoice::Gh {
            bin: set("ANTHREX_GH_BIN").map_or_else(|| "gh".into(), PathBuf::from),
        };
        match set("ANTHREX_CODE_HOST").as_deref() {
            None | Some("gh") => (gh(), None),
            Some("fake") => (
                CodeHostChoice::Fake {
                    dir: set("ANTHREX_FAKE_HOST_DIR").map(PathBuf::from),
                },
                None,
            ),
            Some(other) => (
                gh(),
                Some(format!(
                    "ANTHREX_CODE_HOST={} is not gh or fake; using gh",
                    other.escape_debug()
                )),
            ),
        }
    }
}

/// Builds the host of `choice`. Called once per daemon (`RunContext::new`).
pub fn build(choice: &CodeHostChoice) -> Arc<dyn CodeHost> {
    match choice {
        CodeHostChoice::Gh { bin } => Arc::new(GhHost::new(
            SystemRunner::new(bin.clone(), "git"),
            bin.clone(),
            "git",
        )),
        CodeHostChoice::Fake { dir: Some(dir) } => Arc::new(FakeHost::new(dir.clone())),
        CodeHostChoice::Fake { dir: None } => Arc::new(Unconfigured),
    }
}

/// `ANTHREX_CODE_HOST=fake` without its directory: every call fails, nothing runs.
struct Unconfigured;

fn unconfigured<T>() -> Result<T, HostError> {
    Err(HostError::Missing(FAKE_NEEDS_DIR.to_string()))
}

impl CodeHost for Unconfigured {
    fn preflight(&self, _: &PreflightReq) -> Result<HostRepo, HostError> {
        unconfigured()
    }
    fn detect(&self, _: &Path, _: &str) -> Result<HostRepo, HostError> {
        unconfigured()
    }
    fn push(&self, _: &PushReq) -> Result<PushOutcome, HostError> {
        unconfigured()
    }
    fn fetch(&self, _: &FetchReq) -> Result<FetchOutcome, HostError> {
        unconfigured()
    }
    fn open_pr(&self, _: &OpenPrReq) -> Result<PrRef, HostError> {
        unconfigured()
    }
    fn view_pr(&self, _: &HostRepo, _: u64) -> Result<PrView, HostError> {
        unconfigured()
    }
    fn failed_logs(&self, _: &HostRepo, _: u64, _: u64, _: &Path) -> Result<LogFile, HostError> {
        unconfigured()
    }
    fn rerun_failed(&self, _: &HostRepo, _: u64) -> Result<(), HostError> {
        unconfigured()
    }
    fn reply(&self, _: &ReplyReq) -> Result<u64, HostError> {
        unconfigured()
    }
    fn retarget(&self, _: &HostRepo, _: u64, _: &str) -> Result<(), HostError> {
        unconfigured()
    }
    fn permission(&self, _: &HostRepo, _: &str) -> Result<RepoPermission, HostError> {
        unconfigured()
    }
    fn delete_branch(&self, _: &DeleteBranchReq) -> Result<(), HostError> {
        unconfigured()
    }
}

#[cfg(test)]
#[path = "tests_select.rs"]
mod tests;
