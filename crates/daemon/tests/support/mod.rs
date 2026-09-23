//! Shared harness for the daemon's integration tests: a real daemon on an isolated,
//! temporary Unix socket, and a minimal client that speaks the wire protocol directly
//! (not `daemon::server`'s own client-side counterpart, so tests exercise exactly what
//! goes over the socket). Split out of `tests/server.rs` per AGENTS.md's ~600-line
//! guideline once `tests/server_git.rs` needed the same harness.
#![allow(dead_code)]

pub mod run_git;

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::server::serve;
use proto::{ClientKind, ClientMsg, DaemonMsg, Runtime, WindowSpec, read_frame, write_frame};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;

pub struct TestDaemon {
    pub _dir: tempfile::TempDir,
    pub socket: PathBuf,
    /// The `<data_dir>/worktrees` this daemon lays its linked worktrees out under, inside
    /// the harness's own `TempDir` so no test can reach a real data directory, and
    /// canonical so the paths a test computes spell themselves the same way the daemon's
    /// do (`worktree::create` canonicalizes, and a macOS `TempDir` under `/var` is a
    /// symlink to `/private/var`).
    pub worktrees_root: PathBuf,
    pub shutdown: CancellationToken,
    pub manager: Arc<WindowManager>,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.shutdown.cancel();
        for window in self.manager.list() {
            let _ = self.manager.remove(window.id);
        }
    }
}

pub async fn start_daemon() -> TestDaemon {
    start_daemon_with_git(true).await
}

pub async fn start_daemon_with_git(git_enabled: bool) -> TestDaemon {
    start_daemon_configured(git_enabled, |_| {}).await
}

/// As [`start_daemon_with_git`], with a last say over the manager's configuration
/// (`conversation.linger_secs`, for instance).
pub async fn start_daemon_configured(
    git_enabled: bool,
    configure: impl FnOnce(&mut ManagerConfig),
) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let stub = dir.path().join("stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let worktrees_root = dir.path().canonicalize().unwrap().join("worktrees");
    let mut config = ManagerConfig::new(socket.clone(), "/bin/sh".into());
    config.claude_bin = stub.to_str().unwrap().into();
    config.worktrees_root = worktrees_root.clone();
    configure(&mut config);
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let shutdown = CancellationToken::new();
    tokio::spawn(serve(
        listener,
        manager.clone(),
        config::Git {
            enabled: git_enabled,
            ..config::Git::default()
        },
        shutdown.clone(),
    ));
    TestDaemon {
        _dir: dir,
        socket,
        worktrees_root,
        shutdown,
        manager,
    }
}

pub struct Client {
    pub rd: OwnedReadHalf,
    pub wr: OwnedWriteHalf,
}

impl Client {
    pub async fn connect(d: &TestDaemon, version: u32) -> (Self, DaemonMsg) {
        let stream = UnixStream::connect(&d.socket).await.unwrap();
        let (rd, wr) = stream.into_split();
        let mut c = Client { rd, wr };
        c.send(ClientMsg::Hello {
            proto_version: version,
            client: ClientKind::Cli,
        })
        .await;
        let first = c.recv().await;
        (c, first)
    }

    pub async fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    pub async fn recv(&mut self) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.rd))
            .await
            .expect("timed out")
            .unwrap()
            .expect("daemon closed")
    }

    pub async fn recv_until(&mut self, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let m = self.recv().await;
                if pred(&m) {
                    return m;
                }
            }
        })
        .await
        .expect("timed out waiting for message")
    }
}

pub fn shell_spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

pub fn git(dir: &std::path::Path, args: &[&std::ffi::OsStr]) {
    let output = git_output(dir, args);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// As [`git`], but hands back the result instead of asserting success, for the queries
/// whose *failure* is the answer (`show-ref --verify` on a branch that does not exist).
pub fn git_output(dir: &std::path::Path, args: &[&std::ffi::OsStr]) -> std::process::Output {
    git_output_env(dir, &[], args)
}

/// As [`git_output`], with extra environment variables — `GIT_SEQUENCE_EDITOR` for a
/// scripted `rebase -i`, which is the only way to reach a rebase paused at `edit`
/// without a human.
pub fn git_output_env(
    dir: &std::path::Path,
    envs: &[(&str, &std::ffi::OsStr)],
    args: &[&std::ffi::OsStr],
) -> std::process::Output {
    let mut command = std::process::Command::new("git");
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

/// Adds a second and third commit on the current branch, so a worktree cut from it has
/// history to rebase or detach onto.
pub fn commit_more(dir: &std::path::Path, count: usize) {
    use std::ffi::OsStr;
    for n in 0..count {
        std::fs::write(dir.join("README"), format!("line {n}\n")).unwrap();
        git(dir, &[OsStr::new("add"), OsStr::new("README")]);
        git(
            dir,
            &[
                OsStr::new("commit"),
                OsStr::new("-m"),
                &std::ffi::OsString::from(format!("commit {n}")),
            ],
        );
    }
}

/// A real repository in a temporary directory: one commit holding `README` and a
/// `.gitignore` that ignores `ignored-*`, on the deterministic branch `main`.
///
/// The `.gitignore` is in that first commit deliberately: every linked worktree this
/// repository grows starts from a commit that already has it, so an ignored file in a
/// worktree really is ignored there. A `.gitignore` committed later, after the worktree
/// was added, would leave the worktree's own branch without it and the "ignored files do
/// not count" half of the dirty check would silently test nothing.
pub struct TempRepo {
    pub dir: tempfile::TempDir,
    /// Canonical, because `project::detect_roots` canonicalizes both roots and every
    /// path assertion here compares against what the daemon computed.
    pub root: PathBuf,
}

impl TempRepo {
    pub fn new() -> Self {
        Self::init_in(tempfile::tempdir().unwrap())
    }

    /// As [`TempRepo::new`], in a fresh directory under `/tmp` whose name starts with
    /// `prefix` — for paths with spaces and non-ASCII characters in them.
    pub fn with_prefix(prefix: &str) -> Self {
        Self::init_in(
            tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in("/tmp")
                .unwrap(),
        )
    }

    fn init_in(dir: tempfile::TempDir) -> Self {
        use std::ffi::OsStr;
        let path = dir.path();
        std::fs::write(path.join("README"), "one\n").unwrap();
        std::fs::write(path.join(".gitignore"), "ignored-*\n").unwrap();
        git(path, &[OsStr::new("init")]);
        git(
            path,
            &[
                OsStr::new("add"),
                OsStr::new("README"),
                OsStr::new(".gitignore"),
            ],
        );
        git(
            path,
            &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("init")],
        );
        let root = path.canonicalize().unwrap();
        Self { dir, root }
    }

    /// Runs git in the main checkout, asserting success.
    pub fn git(&self, args: &[&std::ffi::OsStr]) {
        git(&self.root, args);
    }

    pub fn branch_exists(&self, branch: &str) -> bool {
        use std::ffi::OsStr;
        git_output(
            &self.root,
            &[
                OsStr::new("show-ref"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                &std::ffi::OsString::from(format!("refs/heads/{branch}")),
            ],
        )
        .status
        .success()
    }

    /// Every checkout git still knows about, the main one first: the `worktree ` lines of
    /// `git worktree list --porcelain`.
    pub fn worktree_paths(&self) -> Vec<PathBuf> {
        use std::ffi::OsStr;
        let output = git_output(
            &self.root,
            &[
                OsStr::new("worktree"),
                OsStr::new("list"),
                OsStr::new("--porcelain"),
            ],
        );
        assert!(output.status.success(), "git worktree list failed");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("worktree ").map(PathBuf::from))
            .collect()
    }

    /// Installs a `post-checkout` hook that sleeps, so a `git worktree add` in this
    /// repository blocks *after* it has created the branch and checked the tree out —
    /// the state a create must still clean up when its deadline strikes.
    pub fn slow_post_checkout(&self, secs: u64) {
        self.post_checkout_hook(&format!("sleep {secs}"));
    }

    /// The file every hook installed here touches before it does anything else.
    ///
    /// It is what lets a test wait for git to be *inside* `worktree add` rather than
    /// sleeping and hoping: a fixed sleep would pass whether or not the create had got
    /// that far, which is exactly the synchronisation AGENTS.md rule 6 forbids.
    pub fn hook_marker(&self) -> PathBuf {
        self.root.join(".hook-started")
    }

    /// Installs a `post-checkout` hook that exits non-zero, which makes `git worktree
    /// add` itself exit non-zero *after* it has created the branch and checked the tree
    /// out (verified against git 2.50.1: exit 3, worktree and branch both present). That
    /// is the create-failed-part-way case without a timeout in it.
    pub fn failing_post_checkout(&self) {
        self.post_checkout_hook("exit 3");
    }

    fn post_checkout_hook(&self, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let hook = self.root.join(".git/hooks/post-checkout");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        // The marker is written with an absolute path because the hook runs with the new
        // worktree as its working directory, not the main checkout.
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\n: > '{}'\n{body}\n",
                self.hook_marker().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The branch `path`'s `HEAD` points at, as `git worktree add` left it.
pub fn head_branch(path: &std::path::Path) -> String {
    use std::ffi::OsStr;
    let output = git_output(
        path,
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--abbrev-ref"),
            OsStr::new("HEAD"),
        ],
    );
    assert!(output.status.success(), "git rev-parse failed");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub async fn claude_window(d: &TestDaemon, name: &str) -> u32 {
    let mut spec = shell_spec(name);
    spec.runtime = Runtime::Claude;
    d.manager
        .create(spec, std::env::temp_dir(), None, 80, 24)
        .await
        .unwrap()
        .id
}

/// A `git` stand-in for tests: a script in `dir` that appends one `argv` line (each
/// argument after a tab) and one `env` line per `GIT_*` variable it was given to
/// `<dir>/git.log`, then execs the real `git` with the same arguments.
pub fn recording_git(dir: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("git.log");
    let script = dir.join("recording-git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             log='{log}'\n\
             {{ printf 'argv'; for a in \"$@\"; do printf '\\t%s' \"$a\"; done; printf '\\n'; }} >> \"$log\"\n\
             env | grep '^GIT_' | while IFS= read -r l; do printf 'env\\t%s\\n' \"$l\"; done >> \"$log\"\n\
             exec git \"$@\"\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}
