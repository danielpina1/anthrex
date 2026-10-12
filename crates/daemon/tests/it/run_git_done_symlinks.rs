//! PR #60 review I2 (milestone 9.13 decision 1): a directory a protected entry lies
//! under, committed as a symbolic link, is a protected change, so its target cannot
//! carry a workflow or a cargo config past the done gate. Against real task worktrees.

use crate::support;

use daemon::run::git::{
    DoneChecked, RefreshedIn, count_commits_checked, prepare_worktree, verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::builtin_protected;
use std::path::PathBuf;
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, write, wt_dir};

struct Task {
    path: PathBuf,
    start: String,
    _repo: TempRepo,
    _wt: support::TestDir,
}

/// A task worktree of a fresh repository whose base commit holds `src/lib.rs`.
fn task(name: &str) -> Task {
    let repo = repo();
    commit_file(&repo.root, "src/lib.rs", "pub fn a() {}\n", "src");
    let (keep, wt) = wt_dir();
    let start = head(&repo.root);
    let path = wt.join(format!("runs/vd02/{name}"));
    let branch = format!("anthrex/vd02/{name}");
    prepare_worktree(real_git(), &repo.root, &branch, &start, &path, T).unwrap();
    Task {
        path,
        start,
        _repo: repo,
        _wt: keep,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// `verify_done` on `t` with the built-in protected list and `owns`.
fn check(t: &Task, owns: &[&str]) -> DoneChecked {
    let generated = OwnsMatcher::new(&[]).unwrap();
    let protected = ProtectedMatcher::new(&builtin_protected()).unwrap();
    let owns = strings(owns);
    let matchers = (&generated, &protected);
    verify_done(
        real_git(),
        &t.path,
        &t.start,
        &t.start,
        &owns,
        matchers.0,
        matchers.1,
        None,
        T,
    )
    .unwrap()
}

/// A task in a fresh repository that commits `link` as a symbolic link to `target`,
/// and `file` (under `target`) with `body`.
fn linked(name: &str, link: &str, target: &str, file: &str, body: &str) -> Task {
    let t = task(name);
    write(&t.path, file, body);
    let at = t.path.join(link);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, &at).unwrap();
    out(&t.path, &["add", "-A"]);
    out(&t.path, &["commit", "-q", "-m", "link"]);
    t
}

/// I2 (a): `crates/foo/.cargo` linked to `cfg`, whose `config.toml` sets a rustc
/// wrapper, with `owns` covering all of `crates/foo`: the link is protected.
#[test]
fn a_symlinked_cargo_dir_is_a_protected_change() {
    let wrapper = "[build]\nrustc-wrapper = \"./evil.sh\"\n";
    let t = linked(
        "cargo",
        "crates/foo/.cargo",
        "cfg",
        "crates/foo/cfg/config.toml",
        wrapper,
    );
    let r = check(&t, &["crates/foo/**"]);
    assert_eq!(
        r.protected_changed,
        strings(&["crates/foo/.cargo"]),
        "{r:?}"
    );
    let r = check(&t, &["crates/foo/**", "crates/foo/.cargo"]);
    assert!(r.protected_changed.is_empty(), "{r:?}");
}

/// I2 (b): `.github` linked to `x`, with `x/workflows/ci.yml`. Neither changed path
/// matches `.github/workflows/**`, but the link is an ancestor of it: protected, in the
/// done gate and in the override's count. Named exactly, it is a grant.
#[test]
fn a_symlinked_github_dir_is_a_protected_change() {
    let t = linked("github", ".github", "x", "x/workflows/ci.yml", "on: push\n");
    let r = check(&t, &["**"]);
    assert_eq!(r.protected_changed, strings(&[".github"]), "{r:?}");
    assert!(r.outside_owns.is_empty(), "{r:?}");
    let r = check(&t, &["x/**", ".github"]);
    assert!(r.protected_changed.is_empty(), "{r:?}");
    let ((count, _), protected) = count_commits_checked(
        real_git(),
        &t.path,
        (&t.start, &t.start),
        &RefreshedIn::default(),
        (&strings(&["**"]), &builtin_protected()),
        T,
    )
    .unwrap();
    assert_eq!((count, protected), (1, strings(&[".github"])));
}

/// A symlink that is no ancestor of a protected entry is an ordinary change: a linked
/// source directory, and a `.github` below the root (`.github/workflows/**` is rooted).
#[test]
fn a_symlink_elsewhere_is_not_refused() {
    let t = linked("elsewhere", "src/link", "lib", "src/lib/a.rs", "a\n");
    let r = check(&t, &["src/**"]);
    assert!(r.protected_changed.is_empty(), "{r:?}");
    assert!(r.outside_owns.is_empty(), "{r:?}");
    let t = linked(
        "nested",
        "docs/.github",
        "x",
        "docs/x/workflows/a.yml",
        "a\n",
    );
    let r = check(&t, &["docs/**"]);
    assert!(r.protected_changed.is_empty(), "{r:?}");
}
