//! Task M9.2.12: decision 8's executor. Host calls run off the engine lock and off the
//! event loop, within decision 9's bound; pushes, fetches and deletes wait for the
//! project's git queue; every field of an op reaches its request; a panicking host is a
//! halt, never a retry; a changed remote is refused. The hosts here are test stand-ins
//! for `CodeHost` (no process, no network) or `GhHost` over a scripted runner.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::run_wire::request;
use proto::{PrState, RunReply, RunRequest};

use super::*;
use crate::host::{
    Adopt, Capture, FetchOutcome, GhHost, LogFile, Mergeable, PrRef, PrView, PreflightReq, Program,
    PushOutcome, RepoPermission, RunOutput, Runner,
};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::effects::Ready;
use crate::run::driver::{OpCtx, RunContext, RunService};
use crate::run::journal::{JOURNAL_FILE, JournalLine};
use crate::run::model::OpId;

pub(in crate::run::driver) struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

pub(in crate::run::driver) const RUN_ID: &str = "r1a2b";
const SHA: &str = "a560bea91b8cd58b3c0d78e5db98c7fdc8e5036b";

/// A `CodeHost` that records each call, answers it, and can hold `view_pr` until the
/// test releases it, sleep in it, or panic in `push` and `permission` as `FakeGh` does
/// when asked to land something.
#[derive(Default)]
pub(in crate::run::driver) struct Stub {
    pub calls: Mutex<Vec<String>>,
    /// While set, `view_pr` waits for a message on it.
    pub gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    pub entered: AtomicBool,
    pub view: Mutex<Option<PrView>>,
    pub panics: bool,
    /// Set by the test before it releases the queue: a write that ran before it is a
    /// write that did not wait.
    pub released: AtomicBool,
    pub early: Mutex<Vec<String>>,
    pub fetch: Mutex<Option<FetchReq>>,
    pub reply: Mutex<Option<ReplyReq>>,
    pub opened: Mutex<Option<(OpenPrReq, String, u32)>>,
}

impl Stub {
    fn record(&self, call: &str) {
        crate::lock(&self.calls).push(call.to_string());
    }

    fn write(&self, call: &str) {
        self.record(call);
        if !self.released.load(Ordering::SeqCst) {
            crate::lock(&self.early).push(call.to_string());
        }
    }

    pub(in crate::run::driver) fn called(&self, call: &str) -> bool {
        crate::lock(&self.calls).iter().any(|c| c == call)
    }
}

pub(in crate::run::driver) fn view(number: u64) -> PrView {
    PrView {
        number,
        state: PrState::Open,
        merged_at: None,
        merge_commit: None,
        base_ref: "main".into(),
        head_oid: SHA.into(),
        mergeable: Mergeable::Mergeable,
        review_decision: None,
        checks: Vec::new(),
        reviews: Vec::new(),
        comments: Vec::new(),
        threads: Vec::new(),
    }
}

impl CodeHost for Stub {
    fn preflight(&self, _: &PreflightReq) -> Result<HostRepo, HostError> {
        unreachable!("the executor never preflights")
    }
    fn detect(&self, _: &Path, _: &str) -> Result<HostRepo, HostError> {
        unreachable!("the executor never detects")
    }
    fn push(&self, _: &PushReq) -> Result<PushOutcome, HostError> {
        if self.panics {
            panic!("FakeHost: anthrex asked to merge gh pr merge 7; anthrex never lands anything");
        }
        self.write("push");
        Ok(PushOutcome::Pushed)
    }
    fn fetch(&self, req: &FetchReq) -> Result<FetchOutcome, HostError> {
        self.write("fetch");
        *crate::lock(&self.fetch) = Some(req.clone());
        Ok(FetchOutcome::Fetched {
            sha: SHA.into(),
            parents: Some(2),
        })
    }
    fn open_pr(&self, req: &OpenPrReq) -> Result<PrRef, HostError> {
        self.record("open_pr");
        use std::os::unix::fs::PermissionsExt;
        let body = std::fs::read_to_string(&req.body_file).unwrap();
        let mode = std::fs::metadata(&req.body_file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        *crate::lock(&self.opened) = Some((req.clone(), body, mode));
        Ok(PrRef {
            number: 7,
            url: "https://github.com/fake/app/pull/7".into(),
            state: PrState::Open,
            existed: false,
            base: None,
        })
    }
    fn view_pr(&self, _: &HostRepo, number: u64) -> Result<PrView, HostError> {
        self.record("view_pr");
        self.entered.store(true, Ordering::SeqCst);
        let gate = crate::lock(&self.gate).take();
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(Duration::from_secs(30));
        }
        Ok(crate::lock(&self.view)
            .clone()
            .unwrap_or_else(|| view(number)))
    }
    fn failed_logs(&self, _: &HostRepo, _: u64, _: u64, _: &Path) -> Result<LogFile, HostError> {
        unreachable!("the log test uses GhHost")
    }
    fn rerun_failed(&self, _: &HostRepo, _: u64) -> Result<(), HostError> {
        self.record("rerun_failed");
        Ok(())
    }
    fn reply(&self, req: &ReplyReq) -> Result<u64, HostError> {
        self.record("reply");
        *crate::lock(&self.reply) = Some(req.clone());
        Ok(42)
    }
    fn retarget(&self, _: &HostRepo, _: u64, _: &str) -> Result<(), HostError> {
        self.record("retarget");
        Ok(())
    }
    fn permission(&self, _: &HostRepo, _: &str) -> Result<RepoPermission, HostError> {
        if self.panics {
            panic!(
                "FakeHost: anthrex asked to approve gh pr review 7; anthrex never lands anything"
            );
        }
        self.record("permission");
        Ok(RepoPermission::Write)
    }
    fn delete_branch(&self, _: &DeleteBranchReq) -> Result<(), HostError> {
        self.write("delete_branch");
        Ok(())
    }
    /// The real seal: `GhHost`'s two reads, through the allow-list, on real git.
    fn remote_seal(&self, root: &Path, remote: &str) -> Result<String, HostError> {
        real_host().remote_seal(root, remote)
    }
}

/// `GhHost` over the system's git and a `gh` that does not exist.
pub(in crate::run::driver) fn real_host() -> Arc<dyn CodeHost> {
    crate::host::select::build(&crate::host::select::CodeHostChoice::Gh {
        bin: crate::manager::TEST_GH_BIN.into(),
    })
}

/// `root`'s `origin`, sealed as preflight seals it.
pub(in crate::run::driver) fn sealed(root: &Path) -> String {
    real_host().remote_seal(root, "origin").unwrap()
}

pub(in crate::run::driver) fn repo(root: &Path) -> HostRepo {
    HostRepo {
        host: "github.com".into(),
        owner: "fake".into(),
        name: "app".into(),
        remote: "origin".into(),
        root: root.to_path_buf(),
    }
}

pub(in crate::run::driver) fn at(dir: &Path, seal: Option<String>) -> HostAt {
    HostAt {
        run_id: RUN_ID.into(),
        project: dir.join("project"),
        data_dir: dir.join("data"),
        seal,
    }
}

pub(in crate::run::driver) fn exec(host: Arc<dyn CodeHost>, queue: Arc<GitQueue>) -> HostExec {
    HostExec {
        host,
        queue,
        cap: None,
    }
}

fn host_result(result: OpResult) -> HostResult {
    match result {
        OpResult::Host(result) => result,
        other => panic!("not a host answer: {other:?}"),
    }
}

/// A service whose code host is `host`, its data under `data`; no agent can start.
pub(in crate::run::driver) fn service_with(
    host: Arc<dyn CodeHost>,
    data: &Path,
) -> Arc<RunService> {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let ctx = RunContext::new(
        data.to_path_buf(),
        manager.config(),
        config::Orchestrator::default(),
        Arc::new(NoRoots),
    )
    .with_host(host);
    RunService::new(manager, ctx)
}

pub(in crate::run::driver) fn op_ctx(data_dir: &Path, project: &Path) -> OpCtx {
    OpCtx {
        run_id: RUN_ID.into(),
        project: project.to_path_buf(),
        data_dir: data_dir.to_path_buf(),
        git_timeout: Duration::from_secs(30),
        check_timeout: Duration::from_secs(30),
        confine: None,
    }
}

pub(in crate::run::driver) fn host_kind(root: &Path, op: HostOp) -> crate::run::engine::OpKind {
    crate::run::engine::OpKind::Host {
        repo: repo(root),
        op,
    }
}

/// Op `op`'s `done` line in `data_dir`'s journal, waited for with a deadline.
pub(in crate::run::driver) async fn done_line(data_dir: &Path, op: OpId) -> (String, OpResult) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let text = std::fs::read_to_string(data_dir.join(JOURNAL_FILE)).unwrap_or_default();
        for line in text.lines() {
            if let Ok(JournalLine::Done { op: id, result }) = serde_json::from_str(line)
                && id == op
            {
                return (line.to_string(), result);
            }
        }
        assert!(Instant::now() < deadline, "op {op} was never done: {text}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_calls_run_off_the_engine_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let (release, gate) = std::sync::mpsc::channel();
    let stub = Arc::new(Stub::default());
    *crate::lock(&stub.gate) = Some(gate);
    let s = service_with(stub.clone(), &data);
    let handle = s.spawn(tokio_util::sync::CancellationToken::new());
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.data_dir = data.join("runs").join(&run.id);
    let run_id = run.id.clone();
    crate::lock(&s.state)
        .runs
        .insert(run_id.clone(), run.clone());

    // The engine's view of PR #7, executed as the driver executes every op.
    let ctx = OpCtx {
        run_id: run_id.clone(),
        ..op_ctx(&run.data_dir, tmp.path())
    };
    let kind = host_kind(
        tmp.path(),
        HostOp::ViewPr {
            stage: 1,
            number: 7,
        },
    );
    s.execute(vec![Ready::Op { ctx, op: 9, kind }], 0).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stub.entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "view_pr never started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // While it blocks: the snapshot (the engine lock) and an edit (the event loop).
    let s2 = s.clone();
    let list = tokio::spawn(async move { s2.request(RunRequest::List).await });
    let listed = tokio::time::timeout(Duration::from_secs(10), list)
        .await
        .expect("List answered while a host call blocks")
        .unwrap();
    let RunReply::Snapshot(snap) = listed else {
        panic!("{listed:?}");
    };
    assert_eq!(snap.runs[0].run_id, run_id);
    let s3 = s.clone();
    let id = run_id.clone();
    let edit = tokio::spawn(async move {
        s3.request(RunRequest::Edit {
            run_id: id,
            edits: Vec::new(),
            submit: false,
        })
        .await
    });
    let edited = tokio::time::timeout(Duration::from_secs(10), edit)
        .await
        .expect("run edit answered while a host call blocks")
        .unwrap();
    // The engine's own answer (`run/engine/batch.rs`), so the edit reached the event
    // loop, not a refusal before it.
    assert_eq!(edited, RunReply::done(request::EDIT, "applied 0 edits"));
    assert!(
        crate::lock(&stub.calls).len() == 1,
        "the view is still in flight"
    );

    release.send(()).unwrap();
    let (_, result) = done_line(&run.data_dir, 9).await;
    assert_eq!(
        result,
        OpResult::Host(HostResult::PrViewed(Box::new(view(7))))
    );
    s.stop().await;
    handle.abort();
}

/// A runner whose every command sleeps for [`SLEEP`], ignoring its timeout, and then
/// notes that it finished.
pub(in crate::run::driver) struct Sleepy(pub Arc<AtomicBool>);
const SLEEP: Duration = Duration::from_secs(2);

impl Runner for Sleepy {
    fn run(
        &self,
        _: Program,
        _: &Path,
        _: &[String],
        _: &[(String, String)],
        _: Duration,
        _: Capture,
    ) -> Result<RunOutput, HostError> {
        std::thread::sleep(SLEEP);
        self.0.store(true, Ordering::SeqCst);
        Ok(RunOutput::default())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_op_that_times_out_answers_timed_out() {
    let tmp = tempfile::tempdir().unwrap();
    for op in [
        HostOp::ViewPr {
            stage: 1,
            number: 7,
        },
        HostOp::Push {
            stage: 1,
            sha: SHA.into(),
        },
    ] {
        let finished = Arc::new(AtomicBool::new(false));
        let host = GhHost::new(
            Sleepy(finished.clone()),
            "/nonexistent/anthrex-test/gh",
            "git",
        );
        let mut e = exec(Arc::new(host), Arc::new(GitQueue::new()));
        e.cap = Some(Duration::from_millis(300));
        let name = crate::run::engine::delivery::op_name(&op);
        let answer = execute(&e, &at(tmp.path(), None), repo(tmp.path()), op).await;
        // The answer came while the runner still slept: the bound, not the command.
        assert!(
            !finished.load(Ordering::SeqCst),
            "{name} waited for its runner"
        );
        assert_eq!(
            host_result(answer),
            HostResult::Error(HostError::TimedOut(format!(
                "{name} did not answer within 1 s"
            )))
        );
    }
    // Decision 9's bounds: each op's commands, and a margin.
    assert_eq!(
        bound(&HostOp::ViewPr {
            stage: 1,
            number: 7
        }),
        Duration::from_secs(65)
    );
    assert_eq!(
        bound(&HostOp::Push {
            stage: 1,
            sha: SHA.into()
        }),
        Duration::from_secs(185)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn push_and_fetch_go_through_the_git_queue() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = Arc::new(Stub::default());
    let queue = Arc::new(GitQueue::new());
    let e = Arc::new(exec(stub.clone(), queue.clone()));
    let project = at(tmp.path(), None).project;

    // Another git write holds the project's queue until the test releases it.
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = Mutex::new(released);
    let holder = {
        let (queue, project) = (queue.clone(), project.clone());
        tokio::spawn(async move {
            queue
                .write(&project, move || {
                    let _ = held_tx.send(());
                    let _ = crate::lock(&released).recv_timeout(Duration::from_secs(30));
                    Ok(())
                })
                .await
        })
    };
    held.recv_timeout(Duration::from_secs(10)).unwrap();

    let adopt = Adopt {
        local_ref: format!("anthrex/{RUN_ID}/stage-1"),
        expected_local: SHA.into(),
        also_integration: true,
    };
    let ops = [
        HostOp::Push {
            stage: 1,
            sha: SHA.into(),
        },
        HostOp::Fetch {
            stage: Some(1),
            branch: format!("anthrex/{RUN_ID}/stage-1"),
            into: format!("refs/anthrex/{RUN_ID}/remote/stage-1"),
            adopt: Some(adopt.clone()),
            parents_of: None,
        },
        HostOp::DeleteBranch { stage: 1 },
    ];
    let mut writes = Vec::new();
    for op in ops {
        let (e, dir) = (e.clone(), tmp.path().to_path_buf());
        writes.push(tokio::spawn(async move {
            execute(&e, &at(&dir, None), repo(&dir), op).await
        }));
    }
    // A read does not wait for the queue.
    let viewed = execute(
        &e,
        &at(tmp.path(), None),
        repo(tmp.path()),
        HostOp::ViewPr {
            stage: 1,
            number: 7,
        },
    )
    .await;
    assert!(matches!(host_result(viewed), HostResult::PrViewed(_)));
    for call in ["push", "fetch", "delete_branch"] {
        assert!(!stub.called(call), "{call} ran while the queue was held");
    }

    stub.released.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    holder.await.unwrap().unwrap();
    for write in writes {
        let answer = host_result(write.await.unwrap());
        assert!(!matches!(answer, HostResult::Error(_)), "{answer:?}");
    }
    assert!(crate::lock(&stub.early).is_empty(), "a write did not wait");
    for call in ["push", "fetch", "delete_branch"] {
        assert!(stub.called(call), "{call}");
    }
    // The adoption (its compare-and-swap is inside `fetch`) ran inside the queue too.
    assert_eq!(
        crate::lock(&stub.fetch).as_ref().unwrap().adopt,
        Some(adopt)
    );
}

/// Fix wave A2 (review A, M2): a queued op's bound starts once it holds the project's
/// queue, so a write that waited behind others longer than its whole bound still runs
/// and answers. (The wait is the condition under test, not a synchronisation: the op is
/// held for longer than its cap, then released.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queued_ops_bound_starts_when_it_takes_the_queue() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = Arc::new(Stub::default());
    let queue = Arc::new(GitQueue::new());
    let mut e = exec(stub.clone(), queue.clone());
    e.cap = Some(QUEUED_CAP);
    let project = at(tmp.path(), None).project;

    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = Mutex::new(released);
    let holder = {
        let (queue, project) = (queue.clone(), project.clone());
        tokio::spawn(async move {
            queue
                .write(&project, move || {
                    let _ = held_tx.send(());
                    let _ = crate::lock(&released).recv_timeout(Duration::from_secs(30));
                    Ok(())
                })
                .await
        })
    };
    held.recv_timeout(Duration::from_secs(10)).unwrap();
    let push = {
        let dir = tmp.path().to_path_buf();
        tokio::spawn(async move {
            let op = HostOp::Push {
                stage: 1,
                sha: SHA.into(),
            };
            execute(&e, &at(&dir, None), repo(&dir), op).await
        })
    };
    tokio::time::sleep(QUEUED_CAP * 2).await;
    assert!(
        !stub.called("push"),
        "the push ran while the queue was held"
    );
    stub.released.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    holder.await.unwrap().unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(30), push)
        .await
        .expect("the push answered")
        .unwrap();
    assert_eq!(host_result(answer), HostResult::Pushed(PushOutcome::Pushed));
    assert!(stub.called("push"));
}

/// [`a_queued_ops_bound_starts_when_it_takes_the_queue`]'s cap: the push after the
/// queue is released is one in-memory call, far inside it.
const QUEUED_CAP: Duration = Duration::from_secs(1);

#[path = "host_ops_tests_fields.rs"]
mod fields;
