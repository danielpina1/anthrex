//! M8a.8: `verify_done` (decisions 32, 55 and 56), `count_commits` and `diff_so_far`,
//! against real task worktrees. Split from `run_git.rs` to keep both under AGENTS.md
//! rule 8's ~600 lines.

mod support;

use daemon::run::git::{DoneChecked, count_commits, diff_so_far, prepare_worktree, verify_done};
use daemon::run::globs::OwnsMatcher;
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
    let protected = OwnsMatcher::new(&strings(BUILTIN_PROTECTED)).unwrap();
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
    assert!(r.commits >= 1, "{r:?}");

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
        count_commits(real_git(), &t.path, &t.start, T).unwrap(),
        (0, t.start.clone())
    );
    commit_file(&t.path, "src/one.rs", "one\n", "one");
    let h = commit_file(&t.path, "src/two.rs", "two\n", "two");
    // Uncommitted work is not part of the diff so far.
    write(&t.path, "src/lib.rs", "uncommitted\n");

    assert_eq!(
        count_commits(real_git(), &t.path, &t.start, T).unwrap(),
        (2, h.clone())
    );
    let (stat, patch) = diff_so_far(real_git(), &t.path, &t.start, T).unwrap();
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
