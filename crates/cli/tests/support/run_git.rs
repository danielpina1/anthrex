//! The run harness's git helpers (moved out of `run_harness.rs` unchanged by the final
//! fix wave, W3): a test repository's initial commit, and git isolated from the user's
//! configuration.

use std::path::Path;
use std::process::Command;

/// Initialises a repository at `path` with one commit of `README` (and `files`).
pub fn init_repo(path: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(path).unwrap();
    git_in(path, &["init", "-q", "-b", "main"]);
    git_in(path, &["config", "user.name", "Test User"]);
    git_in(path, &["config", "user.email", "test@example.com"]);
    git_in(path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join("README"), "readme\n").unwrap();
    for (name, content) in files {
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }
    git_in(path, &["add", "-A"]);
    git_in(path, &["commit", "-q", "-m", "initial"]);
}

/// `git <args>` in `dir`, isolated from the user's configuration and with AGENTS.md
/// hard rule 11's environment scrubbed (the final fix wave's FW-63); its output, which
/// must be a success.
fn isolated_git(dir: &Path, args: &[&str]) -> std::process::Output {
    let mut git = Command::new("git");
    git.args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
    ] {
        git.env_remove(var);
    }
    let output = git.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// `git <args>` in `dir`, isolated from the user's configuration; its trimmed stdout.
pub fn git_in(dir: &Path, args: &[&str]) -> String {
    let output = isolated_git(dir, args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// [`git_in`] as a reader (`--no-optional-locks`, AGENTS.md hard rule 11): its stdout
/// byte for byte, never trimmed, as UTF-8.
pub fn git_raw_in(dir: &Path, args: &[&str]) -> String {
    let mut all = vec!["--no-optional-locks"];
    all.extend_from_slice(args);
    let output = isolated_git(dir, &all);
    String::from_utf8(output.stdout).expect("git's output is UTF-8")
}
