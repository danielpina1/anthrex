//! The final fix wave's pack items (FW-33, FW-34, FW-41): a start that races the
//! round's first never sends a partial pack; a missing or changed pack file reaches the
//! engine as `OpResult::DesignPackUnreadable`; a pack write that failed names its cause.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use proto::RunState;

use super::read_rig::Rig;
use crate::run::design::pack::{PackFile, freeze};
use crate::run::design::state::{self, DesignState, sha256_hex};
use crate::run::driver::design_io::write_new;
use crate::run::engine::OpResult;
use crate::run::model::Run;
use crate::scout::design_spec::{DesignAgentKind, brainstormer_spec};

fn brainstorming(run: &mut Run, _: &Path) {
    run.design_mode = proto::DesignMode::Full;
    run.state = RunState::Brainstorming;
    run.orch.design = Some(DesignState {
        answers: Some("Links live an hour.".into()),
        pack: Some(freeze(run, None)),
        ..DesignState::default()
    });
}

fn pack_path(rig: &Rig) -> std::path::PathBuf {
    let engine = crate::lock(&rig.runs.state);
    state::design_dir(&engine.runs[&rig.run_id]).join("brainstorm/pack-r1.md")
}

/// FW-33: the first start writes the pack where it lies (the in-place fallback, with no
/// hard links) and is held halfway; a second start meanwhile waits for it, then sends
/// exactly the first's pack, never the half written.
#[tokio::test(flavor = "multi_thread")]
async fn a_racing_start_waits_for_the_first_starts_pack() {
    let rig = Rig::new(brainstorming).await;
    let path = pack_path(&rig);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let first_text = "first pack, written in place, ".repeat(64);
    let (entered, halfway) = std::sync::mpsc::channel::<()>();
    let (release, held) = std::sync::mpsc::channel::<()>();
    let in_place = move |path: &Path, text: &str| {
        let shown = |e: std::io::Error| e.to_string();
        let mut file = (std::fs::OpenOptions::new().write(true).create_new(true))
            .open(path)
            .map_err(shown)?;
        let (head, tail) = text.as_bytes().split_at(text.len() / 2);
        file.write_all(head).map_err(shown)?;
        file.flush().map_err(shown)?;
        let _ = entered.send(());
        let _ = held.recv_timeout(Duration::from_secs(30));
        file.write_all(tail).map_err(shown)
    };
    let (s, p, text) = (rig.runs.clone(), path.clone(), first_text.clone());
    let run = rig.run_id.clone();
    let first = tokio::spawn(async move { s.pack_file(&run, p, Some(text), None, in_place).await });
    let began = tokio::task::spawn_blocking(move || halfway.recv_timeout(Duration::from_secs(10)));
    assert!(began.await.unwrap().is_ok(), "the first start is halfway");
    // The slot's holders: the map, the first start's guard, and this clone.
    let slot = rig.runs.doc_writes.pack_slot(&rig.run_id, &path);
    let (s, p, run) = (rig.runs.clone(), path.clone(), rig.run_id.clone());
    let second_text = "the second start's own pack".to_string();
    let second = tokio::spawn(async move {
        s.pack_file(&run, p, Some(second_text), None, write_new)
            .await
    });
    // The W2 re-review's N3: event-driven, no window. The second start is waiting once
    // it holds a clone of the slot; unfixed, it reads the half-written file and ends.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::sync::Arc::strong_count(&slot) < 4 && !second.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "the second start never began"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !second.is_finished(),
        "the second start sent before the first ended"
    );
    drop(slot);
    drop(release);
    let first = first.await.unwrap().expect("the first start's pack");
    let second = tokio::time::timeout(Duration::from_secs(10), second)
        .await
        .expect("the second start ends")
        .unwrap()
        .expect("the second start's pack");
    assert_eq!(first, first_text);
    assert_eq!(sha256_hex(second.as_bytes()), sha256_hex(first.as_bytes()));
}

/// FW-34: `StartDesignAgent` of a brainstormer whose round's recorded pack file is
/// missing, or changed, is `DesignPackUnreadable` with why.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_or_changed_pack_is_unreadable() {
    let rig = Rig::new(brainstorming).await;
    let path = pack_path(&rig);
    let text = "the round's pack\n";
    let (ctx, spec) = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        let design = run.orch.design.as_mut().unwrap();
        design.pack.as_mut().unwrap().file = Some(PackFile {
            bytes: text.len() as u64,
            sha256: sha256_hex(text.as_bytes()),
        });
        let agent = crate::run::design::state::DesignAgent {
            label: "A".into(),
            role: proto::AgentRole::Brainstormer,
            route: proto::Route {
                runtime: proto::Runtime::Claude,
                model: "m".into(),
                strength: proto::Strength::Frontier,
                effort: proto::Effort::HIGH,
            },
            session: 1,
            window_id: None,
            state: crate::run::design::state::DesignAgentState::Queued,
            calls: 0,
            tokens: 0,
            started: None,
            listed: false,
            unsubmitted: false,
            round: 1,
        };
        let spec = brainstormer_spec(run, &agent);
        assert!(matches!(spec.kind, DesignAgentKind::Brainstormer { .. }));
        (crate::run::driver::OpCtx::of(run), spec)
    };
    let missing = rig.runs.start_design_agent(&ctx, spec.clone()).await;
    match missing {
        OpResult::DesignPackUnreadable { reason } => {
            assert!(reason.contains("pack-r1.md"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "another pack\n").unwrap();
    match rig.runs.start_design_agent(&ctx, spec).await {
        OpResult::DesignPackUnreadable { reason } => {
            let says = "does not match the pack this round's first start wrote";
            assert!(reason.ends_with(says), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

/// FW-41: the round's first start cannot write its pack (its folder refuses writes):
/// the reason names the write's own error, not only the read that followed.
#[tokio::test(flavor = "multi_thread")]
async fn a_pack_write_failure_names_its_cause() {
    let rig = Rig::new(brainstorming).await;
    let path = pack_path(&rig);
    let refused = |_: &Path, _: &str| Err("pack-r1.md: Permission denied (os error 13)".into());
    let error = rig
        .runs
        .pack_file(&rig.run_id, path, Some("a pack".into()), None, refused)
        .await
        .unwrap_err();
    assert!(
        error.contains("writing it failed: pack-r1.md: Permission denied"),
        "{error}"
    );
}

/// The W2 re-review's N1: a run's pack slots go once the run ends (or is discarded),
/// as its pasted notes' record does; a run that goes on keeps them.
#[tokio::test(flavor = "multi_thread")]
async fn a_runs_pack_slots_go_when_it_ends() {
    let rig = Rig::new(brainstorming).await;
    let path = pack_path(&rig);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let packed = (rig.runs.pack_file(
        &rig.run_id,
        path.clone(),
        Some("a pack".into()),
        None,
        write_new,
    ))
    .await;
    assert_eq!(packed.as_deref(), Ok("a pack"));
    rig.runs.check_orchestrators();
    assert_eq!(rig.runs.doc_writes.pack_paths(), [path], "still going");
    let failed = RunState::Failed;
    crate::lock(&rig.runs.state)
        .runs
        .get_mut(&rig.run_id)
        .unwrap()
        .state = failed;
    rig.runs.check_orchestrators();
    assert_eq!(
        rig.runs.doc_writes.pack_paths(),
        Vec::<std::path::PathBuf>::new(),
        "ended"
    );
}
