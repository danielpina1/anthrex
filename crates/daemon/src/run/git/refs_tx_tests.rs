//! Milestone 9.1 task M9.1.12: the all-or-nothing ref transaction and the create-only
//! stage branch, against a temporary git repository of their own (their own
//! `user.name`/`user.email`, git's global and system config out of the way).

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{alias_of, cas, create_branch};

const T: Duration = Duration::from_secs(20);

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A repository with two commits on `main`: `(dir, first, second)`.
fn repo() -> (tempfile::TempDir, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "anthrex test"]);
    git(dir, &["config", "user.email", "test@anthrex.invalid"]);
    git(dir, &["config", "core.logAllRefUpdates", "always"]);
    std::fs::write(dir.join("a"), "1\n").unwrap();
    git(dir, &["add", "a"]);
    git(dir, &["commit", "-q", "-m", "one"]);
    let first = git(dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("a"), "2\n").unwrap();
    git(dir, &["commit", "-q", "-am", "two"]);
    let second = git(dir, &["rev-parse", "HEAD"]);
    (tmp, first, second)
}

fn head_of(dir: &Path, refname: &str) -> String {
    git(dir, &["rev-parse", refname])
}

fn reflog_len(dir: &Path, refname: &str) -> usize {
    git(dir, &["reflog", "show", "--format=%H", refname])
        .lines()
        .count()
}

fn prog() -> &'static OsStr {
    OsStr::new("git")
}

#[test]
fn cas_updates_every_ref_or_none() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let (a, b) = (
        "refs/heads/anthrex/r1/stage-2",
        "refs/heads/anthrex/r1/integration",
    );
    git(dir, &["update-ref", a, &first]);
    git(dir, &["update-ref", b, &first]);
    let (ra, rb) = (reflog_len(dir, a), reflog_len(dir, b));
    // One expected value wrong: neither ref moves.
    let wrong = vec![
        (a.to_string(), second.clone(), first.clone()),
        (b.to_string(), second.clone(), second.clone()),
    ];
    assert!(!cas(prog(), dir, &wrong, T).unwrap());
    assert_eq!(head_of(dir, a), first);
    assert_eq!(head_of(dir, b), first);
    assert_eq!((reflog_len(dir, a), reflog_len(dir, b)), (ra, rb));
    // Both right: both move, with one reflog entry each.
    let right = vec![
        (a.to_string(), second.clone(), first.clone()),
        (b.to_string(), second.clone(), first.clone()),
    ];
    assert!(cas(prog(), dir, &right, T).unwrap());
    assert_eq!(head_of(dir, a), second);
    assert_eq!(head_of(dir, b), second);
    assert_eq!((reflog_len(dir, a), reflog_len(dir, b)), (ra + 1, rb + 1));
}

#[test]
fn cas_refuses_a_missing_ref_and_never_follows_a_symbolic_one() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let a = "refs/heads/anthrex/r1/stage-1";
    let missing = vec![(a.to_string(), second.clone(), first.clone())];
    assert!(!cas(prog(), dir, &missing, T).unwrap());
    assert!(
        git(dir, &["for-each-ref", "--format=%(refname)", a]).is_empty(),
        "a missing ref is never created"
    );
    // A branch made a symbolic ref to `main` is replaced, never followed onto `main`.
    git(dir, &["symbolic-ref", a, "refs/heads/main"]);
    let main_before = head_of(dir, "refs/heads/main");
    let over = vec![(a.to_string(), first.clone(), second.clone())];
    assert!(cas(prog(), dir, &over, T).unwrap());
    assert_eq!(head_of(dir, "refs/heads/main"), main_before);
    assert_eq!(head_of(dir, a), first);
}

#[test]
fn create_branch_is_create_only() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let branch = "anthrex/r1/stage-2";
    assert!(create_branch(prog(), dir, branch, &first, T).unwrap());
    assert_eq!(head_of(dir, &format!("refs/heads/{branch}")), first);
    // It exists now: a second create leaves it where it is, whatever it names.
    assert!(!create_branch(prog(), dir, branch, &second, T).unwrap());
    assert_eq!(head_of(dir, &format!("refs/heads/{branch}")), first);
}

#[test]
fn a_stage_branch_reattaches_to_the_integration_alias() {
    assert_eq!(alias_of("anthrex/r1/stage-2"), "anthrex/r1/integration");
    assert_eq!(alias_of("anthrex/r1/integration"), "anthrex/r1/integration");
    assert_eq!(alias_of("anthrex/r1/t1"), "anthrex/r1/t1");
}
