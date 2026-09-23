//! M8a.8: `verify_done` (decisions 32, 55 and 56), `count_commits` and `diff_so_far`,
//! against real task worktrees. Split from `run_git.rs` to keep both under AGENTS.md
//! rule 8's ~600 lines.

mod support;

use daemon::run::git::{DoneChecked, count_commits, diff_so_far, prepare_worktree, verify_done};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use std::path::PathBuf;
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, write, wt_dir};

struct Task {
    path: PathBuf,
    start: String,
}

fn task(repo: &TempRepo, wt: &std::path::Path, name: &str) -> Task {
    let start = head(&repo.root);
    let path = wt.join(format!("runs/vd01/{name}"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/vd01/{name}"),
        &start,
        &path,
        T,
    )
    .unwrap();
    Task { path, start }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn check(
    t: &Task,
    run_head: &str,
    owns: &[&str],
    generated: &[&str],
    red: Option<&str>,
) -> DoneChecked {
    let generated = OwnsMatcher::new(&strings(generated)).unwrap();
    let protected = ProtectedMatcher::new(&strings(BUILTIN_PROTECTED)).unwrap();
    verify_done(
        real_git(),
        &t.path,
        &t.start,
        run_head,
        &strings(owns),
        &generated,
        &protected,
        red,
        T,
    )
    .unwrap()
}

#[test]
fn verify_done_reports_each_condition() {
    let repo = repo();
    commit_file(&repo.root, "src/lib.rs", "pub fn a() {}\n", "src");
    let (_keep, wt) = wt_dir();

    // No commits.
    let t = task(&repo, &wt, "none");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(
        r,
        DoneChecked {
            head: t.start.clone(),
            head_branch: Some("anthrex/vd01/none".to_string()),
            ..DoneChecked::default()
        }
    );

    // One commit inside owns.
    let t = task(&repo, &wt, "one");
    let one = commit_file(&t.path, "src/one.rs", "1\n", "one");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!((r.commits, r.head.as_str()), (1, one.as_str()));
    assert!(
        r.outside_owns.is_empty() && r.protected_changed.is_empty(),
        "{r:?}"
    );

    // Dirty tracked files: two of them.
    let t = task(&repo, &wt, "dirty");
    commit_file(&t.path, "src/d.rs", "d\n", "d");
    write(&t.path, "README", "changed\n");
    write(&t.path, "src/lib.rs", "changed\n");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.dirty_tracked, 2, "{r:?}");
    assert!(!r.merge_in_progress);

    // A merge in progress.
    let t = task(&repo, &wt, "merging");
    out(&t.path, &["checkout", "-q", "-b", "side-vd"]);
    commit_file(&t.path, "src/lib.rs", "side\n", "side");
    out(&t.path, &["checkout", "-q", "anthrex/vd01/merging"]);
    commit_file(&t.path, "src/lib.rs", "mine\n", "mine");
    assert!(!try_git(&t.path, &["merge", "side-vd"]).status.success());
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert!(r.merge_in_progress, "{r:?}");

    // Untracked files: only the one inside owns is reported.
    let t = task(&repo, &wt, "untracked");
    commit_file(&t.path, "src/u.rs", "u\n", "u");
    write(&t.path, "src/new/forgot.rs", "forgot\n");
    write(&t.path, "notes.txt", "outside\n");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.untracked_in_owns, strings(&["src/new/forgot.rs"]));
    assert_eq!(r.dirty_tracked, 0);

    // A committed file outside owns.
    let t = task(&repo, &wt, "outside");
    commit_file(&t.path, "src/o.rs", "o\n", "o");
    commit_file(&t.path, "docs/x.md", "x\n", "docs");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.outside_owns, strings(&["docs/x.md"]));
    assert!(r.generated_outside_owns.is_empty());

    // A generated file outside owns, then inside owns.
    let t = task(&repo, &wt, "generated");
    commit_file(&t.path, "src/g.rs", "g\n", "g");
    commit_file(&t.path, "Cargo.lock", "lock\n", "lock");
    let r = check(&t, &t.start, &["src/**"], &["Cargo.lock"], None);
    assert_eq!(r.generated_outside_owns, strings(&["Cargo.lock"]));
    assert!(r.outside_owns.is_empty(), "{r:?}");
    let r = check(
        &t,
        &t.start,
        &["src/**", "Cargo.lock"],
        &["Cargo.lock"],
        None,
    );
    assert!(
        r.generated_outside_owns.is_empty() && r.outside_owns.is_empty(),
        "{r:?}"
    );

    // Protected paths.
    let t = task(&repo, &wt, "agents");
    commit_file(&t.path, "AGENTS.md", "rules\n", "agents");
    let r = check(&t, &t.start, &["**"], &[], None);
    assert_eq!(r.protected_changed, strings(&["AGENTS.md"]));
    let r = check(&t, &t.start, &["AGENTS.md"], &[], None);
    assert!(
        r.protected_changed.is_empty() && r.outside_owns.is_empty(),
        "{r:?}"
    );
    let t = task(&repo, &wt, "claude");
    commit_file(&t.path, ".claude/settings.json", "{}", "settings");
    let r = check(&t, &t.start, &[".claude/**"], &[], None);
    assert_eq!(r.protected_changed, strings(&[".claude/settings.json"]));
    let t = task(&repo, &wt, "nested");
    commit_file(&t.path, "src/n.rs", "n\n", "n");
    commit_file(&t.path, "docs/AGENTS.md", "nested\n", "nested agents");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.protected_changed, strings(&["docs/AGENTS.md"]));
    assert!(r.outside_owns.is_empty(), "{r:?}");

    // After a newer run head is merged in, its files are not the task's.
    let t = task(&repo, &wt, "handback");
    commit_file(&t.path, "src/h.rs", "h\n", "h");
    let run_head = commit_file(&repo.root, "other/theirs.txt", "theirs\n", "run moved");
    out(&t.path, &["merge", "-q", "--no-edit", &run_head]);
    let r = check(&t, &run_head, &["src/**"], &[], None);
    assert!(r.outside_owns.is_empty(), "{r:?}");
    // The task's own commit and the merge; not the run head's commit.
    assert_eq!(r.commits, 2, "{r:?}");

    // red: the start commit, a commit on another branch, a valid one.
    let t = task(&repo, &wt, "red");
    let red = commit_file(&t.path, "src/red_test.rs", "#[test] fn t() {}\n", "red");
    commit_file(&t.path, "src/green.rs", "green\n", "green");
    out(&repo.root, &["branch", "elsewhere-vd", &t.start]);
    let elsewhere = {
        let other = wt.join("elsewhere");
        out(
            &repo.root,
            &[
                "worktree",
                "add",
                "-q",
                other.to_str().unwrap(),
                "elsewhere-vd",
            ],
        );
        commit_file(&other, "e.txt", "e\n", "elsewhere")
    };
    let start = t.start.clone();
    assert_eq!(
        check(&t, &start, &["src/**"], &[], Some(&start)).red_ok,
        Some(false)
    );
    assert_eq!(
        check(&t, &start, &["src/**"], &[], Some(&elsewhere)).red_ok,
        Some(false)
    );
    assert_eq!(
        check(&t, &start, &["src/**"], &[], Some(&red)).red_ok,
        Some(true)
    );
    assert_eq!(
        check(&t, &start, &["src/**"], &[], Some("not-a-commit")).red_ok,
        Some(false)
    );
}

#[test]
fn count_commits_and_diff_so_far() {
    let repo = repo();
    commit_file(&repo.root, "src/lib.rs", "pub fn a() {}\n", "src");
    let (_keep, wt) = wt_dir();
    let t = task(&repo, &wt, "count");
    assert_eq!(
        count_commits(real_git(), &t.path, &t.start, &t.start, T).unwrap(),
        (0, t.start.clone())
    );
    commit_file(&t.path, "src/one.rs", "one\n", "one");
    let h = commit_file(&t.path, "src/two.rs", "two\n", "two");
    // Uncommitted work is not part of the diff so far.
    write(&t.path, "src/lib.rs", "uncommitted\n");

    assert_eq!(
        count_commits(real_git(), &t.path, &t.start, &t.start, T).unwrap(),
        (2, h.clone())
    );
    let (stat, patch) = diff_so_far(real_git(), &t.path, &t.start, &t.start, T).unwrap();
    let range = format!("{}..{h}", t.start);
    assert_eq!(
        stat,
        String::from_utf8(try_git(&t.path, &["diff", "--stat", &range]).stdout).unwrap()
    );
    assert!(
        stat.contains("src/one.rs") && stat.contains("src/two.rs"),
        "{stat}"
    );
    assert_eq!(
        patch,
        String::from_utf8(try_git(&t.path, &["diff", &range]).stdout).unwrap()
    );
    assert!(!patch.contains("uncommitted"), "{patch}");
}

/// Stages a gitlink at `path` pointing at `sha`.
fn gitlink(dir: &std::path::Path, sha: &str, path: &str) {
    let info = format!("160000,{sha},{path}");
    out(dir, &["update-index", "--add", "--cacheinfo", &info]);
}

/// Ruling T14-R3 (R2-1): a gitlink change is a changed path for the owns and protected
/// checks, whatever `.gitmodules` (`ignore = all`) or the configuration
/// (`diff.ignoreSubmodules=all`) says about submodules.
#[test]
fn verify_done_sees_gitlinks_that_config_ignores() {
    let repo = repo();
    let base = head(&repo.root);
    write(
        &repo.root,
        ".gitmodules",
        "[submodule \"vendor/lib\"]\n\tpath = vendor/lib\n\turl = ./a\n\tignore = all\n\
         [submodule \".claude/skills\"]\n\tpath = .claude/skills\n\turl = ./b\n\tignore = all\n",
    );
    out(&repo.root, &["add", ".gitmodules"]);
    gitlink(&repo.root, &base, "vendor/lib");
    gitlink(&repo.root, &base, ".claude/skills");
    out(&repo.root, &["commit", "-q", "-m", "submodules"]);
    let newer = commit_file(&repo.root, "src/lib.rs", "pub fn a() {}\n", "src");
    let (_keep, wt) = wt_dir();

    // Both gitlinks moved, ignored by `.gitmodules`.
    let t = task(&repo, &wt, "gitmodules");
    gitlink(&t.path, &newer, "vendor/lib");
    gitlink(&t.path, &newer, ".claude/skills");
    out(&t.path, &["commit", "-q", "-m", "move the submodules"]);
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.outside_owns, strings(&["vendor/lib"]), "{r:?}");
    assert_eq!(r.protected_changed, strings(&[".claude/skills"]), "{r:?}");

    // A new gitlink, with `diff.ignoreSubmodules=all` in the repository's config.
    out(&repo.root, &["config", "diff.ignoreSubmodules", "all"]);
    let t = task(&repo, &wt, "config");
    gitlink(&t.path, &newer, "vendor/new");
    out(&t.path, &["commit", "-q", "-m", "add a submodule"]);
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.outside_owns, strings(&["vendor/new"]), "{r:?}");
}

#[test]
fn verify_done_edge_cases() {
    let repo = repo();
    commit_file(&repo.root, "src/lib.rs", "pub fn a() {}\n", "src");
    commit_file(&repo.root, "docs/guide.md", "guide\n", "docs");
    let (_keep, wt) = wt_dir();

    // Protected matching ignores case: on a case-insensitive file system these are the
    // files Claude Code and Codex read.
    let t = task(&repo, &wt, "case");
    commit_file(&t.path, "agents.md", "lower\n", "lower agents");
    commit_file(&t.path, ".Claude/settings.json", "{}", "mixed claude");
    let r = check(&t, &t.start, &["**"], &[], None);
    assert_eq!(
        r.protected_changed,
        strings(&[".Claude/settings.json", "agents.md"])
    );

    // A rename from outside owns into owns reports the source (`--no-renames`); a
    // rename out of owns reports the destination.
    let t = task(&repo, &wt, "rename-in");
    out(&t.path, &["mv", "docs/guide.md", "src/guide.md"]);
    out(&t.path, &["commit", "-q", "-m", "move in"]);
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.outside_owns, strings(&["docs/guide.md"]));
    let t = task(&repo, &wt, "rename-out");
    out(&t.path, &["mv", "src/lib.rs", "docs/lib.rs"]);
    out(&t.path, &["commit", "-q", "-m", "move out"]);
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.outside_owns, strings(&["docs/lib.rs"]));

    // A rename in the working tree is one dirty file, not two.
    let t = task(&repo, &wt, "wt-rename");
    std::fs::rename(t.path.join("src/lib.rs"), t.path.join("src/moved.rs")).unwrap();
    out(&t.path, &["add", "-N", "src/moved.rs"]);
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.dirty_tracked, 1, "{r:?}");

    // The branch HEAD is on: another branch, then detached.
    let t = task(&repo, &wt, "switched");
    out(&t.path, &["switch", "-q", "-c", "scratch-vd"]);
    commit_file(&t.path, "src/s.rs", "s\n", "on scratch");
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.head_branch.as_deref(), Some("scratch-vd"));
    out(&t.path, &["switch", "-q", "--detach"]);
    let r = check(&t, &t.start, &["src/**"], &[], None);
    assert_eq!(r.head_branch, None);
}

#[test]
fn count_and_diff_so_far_leave_out_a_merged_run_head() {
    let repo = repo();
    let (_keep, wt) = wt_dir();
    let t = task(&repo, &wt, "handed-back");
    commit_file(&t.path, "src/mine.rs", "mine\n", "mine");
    commit_file(&repo.root, "other/o1.txt", "o1\n", "o1");
    let run_head = commit_file(&repo.root, "other/o2.txt", "o2\n", "o2");
    out(&t.path, &["merge", "-q", "--no-edit", &run_head]);
    let h = head(&t.path);

    assert_eq!(
        count_commits(real_git(), &t.path, &t.start, &run_head, T).unwrap(),
        (2, h)
    );
    let (stat, patch) = diff_so_far(real_git(), &t.path, &t.start, &run_head, T).unwrap();
    assert!(stat.contains("src/mine.rs"), "{stat}");
    assert!(
        !stat.contains("o1.txt") && !stat.contains("o2.txt"),
        "{stat}"
    );
    assert!(patch.contains("+mine") && !patch.contains("+o1"), "{patch}");
}
