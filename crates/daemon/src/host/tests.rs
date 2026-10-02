//! Task M9.2.4: `GhHost`'s exact argv, and the allow-list accepting every command it
//! builds, through `ScriptedRunner`, which answers from M9.2.1's fixtures
//! (`host/fixtures/`). The fixtures and helpers here are shared with `tests_parse.rs`
//! and `tests_allow.rs`. No test here starts a process.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::scripted::{Call, ScriptedRunner};
use super::*;

pub(super) const SHA: &str = "a560bea91b8cd58b3c0d78e5db98c7fdc8e5036b";
pub(super) const SHA2: &str = "2537a3b6931a787d6b4b0ab686cd4cf7eda0dfda";
pub(super) const RUN: &str = "r1a2b";

pub(super) const WRITE: [&str; 6] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "commit.gpgSign=false",
    "-c",
    "core.logAllRefUpdates=false",
];
pub(super) const NOHOOK: [&str; 2] = ["-c", "core.hooksPath=/dev/null"];

pub(super) const PR_VIEW_OPEN_RED: &str = include_str!("fixtures/pr_view_open_red.json");
pub(super) const PR_VIEW_CONFLICTING: &str =
    include_str!("fixtures/pr_view_conflicting_changes_requested.json");
pub(super) const PR_VIEW_MERGED: &str = include_str!("fixtures/pr_view_merged.json");
pub(super) const STATUS_CONTEXT: &str =
    include_str!("fixtures/status_check_rollup_status_context.json");
pub(super) const PR_LIST_HEAD: &str = include_str!("fixtures/pr_list_head.json");
pub(super) const PR_LIST_HEAD_NONE: &str = include_str!("fixtures/pr_list_head_none.json");
pub(super) const PR_LIST_HEAD_OWNER: &str = include_str!("fixtures/pr_list_head_owner.json");
pub(super) const REST_REVIEW_COMMENTS: &str =
    include_str!("fixtures/rest_pull_review_comments.json");
pub(super) const REST_ISSUE_COMMENTS: &str = include_str!("fixtures/rest_issue_comments.json");
pub(super) const RUN_LOG_FAILED: &str = include_str!("fixtures/run_view_log_failed.txt");
pub(super) const GH_ERRORS: &str = include_str!("fixtures/gh_errors.json");
pub(super) const THREADS_DEPRECATED_ID: &str = include_str!("fixtures/threads_query_response.json");
pub(super) const THREADS_EXTENDED: &str =
    include_str!("fixtures/threads_query_response_extended.json");
pub(super) const PERMISSION: &str = include_str!("fixtures/rest_collaborator_permission.json");

pub(super) fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

pub(super) fn with(flags: &[&str], parts: &[&str]) -> Vec<String> {
    flags.iter().chain(parts).map(|p| p.to_string()).collect()
}

pub(super) fn repo(root: &Path) -> HostRepo {
    HostRepo {
        host: "github.com".to_string(),
        owner: "cli".to_string(),
        name: "cli".to_string(),
        remote: "origin".to_string(),
        root: root.to_path_buf(),
    }
}

pub(super) fn host(runner: ScriptedRunner) -> GhHost<ScriptedRunner> {
    GhHost::new(runner, "/nonexistent/anthrex-test/gh", "git")
}

pub(super) fn gh_env(host: &str) -> Vec<(String, String)> {
    [
        ("GH_HOST", host),
        ("GH_PROMPT_DISABLED", "1"),
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1"),
        ("GH_SPINNER_DISABLED", "1"),
        ("GH_PAGER", "cat"),
        ("NO_COLOR", "1"),
        ("CLICOLOR", "0"),
        ("GIT_OPTIONAL_LOCKS", "0"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

pub(super) const VERSION: &str =
    "gh version 2.92.0 (2026-04-28)\nhttps://github.com/cli/cli/releases/tag/v2.92.0\n";

pub(super) fn preflight_req(root: &Path) -> PreflightReq {
    PreflightReq {
        root: root.to_path_buf(),
        remote: "origin".to_string(),
        base_branch: "trunk".to_string(),
        base_sha: SHA.to_string(),
        nonce: "0a1b2c3d".to_string(),
    }
}

/// Preflight scripted up to (not including) `gh auth status`.
pub(super) fn preflight_until_auth(url: &str) -> ScriptedRunner {
    ScriptedRunner::new().ok(&format!("{url}\n")).ok(VERSION)
}

/// Every method once, each answered from a fixture, in the order of the brief's
/// "GhHost commands" table. Returns the runner (its calls) and the out directory.
fn every_method(root: &Path) -> GhHost<ScriptedRunner> {
    let porcelain = |flag: &str, src: &str, dst: &str, summary: &str| {
        format!("To https://github.com/cli/cli.git\n{flag}\t{src}:{dst}\t{summary}\nDone\n")
    };
    let stage1 = format!("refs/heads/anthrex/{RUN}/stage-1");
    let created = {
        let comments: Value = serde_json::from_str(REST_REVIEW_COMMENTS).unwrap();
        comments[0].to_string()
    };
    let runner = ScriptedRunner::new()
        // preflight
        .ok("https://github.com/cli/cli.git\n")
        .ok(VERSION)
        .ok("")
        .ok("{\"nameWithOwner\":\"cli/cli\"}\n")
        .ok(&porcelain(
            "*",
            SHA,
            "refs/heads/anthrex/preflight-0a1b2c3d",
            "[new branch]",
        ))
        .ok(&format!("{SHA}\trefs/heads/trunk\n"))
        // push
        .ok(&porcelain(" ", SHA, &stage1, "1111111..a560bea"))
        // fetch, then fetch with an adoption whose remote does not descend
        .ok("")
        .ok(&format!("{SHA}\n"))
        .ok("")
        .ok(&format!("{SHA2}\n"))
        .fails("", "")
        // open_pr: none for the head, so one is created
        .ok(PR_LIST_HEAD_NONE)
        .ok("https://github.com/cli/cli/pull/14000\n")
        // view_pr
        .ok(PR_VIEW_OPEN_RED)
        .ok(THREADS_EXTENDED)
        // failed_logs, rerun_failed
        .ok(RUN_LOG_FAILED)
        .ok("")
        // reply in a thread, then in the conversation (no marker in either listing)
        .ok("[]")
        .ok(&created)
        .ok(REST_ISSUE_COMMENTS)
        .ok("https://github.com/cli/cli/pull/13982#issuecomment-5200000001\n")
        // retarget, permission, delete_branch
        .ok("")
        .ok(PERMISSION)
        .ok(&porcelain(
            "-",
            "",
            &format!("refs/heads/anthrex/{RUN}/stage-2"),
            "[deleted]",
        ));
    let host = host(runner);
    let repo = repo(root);

    assert_eq!(host.preflight(&preflight_req(root)).unwrap(), repo);
    let push = PushReq {
        repo: repo.clone(),
        run_id: RUN.to_string(),
        stage: 1,
        sha: SHA.to_string(),
    };
    assert_eq!(host.push(&push).unwrap(), PushOutcome::Pushed);
    let into = format!("refs/anthrex/{RUN}/remote/stage-1");
    let fetch = FetchReq {
        repo: repo.clone(),
        run_id: RUN.to_string(),
        branch: format!("anthrex/{RUN}/stage-1"),
        into: into.clone(),
        adopt: None,
    };
    assert_eq!(
        host.fetch(&fetch).unwrap(),
        FetchOutcome::Fetched {
            sha: SHA.to_string()
        }
    );
    let adopting = FetchReq {
        adopt: Some(Adopt {
            local_ref: format!("anthrex/{RUN}/stage-1"),
            expected_local: SHA.to_string(),
            also_integration: false,
        }),
        ..fetch
    };
    assert_eq!(
        host.fetch(&adopting).unwrap(),
        FetchOutcome::NotDescendant {
            remote: SHA2.to_string()
        }
    );
    let open = OpenPrReq {
        repo: repo.clone(),
        run_id: RUN.to_string(),
        base: "trunk".to_string(),
        head: format!("anthrex/{RUN}/stage-1"),
        title: "[anthrex r1a2b 1/2] Add the parser (+1 more)".to_string(),
        body_file: root.join("pr-1.md"),
    };
    assert_eq!(
        host.open_pr(&open).unwrap(),
        PrRef {
            number: 14000,
            url: "https://github.com/cli/cli/pull/14000".to_string(),
            state: PrState::Open,
            existed: false,
        }
    );
    assert_eq!(host.view_pr(&repo, 13788).unwrap().number, 13788);
    let log = root.join("ci-28656994029.log");
    let file = host
        .failed_logs(&repo, 28656994029, 2 * 1024 * 1024, &log)
        .unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), RUN_LOG_FAILED);
    assert!(!file.truncated);
    host.rerun_failed(&repo, 28656994029).unwrap();
    let marker = |key: &str| format!("<!-- anthrex:reply {RUN} 13982:{key} 1a2b3c4 -->");
    let thread = ReplyReq {
        repo: repo.clone(),
        number: 13982,
        target: ReplyTarget::Thread {
            comment_id: 3658443343,
        },
        body: "Addressed in 1a2b3c4 by task fix4.".to_string(),
        marker: marker("t3658443343"),
    };
    assert_eq!(host.reply(&thread).unwrap(), 3658443294);
    let conversation = ReplyReq {
        target: ReplyTarget::Conversation,
        body: "@alice Addressed in 1a2b3c4 by task fix5.".to_string(),
        marker: marker("c5154974588"),
        ..thread
    };
    assert_eq!(host.reply(&conversation).unwrap(), 5200000001);
    host.retarget(&repo, 14001, "trunk").unwrap();
    assert_eq!(
        host.permission(&repo, "tester").unwrap(),
        RepoPermission::Write
    );
    host.delete_branch(&DeleteBranchReq {
        repo,
        run_id: RUN.to_string(),
        stage: 2,
    })
    .unwrap();
    assert_eq!(host.runner().unanswered(), 0);
    host
}

#[test]
fn every_method_builds_its_exact_argv() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let host = every_method(root);
    let calls = host.runner().calls();
    let stage1 = format!("refs/heads/anthrex/{RUN}/stage-1");
    let into = format!("refs/anthrex/{RUN}/remote/stage-1");
    let head = format!("anthrex/{RUN}/stage-1");
    let body_file = root.join("pr-1.md").to_string_lossy().into_owned();
    let graphql = [
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        // The query byte for byte (rulings R-2 and m4: the newest 100 threads).
        "query=query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { reviewThreads(last: 100) { nodes { isResolved path line comments(first: 50) { nodes { fullDatabaseId body diffHunk author { __typename login } } } } } reviews(last: 100) { nodes { fullDatabaseId state body author { __typename login } } } comments(last: 100) { nodes { fullDatabaseId body author { __typename login } } } } } }".to_string(),
        "-f".to_string(),
        "owner=cli".to_string(),
        "-f".to_string(),
        "name=cli".to_string(),
        "-F".to_string(),
        "number=13788".to_string(),
    ];
    let reply_body = format!(
        "body=Addressed in 1a2b3c4 by task fix4.\n\n<!-- anthrex:reply {RUN} 13982:t3658443343 1a2b3c4 -->"
    );
    let (r, w, p, l) = (
        HOST_READ_TIMEOUT,
        HOST_WRITE_TIMEOUT,
        PUSH_TIMEOUT,
        LOG_TIMEOUT,
    );
    let expected: Vec<(Program, Vec<String>, Duration)> = vec![
        (
            Program::Git,
            with(&NOHOOK, &["config", "--get", "remote.origin.url"]),
            r,
        ),
        (Program::Gh, argv(&["--version"]), r),
        (
            Program::Gh,
            argv(&["auth", "status", "--hostname", "github.com"]),
            r,
        ),
        (
            Program::Gh,
            argv(&["repo", "view", "cli/cli", "--json", "nameWithOwner"]),
            r,
        ),
        (
            Program::Git,
            with(
                &WRITE,
                &[
                    "push",
                    "--dry-run",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    "origin",
                    &format!("{SHA}:refs/heads/anthrex/preflight-0a1b2c3d"),
                ],
            ),
            p,
        ),
        (
            Program::Git,
            with(
                &NOHOOK,
                &["ls-remote", "--heads", "origin", "refs/heads/trunk"],
            ),
            r,
        ),
        (
            Program::Git,
            with(
                &WRITE,
                &[
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    "origin",
                    &format!("{SHA}:{stage1}"),
                ],
            ),
            p,
        ),
        (
            Program::Git,
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
                    &format!("+refs/heads/{head}:{into}"),
                ],
            ),
            p,
        ),
        (
            Program::Git,
            with(
                &NOHOOK,
                &["rev-parse", "--verify", &format!("{into}^{{commit}}")],
            ),
            r,
        ),
        (
            Program::Git,
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
                    &format!("+refs/heads/{head}:{into}"),
                ],
            ),
            p,
        ),
        (
            Program::Git,
            with(
                &NOHOOK,
                &["rev-parse", "--verify", &format!("{into}^{{commit}}")],
            ),
            r,
        ),
        (
            Program::Git,
            with(&NOHOOK, &["merge-base", "--is-ancestor", SHA, SHA2]),
            r,
        ),
        (
            Program::Gh,
            argv(&[
                "pr",
                "list",
                "--repo",
                "cli/cli",
                "--head",
                &head,
                "--state",
                "all",
                "--json",
                "number,url,state,baseRefName,headRepositoryOwner,isCrossRepository",
            ]),
            r,
        ),
        (
            Program::Gh,
            argv(&[
                "pr",
                "create",
                "--repo",
                "cli/cli",
                "--base",
                "trunk",
                "--head",
                &head,
                "--title",
                "[anthrex r1a2b 1/2] Add the parser (+1 more)",
                "--body-file",
                &body_file,
            ]),
            w,
        ),
        (
            Program::Gh,
            argv(&[
                "pr",
                "view",
                "13788",
                "--repo",
                "cli/cli",
                "--json",
                "state,mergedAt,mergeCommit,baseRefName,headRefOid,mergeable,reviewDecision,statusCheckRollup",
            ]),
            r,
        ),
        (Program::Gh, graphql.to_vec(), r),
        (
            Program::Gh,
            argv(&[
                "run",
                "view",
                "28656994029",
                "--repo",
                "cli/cli",
                "--log-failed",
            ]),
            l,
        ),
        (
            Program::Gh,
            argv(&[
                "run",
                "rerun",
                "28656994029",
                "--repo",
                "cli/cli",
                "--failed",
            ]),
            w,
        ),
        (
            Program::Gh,
            argv(&["api", "repos/cli/cli/pulls/13982/comments", "--paginate"]),
            r,
        ),
        (
            Program::Gh,
            argv(&[
                "api",
                "-X",
                "POST",
                "repos/cli/cli/pulls/13982/comments/3658443343/replies",
                "-f",
                &reply_body,
            ]),
            w,
        ),
        (
            Program::Gh,
            argv(&["api", "repos/cli/cli/issues/13982/comments", "--paginate"]),
            r,
        ),
        // The conversation reply's body file is a temporary path, checked below.
        (Program::Gh, Vec::new(), w),
        (
            Program::Gh,
            argv(&[
                "pr", "edit", "14001", "--repo", "cli/cli", "--base", "trunk",
            ]),
            w,
        ),
        (
            Program::Gh,
            argv(&["api", "repos/cli/cli/collaborators/tester/permission"]),
            r,
        ),
        (
            Program::Git,
            with(
                &WRITE,
                &[
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    "origin",
                    "--delete",
                    &format!("refs/heads/anthrex/{RUN}/stage-2"),
                ],
            ),
            p,
        ),
    ];
    assert_eq!(calls.len(), expected.len());
    for (i, (call, (program, want, timeout))) in calls.iter().zip(&expected).enumerate() {
        assert_eq!(call.program, *program, "call {i}: {:?}", call.argv);
        assert_eq!(call.timeout, *timeout, "call {i}: {:?}", call.argv);
        assert_eq!(call.dir, root, "call {i}");
        if !want.is_empty() {
            assert_eq!(&call.argv, want, "call {i}");
        }
        match call.program {
            Program::Gh => assert_eq!(call.env, gh_env("github.com"), "call {i}"),
            Program::Git => assert!(call.env.is_empty(), "call {i}: {:?}", call.env),
        }
    }
    let comment = &calls[21].argv;
    assert_eq!(
        comment[..6],
        argv(&["pr", "comment", "13982", "--repo", "cli/cli", "--body-file"])[..]
    );
    assert_eq!(comment.len(), 7);
    let body = Path::new(&comment[6]);
    assert!(body.starts_with(std::env::temp_dir()), "{body:?}");
    assert!(!body.exists(), "the reply's body file is removed: {body:?}");
    assert_eq!(
        calls[16].cap,
        Capture::HeadTail {
            head: 64 * 1024,
            tail: 2 * 1024 * 1024 - 64 * 1024 - 64,
        }
    );
}

#[test]
fn allow_list_accepts_every_built_command() {
    let tmp = tempfile::tempdir().unwrap();
    let host = every_method(tmp.path());
    let calls: Vec<Call> = host.runner().calls();
    assert_eq!(calls.len(), 25);
    let ctx = allow::AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: Some("trunk"),
        repo: Some("cli/cli"),
    };
    for call in &calls {
        assert_eq!(
            allow::check(call.program, &call.argv, &ctx),
            Ok(()),
            "{:?}",
            call.argv
        );
    }
}
