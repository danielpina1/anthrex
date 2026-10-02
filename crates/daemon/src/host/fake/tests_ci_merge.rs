//! Task M9.2.5: `FakeHost`'s scripted CI, the user's merges on the bare base, GitHub's
//! retargeting when a merged head branch is deleted, and mergeability. The rig is in
//! `tests.rs`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::tests::{Rig, git, head};
use super::*;
use crate::host::{
    CheckStatus, CodeHost, Conclusion, DeleteBranchReq, HostError, Mergeable, PrState,
};

fn parents(rig: &Rig, sha: &str) -> Vec<String> {
    let line = git(&rig.bare, &["rev-list", "--parents", "-n", "1", sha]);
    line.split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn tree(rig: &Rig, sha: &str) -> String {
    git(&rig.bare, &["rev-parse", &format!("{sha}^{{tree}}")])
}

/// The bare repository's `main`, fetched into the work tree, so a new stage starts from it.
fn remote_main(rig: &Rig) -> String {
    git(&rig.work, &["fetch", "-q", "origin", "main"]);
    git(&rig.work, &["rev-parse", "FETCH_HEAD"])
}

#[test]
fn fake_ci_rules_fail_on_file_content() {
    let rig = Rig::new();
    rig.ctl.set_ci(vec![
        CiRule::new("test", Conclusion::Failure)
            .when("src/lib.rs", "BUG")
            .log("thread 'adds' panicked\nassertion failed: 1 + 1 == 3")
            .failing(&["tests::adds"]),
    ]);
    let red = rig.commit(
        &rig.base,
        &[("src/lib.rs", Some("fn add() {} // BUG\n"))],
        "bug",
    );
    rig.push(1, &red);
    rig.open(1, "main", "Stage 1");
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.checks.len(), 1);
    let check = &view.checks[0];
    assert_eq!(
        (check.name.as_str(), check.status),
        ("test", CheckStatus::Completed)
    );
    assert_eq!(check.conclusion, Some(Conclusion::Failure));
    let red_run = check.ci_run.unwrap();
    let out = rig.dir.join("ci.log");
    let log = rig
        .host
        .failed_logs(&rig.repo(), red_run, 1 << 20, &out)
        .unwrap();
    assert!(!log.truncated);
    let text = std::fs::read_to_string(&out).unwrap();
    // gh's `--log-failed` line shape (M9.2.1 check 4): `<job>\t<step>\t<time>Z <text>`,
    // a byte-order mark before the job's first timestamp.
    let first = text.lines().next().unwrap();
    assert!(
        first.starts_with("test\tUNKNOWN STEP\t\u{feff}"),
        "{first:?}"
    );
    assert!(text.contains("Z assertion failed: 1 + 1 == 3\n"), "{text}");
    assert!(text.contains("Z --- FAIL: tests::adds ("), "{text}");

    let green = rig.commit(&red, &[("src/lib.rs", Some("fn add() {}\n"))], "fix");
    rig.push(1, &green);
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.head_oid, green);
    assert_eq!(view.checks[0].conclusion, Some(Conclusion::Success));
    assert_ne!(
        view.checks[0].ci_run,
        Some(red_run),
        "a new head is a new run"
    );

    // A flaky check: red once, then green after `gh run rerun --failed` of the same run.
    rig.ctl
        .set_ci(vec![CiRule::new("flaky", Conclusion::Cancelled).times(1)]);
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.checks[0].conclusion, Some(Conclusion::Cancelled));
    let flaky = view.checks[0].ci_run.unwrap();
    rig.host.rerun_failed(&rig.repo(), flaky).unwrap();
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.checks[0].conclusion, Some(Conclusion::Success));
    assert_eq!(
        view.checks[0].ci_run,
        Some(flaky),
        "a rerun keeps its run id"
    );

    // A check still running: pending, no logs yet, and a rerun of it is already
    // running (decision 10: success).
    rig.ctl
        .set_ci(vec![CiRule::new("slow", Conclusion::Failure).pending(2)]);
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.checks[0].status, CheckStatus::Pending);
    assert_eq!(view.checks[0].conclusion, None);
    let slow = view.checks[0].ci_run.unwrap();
    assert!(
        rig.host
            .failed_logs(&rig.repo(), slow, 1 << 20, &out)
            .is_err()
    );
    rig.host.rerun_failed(&rig.repo(), slow).unwrap();
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.checks[0].status, CheckStatus::Pending);
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.checks[0].status, CheckStatus::Completed);
    assert_eq!(view.checks[0].conclusion, Some(Conclusion::Failure));
    assert!(rig.ctl.forbidden().is_empty());
}

#[test]
fn fake_merge_commit_and_squash_update_the_bare_base() {
    let rig = Rig::new();
    let one = rig.commit(&rig.base, &[("a.txt", Some("a\n"))], "stage 1");
    rig.push(1, &one);
    rig.open(1, "main", "Stage 1");
    rig.ctl.merge(1, MergeMethodArg::Merge, false);
    let merged = rig.remote("main").unwrap();
    assert_eq!(parents(&rig, &merged), vec![rig.base.clone(), one.clone()]);
    let pr = &rig.ctl.prs()[0];
    assert_eq!(pr.state, PrState::Merged);
    assert_eq!(pr.merge_commit.as_deref(), Some(merged.as_str()));
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.state, PrState::Merged);
    assert_eq!(view.merge_commit.map(|c| c.oid), Some(merged.clone()));
    assert!(view.merged_at.is_some());
    assert_eq!(view.mergeable, Mergeable::Unknown);
    assert_eq!(view.head_oid, one);

    let from = remote_main(&rig);
    let two = rig.commit(&from, &[("b.txt", Some("b\n"))], "stage 2");
    rig.push(2, &two);
    rig.open(2, "main", "Stage 2");
    rig.ctl.merge(2, MergeMethodArg::Squash, false);
    let squashed = rig.remote("main").unwrap();
    assert_eq!(parents(&rig, &squashed), vec![merged.clone()]);
    assert_eq!(tree(&rig, &squashed), tree(&rig, &two));
    assert_ne!(squashed, two);

    let from = remote_main(&rig);
    let three_a = rig.commit(&from, &[("c.txt", Some("c\n"))], "stage 3a");
    let three_b = rig.commit(&three_a, &[("d.txt", Some("d\n"))], "stage 3b");
    rig.push(3, &three_b);
    rig.open(3, "main", "Stage 3");
    rig.ctl.merge(3, MergeMethodArg::Rebase, false);
    let rebased = rig.remote("main").unwrap();
    let below = parents(&rig, &rebased);
    assert_eq!(below.len(), 1);
    assert_eq!(parents(&rig, &below[0]), vec![squashed.clone()]);
    assert_eq!(tree(&rig, &rebased), tree(&rig, &three_b));
    // Every stage branch is still there: no branch was asked to be deleted.
    assert!(rig.remote(&head(1)).is_some() && rig.remote(&head(3)).is_some());
}

#[test]
fn fake_merge_with_branch_deletion_retargets_dependents() {
    let rig = Rig::new();
    let one = rig.commit(&rig.base, &[("a.txt", Some("a\n"))], "stage 1");
    let two = rig.commit(&one, &[("b.txt", Some("b\n"))], "stage 2");
    rig.push(1, &one);
    rig.push(2, &two);
    rig.open(1, "main", "Stage 1");
    rig.open(2, &head(1), "Stage 2");
    rig.ctl.merge(1, MergeMethodArg::Squash, true);
    assert_eq!(rig.remote(&head(1)), None);
    let prs = rig.ctl.prs();
    assert_eq!(prs[0].state, PrState::Merged);
    assert_eq!(
        (prs[1].state, prs[1].base.as_str()),
        (PrState::Open, "main")
    );
    let view = rig.host.view_pr(&rig.repo(), 2).unwrap();
    assert_eq!(view.base_ref, "main");
    assert_eq!(view.state, PrState::Open);
    // anthrex's own retarget converges with GitHub's (Risks): a no-op now.
    rig.host.retarget(&rig.repo(), 2, "main").unwrap();
    assert_eq!(rig.ctl.prs()[1].base, "main");

    // GitHub also retargets when the merged head is deleted later, by anyone.
    let from = rig.remote("main").unwrap();
    git(&rig.work, &["fetch", "-q", "origin", "main"]);
    let three = rig.commit(&from, &[("c.txt", Some("c\n"))], "stage 3");
    let four = rig.commit(&three, &[("d.txt", Some("d\n"))], "stage 4");
    rig.push(3, &three);
    rig.push(4, &four);
    rig.open(3, "main", "Stage 3");
    rig.open(4, &head(3), "Stage 4");
    rig.ctl.merge(3, MergeMethodArg::Merge, false);
    assert_eq!(rig.ctl.prs()[3].base, head(3));
    let delete = |stage| DeleteBranchReq {
        repo: rig.repo(),
        run_id: super::tests::RUN.to_string(),
        stage,
    };
    rig.host.delete_branch(&delete(3)).unwrap();
    assert_eq!(rig.host.view_pr(&rig.repo(), 4).unwrap().base_ref, "main");

    // Deleting the head branch of an open pull request closes it.
    rig.host.delete_branch(&delete(4)).unwrap();
    assert_eq!(
        rig.host.view_pr(&rig.repo(), 4).unwrap().state,
        PrState::Closed
    );
    assert!(rig.ctl.forbidden().is_empty());
}

#[test]
fn fake_mergeable_is_conflicting_when_the_base_conflicts() {
    let rig = Rig::new();
    let one = rig.commit(
        &rig.base,
        &[("README.md", Some("# widgets\n\nline two, by the stage\n"))],
        "stage 1",
    );
    rig.push(1, &one);
    rig.open(1, "main", "Stage 1");
    let mergeable = || rig.host.view_pr(&rig.repo(), 1).unwrap().mergeable;
    assert_eq!(mergeable(), Mergeable::Mergeable);
    // A base change elsewhere keeps it mergeable.
    let elsewhere = rig.ctl.commit("main", "docs/other.md", "other\n", "docs");
    assert_eq!(rig.remote("main").as_deref(), Some(elsewhere.as_str()));
    assert_eq!(mergeable(), Mergeable::Mergeable);
    // The same line changed on the base conflicts.
    rig.ctl.commit(
        "main",
        "README.md",
        "# widgets\n\nline two, by a user\n",
        "user",
    );
    assert_eq!(mergeable(), Mergeable::Conflicting);
    // A rewrite of the stage branch, as a force-push leaves it, is not a descendant.
    let rewritten = rig.ctl.force_rewrite(&head(1));
    assert_eq!(rig.remote(&head(1)).as_deref(), Some(rewritten.as_str()));
    assert_ne!(rewritten, one);
    let ancestor = git(&rig.bare, &["merge-base", &rewritten, &one]);
    assert_ne!(ancestor, one);
}

#[test]
fn fake_close_reopen_resolve_and_the_edit_refusals() {
    let rig = Rig::new();
    let one = rig.commit(&rig.base, &[("a.txt", Some("a\n"))], "stage 1");
    rig.push(1, &one);
    rig.open(1, "main", "Stage 1");
    git(
        &rig.work,
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/dev", rig.base),
        ],
    );
    let view = || rig.host.view_pr(&rig.repo(), 1).unwrap();

    let first = rig.ctl.review_comment(1, "alice", "a.txt", 1, "Why a?");
    assert!(!view().threads[0].resolved);
    rig.ctl.resolve(1, first);
    assert!(view().threads[0].resolved);

    rig.ctl.close(1);
    assert_eq!(view().state, PrState::Closed);
    // Constructed texts (not observed), classified as `GhHost` reads them.
    assert_eq!(
        rig.host.retarget(&rig.repo(), 1, "dev"),
        Err(HostError::Failed(
            "GraphQL: Cannot change the base branch of a closed pull request. (updatePullRequest)"
                .to_string()
        ))
    );
    // Setting the base it already has is harmless, closed or not.
    rig.host.retarget(&rig.repo(), 1, "main").unwrap();

    rig.ctl.reopen(1);
    assert_eq!(view().state, PrState::Open);
    assert_eq!(
        rig.host.retarget(&rig.repo(), 1, "nope"),
        Err(HostError::NotFound(
            "GraphQL: Could not resolve to a Ref with the name 'refs/heads/nope'. (updatePullRequest)"
                .to_string()
        ))
    );
    rig.host.retarget(&rig.repo(), 1, "dev").unwrap();
    assert_eq!(view().base_ref, "dev");

    // A PR whose head branch is gone was closed by GitHub and cannot be reopened.
    rig.host
        .delete_branch(&DeleteBranchReq {
            repo: rig.repo(),
            run_id: super::tests::RUN.to_string(),
            stage: 1,
        })
        .unwrap();
    assert_eq!(view().state, PrState::Closed);
    let reopened = catch_unwind(AssertUnwindSafe(|| rig.ctl.reopen(1)));
    assert!(reopened.is_err());
    assert_eq!(rig.ctl.prs()[0].state, PrState::Closed);
    assert!(rig.ctl.forbidden().is_empty());
}
