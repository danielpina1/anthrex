//! M8a.9: accept onto the base branch (decision 20): its preconditions, the `--no-ff`
//! merge onto the recorded or an advanced base, a conflict against an advanced base, a
//! base that moves again, a merge or rebase the user already has in progress, and a
//! merge that outlives its deadline. Split from `run_git_finish.rs` to keep each under
//! AGENTS.md rule 8's ~600 lines.

mod support;

use daemon::run::git::{ACCEPT_MERGE_TIMEOUT, AcceptOutcome, accept, accept_with_merge_timeout};
use std::path::Path;
use std::time::Duration;
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wrapper_git, write};

/// A run branch `anthrex/<run>/integration` one commit ahead of `main`'s current head,
/// changing `file`. Returns `(base_sha, run_head)`.
fn run_branch(repo: &TempRepo, run: &str, file: &str, content: &str) -> (String, String) {
    let base = head(&repo.root);
    let branch = format!("anthrex/{run}/integration");
    out(&repo.root, &["checkout", "-q", "-b", &branch]);
    let run_head = commit_file(&repo.root, file, content, "run work");
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

fn accept_run(repo: &TempRepo, run: &str, expected_base: &str) -> Result<AcceptOutcome, String> {
    accept(
        real_git(),
        &repo.root,
        "main",
        expected_base,
        &format!("anthrex/{run}/integration"),
        &format!("anthrex: accept run {run}: do the thing"),
        T,
    )
}

#[test]
fn accept_requires_the_base_branch_and_a_clean_tree() {
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "ac01", "a.txt", "run\n");

    out(&repo.root, &["checkout", "-q", "-b", "feature"]);
    assert_eq!(
        accept_run(&repo, "ac01", &base),
        Err(format!(
            "check out main in {} first (currently feature)",
            repo.root.display()
        ))
    );
    out(&repo.root, &["checkout", "-q", "--detach"]);
    assert_eq!(
        accept_run(&repo, "ac01", &base),
        Err(format!(
            "check out main in {} first (currently a detached HEAD)",
            repo.root.display()
        ))
    );
    out(&repo.root, &["checkout", "-q", "main"]);

    write(&repo.root, "README", "uncommitted\n");
    assert_eq!(
        accept_run(&repo, "ac01", &base),
        Err(format!(
            "the working tree at {} has uncommitted changes; commit or stash them first",
            repo.root.display()
        ))
    );
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), base);
    assert!(!repo.root.join("a.txt").exists());

    // Untracked files alone do not block (decision 17's check is of tracked files).
    out(&repo.root, &["checkout", "--", "README"]);
    write(&repo.root, "notes.txt", "mine\n");
    assert!(matches!(
        accept_run(&repo, "ac01", &base),
        Ok(AcceptOutcome::Merged { .. })
    ));
}

#[test]
fn accept_merges_no_ff() {
    let repo = repo();
    let (base, run_head) = run_branch(&repo, "ac02", "a.txt", "run\n");

    let outcome = accept_run(&repo, "ac02", &base).unwrap();
    let AcceptOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), commit);
    assert_eq!(parents(&repo.root, &commit), vec![base, run_head]);
    assert_eq!(
        out(&repo.root, &["log", "-1", "--format=%B", &commit]),
        "anthrex: accept run ac02: do the thing"
    );
    assert_eq!(
        out(&repo.root, &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
    assert_eq!(out(&repo.root, &["show", "HEAD:a.txt"]), "run");
}

#[test]
fn accept_onto_an_advanced_base_merges_no_ff() {
    let repo = repo();
    let (_base, run_head) = run_branch(&repo, "ac03", "a.txt", "run\n");
    let advanced = commit_file(&repo.root, "other.txt", "user\n", "user work");

    let outcome = accept_run(&repo, "ac03", &advanced).unwrap();
    let AcceptOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), commit);
    assert_eq!(parents(&repo.root, &commit), vec![advanced, run_head]);
    assert!(repo.root.join("a.txt").exists());
    assert!(repo.root.join("other.txt").exists());
}

#[test]
fn accept_conflict_with_an_advanced_base_aborts_and_leaves_base_untouched() {
    let repo = repo();
    let (_base, _run_head) = run_branch(&repo, "ac04", "README", "run line\n");
    let advanced = commit_file(&repo.root, "README", "user line\n", "user edit");
    write(&repo.root, "notes.txt", "untracked, mine\n");
    let status_before = out(&repo.root, &["status", "--porcelain"]);

    let outcome = accept_run(&repo, "ac04", &advanced).unwrap();
    assert_eq!(
        outcome,
        AcceptOutcome::Conflict {
            files: vec!["README".to_string()]
        }
    );
    assert_eq!(out(&repo.root, &["rev-parse", "refs/heads/main"]), advanced);
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), status_before);
    assert!(
        !try_git(&repo.root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("README")).unwrap(),
        "user line\n"
    );
}

#[test]
fn accept_refuses_when_the_base_moved_again() {
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "ac05", "a.txt", "run\n");
    let moved = commit_file(&repo.root, "other.txt", "user\n", "user work");

    assert_eq!(
        accept_run(&repo, "ac05", &base),
        Err("the base branch moved again; run accept again".to_string())
    );
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), moved);
    assert!(!repo.root.join("a.txt").exists());
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
}

fn in_progress(repo: &TempRepo, kind: &str) -> String {
    format!(
        "a {kind} is in progress in {}; finish or abort it first",
        repo.root.display()
    )
}

#[test]
fn accept_refuses_while_a_merge_cherry_pick_or_rebase_is_in_progress() {
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "ip01", "a.txt", "run\n");

    // The user's own merge, concluded in their favour but not yet committed: the tracked
    // tree is clean, so only `MERGE_HEAD` says a merge is pending. Accept must neither
    // merge over it nor abort it.
    out(&repo.root, &["branch", "side", &base]);
    out(&repo.root, &["checkout", "-q", "side"]);
    let side = commit_file(&repo.root, "side.txt", "side\n", "side");
    out(&repo.root, &["checkout", "-q", "main"]);
    out(
        &repo.root,
        &["merge", "-q", "--no-commit", "-s", "ours", "side"],
    );
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
    assert_eq!(
        accept_run(&repo, "ip01", &base),
        Err(in_progress(&repo, "merge"))
    );
    assert_eq!(out(&repo.root, &["rev-parse", "MERGE_HEAD"]), side);
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), base);
    out(&repo.root, &["merge", "--abort"]);

    // A stopped cherry-pick.
    out(&repo.root, &["checkout", "-q", "-b", "pick", &base]);
    let pick = commit_file(&repo.root, "README", "pick\n", "pick");
    out(&repo.root, &["checkout", "-q", "main"]);
    let moved = commit_file(&repo.root, "README", "main\n", "main edit");
    assert!(
        !try_git(&repo.root, &["cherry-pick", &pick])
            .status
            .success()
    );
    assert_eq!(
        accept_run(&repo, "ip01", &moved),
        Err(in_progress(&repo, "cherry-pick"))
    );
    assert_eq!(out(&repo.root, &["rev-parse", "CHERRY_PICK_HEAD"]), pick);
    out(&repo.root, &["cherry-pick", "--abort"]);

    // A stopped rebase of `main` (HEAD is detached while it runs).
    assert!(!try_git(&repo.root, &["rebase", "pick"]).status.success());
    assert_eq!(
        accept_run(&repo, "ip01", &moved),
        Err(in_progress(&repo, "rebase"))
    );
    out(&repo.root, &["rebase", "--abort"]);
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), moved);
}

#[test]
fn accept_merge_that_outlives_its_deadline_is_aborted() {
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(ACCEPT_MERGE_TIMEOUT, Duration::from_secs(600));
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "to01", "a.txt", "run\n");
    write(&repo.root, "notes.txt", "untracked, mine\n");
    let status_before = out(&repo.root, &["status", "--porcelain"]);

    // The user's signing runs inside accept's merge (decision 18 keeps it; their hooks
    // no longer run there, final fix batch F1), after git has merged the index and the
    // tree; a signing program waiting for a touch outlives the merge's deadline.
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().join("signing-started");
    let signer = tools.path().join("slow-gpg");
    std::fs::write(
        &signer,
        format!("#!/bin/sh\n: > '{}'\nsleep 30\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&signer, std::fs::Permissions::from_mode(0o755)).unwrap();
    out(&repo.root, &["config", "commit.gpgSign", "true"]);
    out(
        &repo.root,
        &["config", "gpg.program", signer.to_str().unwrap()],
    );

    let result = accept_with_merge_timeout(
        real_git(),
        &repo.root,
        "main",
        &base,
        "anthrex/to01/integration",
        "anthrex: accept run to01: slow signing",
        Duration::from_secs(2),
        T,
    );
    let err = result.expect_err("the merge was killed at its deadline");
    assert!(err.contains("timed out"), "{err}");
    assert!(
        marker.exists(),
        "the merge was inside its signing when it was killed"
    );
    assert!(
        !try_git(&repo.root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success(),
        "the half-done merge was aborted"
    );
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), status_before);
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), base);
    assert!(!repo.root.join("a.txt").exists());
}

#[test]
fn accept_undoes_its_merge_when_the_base_moved_under_it() {
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "mv01", "a.txt", "run\n");
    // A commit lands on `main` in `root` after accept checked `expected_base` and just
    // before its merge runs.
    let tools = tempfile::tempdir().unwrap();
    let git = wrapper_git(
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
        "anthrex/mv01/integration",
        "anthrex: accept run mv01: x",
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
        "only the merge was undone"
    );
    assert!(!repo.root.join("a.txt").exists());
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
}

#[test]
fn accept_merges_the_run_branch_not_a_tag_of_the_same_name() {
    let repo = repo();
    let (base, run_head) = run_branch(&repo, "tg01", "a.txt", "run\n");
    out(&repo.root, &["checkout", "-q", "-b", "decoy", &base]);
    let decoy = commit_file(&repo.root, "decoy.txt", "decoy\n", "decoy");
    out(&repo.root, &["checkout", "-q", "main"]);
    out(&repo.root, &["tag", "anthrex/tg01/integration", &decoy]);

    let outcome = accept_run(&repo, "tg01", &base).unwrap();
    let AcceptOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(parents(&repo.root, &commit), vec![base, run_head]);
    assert!(!repo.root.join("decoy.txt").exists());
}
