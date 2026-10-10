//! Milestone 9.11 decision 15 (A15): a task checkout's repository includes the user's
//! repository config by path, so an `includeIf "gitdir:…"` there was evaluated
//! against the checkout's own git directory and never matched: the worker committed
//! as the repository's base identity, not the one the user's git resolves. The
//! identity the user's repository resolves is carried into the checkout.

mod support;

use daemon::run::git::prepare_worktree;
use std::path::Path;
use support::run_git::{T, head, out, real_git, repo, wt_dir};

/// git in `dir` as the user's own git runs: no identity on the command line (the
/// shared helpers pass one), and no machine configuration.
fn plain(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn a_task_checkout_commits_as_the_users_includeif_identity() {
    let repo = repo();
    // The repository's own identity (from `repo()`): a different name and email.
    out(&repo.root, &["config", "user.email", "base@example.test"]);
    let included = repo.dir.path().join("included.gitconfig");
    std::fs::write(
        &included,
        "[user]\n\tname = Included Name\n\temail = inc@example.test\n",
    )
    .unwrap();
    // `gitdir:<root>/` (git appends `**`) matches `<root>/.git`; the task checkout's
    // own git directory is under anthrex's data directory, which it does not match.
    let condition = format!("includeIf.gitdir:{}/.path", repo.root.display());
    out(
        &repo.root,
        &["config", &condition, included.to_str().unwrap()],
    );
    // The user's own git resolves the included identity.
    assert_eq!(
        plain(&repo.root, &["config", "user.email"]),
        "inc@example.test"
    );

    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/id01/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/id01/t1", &base, &task, T).unwrap();

    assert_eq!(plain(&task, &["config", "user.email"]), "inc@example.test");
    assert_eq!(plain(&task, &["config", "user.name"]), "Included Name");
    // And a commit there (as the worker makes one) carries it.
    std::fs::write(task.join("w.txt"), "w\n").unwrap();
    plain(&task, &["add", "w.txt"]);
    plain(&task, &["commit", "-q", "-m", "worker"]);
    assert_eq!(
        plain(&task, &["log", "-1", "--format=%an <%ae> / %cn <%ce>"]),
        "Included Name <inc@example.test> / Included Name <inc@example.test>"
    );
}

#[test]
fn an_identity_with_quotes_and_backslashes_survives() {
    let repo = repo();
    out(
        &repo.root,
        &["config", "user.name", "A \"Quoted\" \\ Name; #1"],
    );
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/id02/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/id02/t1", &base, &task, T).unwrap();
    assert_eq!(
        plain(&task, &["config", "user.name"]),
        "A \"Quoted\" \\ Name; #1"
    );
}
