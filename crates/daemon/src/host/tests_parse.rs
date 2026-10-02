//! Task M9.2.4: reading `gh`'s output (the recorded fixtures), the error classes,
//! remote URLs, and `open_pr`'s, `reply`'s and `failed_logs`'s behaviour, through
//! `ScriptedRunner`. No test here starts `gh`; `failed_logs_…` runs a stand-in script it
//! writes itself. Split from `tests.rs` for AGENTS.md rule 8.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::gh_parse::{self, classify};
use super::remote::{self, GithubRemote};
use super::scripted::ScriptedRunner;
use super::tests::*;
use super::*;

#[test]
fn remote_urls_parse_https_ssh_and_scp_forms() {
    let gh = |host: &str| GithubRemote {
        host: host.to_string(),
        owner: "cli".to_string(),
        name: "cli".to_string(),
    };
    for url in [
        "https://github.com/cli/cli.git",
        "https://github.com/cli/cli",
        "https://github.com/cli/cli/",
        "https://x-access-token:secret@github.com/cli/cli.git",
        "https://github.com:443/cli/cli.git",
        "git@github.com:cli/cli.git",
        "git@github.com:cli/cli",
        "ssh://git@github.com/cli/cli.git",
        "ssh://git@github.com:22/cli/cli",
        "ssh://git@ssh.github.com:443/cli/cli.git",
        "  https://GitHub.com/cli/cli.git\n",
    ] {
        assert_eq!(remote::parse(url), Some(gh("github.com")), "{url}");
    }
    assert_eq!(
        remote::parse("https://ghe.example.com/cli/cli.git"),
        Some(gh("ghe.example.com"))
    );
    assert_eq!(
        remote::parse("git@ghe.example.com:cli/cli.git"),
        Some(gh("ghe.example.com"))
    );
    for url in [
        "",
        "/srv/git/cli.git",
        "../remote.git",
        "file:///srv/git/cli.git",
        "http://github.com/cli/cli.git",
        "git://github.com/cli/cli.git",
        "https://github.com/cli",
        "https://github.com/cli/cli/extra",
        "https://github.com:port/cli/cli",
        "github.com:cli/cli",
        "git@github.com:/cli/cli",
        "https://github.com/-cli/cli",
        "https://github.com/cli/..",
        "https://-github.com/cli/cli",
    ] {
        assert_eq!(remote::parse(url), None, "{url}");
    }
    assert_eq!(
        remote::redact("https://u:tok@github.com/cli/cli"),
        "https://***@github.com/cli/cli"
    );

    // A host other than github.com that `gh auth status` does not know is refused as
    // "not a GitHub repository"; github.com logged out is "not logged in".
    let tmp = tempfile::tempdir().unwrap();
    let url = "https://git.example.org/cli/cli.git";
    let runner = preflight_until_auth(url).fails(
        "",
        "You are not logged into any accounts on git.example.org\n",
    );
    let refused = host(runner)
        .preflight(&preflight_req(tmp.path()))
        .unwrap_err();
    assert_eq!(
        refused,
        HostError::Rejected(format!(
            "remote origin is not a GitHub repository ({url}); use --delivery local"
        ))
    );
    let runner = preflight_until_auth("git@github.com:cli/cli.git")
        .fails("", "You are not logged into any accounts on github.com\n");
    let refused = host(runner)
        .preflight(&preflight_req(tmp.path()))
        .unwrap_err();
    assert_eq!(
        refused,
        HostError::Auth(
            "gh is not logged in to github.com; run gh auth login, or use --delivery local"
                .to_string()
        )
    );
    // Not a GitHub URL at all, and no URL.
    let runner = ScriptedRunner::new().ok("/srv/git/cli.git\n");
    let refused = host(runner)
        .preflight(&preflight_req(tmp.path()))
        .unwrap_err();
    assert_eq!(
        refused.text(),
        "remote origin is not a GitHub repository (/srv/git/cli.git); use --delivery local"
    );
    let runner = ScriptedRunner::new().fails("", "");
    let refused = host(runner)
        .preflight(&preflight_req(tmp.path()))
        .unwrap_err();
    assert_eq!(
        refused.text(),
        "remote origin is not set in this repository; use --delivery local"
    );
}

fn view_with(view: &str, threads: &str) -> Result<PrView, HostError> {
    let tmp = tempfile::tempdir().unwrap();
    let runner = ScriptedRunner::new().ok(view).ok(threads);
    host(runner).view_pr(&repo(tmp.path()), 13788)
}

#[test]
fn view_pr_parses_the_recorded_fixtures() {
    let red = view_with(PR_VIEW_OPEN_RED, THREADS_EXTENDED).unwrap();
    assert_eq!(red.number, 13788);
    assert_eq!(red.state, PrState::Open);
    assert_eq!(red.merged_at, None);
    assert_eq!(red.merge_commit, None);
    assert_eq!(red.base_ref, "trunk");
    assert_eq!(red.head_oid, SHA2);
    assert_eq!(red.mergeable, Mergeable::Mergeable);
    assert_eq!(red.review_decision.as_deref(), Some("REVIEW_REQUIRED"));
    assert_eq!(red.checks.len(), 11);
    assert_eq!(
        red.checks[0],
        CheckRun {
            name: "build (ubuntu-latest)".to_string(),
            status: CheckStatus::Completed,
            conclusion: Some(Conclusion::Failure),
            ci_run: Some(28656994029),
            url: "https://github.com/cli/cli/actions/runs/28656994029/job/84988385122".to_string(),
        }
    );
    let red_checks: Vec<&str> = red
        .checks
        .iter()
        .filter(|c| c.conclusion.is_some_and(Conclusion::is_red))
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        red_checks,
        [
            "build (ubuntu-latest)",
            "build (windows-latest)",
            "build (macos-latest)"
        ]
    );
    let codeql = red.checks.last().unwrap();
    assert_eq!((codeql.name.as_str(), codeql.ci_run), ("CodeQL", None));

    // From the GraphQL read (ruling R-2), newest first, bots by `__typename`.
    let ids: Vec<u64> = red.reviews.iter().map(|r| r.id).collect();
    assert_eq!(ids, [3101746448, 3098409423, 3082101354, 3080970818]);
    assert_eq!(
        red.reviews[2],
        Review {
            id: 3082101354,
            author: Author {
                login: "tester".to_string(),
                bot: false
            },
            state: ReviewState::ChangesRequested,
            body: "Breaking: the exit code changed. Please keep the old behaviour.\n\n- `gh ext upgrade --all` (text replaced)".to_string(),
        }
    );
    assert!(red.reviews[3].author.bot);
    assert_eq!(
        red.comments,
        vec![
            IssueComment {
                id: 5154974588,
                author: Author {
                    login: "alice".to_string(),
                    bot: false
                },
                body: "Comment 1 by a person (text replaced).".to_string(),
            },
            IssueComment {
                id: 5093042634,
                author: Author {
                    login: "github-actions".to_string(),
                    bot: true
                },
                body: "Thanks for your pull request! (bot text, replaced)".to_string(),
            },
        ]
    );
    let firsts: Vec<u64> = red.threads.iter().map(|t| t.comments[0].id).collect();
    assert_eq!(firsts, [3658443294, 987654321, 1234567]);
    let open = &red.threads[0];
    assert!(!open.resolved);
    assert_eq!(open.path.as_deref(), Some("pkg/extensions/extension.go"));
    assert_eq!(open.line, Some(34));
    assert_eq!(open.comments.len(), 2);
    assert_eq!(open.comments[1].id, 3658444294);
    assert_eq!(open.comments[1].author.login, "bob");
    assert!(
        open.comments[0]
            .diff_hunk
            .starts_with("@@ -28,12 +28,24 @@")
    );
    assert!(red.threads[1].comments[0].author.bot);
    assert!(red.threads[2].resolved);
    assert_eq!(red.threads[2].line, None);

    let conflicting = view_with(PR_VIEW_CONFLICTING, THREADS_EXTENDED).unwrap();
    assert_eq!(conflicting.mergeable, Mergeable::Conflicting);
    assert_eq!(
        conflicting.review_decision.as_deref(),
        Some("CHANGES_REQUESTED")
    );
    assert!(conflicting.checks.is_empty());

    let merged = view_with(PR_VIEW_MERGED, THREADS_EXTENDED).unwrap();
    assert_eq!(merged.state, PrState::Merged);
    assert_eq!(merged.merged_at, Some(1790703776));
    assert_eq!(
        merged.merge_commit,
        Some(MergeCommit {
            oid: "1863cb7c0f1d27e9479a95edb30ba97fcc6c01b1".to_string()
        })
    );
    assert_eq!(merged.mergeable, Mergeable::Unknown);
    assert_eq!(merged.checks.len(), 18);
    assert!(
        merged
            .checks
            .iter()
            .all(|c| c.status == CheckStatus::Completed)
    );
    assert_eq!(merged.checks[7].conclusion, Some(Conclusion::Skipped));

    // A status context has no Actions run; PENDING is pending, SUCCESS green.
    let mut view: Value = serde_json::from_str(PR_VIEW_OPEN_RED).unwrap();
    let contexts: Value = serde_json::from_str(STATUS_CONTEXT).unwrap();
    view["statusCheckRollup"] = contexts["statusCheckRollup"].clone();
    let ctx = view_with(&view.to_string(), THREADS_EXTENDED).unwrap();
    assert_eq!(ctx.checks.len(), 2);
    assert_eq!(ctx.checks[0].name, "tide");
    assert_eq!(ctx.checks[0].status, CheckStatus::Pending);
    assert_eq!(ctx.checks[0].conclusion, None);
    assert_eq!(ctx.checks[0].ci_run, None);
    assert_eq!(ctx.checks[1].name, "EasyCLA");
    assert_eq!(ctx.checks[1].conclusion, Some(Conclusion::Success));
    assert!(ctx.checks[1].url.starts_with("https://easycla"));

    // Ruling R-5: a check run with a conclusion anthrex does not know is pending.
    let mut odd: Value = serde_json::from_str(PR_VIEW_OPEN_RED).unwrap();
    odd["statusCheckRollup"][0]["conclusion"] = Value::String("SOMETHING_NEW".into());
    odd["statusCheckRollup"][1]["status"] = Value::String("WAITING".into());
    let odd = view_with(&odd.to_string(), THREADS_EXTENDED).unwrap();
    assert_eq!(odd.checks[0].status, CheckStatus::Pending);
    assert_eq!(odd.checks[0].conclusion, None);
    assert_eq!(odd.checks[1].status, CheckStatus::Pending);

    // Bodies are cut to VIEW_TEXT_MAX characters.
    let mut long: Value = serde_json::from_str(THREADS_EXTENDED).unwrap();
    let pr = &mut long["data"]["repository"]["pullRequest"];
    pr["comments"]["nodes"][1]["body"] = Value::String("é".repeat(VIEW_TEXT_MAX + 500));
    pr["reviewThreads"]["nodes"][0]["comments"]["nodes"][0]["body"] =
        Value::String("x".repeat(VIEW_TEXT_MAX * 2));
    let cut = view_with(PR_VIEW_OPEN_RED, &long.to_string()).unwrap();
    assert_eq!(cut.comments[0].body.chars().count(), VIEW_TEXT_MAX);
    assert_eq!(
        cut.threads[0].comments[0].body.chars().count(),
        VIEW_TEXT_MAX
    );

    // Only the fields M9.2.1 recorded are read; a missing one is never guessed.
    let mut deprecated: Value = serde_json::from_str(THREADS_DEPRECATED_ID).unwrap();
    let pr = &mut deprecated["data"]["repository"]["pullRequest"];
    pr["reviews"] = serde_json::json!({ "nodes": [] });
    pr["comments"] = serde_json::json!({ "nodes": [] });
    assert_eq!(
        view_with(PR_VIEW_OPEN_RED, &deprecated.to_string()),
        Err(HostError::Rejected(
            "gh output changed: fullDatabaseId".to_string()
        ))
    );
    let mut no_state: Value = serde_json::from_str(PR_VIEW_OPEN_RED).unwrap();
    no_state.as_object_mut().unwrap().remove("mergeable");
    assert_eq!(
        view_with(&no_state.to_string(), THREADS_EXTENDED),
        Err(HostError::Rejected(
            "gh output changed: mergeable".to_string()
        ))
    );
}

#[test]
fn errors_classify_rate_limit_auth_not_found_and_rejected() {
    let errors: Value = serde_json::from_str(GH_ERRORS).unwrap();
    let kind = |key: &str| classify(errors[key]["stderr"].as_str().unwrap());
    assert_eq!(
        kind("auth_missing"),
        HostError::Auth("You are not logged into any accounts on example.invalid".to_string())
    );
    assert!(matches!(kind("repo_missing"), HostError::NotFound(_)));
    assert!(matches!(kind("pr_missing"), HostError::NotFound(_)));
    assert!(matches!(kind("run_missing"), HostError::NotFound(_)));
    // The anthrex user lacks push access: not "no permission" (finding 5).
    assert_eq!(
        kind("permission_without_push_access"),
        HostError::Failed(
            "gh: Must have push access to view collaborator permission. (HTTP 403)".to_string()
        )
    );
    for key in [
        "rate_limit_rest",
        "rate_limit_graphql",
        "rate_limit_secondary",
    ] {
        assert!(matches!(kind(key), HostError::RateLimited(_)), "{key}");
    }
    assert!(matches!(kind("rerun_in_progress"), HostError::Failed(_)));
    for stderr in [
        "To ../remote.git\n ! [rejected]        x -> x (fetch first)\n",
        "error: failed to push some refs\nhint: Updates were rejected (non-fast-forward)\n",
    ] {
        assert!(
            matches!(classify(stderr), HostError::Rejected(_)),
            "{stderr}"
        );
    }
    assert!(matches!(
        classify("gh: HTTP 401: Bad credentials"),
        HostError::Auth(_)
    ));
    assert!(matches!(
        classify(
            "fatal: unable to access 'https://github.com/o/r/': Could not resolve host: github.com"
        ),
        HostError::Failed(_)
    ));

    // Through the methods: a missing collaborator is no permission; a rerun GitHub is
    // already running is success; the 403 stays an error.
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo(tmp.path());
    let not_found = "gh: Not Found (HTTP 404)\n";
    let h = host(ScriptedRunner::new().fails("{\"message\":\"Not Found\"}", not_found));
    assert_eq!(
        h.permission(&repo, "octocat").unwrap(),
        RepoPermission::None
    );
    let forbidden = errors["permission_without_push_access"]["stderr"]
        .as_str()
        .unwrap();
    let h = host(ScriptedRunner::new().fails("", forbidden));
    assert!(matches!(
        h.permission(&repo, "octocat"),
        Err(HostError::Failed(_))
    ));
    let running = errors["rerun_in_progress"]["stderr"].as_str().unwrap();
    let h = host(ScriptedRunner::new().fails("", running));
    assert_eq!(h.rerun_failed(&repo, 1), Ok(()));
    let limited = errors["rate_limit_graphql"]["stderr"].as_str().unwrap();
    let h = host(
        ScriptedRunner::new()
            .ok(PR_VIEW_OPEN_RED)
            .fails("", limited),
    );
    assert!(matches!(
        h.view_pr(&repo, 1),
        Err(HostError::RateLimited(_))
    ));
    // A timed-out preflight check names the check.
    let h = host(
        ScriptedRunner::new()
            .ok("https://github.com/cli/cli.git\n")
            .answer(Err(HostError::TimedOut(
                "gh --version did not answer".into(),
            ))),
    );
    assert_eq!(
        h.preflight(&preflight_req(tmp.path())),
        Err(HostError::TimedOut(
            "gh --version: gh did not answer within 30 s".to_string()
        ))
    );
    // `gh` older than the version M9.2.1 checked is refused.
    let h = host(
        ScriptedRunner::new()
            .ok("https://github.com/cli/cli.git\n")
            .ok("gh version 2.40.1 (2023-12-13)\n"),
    );
    assert!(
        h.preflight(&preflight_req(tmp.path()))
            .unwrap_err()
            .text()
            .starts_with("gh 2.40.1 is older than 2.92.0")
    );
    assert_eq!(gh_parse::gh_version(VERSION), Some((2, 92, 0)));
}

#[test]
fn open_pr_returns_an_existing_pr_for_the_head() {
    let tmp = tempfile::tempdir().unwrap();
    let req = OpenPrReq {
        repo: repo(tmp.path()),
        run_id: "r1a2b".to_string(),
        base: "trunk".to_string(),
        head: format!("anthrex/{RUN}/stage-1"),
        title: "[anthrex r1a2b 1/1] Add the parser".to_string(),
        body_file: tmp.path().join("pr-1.md"),
    };
    // The repository's own PR for the head; the fork's open PR of the same branch name
    // (ruling R-10) is not ours, even though it is open and newer.
    let h = host(ScriptedRunner::new().ok(PR_LIST_HEAD_OWNER));
    assert_eq!(
        h.open_pr(&req).unwrap(),
        PrRef {
            number: 13940,
            url: "https://github.com/cli/cli/pull/13940".to_string(),
            state: PrState::Merged,
            existed: true,
        }
    );
    assert_eq!(h.runner().calls().len(), 1, "no gh pr create runs");
    assert!(
        !h.runner()
            .argvs()
            .iter()
            .any(|a| a.get(1).is_some_and(|c| c == "create"))
    );

    // Only the fork has one: a new PR is created.
    let mut rows: Value = serde_json::from_str(PR_LIST_HEAD_OWNER).unwrap();
    rows.as_array_mut().unwrap().remove(0);
    let h = host(
        ScriptedRunner::new()
            .ok(&rows.to_string())
            .ok("https://github.com/cli/cli/pull/14001\n"),
    );
    let made = h.open_pr(&req).unwrap();
    assert_eq!((made.number, made.existed), (14001, false));
    assert_eq!(h.runner().argvs()[1][1], "create");

    // gh's answer without the owner fields (M9.2.1's recording predates R-10) is never
    // read as "none of ours".
    let h = host(ScriptedRunner::new().ok(PR_LIST_HEAD));
    assert_eq!(
        h.open_pr(&req),
        Err(HostError::Rejected(
            "gh output changed: headRepositoryOwner".to_string()
        ))
    );
    assert_eq!(h.runner().calls().len(), 1);
}

#[test]
fn reply_skips_when_its_marker_is_there() {
    let tmp = tempfile::tempdir().unwrap();
    let req = ReplyReq {
        repo: repo(tmp.path()),
        number: 142,
        target: ReplyTarget::Thread {
            comment_id: 3658443294,
        },
        body: "Addressed in 1a2b3c4 by task fix4.".to_string(),
        marker: "<!-- anthrex:reply r1a2b 142:t3658443294 1a2b3c4 -->".to_string(),
    };
    let h = host(ScriptedRunner::new().ok(REST_REVIEW_COMMENTS));
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(
        h.runner().argvs(),
        vec![argv(&[
            "api",
            "repos/cli/cli/pulls/142/comments",
            "--paginate"
        ])]
    );
    // `--paginate` prints one array per page; the marker can be on any of them.
    let comments: Value = serde_json::from_str(REST_REVIEW_COMMENTS).unwrap();
    let rows = comments.as_array().unwrap();
    let pages = format!(
        "{}\n{}",
        Value::Array(rows[..2].to_vec()),
        Value::Array(rows[2..].to_vec())
    );
    let h = host(ScriptedRunner::new().ok(&pages));
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(h.runner().calls().len(), 1);
    // A conversation whose listing has the marker posts nothing either.
    let mut issue: Value = serde_json::from_str(REST_ISSUE_COMMENTS).unwrap();
    let marker = "<!-- anthrex:reply r1a2b 142:c5154974588 1a2b3c4 -->";
    issue[1]["body"] = Value::String(format!("@alice Addressed.\n\n{marker}"));
    let h = host(ScriptedRunner::new().ok(&issue.to_string()));
    let conversation = ReplyReq {
        target: ReplyTarget::Conversation,
        marker: marker.to_string(),
        ..req
    };
    assert_eq!(h.reply(&conversation).unwrap(), 5154974588);
    assert_eq!(h.runner().calls().len(), 1);
}

/// A stand-in `gh` that prints `log` `times` times (the only process this file starts).
fn stand_in_gh(dir: &Path, log: &Path, times: usize) -> PathBuf {
    let script = dir.join("gh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt {times} ]; do cat '{}'; i=$((i + 1)); done\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn failed_logs_keeps_the_head_and_the_tail_under_the_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("fixture.log");
    std::fs::write(&fixture, RUN_LOG_FAILED).unwrap();
    let repo = repo(tmp.path());
    let first_line = RUN_LOG_FAILED.lines().next().unwrap();
    let last_line = RUN_LOG_FAILED.lines().last().unwrap();

    // 1000 copies (about 700 KB) under a 256 KiB cap.
    let gh = stand_in_gh(tmp.path(), &fixture, 1000);
    let host = GhHost::new(SystemRunner::new(&gh, "git"), &gh, "git");
    let cap = 256 * 1024;
    let out = tmp.path().join("ci-1.log");
    let file = host.failed_logs(&repo, 28656994029, cap, &out).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert!(file.truncated);
    assert_eq!(file.bytes, bytes.len() as u64);
    assert!(file.bytes <= cap, "{} > {cap}", file.bytes);
    assert!(file.bytes > cap - 128, "the cap is used: {}", file.bytes);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with(first_line), "the head is kept");
    assert!(text.trim_end().ends_with(last_line), "the tail is kept");
    let dropped = RUN_LOG_FAILED.len() as u64 * 1000 - (bytes.len() as u64)
        + text.lines().find(|l| l.contains("cut here")).unwrap().len() as u64
        + 2;
    assert!(
        text.contains(&format!("… {dropped} bytes of the log cut here …")),
        "{dropped}"
    );
    let head_end = text.find("\n… ").unwrap();
    assert_eq!(head_end, 64 * 1024);
    // Task M9.2.9: the answer's `tail` is the file's last 48 KiB, for the engine.
    let want = &bytes[bytes.len() - crate::decider::CI_SUMMARY_INPUT_BYTES..];
    assert_eq!(file.tail, String::from_utf8_lossy(want));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Under the cap the log is kept whole.
    let gh = stand_in_gh(tmp.path(), &fixture, 3);
    let host = GhHost::new(SystemRunner::new(&gh, "git"), &gh, "git");
    let file = host.failed_logs(&repo, 1, cap, &out).unwrap();
    assert!(!file.truncated);
    assert_eq!(
        file.tail,
        RUN_LOG_FAILED.repeat(3),
        "a short log is its own tail"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        RUN_LOG_FAILED.repeat(3)
    );

    // A gh that is not there is `Missing`, naming the path.
    let host = GhHost::new(
        SystemRunner::new("/nonexistent/anthrex-test/gh", "git"),
        "/nonexistent/anthrex-test/gh",
        "git",
    );
    assert_eq!(
        host.failed_logs(&repo, 1, cap, &out),
        Err(HostError::Missing(
            "gh is not installed (looked for /nonexistent/anthrex-test/gh)".to_string()
        ))
    );
}
