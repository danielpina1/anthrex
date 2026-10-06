//! The two shapes of a worker's git grant (`run::git::worker_git_grant`), against real
//! temporary repositories, on any host.
//!
//! macOS Seatbelt lets a process create a missing file at a granted path, so the grant
//! names the exact files a commit writes, each with its `.lock`. Claude Code's Linux
//! sandbox (bubblewrap) bind-mounts each granted path, which works only for a path that
//! exists, and a lock file never exists beforehand: git creates it with `O_EXCL`. So on
//! Linux the grant is the checkout's own git directory, whole, with the entries a worker
//! must never change denied, each made to exist first (bubblewrap can only deny a path
//! that exists, and Claude Code's sandbox would otherwise put a mount point there).

mod support;

use daemon::run::git::{GrantShape, WorkerGrant, prepare_task_worktree, worker_git_grant};
use daemon::run::role_launch::{task_repo_dir, worker_git_roots};
use std::path::{Path, PathBuf};
use support::run_git::{T, commit_file, out, real_git, repo, wt_dir};

/// The names a worker's Linux grant denies in its git directory; `commondir` only where
/// it exists (a linked worktree's).
const DENIED: [&str; 9] = [
    "gitdir",
    "config",
    "config.worktree",
    "locked",
    "packed-refs",
    "logs",
    "refs",
    "info",
    "hooks",
];

struct Checkout {
    _repo: support::TempRepo,
    _wt: tempfile::TempDir,
    data: tempfile::TempDir,
    common: PathBuf,
    task: PathBuf,
    admin: PathBuf,
}

/// A task checkout (its own repository in the run's data directory), as the engine
/// makes one.
fn checkout(run: &str) -> Checkout {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_task_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        &task_repo_dir(data.path(), "t1"),
        T,
    )
    .unwrap();
    let common = git_common_dir(&repo.root);
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    Checkout {
        _repo: repo,
        _wt: wt,
        data,
        common,
        task,
        admin,
    }
}

fn git_common_dir(root: &Path) -> PathBuf {
    PathBuf::from(out(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap()
}

fn grant(c: &Checkout, shape: GrantShape) -> WorkerGrant {
    let roots = worker_git_roots(c.data.path(), "t1");
    worker_git_grant(&c.common, &c.task, &roots, shape).unwrap()
}

/// The private roots, as the grant spells them (made and named by the engine).
fn roots(c: &Checkout) -> Vec<PathBuf> {
    worker_git_roots(c.data.path(), "t1")
        .iter()
        .map(|root| daemon::run::git::private_dir(&c.common, root).unwrap())
        .collect()
}

/// Linux: the checkout's git directory whole, beside the private roots, and every
/// entry a worker must not change denied, each existing (an empty placeholder where it
/// was missing; `packed-refs` a header with no ref). A repository's own git directory
/// has no `commondir`; none is made (any file there redirects or breaks every git
/// command in the checkout), and none is denied (Claude Code's sandbox would mount
/// `/dev/null` at it, which makes git fail).
#[test]
fn the_linux_grant_is_the_git_dir_whole_with_its_config_denied() {
    let c = checkout("gs01");
    let granted = grant(&c, GrantShape::WholeDir);
    let mut writable = roots(&c);
    writable.push(c.admin.clone());
    assert_eq!(granted.writable, writable);
    let denied: Vec<PathBuf> = DENIED.iter().map(|name| c.admin.join(name)).collect();
    assert_eq!(granted.deny, denied);
    for path in &granted.deny {
        let meta = std::fs::symlink_metadata(path)
            .unwrap_or_else(|err| panic!("{} does not exist: {err}", path.display()));
        assert!(!meta.file_type().is_symlink(), "{}", path.display());
    }
    for name in ["gitdir", "config.worktree", "locked"] {
        assert_eq!(std::fs::read(c.admin.join(name)).unwrap(), b"", "{name}");
    }
    assert_eq!(
        std::fs::read_to_string(c.admin.join("packed-refs")).unwrap(),
        "# pack-refs with: peeled fully-peeled sorted \n"
    );
    for name in ["logs", "hooks"] {
        assert!(c.admin.join(name).is_dir(), "{name}");
    }
    // The engine's own config is kept, not replaced by a placeholder.
    let config = std::fs::read_to_string(c.admin.join("config")).unwrap();
    assert!(config.contains("logAllRefUpdates = false"), "{config}");
    assert!(std::fs::symlink_metadata(c.admin.join("commondir")).is_err());
    // Nothing of the common directory.
    for path in granted.writable.iter().chain(&granted.deny) {
        assert!(!path.starts_with(&c.common), "{}", path.display());
    }
    // The checkout is still the engine's: every daemon git call in it passes its check,
    // and git reads the placeholders as nothing.
    assert_eq!(out(&c.task, &["for-each-ref"]), "");
    out(&c.task, &["status", "--short"]);
    daemon::worktree::run_git(
        real_git(),
        &c.task,
        &[
            std::ffi::OsStr::new("status"),
            std::ffi::OsStr::new("--short"),
        ],
        std::time::Instant::now() + T,
    )
    .unwrap();
}

/// macOS: today's exact files, each with its `.lock`, and the rebase directories; no
/// denial and no placeholder.
#[test]
fn the_macos_grant_names_exact_files_and_makes_nothing() {
    let c = checkout("gs02");
    let granted = grant(&c, GrantShape::Files);
    let mut writable = roots(&c);
    for file in daemon::run::git::WORKTREE_GIT_FILES {
        writable.push(c.admin.join(file));
        writable.push(c.admin.join(format!("{file}.lock")));
    }
    for dir in daemon::run::git::WORKTREE_GIT_DIRS {
        writable.push(c.admin.join(dir));
    }
    assert_eq!(granted.writable, writable);
    assert!(granted.deny.is_empty(), "{:?}", granted.deny);
    assert!(!granted.writable.contains(&c.admin));
    for name in [
        "gitdir",
        "config.worktree",
        "locked",
        "packed-refs",
        "hooks",
    ] {
        assert!(
            std::fs::symlink_metadata(c.admin.join(name)).is_err(),
            "{name} was made"
        );
    }
    // The grant function the macOS tests call is this shape.
    let roots = worker_git_roots(c.data.path(), "t1");
    let dirs = daemon::run::git::worker_git_dirs(&c.common, &c.task, &roots).unwrap();
    assert_eq!(dirs, granted.writable);
}

/// The host's shape: the whole directory on Linux, exact files elsewhere.
#[test]
fn the_host_shape_follows_the_os() {
    let expected = if cfg!(target_os = "linux") {
        GrantShape::WholeDir
    } else {
        GrantShape::Files
    };
    assert_eq!(GrantShape::host(), expected);
}

/// A linked worktree's git directory `<common>/worktrees/<name>`: its `commondir` and
/// `gitdir` exist and are denied, with the rest; the placeholders leave it a worktree
/// the daemon's check accepts.
#[test]
fn a_linked_worktrees_grant_denies_its_commondir_and_gitdir() {
    let repo = repo();
    commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt) = wt_dir();
    let linked = wt.join("linked");
    out(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    let common = git_common_dir(&repo.root);
    let admin = PathBuf::from(out(&linked, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    assert!(admin.starts_with(common.join("worktrees")));
    let granted = worker_git_grant(&common, &linked, &[], GrantShape::WholeDir).unwrap();
    assert_eq!(granted.writable, vec![admin.clone()]);
    let mut denied = vec![admin.join("commondir")];
    denied.extend(DENIED.iter().map(|name| admin.join(name)));
    assert_eq!(granted.deny, denied);
    for path in &granted.deny {
        assert!(
            std::fs::symlink_metadata(path).is_ok(),
            "{}",
            path.display()
        );
    }
    // `commondir` and `gitdir` are git's own, untouched.
    let named = std::fs::read_to_string(admin.join("commondir")).unwrap();
    assert!(!named.trim().is_empty());
    let back = std::fs::read_to_string(admin.join("gitdir")).unwrap();
    assert!(back.trim().ends_with("linked/.git"), "{back}");
    // An empty `locked` locks the worktree, which the engine unlocks before it removes
    // one; an empty `config.worktree` configures nothing.
    let pin = daemon::worktree::pinned::pin(&common, &linked, Default::default());
    daemon::worktree::pinned::check(&linked, &pin).unwrap();
    out(&linked, &["status", "--short"]);
    daemon::worktree::pinned::unpin(&linked);
}

/// A `commondir` a Linux worker makes in its checkout's own git directory (no
/// placeholder is possible there) is refused by the daemon's check, and a daemon git
/// call that started before it appeared reads no config through it: every daemon git
/// call in a standalone checkout names its own git directory as `GIT_COMMON_DIR`.
#[test]
fn the_daemons_git_reads_no_config_through_a_planted_commondir() {
    use daemon::worktree::pinned;
    let c = checkout("gs04");
    let pin = pinned::pinned(&c.task).unwrap();
    assert_eq!(
        pinned::common_dir_env(&pin),
        Some(("GIT_COMMON_DIR", pin.git_dir.as_path()))
    );
    let evil = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(evil.path().join("refs")).unwrap();
    std::fs::create_dir_all(evil.path().join("objects")).unwrap();
    std::fs::write(
        evil.path().join("config"),
        "[core]\n\tlogAllRefUpdates = always\n",
    )
    .unwrap();
    std::fs::write(
        c.admin.join("commondir"),
        format!("{}\n", evil.path().display()),
    )
    .unwrap();
    let err = pinned::check(&c.task, &pin).unwrap_err();
    assert!(err.contains("commondir"), "{err}");
    let read = |env: Option<(&str, &Path)>| {
        let mut command = std::process::Command::new("git");
        command
            .args(pinned::flags(&c.task, &pin))
            .args(["config", "core.logAllRefUpdates"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_DIR");
        if let Some((key, value)) = env {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    assert_eq!(read(None), "always", "the plant is live without it");
    assert_eq!(read(pinned::common_dir_env(&pin)), "false");
    std::fs::remove_file(c.admin.join("commondir")).unwrap();
    pinned::check(&c.task, &pin).unwrap();

    // A linked worktree's `commondir` is git's own: no override.
    let linked = pinned::Pin {
        standalone: false,
        ..pin
    };
    assert_eq!(pinned::common_dir_env(&linked), None);
}

/// The daemon's check still refuses a `config.worktree` that configures something (or
/// is a link): only the empty placeholder is accepted.
#[test]
fn a_config_worktree_with_content_is_still_refused() {
    let repo = repo();
    commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt) = wt_dir();
    let linked = wt.join("linked");
    out(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    let common = git_common_dir(&repo.root);
    let admin = PathBuf::from(out(&linked, &["rev-parse", "--absolute-git-dir"]));
    let pin = daemon::worktree::pinned::pin(&common, &linked, Default::default());
    std::fs::write(admin.join("config.worktree"), "[core]\n\tfsmonitor = x\n").unwrap();
    let err = daemon::worktree::pinned::check(&linked, &pin).unwrap_err();
    assert!(err.contains("config.worktree"), "{err}");
    std::fs::remove_file(admin.join("config.worktree")).unwrap();
    let elsewhere = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), admin.join("config.worktree")).unwrap();
    let err = daemon::worktree::pinned::check(&linked, &pin).unwrap_err();
    assert!(err.contains("config.worktree"), "{err}");
    std::fs::remove_file(admin.join("config.worktree")).unwrap();
    std::fs::write(admin.join("config.worktree"), "").unwrap();
    daemon::worktree::pinned::check(&linked, &pin).unwrap();
    daemon::worktree::pinned::unpin(&linked);
}

/// F1c's sweep, for the Linux shape: a link a worker left at a denied path is removed
/// before the next grant and replaced by its placeholder, so the sandbox denies the
/// path itself, never the link's target; nothing at the target is touched. The
/// reflog removal stays.
#[test]
fn links_left_at_denied_paths_are_replaced_by_placeholders() {
    let c = checkout("gs03");
    let elsewhere = tempfile::tempdir().unwrap();
    let target_file = elsewhere.path().join("file");
    std::fs::write(&target_file, "theirs\n").unwrap();
    for name in ["locked", "config.worktree", "packed-refs"] {
        let _ = std::fs::remove_file(c.admin.join(name));
        std::os::unix::fs::symlink(&target_file, c.admin.join(name)).unwrap();
    }
    for name in ["logs", "hooks"] {
        let _ = std::fs::remove_dir_all(c.admin.join(name));
        std::os::unix::fs::symlink(elsewhere.path(), c.admin.join(name)).unwrap();
    }
    let granted = grant(&c, GrantShape::WholeDir);
    for name in ["locked", "config.worktree", "packed-refs", "logs", "hooks"] {
        let path = c.admin.join(name);
        assert!(granted.deny.contains(&path), "{name} not denied");
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(!meta.file_type().is_symlink(), "{name} is still a link");
    }
    assert_eq!(std::fs::read_to_string(&target_file).unwrap(), "theirs\n");
    assert!(elsewhere.path().is_dir());

    // A reflog from before is removed, and `logs` is left an empty directory.
    std::fs::write(c.admin.join("logs/HEAD"), "old\n").unwrap();
    let granted = grant(&c, GrantShape::WholeDir);
    assert!(granted.deny.contains(&c.admin.join("logs")));
    assert!(!c.admin.join("logs/HEAD").exists());
}
