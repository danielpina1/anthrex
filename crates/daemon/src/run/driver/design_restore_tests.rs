//! Milestone 9.6 task M9.6.7 (task 5's carry): the restore's read-back of a design
//! run's versions, on a real temporary folder: a missing file and a changed file are
//! reported, the latest text of a gate document is kept, and nothing else is read into
//! memory.

use proto::{DocAuthor, DocKind};

use super::check;
use crate::run::design::state::{DocVersion, sha256_hex};
use crate::run::engine::DocChecked;

fn version(kind: DocKind, n: u32, text: &str) -> DocVersion {
    DocVersion {
        kind,
        n,
        author: DocAuthor::Orchestrator,
        reason: "submitted".into(),
        bytes: text.len() as u64,
        sha256: sha256_hex(text.as_bytes()),
        at: 1,
        requirements: Vec::new(),
        disputed: Vec::new(),
        not_reviewed: None,
        changes: Vec::new(),
        same_runtime: false,
    }
}

#[test]
fn every_version_is_read_back_and_checked_against_its_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name);
    std::fs::write(path("brainstorm-v1.md"), "report one").unwrap();
    std::fs::write(path("brainstorm-v2.md"), "report two").unwrap();
    std::fs::write(path("spec-v1.md"), "a spec, then changed by hand").unwrap();
    let docs = vec![
        (
            version(DocKind::Brainstorm, 1, "report one"),
            path("brainstorm-v1.md"),
            false,
        ),
        (
            version(DocKind::Brainstorm, 2, "report two"),
            path("brainstorm-v2.md"),
            true,
        ),
        (
            version(DocKind::Spec, 1, "a spec"),
            path("spec-v1.md"),
            true,
        ),
        (
            version(DocKind::Plan, 1, "a plan"),
            path("plan-v1.md"),
            true,
        ),
    ];
    let checked = check(docs);
    let reads: Vec<_> = checked.iter().map(|c| (c.kind, c.n)).collect();
    assert_eq!(
        reads,
        [
            (DocKind::Brainstorm, 1),
            (DocKind::Brainstorm, 2),
            (DocKind::Spec, 1),
            (DocKind::Plan, 1)
        ]
    );
    assert_eq!(
        checked[0].read,
        Ok(None),
        "an older version: checked, not kept"
    );
    assert_eq!(checked[1].read, Ok(Some("report two".into())));
    assert_eq!(
        checked[2],
        DocChecked {
            kind: DocKind::Spec,
            n: 1,
            read: Err("its file differs from what was stored".into()),
        }
    );
    let missing = checked[3].read.clone().unwrap_err();
    assert!(missing.contains("plan-v1.md"), "{missing}");
}

mod service {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use proto::run_wire::request;
    use proto::{DocAuthor, DocGateAction, DocGateKind, DocKind, RunReply, RunRequest, RunState};
    use tokio_util::sync::CancellationToken;

    use crate::manager::{GitRoots, ManagerConfig, WindowManager};
    use crate::run::design::state::{self, DesignState, DocGate, NewDoc};
    use crate::run::driver::RunService;
    use crate::run::engine::Effect;
    use crate::run::test_support::{PROFILE, RUN_ID, plan_with, run_ok, task_toml};

    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }

    /// A design run at its spec gate, v1, whose file the service wrote; then `edit`
    /// applied to that file.
    async fn at_spec_gate(data: &Path, edit: impl FnOnce(&Path)) -> Arc<RunService> {
        let mut run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", "[\"a\"]", "")]));
        run.data_dir = data.join("runs").join(RUN_ID);
        run.design_mode = proto::DesignMode::Full;
        run.orch.design = Some(DesignState::default());
        run.state = RunState::AwaitingApproval;
        let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "submitted", "# S\n");
        let (_, write) = state::store(&mut run, doc, 2_000).unwrap();
        run.orch.design.as_mut().unwrap().gate = Some(DocGate {
            kind: DocGateKind::Spec,
            version: 1,
            opened_at: 2_000,
            revising: None,
            review: false,
            cause: Default::default(),
        });
        let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        let (manager, _events) = WindowManager::new(config);
        let s = RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots));
        crate::lock(&s.state).runs.insert(run.id.clone(), run);
        let Effect::WriteDoc { path, text, index } = write else {
            panic!("a write: {write:?}");
        };
        s.write_doc(path.clone(), text, index).await;
        edit(&path);
        s
    }

    /// The restore's check, through the engine: a changed file reopens the gate as
    /// revising, and a `run approve --gate spec` is then refused by the engine's own
    /// rule (`RunRequest::DocGate` reaches it).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_changed_gate_file_reopens_the_gate_through_the_engine() {
        let data = tempfile::tempdir().unwrap();
        let changed = |path: &Path| std::fs::write(path, "# S, edited by hand\n").unwrap();
        let s = at_spec_gate(data.path(), changed).await;
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        s.check_design_docs().await;
        let deadline = Instant::now() + Duration::from_secs(10);
        let revising = loop {
            let gate = {
                let state = crate::lock(&s.state);
                let design = state.runs[RUN_ID].orch.design.clone();
                design.and_then(|d| d.gate).and_then(|g| g.revising)
            };
            if gate.is_some() || Instant::now() >= deadline {
                break gate;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(
            revising.as_deref(),
            Some("anthrex could not read back the stored spec v1; submit it again")
        );
        let reply = s
            .request(RunRequest::DocGate {
                run: RUN_ID.into(),
                kind: DocGateKind::Spec,
                action: DocGateAction::Approve,
            })
            .await;
        assert_eq!(
            reply,
            RunReply::refused(
                request::DOC_GATE,
                "the orchestrator is revising spec v1; wait for it"
            )
        );
        shutdown.cancel();
    }

    /// Review m4 and m5: the read-back is one blocking task for every run, bounded by
    /// its wait. A file that never answers (a FIFO with no writer) times it out: the
    /// restore goes on, and the engine is told nothing, so the gate stays as it was.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_back_that_times_out_changes_nothing() {
        let data = tempfile::tempdir().unwrap();
        let made = std::sync::Mutex::new(None);
        let fifo = |path: &Path| {
            std::fs::remove_file(path).unwrap();
            let status = std::process::Command::new("mkfifo").arg(path).status();
            assert!(status.unwrap().success(), "mkfifo {}", path.display());
            *made.lock().unwrap() = Some(path.to_path_buf());
        };
        let s = at_spec_gate(data.path(), fifo).await;
        let fifo = made.lock().unwrap().clone().unwrap();
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        let gate_of = |s: &RunService| {
            let state = crate::lock(&s.state);
            let design = state.runs[RUN_ID].orch.design.clone();
            design.and_then(|d| d.gate)
        };
        let before = gate_of(&s);
        let started = Instant::now();
        s.check_design_docs_within(Duration::from_millis(300)).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "bounded by its wait"
        );
        // The service still answers, and nothing reached the engine.
        let reply = s
            .request(RunRequest::DocGate {
                run: RUN_ID.into(),
                kind: DocGateKind::Spec,
                action: DocGateAction::Rethink { note: "r".into() },
            })
            .await;
        let refused = "rethink is only for the brainstorm gate";
        assert_eq!(reply, RunReply::refused(request::DOC_GATE, refused));
        assert_eq!(gate_of(&s), before, "the gate is unchanged");
        // The blocked read gets its end of file, so the blocking thread ends. Opened
        // without blocking: with no reader waiting, it fails instead of hanging.
        use std::os::unix::fs::OpenOptionsExt;
        let mut open = std::fs::OpenOptions::new();
        open.write(true).custom_flags(libc::O_NONBLOCK);
        drop(
            open.open(&fifo)
                .expect("the read-back still waits on the FIFO"),
        );
        shutdown.cancel();
    }
}
