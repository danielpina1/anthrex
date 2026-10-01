//! Milestone 9.1 task M9.1.12: the all-or-nothing ref transaction and the create-only
//! stage branch, against a temporary git repository of their own (their own
//! `user.name`/`user.email`, git's global and system config out of the way).

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{Swap, alias_of, cas, create_branch, free_ref, salvage_and_move, symbolic};
use crate::run::git::{RefCheck, guard_refs};

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
    // Controller ruling C-15 (M-2): the refusal names the ref that differed.
    assert_eq!(
        cas(prog(), dir, &wrong, &[], T).unwrap(),
        Swap::Moved(b.to_string())
    );
    assert_eq!(head_of(dir, a), first);
    assert_eq!(head_of(dir, b), first);
    assert_eq!((reflog_len(dir, a), reflog_len(dir, b)), (ra, rb));
    // Both right: both move, with one reflog entry each.
    let right = vec![
        (a.to_string(), second.clone(), first.clone()),
        (b.to_string(), second.clone(), first.clone()),
    ];
    assert_eq!(cas(prog(), dir, &right, &[], T).unwrap(), Swap::Done);
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
    assert_eq!(
        cas(prog(), dir, &missing, &[], T).unwrap(),
        Swap::Moved(a.to_string())
    );
    assert!(
        git(dir, &["for-each-ref", "--format=%(refname)", a]).is_empty(),
        "a missing ref is never created"
    );
    // A branch made a symbolic ref to `main` is replaced, never followed onto `main`.
    git(dir, &["symbolic-ref", a, "refs/heads/main"]);
    let main_before = head_of(dir, "refs/heads/main");
    let over = vec![(a.to_string(), first.clone(), second.clone())];
    assert_eq!(cas(prog(), dir, &over, &[], T).unwrap(), Swap::Done);
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

/// Controller ruling C-15 (M-3): a guarded ref that is not moved is verified in the
/// same transaction as the swap. It moves between the guard and the swap, and the
/// swap is refused, with no ref moved.
#[test]
fn a_guarded_ref_that_moves_after_the_guard_refuses_the_swap() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let main = head_of(dir, "refs/heads/main");
    let (s1, s2, int) = (
        "anthrex/r1/stage-1",
        "anthrex/r1/stage-2",
        "anthrex/r1/integration",
    );
    for b in [s1, s2, int] {
        git(dir, &["branch", b, &first]);
    }
    let guarded: Vec<(String, String)> = [s1, s2, int]
        .map(|b| (b.to_string(), first.clone()))
        .to_vec();
    let checked = guard_refs(prog(), dir, "main", &main, &guarded, T).unwrap();
    assert_eq!(checked, RefCheck::Ok);
    // Stage 2 moves after the guard; the merge into stage 1 verifies it.
    git(dir, &["update-ref", &format!("refs/heads/{s2}"), &second]);
    let updates = [(format!("refs/heads/{s1}"), second.clone(), first.clone())];
    let verifies = [
        (format!("refs/heads/{s2}"), first.clone()),
        (format!("refs/heads/{int}"), first.clone()),
    ];
    assert_eq!(
        cas(prog(), dir, &updates, &verifies, T).unwrap(),
        Swap::Moved(format!("refs/heads/{s2}"))
    );
    assert_eq!(head_of(dir, &format!("refs/heads/{s1}")), first);
    assert_eq!(head_of(dir, &format!("refs/heads/{int}")), first);
    // Where every verified ref is, the swap lands.
    let verifies = [(format!("refs/heads/{int}"), first.clone())];
    assert_eq!(
        cas(prog(), dir, &updates, &verifies, T).unwrap(),
        Swap::Done
    );
    assert_eq!(head_of(dir, &format!("refs/heads/{s1}")), second);
}

/// Controller ruling C-15 (I-1): `integration` goes back to the highest stage's head,
/// and the commit it was moved to is salvaged, in one transaction.
#[test]
fn salvage_and_move_keeps_the_old_commit_and_moves_only_from_the_value_read() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let int = "anthrex/r1/integration";
    let salvage = "refs/anthrex/salvage/r1/_integration-1700000000";
    git(dir, &["branch", int, &second]);
    // Read at `first`, but it is at `second`: nothing moves, nothing is salvaged.
    let moved = salvage_and_move(prog(), dir, salvage, int, (&first, &first), &[], T).unwrap();
    assert_eq!(moved, Swap::Moved(format!("refs/heads/{int}")));
    assert!(git(dir, &["for-each-ref", "--format=%(refname)", salvage]).is_empty());
    let done = salvage_and_move(prog(), dir, salvage, int, (&first, &second), &[], T).unwrap();
    assert_eq!(done, Swap::Done);
    assert_eq!(head_of(dir, &format!("refs/heads/{int}")), first);
    assert_eq!(head_of(dir, salvage), second);
    // A salvage ref is never overwritten.
    git(dir, &["update-ref", &format!("refs/heads/{int}"), &second]);
    let again = salvage_and_move(prog(), dir, salvage, int, (&first, &second), &[], T).unwrap();
    assert_eq!(again, Swap::Moved(salvage.to_string()));
    assert_eq!(head_of(dir, &format!("refs/heads/{int}")), second);
}

#[test]
fn symbolic_names_a_symbolic_ref_only() {
    let (tmp, first, _) = repo();
    let dir = tmp.path();
    let (plain, sym) = (
        "refs/heads/anthrex/r1/stage-1",
        "refs/heads/anthrex/r1/stage-2",
    );
    git(dir, &["update-ref", plain, &first]);
    git(dir, &["symbolic-ref", sym, "refs/heads/main"]);
    assert_eq!(symbolic(prog(), dir, plain, T).unwrap(), None);
    assert_eq!(symbolic(prog(), dir, "refs/heads/nope", T).unwrap(), None);
    assert_eq!(
        symbolic(prog(), dir, sym, T).unwrap(),
        Some("refs/heads/main".to_string())
    );
}

/// Controller ruling C-16 (3): every stage ref the rebaseline read is verified in the
/// salvage's transaction. The top stage moving between the read and the move refuses
/// the whole of it: `integration` stays, and nothing is salvaged.
#[test]
fn salvage_and_move_refuses_when_a_stage_moved_after_the_read() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let (int, s2) = ("anthrex/r1/integration", "anthrex/r1/stage-2");
    let salvage = "refs/anthrex/salvage/r1/_integration-5";
    git(dir, &["branch", int, &second]);
    git(dir, &["branch", s2, &first]);
    // Read at `first`; it moves to `second` before the move.
    git(dir, &["update-ref", &format!("refs/heads/{s2}"), &second]);
    let verifies = [(format!("refs/heads/{s2}"), first.clone())];
    let moved =
        salvage_and_move(prog(), dir, salvage, int, (&first, &second), &verifies, T).unwrap();
    assert_eq!(moved, Swap::Moved(format!("refs/heads/{s2}")));
    assert_eq!(head_of(dir, &format!("refs/heads/{int}")), second);
    assert!(git(dir, &["for-each-ref", "--format=%(refname)", salvage]).is_empty());
}

/// Controller ruling C-16 (4): two salvages in one second take `-2`, `-3`, ….
#[test]
fn free_ref_picks_the_first_free_suffix() {
    let (tmp, first, _) = repo();
    let dir = tmp.path();
    let base = "refs/anthrex/salvage/r1/_integration-100";
    assert_eq!(free_ref(prog(), dir, base, T).unwrap(), base);
    git(dir, &["update-ref", base, &first]);
    assert_eq!(free_ref(prog(), dir, base, T).unwrap(), format!("{base}-2"));
    git(dir, &["update-ref", &format!("{base}-2"), &first]);
    assert_eq!(free_ref(prog(), dir, base, T).unwrap(), format!("{base}-3"));
}

/// Controller ruling C-16 (5): git takes a symbolic ref whose target is at the value
/// expected (`verify` and `update --no-deref` both succeed), so a symbolic ref is never
/// why a transaction is refused. The read-back names the ref that really moved, even
/// with a symbolic one listed before it.
#[test]
fn a_refusal_names_the_moved_ref_not_a_symbolic_one() {
    let (tmp, first, second) = repo();
    let dir = tmp.path();
    let (sym, s2, s1) = (
        "refs/heads/anthrex/r1/stage-3",
        "refs/heads/anthrex/r1/stage-2",
        "refs/heads/anthrex/r1/stage-1",
    );
    git(dir, &["update-ref", "refs/heads/main", &first]);
    git(dir, &["symbolic-ref", sym, "refs/heads/main"]);
    git(dir, &["update-ref", s2, &second]);
    git(dir, &["update-ref", s1, &first]);
    // Alone, the symbolic ref at the value expected passes.
    let updates = [(s1.to_string(), second.clone(), first.clone())];
    let only_sym = [(sym.to_string(), first.clone())];
    assert_eq!(
        cas(prog(), dir, &updates, &only_sym, T).unwrap(),
        Swap::Done
    );
    git(dir, &["update-ref", s1, &first]);
    let verifies = [
        (sym.to_string(), first.clone()),
        (s2.to_string(), first.clone()),
    ];
    assert_eq!(
        cas(prog(), dir, &updates, &verifies, T).unwrap(),
        Swap::Moved(s2.to_string())
    );
    assert_eq!(head_of(dir, s1), first);
}
