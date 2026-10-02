//! The final fix wave (W2): `FakeGh`'s last guard covers `git` too. Asked to push in a
//! way that could move a base branch (a force, a mirror, a destination or a delete
//! outside the run's stage branches), it records the argv in `forbidden.jsonl` and
//! panics before any git runs (A3). For `gh`, it also catches `merge-upstream`,
//! `gh repo sync`, and the spellings deferred from task 5: a trailing-slash `graphql/`,
//! a mutation in a positional's query string, percent-encoded segments, a JSON-escaped
//! `mutation` in an `--input` file, and any write to `git/refs`. The allow-list, the
//! first guard, refuses every one of them.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Duration;

use super::tests::{FULL, RUN, strings};
use super::*;
use crate::host::allow::{self, AllowCtx};
use crate::host::{Capture, HostError, Program, RunOutput, Runner};
use crate::run::git::WRITE_FLAGS;

const SHA: &str = "a560bea91b8cd58b3c0d78e5db98c7fdc8e5036b";

fn run_in(gh: &FakeGh, program: Program, dir: &Path, args: &[String]) -> RunOutput {
    // `git` runs with `run_git`'s environment only.
    let env = match program {
        Program::Gh => vec![("GH_HOST".to_string(), "github.com".to_string())],
        Program::Git => Vec::new(),
    };
    gh.run(
        program,
        dir,
        args,
        &env,
        Duration::from_secs(30),
        Capture::Bytes(1 << 20),
    )
    .unwrap()
}

fn panic_text(gh: &FakeGh, program: Program, dir: &Path, args: &[String]) -> String {
    let caught = catch_unwind(AssertUnwindSafe(|| run_in(gh, program, dir, args)));
    let payload = match caught {
        Ok(out) => panic!("FakeGh answered {args:?} instead of panicking: {out:?}"),
        Err(payload) => payload,
    };
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(other) => other
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}

fn ctx() -> AllowCtx<'static> {
    AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: Some("main"),
        repo: Some(FULL),
    }
}

fn assert_refused(program: Program, args: &[String]) {
    match allow::check(program, args, &ctx()) {
        Err(HostError::Forbidden(_)) => {}
        other => panic!("the allow-list did not refuse {program:?} {args:?}: {other:?}"),
    }
}

/// `git <WRITE_FLAGS> push <args>`, as `GhHost` spells its pushes.
fn push(args: &[&str]) -> Vec<String> {
    let mut all: Vec<&str> = WRITE_FLAGS.to_vec();
    all.push("push");
    all.extend_from_slice(args);
    strings(&all)
}

/// The flags every push of `GhHost`'s carries (ruling I1).
const FLAGS: [&str; 3] = ["--porcelain", "--no-follow-tags", "--recurse-submodules=no"];

fn flagged(rest: &[&str]) -> Vec<String> {
    let mut all = FLAGS.to_vec();
    all.extend_from_slice(rest);
    push(&all)
}

#[test]
fn fake_host_panics_on_a_git_push_that_could_land() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fake");
    let gh = FakeGh::new(&dir);
    let stage = format!("refs/heads/anthrex/{RUN}/stage-1");
    let to_stage = format!("{SHA}:{stage}");
    let forced = format!("+{SHA}:{stage}");
    let to_main = format!("{SHA}:refs/heads/main");
    let short_main = format!("{SHA}:main");
    let to_tag = format!("{SHA}:refs/tags/v1");
    let cases: Vec<(Vec<String>, &str)> = vec![
        (flagged(&["origin", &to_main]), "push to refs/heads/main"),
        (flagged(&["origin", &short_main]), "push to refs/heads/main"),
        (flagged(&["origin", &to_tag]), "push to refs/tags/v1"),
        (flagged(&["origin", &forced]), "force-push"),
        (flagged(&["--force", "origin", &to_stage]), "force-push"),
        (flagged(&["-f", "origin", &to_stage]), "force-push"),
        (flagged(&["-uf", "origin", &to_stage]), "force-push"),
        (
            flagged(&["--force-with-lease", "origin", &to_stage]),
            "force-push",
        ),
        (flagged(&["--mirror", "origin"]), "push every ref"),
        (flagged(&["--all", "origin"]), "push every ref"),
        (flagged(&["--tags", "origin", &to_stage]), "push every ref"),
        (
            flagged(&["--prune", "origin", &to_stage]),
            "prune remote branches",
        ),
        (
            flagged(&["origin", "--delete", "refs/heads/main"]),
            "delete refs/heads/main",
        ),
        (flagged(&["-d", "origin", "main"]), "delete refs/heads/main"),
        (
            flagged(&["origin", ":refs/heads/main"]),
            "delete refs/heads/main",
        ),
        (
            flagged(&[
                "origin",
                "--delete",
                "refs/heads/anthrex/preflight-0a1b2c3d",
            ]),
            "delete refs/heads/anthrex/preflight-0a1b2c3d",
        ),
        (flagged(&["origin"]), "push without a refspec"),
        (push(&["origin", "HEAD"]), "push HEAD without a destination"),
    ];
    for (args, verb) in &cases {
        assert_eq!(
            panic_text(&gh, Program::Git, tmp.path(), args),
            format!(
                "FakeHost: anthrex asked to {verb} git {}; anthrex never lands anything",
                args.join(" ")
            ),
            "{args:?}"
        );
        assert_refused(Program::Git, args);
    }
    // Recorded as `git …`, so a `gh` argv and a `git` one never read alike; `calls.jsonl`
    // stays `gh`'s own.
    let ctl = FakeGithubCtl::open(&dir);
    let expected: Vec<Vec<String>> = cases
        .iter()
        .map(|(args, _)| {
            let mut line = vec!["git".to_string()];
            line.extend(args.iter().cloned());
            line
        })
        .collect();
    assert_eq!(ctl.forbidden(), expected);
    assert!(ctl.calls().is_empty());

    // Not landings: `GhHost`'s own push, dry run, delete and fetch reach git (which
    // fails here, in a directory that is no repository) and record nothing.
    let not = [
        flagged(&["origin", &to_stage]),
        push(&[
            "--dry-run",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "origin",
            &format!("{SHA}:refs/heads/anthrex/preflight-0a1b2c3d"),
        ]),
        flagged(&["origin", "--delete", &stage]),
        strings(&[
            "-c",
            "core.hooksPath=/dev/null",
            "fetch",
            "--no-tags",
            "origin",
            &format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base"),
        ]),
    ];
    let outside = tempfile::tempdir().unwrap();
    for args in &not {
        assert!(
            !run_in(&gh, Program::Git, outside.path(), args).success,
            "{args:?}"
        );
    }
    assert_eq!(ctl.forbidden().len(), cases.len());
}

const MERGE_MUTATION: &str =
    "mutation { mergePullRequest(input: {pullRequestId: \"PR_1\"}) { clientMutationId } }";

#[test]
fn fake_host_panics_on_base_moving_and_disguised_gh_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fake");
    let gh = FakeGh::new(&dir);
    // A JSON body whose `mutation` is escaped, read relative to where `gh` runs.
    std::fs::write(
        tmp.path().join("escaped.json"),
        "{\"query\": \"\\u006dutation { mergePullRequest(input: {pullRequestId: \\\"PR_1\\\"}) { clientMutationId } }\"}",
    )
    .unwrap();
    let encoded_query = format!(
        "graphql?query={}",
        MERGE_MUTATION
            .bytes()
            .map(|b| format!("%{b:02X}"))
            .collect::<String>()
    );
    let raw = format!("query={MERGE_MUTATION}");
    let cases: Vec<(Vec<String>, &str)> = vec![
        (
            strings(&[
                "api",
                "-X",
                "POST",
                "repos/anthrex-test/widgets/merge-upstream",
                "-f",
                "branch=main",
            ]),
            "merge",
        ),
        (
            strings(&[
                "api",
                "repos/anthrex-test/widgets/merge-upstream",
                "-f",
                "branch=main",
            ]),
            "merge",
        ),
        (strings(&["repo", "sync", FULL, "--branch", "main"]), "sync"),
        (strings(&["repo", "sync", "--force"]), "sync"),
        (strings(&["api", "graphql/", "-f", &raw]), "merge"),
        (
            strings(&[
                "api",
                "https://ghe.example.invalid/api/graphql/",
                "-f",
                &raw,
            ]),
            "merge",
        ),
        (strings(&["api", "-X", "POST", &encoded_query]), "merge"),
        (
            strings(&[
                "api",
                "graphql?query=mutation%20%7B%20x%20%7D",
                "-X",
                "POST",
            ]),
            "merge",
        ),
        (
            strings(&[
                "api",
                "repos/anthrex-test/widgets/pulls/1/%6Derge",
                "-f",
                "merge_method=squash",
            ]),
            "merge",
        ),
        (
            strings(&[
                "api",
                "repos/anthrex-test/widgets/pulls/1/%72eviews",
                "-f",
                "event=APPROVE",
            ]),
            "approve",
        ),
        (strings(&["api", "%67raphql", "-f", &raw]), "merge"),
        (
            strings(&["api", "--input", "escaped.json", "graphql"]),
            "merge",
        ),
        (
            strings(&[
                "api",
                "-X",
                "PATCH",
                "repos/anthrex-test/widgets/git/refs/heads/main",
                "-f",
                &format!("sha={SHA}"),
            ]),
            "move a branch",
        ),
        (
            strings(&[
                "api",
                "-X",
                "PUT",
                "repos/anthrex-test/widgets/git/refs/heads/main",
            ]),
            "move a branch",
        ),
        (
            strings(&[
                "api",
                "--method=DELETE",
                "repos/anthrex-test/widgets/git/refs/heads/main",
            ]),
            "move a branch",
        ),
        (
            strings(&[
                "api",
                "repos/anthrex-test/widgets/git/refs",
                "-f",
                "ref=refs/heads/main",
                "-f",
                &format!("sha={SHA}"),
            ]),
            "move a branch",
        ),
    ];
    for (args, verb) in &cases {
        assert_eq!(
            panic_text(&gh, Program::Gh, tmp.path(), args),
            format!(
                "FakeHost: anthrex asked to {verb} gh {}; anthrex never lands anything",
                args.join(" ")
            ),
            "{args:?}"
        );
        assert_refused(Program::Gh, args);
    }
    let ctl = FakeGithubCtl::open(&dir);
    let expected: Vec<Vec<String>> = cases.iter().map(|(a, _)| a.clone()).collect();
    assert_eq!(ctl.forbidden(), expected);
    assert_eq!(ctl.calls(), expected);

    // Not landings: a reviewer whose login starts with `merge`, a read of a ref, and the
    // viewer's login.
    let not = [
        strings(&[
            "api",
            "repos/anthrex-test/widgets/collaborators/mergebot/permission",
        ]),
        strings(&[
            "api",
            "repos/anthrex-test/widgets/collaborators/reviews/permission",
        ]),
        strings(&["api", "repos/anthrex-test/widgets/git/refs/heads/main"]),
        strings(&["api", "user"]),
    ];
    for args in &not {
        let caught = catch_unwind(AssertUnwindSafe(|| {
            run_in(&gh, Program::Gh, tmp.path(), args)
        }));
        assert!(caught.is_ok(), "{args:?} panicked");
    }
    assert_eq!(ctl.forbidden().len(), cases.len());
}

/// `git <global> <WRITE_FLAGS> push <FLAGS> <rest>`: a push behind global options.
fn wrapped(global: &[&str], rest: &[&str]) -> Vec<String> {
    let mut all = strings(global);
    all.extend(flagged(rest));
    all
}

#[test]
fn fake_host_sees_a_push_behind_git_global_options() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fake");
    let gh = FakeGh::new(&dir);
    let stage = format!("refs/heads/anthrex/{RUN}/stage-1");
    let to_main = format!("{SHA}:refs/heads/main");
    let forced = format!("+{SHA}:{stage}");
    let globals: [&[&str]; 13] = [
        &["--git-dir", "/nonexistent/.git"],
        &["--git-dir=/nonexistent/.git"],
        &["--work-tree", "/nonexistent"],
        &["--work-tree=/nonexistent"],
        &["--namespace", "ns"],
        &["--namespace=ns"],
        &["--attr-source", "HEAD"],
        &["-c", "user.name=x"],
        &["-c", "alias.p=log"],
        &["-C", "/nonexistent"],
        &["--bare"],
        &["--git-dir", "push", "--work-tree", "push"],
        &["-c", "alias.push=status", "-C", "push"],
    ];
    let mut cases: Vec<(Vec<String>, &str)> = Vec::new();
    for global in globals {
        cases.push((
            wrapped(global, &["origin", &to_main]),
            "push to refs/heads/main",
        ));
        cases.push((wrapped(global, &["origin", &forced]), "force-push"));
    }
    // A push spelled through an alias that `-c` defines, plain or as a shell alias.
    let through_alias = |alias: &str, verb: &str, rest: &[&str]| {
        let mut all = vec!["-c", alias];
        all.extend_from_slice(&WRITE_FLAGS);
        all.push(verb);
        all.extend_from_slice(&FLAGS);
        all.extend_from_slice(rest);
        strings(&all)
    };
    cases.push((
        through_alias("alias.p=push", "p", &["origin", &to_main]),
        "push to refs/heads/main",
    ));
    cases.push((
        through_alias("alias.P=push --force", "p", &["origin", &stage]),
        "force-push",
    ));
    cases.push((
        through_alias("alias.s=!git push", "s", &["origin", &to_main]),
        "run a shell alias",
    ));
    for (args, verb) in &cases {
        assert_eq!(
            panic_text(&gh, Program::Git, tmp.path(), args),
            format!(
                "FakeHost: anthrex asked to {verb} git {}; anthrex never lands anything",
                args.join(" ")
            ),
            "{args:?}"
        );
        assert_refused(Program::Git, args);
    }
    let ctl = FakeGithubCtl::open(&dir);
    assert_eq!(ctl.forbidden().len(), cases.len());

    // Not landings: a stage push behind the same options reaches git (which fails here,
    // outside any repository) and records nothing.
    let to_stage = format!("{SHA}:{stage}");
    let outside = tempfile::tempdir().unwrap();
    let mut not: Vec<Vec<String>> = globals
        .iter()
        .map(|global| wrapped(global, &["origin", &to_stage]))
        .collect();
    not.push(through_alias("alias.p=push", "p", &["origin", &to_stage]));
    for args in &not {
        assert!(
            !run_in(&gh, Program::Git, outside.path(), args).success,
            "{args:?}"
        );
    }
    assert_eq!(ctl.forbidden().len(), cases.len());
}
