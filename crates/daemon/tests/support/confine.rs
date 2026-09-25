//! The world the confinement tests (`tests/run_confine*.rs`) share: a repository, a
//! run's worktree and data directories, and a directory outside all of them that
//! stands in for the user's `$HOME` and for a cache directory.

use super::TempRepo;
use super::run_git::{T, commit_file, real_git, repo, wt_dir};
use daemon::run::confine::ConfineSpec;
use daemon::run::git::{checkout_repo_dir, prepare_task_worktree};
use std::path::{Path, PathBuf};

pub struct World {
    pub repo: TempRepo,
    _wt: tempfile::TempDir,
    pub wt: PathBuf,
    _data: tempfile::TempDir,
    pub data: PathBuf,
    _outside: tempfile::TempDir,
    /// A stand-in for the user's `$HOME` and for a cache directory, outside everything.
    pub outside: PathBuf,
}

pub fn world() -> World {
    let repo = repo();
    commit_file(&repo.root, "README", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let data_path = data.path().canonicalize().unwrap().join("runs/cf01");
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    World {
        repo,
        _wt: wt,
        wt: wt_path,
        _data: data,
        data: data_path,
        _outside: outside,
        outside: outside_path,
    }
}

impl World {
    pub fn common(&self) -> PathBuf {
        self.repo.root.join(".git").canonicalize().unwrap()
    }

    pub fn spec(&self, cache_dirs: &[&Path]) -> ConfineSpec {
        ConfineSpec {
            data_dir: self.data.clone(),
            common_dir: self.common(),
            cache_dirs: cache_dirs.iter().map(|p| p.display().to_string()).collect(),
            network: false,
            daemon_socket: self.outside.join("daemon.sock"),
        }
    }

    pub fn task(&self) -> PathBuf {
        let path = self.wt.join("runs/cf01/t1");
        let base = super::run_git::head(&self.repo.root);
        prepare_task_worktree(
            real_git(),
            &self.repo.root,
            "anthrex/cf01/t1",
            &base,
            &path,
            &checkout_repo_dir(&self.data, &path),
            T,
        )
        .unwrap();
        path
    }
}
