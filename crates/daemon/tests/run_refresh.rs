//! Milestone 9 task M9.13a, decision 42e: a refresh's git side against real task
//! worktrees. The executor is M8a's hand-back (`merge-tree`, `commit-tree`, the CAS
//! `update-ref`; never `git merge`, never a rebase) with the refresh's clean-tree check
//! and merged-commit list (`git::hand_back_listing`). The reducer's side is
//! `run/engine/tests/refresh.rs`, the driver's acceptance check
//! `run/driver/refresh_tests.rs`; `run_git_env.rs` checks every refresh git call's
//! `--no-optional-locks` and scrubbed environment.

mod support;

use daemon::run::git::{
    MERGED_LIST_MAX, MergedCommits, RefreshedIn, UNCOMMITTED, count_commits,
    count_commits_excluding, diff_so_far, hand_back_listing, prepare_worktree, task_summary,
    task_summary_excluding, verify_done, verify_done_excluding,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::history_io::measure_diff;
use daemon::run::orch::contract::refresh_clean;
use daemon::run::plan::BUILTIN_PROTECTED;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, write, wt_dir};

const RUN: &str = "rf01";

/// The run branch, as the merge queue leaves it: `path` committed on it.
fn merge_on_run_branch(root: &Path, path: &str, content: &str, subject: &str) -> String {
    let branch = format!("anthrex/{RUN}/integration");
    let exists = try_git(root, &["rev-parse", "-q", "--verify", &branch])
        .status
        .success();
    let back = out(root, &["symbolic-ref", "--short", "HEAD"]);
    if exists {
        out(root, &["checkout", "-q", &branch]);
    } else {
        out(root, &["checkout", "-q", "-b", &branch]);
    }
    let sha = commit_file(root, path, content, subject);
    out(root, &["checkout", "-q", &back]);
    sha
}

/// A task worktree for `t1` at the repository's head: (keep, worktree, start).
fn task(repo: &TempRepo) -> (tempfile::TempDir, PathBuf, String) {
    let start = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{RUN}/t1"));
    let branch = format!("anthrex/{RUN}/t1");
    prepare_worktree(real_git(), &repo.root, &branch, &start, &path, T).unwrap();
    (keep, path, start)
}

fn base() -> TempRepo {
    let repo = repo();
    commit_file(&repo.root, "src/lib.rs", "pub fn a() {}\n", "base");
    repo
}

fn is_ancestor(dir: &Path, old: &str, new: &str) -> bool {
    try_git(dir, &["merge-base", "--is-ancestor", old, new])
        .status
        .success()
}

#[test]
fn refresh_merges_cleanly_and_the_next_turn_names_the_commits() {
    let repo = base();
    let (_keep, wt, _) = task(&repo);
    let own = commit_file(&wt, "src/own.rs", "own\n", "task work");
    merge_on_run_branch(&repo.root, "docs/a.md", "a\n", "docs: add a");
    let run_head = merge_on_run_branch(&repo.root, "docs/b.md", "b\n", "docs: add b");

    let (back, merged) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    assert_eq!(back.onto, own);
    assert_ne!(back.head, own, "a merge commit");
    assert_eq!(head(&wt), back.head);
    assert_eq!((merged.lines.len(), merged.total), (2, 2), "{merged:?}");
    assert!(
        merged.lines[0].ends_with(" docs: add b"),
        "newest first: {merged:?}"
    );
    assert!(merged.lines[1].ends_with(" docs: add a"), "{merged:?}");
    // What the worker's next turn says: every merged commit, so n is the list's length.
    let list: Vec<(String, String)> = merged
        .lines
        .iter()
        .map(|l| {
            let (sha, subject) = l.split_once(' ').unwrap();
            (sha.to_string(), subject.to_string())
        })
        .collect();
    let text = refresh_clean(merged.total as usize, &list);
    assert!(text.contains("(2 commits: "), "{text}");
    assert!(
        text.contains("docs: add b") && text.contains("docs: add a"),
        "{text}"
    );
    assert!(wt.join("docs/b.md").is_file(), "the files followed");
}

#[test]
fn refresh_never_rebases() {
    let repo = base();
    let (_keep, wt, _) = task(&repo);
    let old = commit_file(&wt, "src/own.rs", "own\n", "task work");
    let run_head = merge_on_run_branch(&repo.root, "docs/a.md", "a\n", "docs: add a");
    let (back, _) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    assert!(
        is_ancestor(&wt, &old, &back.head),
        "the old head is kept, not rewritten"
    );
    assert!(is_ancestor(&wt, &run_head, &back.head));
    let parents = out(&wt, &["rev-list", "--parents", "-n", "1", &back.head]);
    let parents: Vec<&str> = parents.split(' ').skip(1).collect();
    assert_eq!(
        parents,
        vec![old.as_str(), run_head.as_str()],
        "a two-parent merge"
    );
    let branch = out(&repo.root, &["rev-parse", &format!("anthrex/{RUN}/t1")]);
    assert_eq!(branch, back.head);
}

#[test]
fn refresh_conflict_leaves_markers_sets_resolving_and_does_not_count_a_conflict() {
    // The git side: markers and the engine's `MERGE_HEAD`, the branch unmoved. The
    // reducer sets `resolving` and leaves `Task.conflicts` alone
    // (`refresh_conflict_sets_resolving_and_leaves_conflicts_alone`).
    let repo = base();
    let (_keep, wt, _) = task(&repo);
    let own = commit_file(&wt, "src/lib.rs", "pub fn task() {}\n", "task work");
    let run_head = merge_on_run_branch(&repo.root, "src/lib.rs", "pub fn run() {}\n", "run");
    let (back, merged) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    assert_eq!(back.files, vec!["src/lib.rs".to_string()]);
    assert_eq!(back.head, own, "nothing is committed for the worker");
    assert_eq!(merged, MergedCommits::default());
    let text = std::fs::read_to_string(wt.join("src/lib.rs")).unwrap();
    assert!(
        text.contains("<<<<<<<") && text.contains(">>>>>>>"),
        "{text}"
    );
    assert_eq!(out(&wt, &["rev-parse", "MERGE_HEAD"]), run_head);
}

#[test]
fn refresh_dirty_at_the_boundary_fails_without_a_block() {
    let repo = base();
    let (_keep, wt, _) = task(&repo);
    let own = commit_file(&wt, "src/own.rs", "own\n", "task work");
    let run_head = merge_on_run_branch(&repo.root, "docs/a.md", "a\n", "docs: add a");
    // An unstaged tracked change, then a staged one: each fails the refresh.
    write(&wt, "src/own.rs", "edited\n");
    let error = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap_err();
    assert_eq!(error, UNCOMMITTED);
    out(&wt, &["add", "src/own.rs"]);
    let error = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap_err();
    assert_eq!(error, UNCOMMITTED);
    assert_eq!(head(&wt), own, "nothing moved");
    assert!(!wt.join("docs/a.md").exists());
    // An untracked file is not a tracked change.
    out(&wt, &["checkout", "--", "."]);
    out(&wt, &["reset", "-q", "--", "src/own.rs"]);
    out(&wt, &["checkout", "--", "src/own.rs"]);
    write(&wt, "notes.txt", "scratch\n");
    assert!(hand_back_listing(real_git(), &wt, &run_head, true, T).is_ok());
}

#[test]
fn net_diff_excludes_refreshed_commits() {
    let repo = base();
    let (_keep, wt, start) = task(&repo);
    commit_file(&wt, "src/own.rs", "own\n", "task work");
    // The merged file is outside the task's owns.
    let run_head = merge_on_run_branch(&repo.root, "docs/other.md", "x\n", "docs: other");
    let (back, _) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();

    let owns = vec!["src/own.rs".to_string()];
    let none = OwnsMatcher::new(&[]).unwrap();
    let builtin: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    let protected = ProtectedMatcher::new(&builtin).unwrap();
    let done = verify_done(
        real_git(),
        &wt,
        &start,
        &run_head,
        &owns,
        &none,
        &protected,
        None,
        T,
    )
    .unwrap();
    assert!(
        done.outside_owns.is_empty(),
        "no spill: {:?}",
        done.outside_owns
    );
    let stats = measure_diff(real_git(), &repo.root, &run_head, &back.head, true, T).unwrap();
    assert_eq!(stats.files, 1, "only the task's own file: {stats:?}");
    let (stat, _) = diff_so_far(real_git(), &wt, &start, &run_head, T).unwrap();
    assert!(
        stat.contains("src/own.rs") && !stat.contains("docs/other.md"),
        "{stat}"
    );
    // The commit count holds the task's commit and the refresh's merge commit; the
    // engine leaves the merge out (`own_commits`, `a_refresh_merge_alone_is_not_work`).
    let (count, _) = count_commits(real_git(), &wt, &start, &run_head, T).unwrap();
    assert_eq!((count, done.commits), (2, 2));
}

/// M9.13a review, item 6: a refresh of more commits than it lists counts them all.
#[test]
fn a_refresh_counts_every_merged_commit_and_lists_the_newest() {
    let repo = base();
    let (_keep, wt, _) = task(&repo);
    commit_file(&wt, "src/own.rs", "own\n", "task work");
    let mut run_head = String::new();
    for n in 0..25 {
        let path = format!("docs/{n}.md");
        run_head = merge_on_run_branch(&repo.root, &path, "x\n", &format!("docs: {n}"));
    }
    let (_, merged) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    assert_eq!(merged.total, 25);
    assert_eq!(merged.lines.len(), MERGED_LIST_MAX);
    assert!(merged.lines[0].ends_with(" docs: 24"), "{merged:?}");
}

/// A refreshed task: one own commit, then a refresh merge of one run commit. (keep,
/// worktree, start, run head, the merge).
fn refreshed(repo: &TempRepo) -> (tempfile::TempDir, PathBuf, String, String, String) {
    let (keep, wt, start) = task(repo);
    commit_file(&wt, "src/own.rs", "own\n", "task work");
    let run_head = merge_on_run_branch(&repo.root, "docs/other.md", "x\n", "docs: other");
    let (back, _) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    (keep, wt, start, run_head, back.head)
}

fn done_excluding(wt: &Path, start: &str, run_head: &str, refreshed: &RefreshedIn) -> u32 {
    let owns = vec!["src/**".to_string()];
    let none = OwnsMatcher::new(&[]).unwrap();
    let builtin: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    let protected = ProtectedMatcher::new(&builtin).unwrap();
    let matchers = (&none, &protected);
    let git = real_git();
    verify_done_excluding(
        git, wt, start, run_head, &owns, matchers, None, refreshed, T,
    )
    .unwrap()
    .commits
}

/// M9.13a review, item 3: the counts leave out a recorded merge the branch has, once
/// however often it is named; a merge alone is no commit.
#[test]
fn the_counts_leave_a_recorded_refresh_merge_out() {
    let repo = base();
    let (_keep, wt, start) = task(&repo);
    let run_head = merge_on_run_branch(&repo.root, "docs/other.md", "x\n", "docs: other");
    let (back, _) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    let merge = back.head;
    let twice = merges_only(&[merge.clone(), merge.clone()]);
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &run_head, &twice, T).unwrap();
    assert_eq!(count, 0, "the merge alone");
    assert_eq!(done_excluding(&wt, &start, &run_head, &twice), 0);
    commit_file(&wt, "src/own.rs", "own\n", "task work");
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &run_head, &twice, T).unwrap();
    assert_eq!(count, 1);
    assert_eq!(done_excluding(&wt, &start, &run_head, &twice), 1);
}

/// M9.13a review, item 3: a worker that rebases onto the run head after a refresh,
/// or resets below the merge, keeps the commit it made; subtracting the recorded
/// merges from the count used to lose it.
#[test]
fn a_rebase_or_reset_after_a_refresh_keeps_the_real_commit() {
    let repo = base();
    let (_keep, wt, start, run_head, merge) = refreshed(&repo);
    let not_own = merges_only(std::slice::from_ref(&merge));
    // Rebased onto the run head: one real commit, the merge gone.
    out(&wt, &["rebase", "-q", "--onto", &run_head, &start]);
    assert!(!is_ancestor(&wt, &merge, "HEAD"));
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &run_head, &not_own, T).unwrap();
    assert_eq!(count, 1);
    assert_eq!(done_excluding(&wt, &start, &run_head, &not_own), 1);

    let repo = base();
    let (_keep, wt, start, run_head, merge) = refreshed(&repo);
    let not_own = merges_only(std::slice::from_ref(&merge));
    // Reset below the merge, then a new commit.
    out(&wt, &["reset", "-q", "--hard", &start]);
    commit_file(&wt, "src/again.rs", "again\n", "task work again");
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &run_head, &not_own, T).unwrap();
    assert_eq!(count, 1);
    assert_eq!(done_excluding(&wt, &start, &run_head, &not_own), 1);
}

/// M9.13a review, item 2: `task_result`'s log and diffstat leave out the run work a
/// refresh merged in; before, they listed the run's commits and other tasks' files.
#[test]
fn task_result_leaves_out_refreshed_run_work() {
    let repo = base();
    let (_keep, wt, start, run_head, merge) = refreshed(&repo);
    let branch = format!("anthrex/{RUN}/t1");
    let naive = task_summary(real_git(), &repo.root, &start, &branch, T).unwrap();
    assert!(
        naive.diffstat.contains("docs/other.md"),
        "the bug: {naive:?}"
    );
    // As the engine records it: the merge, and the run head it merged (twice, left
    // out once).
    let merges = RefreshedIn::of(
        &[merge.clone(), merge],
        &[run_head.clone(), run_head.clone()],
    );
    let summary =
        task_summary_excluding(real_git(), &repo.root, &start, &branch, &merges, T).unwrap();
    let subjects: Vec<&str> = summary.commits.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(subjects, vec!["task work"], "{summary:?}");
    assert!(
        summary.diffstat.contains("src/own.rs"),
        "{}",
        summary.diffstat
    );
    assert!(
        !summary.diffstat.contains("docs/other.md"),
        "{}",
        summary.diffstat
    );
    // Own work after the refresh counts too.
    commit_file(&wt, "src/later.rs", "later\n", "later work");
    // The engine records the worker's `HEAD` on the branch as it counts.
    count_commits(real_git(), &wt, &start, &run_head, T).unwrap();
    let merges = RefreshedIn::of(
        &[head_parent_merge(&repo.root, &branch)],
        std::slice::from_ref(&run_head),
    );
    let summary =
        task_summary_excluding(real_git(), &repo.root, &start, &branch, &merges, T).unwrap();
    let subjects: Vec<&str> = summary.commits.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(subjects, vec!["later work", "task work"]);
    assert!(summary.diffstat.contains("src/later.rs") && !summary.diffstat.contains("docs/"));
}

/// The branch's first parent: the refresh merge under the newest commit.
fn head_parent_merge(root: &Path, branch: &str) -> String {
    out(root, &["rev-parse", &format!("{branch}^")])
}

/// Only merges recorded, as the first review's fixes had it.
fn merges_only(merges: &[String]) -> RefreshedIn {
    RefreshedIn::of(merges, &[])
}

/// A branch's commit subjects in `task_result` with `refreshed` left out.
fn result_of(root: &Path, start: &str, refreshed: &RefreshedIn) -> (Vec<String>, String) {
    let branch = format!("anthrex/{RUN}/t1");
    let summary = task_summary_excluding(real_git(), root, start, &branch, refreshed, T).unwrap();
    let subjects = summary.commits.into_iter().map(|(_, s)| s).collect();
    (subjects, summary.diffstat)
}

/// M9.13a re-review, item 1, the reviewer's scenario: a refresh that conflicts, which
/// the worker resolves and commits. The task's result has no run commit and no run
/// file; the worker's resolving merge is its own (recorded choice).
#[test]
fn a_conflicted_refresh_leaves_run_work_out_of_task_result() {
    let repo = base();
    let (_keep, wt, start) = task(&repo);
    commit_file(&wt, "src/lib.rs", "pub fn task() {}\n", "task: lib");
    merge_on_run_branch(&repo.root, "docs/other.md", "x\n", "run: docs");
    let run_head = merge_on_run_branch(&repo.root, "src/lib.rs", "pub fn run() {}\n", "run: lib");
    let (back, _) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    assert_eq!(back.files, vec!["src/lib.rs".to_string()]);
    write(&wt, "src/lib.rs", "pub fn task() {}\npub fn run() {}\n");
    out(&wt, &["add", "src/lib.rs"]);
    out(&wt, &["commit", "-q", "-m", "resolve"]);
    count_commits(real_git(), &wt, &start, &run_head, T).unwrap();

    // Before: no merge recorded for a conflicted refresh, so the run work was listed.
    let (subjects, stat) = result_of(&repo.root, &start, &RefreshedIn::default());
    assert!(subjects.iter().any(|s| s.starts_with("run: ")) && stat.contains("docs/other.md"));
    let refreshed = RefreshedIn::of(&[], std::slice::from_ref(&run_head));
    let (subjects, stat) = result_of(&repo.root, &start, &refreshed);
    assert_eq!(subjects, vec!["resolve", "task: lib"], "{stat}");
    assert!(!stat.contains("docs/other.md"), "{stat}");
    assert!(stat.contains("src/lib.rs"), "{stat}");
    // The counts: the task's commit and its resolving merge, no run commit, even with
    // the run head rewound to the start.
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &start, &refreshed, T).unwrap();
    assert_eq!(count, 2);
}

/// M9.13a re-review, item 2: after a refresh the worker rebases onto the run head. The
/// merge is gone, but the target is still the base of the diff.
#[test]
fn a_rebase_after_a_refresh_leaves_run_work_out_of_the_diffstat() {
    let repo = base();
    let (_keep, wt, start, run_head, merge) = refreshed(&repo);
    out(&wt, &["rebase", "-q", "--onto", &run_head, &start]);
    count_commits(real_git(), &wt, &start, &run_head, T).unwrap();
    let refreshed = RefreshedIn::of(&[merge], std::slice::from_ref(&run_head));
    let (subjects, stat) = result_of(&repo.root, &start, &refreshed);
    assert_eq!(subjects, vec!["task work"]);
    assert!(
        stat.contains("src/own.rs") && !stat.contains("docs/other.md"),
        "{stat}"
    );
}

/// M9.13a re-review, item 3: `resume --rebaseline` rewinds the run head below what a
/// refresh merged. A task whose only commit is the refresh's merge still counts 0.
#[test]
fn a_rewound_run_head_does_not_make_refreshed_run_work_count() {
    let repo = base();
    let (_keep, wt, start) = task(&repo);
    let run_head = merge_on_run_branch(&repo.root, "docs/other.md", "x\n", "docs: other");
    let (back, _) = hand_back_listing(real_git(), &wt, &run_head, true, T).unwrap();
    let rewound = start.clone();
    let merges_alone = merges_only(std::slice::from_ref(&back.head));
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &rewound, &merges_alone, T).unwrap();
    assert_eq!(count, 1, "the bug: the run commit counts");
    let refreshed = RefreshedIn::of(&[back.head], std::slice::from_ref(&run_head));
    let (count, _) =
        count_commits_excluding(real_git(), &wt, &start, &rewound, &refreshed, T).unwrap();
    assert_eq!(count, 0);
    assert_eq!(done_excluding(&wt, &start, &rewound, &refreshed), 0);
}
