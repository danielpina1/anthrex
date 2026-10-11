//! M9.11.12 (B4): the daemon's `shutdown` waits for a worktree removal already under
//! way, so the daemon never exits with `git worktree remove` cut off half done.
//!
//! Its own test binary: it puts a `git` stand-in first on this process's `PATH`, which
//! every git the manager runs reads (AGENTS.md, "Facts learned").

use daemon::manager::{GitRoots, KILL_GRACE, ManagerConfig, WindowManager};
use daemon::worktree::OPERATION_TIMEOUT;
use proto::{Runtime, Status, WindowSpec};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The `git worktree remove` the stand-in holds: the injected stall.
const STALL: Duration = Duration::from_secs(2);
/// The workspace's allowance for a loaded machine (`docs/timing-budgets.md`).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(5);

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn real_git() -> PathBuf {
    let out = Command::new("/bin/sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(out.status.success(), "git is on PATH");
    PathBuf::from(String::from_utf8(out.stdout).unwrap().trim())
}

/// The test's own setup and checks, as AGENTS.md rule 11 asks of every git: no
/// optional locks, an outer git's repository variables removed, no system or signing
/// configuration, and an identity of its own (CI has none).
fn git(git: &Path, dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new(git);
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
    ] {
        command.env_remove(name);
    }
    let out = command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("--no-optional-locks")
        .args(["-c", "commit.gpgsign=false"])
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Anthrex Test",
            "-c",
            "user.email=test@anthrex.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

async fn wait_until(what: &str, bound: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + bound;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_a_worktree_removal_under_way() {
    let real = real_git();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let repo = base.join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&real, &repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "one\n").unwrap();
    git(&real, &repo, &["add", "README"]);
    git(&real, &repo, &["commit", "-q", "-m", "one"]);

    // The stand-in: `worktree remove` notes that it started and stalls, then every call
    // runs the real git.
    let bin = base.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let started = base.join("started");
    let stand_in = testexec::write_executable(
        bin.join("git"),
        format!(
            "#!/bin/sh\ncase \" $* \" in *\" worktree remove \"*) : > '{}'; sleep {} ;; esac\nexec '{}' \"$@\"\n",
            started.display(),
            STALL.as_secs(),
            real.display()
        ),
    );
    // Its first exec is paid here, before any bounded wait (macOS's first-exec stall).
    assert!(
        Command::new(&stand_in)
            .arg("--version")
            .output()
            .unwrap()
            .status
            .success()
    );
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin.clone()];
    paths.extend(std::env::split_paths(&path));
    // SAFETY: this binary's only test, before anything else in it reads the environment.
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };

    let mut config = ManagerConfig::for_tests(base.join("d.sock"), "/bin/sh".into());
    config.worktrees_root = base.join("worktrees");
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let spec = WindowSpec {
        name: Some("removing".into()),
        runtime: Runtime::Shell,
        cwd: repo.clone(),
        worktree_branch: Some("feat/removing".into()),
        model: None,
        initial_prompt: None,
    };
    let info = manager
        .create(spec, repo.clone(), Some(repo.clone()), 80, 24)
        .await
        .expect("the worktree window");
    let checkout = info.cwd.clone();
    assert!(git(&real, &repo, &["worktree", "list"]).contains(checkout.to_str().unwrap()));

    let removing = manager.clone();
    let removal = tokio::spawn(async move {
        removing
            .remove_with_worktree(info.id, false, &NoRoots)
            .await
    });
    // The removal's own bound: its dirty check, its kill wait and its git share one
    // `OPERATION_TIMEOUT` deadline, plus `KILL_GRACE` (`remove_with_worktree`'s doc).
    let removal_bound = OPERATION_TIMEOUT + KILL_GRACE + WALL_CLOCK_SLACK;
    wait_until("the removal's git", removal_bound, || started.exists()).await;

    tokio::time::timeout(removal_bound, manager.shutdown())
        .await
        .expect("shutdown returns");
    let listed = git(&real, &repo, &["worktree", "list"]);
    assert!(
        !listed.contains(checkout.to_str().unwrap()),
        "shutdown returned with the removal still under way:\n{listed}"
    );
    removal.await.unwrap().expect("the removal finished");
    assert!(
        manager
            .list()
            .iter()
            .all(|w| w.id != info.id || w.status == Status::Exited)
    );
}
