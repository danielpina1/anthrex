//! Final fix batch F1, finding D-2: a base branch that "advanced" onto the run's own
//! work is not the user committing on it. Decision 21's guard halts the run instead of
//! reporting an advance, and accept's listing (`run_work_on_base`) refuses such a base
//! unless the work is the run head itself, merged by the user.

mod support;

use daemon::run::git::{RefCheck, guard_refs, run_work_on_base};
use std::path::Path;
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo};

const RUN_ID: &str = "bg01";
const RUN: &str = "anthrex/bg01/integration";

/// `branch` created from `from` when missing, one commit on it, `main` checked out again.
fn commit_on(root: &Path, branch: &str, from: &str, file: &str) -> String {
    out(root, &["checkout", "-q", "-B", branch, from]);
    let sha = commit_file(
        root,
        file,
        &format!("{file}\n"),
        &format!("{branch}: {file}"),
    );
    out(root, &["checkout", "-q", "main"]);
    sha
}

/// A run at `base_sha` whose integration branch holds one merged task (`run_head`), and
/// a second task branch with a commit the run has not merged (`task2`).
struct Run {
    repo: TempRepo,
    base_sha: String,
    run_head: String,
    task2: String,
}

fn run() -> Run {
    let repo = repo();
    let base_sha = head(&repo.root);
    let task1 = commit_on(&repo.root, "anthrex/bg01/t1", &base_sha, "t1.txt");
    out(&repo.root, &["checkout", "-q", "-B", RUN, &base_sha]);
    out(
        &repo.root,
        &["merge", "-q", "--no-ff", "--no-edit", "anthrex/bg01/t1"],
    );
    let run_head = head(&repo.root);
    out(&repo.root, &["checkout", "-q", "main"]);
    let task2 = commit_on(&repo.root, "anthrex/bg01/t2", &run_head, "t2.txt");
    assert_ne!(task1, run_head);
    Run {
        repo,
        base_sha,
        run_head,
        task2,
    }
}

fn guard(r: &Run) -> RefCheck {
    guard_refs(
        real_git(),
        &r.repo.root,
        "main",
        &r.base_sha,
        RUN,
        &r.run_head,
        T,
    )
    .unwrap()
}

fn halt(n: u32) -> RefCheck {
    let noun = if n == 1 { "commit" } else { "commits" };
    RefCheck::Halt {
        reason: format!("refs/heads/main contains unaccepted run work ({n} {noun})"),
    }
}

#[test]
fn a_base_moved_onto_run_work_halts_the_run() {
    // `git update-ref refs/heads/main <task commit>` from a task worktree: the base
    // fast-forwards onto a worker's commit.
    let r = run();
    out(&r.repo.root, &["update-ref", "refs/heads/main", &r.task2]);
    out(&r.repo.root, &["reset", "-q", "--hard"]);
    assert_eq!(guard(&r), halt(3));

    // Onto the run head itself, merged work and all: still not the user's commit.
    let r = run();
    out(
        &r.repo.root,
        &["update-ref", "refs/heads/main", &r.run_head],
    );
    assert_eq!(guard(&r), halt(2));

    // Onto a salvage commit, under a user commit that hides it.
    let r = run();
    let salvage = commit_on(&r.repo.root, "scratch", &r.base_sha, "wip.txt");
    out(
        &r.repo.root,
        &["update-ref", "refs/anthrex/salvage/bg01/t3/1", &salvage],
    );
    out(&r.repo.root, &["branch", "-D", "scratch"]);
    out(&r.repo.root, &["merge", "-q", "--ff-only", &salvage]);
    let to = commit_file(&r.repo.root, "user.txt", "u\n", "user work");
    assert_ne!(to, salvage);
    assert_eq!(guard(&r), halt(1));

    // The user's own commit is an advance, as before.
    let r = run();
    let to = commit_file(&r.repo.root, "user.txt", "u\n", "user work");
    assert_eq!(guard(&r), RefCheck::BaseAdvanced { to, commits: 1 });

    // Another run's branches are not this run's work.
    let r = run();
    let other = commit_on(&r.repo.root, "anthrex/bg010/t1", &r.base_sha, "o.txt");
    out(&r.repo.root, &["merge", "-q", "--ff-only", &other]);
    assert_eq!(
        guard(&r),
        RefCheck::BaseAdvanced {
            to: other,
            commits: 1
        }
    );
}

#[test]
fn accept_allows_only_the_run_head_merged_by_the_user() {
    let count = |r: &Run, merged: Option<&str>| {
        let to = head(&r.repo.root);
        run_work_on_base(
            real_git(),
            &r.repo.root,
            RUN_ID,
            &r.base_sha,
            &to,
            merged,
            T,
        )
        .unwrap()
    };

    // The user merged the run by hand (decision 20's advice after a conflict).
    let r = run();
    commit_file(&r.repo.root, "user.txt", "u\n", "user work");
    out(&r.repo.root, &["merge", "-q", "--no-ff", "--no-edit", RUN]);
    assert_eq!(count(&r, Some(&r.run_head)), 0);
    assert_eq!(count(&r, None), 2, "without the allowance it is run work");

    // A task commit the run never merged is refused even then.
    out(
        &r.repo.root,
        &["merge", "-q", "--no-ff", "--no-edit", &r.task2],
    );
    assert_eq!(count(&r, Some(&r.run_head)), 1);

    // Part of the run (a task commit the run head contains) without the run head: the
    // allowance is for the run head merged whole, so this is refused.
    let r = run();
    let task1 = out(&r.repo.root, &["rev-parse", "anthrex/bg01/t1"]);
    out(&r.repo.root, &["merge", "-q", "--ff-only", &task1]);
    assert_eq!(count(&r, Some(&r.run_head)), 1);

    // Nothing of the run's: nothing to refuse.
    let r = run();
    commit_file(&r.repo.root, "user.txt", "u\n", "user work");
    assert_eq!(count(&r, Some(&r.run_head)), 0);
}
