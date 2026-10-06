//! Milestone 9.7's final fix wave, FW-3 and FW-4 (review B I1 and m2): the `contains`
//! check when the counted merge commit is another stage's, and when GitHub deleted the
//! stage branch after the merge (its PR's `refs/pull/<n>/head` is fetched instead, into
//! the same private ref). Real git, a local bare repository as the remote.

use std::path::Path;

use super::allow::{AllowCtx, check};
use super::gh::run_ctx;
use super::tests_git::{RUN, Rig, commit, git};
use super::tests_git_contains::{
    Recorder, ask, base_fetch, fetched, has_ref, squash, stage_into, start, sub, user_clone,
    user_pushes_on_the_stage,
};
use super::*;

const PR: u64 = 7;

fn branch(n: u16) -> String {
    format!("anthrex/{RUN}/stage-{n}")
}

/// The question about stage `n`, whose PR is `pr`.
fn ask_stage(n: u16, head: &str, merged: &str, pr: Option<u64>) -> Contains {
    Contains {
        stage: n,
        branch: branch(n),
        into: format!("refs/anthrex/{RUN}/remote/stage-{n}"),
        pr,
        ..ask(head, merged)
    }
}

/// GitHub's `refs/pull/<pr>/head` at `sha` in the bare repository.
fn pull_ref(bare: &Path, pr: u64, sha: &str) {
    git(bare, &["update-ref", &format!("refs/pull/{pr}/head"), sha]);
}

/// The user deletes stage 1's branch on the remote (GitHub's auto-delete).
fn delete_stage_1(user: &Path) {
    let dst = format!("refs/heads/{}", branch(1));
    git(user, &["push", "-q", "origin", "--delete", &dst]);
}

/// FW-3 (review B I1): stage 1 merged with a merge commit and stage 2 squash-merged,
/// both seen in one fetch. The parents counted are stage 1's merge commit's, so they
/// say nothing about stage 2's merged head: its fetch is made after all.
#[test]
fn another_stages_merge_commit_does_not_skip_the_asked_stages_fetch() {
    let rig = Rig::new();
    commit(&rig.work, "a");
    git(&rig.work, &["push", "-q", "origin", "main"]);
    git(&rig.work, &["checkout", "-q", "-b", &branch(1)]);
    let h1 = commit(&rig.work, "h1");
    rig.push(1, &h1).unwrap();
    git(&rig.work, &["checkout", "-q", "-b", &branch(2)]);
    let h2 = commit(&rig.work, "h2");
    rig.push(2, &h2).unwrap();
    git(&rig.work, &["checkout", "-q", "main"]);
    let user = user_clone(&rig);
    git(&user, &["fetch", "-q", "origin"]);
    // Stage 1: a merge commit.
    git(&user, &["checkout", "-q", "-B", "main", "origin/main"]);
    let one = format!("origin/{}", branch(1));
    git(
        &user,
        &["merge", "-q", "--no-ff", "-m", "merge stage 1", &one],
    );
    let m = git(&user, &["rev-parse", "HEAD"]);
    // Stage 2: the user's commit on its branch, then a squash.
    git(
        &user,
        &[
            "checkout",
            "-q",
            "-B",
            "s",
            &format!("origin/{}", branch(2)),
        ],
    );
    let u2 = commit(&user, "u2");
    let dst = format!("HEAD:refs/heads/{}", branch(2));
    git(&user, &["push", "-q", "origin", &dst]);
    git(&user, &["checkout", "-q", "main"]);
    git(&user, &["merge", "-q", "--squash", "s"]);
    let s = commit(&user, "squash of stage 2");
    git(&user, &["push", "-q", "origin", "main"]);

    let answer = rig.fetch(&base_fetch(&rig, &m, ask_stage(2, &h2, &u2, Some(8))));
    assert_eq!(answer, fetched(&s, 2, Some(true)));
    let into = format!("refs/anthrex/{RUN}/remote/stage-2");
    assert_eq!(git(&rig.work, &["rev-parse", &into]), u2);
}

/// FW-4: GitHub deleted the squash-merged stage's branch; its PR's own ref still holds
/// the merged head, fetched into the same private ref, never anywhere else.
#[test]
fn a_deleted_stage_branch_reads_the_prs_own_ref() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    pull_ref(&rig.bare, PR, &u2);
    let s = squash(&user);
    delete_stage_1(&user);

    let recorder = Recorder::new(&rig);
    let c = ask_stage(1, &h2, &u2, Some(PR));
    let answer = recorder.host.fetch(&base_fetch(&rig, &s, c));
    assert_eq!(answer, fetched(&s, 1, Some(true)));
    assert_eq!(git(&rig.work, &["rev-parse", &stage_into()]), u2);
    let refs = git(&rig.work, &["for-each-ref", "--format=%(refname)"]);
    assert!(!refs.contains("refs/pull/"), "{refs}");
    let calls = recorder.calls();
    let subcommands: Vec<&str> = calls.iter().map(|argv| sub(argv)).collect();
    assert_eq!(
        subcommands,
        [
            "fetch",
            "rev-parse",
            "rev-list",
            "fetch",
            "fetch",
            "merge-base"
        ]
    );
    let pull = &calls[4];
    let spec = format!("+refs/pull/{PR}/head:{}", stage_into());
    assert!(pull.ends_with(&["origin".to_string(), spec]), "{pull:?}");
    // Allowed for this PR only: the run's own context without it refuses the fetch.
    let pulls = [PR];
    let ours = AllowCtx {
        pulls: &pulls,
        ..run_ctx(&rig.repo, Some(RUN))
    };
    assert_eq!(check(Program::Git, &pull[5..], &ours), Ok(()), "{pull:?}");
    let without = run_ctx(&rig.repo, Some(RUN));
    assert!(check(Program::Git, &pull[5..], &without).is_err());
}

/// FW-4: with neither the branch nor the PR's ref, the check could not be made.
#[test]
fn a_deleted_stage_branch_without_the_prs_ref_is_none() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);
    delete_stage_1(&user);
    for pr in [None, Some(PR)] {
        let answer = rig.fetch(&base_fetch(&rig, &s, ask_stage(1, &h2, &u2, pr)));
        assert_eq!(answer, fetched(&s, 1, None), "{pr:?}");
    }
    assert!(!has_ref(&rig.work, &stage_into()));
}

/// FW-4's allow-list: `refs/pull/<n>/head` only for this run's PR numbers, and only
/// into one of this run's `refs/anthrex/<run>/remote/stage-<k>`.
#[test]
fn the_pull_ref_fetch_is_allowed_for_the_runs_prs_into_its_stage_refs_only() {
    let repo = HostRepo {
        host: "github.com".into(),
        owner: "o".into(),
        name: "r".into(),
        remote: "origin".into(),
        root: "/tmp/x".into(),
    };
    let pulls = [PR];
    let ctx = AllowCtx {
        pulls: &pulls,
        ..run_ctx(&repo, Some(RUN))
    };
    let argv = |spec: &str| -> Vec<String> {
        let mut argv: Vec<String> = crate::run::git::WRITE_FLAGS
            .iter()
            .map(|s| s.to_string())
            .collect();
        argv.extend(
            [
                "fetch",
                "--no-tags",
                "--no-prune",
                "--no-prune-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                "origin",
                spec,
            ]
            .map(str::to_string),
        );
        argv
    };
    let allowed = |spec: &str| check(Program::Git, &argv(spec), &ctx).is_ok();
    assert!(allowed(&format!(
        "+refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/stage-1"
    )));
    assert!(allowed(&format!(
        "+refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/stage-12"
    )));
    for refused in [
        // Another run's PR number (not in this run's list).
        format!("+refs/pull/8/head:refs/anthrex/{RUN}/remote/stage-1"),
        format!("+refs/pull/{PR}0/head:refs/anthrex/{RUN}/remote/stage-1"),
        // Any destination outside the stage's private ref.
        format!("+refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/base"),
        format!("+refs/pull/{PR}/head:refs/anthrex/other/remote/stage-1"),
        format!("+refs/pull/{PR}/head:refs/heads/anthrex/{RUN}/stage-1"),
        format!("+refs/pull/{PR}/head:refs/heads/main"),
        format!("+refs/pull/{PR}/head:refs/pull/{PR}/head"),
        format!("+refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/stage-"),
        format!("+refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/stage-1/x"),
        // Another source shape, or no `+`.
        format!("+refs/pull/{PR}/merge:refs/anthrex/{RUN}/remote/stage-1"),
        format!("refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/stage-1"),
    ] {
        assert!(!allowed(&refused), "{refused}");
    }
    // Without the PR in the context, even the run's own shape is refused.
    let none = run_ctx(&repo, Some(RUN));
    let spec = format!("+refs/pull/{PR}/head:refs/anthrex/{RUN}/remote/stage-1");
    assert!(check(Program::Git, &argv(&spec), &none).is_err());
}
