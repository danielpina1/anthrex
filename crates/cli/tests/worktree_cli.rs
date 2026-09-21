//! Round-trip coverage for M5.7: `anthrex new --worktree`, `anthrex rm --worktree
//! [--force]` and the `ls` branch column, driven through the real `anthrex` binary
//! against a real daemon and a real git repository. Parsing a flag correctly is not the
//! same as the flag doing the right thing end to end, so this exercises the full path:
//! the daemon actually creates a linked worktree, `ls` actually shows its branch, a
//! plain `rm` actually leaves the checkout on disk, an untracked file actually produces
//! the `remove-dirty` refusal and its hint, and `--force` actually deletes it.

mod support;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use support::{RunningCommand, TestDaemon, tempdir};

const GIT_TIMEOUT: Duration = Duration::from_secs(10);

fn run_git(dir: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
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
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    let output = RunningCommand::start(&mut command).finish(GIT_TIMEOUT);
    assert!(
        output.status.success(),
        "git {args:?} failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempdir();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["commit", "--allow-empty", "-m", "init"]);
    dir
}

fn wait_for_window(daemon: &TestDaemon, id: u32) -> proto::WindowInfo {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(w) = daemon.client().windows().into_iter().find(|w| w.id == id) {
            return w;
        }
        assert!(Instant::now() < deadline, "window {id} never appeared");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `new --worktree` makes a real linked worktree and starts the window in it; the branch
/// then shows up in `ls`'s new column, between STATUS and DIR.
#[test]
fn new_worktree_creates_a_real_checkout_and_ls_shows_its_branch() {
    let repo = init_repo();
    let daemon = TestDaemon::start(&[]);

    let output = daemon.anthrex(&[
        "new",
        "--runtime",
        "shell",
        "--name",
        "wt-new",
        "--dir",
        repo.path().to_str().unwrap(),
        "--worktree",
        "feat/cli-new",
    ]);
    assert!(
        output.status.success(),
        "new --worktree failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let id: u32 = std::str::from_utf8(&output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    let window = wait_for_window(&daemon, id);
    assert_eq!(window.branch.as_deref(), Some("feat/cli-new"));
    assert_ne!(
        window.cwd,
        repo.path(),
        "the window must run inside the new linked worktree, not the main checkout"
    );
    assert!(
        window.cwd.join(".git").exists(),
        "the linked worktree must exist on disk at {}",
        window.cwd.display()
    );

    let ls = daemon.anthrex(&["ls"]);
    assert!(ls.status.success());
    let ls_text = String::from_utf8_lossy(&ls.stdout);
    let lines: Vec<&str> = ls_text.lines().collect();
    assert!(
        lines[0].contains("BRANCH"),
        "ls header is missing BRANCH: {ls_text}"
    );
    let status_at = lines[0].find("STATUS").unwrap();
    let branch_at = lines[0].find("BRANCH").unwrap();
    let dir_at = lines[0].rfind("DIR").unwrap();
    assert!(
        status_at < branch_at && branch_at < dir_at,
        "BRANCH must sit between STATUS and DIR: {}",
        lines[0]
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("wt-new") && l.contains("feat/cli-new")),
        "ls did not show the worktree window's branch: {ls_text}"
    );

    // Clean up: force is not needed, the worktree has no changes.
    let rm = daemon.anthrex(&["rm", &id.to_string(), "--worktree"]);
    assert!(
        rm.status.success(),
        "cleanup rm --worktree failed: {}",
        String::from_utf8_lossy(&rm.stderr)
    );
    assert!(
        !window.cwd.exists(),
        "rm --worktree must delete the checkout: {}",
        window.cwd.display()
    );

    drop(daemon);
}

/// `rm` of a worktree window without `--worktree` keeps the checkout on disk and tells
/// the user so on stderr (decision 39) - worded as a fact about what was kept, since the
/// agent is already dead by the time this prints.
#[test]
fn rm_without_worktree_flag_keeps_the_checkout_and_reports_it() {
    let repo = init_repo();
    let daemon = TestDaemon::start(&[]);

    let output = daemon.anthrex(&[
        "new",
        "--runtime",
        "shell",
        "--name",
        "wt-kept",
        "--dir",
        repo.path().to_str().unwrap(),
        "--worktree",
        "feat/cli-kept",
    ]);
    assert!(output.status.success());
    let id: u32 = std::str::from_utf8(&output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let window = wait_for_window(&daemon, id);

    let rm = daemon.anthrex(&["rm", &id.to_string()]);
    assert!(
        rm.status.success(),
        "plain rm of a worktree window must still succeed: {}",
        String::from_utf8_lossy(&rm.stderr)
    );
    let stderr = String::from_utf8_lossy(&rm.stderr);
    assert_eq!(
        stderr.trim(),
        format!(
            "kept worktree {} on branch feat/cli-kept",
            window.cwd.display()
        ),
        "unexpected kept-worktree wording"
    );
    assert!(
        window.cwd.join(".git").exists(),
        "the checkout must survive a plain rm: {}",
        window.cwd.display()
    );

    // Window itself is gone.
    let windows = daemon.client().windows();
    assert!(!windows.iter().any(|w| w.id == id));

    drop(daemon);
}

/// A dirty worktree refuses a plain `rm --worktree` with the daemon's `remove-dirty`
/// message plus the CLI's hint naming both follow-up commands, exits non-zero, and
/// leaves the (killed) window listed so the user can retry; `--force` then deletes the
/// checkout including the untracked change.
#[test]
fn rm_worktree_on_a_dirty_checkout_refuses_then_force_deletes_it() {
    let repo = init_repo();
    let daemon = TestDaemon::start(&[]);

    let output = daemon.anthrex(&[
        "new",
        "--runtime",
        "shell",
        "--name",
        "wt-dirty",
        "--dir",
        repo.path().to_str().unwrap(),
        "--worktree",
        "feat/cli-dirty",
    ]);
    assert!(output.status.success());
    let id: u32 = std::str::from_utf8(&output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let window = wait_for_window(&daemon, id);

    std::fs::write(window.cwd.join("untracked.txt"), b"scratch").unwrap();

    let refused = daemon.anthrex(&["rm", &id.to_string(), "--worktree"]);
    assert!(
        !refused.status.success(),
        "rm --worktree on a dirty checkout must fail"
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("uncommitted") || stderr.contains("untracked"),
        "expected the daemon's dirty message, got: {stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "run 'anthrex rm {id} --worktree --force' to discard it anyway"
        )) && stderr.contains(&format!("'anthrex rm {id}' to keep the worktree")),
        "expected the CLI's dirty hint naming both follow-up commands, got: {stderr}"
    );
    assert!(
        window.cwd.exists(),
        "a refused removal must leave the checkout in place"
    );
    assert!(
        daemon.client().windows().iter().any(|w| w.id == id),
        "a refused removal must leave the window listed so the user can retry"
    );

    let forced = daemon.anthrex(&["rm", &id.to_string(), "--worktree", "--force"]);
    assert!(
        forced.status.success(),
        "rm --worktree --force must delete a dirty checkout: {}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert!(
        !window.cwd.exists(),
        "the checkout must be gone after --force: {}",
        window.cwd.display()
    );
    assert!(!daemon.client().windows().iter().any(|w| w.id == id));

    drop(daemon);
}
