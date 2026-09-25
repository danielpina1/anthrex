//! Confining the engine's check and proof commands (final fix batch F1c, re-review 4,
//! I2; rebuilt on a deny-by-default base in F1d). Blocking file-system work: call only
//! from `spawn_blocking` or a dedicated thread (AGENTS.md rule 2).
//!
//! A check or a proof runs code a worker wrote (a test, a `build.rs`, a script the
//! check calls). Workers are sandboxed so that nothing they write reaches the user's
//! repository but through the engine; a check run with the daemon's own rights would
//! undo that. So when the run's workers are sandboxed (`[orchestrator] worker_sandbox`,
//! decision 54), each check, proof run, `setup` and integration-candidate check runs
//! under the seatbelt profile of [`super::seatbelt`]: `(deny default)` with an explicit
//! allow-list. It may write only:
//! - the checkout it runs in (its files; for the integration checkout, never its git
//!   dir, which is in the user's `.git`);
//! - the checkout's own object store when it is its own repository (F1c, 3a);
//! - its short per-task temporary directory (`run::git::task_tmp`, F1d), which is
//!   `TMPDIR` for the command;
//! - the user's `cache_dirs` for the repository ([`super::confine_cache`]: absolute,
//!   resolved without following a link a run could have planted, and kept clear of
//!   `$HOME` itself, the run's own areas, the git common directory, anthrex's data
//!   directory and the daemon's socket);
//! - a few harmless `/dev` nodes, and the PTYs it opens itself.
//!
//! It reaches no network, localhost included, unless the user's own config enables it
//! for the repository: `[orchestrator.confined_network]` (F1d R4) opens outbound IP to
//! remote hosts and DNS only (round 2, S1); `confined_unix_sockets` names exact Unix
//! sockets and `confined_localhost_ports` loopback ports. Never the anthrex daemon's
//! socket, launchd's per-user sockets (ssh-agent) or the keychain. It
//! cannot hand work to an unconfined actor: LaunchServices (`open`), cfprefsd
//! (`defaults write`), AppleEvents and launchd are all outside the allow-list.
//! `run::exec::engine_env` also strips every `ANTHREX_*` variable from a confined
//! command, so it is not even handed the socket path.
//!
//! Every writable path is spelled the way [`super::git::private_dir`] spells a worker's
//! grant (I1): the engine's parent resolved, the leaf's name appended and checked with
//! `lstat`, never a path resolved through something the command could have swapped.
//!
//! On macOS the profile is applied with `/usr/bin/sandbox-exec -p <profile> /bin/sh`,
//! which applies the profile and `exec`s the shell: the pid the engine waits for and
//! kills is the shell's, as before. Linux has no equivalent the engine can rely on, so
//! there [`AVAILABLE`] is false and `run start` refuses a sandboxed run unless the user
//! allows unconfined checks (recorded in the brief's implementation notes and manual
//! check 4e).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::confine_cache::{Areas, resolve_all, resolve_sockets};
use super::exec::{ShellOutcome, run_confined};
use super::git::{Repo, checkout_repo_dir, private_dir, tmp_root};
use super::model::Run;
use super::seatbelt::{Grants, profile};

/// The program that applies a profile, on macOS.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether this platform can confine a command.
pub const AVAILABLE: bool = cfg!(target_os = "macos");

/// The daemon environment variable that, set to `unavailable`, makes the daemon treat
/// this platform as one without confinement: the tests' stand-in for Linux on macOS.
/// It can only lead to a refusal, or to what `--unconfined-checks` allows.
pub const CONFINEMENT_ENV: &str = "ANTHREX_CHECK_CONFINEMENT";

/// Whether the daemon can confine checks here ([`AVAILABLE`], unless
/// [`CONFINEMENT_ENV`] says otherwise).
pub fn available() -> bool {
    AVAILABLE && std::env::var_os(CONFINEMENT_ENV).is_none_or(|v| v != "unavailable")
}

/// Final fix batch F1c round 2: `run start`'s refusal of a run whose checks, proofs and
/// `setup` could not be confined, unless the user allowed it. `None` when the run may
/// start. A run whose workers are unsandboxed (`worker_sandbox = false`) is already the
/// user's explicit choice and is not refused here.
pub fn start_refusal(worker_sandbox: bool, available: bool, allowed: bool) -> Option<String> {
    (worker_sandbox && !available && !allowed).then(|| {
        "this platform cannot confine the run's checks, proofs and setup, which run code \
         the workers wrote; pass --unconfined-checks to run them unconfined anyway, or set \
         [orchestrator] unconfined_checks = true in your config"
            .to_string()
    })
}

/// What a run's confined commands may write and reach, from its frozen record: set
/// only when its workers are sandboxed and [`AVAILABLE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfineSpec {
    /// The run's data directory, `<data>/runs/<run>`.
    pub data_dir: PathBuf,
    /// The repository's git common directory.
    pub common_dir: PathBuf,
    /// The user's `cache_dirs` for the repository, as written (absolute, or `~/…` under
    /// the daemon's `$HOME`).
    pub cache_dirs: Vec<String>,
    /// The user's `confined_network` for the repository (F1d, R4): outbound IP to
    /// remote hosts only (round 2, S1).
    pub network: bool,
    /// The user's `confined_unix_sockets` for the repository, as written (F1d round 2).
    pub unix_sockets: Vec<String>,
    /// The user's `confined_localhost_ports` for the repository (F1d round 2).
    pub localhost_ports: Vec<u16>,
    /// The anthrex daemon's socket, never reachable (F1d).
    pub daemon_socket: PathBuf,
}

/// One command's confinement: what it may write, its `TMPDIR`, and whether it has the
/// network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confinement {
    /// The checkout, whose protected agent-config paths are denied (F2 round 2).
    checkout: PathBuf,
    writable: Vec<PathBuf>,
    tmp: PathBuf,
    network: bool,
    unix_sockets: Vec<PathBuf>,
    localhost_ports: Vec<u16>,
    daemon_socket: PathBuf,
}

impl ConfineSpec {
    /// `run`'s, when its workers are sandboxed and this platform can confine.
    pub fn for_run(run: &Run) -> Option<Self> {
        (run.limits.worker_sandbox && !run.limits.unconfined_checks && AVAILABLE).then(|| {
            ConfineSpec {
                data_dir: run.data_dir.clone(),
                common_dir: run.git_common_dir.clone(),
                cache_dirs: run.profile.cache_dirs.clone(),
                network: run.profile.confined_network,
                unix_sockets: run.profile.confined_unix_sockets.clone(),
                localhost_ports: run.profile.confined_localhost_ports.clone(),
                daemon_socket: proto::paths::socket_path(),
            }
        })
    }

    /// The confinement for a command in the checkout `dir`. Fails closed: a checkout,
    /// temporary directory or object store that is not a plain directory, or a cache
    /// directory [`super::confine_cache`] refuses, is an error, and the command must
    /// not run.
    pub fn for_checkout(&self, dir: &Path) -> Result<Confinement, String> {
        let checkout = plain_dir(dir)?;
        let repo = Repo::at(&checkout_repo_dir(&self.data_dir, dir));
        let tmp = private_dir(&self.common_dir, &repo.tmp())?;
        let mut writable = vec![checkout.clone(), tmp.clone()];
        if repo.git_dir().is_dir() {
            writable.push(private_dir(&self.common_dir, &repo.objects())?);
        }
        let common = canonical(&self.common_dir);
        let data = canonical(
            self.data_dir
                .parent()
                .and_then(Path::parent)
                .unwrap_or(&self.data_dir),
        );
        let checkouts = checkout.parent().unwrap_or(&checkout).to_path_buf();
        let root = tmp_root();
        let areas = Areas {
            checkouts: &checkouts,
            tmp_root: &root,
            data: &data,
            common: &common,
            daemon_socket: &self.daemon_socket,
        };
        writable.extend(resolve_all(&self.cache_dirs, &areas)?);
        let unix_sockets = resolve_sockets(&self.unix_sockets, &areas)?;
        Ok(Confinement {
            checkout,
            writable,
            tmp,
            network: self.network,
            unix_sockets,
            localhost_ports: self.localhost_ports.clone(),
            daemon_socket: self.daemon_socket.clone(),
        })
    }
}

impl Confinement {
    /// The paths the command may write, beside a few `/dev` nodes.
    pub fn writable(&self) -> &[PathBuf] {
        &self.writable
    }

    /// The command's `TMPDIR`.
    pub fn tmp(&self) -> &Path {
        &self.tmp
    }

    /// The `sandbox-exec` profile ([`super::seatbelt`]).
    pub fn profile(&self) -> Result<String, String> {
        profile(&Grants {
            writable: &self.writable,
            network: self.network,
            unix_sockets: &self.unix_sockets,
            localhost_ports: &self.localhost_ports,
            daemon_socket: &self.daemon_socket,
            checkout: Some(&self.checkout),
        })
    }
}

/// `command` in the checkout `dir`, confined as `confine` says when it is set; a
/// checkout that cannot be confined fails the command unrun.
pub fn confined(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
    confine: Option<&ConfineSpec>,
) -> ShellOutcome {
    match confine.map(|spec| spec.for_checkout(dir)).transpose() {
        Ok(confinement) => run_confined(dir, command, env, timeout, confinement.as_ref()),
        Err(error) => ShellOutcome::refused(error),
    }
}

/// `path` as a real directory spelled with its parent resolved and its own name
/// appended (never resolved itself): a link or a file there is refused.
fn plain_dir(path: &Path) -> Result<PathBuf, String> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(format!("{} has no parent directory", path.display()));
    };
    let parent = parent
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", parent.display()))?;
    let dir = parent.join(name);
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a plain directory; it was tampered with",
            dir.display()
        ));
    }
    Ok(dir)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(root: &Path, cache_dirs: &[&str]) -> ConfineSpec {
        let common = root.join("repo/.git");
        std::fs::create_dir_all(&common).unwrap();
        ConfineSpec {
            data_dir: root.join("data/runs/r1"),
            common_dir: common,
            cache_dirs: cache_dirs.iter().map(|s| s.to_string()).collect(),
            network: false,
            unix_sockets: Vec::new(),
            localhost_ports: Vec::new(),
            daemon_socket: root.join("sock/daemon.sock"),
        }
    }

    #[test]
    fn a_checkout_gets_itself_its_tmp_and_the_caches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let checkout = root.join("wt/runs/r1/t1");
        std::fs::create_dir_all(&checkout).unwrap();
        let c = spec(&root, &["/opt/cache/x"])
            .for_checkout(&checkout)
            .unwrap();
        // F1d: the temporary directory is short, under the daemon's own root.
        let tmp = crate::run::git::task_tmp(&root.join("data/runs/r1/tasks/t1"));
        assert!(tmp.is_dir());
        assert_eq!(c.tmp(), tmp);
        assert_eq!(
            c.writable(),
            [
                checkout.clone(),
                tmp.clone(),
                canonical(Path::new("/opt")).join("cache/x"),
            ]
        );
        let profile = c.profile().unwrap();
        assert!(profile.contains("(deny default)"), "{profile}");
        assert!(profile.contains(&format!("(subpath \"{}\")", checkout.display())));
        assert!(
            profile.contains(&format!(
                "(literal \"{}\")",
                root.join("sock/daemon.sock").display()
            )),
            "{profile}"
        );
        assert!(!profile.contains("(allow network*)"), "{profile}");
        let mut networked = spec(&root, &[]);
        networked.network = true;
        let profile = networked
            .for_checkout(&checkout)
            .unwrap()
            .profile()
            .unwrap();
        assert!(profile.contains("(remote ip \"*:*\")"), "{profile}");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_cache_over_the_repository_or_the_data_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let checkout = root.join("wt/runs/r1/t1");
        std::fs::create_dir_all(&checkout).unwrap();
        let repo = root.join("repo");
        for bad in [
            repo.display().to_string(),
            repo.join(".git/objects").display().to_string(),
            root.display().to_string(),
            root.join("data/runs/r2").display().to_string(),
            "../x".to_string(),
            "rel".to_string(),
        ] {
            let err = spec(&root, &[&bad]).for_checkout(&checkout).unwrap_err();
            assert!(
                err.contains("overlaps") || err.contains("`..`") || err.contains("absolute"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn a_checkout_swapped_for_a_link_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let checkout = root.join("wt/runs/r1/t1");
        std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&root, &checkout).unwrap();
        let err = spec(&root, &[]).for_checkout(&checkout).unwrap_err();
        assert!(err.contains("tampered"), "{err}");
    }

    #[test]
    fn start_is_refused_only_for_sandboxed_workers_without_confinement_or_leave() {
        assert!(start_refusal(true, false, false).is_some());
        assert!(start_refusal(true, false, true).is_none());
        assert!(start_refusal(true, true, false).is_none());
        assert!(start_refusal(false, false, false).is_none());
        let text = start_refusal(true, false, false).unwrap();
        assert!(text.contains("--unconfined-checks"), "{text}");
    }
}
