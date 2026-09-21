//! Discriminates the two call sites `commit c2fee51` changed to use the longer,
//! worktree-aware timeout (`crates/cli/src/main.rs`): `new --worktree`'s choice of
//! `client::WORKTREE_REQUEST_TIMEOUT` over `client::CREATE_WINDOW_REPLY_TIMEOUT`, and `rm
//! --worktree`'s `client::WORKTREE_REQUEST_TIMEOUT` argument to `request_with_timeout`.
//!
//! Every other test in this crate runs against a small local repo where git finishes in
//! milliseconds, so a call site quietly reverted to its old, shorter timeout would still
//! pass them (`cargo test -p cli` stays green either way). These two close that gap by
//! putting a `git` wrapper ahead of the real one on `PATH` — the same technique
//! `create_project_timeout.rs` uses in this crate — that sleeps only for the one `git
//! worktree add` / `git worktree remove` invocation the call site under test triggers,
//! long enough to blow past the short timeout it would fall back to if reverted, but
//! short enough to finish comfortably inside `WORKTREE_REQUEST_TIMEOUT`.
//!
//! The third test is the other half: not "is the longer timeout used here?" but "is the
//! longer timeout long enough?". It drives the create path to the top of the budget
//! `client::WORKTREE_REQUEST_TIMEOUT` is derived against and asserts the CLI comes back
//! with the *daemon's* answer rather than its own "timed out waiting for the daemon".
//!
//! `crates/cli` has no `[lib]` target, so an integration test here cannot `use
//! client::WORKTREE_REQUEST_TIMEOUT` to compute the delay; the values below are
//! transcribed from `crates/cli/src/client.rs` instead of guessed: `REQUEST_TIMEOUT` =
//! 5s, `CREATE_WINDOW_REPLY_TIMEOUT` = `DETECT_TIMEOUT` (5s) + `CREATE_REPLY_ALLOWANCE`
//! (2s) = 7s, `WORKTREE_REQUEST_TIMEOUT` = max(create 45s, removal 33s) +
//! `WORKTREE_TIMEOUT_MARGIN` (12s) = 57s.

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use support::{TestDaemon, tempdir};

/// The delay every test below forces onto one git command: comfortably past the larger
/// of the two short bounds either call site would fall back to if reverted
/// (`CREATE_WINDOW_REPLY_TIMEOUT` = 7s; the other is `REQUEST_TIMEOUT` = 5s), and
/// comfortably under both `WORKTREE_REQUEST_TIMEOUT` (57s) and the daemon's own worst
/// case for either operation (create 45s, removal 33s).
const GIT_DELAY_SECS: u64 = 9;

/// How long each test lets the whole `anthrex` invocation run before giving up itself -
/// generous headroom over `GIT_DELAY_SECS` so this bound is never what fails the test.
fn patience() -> Duration {
    Duration::from_secs(GIT_DELAY_SECS + 15)
}

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
    let output = command.output().unwrap();
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

/// The real `git` this test process would otherwise run, resolved from the test's own
/// (unmodified) `PATH` before the daemon's `PATH` is overridden to see the wrapper
/// instead - so the wrapper can `exec` it once its delay, if any, is over.
fn real_git() -> PathBuf {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .unwrap();
    assert!(output.status.success(), "no git on PATH to wrap");
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim().to_string())
}

/// As [`slow_git_wrapper`], but with a delay for each of several subcommand/action pairs,
/// so one create can be made slow at more than one step. `action` is matched against `$5`
/// for a `worktree` subcommand and ignored otherwise — `("rev-parse", "")` delays project
/// detection, `("worktree", "add")` the checkout, `("worktree", "remove")` the cleanup
/// `discard_new` runs after a failed add.
fn slow_git_wrapper_multi(bin_dir: &Path, delays: &[(&str, &str, u64)]) {
    std::fs::create_dir_all(bin_dir).unwrap();
    let mut body = String::from("#!/bin/sh\n");
    for (subcommand, action, delay_secs) in delays {
        if action.is_empty() {
            body.push_str(&format!(
                "if [ \"$4\" = {subcommand} ]; then\n  sleep {delay_secs}\nfi\n"
            ));
        } else {
            body.push_str(&format!(
                "if [ \"$4\" = {subcommand} ] && [ \"$5\" = {action} ]; then\n  sleep {delay_secs}\nfi\n"
            ));
        }
    }
    body.push_str(&format!("exec \"{}\" \"$@\"\n", real_git().display()));
    let script = bin_dir.join("git");
    std::fs::write(&script, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
}

/// Writes a `git` into `bin_dir` that sleeps `delay_secs` only when invoked as `git -C
/// <dir> --no-optional-locks worktree <action> ...` - the exact shape
/// `crates/daemon/src/worktree.rs::run_git` always produces (`-C <dir>
/// --no-optional-locks <args...>`, design decision 2), so `$4`/`$5` are always that
/// invocation's subcommand and action - and otherwise `exec`s the real git untouched, so
/// project detection, `is_dirty`'s checks, `worktree list`/`prune` and every other
/// invocation this milestone makes stay fast.
fn slow_git_wrapper(bin_dir: &Path, action: &str, delay_secs: u64) {
    std::fs::create_dir_all(bin_dir).unwrap();
    let script = bin_dir.join("git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$4\" = worktree ] && [ \"$5\" = {action} ]; then\n  sleep {delay_secs}\nfi\nexec \"{}\" \"$@\"\n",
            real_git().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
}

fn injected_path(bin: &Path) -> OsString {
    let mut paths = vec![bin.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap()
}

/// Guards `new --worktree`'s `reply_timeout = client::WORKTREE_REQUEST_TIMEOUT` choice
/// (`crates/cli/src/main.rs`, the `worktree.is_some()` branch around line 171). Reverted
/// to `client::CREATE_WINDOW_REPLY_TIMEOUT` (7s), this must fail: the `git worktree add`
/// forced to take `GIT_DELAY_SECS` (9s) would blow past it.
#[test]
fn new_worktree_survives_a_slow_git_worktree_add() {
    let repo = init_repo();
    let bin = repo.path().join("slow-git-bin");
    slow_git_wrapper(&bin, "add", GIT_DELAY_SECS);
    let path = injected_path(&bin);

    let daemon = TestDaemon::start_configured(&[], |command| {
        command.env("PATH", path);
    });

    let output = daemon.anthrex_with_timeout(
        &[
            "new",
            "--runtime",
            "shell",
            "--name",
            "slow-wt-new",
            "--dir",
            repo.path().to_str().unwrap(),
            "--worktree",
            "feat/slow-add",
        ],
        patience(),
    );

    assert!(
        output.status.success(),
        "new --worktree must survive a {GIT_DELAY_SECS}s git worktree add, which clears \
         CREATE_WINDOW_REPLY_TIMEOUT (7s) but stays well under WORKTREE_REQUEST_TIMEOUT \
         (57s): status {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let id: u32 = std::str::from_utf8(&output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    let windows = daemon.client().windows();
    let window = windows
        .iter()
        .find(|w| w.id == id)
        .expect("created window must be listed");
    assert_eq!(window.branch.as_deref(), Some("feat/slow-add"));
    assert!(
        window.cwd.join(".git").exists(),
        "the linked worktree must actually exist on disk at {}",
        window.cwd.display()
    );

    drop(daemon);
}

/// Guards `rm --worktree`'s `client::WORKTREE_REQUEST_TIMEOUT` argument to
/// `request_with_timeout` (`crates/cli/src/main.rs`, around line 250). Reverted to the
/// default `client::REQUEST_TIMEOUT` (5s), this must fail: the `git worktree remove`
/// forced to take `GIT_DELAY_SECS` (9s) would blow past it.
#[test]
fn rm_worktree_survives_a_slow_git_worktree_remove() {
    let repo = init_repo();
    let bin = repo.path().join("slow-git-bin");
    slow_git_wrapper(&bin, "remove", GIT_DELAY_SECS);
    let path = injected_path(&bin);

    let daemon = TestDaemon::start_configured(&[], |command| {
        command.env("PATH", path);
    });

    // Setup: the wrapper only delays `worktree remove`, so this create runs at normal
    // speed and fits the default `anthrex()` patience.
    let created = daemon.anthrex(&[
        "new",
        "--runtime",
        "shell",
        "--name",
        "slow-wt-rm",
        "--dir",
        repo.path().to_str().unwrap(),
        "--worktree",
        "feat/slow-remove",
    ]);
    assert!(
        created.status.success(),
        "setup create failed with {}; stdout: {}; stderr: {}",
        created.status,
        String::from_utf8_lossy(&created.stdout),
        String::from_utf8_lossy(&created.stderr),
    );
    let id: u32 = std::str::from_utf8(&created.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let cwd = daemon
        .client()
        .windows()
        .into_iter()
        .find(|w| w.id == id)
        .expect("created window must be listed")
        .cwd;

    let output = daemon.anthrex_with_timeout(&["rm", &id.to_string(), "--worktree"], patience());

    assert!(
        output.status.success(),
        "rm --worktree must survive a {GIT_DELAY_SECS}s git worktree remove, which \
         clears the old REQUEST_TIMEOUT (5s) but stays well under \
         WORKTREE_REQUEST_TIMEOUT (57s): status {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        !cwd.exists(),
        "rm --worktree must actually delete the checkout: {}",
        cwd.display()
    );

    drop(daemon);
}

/// How long the daemon is forced to spend on one `new --worktree`, term by term, driving
/// the create path to the top of the budget `WORKTREE_REQUEST_TIMEOUT` is derived
/// against:
///
/// | daemon step | bound | how it is forced |
/// |---|---|---|
/// | `project::resolve_roots` | `DETECT_TIMEOUT` 5 s | `rev-parse` sleeps 4 s, so detection still *succeeds* |
/// | `worktree::create`'s git | `OPERATION_TIMEOUT` 30 s | `worktree add` sleeps 60 s and is killed at the deadline |
/// | `discard_new` after that failure | `CLEANUP_TIMEOUT` 10 s | `worktree prune` sleeps 30 s and is killed at its own |
///
/// The cleanup is slowed at `prune` rather than at `remove --force` because an add that
/// was killed never created the path, and `discard_new` skips the removal for a path that
/// is not there — `prune` is the step it always runs.
///
/// 4 + 30 + 10 ≈ **44 s**, against a derived supremum of 45 s. It cannot be pushed to 45:
/// a detection that actually reached `DETECT_TIMEOUT` would fall back, and a create whose
/// roots fell back is refused without touching git at all. That gap is exactly why the
/// budget has to *exceed* the sum rather than match it. Measured, this run takes ~44.8 s
/// end to end; under the old 45 s budget it had a few hundred milliseconds of margin, on
/// a machine where nothing else was competing for the disk. The test that actually
/// discriminates the two budgets is
/// `client::tests::the_budget_clears_the_daemons_worst_case_on_both_git_paths`, which
/// pins the arithmetic against the daemon's own constants; this one is the evidence that
/// the arithmetic describes something the daemon really does.
const SLOW_CREATE_DELAYS: &[(&str, &str, u64)] = &[
    ("rev-parse", "", 4),
    ("worktree", "add", 60),
    ("worktree", "prune", 30),
];

/// Guards the *size* of `client::WORKTREE_REQUEST_TIMEOUT`, which nothing did: the two
/// tests above would pass with any budget over 9 s.
///
/// The create here cannot succeed — its `git worktree add` is killed at the operation
/// deadline — and that is the point. What the CLI must not do is give up first and
/// report its own timeout, because the daemon's message is the only thing that says
/// whether a checkout was left on disk (design decision 16's two suffixes: `; the new
/// worktree was removed` means retry, `; cleanup failed: …` means go and delete it by
/// hand). A client-side timeout replaces that with nothing at all.
#[test]
fn new_worktree_waits_out_the_daemons_whole_create_budget() {
    let repo = init_repo();
    let bin = repo.path().join("slow-git-bin");
    slow_git_wrapper_multi(&bin, SLOW_CREATE_DELAYS);
    let path = injected_path(&bin);

    let daemon = TestDaemon::start_configured(&[], |command| {
        command.env("PATH", path);
    });

    let started = std::time::Instant::now();
    let output = daemon.anthrex_with_timeout(
        &[
            "new",
            "--runtime",
            "shell",
            "--name",
            "slow-wt-budget",
            "--dir",
            repo.path().to_str().unwrap(),
            "--worktree",
            "feat/slow-budget",
        ],
        Duration::from_secs(120),
    );
    let took = started.elapsed();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(
        !stderr.contains("timed out waiting for the daemon"),
        "the CLI gave up before the daemon did, after {took:?}: {stderr}"
    );
    assert!(
        !output.status.success(),
        "a `worktree add` killed at the operation deadline is a failed create: {stderr}"
    );
    assert!(
        stderr.contains("timed out") || stderr.contains("worktree add"),
        "the daemon's own account of the failure must survive to the user: {stderr}"
    );
    assert!(
        took >= Duration::from_secs(35),
        "the daemon was not actually driven near its budget; the wrapper's delays did \
         not take effect ({took:?}): {stderr}"
    );

    drop(daemon);
}
