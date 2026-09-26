//! The rig of M8b.10's verification tests (`profile_verify*.rs`): a real repository
//! from `init_repo`, its preflight and its repository data directory, and
//! `profile::verify` driven directly. No agent runs.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::profile::repo_dir;
use daemon::profile::verify::{Verified, VerifyJob, checkout_path, confine_spec, prepare, verify};
use daemon::run::confine::ConfineSpec;
use daemon::run::git::{GitQueue, checkout_repo_dir, preflight};
use daemon::run::plan::Preflight;
use proto::RepoProfile;

use super::run_harness::init_repo;
use super::{runtime, tempdir};

pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);
/// A command's bound in these tests (each command is a shell builtin or two).
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Rig {
    _dir: Option<tempfile::TempDir>,
    pub root: PathBuf,
    pub repo: PathBuf,
    pub pre: Preflight,
    pub repo_dir: PathBuf,
}

impl Rig {
    pub fn new() -> Rig {
        let dir = tempdir();
        let mut rig = Rig::in_dir(&dir.path().canonicalize().unwrap());
        rig._dir = Some(dir);
        rig
    }

    pub fn in_dir(root: &Path) -> Rig {
        let repo = root.join("repo");
        init_repo(&repo, &[("tests/t_ok.sh", "echo PASS t_ok\n")]);
        let pre = preflight(OsStr::new("git"), &repo, GIT_TIMEOUT).unwrap();
        let repo_dir = repo_dir(&root.join("data"), &pre.project);
        Rig {
            _dir: None,
            root: root.to_path_buf(),
            repo,
            pre,
            repo_dir,
        }
    }

    pub fn worktrees(&self) -> PathBuf {
        self.root.join("worktrees")
    }

    pub fn checkout(&self) -> PathBuf {
        checkout_path(&self.worktrees(), &self.pre.project)
    }

    /// Decision 9's confinement from `config` (the user's tables), as the service
    /// builds it.
    pub fn confine(&self, config: &config::Orchestrator) -> Option<ConfineSpec> {
        confine_spec(
            config,
            &self.repo_dir,
            &self.pre,
            &self.root.join("daemon.sock"),
        )
    }

    pub fn verify_with(
        &self,
        profile: RepoProfile,
        confine: Option<ConfineSpec>,
        timeout: Duration,
    ) -> Verified {
        self.try_verify(profile, confine, timeout)
            .expect("the verification's git steps succeed")
    }

    pub fn try_verify(
        &self,
        profile: RepoProfile,
        confine: Option<ConfineSpec>,
        timeout: Duration,
    ) -> Result<Verified, String> {
        let job = VerifyJob {
            git: "git".into(),
            pre: self.pre.clone(),
            repo_dir: self.repo_dir.clone(),
            worktrees_root: self.worktrees(),
            profile,
            confine,
            timeout,
            git_timeout: GIT_TIMEOUT,
        };
        runtime().block_on(verify(&GitQueue::new(), job))
    }

    pub fn checkout_repo(&self) -> PathBuf {
        checkout_repo_dir(&self.repo_dir, &self.checkout())
    }

    /// A pinned verification checkout at `HEAD`, as `verify` makes it.
    pub fn prepared(&self) -> PathBuf {
        let path = self.checkout();
        prepare(
            OsStr::new("git"),
            &self.pre,
            &path,
            &self.checkout_repo(),
            GIT_TIMEOUT,
        )
        .unwrap();
        path
    }

    /// Verified with the default config's confinement (confined where it can be).
    pub fn verify(&self, profile: RepoProfile) -> Verified {
        let confine = self.confine(&config::Orchestrator::default());
        self.verify_with(profile, confine, COMMAND_TIMEOUT)
    }
}

pub fn check(command: &str) -> RepoProfile {
    RepoProfile {
        check: Some(command.to_string()),
        ..Default::default()
    }
}
