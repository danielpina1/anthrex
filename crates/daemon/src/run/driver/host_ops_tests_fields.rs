//! Task M9.2.12: what the executor hands the host, and what it hands the journal. Every
//! field of an op reaches its request (the carried rulings: `Reply.marker`,
//! `Fetch.parents_of`); a PR body and a CI log are private files; a large answer stays
//! out of the journal (a log by its path, a view by `view_trim`); a panicking host is a
//! halt; a remote whose URLs changed since preflight is refused.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::host::scripted::ScriptedRunner;
use crate::host::{Author, GhHost, IssueComment, ReplyTarget, ReviewThread, ThreadComment};
use crate::run::delivery::view_trim::COMMENT_KEPT_CHARS;
use crate::run::driver::effects::Ready;
use crate::run::journal::COMPACT_AFTER_BYTES;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn every_op_reaches_its_request() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = Arc::new(Stub::default());
    let e = exec(stub.clone(), Arc::new(GitQueue::new()));
    let here = at(tmp.path(), None);
    let run = |op| execute(&e, &here, repo(tmp.path()), op);

    let marker = format!("<!-- anthrex:reply {RUN_ID} 7:t5000000001 1a2b3c4 -->");
    let replied = run(HostOp::Reply {
        stage: 1,
        number: 7,
        thread: "t5000000001".into(),
        target: ReplyTarget::Thread {
            comment_id: 5_000_000_001,
        },
        body: "Addressed in 1a2b3c4 by task fix4.".into(),
        marker: marker.clone(),
    })
    .await;
    assert_eq!(
        replied,
        OpResult::Host(HostResult::Replied { comment_id: 42 })
    );
    let reply = crate::lock(&stub.reply).clone().unwrap();
    assert_eq!(reply.marker, marker, "the engine's marker reaches ReplyReq");
    assert_eq!((reply.number, reply.repo), (7, repo(tmp.path())));

    let fetched = run(HostOp::Fetch {
        stage: None,
        branch: "main".into(),
        into: format!("refs/anthrex/{RUN_ID}/remote/base"),
        adopt: None,
        parents_of: Some(SHA.into()),
    })
    .await;
    assert!(matches!(
        host_result(fetched),
        HostResult::Fetched(FetchOutcome::Fetched {
            parents: Some(2),
            ..
        })
    ));
    let fetch = crate::lock(&stub.fetch).clone().unwrap();
    assert_eq!(
        fetch.parents_of.as_deref(),
        Some(SHA),
        "ruling R-4's parents_of"
    );
    assert_eq!(fetch.run_id, RUN_ID);
    assert_eq!(fetch.branch, "main");

    // The body goes to a private file under the run's data directory, never the argv.
    let opened = run(HostOp::OpenPr {
        stage: 2,
        base: format!("anthrex/{RUN_ID}/stage-1"),
        head: format!("anthrex/{RUN_ID}/stage-2"),
        title: "[anthrex r1a2b 2/2] Two".into(),
        body: "<!-- anthrex:pr r1a2b stage 2 -->\nthe body".into(),
    })
    .await;
    assert!(matches!(host_result(opened), HostResult::PrOpened(_)));
    let (req, body, file_mode) = crate::lock(&stub.opened).clone().unwrap();
    let file = here.data_dir.join(DELIVERY_DIR).join("pr-2.md");
    assert_eq!(req.body_file, file);
    assert_eq!(body, "<!-- anthrex:pr r1a2b stage 2 -->\nthe body");
    assert_eq!(file_mode, 0o600);
    assert_eq!(mode(&here.data_dir.join(DELIVERY_DIR)), 0o700);
    assert_eq!(
        (req.run_id.as_str(), req.base),
        (RUN_ID, format!("anthrex/{RUN_ID}/stage-1"))
    );

    let answers = [
        (
            HostOp::RerunFailed {
                stage: 1,
                ci_run: 9,
            },
            HostResult::Rerun,
        ),
        (
            HostOp::Retarget {
                stage: 2,
                number: 8,
                base: "main".into(),
            },
            HostResult::Retargeted,
        ),
        (
            HostOp::Permission {
                user: "alice".into(),
            },
            HostResult::Permission {
                user: "alice".into(),
                permission: RepoPermission::Write,
            },
        ),
        (HostOp::DeleteBranch { stage: 1 }, HostResult::Deleted),
    ];
    for (op, want) in answers {
        assert_eq!(run(op).await, OpResult::Host(want));
    }
}

/// The carried ruling: a host call that panics (a `FakeGh` asked to land something) is
/// never answered as a retryable error. It is `Forbidden`, on which the engine halts
/// the run (decision 11), through the queue and off it alike.
#[tokio::test]
async fn a_panicking_host_is_a_halt_never_a_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = Arc::new(Stub {
        panics: true,
        ..Stub::default()
    });
    let e = exec(stub, Arc::new(GitQueue::new()));
    for (op, verb) in [
        (
            HostOp::Push {
                stage: 1,
                sha: SHA.into(),
            },
            "merge",
        ),
        (
            HostOp::Permission {
                user: "alice".into(),
            },
            "approve",
        ),
    ] {
        let answer = host_result(execute(&e, &at(tmp.path(), None), repo(tmp.path()), op).await);
        let HostResult::Error(HostError::Forbidden(text)) = answer else {
            panic!("not a halt: {answer:?}");
        };
        assert!(
            text.starts_with(&format!(
                "the host call panicked: FakeHost: anthrex asked to {verb}"
            )),
            "{text}"
        );
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let os: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    let out = crate::worktree::run_git(std::ffi::OsStr::new("git"), dir, &os, deadline).unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

/// The controller's ruling: preflight seals the remote's URLs; a push, fetch or delete
/// is refused once they change (`url`, `pushurl` or an `insteadOf` rewrite), and the
/// host is never asked. A read through `gh` is not affected.
#[tokio::test]
async fn a_changed_remote_refuses_push_and_fetch() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("work");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &[
            "config",
            "remote.origin.url",
            "https://github.com/fake/app.git",
        ],
    );
    let sealed = crate::run::driver::delivery::seal("git".as_ref(), &root, "origin").unwrap();
    let push = || HostOp::Push {
        stage: 1,
        sha: SHA.into(),
    };
    let stub = Arc::new(Stub::default());
    let e = exec(stub.clone(), Arc::new(GitQueue::new()));
    let here = at(tmp.path(), Some(sealed.clone()));
    let answer = execute(&e, &here, repo(&root), push()).await;
    assert_eq!(
        answer,
        OpResult::Host(HostResult::Pushed(PushOutcome::Pushed))
    );

    let changes: [&[&str]; 3] = [
        &[
            "config",
            "remote.origin.pushurl",
            "https://example.invalid/x.git",
        ],
        &[
            "config",
            "remote.origin.url",
            "https://example.invalid/y.git",
        ],
        &[
            "config",
            "url./tmp/elsewhere.git.pushInsteadOf",
            "https://github.com/fake/app.git",
        ],
    ];
    for (i, change) in changes.into_iter().enumerate() {
        let fresh = tmp.path().join(format!("w{i}"));
        std::fs::create_dir_all(&fresh).unwrap();
        git(&fresh, &["init", "-q", "-b", "main"]);
        git(
            &fresh,
            &[
                "config",
                "remote.origin.url",
                "https://github.com/fake/app.git",
            ],
        );
        let sealed = crate::run::driver::delivery::seal("git".as_ref(), &fresh, "origin").unwrap();
        git(&fresh, change);
        let stub = Arc::new(Stub::default());
        let e = exec(stub.clone(), Arc::new(GitQueue::new()));
        let here = at(tmp.path(), Some(sealed));
        for op in [push(), HostOp::DeleteBranch { stage: 1 }] {
            let answer = host_result(execute(&e, &here, repo(&fresh), op).await);
            let HostResult::Error(HostError::Forbidden(text)) = answer else {
                panic!("{change:?}: not refused: {answer:?}");
            };
            assert_eq!(
                text,
                "remote origin's URLs changed since the run started; anthrex pushes and fetches only where preflight checked (restore remote.origin.url and pushurl, then anthrex run resume)"
            );
        }
        assert!(
            crate::lock(&stub.calls).is_empty(),
            "{change:?}: the host was asked"
        );
        let viewed = execute(
            &e,
            &here,
            repo(&fresh),
            HostOp::ViewPr {
                stage: 1,
                number: 7,
            },
        )
        .await;
        assert!(matches!(host_result(viewed), HostResult::PrViewed(_)));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_logs_are_written_0600_and_kept_out_of_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("runs").join(RUN_ID);
    // A 300 KiB log: its first line is cut away from the journal, its last is kept.
    let mut log = String::from("HEAD-OF-THE-LOG build\tUNKNOWN STEP\tfirst line\n");
    while log.len() < 300 * 1024 {
        log.push_str("test\tUNKNOWN STEP\t2026-10-02T10:00:00Z running the suite\n");
    }
    log.push_str("test\tUNKNOWN STEP\t2026-10-02T10:00:01Z --- FAIL: TestTail (0.01s)\n");
    let host = GhHost::new(
        ScriptedRunner::new().ok(&log),
        "/nonexistent/anthrex-test/gh",
        "git",
    );
    let s = service_with(Arc::new(host), tmp.path());
    let op = HostOp::FailedLogs {
        stage: 1,
        ci_run: 28_000_000_001,
        max_bytes: 200_000,
    };
    let ctx = op_ctx(&data_dir, tmp.path());
    let kind = host_kind(tmp.path(), op);
    s.execute(vec![Ready::Op { ctx, op: 3, kind }], 0).await;
    let (line, result) = done_line(&data_dir, 3).await;

    let file = data_dir.join(DELIVERY_DIR).join("ci-28000000001.log");
    let OpResult::Host(HostResult::Logs(logs)) = result else {
        panic!("{result:?}");
    };
    assert_eq!(logs.path, file);
    assert!(logs.truncated);
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(&data_dir.join(DELIVERY_DIR)), 0o700);
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        written.starts_with("HEAD-OF-THE-LOG"),
        "the head is kept in the file"
    );
    assert!(written.len() <= 200_000);
    // The journal names the file and carries only the decider's tail.
    assert!(
        !line.contains("HEAD-OF-THE-LOG"),
        "the log is not journaled"
    );
    assert!(line.contains("TestTail"));
    assert!(line.contains("ci-28000000001.log"));
    assert!(
        line.len() < crate::decider::CI_SUMMARY_INPUT_BYTES + 8 * 1024,
        "{}",
        line.len()
    );
    s.stop().await;
}

/// Task M9.2.10's fix round, concern 2, at its call site: a view is cut to `run.json`'s
/// caps before its `done` line is written (`effects.rs`, `view_trim::journaled`).
#[tokio::test(flavor = "multi_thread")]
async fn a_large_view_is_trimmed_before_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("runs").join(RUN_ID);
    let author = Author {
        login: "alice".into(),
        bot: false,
    };
    let long = "é".repeat(8_000);
    let mut big = view(7);
    big.comments = (0..100)
        .map(|i| IssueComment {
            id: 5_000_000_000 + i,
            author: author.clone(),
            body: long.clone(),
        })
        .collect();
    big.threads = (0..100)
        .map(|t| ReviewThread {
            resolved: false,
            path: Some("src/lib.rs".into()),
            line: Some(1),
            comments: (0..50)
                .map(|c| ThreadComment {
                    id: 6_000_000_000 + t * 100 + c,
                    author: author.clone(),
                    body: long.clone(),
                    diff_hunk: "@@ -1 +1 @@".into(),
                })
                .collect(),
        })
        .collect();
    let stub = Arc::new(Stub::default());
    *crate::lock(&stub.view) = Some(big);
    let s = service_with(stub, tmp.path());
    let ctx = op_ctx(&data_dir, tmp.path());
    let kind = host_kind(
        tmp.path(),
        HostOp::ViewPr {
            stage: 1,
            number: 7,
        },
    );
    s.execute(vec![Ready::Op { ctx, op: 4, kind }], 0).await;
    let (line, result) = done_line(&data_dir, 4).await;
    assert!((line.len() as u64) < COMPACT_AFTER_BYTES, "{}", line.len());
    let OpResult::Host(HostResult::PrViewed(viewed)) = result else {
        panic!("{result:?}");
    };
    assert_eq!(viewed.comments.len(), 100, "every id is kept");
    for comment in &viewed.comments {
        let kept = comment.body.chars().count();
        assert!(
            kept <= COMMENT_KEPT_CHARS,
            "a comment kept {kept} characters"
        );
    }
    s.stop().await;
}
