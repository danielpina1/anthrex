//! M8a final fix batch F1: accept never rewinds a base it did not merge (D-1), reports
//! a merge that landed as merged even when git outlived its deadline (D-3), and merges
//! only the run head the engine verified (D-6). Split from `run_git_accept.rs` for
//! AGENTS.md rule 8.

mod support;

use daemon::run::git::{AcceptOutcome, accept, accept_with_merge_timeout};
use std::path::Path;
use std::time::Duration;
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo};

/// A run branch `anthrex/<run>/integration` one commit ahead of `main`. Returns
/// `(base_sha, run_head)`.
fn run_branch(repo: &TempRepo, run: &str) -> (String, String) {
    let base = head(&repo.root);
    let branch = format!("anthrex/{run}/integration");
    out(&repo.root, &["checkout", "-q", "-b", &branch]);
    let run_head = commit_file(&repo.root, "a.txt", "run\n", "run work");
    out(&repo.root, &["checkout", "-q", "main"]);
    (base, run_head)
}

fn parents(dir: &Path, commit: &str) -> Vec<String> {
    out(dir, &["rev-list", "--parents", "-n", "1", commit])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn accept_at(
    git: &std::ffi::OsStr,
    repo: &TempRepo,
    run: &str,
    expected_base: &str,
    expected_run_head: &str,
) -> Result<AcceptOutcome, String> {
    accept(
        git,
        &repo.root,
        "main",
        expected_base,
        &format!("anthrex/{run}/integration"),
        expected_run_head,
        &format!("anthrex: accept run {run}: x"),
        T,
    )
}

/// D-1: decision 20's advice after an accept conflict is "merge the run branch into the
/// base yourself". Accepting afterwards must leave that merge where it is.
#[test]
fn accept_after_the_user_merged_the_run_by_hand_leaves_their_merge() {
    let repo = repo();
    let (_base, run_head) = run_branch(&repo, "hm01");
    let user = commit_file(&repo.root, "other.txt", "user\n", "user work");
    out(
        &repo.root,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "my own merge",
            "anthrex/hm01/integration",
        ],
    );
    let merged = head(&repo.root);
    assert_eq!(parents(&repo.root, &merged), vec![user, run_head.clone()]);

    let outcome = accept_at(real_git(), &repo, "hm01", &merged, &run_head);
    assert_eq!(
        outcome,
        Ok(AcceptOutcome::Merged {
            commit: merged.clone()
        })
    );
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), merged);
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
}

/// D-3: git exits (or is killed) after the merge commit is on the base: `post-merge`
/// work, or a grandchild holding the pipe. That is a merge, not "the base is
/// untouched".
#[test]
fn accept_whose_merge_landed_before_its_deadline_is_merged() {
    use std::os::unix::fs::PermissionsExt;
    let repo = repo();
    let (base, run_head) = run_branch(&repo, "ld01");
    let tools = tempfile::tempdir().unwrap();
    let which = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(which.stdout).unwrap().trim().to_string();
    // The merge itself completes; then something it started keeps stdout open.
    let script = tools.path().join("slow-after-merge-git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\"{real}\" \"$@\"\nrc=$?\nfor a in \"$@\"; do [ \"$a\" = \"--no-ff\" ] && sleep 30; done\nexit $rc\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let outcome = accept_with_merge_timeout(
        script.as_os_str(),
        &repo.root,
        "main",
        &base,
        "anthrex/ld01/integration",
        &run_head,
        "anthrex: accept run ld01: x",
        Duration::from_secs(2),
        T,
    );
    let main = out(&repo.root, &["rev-parse", "main"]);
    assert_eq!(
        outcome,
        Ok(AcceptOutcome::Merged {
            commit: main.clone()
        })
    );
    assert_eq!(parents(&repo.root, &main), vec![base, run_head]);
}

/// D-6: the run branch moved after the engine verified the run (a leftover worker, a
/// quick fix in the integration worktree). Accept merges nothing unverified.
#[test]
fn accept_refuses_a_run_branch_that_moved_since_it_was_verified() {
    let repo = repo();
    let (base, run_head) = run_branch(&repo, "rh01");
    out(&repo.root, &["checkout", "-q", "anthrex/rh01/integration"]);
    let moved = commit_file(&repo.root, "b.txt", "late\n", "unverified");
    out(&repo.root, &["checkout", "-q", "main"]);

    let outcome = accept_at(real_git(), &repo, "rh01", &base, &run_head);
    assert_eq!(
        outcome,
        Err(format!(
            "refs/heads/anthrex/rh01/integration moved from {} to {}",
            &run_head[..7],
            &moved[..7]
        ))
    );
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), base);
    assert!(!repo.root.join("a.txt").exists());
}

/// Fix round 1, review N5: git's whitespace clean-up rewrites a goal with trailing
/// spaces or repeated blank lines. Accept still knows its own merge, and undoes it when
/// a commit landed on the base just before it.
#[test]
fn accept_knows_its_own_merge_whatever_the_goals_whitespace() {
    let repo = repo();
    let (base, run_head) = run_branch(&repo, "ws01");
    let tools = tempfile::tempdir().unwrap();
    let git = support::run_git::wrapper_git(
        tools.path(),
        r#"for a in "$@"; do
  if [ "$a" = "--no-ff" ]; then
    "$REAL" -C "$2" -c user.name=t -c user.email=t@t -c commit.gpgsign=false \
      -c core.hooksPath=/dev/null commit -q --allow-empty -m sneak || exit 99
    break
  fi
done"#,
    );
    let result = accept(
        git.as_os_str(),
        &repo.root,
        "main",
        &base,
        "anthrex/ws01/integration",
        &run_head,
        "anthrex: accept run ws01: fix  \n\n\n  the thing   ",
        T,
    );
    assert_eq!(
        result,
        Err("the base branch moved again; run accept again".to_string())
    );
    let main = out(&repo.root, &["rev-parse", "main"]);
    assert_eq!(
        out(&repo.root, &["log", "-1", "--format=%s", &main]),
        "sneak"
    );
    assert_eq!(
        parents(&repo.root, &main),
        vec![base],
        "the merge was undone"
    );
}
