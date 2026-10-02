//! Milestone 9.2 task M9.2.7, the controller's carried ruling: opening a stage PR is
//! idempotent across a daemon restart (decision 10). The engine's host ops are executed
//! here against `FakeHost` (`GhHost<FakeGh>`: the real argv, allow-list and parsing; a
//! local bare repository as the remote; no network, no `gh`), standing in for M9.2.12's
//! executor; a restart drops the op in flight, and the run resumed opens no second PR.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use proto::PrState;

use super::control::resume;
use super::control_restore::restart;
use super::delivery_open::{green, host_op, pr_on};
use super::fixture::*;
use super::merge::{doc_task, merge, to_queue, window_of};
use crate::host::fake::{FakeGithubCtl, FakeHost};
use crate::host::{CodeHost, HostRepo, OpenPrReq, PushReq};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::OpResult;
use crate::worktree::run_git;

const URL: &str = "https://github.com/fake/app.git";

fn git(dir: &Path, args: &[&str]) -> String {
    let mut all = vec![
        "-c",
        "user.name=anthrex test",
        "-c",
        "user.email=test@anthrex.invalid",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    all.extend_from_slice(args);
    let os: Vec<&OsStr> = all.iter().map(OsStr::new).collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    let out = run_git(OsStr::new("git"), dir, &os, deadline).unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

/// A repository whose `origin` is a GitHub URL mapped onto a local bare repository,
/// `FakeGithub` knowing it, and one commit on top of `main` to deliver.
struct Rig {
    _tmp: tempfile::TempDir,
    work: PathBuf,
    host: FakeHost,
    ctl: FakeGithubCtl,
    head: String,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let (bare, work, dir) = (
            tmp.path().join("remote.git"),
            tmp.path().join("work"),
            tmp.path().join("github"),
        );
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        let bare_s = bare.to_string_lossy().into_owned();
        git(&work, &["config", "remote.origin.url", URL]);
        for key in ["insteadOf", "pushInsteadOf"] {
            git(&work, &["config", &format!("url.{bare_s}.{key}"), URL]);
        }
        // Both directions reach the bare repository, never GitHub.
        assert_eq!(git(&work, &["remote", "get-url", "origin"]), bare_s);
        assert_eq!(
            git(&work, &["remote", "get-url", "--push", "origin"]),
            bare_s
        );
        std::fs::write(work.join("README.md"), "# app\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        git(&work, &["push", "-q", "origin", "main"]);
        std::fs::write(work.join("t1.md"), "t1's work\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "t1"]);
        let head = git(&work, &["rev-parse", "HEAD"]);
        let ctl = FakeGithubCtl::open(&dir);
        ctl.create_repo("fake", "app", &bare, "main");
        ctl.log_in("github.com");
        Rig {
            host: FakeHost::new(&dir),
            ctl,
            _tmp: tmp,
            work,
            head,
        }
    }

    fn repo(&self) -> HostRepo {
        HostRepo {
            host: "github.com".into(),
            owner: "fake".into(),
            name: "app".into(),
            remote: "origin".into(),
            root: self.work.clone(),
        }
    }

    /// M9.2.12's executor, reduced to the two ops opening needs.
    fn execute(&self, op: &HostOp) -> HostResult {
        match op {
            HostOp::Push { stage, sha } => {
                let req = PushReq {
                    repo: self.repo(),
                    run_id: RUN_ID.into(),
                    stage: *stage,
                    sha: sha.clone(),
                };
                HostResult::Pushed(self.host.push(&req).unwrap())
            }
            HostOp::OpenPr {
                stage,
                base,
                head,
                title,
                body,
            } => {
                let body_file = self.work.join(format!("../pr-{stage}.md"));
                std::fs::write(&body_file, body).unwrap();
                let req = OpenPrReq {
                    repo: self.repo(),
                    run_id: RUN_ID.into(),
                    base: base.clone(),
                    head: head.clone(),
                    title: title.clone(),
                    body_file,
                };
                HostResult::PrOpened(self.host.open_pr(&req).unwrap())
            }
            other => panic!("not an opening op: {other:?}"),
        }
    }

    fn creates(&self) -> usize {
        let calls = self.ctl.calls();
        calls
            .iter()
            .filter(|c| c.starts_with(&["pr".into(), "create".into()]))
            .count()
    }
}

/// Executes the pending host op on the rig; with `record`, the engine gets its answer.
fn run_op(fx: &mut Fixture, rig: &Rig, record: bool) -> HostOp {
    let (op, kind) = host_op(fx);
    let result = rig.execute(&kind);
    if record {
        fx.done(op, OpResult::Host(result));
    }
    kind
}

#[test]
fn opening_survives_restarts_without_a_second_pr() {
    let rig = Rig::new();
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", "")]);
    fx.run_mut().delivery.repo = Some(rig.repo());
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &rig.head);
    green(&mut fx, 1);

    // A restart between the push and its record: the push ran on the host, its answer
    // was lost, and the resumed run pushes again (up to date).
    assert!(matches!(run_op(&mut fx, &rig, false), HostOp::Push { .. }));
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    assert!(matches!(run_op(&mut fx, &rig, true), HostOp::Push { .. }));

    // A restart between the PR's creation and its record.
    assert!(matches!(
        run_op(&mut fx, &rig, false),
        HostOp::OpenPr { .. }
    ));
    assert_eq!(rig.creates(), 1);
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    assert!(matches!(run_op(&mut fx, &rig, true), HostOp::Push { .. }));
    assert!(matches!(run_op(&mut fx, &rig, true), HostOp::OpenPr { .. }));

    let prs = rig.ctl.prs();
    assert_eq!(prs.len(), 1, "one PR: {prs:#?}");
    assert_eq!(rig.creates(), 1, "gh pr create ran once");
    let pr = fx.run().delivery.pr(1).expect("recorded").clone();
    assert_eq!((pr.number, pr.state), (prs[0].number, PrState::Open));
    assert_eq!(pr.pushed_head, rig.head);
    assert_eq!(prs[0].head, format!("anthrex/{RUN_ID}/stage-1"));
    assert_eq!(prs[0].base, "main");
    assert!(rig.ctl.forbidden().is_empty());
}
