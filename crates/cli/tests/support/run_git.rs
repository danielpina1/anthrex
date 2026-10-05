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

/// `git <args>` in `dir`, isolated from the user's configuration; its trimmed stdout.
pub fn git_in(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
