//! Task M9.6: `run::git::summary`, the git reads behind `task_result` (decision 18) and
//! a review task's target (decision 36), against real temporary repositories. Every
//! call goes through `worktree::run_git` (`-C <dir> --no-optional-locks`, a scrubbed
//! environment): the recording stand-in's log is read back. `run_git_env.rs` covers
//! the same calls with the five variables planted in the environment.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use daemon::run::git::{resolve_target, task_summary};
use support::recording_git;
use support::run_git::{T, commit_file, head, out, real_git, repo};

const BRANCH: &str = "anthrex/sum1/t1";

/// A repository whose `BRANCH` has two commits over `base` (`src/a.rs`, `src/b.rs`),
/// and whose base branch moved on by one commit after it forked. Returns the base
/// branch's name and the fork point.
fn forked(root: &Path) -> (String, String) {
    let base_branch = out(root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let base = head(root);
    out(root, &["checkout", "-q", "-b", BRANCH]);
    commit_file(root, "src/a.rs", "pub fn a() {}\n", "add a");
    commit_file(root, "src/b.rs", "pub fn b() {}\n", "add b");
    out(root, &["checkout", "-q", &base_branch]);
    commit_file(root, "README", "two\n", "base moves on");
    (base_branch, base)
}

/// Each `argv` line of the stand-in's log, as its arguments.
fn calls(log_dir: &Path) -> Vec<Vec<String>> {
    let log = std::fs::read_to_string(log_dir.join("git.log")).unwrap();
    for env in log.lines().filter_map(|l| l.strip_prefix("env\t")) {
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_PREFIX",
        ] {
            assert!(!env.starts_with(&format!("{key}=")), "{key} reached git");
        }
    }
    log.lines()
        .filter_map(|l| l.strip_prefix("argv\t"))
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

fn assert_pinned(calls: &[Vec<String>], root: &Path) {
    assert!(!calls.is_empty());
    for argv in calls {
        assert_eq!(argv[0], "-C", "{argv:?}");
        assert_eq!(argv[1], root.display().to_string(), "{argv:?}");
        assert_eq!(argv[2], "--no-optional-locks", "{argv:?}");
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "core.hooksPath=/dev/null"),
            "a read that may run a hook: {argv:?}"
        );
    }
}

#[test]
fn task_summary_lists_commits_and_diffstat() {
    let repo = repo();
    let (_, base) = forked(&repo.root);
    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());
    let summary = task_summary(git.as_os_str(), &repo.root, &base, BRANCH, T).unwrap();
    let subjects: Vec<&str> = summary.commits.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(subjects, ["add b", "add a"]);
    for (sha, _) in &summary.commits {
        assert!(
            sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "{sha}"
        );
    }
    assert!(
        summary
            .diffstat
            .contains("2 files changed, 2 insertions(+)"),
        "{}",
        summary.diffstat
    );
    assert!(
        summary.diffstat.contains("src/a.rs"),
        "{}",
        summary.diffstat
    );
    let calls = calls(scripts.path());
    assert_pinned(&calls, &repo.root);
    let joined: Vec<String> = calls.iter().map(|a| a.join(" ")).collect();
    assert!(
        joined.iter().any(|a| a.contains(" log ")
            && a.ends_with(&format!(
                "--format=%h%x1f%s -n 50 --end-of-options {base}..{BRANCH}"
            ))),
        "{joined:?}"
    );
    assert!(
        joined.iter().any(|a| a.contains("diff")
            && a.contains(&format!("--stat=100 --end-of-options {base}...{BRANCH}"))),
        "{joined:?}"
    );
}

#[test]
fn diffstat_keeps_at_most_60_lines_and_its_total() {
    let repo = repo();
    let root = &repo.root;
    let base = head(root);
    out(root, &["checkout", "-q", "-b", BRANCH]);
    for i in 0..80 {
        support::run_git::write(root, &format!("f/{i:02}.txt"), "x\n");
    }
    out(root, &["add", "--", "f"]);
    out(root, &["commit", "-q", "-m", "eighty files"]);
    let summary = task_summary(real_git(), root, &base, BRANCH, T).unwrap();
    let lines: Vec<&str> = summary.diffstat.lines().collect();
    assert_eq!(lines.len(), 60, "{}", summary.diffstat);
    assert!(lines[59].contains("80 files changed"), "{}", lines[59]);
}

#[test]
fn resolve_target_of_a_range_and_of_a_single_revision() {
    let repo = repo();
    let (base_branch, fork) = forked(&repo.root);
    let tip = out(&repo.root, &["rev-parse", BRANCH]);
    let moved = head(&repo.root);
    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());
    let git = git.as_os_str();
    assert_eq!(
        resolve_target(
            git,
            &repo.root,
            &format!("{base_branch}..{BRANCH}"),
            &base_branch,
            T
        ),
        Ok((moved.clone(), tip.clone()))
    );
    // A single revision reviews what it adds over the base branch.
    assert_eq!(
        resolve_target(git, &repo.root, BRANCH, &base_branch, T),
        Ok((fork.clone(), tip.clone()))
    );
    // A short sha resolves to the full one.
    assert_eq!(
        resolve_target(
            git,
            &repo.root,
            &format!("{}..{}", &fork[..7], &tip[..9]),
            &base_branch,
            T
        ),
        Ok((fork, tip))
    );
    assert_pinned(&calls(scripts.path()), &repo.root);
}

#[test]
fn resolve_target_refuses_an_unknown_revision() {
    let repo = repo();
    let (base_branch, _) = forked(&repo.root);
    for target in ["nope", &format!("{base_branch}..nope"), "nope..HEAD"] {
        let error = resolve_target(real_git(), &repo.root, target, &base_branch, T).unwrap_err();
        assert!(!error.is_empty(), "{target}");
        assert!(error.contains("nope"), "{target}: {error}");
    }
    // A tree is not a commit.
    let tree = out(&repo.root, &["rev-parse", "HEAD^{tree}"]);
    assert!(resolve_target(real_git(), &repo.root, &tree, &base_branch, T).is_err());
}

#[test]
fn summary_times_out() {
    let repo = repo();
    let scripts = tempfile::tempdir().unwrap();
    let stand_in = scripts.path().join("slow-git");
    // It reads nothing and answers nothing; the deadline kills it (the exact child
    // `run_git` spawned, nothing else).
    std::fs::write(&stand_in, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
    let one = Duration::from_secs(1);
    let error = task_summary(stand_in.as_os_str(), &repo.root, "a", BRANCH, one).unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    let error = resolve_target(stand_in.as_os_str(), &repo.root, "a..b", "main", one).unwrap_err();
    assert!(error.contains("timed out"), "{error}");
}

/// Every regular file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// M9.6 review fix M-3: a `start` (or branch) that reads as an option is refused, and
/// never reaches git as one: `--output=<path>` writes no file.
#[test]
fn a_start_that_reads_as_an_option_is_refused() {
    let repo = repo();
    let (_, base) = forked(&repo.root);
    let out_dir = tempfile::tempdir().unwrap();
    // The parents of the files `git log` and `git diff` would write, so a forged
    // option would succeed if it reached git.
    let target = out_dir.path().join("o");
    for range in ["..", "..."] {
        let parent = format!("{}{range}{}", target.display(), BRANCH);
        std::fs::create_dir_all(Path::new(&parent).parent().unwrap()).unwrap();
    }
    let forged = format!("--output={}", target.display());
    let git = real_git();
    let result = task_summary(git, &repo.root, &forged, BRANCH, T);
    let error = result.expect_err("a start that reads as an option");
    assert!(
        error.contains("a revision cannot start with '-'"),
        "refused by the check, not by git: {error}"
    );
    let refused = task_summary(git, &repo.root, &base, "-p", T);
    let error = refused.expect_err("a branch that reads as an option");
    assert!(
        error.contains("a revision cannot start with '-'"),
        "refused by the check, not by git: {error}"
    );
    assert_eq!(
        files_under(out_dir.path()),
        Vec::<std::path::PathBuf>::new()
    );
}
