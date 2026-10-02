//! Task M9.2.4: decision 7's allow-list refuses every shape anthrex must never run, before
//! any process starts. Pure: `allow::check` is called with each refused argv directly,
//! and once through `GhHost` to show the runner never sees a refused command.

use super::allow::{AllowCtx, check};
use super::scripted::ScriptedRunner;
use super::tests::{NOHOOK, SHA, WRITE, argv, with};
use super::*;

const RUN: &str = "r1a2b";

fn ctx() -> AllowCtx<'static> {
    AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: Some("main"),
        repo: Some("o/r"),
    }
}

fn refused(program: Program, args: &[String]) {
    match check(program, args, &ctx()) {
        Err(HostError::Forbidden(text)) => {
            assert!(text.starts_with("anthrex never runs: "), "{text}");
        }
        other => panic!("{program:?} {args:?} was not refused: {other:?}"),
    }
}

fn accepted(program: Program, args: &[String]) {
    assert_eq!(check(program, args, &ctx()), Ok(()), "{args:?}");
}

#[test]
fn allow_list_refuses_merge_approve_and_auto_merge() {
    for args in [
        argv(&["pr", "merge", "1"]),
        argv(&["pr", "merge", "1", "--auto", "--merge"]),
        argv(&["pr", "merge", "1", "--repo", "o/r", "--squash"]),
        argv(&["pr", "merge", "1", "--admin"]),
        argv(&["pr", "review", "1", "--approve"]),
        argv(&["pr", "review", "1", "--repo", "o/r", "--approve"]),
        argv(&["pr", "review", "1", "--comment", "-b", "x"]),
        argv(&["api", "-X", "PUT", "repos/o/r/pulls/1/merge"]),
        argv(&["api", "--method", "PUT", "repos/o/r/pulls/1/merge"]),
        argv(&[
            "api",
            "graphql",
            "-f",
            "query=mutation { enablePullRequestAutoMerge(input: {pullRequestId: \"x\"}) { clientMutationId } }",
        ]),
        argv(&["api", "repos/o/r/pulls/1/reviews", "-f", "event=APPROVE"]),
        argv(&["api", "repos/o/r/pulls/1/reviews", "--paginate"]),
        argv(&[
            "api",
            "-X",
            "POST",
            "repos/o/r/pulls/1/reviews",
            "-f",
            "body=event=APPROVE",
        ]),
        argv(&["pr", "close", "1", "--repo", "o/r"]),
        argv(&["pr", "reopen", "1", "--repo", "o/r"]),
        argv(&["pr", "ready", "1", "--repo", "o/r"]),
    ] {
        refused(Program::Gh, &args);
    }

    // Through GhHost: a refused command never reaches the runner.
    let tmp = tempfile::tempdir().unwrap();
    let host = GhHost::new(ScriptedRunner::new(), "/nonexistent/anthrex-test/gh", "git");
    let repo = HostRepo {
        host: "github.com".to_string(),
        owner: "o".to_string(),
        name: "r".to_string(),
        remote: "origin".to_string(),
        root: tmp.path().to_path_buf(),
    };
    // A PR head that is not a stage branch of any run, and a base that reads as a flag.
    let open = OpenPrReq {
        repo: repo.clone(),
        run_id: RUN.to_string(),
        base: "main".to_string(),
        head: "main".to_string(),
        title: "t".to_string(),
        body_file: tmp.path().join("b.md"),
    };
    assert!(matches!(host.open_pr(&open), Err(HostError::Forbidden(_))));
    // Fix round 1 (m2): the run comes from the request, never from the head or the ref
    // it is checked against, so a head or a fetch ref of another run is refused.
    let other_head = OpenPrReq {
        head: "anthrex/r9999/stage-1".to_string(),
        ..open.clone()
    };
    assert!(matches!(
        host.open_pr(&other_head),
        Err(HostError::Forbidden(_))
    ));
    let other_ref = FetchReq {
        repo: repo.clone(),
        run_id: RUN.to_string(),
        branch: "main".to_string(),
        into: "refs/anthrex/r9999/remote/base".to_string(),
        adopt: None,
    };
    assert!(matches!(
        host.fetch(&other_ref),
        Err(HostError::Forbidden(_))
    ));
    assert!(matches!(
        host.retarget(&repo, 1, "--admin"),
        Err(HostError::Forbidden(_))
    ));
    assert!(matches!(
        host.permission(&repo, "x/../../pulls/1/merge"),
        Err(HostError::Forbidden(_))
    ));
    assert!(host.runner().calls().is_empty());
}

#[test]
fn allow_list_refuses_force_and_foreign_destinations() {
    let stage = format!("refs/heads/anthrex/{RUN}/stage-1");
    let spec = format!("{SHA}:{stage}");
    let push = |extra: &[&str], spec: &str| {
        let mut parts = vec![
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
        ];
        parts.extend_from_slice(extra);
        parts.extend(["origin", spec]);
        with(&WRITE, &parts)
    };
    accepted(Program::Git, &push(&[], &spec));
    for flag in [
        "--force",
        "-f",
        "--force-with-lease",
        "--force-with-lease=refs/heads/x",
        "--force-if-includes",
        "--mirror",
        "--all",
        "--tags",
        "--prune",
    ] {
        refused(Program::Git, &push(&[flag], &spec));
    }
    for bad in [
        format!("+{SHA}:{stage}"),
        format!("{SHA}:refs/heads/main"),
        format!("{SHA}:main"),
        format!("{SHA}:refs/heads/anthrex/r9999/stage-1"),
        format!("{SHA}:refs/heads/anthrex/{RUN}/integration"),
        format!("{SHA}:refs/heads/anthrex/{RUN}/stage-01"),
        format!("{SHA}:refs/heads/anthrex/{RUN}/stage-1/x"),
        format!("{SHA}:refs/tags/v1"),
        format!("HEAD:{stage}"),
        format!("main:{stage}"),
        format!("{}:{stage}", &SHA[..12]),
        SHA.to_string(),
        format!(":{stage}"),
        "refs/heads/*:refs/heads/*".to_string(),
    ] {
        refused(Program::Git, &push(&[], &bad));
    }
    // Two refspecs, or the push without anthrex's write flags.
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &spec,
                &spec,
            ],
        ),
    );
    refused(
        Program::Git,
        &argv(&[
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "origin",
            &spec,
        ]),
    );
    refused(
        Program::Git,
        &with(
            &NOHOOK,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &spec,
            ],
        ),
    );
    // Another remote.
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "upstream",
                &spec,
            ],
        ),
    );
    // Deletes: only this run's stage branches.
    accepted(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                "--delete",
                &stage,
            ],
        ),
    );
    for dst in [
        "refs/heads/main",
        "main",
        "refs/heads/anthrex/r9999/stage-1",
        "refs/heads/anthrex/r1a2b/integration",
    ] {
        refused(
            Program::Git,
            &with(
                &WRITE,
                &[
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    "origin",
                    "--delete",
                    dst,
                ],
            ),
        );
    }
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &format!(":{stage}"),
            ],
        ),
    );
    // The dry run: only with --dry-run, only to anthrex/preflight-<8 hex>.
    let dry = format!("{SHA}:refs/heads/anthrex/preflight-0a1b2c3d");
    accepted(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--dry-run",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &dry,
            ],
        ),
    );
    refused(Program::Git, &push(&[], &dry));
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--dry-run",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &format!("{SHA}:refs/heads/main"),
            ],
        ),
    );
    // Fetch: only into this run's private refs.
    let fetch = |spec: &str| {
        with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                "origin",
                spec,
            ],
        )
    };
    accepted(
        Program::Git,
        &fetch(&format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base")),
    );
    for bad in [
        "+refs/heads/main:refs/heads/main".to_string(),
        "+refs/heads/main:refs/remotes/origin/main".to_string(),
        "+refs/heads/main:refs/anthrex/r9999/remote/base".to_string(),
        format!("+refs/heads/*:refs/anthrex/{RUN}/remote/*"),
        format!("refs/heads/main:refs/anthrex/{RUN}/remote/base"),
        format!("+refs/heads/main:refs/heads/anthrex/{RUN}/stage-1"),
    ] {
        refused(Program::Git, &fetch(&bad));
    }
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                "--prune",
                "origin",
                &format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base"),
            ],
        ),
    );
}

#[test]
fn allow_list_refuses_any_other_api_call_or_mutation() {
    let query = format!("query={THREADS_QUERY}");
    let graphql = |q: &str, owner: &str| {
        argv(&[
            "api", "graphql", "-f", q, "-f", owner, "-f", "name=r", "-F", "number=1",
        ])
    };
    accepted(Program::Gh, &graphql(&query, "owner=o"));
    // Task M9.2.10's fix round: the logged-in user, read bare (no path below it).
    accepted(Program::Gh, &argv(&["api", "user"]));
    for args in [
        graphql(&format!("{query} mutation {{ x }}"), "owner=o"),
        graphql("query=query { viewer { login } }", "owner=o"),
        graphql(&query, "owner=other"),
        graphql(&query, "owner=@/etc/passwd"),
        argv(&["api", "user/repos"]),
        argv(&["api", "user", "--paginate"]),
        argv(&["api", "-X", "PATCH", "user", "-f", "name=x"]),
        argv(&["api", "repos/o/r"]),
        argv(&["api", "-X", "DELETE", "repos/o/r/git/refs/heads/main"]),
        argv(&[
            "api",
            "-X",
            "PATCH",
            "repos/o/r/pulls/1",
            "-f",
            "state=closed",
        ]),
        argv(&[
            "api",
            "-X",
            "POST",
            "repos/o/r/issues/1/comments",
            "-f",
            "body=x",
        ]),
        argv(&[
            "api",
            "-X",
            "POST",
            "repos/o/r/pulls/1/comments",
            "-f",
            "body=x",
        ]),
        argv(&[
            "api",
            "-X",
            "PUT",
            "repos/o/r/pulls/1/comments/2/replies",
            "-f",
            "body=x",
        ]),
        argv(&[
            "api",
            "-X",
            "POST",
            "repos/o/r/pulls/1/comments/2/replies",
            "-f",
            "event=x",
        ]),
        argv(&[
            "api",
            "repos/o/r/pulls/1/comments",
            "--paginate",
            "-f",
            "body=x",
        ]),
        argv(&["api", "repos/o/r/pulls/1/comments"]),
        argv(&["api", "repos/other/r/pulls/1/comments", "--paginate"]),
        argv(&["api", "repos/o/r/collaborators/x/permission", "-X", "PUT"]),
        argv(&["api", "repos/o/r/collaborators/a/b/permission"]),
        argv(&["api", "repos/o/r/actions/runs/1/rerun"]),
        argv(&[
            "pr",
            "edit",
            "1",
            "--repo",
            "o/r",
            "--base",
            "main",
            "--add-reviewer",
            "x",
        ]),
        argv(&["pr", "edit", "1", "--repo", "o/r", "--title", "x"]),
        argv(&["pr", "view", "1", "--repo", "other/r", "--json", "state"]),
        argv(&[
            "pr",
            "create",
            "--repo",
            "o/r",
            "--base",
            "main",
            "--head",
            "main",
            "--title",
            "t",
            "--body-file",
            "/b",
        ]),
        argv(&[
            "pr",
            "create",
            "--repo",
            "o/r",
            "--base",
            "main",
            "--head",
            "anthrex/r1a2b/stage-1",
            "--title",
            "t",
            "--body-file",
            "/b",
            "--draft",
        ]),
        argv(&["pr", "comment", "1", "--repo", "o/r", "--body", "x"]),
        argv(&["run", "rerun", "1", "--repo", "o/r"]),
        argv(&["run", "cancel", "1", "--repo", "o/r"]),
        argv(&["workflow", "run", "ci.yml", "--repo", "o/r"]),
        argv(&["repo", "delete", "o/r", "--yes"]),
        argv(&["repo", "view", "o/r"]),
        argv(&["secret", "set", "X", "--repo", "o/r"]),
        argv(&["auth", "token"]),
        argv(&["auth", "status", "--hostname", "-x"]),
        argv(&["extension", "install", "x/y"]),
        argv(&["alias", "set", "m", "pr merge"]),
        Vec::new(),
    ] {
        refused(Program::Gh, &args);
    }
    // git: only the commands of the table.
    for args in [
        with(&NOHOOK, &["config", "--get", "remote.upstream.url"]),
        with(&NOHOOK, &["config", "remote.origin.url", "x"]),
        with(
            &NOHOOK,
            &["ls-remote", "--heads", "origin", "refs/heads/other"],
        ),
        with(
            &NOHOOK,
            &["rev-parse", "--verify", "refs/heads/main^{commit}"],
        ),
        with(&NOHOOK, &["merge-base", "--is-ancestor", "HEAD", SHA]),
        with(&NOHOOK, &["update-ref", "refs/heads/main", SHA]),
        with(&NOHOOK, &["merge", SHA]),
        with(&NOHOOK, &["rebase", "main"]),
        with(&NOHOOK, &["reset", "--hard", SHA]),
        with(&NOHOOK, &["branch", "-D", "main"]),
        with(&NOHOOK, &["-c", "core.sshCommand=x", "fetch", "origin"]),
        argv(&["config", "--get", "remote.origin.url"]),
    ] {
        refused(Program::Git, &args);
    }
    accepted(
        Program::Git,
        &with(&NOHOOK, &["config", "--get", "remote.origin.url"]),
    );
    accepted(
        Program::Git,
        &with(
            &NOHOOK,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/anthrex/{RUN}/remote/stage-2^{{commit}}"),
            ],
        ),
    );
}

#[test]
fn allow_list_refuses_a_remote_named_like_an_option_and_the_old_push_shapes() {
    // Fix round 1 (m1): a remote that reads as an option is refused even when the
    // context names it.
    let option = AllowCtx {
        remote: "--receive-pack=evil",
        ..ctx()
    };
    let push = with(
        &WRITE,
        &[
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "--receive-pack=evil",
            &format!("{SHA}:refs/heads/anthrex/{RUN}/stage-1"),
        ],
    );
    assert!(matches!(
        check(Program::Git, &push, &option),
        Err(HostError::Forbidden(_))
    ));
    let config = with(&NOHOOK, &["config", "--get", "remote.-x.url"]);
    let dash = AllowCtx {
        remote: "-x",
        ..ctx()
    };
    assert!(matches!(
        check(Program::Git, &config, &dash),
        Err(HostError::Forbidden(_))
    ));
    // Ruling I1: a push or fetch without its no-tags and no-submodules flags.
    let spec = format!("{SHA}:refs/heads/anthrex/{RUN}/stage-1");
    for args in [
        with(&WRITE, &["push", "--porcelain", "origin", &spec]),
        with(
            &WRITE,
            &["push", "--porcelain", "--no-follow-tags", "origin", &spec],
        ),
        with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--refmap=",
                "origin",
                &format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base"),
            ],
        ),
    ] {
        refused(Program::Git, &args);
    }
}
