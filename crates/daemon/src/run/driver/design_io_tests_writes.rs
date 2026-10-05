//! Ruling WB-B-I1 (the final fix wave's FW-27): a design document's write that fails
//! or times out reaches the engine as `EventKind::DesignChecked` with its error, so the
//! gate reopens as revising, as a restore's read-back does; and a `run show` of a
//! version whose write is still in flight is answered with the retry text.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::run_wire::request;
use proto::{DocGateAction, DocGateKind, DocKind, RunReply, RunRequest, RunState};
use tokio_util::sync::CancellationToken;

use super::super::RunService;
use super::IO_WAIT;
use super::tests::{SPEC_1, design_run, service, spec, store, tmp};
use crate::run::design::state::DocGate;
use crate::run::engine::Effect;
use crate::run::test_support::RUN_ID;

/// The parts of a `WriteDoc`, as `write_doc_with` takes them.
type Write = (
    std::path::PathBuf,
    String,
    Option<(std::path::PathBuf, String)>,
    Option<crate::run::design::versions::WrittenDoc>,
);

/// A design run at its spec gate on v1, which is stored but not written yet, its
/// engine running: the service and v1's write.
fn at_spec_gate(data: &Path) -> (Arc<RunService>, Write) {
    let mut run = design_run(data);
    run.state = RunState::AwaitingApproval;
    let s = service(data, run);
    let (_, effect) = store(&s, spec(SPEC_1));
    let mut state = crate::lock(&s.state);
    let design = state.runs.get_mut(RUN_ID).unwrap().orch.design.as_mut();
    design.unwrap().gate = Some(DocGate {
        kind: DocGateKind::Spec,
        version: 1,
        opened_at: 2_000,
        revising: None,
        review: false,
        cause: Default::default(),
    });
    drop(state);
    let Effect::WriteDoc {
        path,
        text,
        index,
        doc,
    } = effect
    else {
        panic!("a write: {effect:?}");
    };
    (s, (path, text, index, doc))
}

/// The spec gate's revising note now.
fn revising_now(s: &RunService) -> Option<String> {
    let state = crate::lock(&s.state);
    let design = state.runs[RUN_ID].orch.design.clone();
    design.and_then(|d| d.gate).and_then(|g| g.revising)
}

/// The spec gate's revising note, once set, within a deadline.
async fn revising(s: &RunService) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let note = revising_now(s);
        if note.is_some() || Instant::now() >= deadline {
            return note;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn approve(s: &RunService) -> RunReply {
    s.request(RunRequest::DocGate {
        run: RUN_ID.into(),
        kind: DocGateKind::Spec,
        action: DocGateAction::APPROVE,
    })
    .await
}

const REOPENED: &str = "anthrex could not read back the stored spec v1; submit it again";
const REVISING: &str = "the orchestrator is revising spec v1; wait for it";

/// The file's write fails (its link refused with EIO, through `write_new_at`'s seam):
/// the gate reopens as revising, and `run approve --gate spec` is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_write_reopens_its_gate_as_revising() {
    let dir = tmp();
    let (s, write) = at_spec_gate(dir.path());
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());
    let failing = |path: &Path, text: &str| {
        let eio = |_: &Path, _: &Path| Err(std::io::Error::from_raw_os_error(libc::EIO));
        super::write_new_at(path, text, 1, &eio)
    };
    s.write_doc_with(write, IO_WAIT, failing).await;
    assert_eq!(revising(&s).await.as_deref(), Some(REOPENED));
    let refused = RunReply::refused(request::DOC_GATE, REVISING);
    assert_eq!(approve(&s).await, refused);
    shutdown.cancel();
}

/// The write passes its wait: the same, without waiting for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_timed_out_write_reopens_its_gate_as_revising() {
    let dir = tmp();
    let (s, write) = at_spec_gate(dir.path());
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());
    let (release, held) = std::sync::mpsc::channel::<()>();
    let stalled = move |path: &Path, text: &str| {
        let _ = held.recv_timeout(Duration::from_secs(30));
        super::write_new(path, text)
    };
    let call = s.write_doc_with(write, Duration::from_millis(200), stalled);
    tokio::time::timeout(Duration::from_secs(10), call)
        .await
        .expect("the write returns at its wait");
    assert_eq!(revising(&s).await.as_deref(), Some(REOPENED));
    drop(release);
    shutdown.cancel();
}

/// While v1's write is held, `run show` of it answers the retry text; once written, it
/// shows the version.
#[tokio::test(flavor = "multi_thread")]
async fn a_version_being_written_is_shown_with_the_retry_text() {
    let dir = tmp();
    let (s, write) = at_spec_gate(dir.path());
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (entered, began) = std::sync::mpsc::channel::<()>();
    let stalled = move |path: &Path, text: &str| {
        let _ = entered.send(());
        let _ = held.recv_timeout(Duration::from_secs(30));
        super::write_new(path, text)
    };
    let writer = s.clone();
    let writing = tokio::spawn(async move { writer.write_doc_with(write, IO_WAIT, stalled).await });
    let began = tokio::task::spawn_blocking(move || began.recv_timeout(Duration::from_secs(10)));
    assert!(began.await.unwrap().is_ok(), "the write started");
    let show = || RunRequest::ShowDoc {
        run: RUN_ID.into(),
        kind: DocKind::Spec,
        version: Some(1),
        diff: false,
        findings: false,
    };
    let retry = "v1 is still being written; try again in a moment";
    let refused = RunReply::refused(request::SHOW_DOC, retry);
    assert_eq!(s.request(show()).await, refused);
    drop(release);
    tokio::time::timeout(Duration::from_secs(10), writing)
        .await
        .expect("the write ends")
        .unwrap();
    match s.request(show()).await {
        RunReply::Doc { doc, .. } => assert_eq!(doc.text, SPEC_1),
        other => panic!("{other:?}"),
    }
    // A write that succeeded sends the engine nothing.
    assert_eq!(revising_now(&s), None);
    shutdown.cancel();
}
