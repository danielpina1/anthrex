use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script = dir.join(name);
    fs::write(&script, body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    script
}

#[test]
fn hash8_matches_fnv1a_vectors() {
    assert_eq!(hash8(Path::new("")), "811c9dc5");
    assert_eq!(hash8(Path::new("a")), "e40c292c");
    assert_eq!(hash8(Path::new("/tmp/repo")), "a3cbe2c8");
}

#[test]
fn branch_dir_name_replaces_slashes() {
    assert_eq!(branch_dir_name("feat/api/v2"), "feat-api-v2");
}

#[test]
fn repo_worktrees_dir_uses_sanitized_basename_and_hash() {
    let worktrees_root = Path::new("/data/worktrees");
    let project_root = Path::new("/tmp/my repo");

    let dir = repo_worktrees_dir(worktrees_root, project_root);

    let expected = worktrees_root.join(format!("my-repo-{}", hash8(project_root)));
    assert_eq!(dir, expected);
}

#[test]
fn branch_syntax_rules_and_messages() {
    let cases = [
        ("", "branch name is required"),
        ("   ", "branch name is required"),
        ("feat api", "branch name cannot contain spaces"),
        (" feat", "branch name cannot contain spaces"),
        ("-feat", "branch name cannot start with '-'"),
        (
            "anthrex/run",
            "branches under anthrex/ are reserved for orchestration runs",
        ),
    ];
    for (branch, expected) in cases {
        let error = check_branch_syntax(branch).unwrap_err();
        assert_eq!(error.to_string(), expected, "branch {branch:?}");
    }

    assert!(check_branch_syntax("feat/api").is_ok());
}

// `the_git_helper_passes_no_optional_locks_and_scrubs_the_environment` lives in
// `crates/daemon/tests/worktree_env.rs`, its own test binary, not here: it mutates
// the real process environment, and `a_missing_git_is_reported_as_such` below
// spawns a process on another libtest thread of *this* binary — a concurrent
// `environ` reader racing that mutation, which is undefined behaviour regardless of
// which variable either side touches. See that file's module doc comment.

#[test]
fn a_passed_deadline_fails_without_spawning() {
    let scripts = tempdir().unwrap();
    let argv_log = scripts.path().join("argv.log");
    let script = write_script(
        scripts.path(),
        "should-not-run",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            argv_log.display()
        ),
    );
    let dir = tempdir().unwrap();
    let deadline = Instant::now() - Duration::from_secs(1);

    let result = run_git(
        script.as_os_str(),
        dir.path(),
        &[OsStr::new("status")],
        deadline,
    );

    assert!(
        matches!(result, Err(WorktreeError::TimedOut { .. })),
        "{result:?}"
    );
    assert!(
        !argv_log.exists(),
        "the script must never have run for a deadline already in the past"
    );
}

#[test]
fn a_missing_git_is_reported_as_such() {
    let dir = tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);

    let result = run_git(
        OsStr::new("/definitely/missing/git-xyz"),
        dir.path(),
        &[OsStr::new("status")],
        deadline,
    );

    assert!(
        matches!(result, Err(WorktreeError::GitMissing)),
        "{result:?}"
    );
}

#[test]
fn stderr_tail_keeps_the_last_lines_within_the_cap() {
    let lines: Vec<String> = (1..=50).map(|n| format!("line {n}")).collect();
    let output = GitOutput {
        stdout: String::new(),
        stderr: lines.join("\n"),
        success: false,
    };

    let tail = output.stderr_tail();

    assert!(tail.starts_with("line 31"), "{tail:?}");
    assert_eq!(tail.lines().count(), 20);
    assert!(tail.ends_with("line 50"), "{tail:?}");

    let huge = GitOutput {
        stdout: String::new(),
        stderr: "x".repeat(5000),
        success: false,
    };

    let tail = huge.stderr_tail();

    assert_eq!(tail.chars().count(), STDERR_TAIL_CHARS);
}
