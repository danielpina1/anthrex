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
        report: None,
        draft_review: None,
        uncapped: false,
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

/// Task M9.6.9: the texts a restore keeps are each gate document's latest version and
/// each brainstormer's latest draft (the merged report's appendix attaches it); older
/// drafts, older versions and a spec's review draft are checked, not kept.
#[test]
fn the_latest_draft_of_each_brainstormer_is_kept() {
    use crate::run::design::state::DesignState;
    let draft = |label: &str, n: u32| DocVersion {
        author: DocAuthor::Brainstormer {
            label: label.into(),
        },
        ..version(DocKind::BrainstormDraft, n, label)
    };
    let design = DesignState {
        versions: vec![
            draft("claude", 1),
            draft("codex", 2),
            version(DocKind::Brainstorm, 1, "report"),
            draft("claude", 3),
            version(DocKind::Spec, 0, "review draft"),
        ],
        ..DesignState::default()
    };
    let kept: Vec<(DocKind, u32)> = (design.versions.iter())
        .filter(|v| super::kept(&design, v))
        .map(|v| (v.kind, v.n))
        .collect();
    assert_eq!(
        kept,
        [
            (DocKind::BrainstormDraft, 2),
            (DocKind::Brainstorm, 1),
            (DocKind::BrainstormDraft, 3)
        ]
    );
}

/// Task M9.6.10: the approved spec is kept while its requirements are not stored (a
/// restore between the approval and its read-back), even when it is not the latest
/// spec; once they are stored, only the latest is.
#[test]
fn the_approved_spec_is_kept_until_its_requirements_are_stored() {
    use crate::run::design::state::{DesignState, Requirement};
    let mut design = DesignState {
        versions: vec![
            version(DocKind::Spec, 1, "v1"),
            version(DocKind::Spec, 2, "v2"),
        ],
        approved_spec: Some(1),
        ..DesignState::default()
    };
    let kept = |design: &DesignState| -> Vec<u32> {
        (design.versions.iter())
            .filter(|v| super::kept(design, v))
            .map(|v| v.n)
            .collect()
    };
    assert_eq!(kept(&design), [1, 2]);
    design.requirements = vec![Requirement {
        id: "R1".into(),
        text: "one".into(),
    }];
    assert_eq!(kept(&design), [2]);
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
        at_spec_gate_with(data, "# S\n", edit).await
    }

    /// [`at_spec_gate`], its spec v1 `text`.
    async fn at_spec_gate_with(
        data: &Path,
        text: &str,
        edit: impl FnOnce(&Path),
    ) -> Arc<RunService> {
        let mut run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", "[\"a\"]", "")]));
        run.data_dir = data.join("runs").join(RUN_ID);
        run.design_mode = proto::DesignMode::Full;
        run.orch.design = Some(DesignState::default());
        run.state = RunState::AwaitingApproval;
        let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "submitted", text);
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
                action: DocGateAction::APPROVE,
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
    /// its wait. A read that never answers (the seam blocks on a channel this test
    /// holds) times it out: the restore goes on, and the engine is told nothing of its
    /// gate documents, so the gate stays as it was. Fix round 2: it cannot hang. The
    /// test's sender is dropped on every exit, a panic's unwinding included, which ends
    /// the blocked read; the read's own wait is bounded too, and the call has a hard
    /// deadline.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_back_that_times_out_changes_nothing() {
        const READ_BOUND: Duration = Duration::from_secs(30);
        let data = tempfile::tempdir().unwrap();
        let s = at_spec_gate(data.path(), |_| {}).await;
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        let gate_of = |s: &RunService| {
            let state = crate::lock(&s.state);
            let design = state.runs[RUN_ID].orch.design.clone();
            design.and_then(|d| d.gate)
        };
        let before = gate_of(&s);
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, read_began) = std::sync::mpsc::channel::<()>();
        let stalled = move |runs: Vec<(String, Vec<super::super::ToCheck>)>| {
            let _ = entered.send(());
            let _ = held.recv_timeout(READ_BOUND);
            runs.into_iter()
                .map(|(id, docs)| (id, super::super::check(docs)))
                .collect()
        };
        let started = Instant::now();
        let call = s.check_design_docs_with(Duration::from_millis(300), stalled);
        tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the read-back returns at its wait");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "bounded by its wait"
        );
        let began = read_began.recv_timeout(Duration::from_secs(5));
        assert!(began.is_ok(), "the read had started: the wait timed it out");
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
        drop(release);
        shutdown.cancel();
    }

    /// Ruling T9-1a: a read-back that times out still answers for each kept
    /// brainstorm draft, as unreadable with why, so a merged report never waits for its
    /// text forever; the older draft (not kept) and the gate are left alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_back_that_times_out_marks_the_kept_drafts_unreadable() {
        const READ_BOUND: Duration = Duration::from_secs(30);
        let data = tempfile::tempdir().unwrap();
        let s = at_spec_gate(data.path(), |_| {}).await;
        let writes = {
            let mut state = crate::lock(&s.state);
            let run = state.runs.get_mut(RUN_ID).unwrap();
            let mut writes = Vec::new();
            for (label, text) in [("claude", "a"), ("codex", "b"), ("claude", "c")] {
                let author = DocAuthor::Brainstormer {
                    label: label.into(),
                };
                let doc = NewDoc::new(DocKind::BrainstormDraft, author, "submitted", text);
                writes.push(state::store(run, doc, 2_001).unwrap().1);
            }
            run.orch.design.as_mut().unwrap().drafts_settled = true;
            writes
        };
        for write in writes {
            let Effect::WriteDoc { path, text, index } = write else {
                panic!("a write");
            };
            s.write_doc(path, text, index).await;
        }
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        let (release, held) = std::sync::mpsc::channel::<()>();
        let stalled = move |runs: Vec<(String, Vec<super::super::ToCheck>)>| {
            let _ = held.recv_timeout(READ_BOUND);
            runs.into_iter()
                .map(|(id, docs)| (id, super::super::check(docs)))
                .collect()
        };
        let call = s.check_design_docs_with(Duration::from_millis(300), stalled);
        tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the read-back returns at its wait");
        let deadline = Instant::now() + Duration::from_secs(10);
        let unread = loop {
            let design = crate::lock(&s.state).runs[RUN_ID]
                .orch
                .design
                .clone()
                .unwrap();
            if design.unread.len() >= 2 || Instant::now() >= deadline {
                break design;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let why = "its read took over 300 ms".to_string();
        // Drafts 3 (claude's latest) and 2 (codex's); claude's first is not kept.
        let mut marked = unread.unread.clone();
        marked.sort();
        assert_eq!(marked, [(2, why.clone()), (3, why)]);
        assert_eq!(
            unread.gate.and_then(|g| g.revising),
            None,
            "the gate as stored"
        );
        drop(release);
        shutdown.cancel();
    }

    /// The spec's approval through the service (task M9.6.10): the engine asks for the
    /// approved version's text (`Effect::ReadBack`), the driver reads it off the engine
    /// lock against its index entry, and the engine stores its requirements and Goal
    /// section from that read; a file changed since it was stored is never used, and
    /// the spec gate reopens instead.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_approved_spec_is_read_back_and_its_requirements_stored() {
        const SPEC: &str = "# S\n\n## Goal and success criteria\nReset passwords.\n\n\
                            ## Requirements\nR1 one\nR2 two\n";
        let approve = RunRequest::DocGate {
            run: RUN_ID.into(),
            kind: DocGateKind::Spec,
            action: DocGateAction::APPROVE,
        };
        let design = |s: &RunService| {
            let state = crate::lock(&s.state);
            (
                state.runs[RUN_ID].orch.design.clone().unwrap(),
                state.runs[RUN_ID].state,
            )
        };
        let settled = async |s: &RunService, done: &dyn Fn(&DesignState, RunState) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let (d, state) = design(s);
                if done(&d, state) || Instant::now() >= deadline {
                    return (d, state);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        let data = tempfile::tempdir().unwrap();
        let s = at_spec_gate_with(data.path(), SPEC, |_| {}).await;
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        assert!(matches!(
            s.request(approve.clone()).await,
            RunReply::Done { .. }
        ));
        let (d, _) = settled(&s, &|d, _| !d.requirements.is_empty()).await;
        let ids: Vec<&str> = d.requirements.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["R1", "R2"]);
        assert_eq!(d.goal_section, "Reset passwords.");
        shutdown.cancel();

        let data = tempfile::tempdir().unwrap();
        let changed = |path: &Path| std::fs::write(path, "# S, edited by hand\n").unwrap();
        let s = at_spec_gate_with(data.path(), SPEC, changed).await;
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        assert!(matches!(s.request(approve).await, RunReply::Done { .. }));
        let (d, state) = settled(&s, &|_, state| state == RunState::AwaitingApproval).await;
        assert_eq!(state, RunState::AwaitingApproval);
        assert!(d.requirements.is_empty());
        let gate = d.gate.unwrap();
        assert_eq!(
            (gate.kind, gate.cause),
            (DocGateKind::Spec, state::Revision::ReadBack)
        );
        shutdown.cancel();
    }
}
