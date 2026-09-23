//! M8a.9: salvage and cleanup (decision 20) and deleting a run's branches. Accept is in
//! `run_git_accept.rs`. Split from `run_git_merge.rs` to keep each under AGENTS.md rule
//! 8's ~600 lines.

mod support;

use daemon::run::git::{delete_branches, prepare_worktree, remove_worktree, salvage};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{
    T, commit_file, head, out, real_git, repo, try_git, worktree_block, wrapper_git, write, wt_dir,
};

fn task_worktree(repo: &TempRepo, run: &str, task: &str) -> (tempfile::TempDir, PathBuf) {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/{task}"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/{task}"),
        &base,
        &path,
        T,
    )
    .unwrap();
    (keep, path)
}

fn salvage_refs(root: &Path) -> String {
    out(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/anthrex"],
    )
}

fn tree_names(dir: &Path, commit: &str) -> Vec<String> {
    // `-z`: without it git quotes a non-ASCII name.
    out(dir, &["ls-tree", "-r", "-z", "--name-only", commit])
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn salvage_of_a_clean_worktree_writes_nothing() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "sv01", "t1");
    commit_file(&task, "done.txt", "committed\n", "committed work");
    // An ignored file does not make a worktree dirty.
    write(&task, "ignored-build.log", "noise\n");

    let saved = salvage(
        real_git(),
        &task,
        "refs/anthrex/salvage/sv01/t1/1",
        "anthrex salvage sv01/t1",
        T,
    )
    .unwrap();
    assert_eq!(saved, None);
    assert_eq!(salvage_refs(&repo.root), "");

    // A lone untracked file is work to save, even when the user's config hides
    // untracked files from `git status`.
    out(&repo.root, &["config", "status.showUntrackedFiles", "no"]);
    write(&task, "notes.txt", "untracked\n");
    let reference = "refs/anthrex/salvage/sv01/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage sv01/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    assert!(tree_names(&repo.root, reference).contains(&"notes.txt".to_string()));
}

#[test]
fn salvage_captures_tracked_and_untracked_changes_but_not_ignored_files() {
    let repo = repo();
    commit_file(
        &repo.root,
        ".gitignore",
        "ignored-*\ntarget/\n",
        "ignore target",
    );
    commit_file(&repo.root, "staged.txt", "base\n", "staged file");
    let (_keep, task) = task_worktree(&repo, "sv02", "t1");
    let task_head = head(&task);
    write(&task, "README", "modified\n");
    write(&task, "staged.txt", "staged change\n");
    out(&task, &["add", "staged.txt"]);
    write(&task, "new dir/ü new.txt", "untracked\n");
    write(&task, "target/x", "ignored\n");

    let reference = "refs/anthrex/salvage/sv02/t1/1";
    let message = "anthrex salvage sv02/t1";
    let saved = salvage(real_git(), &task, reference, message, T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));

    let names = tree_names(&repo.root, reference);
    assert!(names.contains(&"README".to_string()), "{names:?}");
    assert!(
        names.contains(&"new dir/ü new.txt".to_string()),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.starts_with("target/")), "{names:?}");
    assert_eq!(
        out(&repo.root, &["show", &format!("{reference}:README")]),
        "modified"
    );
    assert_eq!(
        out(&repo.root, &["show", &format!("{reference}:staged.txt")]),
        "staged change"
    );
    assert_eq!(
        out(&repo.root, &["rev-list", "--parents", "-n", "1", reference]),
        format!("{} {task_head}", out(&repo.root, &["rev-parse", reference]))
    );
    assert_eq!(
        out(&repo.root, &["log", "-1", "--format=%B", reference]),
        message
    );
    // The task branch did not move.
    assert_eq!(head(&task), task_head);

    // Salvaging again to the same ref with the same content is the same salvage (a
    // replayed op); a second, different salvage takes the next sequence number, and
    // never overwrites the first.
    assert_eq!(
        salvage(real_git(), &task, reference, message, T)
            .unwrap()
            .as_deref(),
        Some(reference)
    );
    write(&task, "more.txt", "more\n");
    let first = out(&repo.root, &["rev-parse", reference]);
    assert!(salvage(real_git(), &task, reference, message, T).is_err());
    assert_eq!(out(&repo.root, &["rev-parse", reference]), first);
    let second = "refs/anthrex/salvage/sv02/t1/2";
    assert_eq!(
        salvage(real_git(), &task, second, message, T)
            .unwrap()
            .as_deref(),
        Some(second)
    );
    assert!(tree_names(&repo.root, second).contains(&"more.txt".to_string()));
}

#[test]
fn salvage_of_a_conflicted_hand_back_keeps_the_markers() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "sv03", "t1");
    commit_file(&task, "shared.txt", "task\n", "task");
    let run_head = commit_file(&repo.root, "shared.txt", "run\n", "run");
    assert!(
        !try_git(&task, &["merge", "--no-edit", &run_head])
            .status
            .success()
    );

    let reference = "refs/anthrex/salvage/sv03/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage sv03/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    let text = out(&repo.root, &["show", &format!("{reference}:shared.txt")]);
    assert!(text.contains("<<<<<<<"), "{text}");
}

#[test]
fn remove_refuses_nothing_after_salvage() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "rm01", "t1");
    assert!(
        worktree_block(&repo.root, &task)
            .unwrap()
            .lines()
            .any(|l| l.starts_with("locked")),
        "the task worktree is locked"
    );
    write(&task, "wip.txt", "uncommitted\n");
    write(&task, "README", "edited\n");

    let reference = "refs/anthrex/salvage/rm01/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage rm01/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    remove_worktree(real_git(), &repo.root, &task, T).unwrap();

    assert!(!task.exists());
    assert_eq!(worktree_block(&repo.root, &task), None);
    assert!(tree_names(&repo.root, reference).contains(&"wip.txt".to_string()));
    // The task branch is kept (decision 20: until accept or discard).
    assert!(repo.branch_exists("anthrex/rm01/t1"));

    // Removing it again, or a worktree deleted by hand, is not an error.
    remove_worktree(real_git(), &repo.root, &task, T).unwrap();
    let (_keep2, gone) = task_worktree(&repo, "rm01", "t2");
    std::fs::remove_dir_all(&gone).unwrap();
    remove_worktree(real_git(), &repo.root, &gone, T).unwrap();
    assert_eq!(worktree_block(&repo.root, &gone), None);
}

#[test]
fn delete_branches_removes_every_run_branch_but_keeps_salvage_refs() {
    let repo = repo();
    let base = head(&repo.root);
    for branch in [
        "anthrex/db01/integration",
        "anthrex/db01/t1",
        "anthrex/db01/t2",
        "anthrex/db010/integration",
        "feature",
    ] {
        out(&repo.root, &["branch", branch, &base]);
    }
    out(
        &repo.root,
        &["update-ref", "refs/anthrex/salvage/db01/t1/1", &base],
    );
    let branches = || out(&repo.root, &["for-each-ref", "--format=%(refname)"]);

    assert_eq!(
        delete_branches(real_git(), &repo.root, "anthrex/db01/", T).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        branches(),
        [
            "refs/anthrex/salvage/db01/t1/1",
            "refs/heads/anthrex/db010/integration",
            "refs/heads/feature",
            "refs/heads/main",
        ]
        .join("\n")
    );

    // Without the trailing slash it means the same prefix, and nothing is left to do.
    delete_branches(real_git(), &repo.root, "anthrex/db01", T).unwrap();
    out(&repo.root, &["branch", "anthrex/db01/t3", &base]);
    delete_branches(real_git(), &repo.root, "anthrex/db01", T).unwrap();
    assert!(!repo.branch_exists("anthrex/db01/t3"));
    assert!(repo.branch_exists("anthrex/db010/integration"));

    // An empty prefix would name every branch.
    assert!(delete_branches(real_git(), &repo.root, "", T).is_err());
    assert!(delete_branches(real_git(), &repo.root, "/", T).is_err());
    assert!(repo.branch_exists("main"));
}

#[test]
fn a_refused_salvage_leaves_the_index_untouched() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "sv04", "t1");
    let task_head = commit_file(&task, "shared.txt", "task\n", "task");
    let run_head = commit_file(&repo.root, "shared.txt", "run\n", "run");
    assert!(
        !try_git(&task, &["merge", "--no-edit", &run_head])
            .status
            .success()
    );
    // A caller that reused a `<seq>`: the ref holds other work.
    let reference = "refs/anthrex/salvage/sv04/t1/1";
    out(&repo.root, &["update-ref", reference, &task_head]);
    let unmerged = || out(&task, &["diff", "--name-only", "--diff-filter=U"]);
    assert_eq!(unmerged(), "shared.txt");

    let refused = salvage(real_git(), &task, reference, "anthrex salvage sv04/t1", T);
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(unmerged(), "shared.txt", "the conflict is still unresolved");
    assert_eq!(out(&repo.root, &["rev-parse", reference]), task_head);
}

#[test]
fn salvage_never_overwrites_a_ref_created_under_it() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "sv05", "t1");
    let task_head = head(&task);
    write(&task, "wip.txt", "uncommitted\n");
    // Another writer creates the salvage ref just before salvage's own `update-ref`.
    let tools = tempfile::tempdir().unwrap();
    let git = wrapper_git(
        tools.path(),
        r#"prev=""
for a in "$@"; do
  if [ "$prev" = "update-ref" ]; then
    case "$a" in refs/anthrex/salvage/*) "$REAL" -C "$2" update-ref "$a" HEAD || exit 99 ;; esac
  fi
  prev="$a"
done"#,
    );
    let reference = "refs/anthrex/salvage/sv05/t1/1";
    let result = salvage(
        git.as_os_str(),
        &task,
        reference,
        "anthrex salvage sv05/t1",
        T,
    );
    assert!(result.is_err(), "{result:?}");
    assert_eq!(out(&repo.root, &["rev-parse", reference]), task_head);
}

#[test]
fn delete_branches_skips_a_checked_out_branch_and_never_follows_a_symref() {
    let repo = repo();
    let base = head(&repo.root);
    // A symbolic ref under the prefix pointing at `main`: the link goes, `main` stays.
    out(
        &repo.root,
        &[
            "symbolic-ref",
            "refs/heads/anthrex/sy01/link",
            "refs/heads/main",
        ],
    );
    assert_eq!(
        delete_branches(real_git(), &repo.root, "anthrex/sy01", T).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(out(&repo.root, &["rev-parse", "refs/heads/main"]), base);
    assert!(
        !try_git(
            &repo.root,
            &[
                "rev-parse",
                "-q",
                "--verify",
                "refs/heads/anthrex/sy01/link"
            ]
        )
        .status
        .success()
    );

    // The user checked a run branch out in `root` to try it: it is skipped and named.
    out(&repo.root, &["branch", "anthrex/co01/integration", &base]);
    out(&repo.root, &["branch", "anthrex/co01/t1", &base]);
    out(&repo.root, &["checkout", "-q", "anthrex/co01/integration"]);
    let mine = commit_file(&repo.root, "mine.txt", "mine\n", "trying the run");
    assert_eq!(
        delete_branches(real_git(), &repo.root, "anthrex/co01", T).unwrap(),
        vec!["anthrex/co01/integration".to_string()]
    );
    assert!(repo.branch_exists("anthrex/co01/integration"));
    assert!(!repo.branch_exists("anthrex/co01/t1"));
    assert_eq!(head(&repo.root), mine);
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
}
